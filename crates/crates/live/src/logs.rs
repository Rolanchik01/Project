//! Pure parsing: turns one raw `logsNotification` WebSocket message into
//! the base64-decoded event payloads Pump's/PumpSwap's `decode_event`
//! expect. No networking here — kept separately testable from the
//! WebSocket connection/reconnect logic in `listener.rs`.

use base64::Engine;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct LogsNotification {
    params: LogsParams,
}

#[derive(Debug, Deserialize)]
struct LogsParams {
    result: LogsResult,
}

#[derive(Debug, Deserialize)]
struct LogsResult {
    value: LogsValue,
}

#[derive(Debug, Deserialize)]
struct LogsValue {
    signature: String,
    err: Option<serde_json::Value>,
    logs: Vec<String>,
}

/// One decoded on-chain instruction's raw event bytes, tagged with the
/// transaction signature it came from and its position among the
/// *successfully decoded* `Program data:` lines within that transaction's
/// logs. `log_index` is *not* the transaction's real on-chain instruction
/// index (the log stream doesn't carry that) — it only distinguishes
/// multiple events within the same transaction from each other, e.g. a
/// same-tx create-then-buy. Whatever assembles a full `core::domain::Event`
/// downstream is responsible for choosing a real `instruction_index` (or
/// documenting that it uses this ordinal instead).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawLogEvent {
    pub signature: String,
    pub log_index: u32,
    pub data: Vec<u8>,
}

/// Result of parsing one `logsNotification`: the successfully decoded
/// events, plus how many `Program data:` lines failed to base64-decode.
/// That's rare but real — e.g. Solana truncating an oversized `logs`
/// array mid-line for a transaction with many CPI calls — and a caller
/// must be able to notice and log the loss rather than it vanishing with
/// zero trace, which would otherwise mean a real trade silently never
/// reaches the risk engine.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExtractedEvents {
    pub events: Vec<RawLogEvent>,
    pub skipped_malformed: u32,
}

/// Parses one raw `logsNotification` JSON message and extracts every
/// `Program data:` line's base64 payload.
///
/// Skips (`Some(ExtractedEvents::default())`, not `None`) a transaction
/// that failed on-chain (`err` is not `null`) — a failed transaction's
/// logs can still contain `Program data:` lines from partial execution
/// before the failure (confirmed against real captured mainnet
/// notifications), and those don't represent a real state change, so
/// treating them as real events would be wrong.
///
/// Returns `None` if `raw` isn't a well-formed logsNotification at all —
/// e.g. the subscription confirmation/error message
/// (`{"result":<id>,...}` / `{"error":{...},...}`, no `params` field)
/// that arrives once right after subscribing, or any other RPC response
/// sharing the same connection. Not an error case: the caller should just
/// skip it, not treat it as a parse failure — `listener.rs` checks the
/// subscribe confirmation/error separately, before it ever calls this.
pub fn extract_events(raw: &str) -> Option<ExtractedEvents> {
    let notification: LogsNotification = serde_json::from_str(raw).ok()?;
    let value = notification.params.result.value;
    if value.err.is_some() {
        return Some(ExtractedEvents::default());
    }
    let mut result = ExtractedEvents::default();
    for log in &value.logs {
        if let Some(b64) = log.strip_prefix("Program data: ") {
            match base64::engine::general_purpose::STANDARD.decode(b64) {
                Ok(data) => {
                    let log_index = result.events.len() as u32;
                    result.events.push(RawLogEvent { signature: value.signature.clone(), log_index, data });
                }
                Err(_) => result.skipped_malformed += 1,
            }
        }
    }
    Some(result)
}
