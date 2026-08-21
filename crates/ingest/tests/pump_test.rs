//! Real end-to-end fixture: the same Pump `CreateEvent` used in
//! `crates/pump/tests/events_test.rs`
//! (mint 3xM2iMg4RZBuzdFpvwYab9cUaVpniuHgLUZRg33ipump), combined with that
//! exact mint's real Token-2022 account, fetched fresh from mainnet — not
//! reused from `crates/token2022`'s fixture, which is a *different* mint
//! (ExXQP6Z...). Proves the whole chain end to end: real raw event bytes ->
//! `PumpAdapter::decode` -> real raw mint bytes -> `inspect_mint` ->
//! `ingest_pump_token_created` -> a `domain::Event` that `risk_engine`
//! accepts with no hard block.

use base64::Engine;
use momentum_core::domain::{EventPayload, Venue};
use momentum_core::risk_engine::{apply_event, HardBlock};
use momentum_core::domain::ReplayState;
use momentum_core::scoring_config::DEFAULT_SCORING_CONFIG;
use momentum_ingest::{ingest_pump_token_created, EventContext};
use momentum_pump::adapter::{Candidate, PumpAdapter};
use momentum_token2022::inspect_mint;
use momentum_core::adapter_contract::VenueAdapter;

const CREATE_B64: &str = "G3KpTd7rY3YEAAAAMTAwawQAAAAxMDBrUAAAAGh0dHBzOi8vaXBmcy5pby9pcGZzL2JhZmtyZWlkcHh5NWkyNXJ2M3Ezb2VvaG5mbXJhZDN2cmJndDVrYWtsbW9ucnI0ZTN3eWxtbmd0Nmg0K+T3P/I+e0zWu6OEE56W088qbp3nm/YbCPoxqcqnl58AAAvh7GU8u74DNCowKDSNy9q8qObVSiwzKs3NxZsNDo7lsMJuRMrT6KfN5AuRku0L5eWtl+NXbyxEgyJtONGfjuWwwm5EytPop83kC5GS7Qvl5a2X41dvLESDIm040Z8NWSpqAAAAAAAQ2EfjzwMAAKwj/AYAAAAAeMX7UdECAACAxqR+jQMABt324e51j94YQl285GzN2rYa/E2DuQ0n/r35KNihi/wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAKwj/AYAAAA=";

/// The real mint account for mint 3xM2iMg4RZBuzdFpvwYab9cUaVpniuHgLUZRg33ipump
/// (the same mint CREATE_B64 creates) — fetched fresh from mainnet, owned
/// by the Token-2022 program, no dangerous extensions.
const REAL_MINT_B64: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAIDGpH6NAwAGAQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAARIAQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACvk9z/yPntM1rujhBOeltPPKm6d55v2Gwj6ManKp5efEwCoAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAK+T3P/I+e0zWu6OEE56W088qbp3nm/YbCPoxqcqnl58EAAAAMTAwawQAAAAxMDBrUAAAAGh0dHBzOi8vaXBmcy5pby9pcGZzL2JhZmtyZWlkcHh5NWkyNXJ2M3Ezb2VvaG5mbXJhZDN2cmJndDVrYWtsbW9ucnI0ZTN3eWxtbmd0Nmg0AAAAAA==";

fn decode_fixture(b64: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(b64).unwrap()
}

fn ctx() -> EventContext {
    EventContext {
        id: "evt-1".to_string(),
        slot: 12345,
        observed_at_ns: 1_700_000_000_000_000_000,
        signature: "3ssTh78rz8SbQJNTnGVTQLDDQ5bfHc55rgT9ARZ4xhKKfYSN4EHfoDXPMot2r16V6GDdDQMdShaAPnNch4gQ31Eq".to_string(),
        instruction_index: 2,
        program_version: "pump-layout-2026-08".to_string(),
    }
}

#[test]
fn ingests_a_real_token_created_candidate_with_no_dangerous_extensions_into_a_clean_event() {
    let adapter = PumpAdapter::new("pump-layout-2026-08");
    let candidate = adapter.decode(&decode_fixture(CREATE_B64)).expect("should decode");

    let mint_data = decode_fixture(REAL_MINT_B64);
    assert_eq!(mint_data.len(), 406, "must match the real mint account's on-chain space exactly");
    let mint_flags = inspect_mint(&mint_data).unwrap();

    // Independently verifiable straight from the raw bytes without trusting
    // inspect_mint: the base Mint layout's two COption<Pubkey> tags
    // (mint_authority at offset 0, freeze_authority at offset 46) are both
    // 0 (None) in this fixture.
    assert_eq!(&mint_data[0..4], &[0, 0, 0, 0]);
    assert_eq!(&mint_data[46..50], &[0, 0, 0, 0]);

    let event = ingest_pump_token_created(&candidate, mint_flags, &ctx()).expect("should produce an event");

    assert_eq!(event.id, "evt-1");
    assert_eq!(event.slot, 12345);
    assert_eq!(event.instruction_index, 2);
    assert_eq!(event.venue, Venue::Pump);
    assert_eq!(event.mint, "3xM2iMg4RZBuzdFpvwYab9cUaVpniuHgLUZRg33ipump");
    match &event.payload {
        EventPayload::TokenCreated {
            mint_authority_active,
            freeze_authority_active,
            transfer_hook,
            transfer_fee_bps,
            permanent_delegate,
            non_transferable,
            default_frozen,
            unsupported_token_program,
            creator_cluster_id,
            creator_history_score,
        } => {
            assert!(!*mint_authority_active);
            assert!(!*freeze_authority_active);
            assert!(!*transfer_hook);
            assert_eq!(*transfer_fee_bps, 0);
            assert!(!*permanent_delegate);
            assert!(!*non_transferable);
            assert!(!*default_frozen);
            assert!(!*unsupported_token_program);
            assert_eq!(*creator_cluster_id, None);
            assert_eq!(*creator_history_score, None);
        }
        other => panic!("expected TokenCreated, got {other:?}"),
    }

    // Feed it through the real risk-engine: a clean mint must clear the
    // hard-veto gate (it will still land on Observe, not an entry decision
    // — no demand/narrative signal has been seen yet — but must not be
    // blocked outright).
    let mut state = ReplayState::new();
    let snapshot = apply_event(&mut state, &event, &DEFAULT_SCORING_CONFIG);
    assert!(snapshot.hard_blocks.is_empty());
}

#[test]
fn a_permanent_delegate_mint_produces_an_event_the_risk_engine_hard_blocks() {
    use momentum_token2022::MintExtensionFlags;

    let adapter = PumpAdapter::new("pump-layout-2026-08");
    let candidate = adapter.decode(&decode_fixture(CREATE_B64)).expect("should decode");

    let dangerous_flags = MintExtensionFlags { permanent_delegate: true, ..MintExtensionFlags::default() };
    let event = ingest_pump_token_created(&candidate, dangerous_flags, &ctx()).unwrap();

    let mut state = ReplayState::new();
    let snapshot = apply_event(&mut state, &event, &DEFAULT_SCORING_CONFIG);
    assert_eq!(snapshot.hard_blocks, vec![HardBlock::RestrictedTransferMechanism]);
}

#[test]
fn a_mint_owned_by_neither_known_token_program_is_flagged_unsupported() {
    use momentum_token2022::MintExtensionFlags;
    use momentum_pump::PUMP_PROGRAM_ID;

    let adapter = PumpAdapter::new("pump-layout-2026-08");
    let mut candidate = adapter.decode(&decode_fixture(CREATE_B64)).expect("should decode");
    if let Candidate::TokenCreated { token_program, .. } = &mut candidate {
        *token_program = PUMP_PROGRAM_ID.parse().unwrap();
    }

    let event = ingest_pump_token_created(&candidate, MintExtensionFlags::default(), &ctx()).unwrap();
    match event.payload {
        EventPayload::TokenCreated { unsupported_token_program, .. } => assert!(unsupported_token_program),
        other => panic!("expected TokenCreated, got {other:?}"),
    }
}

#[test]
fn a_trade_candidate_is_not_a_token_created_event() {
    let adapter = PumpAdapter::new("pump-layout-2026-08");
    // A Trade candidate carries no mint-creation data at all, so this must
    // return None rather than fabricate an event from the wrong variant.
    let trade = Candidate::Trade {
        mint: solana_pubkey::Pubkey::new_unique(),
        bonding_curve: solana_pubkey::Pubkey::new_unique(),
        user: solana_pubkey::Pubkey::new_unique(),
        is_buy: true,
        quote_amount: 1,
        token_amount: 1,
        fee: 0,
        creator_fee: 0,
        creator: solana_pubkey::Pubkey::new_unique(),
    };
    let _ = adapter;
    assert!(ingest_pump_token_created(&trade, momentum_token2022::MintExtensionFlags::default(), &ctx()).is_none());
}
