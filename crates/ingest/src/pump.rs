//! Pump-specific ingestion: `Candidate::TokenCreated` -> `TokenCreated`,
//! `Candidate::Trade` -> `Buy`/`Sell`, `Candidate::Graduated` -> `Graduation`.

use momentum_core::domain::{Event, EventPayload, Venue};
use momentum_pump::adapter::Candidate;
use momentum_pump::{BondingCurve, NATIVE_SOL_QUOTE_SENTINEL};
use momentum_token2022::MintExtensionFlags;

use crate::price::{buy_sell_payload, lamports_to_usd};
use crate::{is_known_token_program, EventContext};

/// Turns a real Pump `CreateEvent` (already decoded into a `Candidate`)
/// plus a Token-2022 mint inspection into the one domain event Stage 0's
/// risk-engine's hard-veto gate depends on. Returns `None` for any other
/// `Candidate` variant (`Trade`, `Graduated`) — those don't produce a
/// `TokenCreated` event and aren't handled by this function.
///
/// `creator_cluster_id`/`creator_history_score` are always `None` here —
/// that's wallet-intelligence data (Stage 4), not something a single
/// on-chain event carries.
pub fn ingest_pump_token_created(candidate: &Candidate, mint_flags: MintExtensionFlags, ctx: &EventContext) -> Option<Event> {
    let Candidate::TokenCreated { mint, token_program, .. } = candidate else {
        return None;
    };

    Some(Event {
        id: ctx.id.clone(),
        slot: ctx.slot,
        observed_at_ns: ctx.observed_at_ns,
        signature: ctx.signature.clone(),
        instruction_index: ctx.instruction_index,
        venue: Venue::Pump,
        program_version: ctx.program_version.clone(),
        mint: mint.to_string(),
        payload: EventPayload::TokenCreated {
            creator_cluster_id: None,
            creator_history_score: None,
            mint_authority_active: mint_flags.mint_authority_active,
            freeze_authority_active: mint_flags.freeze_authority_active,
            transfer_hook: mint_flags.transfer_hook,
            transfer_fee_bps: mint_flags.transfer_fee_bps,
            permanent_delegate: mint_flags.permanent_delegate,
            non_transferable: mint_flags.non_transferable,
            default_frozen: mint_flags.default_frozen,
            restricted_transfer_mechanism: mint_flags.has_restricted_transfer_mechanism(),
            unsupported_token_program: !is_known_token_program(token_program),
        },
    })
}

/// Turns a real Pump `Trade` candidate into a `Buy`/`Sell` domain event,
/// given the bonding curve's cached state (for `quote_mint`) and a
/// SOL/USD price. Refuses (`None`) for a non-SOL-quoted curve —
/// `quote_amount` wouldn't be SOL-denominated there, and converting it
/// with a SOL price would silently mislabel a different currency's amount
/// as USD. Also refuses if `sol_usd_price` isn't a sane positive number
/// (see `price::lamports_to_usd`).
///
/// `buyer_cluster_id`/`seller_cluster_id` are always `None` and
/// `buyer_quality` is always `0.0` — wallet intelligence (Stage 4) doesn't
/// exist yet. `risk_engine::apply_event` only reads `buyer_quality` inside
/// its `Some(cluster_id)` branch, so leaving the cluster id `None` makes
/// this placeholder value inert rather than a fabricated signal feeding
/// the demand score.
pub fn ingest_pump_trade(candidate: &Candidate, curve: &BondingCurve, sol_usd_price: f64, ctx: &EventContext) -> Option<Event> {
    let Candidate::Trade { mint, is_buy, quote_amount, .. } = candidate else {
        return None;
    };
    if curve.quote_mint != NATIVE_SOL_QUOTE_SENTINEL {
        return None;
    }
    let amount_usd = lamports_to_usd(*quote_amount, sol_usd_price)?;
    let payload = buy_sell_payload(*is_buy, amount_usd);

    Some(Event {
        id: ctx.id.clone(),
        slot: ctx.slot,
        observed_at_ns: ctx.observed_at_ns,
        signature: ctx.signature.clone(),
        instruction_index: ctx.instruction_index,
        venue: Venue::Pump,
        program_version: ctx.program_version.clone(),
        mint: mint.to_string(),
        payload,
    })
}

/// A bonding curve completing/graduating -> `EventPayload::Graduation`.
/// Needs no price data at all: the payload carries no fields, it's a pure
/// lifecycle marker (`risk_engine` just sets `lifecycle.graduated = true`).
pub fn ingest_pump_graduated(candidate: &Candidate, ctx: &EventContext) -> Option<Event> {
    let Candidate::Graduated { mint, .. } = candidate else {
        return None;
    };
    Some(Event {
        id: ctx.id.clone(),
        slot: ctx.slot,
        observed_at_ns: ctx.observed_at_ns,
        signature: ctx.signature.clone(),
        instruction_index: ctx.instruction_index,
        venue: Venue::Pump,
        program_version: ctx.program_version.clone(),
        mint: mint.to_string(),
        payload: EventPayload::Graduation,
    })
}
