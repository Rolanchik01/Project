//! Runnable demo: connects to the real public Solana RPC, subscribes to
//! Pump program logs, and prints every real-time Create/Trade/Complete
//! event as it happens. Proves `momentum_live`'s listener + `momentum_pump`'s
//! already-tested decoder work together against the live chain, not just
//! against captured fixtures. Not a production entrypoint — no recorder,
//! no risk-engine wiring (see `crates/live/src/lib.rs` doc comment for
//! what's deliberately not built yet).

use momentum_live::listener::{run, ListenerConfig, EVENT_CHANNEL_CAPACITY};
use momentum_pump::events::{decode_event, PumpEvent};
use momentum_pump::PUMP_PROGRAM_ID;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() {
    let (tx, mut rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
    let config = ListenerConfig {
        ws_url: "wss://api.mainnet-beta.solana.com".to_string(),
        program_id: PUMP_PROGRAM_ID.to_string(),
        commitment: "confirmed".to_string(),
    };

    tokio::spawn(run(config, tx));
    eprintln!("listening for live Pump events (ctrl-c to stop)...");

    loop {
        tokio::select! {
            event = rx.recv() => {
                let Some(raw) = event else { break };
                match decode_event(&raw.data) {
                    Some(PumpEvent::Create(c)) => {
                        println!("[{}] CREATE mint={} name={:?} symbol={:?}", raw.signature, c.mint, c.name, c.symbol);
                    }
                    Some(PumpEvent::Trade(t)) => {
                        let side = if t.is_buy { "BUY" } else { "SELL" };
                        println!("[{}] {side} mint={} sol_amount={} token_amount={}", raw.signature, t.mint, t.sol_amount, t.token_amount);
                    }
                    Some(PumpEvent::Complete(c)) => {
                        println!("[{}] GRADUATED mint={}", raw.signature, c.mint);
                    }
                    None => {}
                }
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("shutting down");
                break;
            }
        }
    }
}
