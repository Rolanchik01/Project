//! Runnable demo: same pattern as `pump_listener.rs`, subscribed to
//! PumpSwap's program instead. Proves `momentum_live`'s listener works
//! unchanged for a second venue — `listener::run` doesn't know or care
//! which program it's watching. Not a production entrypoint — see
//! `bin/pipeline.rs` for the full accountSubscribe + ingest/risk-engine/
//! recorder pipeline.

use momentum_live::listener::{run, ListenerConfig, EVENT_CHANNEL_CAPACITY};
use momentum_pumpswap::events::{decode_event, PumpSwapEvent};
use momentum_pumpswap::PUMPSWAP_PROGRAM_ID;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() {
    let (tx, mut rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
    let config = ListenerConfig {
        ws_url: "wss://api.mainnet-beta.solana.com".to_string(),
        program_id: PUMPSWAP_PROGRAM_ID.to_string(),
        commitment: "confirmed".to_string(),
    };

    tokio::spawn(run(config, tx));
    eprintln!("listening for live PumpSwap events (ctrl-c to stop)...");

    loop {
        tokio::select! {
            event = rx.recv() => {
                let Some(raw) = event else { break };
                match decode_event(&raw.data) {
                    Some(PumpSwapEvent::CreatePool(c)) => {
                        println!("[{}] CREATE_POOL pool={} base_mint={} quote_mint={}", raw.signature, c.pool, c.base_mint, c.quote_mint);
                    }
                    Some(PumpSwapEvent::Buy(b)) => {
                        println!("[{}] BUY pool={} base_amount_out={} quote_amount_in={}", raw.signature, b.pool, b.base_amount_out, b.user_quote_amount_in);
                    }
                    Some(PumpSwapEvent::Sell(s)) => {
                        println!("[{}] SELL pool={} base_amount_in={} quote_amount_out={}", raw.signature, s.pool, s.base_amount_in, s.user_quote_amount_out);
                    }
                    Some(PumpSwapEvent::Deposit(d)) => {
                        println!("[{}] DEPOSIT pool={} base_amount_in={} quote_amount_in={}", raw.signature, d.pool, d.base_amount_in, d.quote_amount_in);
                    }
                    Some(PumpSwapEvent::Withdraw(w)) => {
                        println!("[{}] WITHDRAW pool={} base_amount_out={} quote_amount_out={}", raw.signature, w.pool, w.base_amount_out, w.quote_amount_out);
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
