//! First slice of the live event source ("Dataplane"): a `logsSubscribe`
//! WebSocket listener over the public Solana RPC, feeding raw event bytes
//! that `momentum_pump::events::decode_event` already knows how to parse.
//!
//! `bin/pipeline.rs` is the real thing: `logsSubscribe` (Pump + PumpSwap,
//! `listener`/`logs`) and `accountSubscribe` (Pyth SOL/USD price plus
//! dynamically watched bonding curves/pools, `account_watcher`/
//! `account_notification`) both feed one process, which decides which
//! accounts to watch as new ones are discovered live, fetches a freshly
//! created mint's current state once via `rpc_fetch` instead of watching it
//! (accountSubscribe alone can miss it entirely — see that module's doc
//! comment), and wires everything through `momentum_ingest` into
//! `risk_engine::apply_event` and an NDJSON recording.
//! `bin/pump_listener.rs`/`bin/pumpswap_listener.rs` remain as smaller
//! single-venue demos of `listener::run` alone.
//!
//! Deliberate simplifications, not yet built — see `bin/pipeline.rs`'s own
//! doc comment for the current list (mint extensions inspected once at
//! creation, not re-checked later; nothing ever `Unwatch`ed; one process,
//! one connection per subscription, no fallback RPC provider).
//!
//! Reconnect (both `listener::run` and `account_watcher::run`) is
//! unconditional exponential backoff (1s, capped at 30s). No metrics, no
//! alerting on repeated failures, no fallback to a second RPC provider —
//! a real production deployment needs all of that; this is the minimum
//! that keeps a single connection alive through ordinary network blips.

pub mod account_notification;
pub mod account_watcher;
pub mod listener;
pub mod logs;
pub mod rpc_fetch;
