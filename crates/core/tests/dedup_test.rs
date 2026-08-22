//! Ported from test/dedup.test.js.
use momentum_core::dedup::{dedupe_events, event_dedupe_key, StreamDeduplicator};
use momentum_core::domain::{Event, EventPayload, Venue};

fn feed_event(signature: &str, instruction_index: u32, observed_at_ns: u64) -> Event {
    Event {
        id: format!("{signature}-{instruction_index}"),
        slot: 1,
        observed_at_ns,
        signature: signature.to_string(),
        instruction_index,
        venue: Venue::Pump,
        program_version: "v1".to_string(),
        mint: "MintA".to_string(),
        payload: EventPayload::CurveCreated,
    }
}

#[test]
fn event_dedupe_key_identifies_one_on_chain_instruction_by_venue_signature_and_instruction_index() {
    let event = feed_event("sig-1", 0, 1000);
    assert_eq!(event_dedupe_key(&event), (Venue::Pump, "sig-1".to_string(), 0));
}

#[test]
fn dedupe_events_keeps_the_copy_with_the_earliest_observed_at_ns_across_two_feeds() {
    let from_feed_a = {
        let mut e = feed_event("sig-1", 0, 2000);
        e.id = "feed-a".to_string();
        e
    };
    let from_feed_b = {
        let mut e = feed_event("sig-1", 0, 1500);
        e.id = "feed-b".to_string();
        e
    };
    let winner = dedupe_events(vec![from_feed_a, from_feed_b]);
    assert_eq!(winner.len(), 1);
    assert_eq!(winner[0].id, "feed-b");
}

#[test]
fn dedupe_events_treats_different_instructions_in_the_same_transaction_as_distinct() {
    let first = feed_event("sig-1", 0, 1000);
    let second = feed_event("sig-1", 1, 1000);
    assert_eq!(dedupe_events(vec![first, second]).len(), 2);
}

#[test]
fn stream_deduplicator_admits_the_first_copy_of_an_instruction_and_rejects_the_duplicate() {
    let mut dedup = StreamDeduplicator::new();
    let event = feed_event("sig-1", 0, 1000);
    assert!(dedup.admit(&event));
    assert!(!dedup.admit(&event));
    assert_eq!(dedup.size(), 1);
}

#[test]
fn stream_deduplicator_treats_different_signatures_as_independent_instructions() {
    let mut dedup = StreamDeduplicator::new();
    assert!(dedup.admit(&feed_event("sig-1", 0, 1000)));
    assert!(dedup.admit(&feed_event("sig-2", 0, 1000)));
    assert_eq!(dedup.size(), 2);
}
