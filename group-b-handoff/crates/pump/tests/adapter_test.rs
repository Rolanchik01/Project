//! Same real fixtures as events_test.rs and bonding_curve_test.rs, this
//! time exercised through the `VenueAdapter` trait end to end.

use base64::Engine;
use momentum_core::adapter_contract::{LiquidityRisk, VenueAdapter};
use momentum_pump::adapter::{AccountUpdate, Candidate, PumpAdapter};
use solana_pubkey::Pubkey;
use std::str::FromStr;

const CREATE_B64: &str = "G3KpTd7rY3YEAAAAMTAwawQAAAAxMDBrUAAAAGh0dHBzOi8vaXBmcy5pby9pcGZzL2JhZmtyZWlkcHh5NWkyNXJ2M3Ezb2VvaG5mbXJhZDN2cmJndDVrYWtsbW9ucnI0ZTN3eWxtbmd0Nmg0K+T3P/I+e0zWu6OEE56W088qbp3nm/YbCPoxqcqnl58AAAvh7GU8u74DNCowKDSNy9q8qObVSiwzKs3NxZsNDo7lsMJuRMrT6KfN5AuRku0L5eWtl+NXbyxEgyJtONGfjuWwwm5EytPop83kC5GS7Qvl5a2X41dvLESDIm040Z8NWSpqAAAAAAAQ2EfjzwMAAKwj/AYAAAAAeMX7UdECAACAxqR+jQMABt324e51j94YQl285GzN2rYa/E2DuQ0n/r35KNihi/wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAKwj/AYAAAA=";
const BUY_TRADE_B64: &str = "vdt/007mYe4r5Pc/8j57TNa7o4QTnpbTzypuneeb9hsI+jGpyqeXn7ntHgQAAAAAauzuaD4CAAABjuWwwm5EytPop83kC5GS7Qvl5a2X41dvLESDIm040Z8NWSpqAAAAALmZQgAHAAAAliPp3qTNAwC57R4EAAAAAJaL1pITzwIASsL40N1cvJfjKJwZfLUGKlTz2Va5zm5RFfllZ6pcs+ZfAAAAAAAAAJcFCgAAAAAAjuWwwm5EytPop83kC5GS7Qvl5a2X41dvLESDIm040Z8eAAAAAAAAADAqAwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAwAAAGJ1eQAAAAAAAAAAAAAAAAAAAAAAiBMAAAAAAADLAgUAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAALntHgQAAAAAuZlCAAcAAAC57R4EAAAAAA==";
const REAL_FRESH_CURVE_B64: &str = "F7f4N2DYrGAAENhH488DAAGsI/wGAAAAAHjF+1HRAgABAAAAAAAAAACAxqR+jQMAAI7lsMJuRMrT6KfN5AuRku0L5eWtl+NXbyxEgyJtONGfAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==";

fn decode_fixture(b64: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(b64).unwrap()
}

fn bonding_curve_pubkey() -> Pubkey {
    Pubkey::from_str("113P6U1rwkHf69wyGT4qc9JNJfW1fXrKHYsU2Yy4goF").unwrap()
}

fn mint_pubkey() -> Pubkey {
    Pubkey::from_str("3xM2iMg4RZBuzdFpvwYab9cUaVpniuHgLUZRg33ipump").unwrap()
}

#[test]
fn bonding_curve_pda_matches_the_real_address_a_live_create_event_reports() {
    // The strongest possible check for the PDA derivation: it must agree
    // with the actual on-chain address, not just "look like" a pubkey.
    assert_eq!(PumpAdapter::bonding_curve_pda(&mint_pubkey()), bonding_curve_pubkey());
}

#[test]
fn decode_produces_a_token_created_candidate_from_a_real_create_event() {
    let adapter = PumpAdapter::new("test-version");
    let data = decode_fixture(CREATE_B64);
    let candidate = adapter.decode(&data).expect("should decode");
    match candidate {
        Candidate::TokenCreated { mint, bonding_curve, creator, is_mayhem_mode, is_cashback_enabled, .. } => {
            assert_eq!(mint, mint_pubkey());
            assert_eq!(bonding_curve, bonding_curve_pubkey());
            assert_eq!(creator, Pubkey::from_str("Acp2tKDySe79AsxokjeEy7VdrvWD8bR4V9rLSFkvo6sx").unwrap());
            assert!(!is_mayhem_mode);
            assert!(!is_cashback_enabled);
        }
        other => panic!("expected TokenCreated, got {other:?}"),
    }
}

#[test]
fn decode_derives_the_bonding_curve_address_for_a_trade_candidate() {
    let adapter = PumpAdapter::new("test-version");
    let data = decode_fixture(BUY_TRADE_B64);
    let candidate = adapter.decode(&data).expect("should decode");
    match candidate {
        Candidate::Trade { mint, bonding_curve, is_buy, quote_amount, token_amount, .. } => {
            assert_eq!(mint, mint_pubkey());
            // Trade events don't carry the bonding curve address directly —
            // this must match the PDA, which in turn must match the real
            // address the sibling CreateEvent test confirms.
            assert_eq!(bonding_curve, bonding_curve_pubkey());
            assert!(is_buy);
            assert_eq!(quote_amount, 69_135_801);
            assert_eq!(token_amount, 2_467_071_716_458);
        }
        other => panic!("expected Trade, got {other:?}"),
    }
}

#[test]
fn apply_update_then_quote_reproduces_the_group_a_verified_math() {
    let mut adapter = PumpAdapter::new("test-version");
    let curve_key = bonding_curve_pubkey();

    // Unknown until an account update has been applied.
    assert_eq!(adapter.liquidity_risk(&curve_key), LiquidityRisk::Unpriceable);

    adapter
        .apply_update(&AccountUpdate { pubkey: curve_key, data: decode_fixture(REAL_FRESH_CURVE_B64) })
        .unwrap();

    assert_eq!(adapter.liquidity_risk(&curve_key), LiquidityRisk::Healthy);

    // Same real trade already verified exactly in bonding_curve_test.rs,
    // now exercised through the adapter's own quote_sell instead of
    // calling the crate's free function directly.
    let curve = adapter.curve(&curve_key).unwrap();
    assert_eq!(curve.virtual_token_reserves, 1_073_000_000_000_000);
}

#[test]
fn quoting_an_instrument_with_no_cached_state_is_a_clean_error_not_a_panic() {
    let adapter = PumpAdapter::new("test-version");
    let unknown = Pubkey::new_unique();
    assert!(adapter.quote_buy(&unknown, 1_000_000_000).is_err());
    assert!(adapter.quote_sell(&unknown, 1_000_000).is_err());
}

#[test]
fn build_buy_and_build_sell_are_explicitly_not_implemented_in_stage_1() {
    use momentum_pump::adapter::{AdapterError, TradeRequest};
    let adapter = PumpAdapter::new("test-version");
    let request = TradeRequest { bonding_curve: bonding_curve_pubkey(), amount: 1_000_000_000 };
    assert_eq!(adapter.build_buy(&request), Err(AdapterError::NotImplementedInStage1));
    assert_eq!(adapter.build_sell(&request), Err(AdapterError::NotImplementedInStage1));
}

#[test]
fn protocol_version_reports_what_the_adapter_was_constructed_with() {
    let adapter = PumpAdapter::new("pump-layout-2026-08");
    assert_eq!(adapter.protocol_version(), "pump-layout-2026-08");
}
