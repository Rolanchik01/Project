//! Runnable demo/live-verification for `account_watcher`: watches a couple
//! of real, currently-active PumpSwap pool accounts, then a few seconds in
//! sends an `Unwatch` for one and a `Watch` for a different one — proving
//! the dynamic add/remove path (not just the initial-set path) works
//! against the real endpoint, not only in isolated unit tests. Not a
//! production entrypoint — see `crates/live/src/lib.rs` doc comment for
//! what's deliberately not built yet (deciding which accounts to watch
//! from live events, wiring into `apply_update`).

use momentum_live::account_watcher::{run, WatchCommand, WatcherConfig, UPDATE_CHANNEL_CAPACITY};
use solana_pubkey::Pubkey;
use std::str::FromStr;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() {
    let pool_a = Pubkey::from_str("6w7dv74bn1R8BrmrraYCUuXx1v4dZtgk4Hf2quy4uoeb").unwrap();
    let pool_b = Pubkey::from_str("4qYJkETMAnGmzbeWakoED8im3q9mesAMUFjfUsJdJxSw").unwrap();
    let wrapped_sol_mint = Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap();

    let (update_tx, mut update_rx) = mpsc::channel(UPDATE_CHANNEL_CAPACITY);
    let (cmd_tx, cmd_rx) = mpsc::channel(16);
    let config = WatcherConfig { ws_url: "wss://api.mainnet-beta.solana.com".to_string(), commitment: "confirmed".to_string() };

    tokio::spawn(run(config, vec![pool_a, pool_b], cmd_rx, update_tx));
    eprintln!("watching {pool_a} and {pool_b} (ctrl-c to stop)...");

    let mut swapped = false;
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(5));

    loop {
        tokio::select! {
            update = update_rx.recv() => {
                let Some(u) = update else { break };
                println!("[slot {}] {} lamports={} owner={} data_len={}", u.slot, u.pubkey, u.lamports, u.owner, u.data.len());
            }
            _ = ticker.tick() => {
                if !swapped {
                    swapped = true;
                    eprintln!("swapping: unwatch {pool_b}, watch {wrapped_sol_mint}");
                    let _ = cmd_tx.send(WatchCommand::Unwatch(pool_b)).await;
                    let _ = cmd_tx.send(WatchCommand::Watch(wrapped_sol_mint)).await;
                }
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("shutting down");
                break;
            }
        }
    }
}
