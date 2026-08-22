//! Pure math edge cases for `lamports_to_usd`/`is_wrapped_sol` — no
//! on-chain fixtures needed here, these don't depend on any protocol byte
//! layout. Added after an independent review caught that the original
//! implementation accepted `sol_usd_price == 0.0` (a common "price feed
//! failed open" default) and never checked the multiplication result for
//! overflow — both would have let a broken price feed silently corrupt a
//! risk snapshot with zero or infinite USD amounts instead of being
//! refused.

use momentum_ingest::price::{is_wrapped_sol, lamports_to_usd, WRAPPED_SOL_MINT};
use solana_pubkey::Pubkey;
use std::str::FromStr;

#[test]
fn wrapped_sol_mint_constant_matches_the_real_base58_address() {
    assert_eq!(WRAPPED_SOL_MINT, Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap());
    assert!(is_wrapped_sol(&WRAPPED_SOL_MINT));
    assert!(!is_wrapped_sol(&Pubkey::new_unique()));
}

#[test]
fn converts_one_sol_at_a_round_price() {
    assert_eq!(lamports_to_usd(1_000_000_000, 150.0), Some(150.0));
}

#[test]
fn refuses_a_zero_price_rather_than_reporting_a_zero_value_trade() {
    // A price feed that fails open to exactly 0.0 must be refused just
    // like a negative or NaN price — silently converting a real trade to
    // $0 would be worse than refusing it outright.
    assert_eq!(lamports_to_usd(1_000_000_000, 0.0), None);
}

#[test]
fn refuses_a_negative_or_non_finite_price() {
    assert_eq!(lamports_to_usd(1_000_000_000, -1.0), None);
    assert_eq!(lamports_to_usd(1_000_000_000, f64::NAN), None);
    assert_eq!(lamports_to_usd(1_000_000_000, f64::INFINITY), None);
}

#[test]
fn refuses_a_price_extreme_enough_to_overflow_the_conversion_to_infinity() {
    // sol_usd_price itself is finite and positive here, so only the
    // post-multiplication finiteness check catches this: 2 SOL at
    // f64::MAX per SOL overflows to +inf.
    assert!(f64::MAX.is_finite());
    assert_eq!(lamports_to_usd(2_000_000_000, f64::MAX), None);
}
