//! Fixtures captured live from mainnet during Stage 1 research (see the
//! module docs in src/lib.rs for how they were verified).

use base64::Engine;
use momentum_pumpswap::{quote_buy_exact_quote_in, quote_sell, DecodeError, Pool, QuoteError};
use solana_pubkey::Pubkey;

/// A real Pool account, address A94B7zHr6nhn6FjUd8hnPQPeYyBaXvq7cR8dQD6KZAnY,
/// fetched live from mainnet. Its on-chain size (301 bytes) is larger than
/// the IDL's Pool struct (261 bytes) — the extra 40 bytes are zero-filled
/// Anchor reserve padding, confirmed byte-for-byte in this fixture.
const REAL_POOL_B64: &str = "8ZptBBGxbbz+AACLm8qnfjMlp/ztOdhJn5hQj+f7pjHkeM8mH7ZV8EL7qQabiFf+q4GE+2h/Y0YYwDXaxDncGus7VZig8AAAAAABCnwU1FmE4ulF782BjPZpGrubAYzow8aF5pJjJJUUnifrxDcKAAwPZWcItxaEJZ9nuY9QCeBENoYhNtvD6128LCri5VqXIUXREZL7pYOviL4IQnNO4oiUxLNmXvEaXsfRJPLQ/+E75tarQYrexic02Il6fAgE0HEvyWRuCyBoqTOekXL9kQMAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==";

fn decode_fixture(b64: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(b64).unwrap()
}

#[test]
fn decodes_a_real_pool_including_its_anchor_reserve_padding() {
    let data = decode_fixture(REAL_POOL_B64);
    assert_eq!(data.len(), 301, "must match the real account's on-chain space exactly");

    let pool = Pool::decode(&data).unwrap();
    assert_eq!(pool.base_mint.to_string(), "So11111111111111111111111111111111111111112");
    assert!(!pool.is_mayhem_mode);
    assert!(!pool.is_cashback_coin);
    assert_eq!(pool.virtual_quote_reserves, 0);
    assert!(pool.is_standard());
}

#[test]
fn rejects_truncated_and_mislabeled_accounts() {
    let data = decode_fixture(REAL_POOL_B64);
    assert_eq!(Pool::decode(&data[..40]), Err(DecodeError::TooShort));

    let mut corrupted = data.clone();
    corrupted[0] ^= 0xFF;
    assert_eq!(Pool::decode(&corrupted), Err(DecodeError::DiscriminatorMismatch));
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

/// Real mainnet `SellEvent` on pool A94B7zHr6nhn6FjUd8hnPQPeYyBaXvq7cR8dQD6KZAnY:
/// selling 801,789,442 base tokens against pre-trade reserves
/// (base=125,119,821,337, quote=136,157,057,613,312) produced a gross
/// `quote_amount_out` of exactly 866,962,315,465 — bit-exact, not just
/// within rounding.
#[test]
fn quote_sell_matches_a_real_mainnet_trade_exactly() {
    let pool = standard_pool();
    let quoted = quote_sell(&pool, 125_119_821_337, 136_157_057_613_312, 801_789_442).unwrap();
    assert_eq!(quoted, 866_962_315_465);
}

/// Real mainnet `BuyEvent` (`buy_exact_quote_in`) on a non-boost pool:
/// spending the net (post-fee) `user_quote_amount_in` of 5,067,662,825,055
/// against pre-trade reserves (base=146,736,146,314,
/// quote=625,490,569,997,104) produced `base_amount_out` of exactly
/// 1,179,287,296 — the formula's continuous result rounds down to that
/// same integer.
#[test]
fn quote_buy_matches_a_real_mainnet_trade_exactly() {
    let pool = standard_pool();
    let quoted = quote_buy_exact_quote_in(&pool, 146_736_146_314, 625_490_569_997_104, 5_067_662_825_055).unwrap();
    assert_eq!(quoted, 1_179_287_296);
}

/// A real `buy_exact_quote_in` trade on a *boosted* pool
/// (`virtual_quote_reserves` nonzero, `can_boost = true`) came out ~9% off
/// from this plain formula when checked during research — confirming
/// boosted pools must be refused, not silently mispriced.
#[test]
fn refuses_to_price_a_boosted_pool() {
    let mut pool = standard_pool();
    pool.virtual_quote_reserves = 17_584_505_288;
    assert!(!pool.is_standard());
    assert_eq!(
        quote_buy_exact_quote_in(&pool, 146_736_146_314, 625_490_569_997_104, 1_000_000_000),
        Err(QuoteError::NotStandardPool)
    );
    assert_eq!(quote_sell(&pool, 146_736_146_314, 625_490_569_997_104, 1_000_000_000), Err(QuoteError::NotStandardPool));
}

#[test]
fn refuses_to_price_a_mayhem_or_cashback_pool() {
    let mut mayhem = standard_pool();
    mayhem.is_mayhem_mode = true;
    assert!(!mayhem.is_standard());

    let mut cashback = standard_pool();
    cashback.is_cashback_coin = true;
    assert!(!cashback.is_standard());
}

#[test]
fn round_tripping_a_feeless_buy_then_sell_never_profits_from_rounding() {
    // Deliberately proportionate reserves/trade size here (unlike the real
    // fixtures above, which have an extreme base:quote ratio where a tiny
    // trade's floor rounding on the way out gets amplified on the way
    // back) so this checks the formula's own consistency, not integer
    // rounding artifacts of one specific real pool.
    let pool = standard_pool();
    let base_reserves = 1_000_000_000_000u64;
    let quote_reserves = 30_000_000_000u64;
    let spend = 1_000_000_000u64;

    let base_out = quote_buy_exact_quote_in(&pool, base_reserves, quote_reserves, spend).unwrap();
    let quote_back = quote_sell(&pool, base_reserves - base_out, quote_reserves + spend, base_out).unwrap();

    // Floor rounding on both legs must never hand the trader a profit out
    // of thin air; a small loss to rounding is expected and fine.
    assert!(quote_back <= spend, "round trip must not profit from rounding: got {quote_back} back from {spend}");
    assert!(
        quote_back.abs_diff(spend) <= 1,
        "with proportionate reserves the round-trip loss should be at most 1 unit, got {quote_back} vs {spend}"
    );
}
