//! PumpSwap-specific ingestion: `Candidate::Trade` -> `Buy`/`Sell`,
//! `Candidate::PoolCreated` -> `PoolCreated`, `Candidate::Deposit` ->
//! `LiquidityAdded`, `Candidate::Withdraw` -> `LiquidityRemoved`.
//!
//! Unlike Pump (always memecoin-base/SOL-quote), which side of a PumpSwap
//! pool is SOL is not fixed — confirmed against ~20 real, currently active
//! pools during research: roughly half have SOL as `base_mint`, half as
//! `quote_mint`, and at least one pairs two non-SOL tokens (a memecoin
//! against USDC) with no SOL leg at all. Both functions here check both
//! sides and refuse (`None`) if neither is SOL, rather than assuming a
//! fixed convention.

use momentum_core::domain::{Event, EventPayload, Venue};
use momentum_pumpswap::adapter::Candidate;
use momentum_pumpswap::Pool;
use solana_pubkey::Pubkey;

use crate::price::{buy_sell_payload, is_wrapped_sol, lamports_to_usd};
use crate::EventContext;

enum SolSide {
    Base,
    Quote,
}

fn sol_side(base_mint: &Pubkey, quote_mint: &Pubkey) -> Option<SolSide> {
    if is_wrapped_sol(base_mint) {
        Some(SolSide::Base)
    } else if is_wrapped_sol(quote_mint) {
        Some(SolSide::Quote)
    } else {
        None
    }
}

/// Turns a real PumpSwap `Trade` candidate into a `Buy`/`Sell` domain
/// event, given the pool's cached state (for `base_mint`/`quote_mint`) and
/// a SOL/USD price. `pool` must be the same pool the candidate trades
/// against — callers normally get it from
/// `PumpSwapAdapter::pool(&candidate's pool address)`.
///
/// `buyer_cluster_id`/`seller_cluster_id`/`buyer_quality` follow the same
/// "wallet intelligence not built yet" placeholder convention as
/// `pump::ingest_pump_trade` — see its doc comment.
pub fn ingest_pumpswap_trade(candidate: &Candidate, pool: &Pool, sol_usd_price: f64, ctx: &EventContext) -> Option<Event> {
    let Candidate::Trade { is_buy, base_amount, quote_amount, .. } = candidate else {
        return None;
    };

    let (mint, sol_amount) = match sol_side(&pool.base_mint, &pool.quote_mint)? {
        SolSide::Base => (pool.quote_mint, *base_amount),
        SolSide::Quote => (pool.base_mint, *quote_amount),
    };
    let amount_usd = lamports_to_usd(sol_amount, sol_usd_price)?;
    let payload = buy_sell_payload(*is_buy, amount_usd);

    Some(Event {
        id: ctx.id.clone(),
        slot: ctx.slot,
        observed_at_ns: ctx.observed_at_ns,
        signature: ctx.signature.clone(),
        instruction_index: ctx.instruction_index,
        venue: Venue::PumpSwap,
        program_version: ctx.program_version.clone(),
        mint: mint.to_string(),
        payload,
    })
}

/// A brand-new PumpSwap pool -> `EventPayload::PoolCreated`.
/// `exit_liquidity_usd` values the pool's SOL-side reserve at creation
/// time (how much SOL could realistically be pulled back out, not a
/// doubled-up TVL guess across both sides — a deliberately conservative
/// reading given no confirmed spec for this field's exact intended
/// semantics survived the Stage 0 JS -> Rust port). Kept current after
/// creation by `ingest_pumpswap_deposit`/`ingest_pumpswap_withdraw` below.
pub fn ingest_pumpswap_pool_created(candidate: &Candidate, sol_usd_price: f64, ctx: &EventContext) -> Option<Event> {
    let Candidate::PoolCreated { pool, base_mint, quote_mint, base_amount_in, quote_amount_in, .. } = candidate else {
        return None;
    };

    let (mint, sol_amount) = match sol_side(base_mint, quote_mint)? {
        SolSide::Base => (*quote_mint, *base_amount_in),
        SolSide::Quote => (*base_mint, *quote_amount_in),
    };
    let exit_liquidity_usd = lamports_to_usd(sol_amount, sol_usd_price)?;

    Some(Event {
        id: ctx.id.clone(),
        slot: ctx.slot,
        observed_at_ns: ctx.observed_at_ns,
        signature: ctx.signature.clone(),
        instruction_index: ctx.instruction_index,
        venue: Venue::PumpSwap,
        program_version: ctx.program_version.clone(),
        mint: mint.to_string(),
        payload: EventPayload::PoolCreated { pool_id: pool.to_string(), exit_liquidity_usd },
    })
}

/// A liquidity deposit -> `EventPayload::LiquidityAdded`, valuing only the
/// SOL-side amount actually added, same convention as
/// `ingest_pumpswap_pool_created`. `pool` must be the same pool the
/// candidate deposits into (its `base_mint`/`quote_mint` are needed since
/// `Candidate::Deposit` itself doesn't carry them).
pub fn ingest_pumpswap_deposit(candidate: &Candidate, pool: &Pool, sol_usd_price: f64, ctx: &EventContext) -> Option<Event> {
    let Candidate::Deposit { pool: pool_address, base_amount_in, quote_amount_in, .. } = candidate else {
        return None;
    };

    let (mint, sol_amount) = match sol_side(&pool.base_mint, &pool.quote_mint)? {
        SolSide::Base => (pool.quote_mint, *base_amount_in),
        SolSide::Quote => (pool.base_mint, *quote_amount_in),
    };
    let amount_usd = lamports_to_usd(sol_amount, sol_usd_price)?;

    Some(Event {
        id: ctx.id.clone(),
        slot: ctx.slot,
        observed_at_ns: ctx.observed_at_ns,
        signature: ctx.signature.clone(),
        instruction_index: ctx.instruction_index,
        venue: Venue::PumpSwap,
        program_version: ctx.program_version.clone(),
        mint: mint.to_string(),
        payload: EventPayload::LiquidityAdded { pool_id: pool_address.to_string(), amount_usd },
    })
}

/// PumpSwap permanently locks this many raw LP units at pool creation
/// (`CreatePoolEvent.minimum_liquidity`, confirmed as exactly 100 in the
/// real fixture `crates/pumpswap/tests/events_test.rs::CREATE_POOL_B64`) —
/// the same anti-first-depositor-attack pattern as Uniswap V2's
/// `MINIMUM_LIQUIDITY`. No withdrawal can ever burn this remainder, so a
/// withdrawal leaving only this much (or less, from any per-pool rounding)
/// behind has removed every unit of *actually withdrawable* liquidity —
/// confirmed against a real withdrawal that left exactly 100 raw units of
/// `lp_mint_supply` unburned.
const PERMANENTLY_LOCKED_MINIMUM_LIQUIDITY: u64 = 100;

/// A liquidity withdrawal -> `EventPayload::LiquidityRemoved`, same
/// SOL-side-only valuation convention as `ingest_pumpswap_deposit`.
/// `all_liquidity_removed` is true once this withdrawal has burned the
/// pool's LP supply down to (or below) `PERMANENTLY_LOCKED_MINIMUM_LIQUIDITY`
/// — see `momentum_pumpswap::events::WithdrawEvent`'s doc comment for why
/// `lp_mint_supply` is the *pre*-withdrawal total, not what's left after.
pub fn ingest_pumpswap_withdraw(candidate: &Candidate, pool: &Pool, sol_usd_price: f64, ctx: &EventContext) -> Option<Event> {
    let Candidate::Withdraw { pool: pool_address, base_amount_out, quote_amount_out, lp_token_amount_in, lp_mint_supply, .. } = candidate
    else {
        return None;
    };

    let (mint, sol_amount) = match sol_side(&pool.base_mint, &pool.quote_mint)? {
        SolSide::Base => (pool.quote_mint, *base_amount_out),
        SolSide::Quote => (pool.base_mint, *quote_amount_out),
    };
    let amount_usd = lamports_to_usd(sol_amount, sol_usd_price)?;
    let remaining_supply = lp_mint_supply.saturating_sub(*lp_token_amount_in);
    let all_liquidity_removed = remaining_supply <= PERMANENTLY_LOCKED_MINIMUM_LIQUIDITY;

    Some(Event {
        id: ctx.id.clone(),
        slot: ctx.slot,
        observed_at_ns: ctx.observed_at_ns,
        signature: ctx.signature.clone(),
        instruction_index: ctx.instruction_index,
        venue: Venue::PumpSwap,
        program_version: ctx.program_version.clone(),
        mint: mint.to_string(),
        payload: EventPayload::LiquidityRemoved { pool_id: pool_address.to_string(), amount_usd, all_liquidity_removed },
    })
}
