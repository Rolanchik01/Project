//! Real `getAccountInfo` snapshots of Pyth's SOL/USD `PriceUpdateV2`
//! account (`7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE`), captured live
//! from `https://api.mainnet-beta.solana.com` a minute apart during Stage 1
//! research — not hand-constructed, not derived from the SDK's struct
//! definitions alone (those don't publish the wire discriminator or the
//! `VerificationLevel` tag byte). Both snapshots' embedded feed id matches
//! Pyth's own Hermes API response for `Crypto.SOL/USD`
//! (`ef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d`)
//! byte-for-byte, and both prices are in the same plausible range with
//! `publish_time` within seconds of when each was actually fetched,
//! confirming this is a live-updating feed, not a stale/frozen account.

use base64::Engine;
use momentum_ingest::price_feed::{decode_sol_usd_update, PYTH_RECEIVER_PROGRAM_ID};
use solana_pubkey::Pubkey;
use std::str::FromStr;

const SNAPSHOT_1_B64: &str = "IvEjY51+9M1gMUcENA3t3zcf1CRyFI8kjp0abRpesqw6zYt/1dayQwHvDYtv2izrpB2hXUCV0do5Kg0vjtDGx7wPTPrIwoC1bSgCbzECAAAA5gA0AAAAAAD4////L2mJagAAAAAvaYlqAAAAAKBE4zACAAAAjNVDAAAAAABSdUcaAAAAAAA=";

fn decode_fixture(b64: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(b64).unwrap()
}

fn receiver_program() -> Pubkey {
    Pubkey::from_str(PYTH_RECEIVER_PROGRAM_ID).unwrap()
}

#[test]
fn decodes_a_real_sol_usd_price_update() {
    let owner = receiver_program();
    let data = decode_fixture(SNAPSHOT_1_B64);
    let update = decode_sol_usd_update(&owner, &data).expect("should decode as a real PriceUpdateV2");

    // price_raw = 9419293224, exponent = -8, independently computed from
    // the same captured bytes outside this decoder.
    assert!((update.price - 94.192_932_24).abs() < 1e-9);
    assert!((update.conf - 0.034_081_02).abs() < 1e-9);
    assert_eq!(update.publish_time, 1_787_390_255);
}

#[test]
fn a_second_snapshot_a_minute_later_shows_a_fresher_publish_time_and_a_plausible_price() {
    let owner = receiver_program();
    // Captured a few minutes after SNAPSHOT_1_B64, confirming the account
    // is actually being updated live and not just replaying a frozen value.
    let data = decode_fixture("IvEjY51+9M1gMUcENA3t3zcf1CRyFI8kjp0abRpesqw6zYt/1dayQwHvDYtv2izrpB2hXUCV0do5Kg0vjtDGx7wPTPrIwoC1bbRVMTICAAAAOyc5AAAAAAD4////2GqJagAAAADXaolqAAAAAFTfBzECAAAA8HxDAAAAAADreUcaAAAAAAA=");
    let update = decode_sol_usd_update(&owner, &data).expect("should decode as a real PriceUpdateV2");

    assert!(update.publish_time > 1_787_390_255, "second snapshot should be newer than the first");
    // SOL has plausibly traded well within this band throughout its
    // history; this just guards against a totally broken decode (e.g. a
    // sign or scale error) rather than pinning an exact market price.
    assert!(update.price > 1.0 && update.price < 10_000.0);
}

#[test]
fn rejects_data_from_the_wrong_owner() {
    let wrong_owner = Pubkey::new_unique();
    let data = decode_fixture(SNAPSHOT_1_B64);
    assert_eq!(decode_sol_usd_update(&wrong_owner, &data), None);
}

#[test]
fn rejects_truncated_data() {
    let owner = receiver_program();
    let data = decode_fixture(SNAPSHOT_1_B64);
    assert_eq!(decode_sol_usd_update(&owner, &data[..40]), None);
}

#[test]
fn rejects_a_mismatched_discriminator() {
    let owner = receiver_program();
    let mut data = decode_fixture(SNAPSHOT_1_B64);
    data[0] ^= 0xFF;
    assert_eq!(decode_sol_usd_update(&owner, &data), None);
}

#[test]
fn rejects_a_feed_id_that_is_not_sol_usd() {
    let owner = receiver_program();
    let mut data = decode_fixture(SNAPSHOT_1_B64);
    // The feed id sits right after the 8-byte discriminator, the 32-byte
    // write_authority, and the 1-byte verification tag (Full, no extra
    // payload) — offset 41, confirmed by `decodes_a_real_sol_usd_price_update`
    // already parsing this exact fixture successfully.
    data[41] ^= 0xFF;
    assert_eq!(decode_sol_usd_update(&owner, &data), None);
}
