//! Parsing for `accountSubscribe`'s WebSocket JSON-RPC traffic. Verified
//! against real messages from `wss://api.mainnet-beta.solana.com`, not just
//! the documented shape: a real `accountNotification` for a live PumpSwap
//! pool account (see `tests/account_notification_test.rs`), and real
//! `accountSubscribe`/`accountUnsubscribe` confirmation responses.
//!
//! `accountSubscribe` differs from `logsSubscribe` (`logs.rs`) in two ways
//! that shape this module:
//! - Its `value.data` is a 2-element `[base64_string, "base64"]` tuple, not
//!   a plain string — `logsSubscribe`'s `logs` field has no such wrapper.
//! - A live watcher opens many subscriptions on one connection (one per
//!   watched account) and can add/remove them at runtime, so — unlike
//!   `logsSubscribe`'s single one-shot subscribe at connection start —
//!   this module has to correlate multiple in-flight `id`-tagged
//!   subscribe/unsubscribe requests against their responses, which arrive
//!   interleaved with `accountNotification` messages on the same socket.

use base64::Engine;
use solana_pubkey::Pubkey;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountUpdate {
    pub pubkey: Pubkey,
    pub slot: u64,
    pub lamports: u64,
    pub data: Vec<u8>,
    pub owner: Pubkey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedMessage {
    /// A live account update. Carries the server's numeric subscription
    /// id, not the account's pubkey — the notification itself never
    /// repeats which account it's for, so the caller must keep its own
    /// subscription-id -> pubkey table (built from `SubscribeOk`) to know.
    Notification { subscription_id: u64, slot: u64, lamports: u64, data: Vec<u8>, owner: Pubkey },
    /// Response to an `accountSubscribe` call: `id` is the request id we
    /// sent, `subscription_id` is what the server will tag notifications
    /// with from now on.
    SubscribeOk { id: u64, subscription_id: u64 },
    /// Response to an `accountUnsubscribe` call.
    UnsubscribeOk { id: u64 },
    /// A JSON-RPC `error` response to either of the above. `id` is absent
    /// only if the server's error was malformed enough to omit it too.
    Error { id: Option<u64>, message: String },
}

/// Parses one WebSocket text message. Returns `None` for anything that
/// isn't a recognized `accountSubscribe`-protocol message (malformed JSON,
/// an unrelated method, a response shape this module doesn't expect).
pub fn parse_message(raw: &str) -> Option<ParsedMessage> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;

    if let Some(method) = value.get("method").and_then(|m| m.as_str()) {
        return if method == "accountNotification" { parse_notification(&value) } else { None };
    }

    let id = value.get("id").and_then(|v| v.as_u64());

    if let Some(error) = value.get("error") {
        return Some(ParsedMessage::Error { id, message: error.to_string() });
    }

    let id = id?;
    match value.get("result")? {
        serde_json::Value::Bool(_) => Some(ParsedMessage::UnsubscribeOk { id }),
        serde_json::Value::Number(n) => Some(ParsedMessage::SubscribeOk { id, subscription_id: n.as_u64()? }),
        _ => None,
    }
}

fn parse_notification(value: &serde_json::Value) -> Option<ParsedMessage> {
    let params = value.get("params")?;
    let subscription_id = params.get("subscription")?.as_u64()?;
    let result = params.get("result")?;
    let slot = result.get("context")?.get("slot")?.as_u64()?;
    let v = result.get("value")?;
    let lamports = v.get("lamports")?.as_u64()?;
    let owner = Pubkey::from_str(v.get("owner")?.as_str()?).ok()?;
    let data_b64 = v.get("data")?.get(0)?.as_str()?;
    let data = base64::engine::general_purpose::STANDARD.decode(data_b64).ok()?;
    Some(ParsedMessage::Notification { subscription_id, slot, lamports, data, owner })
}
