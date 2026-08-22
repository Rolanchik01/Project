//! Fixtures captured live from mainnet during Stage 1 research (see the
//! module docs in src/lib.rs for how they were captured and verified).

use base64::Engine;
use momentum_pump::{
    quote_buy_exact_quote_in, quote_buy_exact_token_out, quote_sell, BondingCurve, DecodeError,
    NATIVE_SOL_QUOTE_SENTINEL, QuoteError,
};
use solana_pubkey::Pubkey;

/// A real, untouched (no trades yet) BondingCurve account fetched live from
/// mainnet at slot 440530730ish, address 113P6U1rwkHf69wyGT4qc9JNJfW1fXrKHYsU2Yy4goF.
const REAL_FRESH_CURVE_B64: &str = "F7f4N2DYrGAAENhH488DAAGsI/wGAAAAAHjF+1HRAgABAAAAAAAAAACAxqR+jQMAAI7lsMJuRMrT6KfN5AuRku0L5eWtl+NXbyxEgyJtONGfAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==";

fn decode_fixture(b64: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(b64).unwrap()
}

#[test]
fn decodes_a_real_fresh_curve_matching_the_known_pump_fun_launch_constants() {
    let data = decode_fixture(REAL_FRESH_CURVE_B64);
    assert_eq!(data.len(), 115, "must match the real account's on-chain space exactly");

    let curve = BondingCurve::decode(&data).unwrap();
    assert_eq!(curve.virtual_token_reserves, 1_073_000_000_000_000);
    assert_eq!(curve.real_token_reserves, 793_100_000_000_000);
    assert_eq!(curve.token_total_supply, 1_000_000_000_000_000);
    assert!(!curve.complete);
    assert!(!curve.is_mayhem_mode);
    assert!(!curve.is_cashback_coin);
    assert_eq!(curve.quote_mint, NATIVE_SOL_QUOTE_SENTINEL);
    assert!(curve.is_standard());
}

#[test]
fn rejects_truncated_and_mislabeled_accounts() {
    let data = decode_fixture(REAL_FRESH_CURVE_B64);
    assert_eq!(BondingCurve::decode(&data[..40]), Err(DecodeError::TooShort));

    let mut corrupted = data.clone();
    corrupted[0] ^= 0xFF;
    assert_eq!(BondingCurve::decode(&corrupted), Err(DecodeError::DiscriminatorMismatch));
}

fn standard_curve(virtual_token_reserves: u64, virtual_quote_reserves: u64) -> BondingCurve {
    BondingCurve {
        virtual_token_reserves,
        virtual_quote_reserves,
        real_token_reserves: virtual_token_reserves,
        real_quote_reserves: 0,
        token_total_supply: 1_000_000_000_000_000,
        complete: false,
        creator: Pubkey::new_unique(),
        is_mayhem_mode: false,
        is_cashback_coin: false,
        quote_mint: NATIVE_SOL_QUOTE_SENTINEL,
    }
}

/// Back-solved from a real mainnet `sell` (TradeEvent) on mint
/// ExXQP6ZatSTMXpP7jaWcN76k8V9vwzFUD5rPNfWGpump: a sell of 12,550,942 tokens
/// left the curve at virtual_token_reserves=815,317,187,863,415,
/// virtual_quote_reserves=39,481,566,885, and the trader received
/// sol_amount=607 lamports (gross, before the 6-lamport fee taken
/// separately). The pre-trade reserves aren't directly observable, so this
/// reconstructs them from the post-trade state via `old_vqr = k / old_vtr`
/// (floor). Despite that reconstruction step, quote_sell reproduces the
/// real payout exactly — this only holds because quote_sell computes
/// `token_in * virtual_quote_reserves / (virtual_token_reserves +
/// token_in)` directly; going through an intermediate k (as an earlier,
/// wrong version of this function did) landed the floor one lamport off,
/// which is exactly the bug this test caught (see the PumpSwap sibling
/// crate for the same pitfall confirmed against two independent trades).
#[test]
fn quote_sell_matches_a_real_mainnet_trade_exactly() {
    let new_vtr: u128 = 815_317_187_863_415;
    let new_vqr: u128 = 39_481_566_885;
    let token_amount: u64 = 12_550_942;
    let real_sol_amount: u64 = 607;

    let k = new_vtr * new_vqr;
    let old_vtr = new_vtr - token_amount as u128;
    let old_vqr = k / old_vtr; // floor, same rounding the real program used

    let curve = standard_curve(old_vtr as u64, old_vqr as u64);
    let quoted = quote_sell(&curve, token_amount).unwrap();
    assert_eq!(quoted, real_sol_amount, "reconstructed pre-trade state should reproduce the real payout exactly");
}

#[test]
fn quote_buy_and_quote_sell_move_the_same_constant_product_invariant() {
    let curve = standard_curve(1_073_000_000_000_000, 30_000_000_000);
    let spend = 1_000_000_000u64; // 1 SOL
    let tokens_out = quote_buy_exact_quote_in(&curve, spend).unwrap();

    // Buying via exact-token-out for the same tokens_out must ask for
    // (approximately, modulo integer rounding) the same quote_in.
    let quote_in_for_same_tokens = quote_buy_exact_token_out(&curve, tokens_out).unwrap();
    let diff = quote_in_for_same_tokens.abs_diff(spend);
    assert!(diff <= 1, "exact-token-out and exact-quote-in must agree to within 1 lamport, got diff={diff}");

    // This formula has no fee in it (fees are layered on separately by the
    // caller, per the module docs), so a frictionless round trip — buy
    // tokens_out, then immediately sell exactly tokens_out back — must
    // return to very nearly the same quote amount, not a systematically
    // worse one: price impact without a fee is symmetric, it does not by
    // itself create a spread.
    let curve_after_buy = standard_curve(
        curve.virtual_token_reserves - tokens_out,
        curve.virtual_quote_reserves + spend,
    );
    let sol_back = quote_sell(&curve_after_buy, tokens_out).unwrap();
    assert!(
        sol_back.abs_diff(spend) <= 1,
        "a feeless round trip should return to within 1 lamport of the original spend, got {sol_back} vs {spend}"
    );
}

#[test]
fn refuses_to_price_a_mayhem_mode_curve() {
    let mut curve = standard_curve(1_073_000_000_000_000, 30_000_000_000);
    curve.is_mayhem_mode = true;
    assert!(!curve.is_standard());
    assert_eq!(quote_buy_exact_quote_in(&curve, 1_000_000_000), Err(QuoteError::NotStandardCurve));
    assert_eq!(quote_sell(&curve, 1_000_000), Err(QuoteError::NotStandardCurve));
}

#[test]
fn refuses_to_price_a_non_sol_quote_mint_curve() {
    let mut curve = standard_curve(1_073_000_000_000_000, 30_000_000_000);
    curve.quote_mint = Pubkey::new_unique();
    assert!(!curve.is_standard());
    assert_eq!(quote_buy_exact_quote_in(&curve, 1_000_000_000), Err(QuoteError::NotStandardCurve));
}

#[test]
fn refuses_to_price_a_graduated_curve() {
    let mut curve = standard_curve(1_073_000_000_000_000, 30_000_000_000);
    curve.complete = true;
    assert_eq!(quote_buy_exact_quote_in(&curve, 1_000_000_000), Err(QuoteError::AlreadyGraduated));
    assert_eq!(quote_sell(&curve, 1_000_000), Err(QuoteError::AlreadyGraduated));
}

#[test]
fn refuses_a_buy_that_would_drain_more_tokens_than_the_curve_actually_holds() {
    let curve = standard_curve(1_073_000_000_000_000, 30_000_000_000);
    // real_token_reserves == virtual_token_reserves in this fixture, so
    // asking for the full virtual supply must be rejected as insufficient
    // liquidity rather than silently draining past what the curve holds.
    let result = quote_buy_exact_token_out(&curve, curve.real_token_reserves);
    assert_eq!(result, Err(QuoteError::InsufficientLiquidity));
}
