//! SOL/USD price conversion shared by both venues' trade/liquidity
//! ingestion. This module does **not** fetch a price — callers supply
//! `sol_usd_price` from wherever the live ingestion driver gets it; no
//! price source has been chosen or verified yet (see the crate root doc
//! comment). It only knows how to recognize the wrapped-SOL mint and
//! convert a lamport amount into USD given a price.

use momentum_core::domain::EventPayload;
use solana_pubkey::Pubkey;

/// Wrapped SOL's real SPL mint address (`So11111111111111111111111111111111111111112`),
/// as a compile-time constant — distinct from Pump's own
/// `NATIVE_SOL_QUOTE_SENTINEL` (an all-zero pubkey Pump's bonding curve
/// uses for the same "this is SOL" meaning, verified in `crates/pump`).
/// PumpSwap pools are ordinary SPL token accounts, so a SOL-paired pool
/// uses this real mint — confirmed against real fetched `Pool` accounts.
/// Base58-decoded once by hand rather than parsed at runtime on every
/// call, matching the cheap-`const` pattern `NATIVE_SOL_QUOTE_SENTINEL`
/// already uses.
pub const WRAPPED_SOL_MINT: Pubkey = Pubkey::new_from_array([
    6, 155, 136, 87, 254, 171, 129, 132, 251, 104, 127, 99, 70, 24, 192, 53, 218, 196, 57, 220, 26, 235, 59, 85, 152,
    160, 240, 0, 0, 0, 0, 1,
]);

pub const LAMPORTS_PER_SOL: u64 = 1_000_000_000;

pub fn is_wrapped_sol(mint: &Pubkey) -> bool {
    *mint == WRAPPED_SOL_MINT
}

/// Converts a lamport amount into USD at the given SOL/USD price. Rejects
/// (`None`) a non-positive, non-finite price, or a result that isn't
/// finite either, rather than silently producing a nonsense dollar
/// figure. Two failure modes matter here, not just one: a price feed that
/// fails open to exactly `0.0` (a common default) must not sail through
/// as "zero-value trade" — it must be refused just like a negative or
/// `NaN` price; and a price that's finite but extreme enough to overflow
/// the multiplication into `+inf` must be caught on the way out, since
/// `+inf`/`NaN` flowing into `risk_engine`'s running `buy_usd`/`sell_usd`
/// totals would poison every score derived from them (`sell_pressure`,
/// `demand_score`, `graduation_probability`), not just the one event.
pub fn lamports_to_usd(lamports: u64, sol_usd_price: f64) -> Option<f64> {
    if !sol_usd_price.is_finite() || sol_usd_price <= 0.0 {
        return None;
    }
    let usd = (lamports as f64 / LAMPORTS_PER_SOL as f64) * sol_usd_price;
    if !usd.is_finite() {
        return None;
    }
    Some(usd)
}

/// Shared `Buy`/`Sell` payload construction for both venues — kept in one
/// place so the field mapping can't drift between `pump.rs` and
/// `pumpswap.rs`. `cluster_id`/`quality` come from the caller (normally
/// `momentum_reputation::TraderLedger::observe_trade`, called before this
/// function so they reflect the wallet's history *before* this trade) —
/// `quality` is only meaningful (and only read by `risk_engine`) inside
/// the `Buy` arm; the `Sell` side of `EventPayload` carries no quality
/// field at all, so it's silently unused there rather than threaded
/// through for nothing.
pub(crate) fn buy_sell_payload(is_buy: bool, amount_usd: f64, cluster_id: Option<String>, quality: f64) -> EventPayload {
    if is_buy {
        EventPayload::Buy { buyer_cluster_id: cluster_id, buyer_quality: quality, amount_usd }
    } else {
        EventPayload::Sell { seller_cluster_id: cluster_id, amount_usd }
    }
}
