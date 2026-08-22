//! Paper position/P&L tracking (Group G.6/G.7): the state `sizing.rs`
//! reads and writes, plus the exit-rule check (take-profit/stop-loss).
//! Emergency exits (a hard block or a full liquidity drain appearing
//! *after* entry) are not decided here — the caller (`bin/pipeline.rs`,
//! which already computes `RiskSnapshot::hard_blocks` and
//! `EventPayload::LiquidityRemoved::all_liquidity_removed`) detects those
//! and calls [`Portfolio::close_position`] directly with
//! [`ExitReason::EmergencyExit`], the same as any other close.
//!
//! Deliberately holds no opinion on *how* a position's current value is
//! computed — `check_exit` takes it as a plain `f64` argument. Getting an
//! accurate value requires real bonding-curve/pool quote math
//! (`momentum_pump::quote_sell`/`momentum_pumpswap::quote_sell`), which
//! this crate cannot depend on (`core`-level crates don't depend on venue
//! crates — same rule `momentum_ingest`'s doc comment states). The live
//! wiring in `bin/pipeline.rs` is what supplies a real value; this crate's
//! own tests supply synthetic ones, same "no external ground truth, this
//! is our own policy" stance as `momentum_reputation`.

use std::collections::HashMap;

use momentum_core::domain::Venue;
use momentum_core::risk_engine::Decision;

use crate::sizing::PositionSizingConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitReason {
    TakeProfit,
    StopLoss,
    /// A hard block or a full liquidity drain appeared after entry — see
    /// this module's doc comment for why the *detection* lives in the
    /// caller, not here.
    EmergencyExit,
}

impl ExitReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            ExitReason::TakeProfit => "take_profit",
            ExitReason::StopLoss => "stop_loss",
            ExitReason::EmergencyExit => "emergency_exit",
        }
    }
}

#[derive(Debug, Clone)]
pub struct OpenPosition {
    pub mint: String,
    pub venue: Venue,
    pub entry_usd: f64,
    /// Raw token units received at entry (from a real
    /// `quote_buy_exact_quote_in` call) — kept so the caller can later
    /// call the matching venue's `quote_sell` for an exit quote without
    /// this crate needing to know venue-specific decimals or math.
    pub token_amount: u64,
    pub decision: Decision,
    pub opened_at_ns: u64,
}

#[derive(Debug, Clone)]
pub struct ClosedTrade {
    pub mint: String,
    pub venue: Venue,
    pub entry_usd: f64,
    pub exit_usd: f64,
    pub pnl_usd: f64,
    /// `pnl_usd / entry_usd`. `0.0` in the (should-be-unreachable in
    /// practice, since sizing never opens a `$0` position) case of a
    /// `entry_usd <= 0.0` position, rather than a `NaN`/`inf` that would
    /// poison any later aggregate over `closed` trades.
    pub pnl_pct: f64,
    pub decision: Decision,
    pub reason: ExitReason,
    pub opened_at_ns: u64,
    pub closed_at_ns: u64,
}

#[derive(Debug)]
pub struct Portfolio {
    pub cash_usd: f64,
    open: HashMap<String, OpenPosition>,
    closed: Vec<ClosedTrade>,
}

impl Portfolio {
    pub fn new(initial_cash_usd: f64) -> Self {
        Self { cash_usd: initial_cash_usd, open: HashMap::new(), closed: Vec::new() }
    }

    pub fn is_open(&self, mint: &str) -> bool {
        self.open.contains_key(mint)
    }

    pub fn open_count(&self) -> usize {
        self.open.len()
    }

    pub fn open_position(&mut self, mint: String, venue: Venue, entry_usd: f64, token_amount: u64, decision: Decision, opened_at_ns: u64) {
        self.cash_usd -= entry_usd;
        self.open.insert(mint.clone(), OpenPosition { mint, venue, entry_usd, token_amount, decision, opened_at_ns });
    }

    /// The open position for `mint`, if any — the caller needs
    /// `token_amount`/`venue` from it to compute a real current-value
    /// quote before calling `check_exit`/`close_position`.
    pub fn open_position_for(&self, mint: &str) -> Option<&OpenPosition> {
        self.open.get(mint)
    }

    /// Total entry cost of every currently open position — what
    /// `sizing::position_size_usd`'s deployed-capital cap is measured
    /// against. Deliberately entry cost, not current mark-to-market value:
    /// this is a cap on capital *committed*, not a live risk figure that
    /// would shrink on its own as positions lose value (which would let
    /// the cap loosen exactly when the portfolio is already underwater).
    pub fn deployed_usd(&self) -> f64 {
        self.open.values().map(|p| p.entry_usd).sum()
    }

    /// `Some(TakeProfit | StopLoss)` if `current_value_usd` has crossed
    /// either threshold in `config`, `None` if `mint` isn't held or is
    /// still within both bounds. Never closes the position itself — see
    /// `close_position`.
    pub fn check_exit(&self, mint: &str, current_value_usd: f64, config: &PositionSizingConfig) -> Option<ExitReason> {
        let position = self.open.get(mint)?;
        if position.entry_usd <= 0.0 {
            return None;
        }
        let ratio = current_value_usd / position.entry_usd;
        if ratio >= config.take_profit_multiple {
            Some(ExitReason::TakeProfit)
        } else if ratio <= config.stop_loss_multiple {
            Some(ExitReason::StopLoss)
        } else {
            None
        }
    }

    /// Closes `mint`'s open position at `exit_usd` (credited back to
    /// `cash_usd`), records it in `closed`, and returns the resulting
    /// trade. `None` if `mint` has no open position — a caller reacting to
    /// a stale/duplicate signal for an already-closed mint is a no-op, not
    /// a panic or a double-credit.
    pub fn close_position(&mut self, mint: &str, exit_usd: f64, reason: ExitReason, closed_at_ns: u64) -> Option<ClosedTrade> {
        let position = self.open.remove(mint)?;
        self.cash_usd += exit_usd;
        let pnl_usd = exit_usd - position.entry_usd;
        let pnl_pct = if position.entry_usd > 0.0 { pnl_usd / position.entry_usd } else { 0.0 };
        let trade = ClosedTrade {
            mint: position.mint,
            venue: position.venue,
            entry_usd: position.entry_usd,
            exit_usd,
            pnl_usd,
            pnl_pct,
            decision: position.decision,
            reason,
            opened_at_ns: position.opened_at_ns,
            closed_at_ns,
        };
        self.closed.push(trade.clone());
        Some(trade)
    }

    pub fn closed_trades(&self) -> &[ClosedTrade] {
        &self.closed
    }

    pub fn realized_pnl_usd(&self) -> f64 {
        self.closed.iter().map(|t| t.pnl_usd).sum()
    }

    /// `cash_usd` plus every open position valued via `value_of` — a
    /// caller-supplied closure, for the same reason `check_exit` takes a
    /// plain `f64`: this crate has no venue-specific quoting math of its
    /// own. A closure that can't produce a value for some position (e.g. no
    /// cached curve/pool state yet) should return that position's
    /// `entry_usd` as its best available estimate, not skip it — silently
    /// omitting an open position from equity would understate real risk.
    pub fn equity_usd(&self, mut value_of: impl FnMut(&OpenPosition) -> f64) -> f64 {
        self.cash_usd + self.open.values().map(&mut value_of).sum::<f64>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sizing::DEFAULT_POSITION_SIZING_CONFIG;

    fn config() -> PositionSizingConfig {
        DEFAULT_POSITION_SIZING_CONFIG
    }

    #[test]
    fn opening_a_position_debits_cash_and_records_it_as_held() {
        let mut portfolio = Portfolio::new(1_000.0);
        portfolio.open_position("mint-1".to_string(), Venue::Pump, 100.0, 5_000, Decision::ConfirmedEntry, 0);
        assert!((portfolio.cash_usd - 900.0).abs() < 1e-9);
        assert!(portfolio.is_open("mint-1"));
        assert_eq!(portfolio.open_count(), 1);
        assert_eq!(portfolio.deployed_usd(), 100.0);
    }

    #[test]
    fn take_profit_triggers_at_the_configured_multiple() {
        let mut portfolio = Portfolio::new(1_000.0);
        portfolio.open_position("mint-1".to_string(), Venue::Pump, 100.0, 5_000, Decision::ConfirmedEntry, 0);
        assert_eq!(portfolio.check_exit("mint-1", 299.0, &config()), None, "just under 3x should not trigger yet");
        assert_eq!(portfolio.check_exit("mint-1", 300.0, &config()), Some(ExitReason::TakeProfit));
        assert_eq!(portfolio.check_exit("mint-1", 500.0, &config()), Some(ExitReason::TakeProfit), "well past 3x is still take-profit");
    }

    #[test]
    fn stop_loss_triggers_at_the_configured_multiple() {
        let mut portfolio = Portfolio::new(1_000.0);
        portfolio.open_position("mint-1".to_string(), Venue::Pump, 100.0, 5_000, Decision::ConfirmedEntry, 0);
        assert_eq!(portfolio.check_exit("mint-1", 51.0, &config()), None, "just above 50% loss should not trigger yet");
        assert_eq!(portfolio.check_exit("mint-1", 50.0, &config()), Some(ExitReason::StopLoss));
        assert_eq!(portfolio.check_exit("mint-1", 0.0, &config()), Some(ExitReason::StopLoss), "a total loss is still stop-loss");
    }

    #[test]
    fn a_position_between_the_two_thresholds_does_not_exit() {
        let mut portfolio = Portfolio::new(1_000.0);
        portfolio.open_position("mint-1".to_string(), Venue::Pump, 100.0, 5_000, Decision::ConfirmedEntry, 0);
        assert_eq!(portfolio.check_exit("mint-1", 150.0, &config()), None);
    }

    #[test]
    fn checking_exit_on_a_mint_not_held_is_none_not_a_panic() {
        let portfolio = Portfolio::new(1_000.0);
        assert_eq!(portfolio.check_exit("mint-never-opened", 1_000_000.0, &config()), None);
    }

    #[test]
    fn closing_a_profitable_position_credits_cash_and_records_positive_pnl() {
        let mut portfolio = Portfolio::new(1_000.0);
        portfolio.open_position("mint-1".to_string(), Venue::Pump, 100.0, 5_000, Decision::ConfirmedEntry, 10);
        let trade = portfolio.close_position("mint-1", 350.0, ExitReason::TakeProfit, 20).unwrap();

        assert!((portfolio.cash_usd - 1_250.0).abs() < 1e-9, "900 remaining cash + 350 exit proceeds");
        assert!(!portfolio.is_open("mint-1"));
        assert!((trade.pnl_usd - 250.0).abs() < 1e-9);
        assert!((trade.pnl_pct - 2.5).abs() < 1e-9);
        assert_eq!(trade.reason, ExitReason::TakeProfit);
        assert_eq!(trade.opened_at_ns, 10);
        assert_eq!(trade.closed_at_ns, 20);
        assert!((portfolio.realized_pnl_usd() - 250.0).abs() < 1e-9);
    }

    #[test]
    fn closing_a_losing_position_records_negative_pnl() {
        let mut portfolio = Portfolio::new(1_000.0);
        portfolio.open_position("mint-1".to_string(), Venue::Pump, 100.0, 5_000, Decision::ConfirmedEntry, 0);
        let trade = portfolio.close_position("mint-1", 40.0, ExitReason::StopLoss, 1).unwrap();
        assert!((trade.pnl_usd - -60.0).abs() < 1e-9);
        assert!((trade.pnl_pct - -0.6).abs() < 1e-9);
        assert!((portfolio.realized_pnl_usd() - -60.0).abs() < 1e-9);
    }

    #[test]
    fn an_emergency_exit_can_close_at_zero_without_panicking() {
        let mut portfolio = Portfolio::new(1_000.0);
        portfolio.open_position("mint-1".to_string(), Venue::Pump, 100.0, 5_000, Decision::ConfirmedEntry, 0);
        let trade = portfolio.close_position("mint-1", 0.0, ExitReason::EmergencyExit, 1).unwrap();
        assert!((trade.pnl_usd - -100.0).abs() < 1e-9, "a full rug is a total loss of the entry cost");
        assert!((portfolio.cash_usd - 900.0).abs() < 1e-9, "no exit proceeds credited back");
    }

    #[test]
    fn closing_a_mint_with_no_open_position_is_a_noop_not_a_panic() {
        let mut portfolio = Portfolio::new(1_000.0);
        assert!(portfolio.close_position("mint-never-opened", 100.0, ExitReason::TakeProfit, 0).is_none());
        assert!((portfolio.cash_usd - 1_000.0).abs() < 1e-9, "cash must be untouched");
    }

    #[test]
    fn equity_combines_cash_and_every_open_positions_supplied_value() {
        let mut portfolio = Portfolio::new(1_000.0);
        portfolio.open_position("mint-1".to_string(), Venue::Pump, 100.0, 5_000, Decision::ConfirmedEntry, 0);
        portfolio.open_position("mint-2".to_string(), Venue::PumpSwap, 50.0, 2_000, Decision::ProbeEntry, 0);
        // 850 cash remaining (1000 - 100 - 50) + synthetic valuations below.
        let equity = portfolio.equity_usd(|p| if p.mint == "mint-1" { 200.0 } else { 30.0 });
        assert!((equity - (850.0 + 200.0 + 30.0)).abs() < 1e-9);
    }

    #[test]
    fn deployed_usd_is_entry_cost_not_current_value() {
        let mut portfolio = Portfolio::new(1_000.0);
        portfolio.open_position("mint-1".to_string(), Venue::Pump, 100.0, 5_000, Decision::ConfirmedEntry, 0);
        // deployed_usd must stay 100 regardless of what this position is
        // "really" worth now — equity_usd is where current value belongs.
        assert_eq!(portfolio.deployed_usd(), 100.0);
        let _ = portfolio.equity_usd(|_| 9_999.0);
        assert_eq!(portfolio.deployed_usd(), 100.0);
    }
}
