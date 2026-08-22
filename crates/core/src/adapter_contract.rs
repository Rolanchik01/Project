//! Ported from `src/adapter-contract.js`, plus the full `VenueAdapter`
//! trait from `docs/VENUE_ADAPTER.md` now that the first two real adapters
//! (Pump, PumpSwap) exist to shape it. Associated types let each venue keep
//! its own event/candidate/quote shapes instead of forcing a lowest common
//! denominator — Pump's `Candidate` and PumpSwap's are genuinely different
//! things. `build_buy`/`build_sell` are part of the trait but not
//! meaningfully implemented by either venue yet: Stage 1 is paper-only
//! (decode and quote, never send), building real transactions is Stage 7.

use std::collections::HashMap;
use std::fmt;

use crate::domain::{Event, Venue};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterVersionMismatch {
    pub venue: Venue,
    pub expected: String,
    pub received: String,
}

impl fmt::Display for AdapterVersionMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "HALT {:?}: expected protocol version {}, received {}",
            self.venue, self.expected, self.received
        )
    }
}

impl std::error::Error for AdapterVersionMismatch {}

#[derive(Debug, Default)]
pub struct AdapterRegistry {
    versions: HashMap<Venue, String>,
}

impl AdapterRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(mut self, venue: Venue, version: impl Into<String>) -> Self {
        self.versions.insert(venue, version.into());
        self
    }

    pub fn assert_compatible(&self, event: &Event) -> Result<(), AdapterVersionMismatch> {
        match self.versions.get(&event.venue) {
            None => Err(AdapterVersionMismatch {
                venue: event.venue,
                expected: "registered adapter".to_string(),
                received: event.program_version.clone(),
            }),
            Some(expected) if expected != &event.program_version => Err(AdapterVersionMismatch {
                venue: event.venue,
                expected: expected.clone(),
                received: event.program_version.clone(),
            }),
            Some(_) => Ok(()),
        }
    }
}

/// How much a candidate's pricing can be trusted right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiquidityRisk {
    /// Normal, priceable instrument with real liquidity behind it.
    Healthy,
    /// Priceable, but exit liquidity is thin enough to flag (e.g. a
    /// freshly created curve, or a pool most of whose reserves are gone).
    Thin,
    /// The instrument has migrated/graduated off this venue — quote here
    /// is stale by construction, not just low-liquidity.
    Graduated,
    /// A real variant this venue cannot price safely (e.g. a boosted
    /// PumpSwap pool, a mayhem-mode Pump curve) — never silently guess.
    Unpriceable,
}

/// A venue adapter's hot-path contract: turn raw on-chain bytes into
/// typed candidates and quotes. Mirrors the Rust trait sketched in
/// `docs/VENUE_ADAPTER.md`; associated types let Pump and PumpSwap use
/// their own event/candidate/instrument shapes rather than a shared,
/// artificially generic one.
pub trait VenueAdapter {
    /// What identifies one priceable market on this venue — a bonding
    /// curve address for Pump, a pool address for PumpSwap.
    type Instrument;
    /// What `decode` produces: a typed, venue-specific reading of one
    /// on-chain event, not yet merged into a `core::domain::Event` (that
    /// merge is an ingestion-layer concern, e.g. combining Pump's
    /// `TokenCreated` candidate with a separate Token-2022 mint
    /// inspection — this trait only covers what one piece of raw data by
    /// itself can tell you).
    type Candidate;
    /// `?Sized` so a venue can use `[u8]` directly — one raw event is just
    /// its bytes, no wrapper struct needed.
    type RawEvent: ?Sized;
    type AccountUpdate;
    type Quote;
    type TradeRequest;
    type BuiltTransaction;
    type Error;

    fn decode(&self, event: &Self::RawEvent) -> Option<Self::Candidate>;
    fn apply_update(&mut self, update: &Self::AccountUpdate) -> Result<(), Self::Error>;
    fn quote_buy(&self, instrument: &Self::Instrument, amount_in: u64) -> Result<Self::Quote, Self::Error>;
    fn quote_sell(&self, instrument: &Self::Instrument, amount_in: u64) -> Result<Self::Quote, Self::Error>;
    fn build_buy(&self, request: &Self::TradeRequest) -> Result<Self::BuiltTransaction, Self::Error>;
    fn build_sell(&self, request: &Self::TradeRequest) -> Result<Self::BuiltTransaction, Self::Error>;
    fn liquidity_risk(&self, instrument: &Self::Instrument) -> LiquidityRisk;
    fn protocol_version(&self) -> &'static str;
}
