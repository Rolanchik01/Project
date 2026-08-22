//! Same real fixtures as events_test.rs, plus a freshly fetched real `Pool`
//! account for the same pool as SELL_B64
//! (CevkYxWmzwmRcpK4aFKBtSxSAXzq4K6oxfwqG8uXRTvv), exercised through the
//! `VenueAdapter` trait end to end: `apply_update` with the real `Pool`
//! account plus synthetic token-account updates carrying the exact reserve
//! numbers the real `SellEvent` itself reports, then `quote_sell` must
//! reproduce that same event's real gross output.
//!
//! The synthetic token-account bytes are legitimate here, not a shortcut:
//! `apply_update`'s token-account parsing (amount at a fixed byte offset)
//! was independently verified against two real mainnet accounts — one
//! legacy SPL, one Token-2022 — during research (see the doc comment on
//! `SPL_TOKEN_ACCOUNT_AMOUNT_OFFSET` in src/adapter.rs); what's under test
//! here is the adapter's wiring, and the amounts plugged in are exact real
//! numbers from a verified real trade, not invented ones.

use base64::Engine;
use momentum_core::adapter_contract::{LiquidityRisk, VenueAdapter};
use momentum_pumpswap::adapter::{AccountUpdate, Candidate, PumpSwapAdapter};
use solana_pubkey::Pubkey;
use std::str::FromStr;

const SELL_B64: &str = "Pi83CqUD3CpX6YdqAAAAAMWBcBsAAAAA6IguVvYCAADFgXAbAAAAALLplyteAAAAtDKKwRkAAADSr6oLktMCAEx0eKv/AgAAGQAAAAAAAABqBU/rAQAAAAUAAAAAAAAAFgFDYgAAAADibinA/QIAAMxt5l39AgAArSfSi1Hnc04iy5mSrX5kGQnSEQYKAOrKCOXNi3Pn5z/wbbbJRyF9WU8VAggkhTTV55MUnPuJW6akk69hEbXMoNjYPQfauFOfG34h2FuMlL4CkolAiJJrrU9r2th45KXlQ/JEa+wIfXa/GmNHhHDBze7QwHl3CDiNCExajHZwvgVKwvjQ3Vy8l+MonBl8tQYqVPPZVrnOblEV+WVnqlyz5qAPh45v7lcOwJc4HLGaiyZQwkxXISRSA51Uj6moxVZQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAIgTAAAAAAAAi4AhMQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const BUY_B64: &str = "Z/RSHyz1d3dX6YdqAAAAACnNDFAAAAAA5EbAdUoHAAAAAAAAAAAAANdmsm06CQAAoagAFRoAAACa2MiiC1cCAORGwHVKBwAAGQAAAAAAAABsJPamBAAAAAUAAAAAAAAAfDox7gAAAABoDI+HSQcAAPznmOBEBwAA15MN77NI2BvPq1rfmk6byNA1YkD3xhagq24IZS/CmgsktWboeZxQnNFt127mhc5ARt1Pl25ah3bTqI72KULdxH1S8W8zWN67NUYyCEvq6J4+en+7LQpFm3aQ+2o+WB3hQWnrsZrcBHyyEI+N4IqIqEDNSn09Vhmk7GpV38jrPC5KwvjQ3Vy8l+MonBl8tQYqVPPZVrnOblEV+WVnqlyz5j/Ki0rjxmmlK2xlUmY3RO4xoVzkWz8cWd44M+FPGuP/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABs9yTAAAAAASAAAAYnV5X2V4YWN0X3F1b3RlX2luAAAAAAAAAAAAAAAAAAAAAIgTAAAAAAAAPp0YdwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
/// The real Pool account for the same pool SELL_B64 traded against,
/// fetched fresh from mainnet (address CevkYxWmzwmRcpK4aFKBtSxSAXzq4K6oxfwqG8uXRTvv).
const REAL_SELL_POOL_B64: &str = "8ZptBBGxbbz9AABwXCJuwGxQbmnlFLB25cSetKJ/vMSIgg26KU/MlRxAzAabiFf+q4GE+2h/Y0YYwDXaxDncGus7VZig8AAAAAABmHsP7dFJ+IwJp/5bgJ9wyvdcrplI3Tw9Vw3lOXwBtFEMW7RCvtcD4/Rcq24ByZi0uOg1iUWPmG2Fh3zgQQx3S2DKVZ3QYDMnO/5ZyZ9KoOb9lyD7+hBKfJu0qWYlBeF22quymlgPQotTHWTvJcysL4KRG4BoZ5ADbeC6Vriv8Q98pFh3YggAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==";

fn decode_fixture(b64: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(b64).unwrap()
}

fn sell_pool_pubkey() -> Pubkey {
    Pubkey::from_str("CevkYxWmzwmRcpK4aFKBtSxSAXzq4K6oxfwqG8uXRTvv").unwrap()
}

fn base_token_account_pubkey() -> Pubkey {
    Pubkey::from_str("7Wq5uZK2ZSEpcmy8MNQwPjNTZyXe8DxpcEGPoBSLoiPw").unwrap()
}

fn quote_token_account_pubkey() -> Pubkey {
    Pubkey::from_str("FibjFLVwKUSUscLywhejX1XZZNByVWpxEtLXmai5CfXL").unwrap()
}

/// A minimal SPL Token / Token-2022 account buffer: real accounts are 165
/// bytes (mint, owner, then `amount: u64` at offset 64), plus optional
/// Token-2022 extension TLV bytes after that we don't need here.
fn spl_token_account_bytes(amount: u64) -> Vec<u8> {
    let mut data = vec![0u8; 165];
    data[64..72].copy_from_slice(&amount.to_le_bytes());
    data
}

#[test]
fn decode_produces_a_trade_candidate_from_a_real_sell_event() {
    let adapter = PumpSwapAdapter::new("test-version");
    let data = decode_fixture(SELL_B64);
    let candidate = adapter.decode(&data).expect("should decode");
    match candidate {
        Candidate::Trade { pool, is_buy, base_amount, quote_amount, can_boost, .. } => {
            assert_eq!(pool, sell_pool_pubkey());
            assert!(!is_buy);
            assert_eq!(base_amount, 460_358_085);
            assert_eq!(quote_amount, 3_287_225_363_916);
            assert!(!can_boost);
        }
        other => panic!("expected Trade, got {other:?}"),
    }
}

#[test]
fn decode_produces_a_trade_candidate_from_a_real_buy_event() {
    let adapter = PumpSwapAdapter::new("test-version");
    let data = decode_fixture(BUY_B64);
    let candidate = adapter.decode(&data).expect("should decode");
    match candidate {
        Candidate::Trade { pool, is_buy, base_amount, quote_amount, .. } => {
            assert_eq!(pool, Pubkey::from_str("FWWiDq1gPab1quER2fd7U5qMifLnaPma2APDjJW7Tgfc").unwrap());
            assert!(is_buy);
            assert_eq!(base_amount, 1_343_016_233);
            assert_eq!(quote_amount, 7_992_407_287_804);
        }
        other => panic!("expected Trade, got {other:?}"),
    }
}

#[test]
fn apply_update_then_quote_reproduces_the_real_sell_events_gross_output() {
    let mut adapter = PumpSwapAdapter::new("test-version");
    let pool_key = sell_pool_pubkey();

    // Unknown until the Pool account itself has been applied.
    assert_eq!(adapter.liquidity_risk(&pool_key), LiquidityRisk::Unpriceable);
    assert!(adapter.quote_sell(&pool_key, 460_358_085).is_err());

    adapter.apply_update(&AccountUpdate { pubkey: pool_key, data: decode_fixture(REAL_SELL_POOL_B64) }).unwrap();

    // Pool known, but its reserves aren't yet — still not priceable.
    assert_eq!(adapter.liquidity_risk(&pool_key), LiquidityRisk::Unpriceable);
    assert_eq!(adapter.quote_sell(&pool_key, 460_358_085), Err(momentum_pumpswap::adapter::AdapterError::ReservesNotYetKnown));

    // The exact pre-trade reserves the real SellEvent itself reports.
    adapter
        .apply_update(&AccountUpdate { pubkey: base_token_account_pubkey(), data: spl_token_account_bytes(110_621_242_036) })
        .unwrap();
    adapter
        .apply_update(&AccountUpdate { pubkey: quote_token_account_pubkey(), data: spl_token_account_bytes(795_574_167_842_770) })
        .unwrap();

    assert_eq!(adapter.known_reserves(&pool_key), Some((110_621_242_036, 795_574_167_842_770)));
    assert_eq!(adapter.liquidity_risk(&pool_key), LiquidityRisk::Healthy);

    // Same real trade already verified exactly in events_test.rs, now
    // exercised through the adapter's own quote_sell.
    let quote = adapter.quote_sell(&pool_key, 460_358_085).unwrap();
    assert_eq!(quote.amount_out, 3_297_116_714_060);
}

#[test]
fn quoting_an_instrument_with_no_cached_state_is_a_clean_error_not_a_panic() {
    let adapter = PumpSwapAdapter::new("test-version");
    let unknown = Pubkey::new_unique();
    assert!(adapter.quote_buy(&unknown, 1_000_000_000).is_err());
    assert!(adapter.quote_sell(&unknown, 1_000_000).is_err());
}

#[test]
fn build_buy_and_build_sell_are_explicitly_not_implemented_in_stage_1() {
    use momentum_pumpswap::adapter::{AdapterError, TradeRequest};
    let adapter = PumpSwapAdapter::new("test-version");
    let request = TradeRequest { pool: sell_pool_pubkey(), amount: 1_000_000_000 };
    assert_eq!(adapter.build_buy(&request), Err(AdapterError::NotImplementedInStage1));
    assert_eq!(adapter.build_sell(&request), Err(AdapterError::NotImplementedInStage1));
}

#[test]
fn protocol_version_reports_what_the_adapter_was_constructed_with() {
    let adapter = PumpSwapAdapter::new("pumpswap-layout-2026-08");
    assert_eq!(adapter.protocol_version(), "pumpswap-layout-2026-08");
}
