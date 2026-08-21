//! First slice of the live event source ("Dataplane"): a `logsSubscribe`
//! WebSocket listener over the public Solana RPC, feeding raw event bytes
//! that `momentum_pump::events::decode_event` already knows how to parse.
//!
//! Deliberately scoped narrow for this pass — proving the live pipe works
//! end to end on the real, currently-flowing chain, not yet the full
//! pipeline:
//! - Two demo binaries (`bin/pump_listener.rs`, `bin/pumpswap_listener.rs`)
//!   each run `listener::run` with a different `program_id`, proving the
//!   listener itself is venue-agnostic — but each is its own standalone
//!   process; there is no single process that watches both venues (or any
//!   other) at once yet.
//! - `logsSubscribe` (transaction logs, `listener`/`logs`) and
//!   `accountSubscribe` (`account_watcher`/`account_notification`) both
//!   exist now, but nothing connects them to `TokenCreated`/`Trade` events
//!   yet — `account_watcher::run` will watch whatever pubkeys it's told
//!   to, but no code decides *which* bonding-curve/pool/mint accounts to
//!   watch as new ones are discovered live. That decision layer, and the
//!   `apply_update` wiring it would feed, is still not built.
//! - Nothing here calls into `momentum_ingest`, `risk_engine`, or the
//!   `recorder` yet — this crate only proves raw bytes flow from the real
//!   chain into the already-tested decoder in real time. Wiring that into
//!   a full domain::Event -> risk snapshot -> NDJSON pipeline is the next
//!   piece.
//! - Reconnect is unconditional exponential backoff (1s, capped at 30s).
//!   No metrics, no alerting on repeated failures, no fallback to a
//!   second RPC provider — a real production deployment needs all of
//!   that; this is the minimum that keeps a single connection alive
//!   through ordinary network blips.

pub mod account_notification;
pub mod account_watcher;
pub mod listener;
pub mod logs;
