//! One-shot HTTPS `getAccountInfo`, filling a gap `accountSubscribe`
//! (`account_watcher.rs`) cannot: verified live against
//! `wss://api.mainnet-beta.solana.com` that the WS pubsub endpoint only
//! accepts subscribe/unsubscribe-family methods — a bare `getAccountInfo`
//! sent over that same socket comes back
//! `{"error":{"code":-32601,"message":"Method not found"}}` — and that
//! `accountSubscribe` delivers no initial snapshot, only notifications on
//! *subsequent* changes (confirmed live: subscribing to a just-created
//! mint and waiting produced zero notifications). A freshly created
//! Token-2022 mint whose last write is its own creation transaction can
//! then go the entire process lifetime without a single
//! `accountNotification`, so `pipeline.rs` fetches its current state once,
//! over the ordinary HTTPS JSON-RPC endpoint, right when it first sees the
//! mint's `TokenCreated` candidate.
//!
//! Split the same way `logs.rs`/`account_notification.rs` are split from
//! their networking counterparts: [`parse_get_account_info_response`] is
//! pure and tested against real captured `getAccountInfo` responses
//! (a real Pyth SOL/USD account with data, a real `value: null` for an
//! account that doesn't exist, and a real RPC error for a malformed
//! pubkey), and [`fetch_account`] is the thin `reqwest` wrapper around it.

use base64::Engine;
use solana_pubkey::Pubkey;
use std::str::FromStr;

use crate::account_notification::AccountUpdate;

#[derive(Debug)]
pub enum FetchError {
    Http(reqwest::Error),
    /// The RPC node returned a JSON-RPC `error` object — e.g. a malformed
    /// pubkey (`-32602 Invalid param`, verified live).
    Rpc(String),
    /// `result.value` was `null` — the account doesn't exist at the
    /// requested commitment level (verified live against a random pubkey
    /// with no on-chain account).
    AccountNotFound,
    /// The response didn't match the shape `getAccountInfo` with
    /// `encoding: "base64"` is documented (and verified live) to return.
    Malformed,
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Http(e) => write!(f, "http error: {e}"),
            FetchError::Rpc(message) => write!(f, "rpc error: {message}"),
            FetchError::AccountNotFound => write!(f, "account not found"),
            FetchError::Malformed => write!(f, "malformed getAccountInfo response"),
        }
    }
}

impl std::error::Error for FetchError {}

impl From<reqwest::Error> for FetchError {
    fn from(e: reqwest::Error) -> Self {
        FetchError::Http(e)
    }
}

/// [`fetch_account`], retried a few times when the account isn't found yet.
///
/// Verified live (`bin/pipeline.rs`): fetching a mint immediately after
/// seeing its `TokenCreated` candidate over `logsSubscribe` can genuinely
/// come back `AccountNotFound` even though the account demonstrably exists
/// (later `Trade`/`Graduation` log events for that same mint arrived
/// within the same run) — `https://api.mainnet-beta.solana.com` load-
/// balances across multiple backend nodes, and the one handling this
/// particular HTTP request can lag slightly behind whichever one served
/// the WebSocket log stream. Observed live in an 18-mint sample: 1 genuine
/// `AccountNotFound` race, cleared by the very next attempt after a short
/// delay. Any other error (a malformed response, a real RPC error, a
/// network failure) is returned immediately without retrying — those
/// aren't the replica-lag case this loop targets, and retrying them
/// blindly would just mask a real problem.
pub async fn fetch_account_with_retry(
    client: &reqwest::Client,
    http_url: &str,
    pubkey: &Pubkey,
    commitment: &str,
    max_attempts: u32,
    retry_delay: std::time::Duration,
) -> Result<AccountUpdate, FetchError> {
    let mut attempt = 1;
    loop {
        match fetch_account(client, http_url, pubkey, commitment).await {
            Ok(update) => return Ok(update),
            Err(FetchError::AccountNotFound) if attempt < max_attempts => {
                attempt += 1;
                tokio::time::sleep(retry_delay).await;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Fetches one account's current state via a single `getAccountInfo` call
/// against `http_url` (the plain HTTPS JSON-RPC endpoint — NOT the `wss://`
/// pubsub URL `account_watcher`/`listener` use, which does not implement
/// this method).
pub async fn fetch_account(client: &reqwest::Client, http_url: &str, pubkey: &Pubkey, commitment: &str) -> Result<AccountUpdate, FetchError> {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getAccountInfo",
        "params": [pubkey.to_string(), {"encoding": "base64", "commitment": commitment}],
    });
    let response = client.post(http_url).json(&request).send().await?;
    let text = response.text().await?;
    parse_get_account_info_response(&text, pubkey)
}

fn parse_get_account_info_response(raw: &str, pubkey: &Pubkey) -> Result<AccountUpdate, FetchError> {
    let response: serde_json::Value = serde_json::from_str(raw).map_err(|_| FetchError::Malformed)?;

    if let Some(error) = response.get("error") {
        return Err(FetchError::Rpc(error.to_string()));
    }
    let result = response.get("result").ok_or(FetchError::Malformed)?;
    let slot = result.get("context").and_then(|c| c.get("slot")).and_then(|s| s.as_u64()).ok_or(FetchError::Malformed)?;
    let value = result.get("value").ok_or(FetchError::Malformed)?;
    if value.is_null() {
        return Err(FetchError::AccountNotFound);
    }
    let lamports = value.get("lamports").and_then(|l| l.as_u64()).ok_or(FetchError::Malformed)?;
    let owner = value
        .get("owner")
        .and_then(|o| o.as_str())
        .and_then(|s| Pubkey::from_str(s).ok())
        .ok_or(FetchError::Malformed)?;
    let data_b64 = value.get("data").and_then(|d| d.get(0)).and_then(|d| d.as_str()).ok_or(FetchError::Malformed)?;
    let data = base64::engine::general_purpose::STANDARD.decode(data_b64).map_err(|_| FetchError::Malformed)?;

    Ok(AccountUpdate { pubkey: *pubkey, slot, lamports, data, owner })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real `getAccountInfo` response for the Pyth SOL/USD sponsored
    /// price-update account (`crates/ingest/src/price_feed.rs`), captured
    /// live via curl against `https://api.mainnet-beta.solana.com`.
    const REAL_ACCOUNT_WITH_DATA: &str = r#"{"jsonrpc":"2.0","result":{"context":{"apiVersion":"4.2.0","slot":440897479},"value":{"data":["IvEjY51+9M1gMUcENA3t3zcf1CRyFI8kjp0abRpesqw6zYt/1dayQwHvDYtv2izrpB2hXUCV0do5Kg0vjtDGx7wPTPrIwoC1bcgici8CAAAAU0pHAAAAAAD4////wnKJagAAAADCcolqAAAAAPxy1jACAAAAGalCAAAAAACEj0caAAAAAAA=","base64"],"executable":false,"lamports":1825031,"owner":"rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ","rentEpoch":18446744073709551615,"space":134}},"id":1}"#;

    /// Real `getAccountInfo` response for a random pubkey with no on-chain
    /// account (a freshly generated, never-funded keypair), captured live.
    const REAL_ACCOUNT_NOT_FOUND: &str =
        r#"{"jsonrpc":"2.0","result":{"context":{"apiVersion":"4.2.0","slot":440897525},"value":null},"id":1}"#;

    /// Real `getAccountInfo` error response for a malformed pubkey string,
    /// captured live.
    const REAL_RPC_ERROR: &str = r#"{"jsonrpc":"2.0","error":{"code":-32602,"message":"Invalid param: Invalid"},"id":1}"#;

    #[test]
    fn parses_a_real_account_with_data() {
        let pubkey = Pubkey::from_str("7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE").unwrap();
        let update = parse_get_account_info_response(REAL_ACCOUNT_WITH_DATA, &pubkey).unwrap();
        assert_eq!(update.pubkey, pubkey);
        assert_eq!(update.slot, 440_897_479);
        assert_eq!(update.lamports, 1_825_031);
        assert_eq!(update.owner, Pubkey::from_str("rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ").unwrap());
        assert_eq!(update.data.len(), 134);
        // Anchor account discriminator for PriceUpdateV2 (verified in
        // crates/ingest/src/price_feed.rs against this exact account).
        assert_eq!(&update.data[..8], &[34, 241, 35, 99, 157, 126, 244, 205]);
    }

    #[test]
    fn a_nonexistent_account_is_a_distinct_error_from_a_malformed_response() {
        let pubkey = Pubkey::from_str("6F8dWS3nXx4xFH3QdCprFC7Q5pVQ8EHTWC7HQVkBXoNR").unwrap();
        let err = parse_get_account_info_response(REAL_ACCOUNT_NOT_FOUND, &pubkey).unwrap_err();
        assert!(matches!(err, FetchError::AccountNotFound));
    }

    #[test]
    fn an_rpc_error_response_is_surfaced_not_treated_as_not_found() {
        let pubkey = Pubkey::from_str("6F8dWS3nXx4xFH3QdCprFC7Q5pVQ8EHTWC7HQVkBXoNR").unwrap();
        let err = parse_get_account_info_response(REAL_RPC_ERROR, &pubkey).unwrap_err();
        match err {
            FetchError::Rpc(message) => assert!(message.contains("Invalid param")),
            other => panic!("expected FetchError::Rpc, got {other:?}"),
        }
    }

    #[test]
    fn garbage_input_is_malformed_not_a_panic() {
        let pubkey = Pubkey::from_str("6F8dWS3nXx4xFH3QdCprFC7Q5pVQ8EHTWC7HQVkBXoNR").unwrap();
        let err = parse_get_account_info_response("not json", &pubkey).unwrap_err();
        assert!(matches!(err, FetchError::Malformed));
    }
}

/// Exercises `fetch_account_with_retry`'s retry counting against a tiny
/// hand-rolled local HTTP server (no mocking crate — this workspace has
/// none, and the whole response is 2-3 fixed lines), not just its parsing.
/// The retry loop itself was added in direct response to a real race
/// condition observed live (see its doc comment) — this locks in that its
/// attempt-counting logic is exactly right, independent of whether the
/// live network happens to reproduce that race on any given test run.
#[cfg(test)]
mod retry_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::time::Duration;

    /// Serves one canned JSON body per accepted connection, in order, then
    /// stops. Doesn't parse the incoming request at all — every real
    /// caller here only ever sends one kind of request (getAccountInfo).
    async fn serve_responses(listener: TcpListener, bodies: Vec<&'static str>, request_count: Arc<AtomicUsize>) {
        for body in bodies {
            let (mut stream, _) = listener.accept().await.expect("test server accept failed");
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf).await;
            request_count.fetch_add(1, Ordering::SeqCst);
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
        }
    }

    const NOT_FOUND_BODY: &str = r#"{"jsonrpc":"2.0","result":{"context":{"slot":1},"value":null},"id":1}"#;
    const FOUND_BODY: &str = r#"{"jsonrpc":"2.0","result":{"context":{"slot":2},"value":{"data":["","base64"],"lamports":1,"owner":"11111111111111111111111111111111","executable":false,"rentEpoch":0,"space":0}},"id":1}"#;

    #[tokio::test]
    async fn retries_once_after_a_not_found_response_then_succeeds() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind failed");
        let addr = listener.local_addr().expect("local_addr failed");
        let request_count = Arc::new(AtomicUsize::new(0));
        tokio::spawn(serve_responses(listener, vec![NOT_FOUND_BODY, FOUND_BODY], request_count.clone()));

        let client = reqwest::Client::new();
        let url = format!("http://{addr}");
        let pubkey = Pubkey::from_str("11111111111111111111111111111111").unwrap();
        let result = fetch_account_with_retry(&client, &url, &pubkey, "confirmed", 3, Duration::from_millis(10)).await;

        assert!(result.is_ok(), "expected the retry to succeed on the second attempt, got {result:?}");
        assert_eq!(request_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn stops_retrying_once_max_attempts_is_reached() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind failed");
        let addr = listener.local_addr().expect("local_addr failed");
        let request_count = Arc::new(AtomicUsize::new(0));
        tokio::spawn(serve_responses(listener, vec![NOT_FOUND_BODY, NOT_FOUND_BODY], request_count.clone()));

        let client = reqwest::Client::new();
        let url = format!("http://{addr}");
        let pubkey = Pubkey::from_str("11111111111111111111111111111111").unwrap();
        let result = fetch_account_with_retry(&client, &url, &pubkey, "confirmed", 2, Duration::from_millis(10)).await;

        assert!(matches!(result, Err(FetchError::AccountNotFound)));
        assert_eq!(request_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_single_attempt_budget_never_retries() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind failed");
        let addr = listener.local_addr().expect("local_addr failed");
        let request_count = Arc::new(AtomicUsize::new(0));
        tokio::spawn(serve_responses(listener, vec![NOT_FOUND_BODY], request_count.clone()));

        let client = reqwest::Client::new();
        let url = format!("http://{addr}");
        let pubkey = Pubkey::from_str("11111111111111111111111111111111").unwrap();
        let result = fetch_account_with_retry(&client, &url, &pubkey, "confirmed", 1, Duration::from_millis(10)).await;

        assert!(matches!(result, Err(FetchError::AccountNotFound)));
        assert_eq!(request_count.load(Ordering::SeqCst), 1);
    }
}
