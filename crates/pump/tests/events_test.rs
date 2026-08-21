//! Fixtures captured live from mainnet during Stage 1 research: all three
//! extracted programmatically from raw RPC responses (never hand-copied —
//! an earlier hand-copy of one of these truncated mid-string and cost real
//! debugging time before the real cause turned up).
//!
//! - CREATE_B64 + BUY_TRADE_B64: the same transaction
//!   (3ssTh78rz8SbQJNTnGVTQLDDQ5bfHc55rgT9ARZ4xhKKfYSN4EHfoDXPMot2r16V6GDdDQMdShaAPnNch4gQ31Eq)
//!   — a CreateV2 immediately followed by a same-tx dev BuyV2, on mint
//!   3xM2iMg4RZBuzdFpvwYab9cUaVpniuHgLUZRg33ipump.
//! - SELL_TRADE_B64: a real SellV2
//!   (3sGrXLsz4ZPtNdQgiCSvTaV79iBtijqpZpsGBQTXe95GpZHFQkGsuP4ro9RzogtCpkbfiPvRGTXhsQ4unjZvU5Du)
//!   on mint ExXQP6ZatSTMXpP7jaWcN76k8V9vwzFUD5rPNfWGpump.
//! - COMPLETE_B64: a real `CompleteEvent` (graduation) from signature
//!   5QdfLUr9xwBHQ6LQZtU7ksRbGGeuMkRGcv7y4CsnGjWNmENkAiWmkVd5rZDccSnRs1GDggCAgTk9Kw5kCeeAKsL
//!   on mint 4yGJ5ynrB4QX7AiRps1DTAzdXwEf15NvrRF5gouEpump — captured live
//!   (historical signature search had come up empty; this is the first
//!   real one seen). `quote_mint` is the all-zero sentinel, same as a
//!   SOL-quoted `CreateEvent`/`TradeEvent`.

use base64::Engine;
use momentum_pump::events::{decode_event, PumpEvent};
use momentum_pump::NATIVE_SOL_QUOTE_SENTINEL;
use solana_pubkey::Pubkey;
use std::str::FromStr;

const CREATE_B64: &str = "G3KpTd7rY3YEAAAAMTAwawQAAAAxMDBrUAAAAGh0dHBzOi8vaXBmcy5pby9pcGZzL2JhZmtyZWlkcHh5NWkyNXJ2M3Ezb2VvaG5mbXJhZDN2cmJndDVrYWtsbW9ucnI0ZTN3eWxtbmd0Nmg0K+T3P/I+e0zWu6OEE56W088qbp3nm/YbCPoxqcqnl58AAAvh7GU8u74DNCowKDSNy9q8qObVSiwzKs3NxZsNDo7lsMJuRMrT6KfN5AuRku0L5eWtl+NXbyxEgyJtONGfjuWwwm5EytPop83kC5GS7Qvl5a2X41dvLESDIm040Z8NWSpqAAAAAAAQ2EfjzwMAAKwj/AYAAAAAeMX7UdECAACAxqR+jQMABt324e51j94YQl285GzN2rYa/E2DuQ0n/r35KNihi/wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAKwj/AYAAAA=";
const BUY_TRADE_B64: &str = "vdt/007mYe4r5Pc/8j57TNa7o4QTnpbTzypuneeb9hsI+jGpyqeXn7ntHgQAAAAAauzuaD4CAAABjuWwwm5EytPop83kC5GS7Qvl5a2X41dvLESDIm040Z8NWSpqAAAAALmZQgAHAAAAliPp3qTNAwC57R4EAAAAAJaL1pITzwIASsL40N1cvJfjKJwZfLUGKlTz2Va5zm5RFfllZ6pcs+ZfAAAAAAAAAJcFCgAAAAAAjuWwwm5EytPop83kC5GS7Qvl5a2X41dvLESDIm040Z8eAAAAAAAAADAqAwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAwAAAGJ1eQAAAAAAAAAAAAAAAAAAAAAAiBMAAAAAAADLAgUAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAALntHgQAAAAAuZlCAAcAAAC57R4EAAAAAA==";
const SELL_TRADE_B64: &str = "vdt/007mYe7PYUBfoKDfW6KvO76cx7VyTAervQucEpSyKTc2rBTT318CAAAAAAAAHoO/AAAAAAAARAZeZfu2g5tnnXTowAQe8mCx27NNkGSPH2fM5ef2IZUiUodqAAAAAKXmSDEJAAAAd6Nc04blAgClOiU1AgAAAHcLSof15gEArRHmpPwpRKT6glG++BVCbhv7KMa2ZGZ3YHxq2fVmpkZfAAAAAAAAAAYAAAAAAAAAtIVv7qvTSMYMQxACEwAbA/S4w7sl5sD1Oi1NrPVgxb8AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABAAAAHNlbGwAHgAAAAAAAAACAAAAAAAAAIgTAAAAAAAAAwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABfAgAAAAAAAKXmSDEJAAAApTolNQIAAAA=";
const COMPLETE_B64: &str = "X3JhnNQumAgVfWPULgCfjRKK3R2Dglx1t4NZb28o7w2//1tP0kZSRDr84TXbXA0jyAcB2CgSU1LmHI5ekjqDl3WJQ9mIt9If8ZglL/EODgEWy3HpdwBDrNTbGwWid0frBi+cbejg871JmohqAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

fn decode_fixture(b64: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(b64).unwrap()
}

#[test]
fn decodes_a_real_create_v2_event() {
    let data = decode_fixture(CREATE_B64);
    let event = decode_event(&data).expect("should decode as a known event");
    match event {
        PumpEvent::Create(c) => {
            assert_eq!(c.name, "100k");
            assert_eq!(c.symbol, "100k");
            assert_eq!(c.mint, Pubkey::from_str("3xM2iMg4RZBuzdFpvwYab9cUaVpniuHgLUZRg33ipump").unwrap());
            assert_eq!(c.bonding_curve, Pubkey::from_str("113P6U1rwkHf69wyGT4qc9JNJfW1fXrKHYsU2Yy4goF").unwrap());
            assert_eq!(c.creator, Pubkey::from_str("Acp2tKDySe79AsxokjeEy7VdrvWD8bR4V9rLSFkvo6sx").unwrap());
            assert_eq!(c.virtual_token_reserves, 1_073_000_000_000_000);
            assert_eq!(c.virtual_sol_reserves, 30_000_000_000);
            assert_eq!(c.real_token_reserves, 793_100_000_000_000);
            assert_eq!(c.token_total_supply, 1_000_000_000_000_000);
            assert_eq!(c.token_program, Pubkey::from_str("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb").unwrap());
            assert!(!c.is_mayhem_mode);
            assert!(!c.is_cashback_enabled);
        }
        other => panic!("expected Create, got {other:?}"),
    }
}

#[test]
fn decodes_a_real_buy_v2_trade_event_from_the_same_transaction_as_the_create() {
    let data = decode_fixture(BUY_TRADE_B64);
    let event = decode_event(&data).expect("should decode as a known event");
    match event {
        PumpEvent::Trade(t) => {
            assert!(t.is_buy);
            assert_eq!(t.ix_name, "buy");
            assert_eq!(t.mint, Pubkey::from_str("3xM2iMg4RZBuzdFpvwYab9cUaVpniuHgLUZRg33ipump").unwrap());
            assert_eq!(t.sol_amount, 69_135_801);
            assert_eq!(t.token_amount, 2_467_071_716_458);
            assert_eq!(t.virtual_sol_reserves, 30_069_135_801);
            assert_eq!(t.virtual_token_reserves, 1_070_532_928_283_542);
            assert_eq!(t.fee_basis_points, 95);
            assert_eq!(t.fee, 656_791);
            assert_eq!(t.creator_fee_basis_points, 30);
            assert_eq!(t.creator_fee, 207_408);
            assert!(!t.mayhem_mode);
        }
        other => panic!("expected Trade, got {other:?}"),
    }
}

#[test]
fn decodes_a_real_sell_v2_trade_event() {
    let data = decode_fixture(SELL_TRADE_B64);
    let event = decode_event(&data).expect("should decode as a known event");
    match event {
        PumpEvent::Trade(t) => {
            assert!(!t.is_buy);
            assert_eq!(t.ix_name, "sell");
            assert_eq!(t.mint, Pubkey::from_str("ExXQP6ZatSTMXpP7jaWcN76k8V9vwzFUD5rPNfWGpump").unwrap());
            assert_eq!(t.sol_amount, 607);
            assert_eq!(t.token_amount, 12_550_942);
            assert_eq!(t.virtual_sol_reserves, 39_481_566_885);
            assert_eq!(t.virtual_token_reserves, 815_317_187_863_415);
            assert_eq!(t.fee, 6);
        }
        other => panic!("expected Trade, got {other:?}"),
    }
}

#[test]
fn decodes_a_real_complete_event() {
    let data = decode_fixture(COMPLETE_B64);
    let event = decode_event(&data).expect("should decode as a known event");
    match event {
        PumpEvent::Complete(c) => {
            assert_eq!(c.user, Pubkey::from_str("2StTWuacuBE1rfWVY9G3iXyb3WRcmf8wKbnPBBRe6iDq").unwrap());
            assert_eq!(c.mint, Pubkey::from_str("4yGJ5ynrB4QX7AiRps1DTAzdXwEf15NvrRF5gouEpump").unwrap());
            assert_eq!(c.bonding_curve, Pubkey::from_str("HG5pAcVqdrixHnTVa2fRLSgGvezeM79twufgZABqPjT2").unwrap());
            assert_eq!(c.timestamp, 1_787_337_289);
            assert_eq!(c.quote_mint, NATIVE_SOL_QUOTE_SENTINEL);
        }
        other => panic!("expected Complete, got {other:?}"),
    }
}

#[test]
fn rejects_data_shorter_than_a_discriminator() {
    assert_eq!(decode_event(&[1, 2, 3]), None);
}

#[test]
fn ignores_an_unrecognized_discriminator() {
    let mut data = decode_fixture(CREATE_B64);
    data[0] ^= 0xFF;
    assert_eq!(decode_event(&data), None);
}
