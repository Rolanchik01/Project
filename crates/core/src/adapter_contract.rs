//! Ported from `src/adapter-contract.js`. The full `VenueAdapter` trait
//! (decode/apply_update/quote_buy/quote_sell/build_buy/build_sell/
//! liquidity_risk/protocol_version — see docs/VENUE_ADAPTER.md) is defined
//! once the first real venue adapter (Pump) is built, so its associated
//! types are shaped by real decoding needs instead of guessed in advance.
//! What Stage 0 actually exercises — the version-halt gate — is ported here.

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
