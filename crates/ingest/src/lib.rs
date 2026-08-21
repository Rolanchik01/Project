//! Merges one venue's raw `Candidate` with correlated on-chain state (e.g. a
//! Token-2022 mint inspection) into `core::domain::Event` — the shape
//! `risk_engine`/`replay` already consume. Deliberately its own crate:
//! `core` cannot depend on `pump`/`pumpswap` (they depend on `core` for the
//! `VenueAdapter` trait), so the glue that knows about both sides has to
//! sit above all of them.
//!
//! Scope for this pass: only Pump's `TokenCreated` candidate — the
//! anti-rug-critical path (mint/freeze authority, transfer mechanism).
//! Trade/pool-liquidity events (`Buy`, `Sell`, `PoolCreated`) need a
//! SOL/USD conversion this crate does not have: `domain::EventPayload`
//! requires USD amounts, and no price source has been chosen or verified
//! yet, so those are deliberately not built here rather than invented.
//!
//! One `Candidate::TokenCreated` produces exactly **one** `Event`, not two.
//! An earlier draft of this module also emitted a `CurveCreated` marker
//! event alongside it, reasoning that Pump's `create` instruction brings
//! both the mint and the bonding curve into existence atomically. That
//! turned out to be wrong: `core::dedup`'s dedupe key is
//! `(venue, signature, instruction_index)` — one raw on-chain instruction
//! is assumed to produce exactly one `Event`, and every existing Stage 0
//! fixture (`crates/core/tests/support/mod.rs`) follows that rule. Two
//! events sharing one instruction's key would silently collide in
//! `dedupe_events` (a `HashMap` keyed by that tuple keeps only one) and in
//! `StreamDeduplicator` (the second would be rejected as a "duplicate").
//! `CurveCreated` also isn't read by `risk_engine::hard_blocks` or
//! `snapshot` today, so nothing downstream needs it yet — only
//! `TokenCreated` carries the safety flags the hard-veto gate depends on.
//!
//! Also not mapped here: `Candidate::TokenCreated`'s `is_mayhem_mode`,
//! `is_cashback_enabled`, and non-standard `quote_mint` fields. Those are
//! *venue pricing* risk (`momentum_pump::BondingCurve::is_standard`,
//! surfaced through `VenueAdapter::liquidity_risk`), not *token* safety —
//! folding them into `TokenCreated`'s hard blocks would conflate "this
//! curve can't be priced safely" with "this mint can rug holders", which
//! are different failure modes needing different handling downstream. A
//! trading decision must still gate on both `risk_engine`'s verdict *and*
//! `liquidity_risk() == Healthy` before ever sizing a position — that
//! combination isn't wired up yet (no decision/execution glue exists
//! before this crate).

use momentum_core::domain::{Event, EventPayload, Venue};
use momentum_pump::adapter::Candidate as PumpCandidate;
use momentum_token2022::MintExtensionFlags;
use solana_pubkey::Pubkey;

/// The legacy SPL Token program — a mint owned by this program has no
/// Token-2022 extensions at all (nothing for `inspect_mint` to find), which
/// is fine and expected, not itself a red flag.
pub const LEGACY_TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
pub const TOKEN_2022_PROGRAM_ID: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";

/// Everything about a raw event's place in the chain that a `Candidate`
/// itself doesn't carry (it only holds one instruction's decoded payload) —
/// supplied by whatever fed the adapter the raw bytes in the first place
/// (the recorder / RPC ingestion driver knows the slot, signature, and
/// instruction index the log came from).
#[derive(Debug, Clone)]
pub struct EventContext {
    pub id: String,
    pub slot: u64,
    pub observed_at_ns: u64,
    pub signature: String,
    pub instruction_index: u32,
    pub program_version: String,
}

fn is_known_token_program(token_program: &Pubkey) -> bool {
    let legacy: Pubkey = LEGACY_TOKEN_PROGRAM_ID.parse().expect("LEGACY_TOKEN_PROGRAM_ID is a valid pubkey");
    let token_2022: Pubkey = TOKEN_2022_PROGRAM_ID.parse().expect("TOKEN_2022_PROGRAM_ID is a valid pubkey");
    *token_program == legacy || *token_program == token_2022
}

/// Turns a real Pump `CreateEvent` (already decoded into a `Candidate`)
/// plus a Token-2022 mint inspection into the one domain event Stage 0's
/// risk-engine's hard-veto gate depends on. Returns `None` for any other
/// `Candidate` variant (`Trade`, `Graduated`) — those don't produce a
/// `TokenCreated` event and aren't handled by this function.
///
/// `creator_cluster_id`/`creator_history_score` are always `None` here —
/// that's wallet-intelligence data (Stage 4), not something a single
/// on-chain event carries.
pub fn ingest_pump_token_created(candidate: &PumpCandidate, mint_flags: MintExtensionFlags, ctx: &EventContext) -> Option<Event> {
    let PumpCandidate::TokenCreated { mint, token_program, .. } = candidate else {
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
