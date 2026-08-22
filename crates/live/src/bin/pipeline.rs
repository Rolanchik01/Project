//! The full live pipeline this crate's doc comment has been flagging as
//! not built: `logsSubscribe` (Pump + PumpSwap) and `accountSubscribe`
//! (Pyth SOL/USD price, plus dynamically watched bonding curves/pools —
//! mints are deliberately not `accountSubscribe`-watched, see below)
//! feeding real `core::domain::Event`s through `risk_engine::apply_event`
//! into an NDJSON recording, live against mainnet.
//!
//! Run with `cargo run --bin pipeline` (writes to `./recordings/pipeline.ndjson`,
//! ctrl-c to stop). Verified live: real `TokenCreated`/`Buy`/`Sell`/
//! `PoolCreated`/`LiquidityAdded`/`LiquidityRemoved`/`Graduation` events
//! observed and scored end to end.
//!
//! `creator_history_score`/`buyer_cluster_id`/`buyer_quality`/
//! `seller_cluster_id` (Group E — `momentum_reputation::CreatorLedger`/
//! `TraderLedger`) are populated here too, not left `None` — see those
//! types' module doc comments for what each represents and, importantly,
//! what each deliberately does *not* attempt (a historical on-chain crawl
//! for creators, a funding-source graph check for wallets) and why.
//! Verified live: real repeat creators scored from this process's own
//! observed history, and real same-slot co-buys on the same mint sharing a
//! cluster id (`recordings/creator_ledger.ndjson`/`trader_ledger.ndjson`
//! are this pipeline's own persisted fact logs, separate from
//! `pipeline.ndjson` — see those types' `load`/persistence doc comments
//! for why `core::domain::Event`'s own log can't be replayed to rebuild
//! them).
//!
//! A mint's Token-2022 flags are fetched via a one-shot HTTPS
//! `getAccountInfo` call (`momentum_live::rpc_fetch::fetch_account_with_retry`),
//! not `accountSubscribe` — verified live that `accountSubscribe` delivers
//! no initial snapshot (only notifications on *subsequent* changes), and a
//! freshly created mint's last on-chain write is frequently its own
//! creation instruction, so subscribing to it can go the entire process
//! lifetime without a single notification. The fetch runs in a spawned
//! task (not inline in the event loop) so one slow RPC round-trip can't
//! stall processing of other, unrelated log events arriving in the
//! meantime. It's attempted up to `MINT_FETCH_MAX_ATTEMPTS` times total on
//! `AccountNotFound` specifically — also verified live: the public
//! multi-node RPC endpoint can serve this HTTP call from a backend that
//! briefly lags the one that served the `logsSubscribe` notification,
//! making a mint fetched immediately after its own creation event come
//! back not-found even though it demonstrably exists (see
//! `rpc_fetch::fetch_account_with_retry`'s doc comment for the live
//! numbers this was measured against).
//!
//! Deliberate simplifications, not yet built:
//! - A mint's Token-2022 flags are inspected exactly once, from that
//!   single fetch right after its `TokenCreated` candidate arrives — a
//!   later change to its extensions (e.g. the freeze authority raising
//!   `transfer_fee_bps` post-launch) is the same known gap the README has
//!   flagged since Stage 1, not newly introduced or newly closed here.
//! - A Pump `Trade` candidate is only priced once this process has
//!   received at least one `accountSubscribe` update for that mint's
//!   bonding curve (subscribed the moment its `TokenCreated` arrived) — a
//!   trade landing in the same slot as creation, before that first curve
//!   snapshot arrives, is dropped (logged) rather than guessed at. Same
//!   for a PumpSwap `Trade`/`Deposit`/`Withdraw` needing the pool's own
//!   `Pool` account.
//! - Every watched mint/curve/pool accumulates for the life of the
//!   process — nothing is ever `Unwatch`ed. Fine for a research run, not
//!   for a long-lived production process watching thousands of tokens.
//! - One process, one connection per subscription, no fallback RPC
//!   provider — same as every other binary in this crate.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use momentum_core::adapter_contract::{AdapterRegistry, VenueAdapter};
use momentum_core::dedup::StreamDeduplicator;
use momentum_core::domain::{Event, EventPayload, ReplayState, Venue};
use momentum_core::recorder::NdjsonRecorder;
use momentum_core::risk_engine::{self, Decision, RiskSnapshot};
use momentum_core::scoring_config::DEFAULT_SCORING_CONFIG;
use momentum_ingest::price::{is_wrapped_sol, lamports_to_usd, usd_to_lamports};
use momentum_ingest::price_feed::{decode_sol_usd_update, sol_usd_price_account};
use momentum_ingest::{
    ingest_pump_graduated, ingest_pump_token_created, ingest_pump_trade, ingest_pumpswap_deposit,
    ingest_pumpswap_pool_created, ingest_pumpswap_trade, ingest_pumpswap_withdraw, EventContext,
};
use momentum_live::account_notification::AccountUpdate;
use momentum_live::account_watcher::{self, WatchCommand, WatcherConfig};
use momentum_live::listener::{self, ListenerConfig};
use momentum_live::logs::RawLogEvent;
use momentum_live::rpc_fetch::fetch_account_with_retry;
use momentum_portfolio::{position_size_usd, ClosedTrade, ExitReason, Portfolio, DEFAULT_POSITION_SIZING_CONFIG};
use momentum_pump::adapter::{Candidate as PumpCandidate, PumpAdapter};
use momentum_pump::PUMP_PROGRAM_ID;
use momentum_pumpswap::adapter::{Candidate as PumpSwapCandidate, PumpSwapAdapter};
use momentum_pumpswap::PUMPSWAP_PROGRAM_ID;
use momentum_reputation::{CreatorLedger, TraderLedger};
use momentum_token2022::inspect_mint;
use serde::Serialize;
use solana_pubkey::Pubkey;
use tokio::sync::mpsc;

const PROGRAM_VERSION: &str = "2026-08-stage1";
const CHANNEL_CAPACITY: usize = 1024;
const HTTP_RPC_URL: &str = "https://api.mainnet-beta.solana.com";
const PAPER_TRADES_PATH: &str = "recordings/paper_trades.ndjson";
const CREATOR_LEDGER_PATH: &str = "recordings/creator_ledger.ndjson";
const TRADER_LEDGER_PATH: &str = "recordings/trader_ledger.ndjson";
/// See `rpc_fetch::fetch_account_with_retry`'s doc comment: clears the
/// replica-lag race verified live between `logsSubscribe`'s WebSocket
/// backend and whichever backend serves a given HTTP `getAccountInfo`
/// request against the same public multi-node RPC endpoint.
const MINT_FETCH_MAX_ATTEMPTS: u32 = 3;
const MINT_FETCH_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(300);

/// Outcome of a spawned one-shot `getAccountInfo` fetch for a mint whose
/// `TokenCreated` candidate is waiting in `pending_token_created`. `Err`
/// carries a human-readable reason (the fetch failed, or the account
/// didn't parse as a Token-2022 mint) purely for logging — there is no
/// retry, matching this pipeline's established fail-closed-and-log stance
/// for every other "can't complete this event" case (see
/// `ingest_pumpswap_with_pool`, the bonding-curve-not-cached-yet branch
/// below).
struct MintFetchResult {
    mint: Pubkey,
    outcome: Result<AccountUpdate, String>,
}

/// One entry in `PAPER_TRADES_PATH` — Group G's own accumulating trade
/// record, separate from `pipeline.ndjson` (which records every scored
/// `core::domain::Event`, not just the ones that became a paper trade) and
/// from `creator_ledger.ndjson`/`trader_ledger.ndjson` (reputation facts,
/// not trade outcomes). This is the log `bin/paper_trade_report.rs`
/// (Group G.8) reads to answer "is this strategy actually profitable" —
/// see that binary's doc comment for why it reads *this* accumulated log
/// rather than replaying `pipeline.ndjson` after the fact.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum PaperTradeLogEntry {
    Opened { mint: String, venue: String, entry_usd: f64, decision: String, opened_at_ns: u64 },
    Closed {
        mint: String,
        venue: String,
        entry_usd: f64,
        exit_usd: f64,
        pnl_usd: f64,
        pnl_pct: f64,
        decision: String,
        reason: String,
        opened_at_ns: u64,
        closed_at_ns: u64,
    },
}

fn venue_str(venue: Venue) -> &'static str {
    match venue {
        Venue::Pump => "pump",
        Venue::PumpSwap => "pumpswap",
        Venue::RaydiumCpmm => "raydium_cpmm",
        Venue::RaydiumClmm => "raydium_clmm",
        Venue::RaydiumLaunchLab => "raydium_launch_lab",
        Venue::MeteoraDlmm => "meteora_dlmm",
    }
}

fn append_paper_trade_log(entry: &PaperTradeLogEntry) {
    use std::io::Write;
    let line = match serde_json::to_string(entry) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("pipeline: failed to serialize paper trade log entry: {e}");
            return;
        }
    };
    let result = (|| -> std::io::Result<()> {
        if let Some(parent) = std::path::Path::new(PAPER_TRADES_PATH).parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(PAPER_TRADES_PATH)?;
        writeln!(file, "{line}")
    })();
    if let Err(e) = result {
        eprintln!("pipeline: failed to append paper trade log: {e}");
    }
}

struct Pipeline {
    replay_state: ReplayState,
    dedup: StreamDeduplicator,
    recorder: NdjsonRecorder,
    registry: AdapterRegistry,
    pump_adapter: PumpAdapter,
    pumpswap_adapter: PumpSwapAdapter,
    /// Pump `TokenCreated` candidates waiting on their mint account's
    /// Token-2022 inspection (a spawned `rpc_fetch::fetch_account` task,
    /// see `MintFetchResult`) before they can become a full `Event`. An
    /// entry is only ever removed by `handle_mint_fetch_result`, so a
    /// spawned `spawn_mint_fetch` task that panics before its
    /// `mint_fetch_tx.send` (no panicking code is currently reachable on
    /// that path — see `rpc_fetch::fetch_account`/`fetch_account_with_retry`)
    /// would leak its entry for the life of the process. Same tolerance as
    /// the "everything watched, nothing ever `Unwatch`ed" gap below: not
    /// acceptable for a long-lived production process, not worth guarding
    /// against for this research pipeline.
    pending_token_created: HashMap<Pubkey, (PumpCandidate, EventContext)>,
    sol_usd_price: Option<f64>,
    watch_tx: mpsc::Sender<WatchCommand>,
    http_client: reqwest::Client,
    mint_fetch_tx: mpsc::Sender<MintFetchResult>,
    /// Creator/mint/pool history this process has itself observed — see
    /// `momentum_reputation`'s crate doc comment for why it's built this
    /// way instead of from a historical on-chain crawl. Persisted to
    /// `CREATOR_LEDGER_PATH` and reloaded on startup, so reputation earned
    /// in a previous run survives a restart.
    creator_ledger: CreatorLedger,
    /// Buyer/seller clustering and quality (Group E.2) — see
    /// `momentum_reputation::wallet`'s module doc comment. Persisted to
    /// `TRADER_LEDGER_PATH`; slot-clustering itself is not (see that
    /// module's doc comment on `slot_clusters`).
    trader_ledger: TraderLedger,
    /// Paper positions and realized P&L (Group G.6/G.7) — see
    /// `momentum_portfolio`'s crate doc comment for the sizing/exit
    /// policy. Not persisted across restarts: unlike the reputation
    /// ledgers, an open paper position depends on live-cached
    /// curve/pool state (`pump_adapter`/`pumpswap_adapter`) that itself
    /// resets on restart, so resuming a position without that state would
    /// mean tracking P&L against a price this process can no longer
    /// verify. `PAPER_TRADES_PATH` is the durable record of every trade
    /// this portfolio ever made, across every run.
    portfolio: Portfolio,
    /// `mint -> pool` for every PumpSwap pool this process has decoded —
    /// needed to value (or enter) a position through PumpSwap, since
    /// `PumpSwapAdapter` itself is keyed by pool address, not mint (see
    /// its doc comment). Populated the moment a `PoolCreated` event is
    /// scored, same point `creator_ledger.observe_pool_created` already
    /// runs.
    mint_to_pool: HashMap<String, Pubkey>,
    /// Pool addresses whose two reserve token accounts have already been
    /// `Watch`ed — set once per pool the first time `handle_account_update`
    /// sees that pool's own account decode successfully, so a later
    /// re-update of the same `Pool` account doesn't re-issue the same two
    /// `Watch` commands.
    watched_pool_reserves: HashSet<Pubkey>,
}

impl Pipeline {
    fn apply_and_record(&mut self, event: momentum_core::domain::Event) {
        if let Err(mismatch) = self.registry.assert_compatible(&event) {
            eprintln!("pipeline: HALT — {mismatch}");
            return;
        }
        if !self.dedup.admit(&event) {
            return;
        }
        let is_token_created = matches!(event.payload, EventPayload::TokenCreated { .. });
        let snapshot = risk_engine::apply_event(&mut self.replay_state, &event, &DEFAULT_SCORING_CONFIG);
        // Direct evidence against this mint's creator, independent of
        // whether it ever reaches a PumpSwap pool — see
        // `CreatorLedger::observe_hard_blocked`'s doc comment.
        if is_token_created && !snapshot.hard_blocks.is_empty() {
            self.creator_ledger.observe_hard_blocked(&event.mint);
        }
        if let Err(e) = self.recorder.record(&event) {
            eprintln!("pipeline: failed to record event: {e}");
        }
        println!(
            "[{}] {:?} decision={:?} safety={} demand={} exit_liquidity_usd={:.2}",
            event.mint,
            event.kind(),
            snapshot.decision,
            snapshot.safety_score,
            snapshot.demand_score,
            snapshot.exit_liquidity_usd
        );
        self.handle_position_lifecycle(&event, &snapshot);
    }

    /// Checks a currently-held position for an exit on every event for its
    /// mint (emergency exit first, then take-profit/stop-loss), or — for a
    /// mint with no open position — whether this event's freshly computed
    /// `snapshot` now warrants opening one. Never both in the same call:
    /// a mint already held is only ever evaluated for exit here, not
    /// resized or re-entered (`position_size_usd` would refuse a mint
    /// that's already open anyway, but returning early makes the "one
    /// position at a time per mint" invariant explicit rather than
    /// incidental).
    fn handle_position_lifecycle(&mut self, event: &Event, snapshot: &RiskSnapshot) {
        let mint: &str = &event.mint;

        if self.portfolio.is_open(mint) {
            let emergency = !snapshot.hard_blocks.is_empty()
                || matches!(&event.payload, EventPayload::LiquidityRemoved { all_liquidity_removed: true, .. });
            if emergency {
                // Best available value, or a total loss if none can be
                // quoted at all (e.g. the pool's reserves just went to
                // zero) — matches `Portfolio`'s own doc comment on why an
                // emergency exit is allowed to close at 0.0 rather than
                // block on a quote that may no longer be obtainable.
                let value = self.current_position_value_usd(mint).unwrap_or(0.0);
                if let Some(trade) = self.portfolio.close_position(mint, value, ExitReason::EmergencyExit, event.observed_at_ns) {
                    self.log_paper_trade_closed(&trade);
                }
            } else if let Some(value) = self.current_position_value_usd(mint) {
                if let Some(reason) = self.portfolio.check_exit(mint, value, &DEFAULT_POSITION_SIZING_CONFIG) {
                    if let Some(trade) = self.portfolio.close_position(mint, value, reason, event.observed_at_ns) {
                        self.log_paper_trade_closed(&trade);
                    }
                }
            }
            return;
        }

        self.try_open_position(mint, event.venue, snapshot.decision, snapshot.position_multiplier, event.observed_at_ns);
    }

    /// Sizes and opens a new paper position for `mint` if `decision`
    /// warrants one and a real entry quote can be obtained. If
    /// `position_size_usd` itself says no entry is warranted (no cash, no
    /// capacity, decision doesn't call for a position) this returns silently
    /// — that's the overwhelming majority of scored events under normal
    /// Observe/Reject conditions, and logging it would be pure noise. Once a
    /// size has actually been computed, every further drop reason (missing
    /// SOL price, invalid mint, no cached curve/pool state, quote failure)
    /// is logged via `eprintln!`, since at that point a real entry was
    /// warranted but couldn't be completed.
    fn try_open_position(&mut self, mint: &str, venue: Venue, decision: Decision, position_multiplier: f64, opened_at_ns: u64) {
        let Some(size_usd) = position_size_usd(&DEFAULT_POSITION_SIZING_CONFIG, &self.portfolio, mint, decision, position_multiplier) else {
            return;
        };
        let Some(sol_price) = self.sol_usd_price else {
            eprintln!("pipeline: {decision:?} sized {mint} at ${size_usd:.2} but no SOL/USD price yet, dropping entry");
            return;
        };
        let Some(lamports_in) = usd_to_lamports(size_usd, sol_price) else {
            eprintln!("pipeline: {decision:?} sized {mint} at ${size_usd:.2} but could not convert to lamports at sol_price={sol_price}, dropping entry");
            return;
        };

        let token_amount = match venue {
            Venue::Pump => {
                let Ok(mint_pk) = Pubkey::from_str(mint) else {
                    eprintln!("pipeline: {decision:?} sized {mint} at ${size_usd:.2} but mint is not a valid pubkey, dropping entry");
                    return;
                };
                let curve_key = PumpAdapter::bonding_curve_pda(&mint_pk);
                let Some(curve) = self.pump_adapter.curve(&curve_key) else {
                    eprintln!("pipeline: {decision:?} sized {mint} at ${size_usd:.2} but no cached bonding curve yet, dropping entry");
                    return;
                };
                match momentum_pump::quote_buy_exact_quote_in(curve, lamports_in) {
                    Ok(t) => t,
                    Err(e) => {
                        eprintln!("pipeline: {decision:?} sized {mint} at ${size_usd:.2} but Pump quote failed ({e:?}), dropping entry");
                        return;
                    }
                }
            }
            Venue::PumpSwap => {
                let Some(&pool_addr) = self.mint_to_pool.get(mint) else {
                    eprintln!("pipeline: {decision:?} sized {mint} at ${size_usd:.2} but no known PumpSwap pool yet, dropping entry");
                    return;
                };
                match self.quote_pumpswap_buy_with_sol(&pool_addr, lamports_in) {
                    Some(t) => t,
                    None => {
                        eprintln!("pipeline: {decision:?} sized {mint} at ${size_usd:.2} but PumpSwap quote failed (reserves not yet known or venue not priceable), dropping entry");
                        return;
                    }
                }
            }
            _ => {
                eprintln!("pipeline: {decision:?} sized {mint} at ${size_usd:.2} but venue {venue:?} is not tradeable, dropping entry");
                return;
            }
        };

        self.portfolio.open_position(mint.to_string(), venue, size_usd, token_amount, decision, opened_at_ns);
        append_paper_trade_log(&PaperTradeLogEntry::Opened {
            mint: mint.to_string(),
            venue: venue_str(venue).to_string(),
            entry_usd: size_usd,
            decision: decision.as_str().to_string(),
            opened_at_ns,
        });
        println!("[{mint}] PAPER OPEN venue={venue:?} entry_usd={size_usd:.2} decision={}", decision.as_str());
    }

    fn log_paper_trade_closed(&self, trade: &ClosedTrade) {
        append_paper_trade_log(&PaperTradeLogEntry::Closed {
            mint: trade.mint.clone(),
            venue: venue_str(trade.venue).to_string(),
            entry_usd: trade.entry_usd,
            exit_usd: trade.exit_usd,
            pnl_usd: trade.pnl_usd,
            pnl_pct: trade.pnl_pct,
            decision: trade.decision.as_str().to_string(),
            reason: trade.reason.as_str().to_string(),
            opened_at_ns: trade.opened_at_ns,
            closed_at_ns: trade.closed_at_ns,
        });
        println!(
            "[{}] PAPER CLOSE venue={:?} reason={:?} entry_usd={:.2} exit_usd={:.2} pnl_usd={:.2} ({:+.1}%)",
            trade.mint,
            trade.venue,
            trade.reason,
            trade.entry_usd,
            trade.exit_usd,
            trade.pnl_usd,
            trade.pnl_pct * 100.0
        );
    }

    /// A real, current-state quote for `mint`'s open position, trying
    /// PumpSwap first and falling back to Pump — not a fixed choice based
    /// on which venue the position was opened through, because a mint
    /// opened pre-graduation (Pump) can graduate to a PumpSwap pool while
    /// still held, at which point the bonding curve stops trading and only
    /// the pool has a live price. `None` if neither has cached state to
    /// quote against yet (the position is left open, re-checked on the
    /// next event for this mint).
    fn current_position_value_usd(&self, mint: &str) -> Option<f64> {
        let position = self.portfolio.open_position_for(mint)?;
        let sol_price = self.sol_usd_price?;

        if let Some(&pool_addr) = self.mint_to_pool.get(mint) {
            if let Some(lamports_out) = self.quote_pumpswap_sell_for_sol(&pool_addr, position.token_amount) {
                return lamports_to_usd(lamports_out, sol_price);
            }
        }

        let mint_pk = Pubkey::from_str(mint).ok()?;
        let curve_key = PumpAdapter::bonding_curve_pda(&mint_pk);
        let curve = self.pump_adapter.curve(&curve_key)?;
        let lamports_out = momentum_pump::quote_sell(curve, position.token_amount).ok()?;
        lamports_to_usd(lamports_out, sol_price)
    }

    /// Real quote for spending `sol_lamports_in` to buy `pool`'s non-SOL
    /// side, using `pool`'s currently cached reserves. Which of PumpSwap's
    /// `quote_buy_exact_quote_in`/`quote_sell` computes that depends on
    /// which side of the pool SOL sits on — verified real pools split
    /// roughly evenly (see `crates/ingest/src/pumpswap.rs`'s module doc
    /// comment), so both cases are real, not a hypothetical one this
    /// mirrors:
    /// - SOL is `quote_mint`: spending quote to get base is exactly what
    ///   `quote_buy_exact_quote_in` computes.
    /// - SOL is `base_mint`: spending base to get quote is exactly what
    ///   `quote_sell` computes (it doesn't care that the base side happens
    ///   to be SOL here rather than the token) — no reserve reordering
    ///   needed, just the other function.
    fn quote_pumpswap_buy_with_sol(&self, pool_addr: &Pubkey, sol_lamports_in: u64) -> Option<u64> {
        let pool = self.pumpswap_adapter.pool(pool_addr)?;
        let (base_reserves, quote_reserves) = self.pumpswap_adapter.known_reserves(pool_addr)?;
        if is_wrapped_sol(&pool.quote_mint) {
            momentum_pumpswap::quote_buy_exact_quote_in(pool, base_reserves, quote_reserves, sol_lamports_in).ok()
        } else if is_wrapped_sol(&pool.base_mint) {
            momentum_pumpswap::quote_sell(pool, base_reserves, quote_reserves, sol_lamports_in).ok()
        } else {
            None
        }
    }

    /// Real quote for selling `token_amount_in` (the tracked non-SOL side
    /// of `pool`) back for SOL — the mirror image of
    /// `quote_pumpswap_buy_with_sol`, same reasoning, functions swapped.
    fn quote_pumpswap_sell_for_sol(&self, pool_addr: &Pubkey, token_amount_in: u64) -> Option<u64> {
        let pool = self.pumpswap_adapter.pool(pool_addr)?;
        let (base_reserves, quote_reserves) = self.pumpswap_adapter.known_reserves(pool_addr)?;
        if is_wrapped_sol(&pool.quote_mint) {
            momentum_pumpswap::quote_sell(pool, base_reserves, quote_reserves, token_amount_in).ok()
        } else if is_wrapped_sol(&pool.base_mint) {
            momentum_pumpswap::quote_buy_exact_quote_in(pool, base_reserves, quote_reserves, token_amount_in).ok()
        } else {
            None
        }
    }

    fn ctx(&self, raw: &RawLogEvent) -> EventContext {
        EventContext {
            id: format!("{}:{}", raw.signature, raw.log_index),
            slot: raw.slot,
            observed_at_ns: now_ns(),
            signature: raw.signature.clone(),
            instruction_index: raw.log_index,
            program_version: PROGRAM_VERSION.to_string(),
        }
    }

    async fn handle_pump_log(&mut self, raw: RawLogEvent) {
        let Some(candidate) = self.pump_adapter.decode(&raw.data) else { return };
        let ctx = self.ctx(&raw);
        match &candidate {
            PumpCandidate::TokenCreated { mint, .. } => {
                // The bonding curve's reserves change on every trade, so
                // `accountSubscribe` is the right tool for it. The mint
                // itself is not subscribed here — see this file's crate
                // doc comment: `accountSubscribe` delivers no initial
                // snapshot, and a fresh mint's last write is frequently
                // its own creation instruction, so it's fetched once
                // instead (`spawn_mint_fetch` below).
                let mint = *mint;
                let curve = PumpAdapter::bonding_curve_pda(&mint);
                let _ = self.watch_tx.send(WatchCommand::Watch(curve)).await;
                self.pending_token_created.insert(mint, (candidate, ctx));
                self.spawn_mint_fetch(mint);
            }
            PumpCandidate::Trade { mint, user, is_buy, .. } => {
                // Recorded into the wallet ledger unconditionally, before
                // the cache/price checks below can drop this trade — a
                // decoded Trade candidate is a real on-chain trade this
                // wallet just made, whether or not this process's local
                // bonding-curve cache or SOL/USD price happens to be warm
                // yet. Same "we really observed this, independent of
                // whether we can also score it" stance as
                // `observe_creation` in `handle_mint_fetch_result`.
                let (cluster_id, buyer_quality) = self.trader_ledger.observe_trade(&user.to_string(), &mint.to_string(), ctx.slot, *is_buy);

                let curve_key = PumpAdapter::bonding_curve_pda(mint);
                let Some(curve) = self.pump_adapter.curve(&curve_key) else {
                    eprintln!("pipeline: no cached bonding curve yet for mint {mint}, dropping trade");
                    return;
                };
                let Some(price) = self.sol_usd_price else {
                    eprintln!("pipeline: no SOL/USD price yet, dropping Pump trade for {mint}");
                    return;
                };
                if let Some(event) = ingest_pump_trade(&candidate, curve, price, Some(cluster_id), buyer_quality, &ctx) {
                    self.apply_and_record(event);
                }
            }
            PumpCandidate::Graduated { .. } => {
                if let Some(event) = ingest_pump_graduated(&candidate, &ctx) {
                    self.apply_and_record(event);
                }
            }
        }
    }

    async fn handle_pumpswap_log(&mut self, raw: RawLogEvent) {
        let Some(candidate) = self.pumpswap_adapter.decode(&raw.data) else { return };
        let ctx = self.ctx(&raw);
        match &candidate {
            PumpSwapCandidate::PoolCreated { pool, .. } => {
                let _ = self.watch_tx.send(WatchCommand::Watch(*pool)).await;
                let Some(price) = self.sol_usd_price else {
                    eprintln!("pipeline: no SOL/USD price yet, dropping PumpSwap pool creation for {pool}");
                    return;
                };
                if let Some(event) = ingest_pumpswap_pool_created(&candidate, price, &ctx) {
                    // Links this mint to its pool in the creator ledger
                    // before scoring, so a later drain on this exact pool
                    // is attributable back to this mint's creator. Gated
                    // on `sol_usd_price` being known (same as the event
                    // itself) rather than duplicating `ingest_pumpswap_
                    // pool_created`'s own SOL-side resolution — in
                    // practice the price is already known well before any
                    // real pool creation (it's watched from process
                    // start), so this hasn't been observed to matter.
                    self.creator_ledger.observe_pool_created(&event.mint, &pool.to_string());
                    self.mint_to_pool.insert(event.mint.clone(), *pool);
                    self.apply_and_record(event);
                }
            }
            PumpSwapCandidate::Trade { pool, user, is_buy, .. } => {
                // Keyed by pool address, not the tracked mint: which side
                // of the pool is SOL isn't resolved until the pool's
                // cached state is looked up inside ingest_pumpswap_trade,
                // but a pool address is already a stable per-token
                // identifier on its own (see TraderLedger's doc comment —
                // it doesn't care what a "distinct instrument" key means
                // beyond being distinct and stable). Recorded before
                // handle_pumpswap_trade's own cache/price checks below,
                // same "really happened, independent of whether we can
                // also score it" stance as the Pump Trade arm above.
                let (cluster_id, buyer_quality) = self.trader_ledger.observe_trade(&user.to_string(), &pool.to_string(), ctx.slot, *is_buy);
                self.handle_pumpswap_trade(pool, &candidate, cluster_id, buyer_quality, &ctx);
            }
            PumpSwapCandidate::Deposit { pool, .. } => {
                self.ingest_pumpswap_with_pool(pool, &ctx, ingest_pumpswap_deposit, &candidate);
            }
            PumpSwapCandidate::Withdraw { pool, .. } => {
                self.ingest_pumpswap_with_pool(pool, &ctx, ingest_pumpswap_withdraw, &candidate);
            }
        }
    }

    fn handle_pumpswap_trade(&mut self, pool_address: &Pubkey, candidate: &PumpSwapCandidate, cluster_id: String, buyer_quality: f64, ctx: &EventContext) {
        let Some(pool) = self.pumpswap_adapter.pool(pool_address) else {
            eprintln!("pipeline: no cached pool state yet for {pool_address}, dropping event");
            return;
        };
        let Some(price) = self.sol_usd_price else {
            eprintln!("pipeline: no SOL/USD price yet, dropping PumpSwap event for {pool_address}");
            return;
        };
        if let Some(event) = ingest_pumpswap_trade(candidate, pool, price, Some(cluster_id), buyer_quality, ctx) {
            self.apply_and_record(event);
        }
    }

    fn ingest_pumpswap_with_pool(
        &mut self,
        pool_address: &Pubkey,
        ctx: &EventContext,
        ingest: impl FnOnce(&PumpSwapCandidate, &momentum_pumpswap::Pool, f64, &EventContext) -> Option<momentum_core::domain::Event>,
        candidate: &PumpSwapCandidate,
    ) {
        let Some(pool) = self.pumpswap_adapter.pool(pool_address) else {
            eprintln!("pipeline: no cached pool state yet for {pool_address}, dropping event");
            return;
        };
        let Some(price) = self.sol_usd_price else {
            eprintln!("pipeline: no SOL/USD price yet, dropping PumpSwap event for {pool_address}");
            return;
        };
        if let Some(event) = ingest(candidate, pool, price, ctx) {
            // A real near-total drain is bad signal for this pool's mint's
            // creator regardless of who executed the withdrawal (see
            // `CreatorLedger`'s crate doc comment on why this doesn't try
            // to attribute the withdrawer specifically).
            if let EventPayload::LiquidityRemoved { all_liquidity_removed, .. } = &event.payload {
                self.creator_ledger.observe_liquidity_removed(&pool_address.to_string(), *all_liquidity_removed);
            }
            self.apply_and_record(event);
        }
    }

    async fn handle_account_update(&mut self, update: AccountUpdate) {
        if let Some(price_update) = decode_sol_usd_update(&update.owner, &update.data) {
            self.sol_usd_price = Some(price_update.price);
            return;
        }

        let pubkey = update.pubkey;
        let _ = self.pump_adapter.apply_update(&momentum_pump::adapter::AccountUpdate { pubkey, data: update.data.clone() });
        let _ = self.pumpswap_adapter.apply_update(&momentum_pumpswap::adapter::AccountUpdate { pubkey, data: update.data });

        // `pubkey` is a *pool's own* address only right after its `Pool`
        // account itself decodes successfully (`PumpSwapAdapter::pool`
        // keys by exactly that address) — a reserve token account update
        // has a different pubkey and will never match here. First time
        // this fires for a given pool, watch its two real reserve
        // accounts too; `PumpSwapAdapter::apply_update` already knows how
        // to recognize and cache their balances once watched (see its doc
        // comment), it just needs someone to ask for them.
        if !self.watched_pool_reserves.contains(&pubkey) {
            if let Some(pool) = self.pumpswap_adapter.pool(&pubkey) {
                let _ = self.watch_tx.send(WatchCommand::Watch(pool.pool_base_token_account)).await;
                let _ = self.watch_tx.send(WatchCommand::Watch(pool.pool_quote_token_account)).await;
                self.watched_pool_reserves.insert(pubkey);
            }
        }
    }

    /// Fires off the one-shot `getAccountInfo` fetch for a just-created
    /// mint in a separate task, so waiting on the RPC round-trip can't
    /// stall processing of other log events already queued up. The result
    /// comes back through `mint_fetch_tx`/`mint_fetch_rx`, handled by
    /// `handle_mint_fetch_result` on the main event loop like every other
    /// event source.
    fn spawn_mint_fetch(&self, mint: Pubkey) {
        let client = self.http_client.clone();
        let tx = self.mint_fetch_tx.clone();
        tokio::spawn(async move {
            let outcome = fetch_account_with_retry(&client, HTTP_RPC_URL, &mint, "confirmed", MINT_FETCH_MAX_ATTEMPTS, MINT_FETCH_RETRY_DELAY)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(MintFetchResult { mint, outcome }).await;
        });
    }

    fn handle_mint_fetch_result(&mut self, result: MintFetchResult) {
        let Some((candidate, ctx)) = self.pending_token_created.remove(&result.mint) else { return };
        let PumpCandidate::TokenCreated { creator, .. } = &candidate else {
            // pending_token_created only ever holds TokenCreated candidates
            // (see handle_pump_log's TokenCreated arm, the only inserter).
            return;
        };
        // Recorded unconditionally, even if the fetch below fails or the
        // mint doesn't parse: we definitely observed this creator's real
        // on-chain CreateEvent for this mint, independent of whether we
        // could also inspect the resulting Token-2022 flags. Exactly one
        // call per mint, since pending_token_created.remove above ensures
        // this runs at most once per mint.
        let creator_history_score = self.creator_ledger.observe_creation(&creator.to_string(), &result.mint.to_string());

        let update = match result.outcome {
            Ok(update) => update,
            Err(e) => {
                eprintln!("pipeline: getAccountInfo for mint {} failed ({e}), dropping TokenCreated", result.mint);
                return;
            }
        };
        match inspect_mint(&update.data) {
            Ok(flags) => {
                if let Some(event) = ingest_pump_token_created(&candidate, flags, creator_history_score, &ctx) {
                    self.apply_and_record(event);
                }
            }
            Err(e) => {
                eprintln!("pipeline: mint account {} didn't parse as a Token-2022 mint ({e:?}), dropping TokenCreated", result.mint);
            }
        }
    }
}

fn now_ns() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("system clock is after 1970").as_nanos() as u64
}

#[tokio::main]
async fn main() {
    let recorder = NdjsonRecorder::new("recordings/pipeline.ndjson").expect("failed to create recordings directory");
    let creator_ledger = CreatorLedger::load(CREATOR_LEDGER_PATH).expect("failed to load creator ledger");
    let trader_ledger = TraderLedger::load(TRADER_LEDGER_PATH).expect("failed to load trader ledger");
    let registry = AdapterRegistry::new().register(momentum_core::domain::Venue::Pump, PROGRAM_VERSION).register(
        momentum_core::domain::Venue::PumpSwap,
        PROGRAM_VERSION,
    );

    let (pump_tx, mut pump_rx) = mpsc::channel::<RawLogEvent>(CHANNEL_CAPACITY);
    let (pumpswap_tx, mut pumpswap_rx) = mpsc::channel::<RawLogEvent>(CHANNEL_CAPACITY);
    let (account_tx, mut account_rx) = mpsc::channel::<AccountUpdate>(CHANNEL_CAPACITY);
    let (watch_tx, watch_rx) = mpsc::channel::<WatchCommand>(CHANNEL_CAPACITY);
    let (mint_fetch_tx, mut mint_fetch_rx) = mpsc::channel::<MintFetchResult>(CHANNEL_CAPACITY);

    let pump_config =
        ListenerConfig { ws_url: "wss://api.mainnet-beta.solana.com".to_string(), program_id: PUMP_PROGRAM_ID.to_string(), commitment: "confirmed".to_string() };
    let pumpswap_config = ListenerConfig {
        ws_url: "wss://api.mainnet-beta.solana.com".to_string(),
        program_id: PUMPSWAP_PROGRAM_ID.to_string(),
        commitment: "confirmed".to_string(),
    };
    let watcher_config = WatcherConfig { ws_url: "wss://api.mainnet-beta.solana.com".to_string(), commitment: "confirmed".to_string() };

    tokio::spawn(listener::run(pump_config, pump_tx));
    tokio::spawn(listener::run(pumpswap_config, pumpswap_tx));
    tokio::spawn(account_watcher::run(watcher_config, vec![sol_usd_price_account()], watch_rx, account_tx));

    let mut pipeline = Pipeline {
        replay_state: ReplayState::new(),
        dedup: StreamDeduplicator::new(),
        recorder,
        registry,
        pump_adapter: PumpAdapter::new(PROGRAM_VERSION),
        pumpswap_adapter: PumpSwapAdapter::new(PROGRAM_VERSION),
        pending_token_created: HashMap::new(),
        sol_usd_price: None,
        watch_tx,
        http_client: reqwest::Client::new(),
        mint_fetch_tx,
        creator_ledger,
        trader_ledger,
        portfolio: Portfolio::new(DEFAULT_POSITION_SIZING_CONFIG.initial_capital_usd),
        mint_to_pool: HashMap::new(),
        watched_pool_reserves: HashSet::new(),
    };

    eprintln!("pipeline: listening for live Pump + PumpSwap events, recording to recordings/pipeline.ndjson (ctrl-c to stop)...");

    loop {
        tokio::select! {
            event = pump_rx.recv() => {
                let Some(raw) = event else { break };
                pipeline.handle_pump_log(raw).await;
            }
            event = pumpswap_rx.recv() => {
                let Some(raw) = event else { break };
                pipeline.handle_pumpswap_log(raw).await;
            }
            update = account_rx.recv() => {
                let Some(update) = update else { break };
                pipeline.handle_account_update(update).await;
            }
            result = mint_fetch_rx.recv() => {
                let Some(result) = result else { break };
                pipeline.handle_mint_fetch_result(result);
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("pipeline: shutting down");
                break;
            }
        }
    }
}
