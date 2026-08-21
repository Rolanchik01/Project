//! Real `accountSubscribe`-protocol messages captured live from
//! `wss://api.mainnet-beta.solana.com` during Stage 1 research — not
//! hand-constructed. `REAL_NOTIFICATION` is a real `accountNotification`
//! for an active PumpSwap pool account (owner is the PumpSwap program
//! itself); `REAL_SUBSCRIBE_RESPONSE`/`REAL_UNSUBSCRIBE_RESPONSE` are real
//! confirmations for a throwaway subscribe/unsubscribe round trip against
//! the wrapped-SOL mint account, captured to confirm the response shape
//! (`{"result": <number>, ...}` vs `{"result": true, ...}`) rather than
//! assuming it from documentation.

use momentum_live::account_notification::{parse_message, ParsedMessage};
use momentum_pumpswap::Pool;
use solana_pubkey::Pubkey;
use std::str::FromStr;

const REAL_NOTIFICATION: &str = r#"{"jsonrpc":"2.0","method":"accountNotification","params":{"result":{"context":{"slot":440746106},"value":{"lamports":2985840,"data":["8ZptBBGxbbz/AAB1FSRea9XBLRVZ0rJ5Sga9HLyFV2YjuInYVXplkZRilArfLRy2lEfDVnoyropYKB1dfBl+k6/0RsVgAU+IEku/BpuIV/6rgYT7aH9jRhjANdrEOdwa6ztVmKDwAAAAAAHdTqZc8WwTwFST10k38n8tcbX+lr3tGjwhCuekXQlhz6+YPZtPPrSBHRT6EGTfU5kgA2833uRUBmF1oNdv+oZ9eiiuuweCbnfjuK3HYEgmToPsT5WYIAI0om/OSmx/0yloQ2tZ0AMAANyBSJY0EGJdG/AFLipFi8PAJgR/v6fFm4himrD/aHl+AADJQR4YBAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==","base64"],"owner":"pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA","executable":false,"rentEpoch":18446744073709551615,"space":301}},"subscription":11434397}}"#;
const REAL_SUBSCRIBE_RESPONSE: &str = r#"{"jsonrpc":"2.0","result":8189959,"id":7}"#;
const REAL_UNSUBSCRIBE_RESPONSE: &str = r#"{"jsonrpc":"2.0","result":true,"id":8}"#;

#[test]
fn parses_a_real_account_notification_for_a_live_pumpswap_pool() {
    let parsed = parse_message(REAL_NOTIFICATION).expect("should parse as a known message");
    match parsed {
        ParsedMessage::Notification { subscription_id, slot, lamports, data, owner } => {
            assert_eq!(subscription_id, 11434397);
            assert_eq!(slot, 440746106);
            assert_eq!(lamports, 2985840);
            assert_eq!(data.len(), 301);
            assert_eq!(owner, Pubkey::from_str("pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA").unwrap());

            // The owner is PumpSwap's own program, and the account really
            // is a pool: decoding it end to end with the already-verified
            // `Pool::decode` succeeds, proving this isn't just a
            // same-length coincidence.
            let pool = Pool::decode(&data).expect("a live PumpSwap pool account should decode as a Pool");
            assert_eq!(pool.base_mint, Pubkey::from_str("jSTQ9frNjfqiXf3cnvStPbvAuX8MRMsmwaRVkiApump").unwrap());
            assert_eq!(pool.quote_mint, Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap());
            assert_eq!(pool.lp_supply, 4_193_388_282_728);
            // This particular live pool happens to be boosted
            // (virtual_quote_reserves != 0) — a second real confirmation,
            // independent of the one `lib.rs`'s module docs already cite,
            // that `is_standard()` correctly refuses it.
            assert_eq!(pool.virtual_quote_reserves, 17_584_505_289);
            assert!(!pool.is_standard());
        }
        other => panic!("expected Notification, got {other:?}"),
    }
}

#[test]
fn parses_a_real_subscribe_confirmation() {
    let parsed = parse_message(REAL_SUBSCRIBE_RESPONSE).expect("should parse as a known message");
    assert_eq!(parsed, ParsedMessage::SubscribeOk { id: 7, subscription_id: 8189959 });
}

#[test]
fn parses_a_real_unsubscribe_confirmation() {
    let parsed = parse_message(REAL_UNSUBSCRIBE_RESPONSE).expect("should parse as a known message");
    assert_eq!(parsed, ParsedMessage::UnsubscribeOk { id: 8 });
}

#[test]
fn rejects_a_response_carrying_a_json_rpc_error() {
    let msg = r#"{"jsonrpc":"2.0","error":{"code":-32602,"message":"Invalid param: WrongSize"},"id":9}"#;
    let parsed = parse_message(msg).expect("should parse as a known message");
    match parsed {
        ParsedMessage::Error { id, message } => {
            assert_eq!(id, Some(9));
            assert!(message.contains("WrongSize"));
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[test]
fn ignores_an_unrelated_notification_method() {
    let msg = r#"{"jsonrpc":"2.0","method":"logsNotification","params":{"result":{"context":{"slot":1},"value":{"signature":"x","err":null,"logs":[]}},"subscription":1}}"#;
    assert_eq!(parse_message(msg), None);
}

#[test]
fn rejects_unparseable_json() {
    assert_eq!(parse_message("not json"), None);
}
