//! Real fixtures across three different pool configurations, fetched fresh
//! from mainnet during Group D research: SOL on the base side, SOL on the
//! quote side, and neither side SOL (a memecoin/USDC pool) — confirming
//! `ingest_pumpswap_trade`/`ingest_pumpswap_pool_created` correctly handle
//! all three rather than assuming a fixed base/quote convention (see
//! `crates/ingest/src/pumpswap.rs` module docs for why that assumption
//! would have been wrong: roughly half of ~20 sampled real pools have SOL
//! as base, half as quote).

use base64::Engine;
use momentum_core::adapter_contract::VenueAdapter;
use momentum_core::domain::{EventPayload, Venue};
use momentum_ingest::{ingest_pumpswap_pool_created, ingest_pumpswap_trade, EventContext};
use momentum_pumpswap::adapter::{Candidate, PumpSwapAdapter};
use momentum_pumpswap::Pool;
use solana_pubkey::Pubkey;
use std::str::FromStr;

/// Real SellEvent on pool CevkYxWmzwmRcpK4aFKBtSxSAXzq4K6oxfwqG8uXRTvv,
/// already byte-verified in crates/pumpswap/tests/events_test.rs. This
/// pool has SOL as base_mint.
const SELL_B64: &str = "Pi83CqUD3CpX6YdqAAAAAMWBcBsAAAAA6IguVvYCAADFgXAbAAAAALLplyteAAAAtDKKwRkAAADSr6oLktMCAEx0eKv/AgAAGQAAAAAAAABqBU/rAQAAAAUAAAAAAAAAFgFDYgAAAADibinA/QIAAMxt5l39AgAArSfSi1Hnc04iy5mSrX5kGQnSEQYKAOrKCOXNi3Pn5z/wbbbJRyF9WU8VAggkhTTV55MUnPuJW6akk69hEbXMoNjYPQfauFOfG34h2FuMlL4CkolAiJJrrU9r2th45KXlQ/JEa+wIfXa/GmNHhHDBze7QwHl3CDiNCExajHZwvgVKwvjQ3Vy8l+MonBl8tQYqVPPZVrnOblEV+WVnqlyz5qAPh45v7lcOwJc4HLGaiyZQwkxXISRSA51Uj6moxVZQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAIgTAAAAAAAAi4AhMQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
/// The real Pool account for that same pool (base_mint = wrapped SOL).
const SOL_AS_BASE_POOL_B64: &str = "8ZptBBGxbbz9AABwXCJuwGxQbmnlFLB25cSetKJ/vMSIgg26KU/MlRxAzAabiFf+q4GE+2h/Y0YYwDXaxDncGus7VZig8AAAAAABmHsP7dFJ+IwJp/5bgJ9wyvdcrplI3Tw9Vw3lOXwBtFEMW7RCvtcD4/Rcq24ByZi0uOg1iUWPmG2Fh3zgQQx3S2DKVZ3QYDMnO/5ZyZ9KoOb9lyD7+hBKfJu0qWYlBeF22quymlgPQotTHWTvJcysL4KRG4BoZ5ADbeC6Vriv8Q98pFh3YggAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==";

/// A real, currently-boosted Pool account (pool HskDSyGaQ7r2BnryHdCKP85BhKhJe5hgY1Pi65chYbXN,
/// base_mint = GvSXkTftiuvsRgpdbR4PedGspb1M6bkYGKgHUBAYpump, quote_mint =
/// wrapped SOL) — fetched fresh to prove the quote-side branch against a
/// real account, not just a synthetic one. Boost status doesn't matter
/// here: ingest_pumpswap_trade never reads is_standard()/virtual_quote_reserves,
/// only base_mint/quote_mint.
const SOL_AS_QUOTE_POOL_B64: &str = "8ZptBBGxbbz/AACOMWqt1JkzdiiGlEd7PgG/aPSwiEJqy6cjb9mRtRIBaOyP9C2YmePuS6y2cBFD6tR2HkKqkNqzzaukLY9Mz0pfBpuIV/6rgYT7aH9jRhjANdrEOdwa6ztVmKDwAAAAAAEmg0JLgsJh7R6oX07o7lPUKtuUcqK6F6dNZFhZPL3+ND77iWsaftIJaC5GruCz9dRstkkUgIYnoI7sjpCrgBTMF3gxLuQw0AvMcXgC9a2yWHoZUThFsRTA0HvvOQ2oMlHsQmtZ0AMAAJbtdKkT2xXd7azv1muzkVmzh01EhVo16/YAswoEbOfJAADIQR4YBAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==";

/// A real Pool account with neither side SOL (pool
/// 5nFmNMEWwZzQhSr6ZuWjJitd2pYhadE3HQdmEfXkciVk, memecoin base_mint vs a
/// USDC quote_mint) — proves the refusal is real-data-driven, not just a
/// synthetic Pubkey::new_unique() that happens not to match.
const NEITHER_SOL_POOL_B64: &str = "8ZptBBGxbbz/AABpSCuXTUHDQ3gFgU04wl4PagpX68RBkzP/+IxZNkXj8QRQo0xtm2P7amaPMYZ7uvZTO2bq+9JI6QLz77lSMB6vxvp6877brTo9ZfNqq8l0MbG75MLS9uDkfKYCA0UvXWHrix/mTUdEYuarjyAGkmLqtALkq2llVkgybr8jiBp1BMF4WNyfG3RbhuuXo4CZM0QPcbhtrmVJvaR7LP4hJ5CP6k2eYjE7rnzQZlVwEQhn34jUSNmCJ0/1/ksk75xSR4xpeBVUcQEAABzkxmA/6NwQ1yiO7VX722OenLFb/dbTK9pLh3iWpFVsAAB0LPqVAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==";

fn decode_fixture(b64: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(b64).unwrap()
}

fn ctx() -> EventContext {
    EventContext {
        id: "evt-pumpswap".to_string(),
        slot: 440_624_656,
        observed_at_ns: 1_787_291_991_000_000_000,
        signature: "5GWYCkt67vcSGiLbWBf1gq2PC2emazr9unrckcW5kcWE4xxe7zMYxQgmK3cc4o6SjaTjCB3JbZ6VaAYBXrr99roP".to_string(),
        instruction_index: 0,
        program_version: "pumpswap-layout-2026-08".to_string(),
    }
}

#[test]
fn converts_a_real_sell_on_a_sol_as_base_pool_to_usd() {
    let adapter = PumpSwapAdapter::new("pumpswap-layout-2026-08");
    let candidate = adapter.decode(&decode_fixture(SELL_B64)).expect("should decode");
    let pool = Pool::decode(&decode_fixture(SOL_AS_BASE_POOL_B64)).unwrap();
    assert_eq!(pool.base_mint, Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap());

    let event = ingest_pumpswap_trade(&candidate, &pool, 150.0, &ctx()).expect("should produce an event");

    assert_eq!(event.venue, Venue::PumpSwap);
    // The pool's quote_mint (not SOL) is the tracked token here — SOL is
    // the base leg in this real pool.
    assert_eq!(event.mint, pool.quote_mint.to_string());
    match event.payload {
        EventPayload::Sell { amount_usd, .. } => {
            // Real base_amount_in for this sell: 460,358,085 lamports
            // (verified exactly in crates/pumpswap/tests/events_test.rs).
            let expected = 460_358_085.0 / 1_000_000_000.0 * 150.0;
            assert!((amount_usd - expected).abs() < 1e-9, "{amount_usd} vs {expected}");
        }
        other => panic!("expected Sell, got {other:?}"),
    }
}

#[test]
fn converts_a_trade_on_a_real_sol_as_quote_pool_using_the_quote_leg() {
    let pool = Pool::decode(&decode_fixture(SOL_AS_QUOTE_POOL_B64)).unwrap();
    assert_eq!(pool.quote_mint, Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap());
    assert_ne!(pool.base_mint, Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap());

    // Trade amounts here are illustrative (no real trade fixture was
    // captured against this specific pool) — what's under real-data test
    // is the pool's own base_mint/quote_mint driving which leg
    // ingest_pumpswap_trade reads, not these particular numbers.
    let candidate = Candidate::Trade {
        pool: Pubkey::new_unique(),
        user: Pubkey::new_unique(),
        is_buy: true,
        base_amount: 1_000_000,
        quote_amount: 500_000_000,
        lp_fee: 0,
        protocol_fee: 0,
        coin_creator: Pubkey::new_unique(),
        coin_creator_fee: 0,
        can_boost: true,
    };

    let event = ingest_pumpswap_trade(&candidate, &pool, 150.0, &ctx()).expect("should produce an event");
    assert_eq!(event.mint, pool.base_mint.to_string());
    match event.payload {
        EventPayload::Buy { amount_usd, .. } => {
            let expected = 500_000_000.0 / 1_000_000_000.0 * 150.0;
            assert!((amount_usd - expected).abs() < 1e-9, "{amount_usd} vs {expected}");
        }
        other => panic!("expected Buy, got {other:?}"),
    }
}

#[test]
fn refuses_a_real_pool_with_neither_side_denominated_in_sol() {
    let pool = Pool::decode(&decode_fixture(NEITHER_SOL_POOL_B64)).unwrap();
    let sol = Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap();
    assert_ne!(pool.base_mint, sol);
    assert_ne!(pool.quote_mint, sol);

    let candidate = Candidate::Trade {
        pool: Pubkey::new_unique(),
        user: Pubkey::new_unique(),
        is_buy: true,
        base_amount: 1_000_000,
        quote_amount: 500_000_000,
        lp_fee: 0,
        protocol_fee: 0,
        coin_creator: Pubkey::new_unique(),
        coin_creator_fee: 0,
        can_boost: false,
    };
    assert!(ingest_pumpswap_trade(&candidate, &pool, 150.0, &ctx()).is_none());
}

#[test]
fn refuses_a_non_finite_or_negative_price() {
    let pool = Pool::decode(&decode_fixture(SOL_AS_BASE_POOL_B64)).unwrap();
    let candidate = Candidate::Trade {
        pool: Pubkey::new_unique(),
        user: Pubkey::new_unique(),
        is_buy: true,
        base_amount: 1_000_000,
        quote_amount: 500_000_000,
        lp_fee: 0,
        protocol_fee: 0,
        coin_creator: Pubkey::new_unique(),
        coin_creator_fee: 0,
        can_boost: false,
    };
    assert!(ingest_pumpswap_trade(&candidate, &pool, f64::NAN, &ctx()).is_none());
    assert!(ingest_pumpswap_trade(&candidate, &pool, -1.0, &ctx()).is_none());
}

#[test]
fn a_pool_created_candidate_values_the_sol_side_of_its_initial_liquidity() {
    // No real CreatePoolEvent fixture exists yet (crates/pumpswap/src/events.rs
    // module docs: pool creation proved too rare to capture live during
    // research). The mints reused here (base = SOL, quote = the real
    // non-SOL mint from SOL_AS_BASE_POOL_B64) are real; the liquidity
    // amounts are illustrative.
    let candidate = Candidate::PoolCreated {
        pool: Pubkey::new_unique(),
        base_mint: Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap(),
        quote_mint: Pubkey::from_str("BGDooTSsgDPw2denM7bUh8WiNAnEYTfhN4EqEz6E5KVa").unwrap(),
        creator: Pubkey::new_unique(),
        coin_creator: Pubkey::new_unique(),
        lp_mint: Pubkey::new_unique(),
        is_mayhem_mode: false,
        base_amount_in: 10_000_000_000,
        quote_amount_in: 1_000_000_000_000,
    };

    let event = ingest_pumpswap_pool_created(&candidate, 150.0, &ctx()).expect("should produce an event");
    assert_eq!(event.mint, "BGDooTSsgDPw2denM7bUh8WiNAnEYTfhN4EqEz6E5KVa");
    match event.payload {
        EventPayload::PoolCreated { exit_liquidity_usd, .. } => {
            let expected = 10_000_000_000.0 / 1_000_000_000.0 * 150.0;
            assert!((exit_liquidity_usd - expected).abs() < 1e-9);
        }
        other => panic!("expected PoolCreated, got {other:?}"),
    }
}

#[test]
fn a_trade_candidate_is_not_a_pool_created_event() {
    let candidate = Candidate::Trade {
        pool: Pubkey::new_unique(),
        user: Pubkey::new_unique(),
        is_buy: true,
        base_amount: 1,
        quote_amount: 1,
        lp_fee: 0,
        protocol_fee: 0,
        coin_creator: Pubkey::new_unique(),
        coin_creator_fee: 0,
        can_boost: false,
    };
    assert!(ingest_pumpswap_pool_created(&candidate, 150.0, &ctx()).is_none());
}
