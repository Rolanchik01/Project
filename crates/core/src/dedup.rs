//! Ported from `src/dedup.js`. Two independent Geyser/Yellowstone gRPC feeds
//! will both deliver the same on-chain instruction; the identity of one
//! instruction is (venue, signature, instructionIndex) — the same
//! transaction can carry several instructions for the same venue, so
//! signature alone is not enough.

use std::collections::{HashMap, HashSet};

use crate::domain::{Event, Venue};

pub type DedupeKey = (Venue, String, u32);

pub fn event_dedupe_key(event: &Event) -> DedupeKey {
    (event.venue, event.signature.clone(), event.instruction_index)
}

/// Pure batch dedup: given a set of already-collected events (e.g. two
/// recorded NDJSON files merged for replay), keep one copy per instruction —
/// the one with the earliest observedAtNs, since that is the feed that saw
/// it first and is the more useful latency sample.
pub fn dedupe_events(events: Vec<Event>) -> Vec<Event> {
    let mut seen: HashMap<DedupeKey, Event> = HashMap::new();
    for event in events {
        let key = event_dedupe_key(&event);
        let replace = match seen.get(&key) {
            None => true,
            Some(existing) => event.observed_at_ns < existing.observed_at_ns,
        };
        if replace {
            seen.insert(key, event);
        }
    }
    seen.into_values().collect()
}

/// Streaming dedup for live ingestion: two feeds push events as they arrive
/// and only the first copy of each instruction should be forwarded
/// downstream. Unlike `dedupe_events`, this keeps whichever copy arrives
/// first in wall-clock order, since that is what a live pipeline actually
/// has to decide with.
#[derive(Debug, Default)]
pub struct StreamDeduplicator {
    seen: HashSet<DedupeKey>,
}

impl StreamDeduplicator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if the event is new and should be forwarded; false if it's a duplicate.
    pub fn admit(&mut self, event: &Event) -> bool {
        self.seen.insert(event_dedupe_key(event))
    }

    pub fn size(&self) -> usize {
        self.seen.len()
    }
}
