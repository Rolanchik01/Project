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
//! - CREATE_POOL_B64: a real `CreatePoolEvent` from signature
//!   4u3ZVmp4addMApwXHVtRqyawfkArCPAdMZVUJ2roqxPtzcTPgY1Fatu8oNCyKkjTeYgmrXWxCvsmBqrRJRxeUT5E
//!   — captured live (a ~250-signature historical scan had found none;
//!   this is the first real one seen). Base side is wrapped SOL, quote
//!   side a non-SOL token — confirms the base-side-SOL case of the
//!   defensive `sol_side()` handling in `crates/ingest`.
//! - WITHDRAW_B64: a real `WithdrawEvent` from signature
//!   5ThD4QR63tWxkRJ13RpN7bXXcmFkEx7iwWM4voLbaj69jVny2thvEpptp46g6jmMfk6ReiCrhgxd6RkxGMZ6bfaJ
//!   on pool CumioQrRqWyLv2Xdge2aFVgTgVrBZuF7y2PNTuDQfhXg — burned almost the
//!   entire LP supply in one withdrawal (`lp_token_amount_in` within 100
//!   raw units of `lp_mint_supply`), and `base_amount_out` came out
//!   within single-digit dust of `pool_base_token_reserves` — both
//!   independently confirming this is a near-total drain and that the
//!   reserve fields are pre-withdrawal, same convention as Buy/Sell.
//! - DEPOSIT_B64: a real `DepositEvent` from signature
//!   5MCWio9g2Zkry3jETrovpY7iCP6iE1WZNw1Crgs5khs9aGn7A1pGZ2W37kujZmBwGGxKpBbHSwfWpSQixc9LUk2W
//!   on pool 3Nfu1VWsoUbRwS9sXmMLzGJfVp2dWHf6Xw75JoSvvwZQ — a balanced
//!   two-sided add: `base_amount_in`/`pool_base_token_reserves` and
//!   `quote_amount_in`/`pool_quote_token_reserves` land within ~1% of
//!   each other, exactly as expected for a deposit sized to match the
//!   pool's existing ratio. A second, shorter (105-byte) real event was
//!   also seen under the same discriminator, bundled with a `SellEvent`
//!   in the same transaction — see `events.rs::DepositEvent`'s doc
//!   comment for why it's deliberately not decoded.

use base64::Engine;
use momentum_pumpswap::events::{decode_event, PumpSwapEvent};
use momentum_pumpswap::{quote_buy_exact_quote_in, quote_sell, Pool};
use solana_pubkey::Pubkey;
use std::str::FromStr;

const SELL_B64: &str = "Pi83CqUD3CpX6YdqAAAAAMWBcBsAAAAA6IguVvYCAADFgXAbAAAAALLplyteAAAAtDKKwRkAAADSr6oLktMCAEx0eKv/AgAAGQAAAAAAAABqBU/rAQAAAAUAAAAAAAAAFgFDYgAAAADibinA/QIAAMxt5l39AgAArSfSi1Hnc04iy5mSrX5kGQnSEQYKAOrKCOXNi3Pn5z/wbbbJRyF9WU8VAggkhTTV55MUnPuJW6akk69hEbXMoNjYPQfauFOfG34h2FuMlL4CkolAiJJrrU9r2th45KXlQ/JEa+wIfXa/GmNHhHDBze7QwHl3CDiNCExajHZwvgVKwvjQ3Vy8l+MonBl8tQYqVPPZVrnOblEV+WVnqlyz5qAPh45v7lcOwJc4HLGaiyZQwkxXISRSA51Uj6moxVZQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAIgTAAAAAAAAi4AhMQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const BUY_B64: &str = "Z/RSHyz1d3dX6YdqAAAAACnNDFAAAAAA5EbAdUoHAAAAAAAAAAAAANdmsm06CQAAoagAFRoAAACa2MiiC1cCAORGwHVKBwAAGQAAAAAAAABsJPamBAAAAAUAAAAAAAAAfDox7gAAAABoDI+HSQcAAPznmOBEBwAA15MN77NI2BvPq1rfmk6byNA1YkD3xhagq24IZS/CmgsktWboeZxQnNFt127mhc5ARt1Pl25ah3bTqI72KULdxH1S8W8zWN67NUYyCEvq6J4+en+7LQpFm3aQ+2o+WB3hQWnrsZrcBHyyEI+N4IqIqEDNSn09Vhmk7GpV38jrPC5KwvjQ3Vy8l+MonBl8tQYqVPPZVrnOblEV+WVnqlyz5j/Ki0rjxmmlK2xlUmY3RO4xoVzkWz8cWd44M+FPGuP/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABs9yTAAAAAASAAAAYnV5X2V4YWN0X3F1b3RlX2luAAAAAAAAAAAAAAAAAAAAAIgTAAAAAAAAPp0YdwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const CREATE_POOL_B64: &str = "sTEM0qB2p3TnmYhqAAAAAAAA8EtDhVFpuI0LhlAYp8Ez30aC81gwNL9APNh4IP7WY2oGm4hX/quBhPtof2NGGMA12sQ53BrrO1WYoPAAAAAAAfskbpTzcG/jiCaYABBdWsFNdiCpojv/70dyqVYA6NVACQaA+B/jEAAAAAAgPYh5LQAAgPgf4xAAAAAAID2IeS0AAGQAAAAAAAAAzvNWY7sBAABq81ZjuwEAAPs5AlZd65ScxVl2YVdsfdqEAUqXAvRQkRVd+mXElPbJJLUqDzy3uiHUSxMyiTlB7FOxBHznAKv185xATOvoF8kTgw7qI/DADgDgW9cmhEeA56BwdY7gw7gOkfv+4DHKT0vxhrmBCCiaHCfEygxAIAgaWh7lUkoKF0xY7ZS4yfne2/BLQ4VRabiNC4ZQGKfBM99GgvNYMDS/QDzYeCD+1mNqAA==";
const WITHDRAW_B64: &str = "FgmFGqAsR8Cva4lqAAAAAHQqEQNoAgAAVAR3PQAAAACzRAuyqAAAAAAAAAAAAAAAAAAAAAAAAACP2A8CGAAAAOIVqLrmQQAAi9gPAhgAAAAvC6i65kEAANgqEQNoAgAAsPVS+OjVGIRWr8xyIA1mXmnNi0XgdyqWG21OAdOSkXNvqQ650oQopAr8jThZ0ep6WRY+sA5fx0BNKfJRXuPFMNq/Aq8B1ATQ/4gN4p23vatmsQM4iZQjJaiLGC1xtQTau8XxTkZZb1Vxzo7jJ+bVWMhovQAE8jaCSo7oV4O1z13alI4FZFp0SA1/sd789lLyoLdXN7FPTpzTKG4hRTAqAg==";
const DEPOSIT_B64: &str = "ePg9Ux+Oa5ARbYlqAAAAAJYx+goAAAAAoDegAAAAAAAmUUo5BgAAAKA3oAAAAAAAoO7p6p+aAwAPdq7cgAEAADcGZssN7w4AGbmYAAAAAABGtRXtBQAAAI/ZP7CpGwAAI0SNLcdUU0SYKEnd/8EpJqykaqmbVg36WJTlfthnEie/QBdIUxbev9YUjqy5/LWQEqzCvYhldw06ICYPCxqKTL8fc2RWVI81yPJkto5cbkXnoZidyI6ztikyQz+ID9HJ18APw0jIACyS74uch2G4SBVPdeawhLdBuMY/VZkerQRSor+xxqfdOenryzLU3epnPz85zkG0FtidPNdloAKDCA==";

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
fn decodes_a_real_create_pool_event() {
    let data = decode_fixture(CREATE_POOL_B64);
    let event = decode_event(&data).expect("should decode as a known event");
    match event {
        PumpSwapEvent::CreatePool(c) => {
            assert_eq!(c.pool, Pubkey::from_str("4qYJkETMAnGmzbeWakoED8im3q9mesAMUFjfUsJdJxSw").unwrap());
            assert_eq!(c.creator, Pubkey::from_str("HB1QqxzPVNs7m5PeWbxfsCDDXwccTrgjqpmUfsTe9rAm").unwrap());
            assert_eq!(c.base_mint, Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap());
            assert_eq!(c.quote_mint, Pubkey::from_str("HuMZ4SrjXc75NGbe62ReD9yHF6EHgFPWmLeGY3nn3oxK").unwrap());
            assert_eq!(c.lp_mint, Pubkey::from_str("DCBzUYnb5sGcJcEFdWzMjUiaZG1VVnjakX8kv4gDYsNJ").unwrap());
            assert_eq!(c.coin_creator, Pubkey::from_str("HB1QqxzPVNs7m5PeWbxfsCDDXwccTrgjqpmUfsTe9rAm").unwrap());
            assert_eq!(c.base_amount_in, 72_530_000_000);
            assert_eq!(c.quote_amount_in, 50_000_000_000_000);
            assert!(!c.is_mayhem_mode);
        }
        other => panic!("expected CreatePool, got {other:?}"),
    }
}

#[test]
fn decodes_a_real_withdraw_event_that_drains_nearly_the_entire_pool() {
    let data = decode_fixture(WITHDRAW_B64);
    let event = decode_event(&data).expect("should decode as a known event");
    match event {
        PumpSwapEvent::Withdraw(w) => {
            assert_eq!(w.pool, Pubkey::from_str("CumioQrRqWyLv2Xdge2aFVgTgVrBZuF7y2PNTuDQfhXg").unwrap());
            assert_eq!(w.user, Pubkey::from_str("8WsmeYfYAAu3d4Hf3xt86e38ZGQYANH9Syh4e3mZ6iAT").unwrap());
            assert_eq!(w.lp_token_amount_in, 2_645_751_310_964);
            assert_eq!(w.pool_base_token_reserves, 103_113_808_015);
            assert_eq!(w.pool_quote_token_reserves, 72_459_229_861_346);
            assert_eq!(w.base_amount_out, 103_113_808_011);
            assert_eq!(w.quote_amount_out, 72_459_229_858_607);
            assert_eq!(w.lp_mint_supply, 2_645_751_311_064);

            // Every byte of the body was consumed decoding these 8 fields
            // (11 numeric IDL fields collapsed into exactly the ones kept
            // here, 5 pubkeys) — unlike Buy/Sell/Deposit, this event's
            // wire layout matches the vendored IDL's declared fields
            // exactly, with nothing left over and nothing missing.
            assert_eq!(w.base_amount_out, w.pool_base_token_reserves - 4);
            assert_eq!(w.lp_mint_supply - w.lp_token_amount_in, 100);
        }
        other => panic!("expected Withdraw, got {other:?}"),
    }
}

#[test]
fn decodes_a_real_deposit_event_that_matches_the_pools_existing_ratio() {
    let data = decode_fixture(DEPOSIT_B64);
    let event = decode_event(&data).expect("should decode as a known event");
    match event {
        PumpSwapEvent::Deposit(d) => {
            assert_eq!(d.pool, Pubkey::from_str("3Nfu1VWsoUbRwS9sXmMLzGJfVp2dWHf6Xw75JoSvvwZQ").unwrap());
            assert_eq!(d.user, Pubkey::from_str("DsZZ43xtsiL1aWYZYyWG2iQq3DnhALjCx9uamR4jyVYP").unwrap());
            assert_eq!(d.lp_token_amount_out, 184_168_854);
            assert_eq!(d.pool_base_token_reserves, 1_652_969_862_671);
            assert_eq!(d.pool_quote_token_reserves, 4_203_492_200_023_607);
            assert_eq!(d.base_amount_in, 10_008_857);
            assert_eq!(d.quote_amount_in, 25_452_459_334);
            assert_eq!(d.lp_mint_supply, 30_415_620_397_455);

            // A balanced two-sided deposit adds both sides in the same
            // proportion the pool already has — checked here as an
            // independent self-consistency signal, not just "the bytes
            // happened to parse without erroring".
            let base_ratio = d.base_amount_in as f64 / d.pool_base_token_reserves as f64;
            let quote_ratio = d.quote_amount_in as f64 / d.pool_quote_token_reserves as f64;
            assert!((base_ratio - quote_ratio).abs() / base_ratio < 0.01, "base_ratio={base_ratio} quote_ratio={quote_ratio}");
        }
        other => panic!("expected Deposit, got {other:?}"),
    }
}

/// The 105-byte real event mentioned in `events.rs::DepositEvent`'s doc
/// comment — same discriminator, genuinely shorter body. `decode_deposit`
/// must refuse it (`None`), not partially decode garbage from truncated
/// reads.
#[test]
fn refuses_to_decode_the_shorter_real_deposit_variant() {
    let data = decode_fixture("ePg9Ux+Oa5AdKcVTmOadX86TFqbu//ySVsDSjeZQbN8vnM4IarH5LwHG+nrzvtutOj1l82qryXQxsbvkwtL24OR8pgIDRS9dYdooHgAAAAAAiqdbauO6scX3N9zKsR2fpMXJkxv2l4VWylZnpLqDpv0=");
    assert_eq!(decode_event(&data), None);
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
