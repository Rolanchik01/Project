//! Position sizing (Group G.6): turns a `risk_engine::RiskSnapshot`'s
//! `decision`/`position_multiplier` — which, until this crate, were only
//! ever printed to stdout by `bin/pipeline.rs`, never acted on — into an
//! actual dollar amount to risk, subject to real capital/capacity limits.
//!
//! `RiskSnapshot::position_multiplier` was always a *multiplier* (`1.0` for
//! `ConfirmedEntry`, `0.2` for `ProbeEntry`, `0.0` otherwise) with no base
//! size defined anywhere to multiply against — this module is what supplies
//! that base and the safety rails around it.
//!
//! # Sizing policy
//!
//! `base_position_usd = initial_capital_usd * max_position_fraction` is
//! fixed relative to the portfolio's **starting** capital, not its current
//! equity — sizing off current equity would compound (a winning streak
//! sizes up, quietly raising risk right when overconfidence is most
//! dangerous; a losing streak sizes down, which sounds prudent but also
//! means recovering from a drawdown takes proportionally longer). A fixed
//! fraction of starting capital is simpler and keeps the "controlled risk"
//! half of this project's stated goal (stable profit on a $1000 deposit,
//! not a maximized one) legible: at the default `0.1`, a `ConfirmedEntry`
//! risks $100, unconditionally, for as long as the ledger exists.
//!
//! Actual size is the base scaled by `position_multiplier`, then clamped by
//! three independent limits, each a real, separate failure mode this is
//! guarding against:
//! - `portfolio.cash_usd` — can't spend money that isn't there.
//! - `max_concurrent_positions` — caps how many *simultaneous* bad bets a
//!   coordinated bad narrative/period can produce, independent of dollar
//!   size.
//! - `max_deployed_fraction * initial_capital_usd` — caps total capital at
//!   risk across all open positions at once, independent of position
//!   count (five $150 positions from stacked `position_multiplier`
//!   changes could still over-deploy even under a concurrency cap alone).
//!
//! # Verification note
//!
//! Same as `momentum_reputation`: no external ground truth to check this
//! policy against — these are this project's own risk-budgeting choices,
//! not a protocol this decodes. Tests verify the policy behaves exactly as
//! designed.

use momentum_core::risk_engine::Decision;

use crate::portfolio::Portfolio;

#[derive(Debug, Clone, Copy)]
pub struct PositionSizingConfig {
    pub version: &'static str,
    /// Starting portfolio capital, in USD. `crates/live/src/bin/pipeline.rs`
    /// seeds `Portfolio::new` with this same value.
    pub initial_capital_usd: f64,
    /// Fraction of `initial_capital_usd` risked on a full-confidence
    /// (`position_multiplier == 1.0`) entry, before any of the caps below.
    pub max_position_fraction: f64,
    /// Hard cap on simultaneously open positions.
    pub max_concurrent_positions: usize,
    /// Hard cap on total capital deployed across all open positions at
    /// once, as a fraction of `initial_capital_usd`.
    pub max_deployed_fraction: f64,
    /// Exit once an open position's current value reaches this multiple of
    /// its entry cost (e.g. `3.0` = exit at 3x).
    pub take_profit_multiple: f64,
    /// Exit once an open position's current value falls to this multiple
    /// of its entry cost (e.g. `0.5` = exit at a 50% loss).
    pub stop_loss_multiple: f64,
}

pub const DEFAULT_POSITION_SIZING_CONFIG: PositionSizingConfig = PositionSizingConfig {
    version: "portfolio-baseline-2026-08",
    initial_capital_usd: 1_000.0,
    max_position_fraction: 0.10,
    max_concurrent_positions: 5,
    max_deployed_fraction: 0.5,
    take_profit_multiple: 3.0,
    stop_loss_multiple: 0.5,
};

/// Decides how much (if anything) to risk entering `mint`, given the
/// risk-engine's `decision`/`position_multiplier` for it and the
/// portfolio's current state. Returns `None` — not a zero-sized entry —
/// for every case where no position should be opened at all: `Reject`/
/// `Observe` decisions, a mint already held (this function never adds to
/// an existing position), the concurrency cap, the deployed-capital cap,
/// or insufficient cash. `Some(usd)` is always `> 0.0`.
pub fn position_size_usd(config: &PositionSizingConfig, portfolio: &Portfolio, mint: &str, decision: Decision, position_multiplier: f64) -> Option<f64> {
    if !matches!(decision, Decision::ConfirmedEntry | Decision::ProbeEntry) {
        return None;
    }
    if portfolio.is_open(mint) {
        return None;
    }
    if portfolio.open_count() >= config.max_concurrent_positions {
        return None;
    }

    let base = config.initial_capital_usd * config.max_position_fraction;
    let desired = base * position_multiplier;
    let deploy_cap = config.initial_capital_usd * config.max_deployed_fraction;
    let remaining_deploy_capacity = (deploy_cap - portfolio.deployed_usd()).max(0.0);
    let size = desired.min(portfolio.cash_usd).min(remaining_deploy_capacity);

    if size > 0.0 {
        Some(size)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use momentum_core::domain::Venue;

    fn config() -> PositionSizingConfig {
        DEFAULT_POSITION_SIZING_CONFIG
    }

    #[test]
    fn a_confirmed_entry_sizes_to_the_full_base_fraction() {
        let portfolio = Portfolio::new(1_000.0);
        let size = position_size_usd(&config(), &portfolio, "mint-1", Decision::ConfirmedEntry, 1.0).unwrap();
        assert!((size - 100.0).abs() < 1e-9, "expected $100 (10% of $1000), got {size}");
    }

    #[test]
    fn a_probe_entry_sizes_down_by_its_multiplier() {
        let portfolio = Portfolio::new(1_000.0);
        let size = position_size_usd(&config(), &portfolio, "mint-1", Decision::ProbeEntry, 0.2).unwrap();
        assert!((size - 20.0).abs() < 1e-9, "expected $20 (20% of the $100 base), got {size}");
    }

    #[test]
    fn reject_and_observe_never_open_a_position() {
        let portfolio = Portfolio::new(1_000.0);
        assert_eq!(position_size_usd(&config(), &portfolio, "mint-1", Decision::Reject, 0.0), None);
        assert_eq!(position_size_usd(&config(), &portfolio, "mint-1", Decision::Observe, 0.0), None);
    }

    #[test]
    fn an_already_open_mint_is_never_sized_again() {
        let mut portfolio = Portfolio::new(1_000.0);
        portfolio.open_position("mint-1".to_string(), Venue::Pump, 100.0, 1_000, Decision::ConfirmedEntry, 0);
        assert_eq!(position_size_usd(&config(), &portfolio, "mint-1", Decision::ConfirmedEntry, 1.0), None);
    }

    #[test]
    fn the_concurrency_cap_blocks_a_sixth_position() {
        let mut portfolio = Portfolio::new(10_000.0); // plenty of cash and deploy headroom
        for i in 0..5 {
            portfolio.open_position(format!("mint-{i}"), Venue::Pump, 100.0, 1_000, Decision::ConfirmedEntry, 0);
        }
        assert_eq!(portfolio.open_count(), 5);
        assert_eq!(position_size_usd(&config(), &portfolio, "mint-new", Decision::ConfirmedEntry, 1.0), None);
    }

    #[test]
    fn the_deployed_capital_cap_is_independent_of_the_concurrency_cap() {
        // max_deployed_fraction is 0.5 of $1000 = $500. Two $250 positions
        // (still well under the 5-position concurrency cap) already hit it.
        let mut portfolio = Portfolio::new(1_000.0);
        portfolio.open_position("mint-1".to_string(), Venue::Pump, 250.0, 1_000, Decision::ConfirmedEntry, 0);
        portfolio.open_position("mint-2".to_string(), Venue::Pump, 250.0, 1_000, Decision::ConfirmedEntry, 0);
        assert_eq!(portfolio.open_count(), 2);
        assert_eq!(position_size_usd(&config(), &portfolio, "mint-3", Decision::ConfirmedEntry, 1.0), None);
    }

    #[test]
    fn a_position_is_capped_by_remaining_cash_not_just_the_base_fraction() {
        let mut portfolio = Portfolio::new(1_000.0);
        portfolio.cash_usd = 30.0; // less than the $100 base
        let size = position_size_usd(&config(), &portfolio, "mint-1", Decision::ConfirmedEntry, 1.0).unwrap();
        assert!((size - 30.0).abs() < 1e-9, "expected sizing to be capped at the available $30 cash, got {size}");
    }

    #[test]
    fn zero_remaining_cash_opens_nothing() {
        let mut portfolio = Portfolio::new(1_000.0);
        portfolio.cash_usd = 0.0;
        assert_eq!(position_size_usd(&config(), &portfolio, "mint-1", Decision::ConfirmedEntry, 1.0), None);
    }
}
