//! Group G.8: reports on `recordings/paper_trades.ndjson`, the trade
//! ledger `bin/pipeline.rs`'s live `Portfolio` wiring (Group G.6/G.7)
//! accumulates across every run — the answer to "is this strategy
//! actually profitable" this project's stated goal needs.
//!
//! Deliberately reads *this* log, not a replay of `recordings/pipeline.ndjson`
//! through a hypothetical backtester: `core::domain::Event`'s `Buy`/`Sell`
//! payloads only ever carried `amount_usd` (a dollar flow figure), never a
//! token amount or the underlying curve/pool reserves at that moment — so
//! there is no way to reconstruct "what would a specific paper position,
//! opened at event N, have been worth at event M" from that log alone.
//! Accurate valuation needs the real, live-cached `BondingCurve`/`Pool`
//! quote math `bin/pipeline.rs` already has while running, which a
//! historical replay does not. Building a synthetic backtester on data
//! that can't support accurate position valuation would produce numbers
//! this project's rigor standard couldn't stand behind — so instead,
//! `pipeline.rs` values positions for real, live, as it runs, and *this*
//! binary summarizes what actually happened across every run so far.
//!
//! Run with `cargo run --bin paper_trade_report`.

use std::collections::HashMap;
use std::io::BufRead;

const PAPER_TRADES_PATH: &str = "recordings/paper_trades.ndjson";
/// This project's stated success bar: stable profit of at least this much
/// per day on the default `$1000` starting capital
/// (`momentum_portfolio::DEFAULT_POSITION_SIZING_CONFIG.initial_capital_usd`).
const DAILY_PROFIT_TARGET_USD: f64 = 100.0;
const NS_PER_DAY: f64 = 86_400.0 * 1_000_000_000.0;

#[derive(Debug, Clone)]
struct ClosedTradeRecord {
    #[allow(dead_code)]
    mint: String,
    pnl_usd: f64,
    reason: String,
    opened_at_ns: u64,
    closed_at_ns: u64,
}

fn main() {
    let file = match std::fs::File::open(PAPER_TRADES_PATH) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("No paper trades recorded yet ({PAPER_TRADES_PATH} doesn't exist). Run `cargo run --bin pipeline` first.");
            return;
        }
        Err(e) => {
            eprintln!("paper_trade_report: failed to open {PAPER_TRADES_PATH}: {e}");
            std::process::exit(1);
        }
    };

    let mut opened_count: u64 = 0;
    let mut still_open: HashMap<String, u64> = HashMap::new(); // mint -> opened_at_ns, for whatever's still open at EOF
    let mut closed: Vec<ClosedTradeRecord> = Vec::new();
    let mut skipped = 0u32;

    for (line_no, line) in std::io::BufReader::new(file).lines().enumerate() {
        let Ok(line) = line else {
            skipped += 1;
            continue;
        };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            eprintln!("paper_trade_report: skipping unparseable line {}", line_no + 1);
            skipped += 1;
            continue;
        };
        match value.get("kind").and_then(|k| k.as_str()) {
            Some("opened") => {
                opened_count += 1;
                if let (Some(mint), Some(opened_at_ns)) = (value.get("mint").and_then(|v| v.as_str()), value.get("opened_at_ns").and_then(|v| v.as_u64())) {
                    still_open.insert(mint.to_string(), opened_at_ns);
                }
            }
            Some("closed") => {
                let (Some(mint), Some(pnl_usd), Some(reason), Some(opened_at_ns), Some(closed_at_ns)) = (
                    value.get("mint").and_then(|v| v.as_str()),
                    value.get("pnl_usd").and_then(|v| v.as_f64()),
                    value.get("reason").and_then(|v| v.as_str()),
                    value.get("opened_at_ns").and_then(|v| v.as_u64()),
                    value.get("closed_at_ns").and_then(|v| v.as_u64()),
                ) else {
                    eprintln!("paper_trade_report: skipping malformed 'closed' entry at line {}", line_no + 1);
                    skipped += 1;
                    continue;
                };
                still_open.remove(mint);
                closed.push(ClosedTradeRecord { mint: mint.to_string(), pnl_usd, reason: reason.to_string(), opened_at_ns, closed_at_ns });
            }
            _ => {
                eprintln!("paper_trade_report: skipping entry with unrecognized/missing 'kind' at line {}", line_no + 1);
                skipped += 1;
            }
        }
    }

    println!("=== Paper trading report ({PAPER_TRADES_PATH}) ===\n");

    if skipped > 0 {
        println!("(skipped {skipped} unreadable/malformed line(s))\n");
    }

    println!("Positions opened (all-time): {opened_count}");
    println!("Positions closed:            {}", closed.len());
    println!("Currently open:              {}", still_open.len());

    if closed.is_empty() {
        println!("\nNo closed trades yet — nothing to report on P&L or the daily-profit target.");
        if !still_open.is_empty() {
            println!("{} position(s) still open: {:?}", still_open.len(), still_open.keys().collect::<Vec<_>>());
        }
        return;
    }

    let total_pnl_usd: f64 = closed.iter().map(|t| t.pnl_usd).sum();
    let wins = closed.iter().filter(|t| t.pnl_usd > 0.0).count();
    let losses = closed.iter().filter(|t| t.pnl_usd < 0.0).count();
    let breakeven = closed.len() - wins - losses;
    let win_rate = wins as f64 / closed.len() as f64;
    let avg_pnl_usd = total_pnl_usd / closed.len() as f64;

    let mut by_reason: HashMap<&str, u32> = HashMap::new();
    for t in &closed {
        *by_reason.entry(t.reason.as_str()).or_insert(0) += 1;
    }

    println!("\n--- Realized P&L ---");
    println!("Total realized P&L: ${total_pnl_usd:.2}");
    println!("Average P&L/trade:  ${avg_pnl_usd:.2}");
    println!("Win rate:           {:.1}% ({wins} win / {losses} loss / {breakeven} breakeven)", win_rate * 100.0);
    println!("Exit reasons:");
    for (reason, count) in {
        let mut v: Vec<_> = by_reason.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        v
    } {
        println!("  {reason}: {count}");
    }

    // Elapsed time across every closed trade's real observed timestamps —
    // not wall-clock "how long has this report been running", since these
    // trades may span multiple past `pipeline` runs.
    let earliest_ns = closed.iter().map(|t| t.opened_at_ns).min().unwrap();
    let latest_ns = closed.iter().map(|t| t.closed_at_ns).max().unwrap();
    let elapsed_days = (latest_ns.saturating_sub(earliest_ns)) as f64 / NS_PER_DAY;

    println!("\n--- Against the ${DAILY_PROFIT_TARGET_USD:.0}/day target ---");
    if elapsed_days < 1.0 / 24.0 {
        // Less than an hour of real elapsed time is too little to divide
        // into a "$/day" figure without the result being mostly noise —
        // reported as a plain total instead of manufacturing a misleading
        // extrapolated rate.
        println!("Elapsed span is under an hour ({:.2} real minutes) — too little data for a meaningful $/day rate yet.", elapsed_days * 24.0 * 60.0);
        println!("Total realized P&L so far: ${total_pnl_usd:.2}");
    } else {
        let pnl_per_day = total_pnl_usd / elapsed_days;
        println!("Elapsed span: {elapsed_days:.2} days (first entry to last exit)");
        println!("Realized P&L rate: ${pnl_per_day:.2}/day");
        if pnl_per_day >= DAILY_PROFIT_TARGET_USD {
            println!("=> At this rate, the ${DAILY_PROFIT_TARGET_USD:.0}/day target is being met.");
        } else {
            println!("=> At this rate, the ${DAILY_PROFIT_TARGET_USD:.0}/day target is NOT being met.");
        }
    }

    if !still_open.is_empty() {
        println!("\n({} position(s) still open, not included in realized P&L above)", still_open.len());
    }
}
