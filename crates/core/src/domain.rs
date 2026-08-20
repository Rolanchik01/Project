//! Portable event schema and paper-replay state, ported from the validated
//! Stage 0 JavaScript reference (domain.js). Field presence and event-kind
//! validity that domain.js checked at runtime (validateEvent) are enforced
//! here at compile time instead: `Event`'s fields are all required by the
//! struct, and `EventPayload` is a closed enum, so there is no "missing
//! field" or "unknown kind" state to reject at runtime. `observedAtNs` was a
//! decimal string in JS only to survive `JSON`/`Number` precision loss above
//! 2^53; Rust's `u64` has no such limit, so it is a plain integer here.

use std::collections::HashMap;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Venue {
    Pump,
    #[serde(rename = "pumpswap")]
    PumpSwap,
    RaydiumCpmm,
    RaydiumClmm,
    RaydiumLaunchLab,
    MeteoraDlmm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    TokenCreated,
    MetadataCreated,
    MintTo,
    AuthorityChanged,
    PoolCreated,
    CurveCreated,
    Buy,
    Sell,
    TokenTransfer,
    Graduation,
    Migration,
    LiquidityAdded,
    LiquidityRemoved,
    HolderSnapshot,
    NarrativeUpdated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Authority {
    Mint,
    Freeze,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HolderShare {
    pub cluster_id: String,
    pub share: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EventPayload {
    TokenCreated {
        creator_cluster_id: Option<String>,
        creator_history_score: Option<f64>,
        mint_authority_active: bool,
        freeze_authority_active: bool,
        transfer_hook: bool,
        transfer_fee_bps: u32,
        unsupported_token_program: bool,
    },
    MetadataCreated {
        social_links: Vec<String>,
    },
    MintTo {
        initial_supply: bool,
    },
    AuthorityChanged {
        authority: Authority,
        active: bool,
    },
    PoolCreated {
        pool_id: String,
        exit_liquidity_usd: f64,
    },
    CurveCreated,
    Buy {
        buyer_cluster_id: Option<String>,
        buyer_quality: f64,
        amount_usd: f64,
    },
    Sell {
        seller_cluster_id: Option<String>,
        amount_usd: f64,
    },
    TokenTransfer,
    Graduation,
    Migration,
    LiquidityAdded {
        pool_id: String,
        amount_usd: f64,
    },
    LiquidityRemoved {
        pool_id: String,
        amount_usd: f64,
        all_liquidity_removed: bool,
    },
    HolderSnapshot {
        holders: Vec<HolderShare>,
    },
    NarrativeUpdated {
        mention_acceleration: f64,
        authors_quality: f64,
        semantic_match: bool,
        coordination_risk: f64,
        global_event_match: bool,
    },
}

impl EventPayload {
    pub fn kind(&self) -> EventKind {
        match self {
            EventPayload::TokenCreated { .. } => EventKind::TokenCreated,
            EventPayload::MetadataCreated { .. } => EventKind::MetadataCreated,
            EventPayload::MintTo { .. } => EventKind::MintTo,
            EventPayload::AuthorityChanged { .. } => EventKind::AuthorityChanged,
            EventPayload::PoolCreated { .. } => EventKind::PoolCreated,
            EventPayload::CurveCreated => EventKind::CurveCreated,
            EventPayload::Buy { .. } => EventKind::Buy,
            EventPayload::Sell { .. } => EventKind::Sell,
            EventPayload::TokenTransfer => EventKind::TokenTransfer,
            EventPayload::Graduation => EventKind::Graduation,
            EventPayload::Migration => EventKind::Migration,
            EventPayload::LiquidityAdded { .. } => EventKind::LiquidityAdded,
            EventPayload::LiquidityRemoved { .. } => EventKind::LiquidityRemoved,
            EventPayload::HolderSnapshot { .. } => EventKind::HolderSnapshot,
            EventPayload::NarrativeUpdated { .. } => EventKind::NarrativeUpdated,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: String,
    pub slot: u64,
    pub observed_at_ns: u64,
    pub signature: String,
    pub instruction_index: u32,
    pub venue: Venue,
    pub program_version: String,
    pub mint: String,
    pub payload: EventPayload,
}

impl Event {
    pub fn kind(&self) -> EventKind {
        self.payload.kind()
    }
}

/// Same ordering key as domain.js's `eventOrder`: slot, then observed time,
/// then signature, then instruction index, then id — so replay over the same
/// event set always reaches the same state regardless of arrival order.
pub fn event_order(a: &Event, b: &Event) -> std::cmp::Ordering {
    a.slot
        .cmp(&b.slot)
        .then_with(|| a.observed_at_ns.cmp(&b.observed_at_ns))
        .then_with(|| a.signature.cmp(&b.signature))
        .then_with(|| a.instruction_index.cmp(&b.instruction_index))
        .then_with(|| a.id.cmp(&b.id))
}

pub fn ordered_events(mut events: Vec<Event>) -> Vec<Event> {
    events.sort_by(event_order);
    events
}

#[derive(Debug, Clone, Default)]
pub struct CreatorInfo {
    pub cluster_id: Option<String>,
    pub history_score: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct TechnicalFlags {
    pub created: bool,
    pub mint_authority_active: bool,
    pub freeze_authority_active: bool,
    pub post_launch_mint: bool,
    pub transfer_hook: bool,
    pub transfer_fee_bps: u32,
    pub unsupported_token_program: bool,
}

#[derive(Debug, Clone, Default)]
pub struct TokenMetadata {
    pub created: bool,
    pub social_links: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct PoolState {
    pub exit_liquidity_usd: f64,
}

#[derive(Debug, Clone, Default)]
pub struct BuyerInfo {
    pub quality: f64,
    pub net_buy_usd: f64,
}

#[derive(Debug, Clone, Default)]
pub struct FlowState {
    pub buy_usd: f64,
    pub sell_usd: f64,
    pub buyers: HashMap<String, BuyerInfo>,
}

#[derive(Debug, Clone, Default)]
pub struct NarrativeState {
    pub mention_acceleration: f64,
    pub authors_quality: f64,
    pub semantic_match: bool,
    pub coordination_risk: f64,
    pub global_event_match: bool,
}

#[derive(Debug, Clone, Default)]
pub struct LifecycleState {
    pub curve_created: bool,
    pub graduated: bool,
    pub migrated: bool,
    pub liquidity_removed: bool,
}

#[derive(Debug, Clone)]
pub struct TokenState {
    pub mint: String,
    pub creator: CreatorInfo,
    pub technical: TechnicalFlags,
    pub metadata: TokenMetadata,
    pub pools: HashMap<String, PoolState>,
    pub holder_snapshot: Vec<HolderShare>,
    pub flow: FlowState,
    pub narrative: NarrativeState,
    pub lifecycle: LifecycleState,
    pub transfer_count: u64,
}

impl TokenState {
    pub fn new(mint: impl Into<String>) -> Self {
        Self {
            mint: mint.into(),
            creator: CreatorInfo::default(),
            technical: TechnicalFlags::default(),
            metadata: TokenMetadata::default(),
            pools: HashMap::new(),
            holder_snapshot: Vec::new(),
            flow: FlowState::default(),
            narrative: NarrativeState::default(),
            lifecycle: LifecycleState::default(),
            transfer_count: 0,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ReplayState {
    pub tokens: HashMap<String, TokenState>,
    /// Reserved for adapter halt tracking; unused so far, matching the JS
    /// reference (createReplayState initializes it but nothing writes to it
    /// yet — AdapterVersionMismatch stops replay via an error instead).
    pub halts: HashMap<String, ()>,
    pub applied_events: u64,
}

impl ReplayState {
    pub fn new() -> Self {
        Self::default()
    }
}
