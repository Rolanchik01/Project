//! First slice of the live event source ("Dataplane"): a `logsSubscribe`
//! WebSocket listener over the public Solana RPC, feeding raw event bytes
//! that `momentum_pump::events::decode_event` already knows how to parse.
//!
//! Deliberately scoped narrow for this pass — proving the live pipe works
//! end to end on the real, currently-flowing chain, not yet the full
//! pipeline:
//! - Only Pump's program is listened to (PumpSwap needs the same
//!   `listener::run` with a different `program_id`, not yet duplicated
//!   into a second running task here).
//! - Only `logsSubscribe` (transaction logs) is wired up. `apply_update`
//!   (bonding curve reserves, mint account inspection) needs
//!   `accountSubscribe` on specific accounts discovered dynamically from
//!   `TokenCreated`/`Trade` events as they arrive — a genuinely separate,
//!   stateful subscription-management problem (which accounts are we
//!   watching, when to add/drop one) not built here.
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

pub mod listener;
pub mod logs;
