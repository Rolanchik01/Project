//! `accountSubscribe` connection management: watches a *dynamic* set of
//! accounts on one WebSocket connection, re-subscribing to the current set
//! on every reconnect. This is the piece `crates/live`'s crate doc flagged
//! as "a genuinely separate, stateful subscription-management problem" —
//! deciding which accounts to watch (bonding curves, pools, mints
//! discovered from live `TokenCreated`/`Trade` events) is still the
//! caller's job; this module only tracks whatever set it's told to watch.
//!
//! Shares `listener.rs`'s reconnect-with-backoff shape (same constants,
//! same bounded-channel-with-drop-newest delivery, same independent-review
//! history behind those choices — see `listener.rs`'s doc comment) but
//! can't reuse its code directly: `logsSubscribe` has exactly one
//! subscription per connection, confirmed once before the read loop
//! starts, while `accountSubscribe` here can have any number, opened and
//! closed at any time while the read loop is already running.
//!
//! Known simplification: unwatching an account whose `accountSubscribe`
//! confirmation hasn't arrived yet is handled (the pending subscription is
//! cancelled the moment it confirms — see `pending_cancel` below), but
//! watching the *same* account twice in a row before its first
//! subscription confirms just no-ops the second call rather than queuing
//! it — there is exactly one subscription per pubkey, not one per `Watch`
//! call.
//!
//! Verified live against mainnet (`bin/account_watcher_demo.rs`: real
//! multi-account initial subscribe, plus a dynamic unwatch-one/watch-a-
//! different-one swap mid-run), and `account_notification.rs`'s message
//! parsing has real-fixture tests, but the bookkeeping in
//! `connect_and_stream` below (the `pending_subscribe`/`pending_cancel`/
//! `subscription_of`/`pubkey_of` state machine itself, and the reconnect
//! path) has no automated test driving it against a scripted message/
//! command sequence — an independent review reasoned through the race
//! conditions by inspection rather than a mock-WebSocket test harness,
//! which does not yet exist for this module.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use solana_pubkey::Pubkey;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::account_notification::{parse_message, AccountUpdate, ParsedMessage};

#[derive(Debug, Clone)]
pub struct WatcherConfig {
    pub ws_url: String,
    pub commitment: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchCommand {
    Watch(Pubkey),
    Unwatch(Pubkey),
}

const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const MIN_CONNECTION_DURATION_FOR_BACKOFF_RESET: Duration = Duration::from_secs(30);
const READ_TIMEOUT: Duration = Duration::from_secs(60);
/// How many parsed updates can queue up if the consumer falls behind.
pub const UPDATE_CHANNEL_CAPACITY: usize = 1024;

#[derive(Debug)]
pub enum WatcherError {
    WebSocket(Box<tokio_tungstenite::tungstenite::Error>),
    /// No message arrived for `READ_TIMEOUT`, independent of how much
    /// command traffic (`Watch`/`Unwatch`) was flowing in the meantime —
    /// the deadline only resets when the socket itself yields a message.
    Timeout,
}

impl std::fmt::Display for WatcherError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WatcherError::WebSocket(e) => write!(f, "websocket error: {e}"),
            WatcherError::Timeout => write!(f, "no message received within {READ_TIMEOUT:?}"),
        }
    }
}

impl std::error::Error for WatcherError {}

impl From<tokio_tungstenite::tungstenite::Error> for WatcherError {
    fn from(e: tokio_tungstenite::tungstenite::Error) -> Self {
        WatcherError::WebSocket(Box::new(e))
    }
}

enum Outcome {
    /// The connection ended — `run` should back off and reconnect.
    Closed,
    /// The consumer dropped its receiver — `run` should stop for good.
    ConsumerGone,
}

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Runs the reconnect loop until `tx`'s receiver is dropped, watching
/// `initial_accounts` plus whatever `Watch`/`Unwatch` commands arrive over
/// `commands`. Never panics on a network error — logs it to stderr and
/// retries with capped exponential backoff, same policy as `listener::run`.
pub async fn run(config: WatcherConfig, initial_accounts: Vec<Pubkey>, mut commands: mpsc::Receiver<WatchCommand>, tx: mpsc::Sender<AccountUpdate>) {
    let mut desired: HashSet<Pubkey> = initial_accounts.into_iter().collect();
    let mut backoff = INITIAL_BACKOFF;
    loop {
        if tx.is_closed() {
            return;
        }
        while let Ok(cmd) = commands.try_recv() {
            apply_command(&mut desired, cmd);
        }
        if desired.is_empty() {
            // Nothing to watch: opening a connection with zero
            // subscriptions would just sit idle until READ_TIMEOUT and
            // reconnect in a tight loop for no reason. Block for the next
            // command instead of connecting to anything.
            match commands.recv().await {
                Some(cmd) => {
                    apply_command(&mut desired, cmd);
                    continue;
                }
                None => return,
            }
        }
        let connected_at = Instant::now();
        match connect_and_stream(&config, &mut desired, &mut commands, &tx).await {
            Ok(Outcome::ConsumerGone) => return,
            Ok(Outcome::Closed) => {
                eprintln!("account watcher: connection closed, reconnecting in {backoff:?}");
            }
            Err(e) => {
                eprintln!("account watcher: connection error ({e}), retrying in {backoff:?}");
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

fn apply_command(desired: &mut HashSet<Pubkey>, cmd: WatchCommand) {
    match cmd {
        WatchCommand::Watch(pk) => {
            desired.insert(pk);
        }
        WatchCommand::Unwatch(pk) => {
            desired.remove(&pk);
        }
    }
}

async fn connect_and_stream(
    config: &WatcherConfig,
    desired: &mut HashSet<Pubkey>,
    commands: &mut mpsc::Receiver<WatchCommand>,
    tx: &mpsc::Sender<AccountUpdate>,
) -> Result<Outcome, WatcherError> {
    let (mut ws, _) = tokio_tungstenite::connect_async(&config.ws_url).await?;

    let mut next_id: u64 = 1;
    let mut pending_subscribe: HashMap<u64, Pubkey> = HashMap::new();
    let mut pending_cancel: HashSet<Pubkey> = HashSet::new();
    let mut subscription_of: HashMap<Pubkey, u64> = HashMap::new();
    let mut pubkey_of: HashMap<u64, Pubkey> = HashMap::new();

    for pk in desired.iter().copied() {
        send_subscribe(&mut ws, config, &mut next_id, &mut pending_subscribe, pk).await?;
    }

    let mut deadline = tokio::time::Instant::now() + READ_TIMEOUT;
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => {
                return Err(WatcherError::Timeout);
            }
            msg = ws.next() => {
                deadline = tokio::time::Instant::now() + READ_TIMEOUT;
                let Some(msg) = msg else {
                    return Ok(Outcome::Closed);
                };
                let Message::Text(text) = msg? else {
                    continue;
                };
                let Some(parsed) = parse_message(&text) else {
                    continue;
                };
                match parsed {
                    ParsedMessage::Notification { subscription_id, slot, lamports, data, owner } => {
                        let Some(&pubkey) = pubkey_of.get(&subscription_id) else {
                            continue;
                        };
                        let update = AccountUpdate { pubkey, slot, lamports, data, owner };
                        match tx.try_send(update) {
                            Ok(()) => {}
                            Err(mpsc::error::TrySendError::Full(dropped)) => {
                                eprintln!(
                                    "account watcher: update channel full (capacity {UPDATE_CHANNEL_CAPACITY}), dropped update for {}",
                                    dropped.pubkey
                                );
                            }
                            Err(mpsc::error::TrySendError::Closed(_)) => return Ok(Outcome::ConsumerGone),
                        }
                    }
                    ParsedMessage::SubscribeOk { id, subscription_id } => {
                        if let Some(pk) = pending_subscribe.remove(&id) {
                            if pending_cancel.remove(&pk) {
                                send_unsubscribe(&mut ws, &mut next_id, subscription_id).await?;
                            } else {
                                subscription_of.insert(pk, subscription_id);
                                pubkey_of.insert(subscription_id, pk);
                            }
                        }
                    }
                    ParsedMessage::UnsubscribeOk { .. } => {}
                    ParsedMessage::Error { id, message } => {
                        eprintln!("account watcher: RPC error: {message}");
                        if let Some(id) = id {
                            if let Some(pk) = pending_subscribe.remove(&id) {
                                pending_cancel.remove(&pk);
                            }
                        }
                    }
                }
            }
            cmd = commands.recv() => {
                let Some(cmd) = cmd else {
                    return Ok(Outcome::ConsumerGone);
                };
                match cmd {
                    WatchCommand::Watch(pk) => {
                        desired.insert(pk);
                        let already_subscribing = pending_subscribe.values().any(|&p| p == pk);
                        if !subscription_of.contains_key(&pk) && !already_subscribing {
                            send_subscribe(&mut ws, config, &mut next_id, &mut pending_subscribe, pk).await?;
                        }
                        pending_cancel.remove(&pk);
                    }
                    WatchCommand::Unwatch(pk) => {
                        desired.remove(&pk);
                        if let Some(sub_id) = subscription_of.remove(&pk) {
                            pubkey_of.remove(&sub_id);
                            send_unsubscribe(&mut ws, &mut next_id, sub_id).await?;
                        } else if pending_subscribe.values().any(|&p| p == pk) {
                            pending_cancel.insert(pk);
                        }
                    }
                }
            }
        }
    }
}

async fn send_subscribe(
    ws: &mut WsStream,
    config: &WatcherConfig,
    next_id: &mut u64,
    pending_subscribe: &mut HashMap<u64, Pubkey>,
    pubkey: Pubkey,
) -> Result<(), WatcherError> {
    let id = *next_id;
    *next_id += 1;
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "accountSubscribe",
        "params": [pubkey.to_string(), {"encoding": "base64", "commitment": config.commitment}],
    });
    ws.send(Message::Text(request.to_string())).await?;
    pending_subscribe.insert(id, pubkey);
    Ok(())
}

async fn send_unsubscribe(ws: &mut WsStream, next_id: &mut u64, subscription_id: u64) -> Result<(), WatcherError> {
    let id = *next_id;
    *next_id += 1;
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "accountUnsubscribe",
        "params": [subscription_id],
    });
    ws.send(Message::Text(request.to_string())).await?;
    Ok(())
}
