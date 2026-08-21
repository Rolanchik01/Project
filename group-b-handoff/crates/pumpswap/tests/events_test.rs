//! Fixtures captured live from mainnet during Stage 1 research: extracted
//! programmatically from raw RPC responses (never hand-copied — see
//! `crates/pump/tests/events_test.rs` for why that rule exists).
//!
//! - SELL_B64: a real `SellEvent` on pool
//!   CevkYxWmzwmRcpK4aFKBtSxSAXzq4K6oxfwqG8uXRTvv
//!   (5GWYCkt67vcSGiLbWBf1gq2PC2emazr9unrckcW5kcWE4xxe7zMYxQgmK3cc4o6SjaTjCB3JbZ6VaAYBXrr99roP),
//!   a non-boosted pool. Its `quote_amount_out` field matches
//!   `lib.rs::quote_sell` fed the event's own pre-trade
//!   `pool_*_token_reserves` exactly.
//! - BUY_B64: a real `buy_exact_quote_in` `BuyEvent` on pool
//!   FWWiDq1gPab1quER2fd7U5qMifLnaPma2APDjJW7Tgfc
//!   (n1XCWAR3tEx79dSuhpmmQc1j1rW2iWem2JtPXR6arXGnx4Hb6MSfppPkGVjFgZeXzTFQBGHgR2wgmyq1PhrPDQV),
//!   also non-boosted. Its `base_amount_out` matches
//!   `lib.rs::quote_buy_exact_quote_in` exactly, same methodology. (A plain
//!   `buy` instruction's `base_amount_out` is the user's *requested*
//!   amount, priced the other way around — it does not match this formula,
//!   and is intentionally not what this fixture tests.)

use base64::Engine;
use momentum_pumpswap::events::{decode_event, PumpSwapEvent};
use momentum_pumpswap::{quote_buy_exact_quote_in, quote_sell, Pool};
use solana_pubkey::Pubkey;
use std::str::FromStr;

const SELL_B64: &str = "Pi83CqUD3CpX6YdqAAAAAMWBcBsAAAAA6IguVvYCAADFgXAbAAAAALLplyteAAAAtDKKwRkAAADSr6oLktMCAEx0eKv/AgAAGQAAAAAAAABqBU/rAQAAAAUAAAAAAAAAFgFDYgAAAADibinA/QIAAMxt5l39AgAArSfSi1Hnc04iy5mSrX5kGQnSEQYKAOrKCOXNi3Pn5z/wbbbJRyF9WU8VAggkhTTV55MUnPuJW6akk69hEbXMoNjYPQfauFOfG34h2FuMlL4CkolAiJJrrU9r2th45KXlQ/JEa+wIfXa/GmNHhHDBze7QwHl3CDiNCExajHZwvgVKwvjQ3Vy8l+MonBl8tQYqVPPZVrnOblEV+WVnqlyz5qAPh45v7lcOwJc4HLGaiyZQwkxXISRSA51Uj6moxVZQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAIgTAAAAAAAAi4AhMQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const BUY_B64: &str = "Z/RSHyz1d3dX6YdqAAAAACnNDFAAAAAA5EbAdUoHAAAAAAAAAAAAANdmsm06CQAAoagAFRoAAACa2MiiC1cCAORGwHVKBwAAGQAAAAAAAABsJPamBAAAAAUAAAAAAAAAfDox7gAAAABoDI+HSQcAAPznmOBEBwAA15MN77NI2BvPq1rfmk6byNA1YkD3xhagq24IZS/CmgsktWboeZxQnNFt127mhc5ARt1Pl25ah3bTqI72KULdxH1S8W8zWN67NUYyCEvq6J4+en+7LQpFm3aQ+2o+WB3hQWnrsZrcBHyyEI+N4IqIqEDNSn09Vhmk7GpV38jrPC5KwvjQ3Vy8l+MonBl8tQYqVPPZVrnOblEV+WVnqlyz5j/Ki0rjxmmlK2xlUmY3RO4xoVzkWz8cWd44M+FPGuP/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABs9yTAAAAAASAAAAYnV5X2V4YWN0X3F1b3RlX2luAAAAAAAAAAAAAAAAAAAAAIgTAAAAAAAAPp0YdwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

fn decode_fixture(b64: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(b64).unwrap()
}

#[test]
fn decodes_a_real_sell_event_and_reproduces_its_quote_amount_out() {
    let data = decode_fixture(SELL_B64);
    let event = decode_event(&data).expect("should decode as a known event");
    match event {
        PumpSwapEvent::Sell(s) => {
            assert_eq!(s.pool, Pubkey::from_str("CevkYxWmzwmRcpK4aFKBtSxSAXzq4K6oxfwqG8uXRTvv").unwrap());
            assert_eq!(s.base_amount_in, 460_358_085);
            assert_eq!(s.pool_base_token_reserves, 110_621_242_036);
            assert_eq!(s.pool_quote_token_reserves, 795_574_167_842_770);
            assert_eq!(s.lp_fee, 8_242_791_786);
            assert_eq!(s.protocol_fee, 1_648_558_358);
            assert!(!s.can_boost);
            assert_eq!(s.virtual_quote_reserves, 0);

            // The event's own gross output, reproduced from its own
            // pre-trade reserves via the crate's verified quote_sell.
            let pool = standard_pool();
            let gross_out = quote_sell(&pool, s.pool_base_token_reserves, s.pool_quote_token_reserves, s.base_amount_in).unwrap();
            assert_eq!(gross_out, 3_297_116_714_060);
        }
        other => panic!("expected Sell, got {other:?}"),
    }
}

#[test]
fn decodes_a_real_buy_exact_quote_in_event_and_reproduces_its_base_amount_out() {
    let data = decode_fixture(BUY_B64);
    let event = decode_event(&data).expect("should decode as a known event");
    match event {
        PumpSwapEvent::Buy(b) => {
            assert_eq!(b.pool, Pubkey::from_str("FWWiDq1gPab1quER2fd7U5qMifLnaPma2APDjJW7Tgfc").unwrap());
            assert_eq!(b.ix_name, "buy_exact_quote_in");
            assert_eq!(b.base_amount_out, 1_343_016_233);
            assert_eq!(b.pool_base_token_reserves, 112_021_514_401);
            assert_eq!(b.pool_quote_token_reserves, 658_657_440_749_722);
            assert_eq!(b.user_quote_amount_in, 7_992_407_287_804);
            assert_eq!(b.lp_fee, 19_981_018_220);
            assert_eq!(b.protocol_fee, 3_996_203_644);
            assert!(!b.can_boost);
            assert_eq!(b.virtual_quote_reserves, 0);

            let pool = standard_pool();
            let gross_out = quote_buy_exact_quote_in(&pool, b.pool_base_token_reserves, b.pool_quote_token_reserves, b.user_quote_amount_in).unwrap();
            assert_eq!(gross_out, b.base_amount_out);
        }
        other => panic!("expected Buy, got {other:?}"),
    }
}

#[test]
fn rejects_data_shorter_than_a_discriminator() {
    assert_eq!(decode_event(&[1, 2, 3]), None);
}

#[test]
fn ignores_an_unrecognized_discriminator() {
    let mut data = decode_fixture(SELL_B64);
    data[0] ^= 0xFF;
    assert_eq!(decode_event(&data), None);
}

fn standard_pool() -> Pool {
    Pool {
        pool_bump: 254,
        index: 0,
        creator: Pubkey::new_unique(),
        base_mint: Pubkey::new_unique(),
        quote_mint: Pubkey::new_unique(),
        lp_mint: Pubkey::new_unique(),
        pool_base_token_account: Pubkey::new_unique(),
        pool_quote_token_account: Pubkey::new_unique(),
        lp_supply: 0,
        coin_creator: Pubkey::new_unique(),
        is_mayhem_mode: false,
        is_cashback_coin: false,
        virtual_quote_reserves: 0,
    }
}
