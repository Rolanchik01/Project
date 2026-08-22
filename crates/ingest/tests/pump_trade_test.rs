//! Real fixtures: the same `BUY_TRADE_B64`/`REAL_FRESH_CURVE_B64` already
//! byte-verified in `crates/pump/tests/events_test.rs` and
//! `crates/pump/tests/bonding_curve_test.rs` — same mint
//! (3xM2iMg4RZBuzdFpvwYab9cUaVpniuHgLUZRg33ipump), same bonding curve
//! (113P6U1rwkHf69wyGT4qc9JNJfW1fXrKHYsU2Yy4goF), confirming
//! `ingest_pump_trade` correctly converts the real on-chain lamport amount
//! (69,135,801) at an illustrative price, and correctly refuses when the
//! curve's `quote_mint` isn't SOL.

use base64::Engine;
use momentum_core::adapter_contract::VenueAdapter;
use momentum_core::domain::{EventPayload, Venue};
use momentum_ingest::{ingest_pump_graduated, ingest_pump_trade, EventContext};
use momentum_pump::adapter::{Candidate, PumpAdapter};
use momentum_pump::BondingCurve;
use solana_pubkey::Pubkey;

const BUY_TRADE_B64: &str = "vdt/007mYe4r5Pc/8j57TNa7o4QTnpbTzypuneeb9hsI+jGpyqeXn7ntHgQAAAAAauzuaD4CAAABjuWwwm5EytPop83kC5GS7Qvl5a2X41dvLESDIm040Z8NWSpqAAAAALmZQgAHAAAAliPp3qTNAwC57R4EAAAAAJaL1pITzwIASsL40N1cvJfjKJwZfLUGKlTz2Va5zm5RFfllZ6pcs+ZfAAAAAAAAAJcFCgAAAAAAjuWwwm5EytPop83kC5GS7Qvl5a2X41dvLESDIm040Z8eAAAAAAAAADAqAwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAwAAAGJ1eQAAAAAAAAAAAAAAAAAAAAAAiBMAAAAAAADLAgUAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAALntHgQAAAAAuZlCAAcAAAC57R4EAAAAAA==";
const REAL_FRESH_CURVE_B64: &str = "F7f4N2DYrGAAENhH488DAAGsI/wGAAAAAHjF+1HRAgABAAAAAAAAAACAxqR+jQMAAI7lsMJuRMrT6KfN5AuRku0L5eWtl+NXbyxEgyJtONGfAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==";

fn decode_fixture(b64: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(b64).unwrap()
}

fn ctx() -> EventContext {
    EventContext {
        id: "evt-trade".to_string(),
        slot: 12346,
        observed_at_ns: 1_700_000_000_000_000_000,
        signature: "3ssTh78rz8SbQJNTnGVTQLDDQ5bfHc55rgT9ARZ4xhKKfYSN4EHfoDXPMot2r16V6GDdDQMdShaAPnNch4gQ31Eq".to_string(),
        instruction_index: 3,
        program_version: "pump-layout-2026-08".to_string(),
    }
}

#[test]
fn converts_a_real_buy_trades_lamport_amount_to_usd_at_a_given_price() {
    let adapter = PumpAdapter::new("pump-layout-2026-08");
    let candidate = adapter.decode(&decode_fixture(BUY_TRADE_B64)).expect("should decode");
    let curve = BondingCurve::decode(&decode_fixture(REAL_FRESH_CURVE_B64)).unwrap();

    let event = ingest_pump_trade(&candidate, &curve, 150.0, &ctx()).expect("should produce an event");

    assert_eq!(event.venue, Venue::Pump);
    assert_eq!(event.mint, "3xM2iMg4RZBuzdFpvwYab9cUaVpniuHgLUZRg33ipump");
    match event.payload {
        EventPayload::Buy { amount_usd, buyer_cluster_id, buyer_quality } => {
            // Real on-chain amount: 69,135,801 lamports (verified exactly
            // in crates/pump/tests/events_test.rs). 150.0 is an
            // illustrative test price, not fetched — ingest_pump_trade
            // never fetches a price itself.
            let expected = 69_135_801.0 / 1_000_000_000.0 * 150.0;
            assert!((amount_usd - expected).abs() < 1e-9, "{amount_usd} vs {expected}");
            assert_eq!(buyer_cluster_id, None);
            assert_eq!(buyer_quality, 0.0);
        }
        other => panic!("expected Buy, got {other:?}"),
    }
}

#[test]
fn refuses_to_convert_a_trade_on_a_non_sol_quoted_curve() {
    let adapter = PumpAdapter::new("pump-layout-2026-08");
    let candidate = adapter.decode(&decode_fixture(BUY_TRADE_B64)).expect("should decode");
    let mut curve = BondingCurve::decode(&decode_fixture(REAL_FRESH_CURVE_B64)).unwrap();
    curve.quote_mint = Pubkey::new_unique();

    assert!(ingest_pump_trade(&candidate, &curve, 150.0, &ctx()).is_none());
}

#[test]
fn refuses_a_non_finite_or_negative_price_rather_than_producing_garbage_usd() {
    let adapter = PumpAdapter::new("pump-layout-2026-08");
    let candidate = adapter.decode(&decode_fixture(BUY_TRADE_B64)).expect("should decode");
    let curve = BondingCurve::decode(&decode_fixture(REAL_FRESH_CURVE_B64)).unwrap();

    assert!(ingest_pump_trade(&candidate, &curve, f64::NAN, &ctx()).is_none());
    assert!(ingest_pump_trade(&candidate, &curve, f64::INFINITY, &ctx()).is_none());
    assert!(ingest_pump_trade(&candidate, &curve, -1.0, &ctx()).is_none());
}

#[test]
fn a_graduated_candidate_needs_no_price_and_carries_no_fields() {
    // No real CompleteEvent fixture exists yet (see crates/pump/src/events.rs
    // module docs) so this candidate is constructed directly rather than
    // decoded — ingest_pump_graduated's own logic is a pure field mapping
    // with no math, unlike decode_complete's still-unverified byte layout.
    let candidate = Candidate::Graduated {
        mint: Pubkey::new_unique(),
        bonding_curve: Pubkey::new_unique(),
        user: Pubkey::new_unique(),
    };
    let event = ingest_pump_graduated(&candidate, &ctx()).expect("should produce an event");
    assert_eq!(event.venue, Venue::Pump);
    assert!(matches!(event.payload, EventPayload::Graduation));
}

#[test]
fn a_token_created_candidate_is_not_a_trade_or_graduation() {
    let candidate = Candidate::TokenCreated {
        mint: Pubkey::new_unique(),
        bonding_curve: Pubkey::new_unique(),
        creator: Pubkey::new_unique(),
        user: Pubkey::new_unique(),
        name: "x".to_string(),
        symbol: "x".to_string(),
        uri: "x".to_string(),
        token_program: Pubkey::new_unique(),
        is_mayhem_mode: false,
        is_cashback_enabled: false,
        quote_mint: Pubkey::new_unique(),
    };
    let curve = BondingCurve::decode(&decode_fixture(REAL_FRESH_CURVE_B64)).unwrap();
    assert!(ingest_pump_trade(&candidate, &curve, 150.0, &ctx()).is_none());
    assert!(ingest_pump_graduated(&candidate, &ctx()).is_none());
}
