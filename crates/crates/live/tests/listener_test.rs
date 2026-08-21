//! `check_subscribe_confirmation` is the fix for a gap an independent
//! review caught: the first version of `listener.rs` never looked at the
//! subscribe response's own `error` field, so a rejected or rate-limited
//! `logsSubscribe` call looked identical to a successful one — the
//! connection would sit open, deliver nothing, and never trigger a
//! reconnect.

use momentum_live::listener::check_subscribe_confirmation;
use tokio_tungstenite::tungstenite::Message;

#[test]
fn accepts_a_real_successful_subscribe_confirmation() {
    let msg = Message::Text(r#"{"jsonrpc":"2.0","result":10397413,"id":1}"#.to_string());
    assert!(check_subscribe_confirmation(&msg).is_ok());
}

#[test]
fn rejects_a_subscribe_response_carrying_a_json_rpc_error() {
    let msg = Message::Text(
        r#"{"jsonrpc":"2.0","error":{"code":-32600,"message":"too many subscriptions"},"id":1}"#.to_string(),
    );
    let err = check_subscribe_confirmation(&msg).expect_err("should reject an error response");
    assert!(err.to_string().contains("too many subscriptions"));
}

#[test]
fn rejects_an_unparseable_response() {
    let msg = Message::Text("not json".to_string());
    assert!(check_subscribe_confirmation(&msg).is_err());
}

#[test]
fn rejects_a_non_text_response() {
    let msg = Message::Binary(vec![1, 2, 3]);
    assert!(check_subscribe_confirmation(&msg).is_err());
}
