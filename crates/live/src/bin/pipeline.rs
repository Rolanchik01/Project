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

use std::collections::HashMap;

use momentum_core::adapter_contract::{AdapterRegistry, VenueAdapter};
use momentum_core::dedup::StreamDeduplicator;
use momentum_core::domain::ReplayState;
use momentum_core::recorder::NdjsonRecorder;
use momentum_core::risk_engine;
use momentum_core::scoring_config::DEFAULT_SCORING_CONFIG;
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
use momentum_pump::adapter::{Candidate as PumpCandidate, PumpAdapter};
use momentum_pump::PUMP_PROGRAM_ID;
use momentum_pumpswap::adapter::{Candidate as PumpSwapCandidate, PumpSwapAdapter};
use momentum_pumpswap::PUMPSWAP_PROGRAM_ID;
use momentum_token2022::inspect_mint;
use solana_pubkey::Pubkey;
use tokio::sync::mpsc;

const PROGRAM_VERSION: &str = "2026-08-stage1";
const CHANNEL_CAPACITY: usize = 1024;
const HTTP_RPC_URL: &str = "https://api.mainnet-beta.solana.com";
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
        let snapshot = risk_engine::apply_event(&mut self.replay_state, &event, &DEFAULT_SCORING_CONFIG);
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
            PumpCandidate::Trade { mint, .. } => {
                let curve_key = PumpAdapter::bonding_curve_pda(mint);
                let Some(curve) = self.pump_adapter.curve(&curve_key) else {
                    eprintln!("pipeline: no cached bonding curve yet for mint {mint}, dropping trade");
                    return;
                };
                let Some(price) = self.sol_usd_price else {
                    eprintln!("pipeline: no SOL/USD price yet, dropping Pump trade for {mint}");
                    return;
                };
                if let Some(event) = ingest_pump_trade(&candidate, curve, price, &ctx) {
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
                    self.apply_and_record(event);
                }
            }
            PumpSwapCandidate::Trade { pool, .. } => {
                self.ingest_pumpswap_with_pool(pool, &ctx, ingest_pumpswap_trade, &candidate);
            }
            PumpSwapCandidate::Deposit { pool, .. } => {
                self.ingest_pumpswap_with_pool(pool, &ctx, ingest_pumpswap_deposit, &candidate);
            }
            PumpSwapCandidate::Withdraw { pool, .. } => {
                self.ingest_pumpswap_with_pool(pool, &ctx, ingest_pumpswap_withdraw, &candidate);
            }
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
        let update = match result.outcome {
            Ok(update) => update,
            Err(e) => {
                eprintln!("pipeline: getAccountInfo for mint {} failed ({e}), dropping TokenCreated", result.mint);
                return;
            }
        };
        match inspect_mint(&update.data) {
            Ok(flags) => {
                if let Some(event) = ingest_pump_token_created(&candidate, flags, &ctx) {
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
