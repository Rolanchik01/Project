//! Paper trading (Group G): turns `risk_engine::RiskSnapshot`'s
//! `decision`/`position_multiplier` from something `bin/pipeline.rs` only
//! ever printed into an actual sized paper position, tracked to a real
//! exit and a real profit/loss figure. See [`sizing`] for how much gets
//! risked and why, and [`portfolio`] for how an open position's life
//! (take-profit/stop-loss/emergency exit) is tracked.
//!
//! Deliberately venue-agnostic and quote-math-free — see `portfolio`'s
//! doc comment for why an accurate current value has to come from the
//! caller (`bin/pipeline.rs`, which has real `BondingCurve`/`Pool` state
//! to quote against), not from this crate.

pub mod portfolio;
pub mod sizing;

pub use portfolio::{ClosedTrade, ExitReason, OpenPosition, Portfolio};
pub use sizing::{position_size_usd, PositionSizingConfig, DEFAULT_POSITION_SIZING_CONFIG};
