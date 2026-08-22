//! WebSocket connection to Solana's `logsSubscribe` RPC method, with
//! automatic reconnect-with-backoff. Delivers parsed `RawLogEvent`s (see
//! `logs.rs`) over a bounded `mpsc` channel; reconnects (re-subscribing)
//! on any connection error, rejected subscribe, stalled connection, or
//! clean server-side close rather than terminating, since a live trading
//! pipeline must keep running through transient network blips — only
//! stops once the receiving end is dropped.
//!
//! Verified against the real public endpoint
//! (`wss://api.mainnet-beta.solana.com`), not just written to match the
//! documented protocol: connects, subscribes, and receives real live
//! `logsNotification` messages that this module's own `extract_events`
//! successfully parses.
//!
//! An independent review of the first version of this module (single
//! connection tested live, but not stress-tested against failure modes)
//! found real gaps this version closes:
//! - No read timeout meant a connection that looked open but had gone
//!   silent — a rejected/rate-limited subscribe with no error, or a
//!   half-dead TCP connection with no RST — would leave `connect_and_stream`
//!   awaiting forever with no reconnect and nothing logged. Every read
//!   (including the subscribe confirmation itself) is now wrapped in
//!   [`READ_TIMEOUT`].
//! - The subscribe response's own `error` field was never checked — a
//!   rejected subscribe looked identical to a successful one as far as
//!   this module was concerned. Now checked explicitly before the read
//!   loop starts.
//! - `mpsc::unbounded_channel` let a slow consumer grow memory without
//!   bound. Now a bounded channel with [`EVENT_CHANNEL_CAPACITY`],
//!   dropping (and logging) the newest event rather than blocking the
//!   socket read when full — the listener must keep draining the socket,
//!   not stall behind a slow consumer.
//! - Backoff reset unconditionally on every clean close, including a
//!   close caused by exactly the condition backoff exists to protect
//!   against (e.g. a provider that rate-limits by immediately closing the
//!   connection after every subscribe) — which would keep the loop
//!   retrying at ~1s forever instead of ever escalating. Backoff now only
//!   resets after a connection stays usable for
//!   [`MIN_CONNECTION_DURATION_FOR_BACKOFF_RESET`].

use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::logs::{extract_events, RawLogEvent};

#[derive(Debug, Clone)]
pub struct ListenerConfig {
    pub ws_url: String,
    pub program_id: String,
    pub commitment: String,
}

const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const MIN_CONNECTION_DURATION_FOR_BACKOFF_RESET: Duration = Duration::from_secs(30);
const READ_TIMEOUT: Duration = Duration::from_secs(60);
/// How many parsed events can queue up if the consumer falls behind.
pub const EVENT_CHANNEL_CAPACITY: usize = 1024;

#[derive(Debug)]
pub enum ListenerError {
    WebSocket(Box<tokio_tungstenite::tungstenite::Error>),
    /// No message (including the subscribe confirmation) arrived within
    /// `READ_TIMEOUT` — the connection is treated as dead even though the
    /// TCP socket might still look open.
    Timeout,
    /// The connection closed before the subscribe was ever confirmed.
    StreamEndedBeforeSubscribeConfirmed,
    /// The RPC node's subscribe response itself carried a JSON-RPC
    /// `error` field (e.g. rejected, rate-limited).
    SubscribeRejected(String),
}

impl std::fmt::Display for ListenerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ListenerError::WebSocket(e) => write!(f, "websocket error: {e}"),
            ListenerError::Timeout => write!(f, "no message received within {READ_TIMEOUT:?}"),
            ListenerError::StreamEndedBeforeSubscribeConfirmed => {
                write!(f, "connection closed before the subscribe was confirmed")
            }
            ListenerError::SubscribeRejected(msg) => write!(f, "subscribe request rejected: {msg}"),
        }
    }
}

impl std::error::Error for ListenerError {}

impl From<tokio_tungstenite::tungstenite::Error> for ListenerError {
    fn from(e: tokio_tungstenite::tungstenite::Error) -> Self {
        ListenerError::WebSocket(Box::new(e))
    }
}

enum ConnectionOutcome {
    /// The connection ended (server close, or our own read timeout) —
    /// `run` should back off and reconnect.
    Closed,
    /// The consumer dropped its receiver — `run` should stop for good,
    /// not treat this as a failure to retry.
    ConsumerGone,
}

/// Runs the reconnect loop until `tx`'s receiver is dropped. Never panics
/// on a network error — logs it to stderr and retries with capped
/// exponential backoff instead, since one bad reconnect must not take
/// down whatever else is running in the same process.
pub async fn run(config: ListenerConfig, tx: mpsc::Sender<RawLogEvent>) {
    let mut backoff = INITIAL_BACKOFF;
    while !tx.is_closed() {
        let connected_at = Instant::now();
        match connect_and_stream(&config, &tx).await {
            Ok(ConnectionOutcome::ConsumerGone) => return,
            Ok(ConnectionOutcome::Closed) => {
                eprintln!("live listener: connection closed, reconnecting in {backoff:?}");
            }
            Err(e) => {
                eprintln!("live listener: connection error ({e}), retrying in {backoff:?}");
            }
        }
        if connected_at.elapsed() >= MIN_CONNECTION_DURATION_FOR_BACKOFF_RESET {
            backoff = INITIAL_BACKOFF;
        }
        if tx.is_closed() {
            return;
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

async fn connect_and_stream(config: &ListenerConfig, tx: &mpsc::Sender<RawLogEvent>) -> Result<ConnectionOutcome, ListenerError> {
    let (mut ws, _) = tokio_tungstenite::connect_async(&config.ws_url).await?;

    let subscribe = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "logsSubscribe",
        "params": [{"mentions": [config.program_id]}, {"commitment": config.commitment}],
    });
    ws.send(Message::Text(subscribe.to_string())).await?;

    let confirmation = tokio::time::timeout(READ_TIMEOUT, ws.next())
        .await
        .map_err(|_| ListenerError::Timeout)?
        .ok_or(ListenerError::StreamEndedBeforeSubscribeConfirmed)??;
    check_subscribe_confirmation(&confirmation)?;

    loop {
        let next = tokio::time::timeout(READ_TIMEOUT, ws.next()).await.map_err(|_| ListenerError::Timeout)?;
        let Some(msg) = next else {
            return Ok(ConnectionOutcome::Closed);
        };
        let msg = msg?;
        let Message::Text(text) = msg else {
            continue;
        };
        let Some(extracted) = extract_events(&text) else {
            continue;
        };
        if extracted.skipped_malformed > 0 {
            eprintln!("live listener: {} Program data line(s) failed to base64-decode, dropped", extracted.skipped_malformed);
        }
        for event in extracted.events {
            match tx.try_send(event) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(dropped)) => {
                    eprintln!("live listener: event channel full (capacity {EVENT_CHANNEL_CAPACITY}), dropped event for signature {}", dropped.signature);
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    return Ok(ConnectionOutcome::ConsumerGone);
                }
            }
        }
    }
}

/// The first message after subscribing must be the JSON-RPC response to
/// the `logsSubscribe` call itself (`{"result": <subscription id>, ...}`)
/// — never a `logsNotification` (no subscription id exists yet for the
/// server to notify against). Checked for an `error` field explicitly:
/// a rejected/rate-limited subscribe otherwise looks identical to a
/// successful one from this module's point of view.
pub fn check_subscribe_confirmation(msg: &Message) -> Result<(), ListenerError> {
    let Message::Text(text) = msg else {
        return Err(ListenerError::SubscribeRejected("non-text response to subscribe".to_string()));
    };
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| ListenerError::SubscribeRejected(format!("unparseable subscribe response: {e}")))?;
    if let Some(error) = value.get("error") {
        return Err(ListenerError::SubscribeRejected(error.to_string()));
    }
    Ok(())
}
