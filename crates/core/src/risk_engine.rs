//! Ported from `src/risk-engine.js`. Same formulas, same thresholds, same
//! decision table — see that file's history for the paper-trading rationale.

use crate::domain::{
    Authority, Event, EventPayload, HolderShare, NarrativeState, PoolState, ReplayState, TokenState,
};
use crate::scoring_config::ScoringConfig;

fn clamp(value: f64, min: f64, max: f64) -> f64 {
    value.max(min).min(max)
}

fn clamp01_100(value: f64) -> f64 {
    clamp(value, 0.0, 100.0)
}

fn pool_exit_liquidity(token: &TokenState) -> f64 {
    token
        .pools
        .values()
        .map(|pool| pool.exit_liquidity_usd.max(0.0))
        .sum()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardBlock {
    TokenNotCreated,
    MintAuthorityActive,
    FreezeAuthorityActive,
    PostLaunchMint,
    RestrictedTransferMechanism,
    UnsupportedTokenProgram,
    LiquidityRemoved,
}

impl HardBlock {
    pub fn as_str(&self) -> &'static str {
        match self {
            HardBlock::TokenNotCreated => "token_not_created",
            HardBlock::MintAuthorityActive => "mint_authority_active",
            HardBlock::FreezeAuthorityActive => "freeze_authority_active",
            HardBlock::PostLaunchMint => "post_launch_mint",
            HardBlock::RestrictedTransferMechanism => "restricted_transfer_mechanism",
            HardBlock::UnsupportedTokenProgram => "unsupported_token_program",
            HardBlock::LiquidityRemoved => "liquidity_removed",
        }
    }
}

fn hard_blocks(token: &TokenState) -> Vec<HardBlock> {
    let mut blocks = Vec::new();
    if !token.technical.created {
        blocks.push(HardBlock::TokenNotCreated);
    }
    if token.technical.mint_authority_active {
        blocks.push(HardBlock::MintAuthorityActive);
    }
    if token.technical.freeze_authority_active {
        blocks.push(HardBlock::FreezeAuthorityActive);
    }
    if token.technical.post_launch_mint {
        blocks.push(HardBlock::PostLaunchMint);
    }
    if token.technical.restricted_transfer_mechanism {
        blocks.push(HardBlock::RestrictedTransferMechanism);
    }
    if token.technical.unsupported_token_program {
        blocks.push(HardBlock::UnsupportedTokenProgram);
    }
    if token.lifecycle.liquidity_removed {
        blocks.push(HardBlock::LiquidityRemoved);
    }
    blocks
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Reject,
    Observe,
    ConfirmedEntry,
    ProbeEntry,
}

impl Decision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Decision::Reject => "reject",
            Decision::Observe => "observe",
            Decision::ConfirmedEntry => "confirmed_entry",
            Decision::ProbeEntry => "probe_entry",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RiskSnapshot {
    pub mint: String,
    pub hard_blocks: Vec<HardBlock>,
    pub safety_score: i64,
    pub creator_score: i64,
    pub demand_score: i64,
    pub narrative_score: i64,
    pub holder_concentration: f64,
    pub sell_pressure: i64,
    pub exit_liquidity_usd: f64,
    pub graduation_probability: i64,
    pub independent_strong_clusters: u32,
    pub decision: Decision,
    pub position_multiplier: f64,
    pub scoring_config_version: &'static str,
}

pub fn snapshot(token: &TokenState, config: &ScoringConfig) -> RiskSnapshot {
    let exit_liquidity_usd = pool_exit_liquidity(token);
    let blocks = hard_blocks(token);

    let holder_concentration = if token.holder_snapshot.is_empty() {
        1.0
    } else {
        token
            .holder_snapshot
            .iter()
            .map(|h| h.share)
            .fold(f64::MIN, f64::max)
    };

    let total_flow = token.flow.buy_usd + token.flow.sell_usd;
    let sell_pressure = if total_flow == 0.0 {
        0.0
    } else {
        ((token.flow.sell_usd / total_flow) * 100.0).round()
    };

    let strong_clusters = token
        .flow
        .buyers
        .values()
        .filter(|b| b.quality >= config.demand.strong_cluster_quality_threshold && b.net_buy_usd > 0.0)
        .count() as u32;

    let safety_score = if !blocks.is_empty() {
        0.0
    } else {
        clamp01_100(
            config.safety.base
                - if holder_concentration > config.safety.concentration_free_threshold {
                    (holder_concentration - config.safety.concentration_free_threshold)
                        * config.safety.concentration_penalty_per_unit
                } else {
                    0.0
                },
        )
    };

    let creator_score = match token.creator.history_score {
        None => config.creator.unknown_score,
        Some(history_score) => clamp01_100(history_score * 100.0),
    };

    let demand_score = clamp01_100(
        config.demand.base
            + (token.flow.buy_usd - token.flow.sell_usd).max(0.0).sqrt() * config.demand.buy_sell_delta_weight
            + (strong_clusters as f64) * config.demand.strong_cluster_weight
            - sell_pressure * config.demand.sell_pressure_weight,
    );

    let has_social_links = !token.metadata.social_links.is_empty();
    let narrative_score = clamp01_100(
        token.narrative.mention_acceleration * config.narrative.mention_acceleration_weight
            + token.narrative.authors_quality * config.narrative.authors_quality_weight
            + if token.narrative.semantic_match { config.narrative.semantic_match_bonus } else { 0.0 }
            + if has_social_links { config.narrative.social_links_bonus } else { 0.0 }
            - token.narrative.coordination_risk * config.narrative.coordination_risk_penalty,
    );

    let graduation_probability = clamp01_100(
        config.graduation.demand_weight * demand_score
            + config.graduation.narrative_weight * narrative_score
            + (exit_liquidity_usd / config.graduation.exit_liquidity_divisor)
                .min(config.graduation.exit_liquidity_cap)
            + (strong_clusters as f64) * config.graduation.strong_cluster_weight
            - sell_pressure * config.graduation.sell_pressure_penalty
            - holder_concentration * config.graduation.concentration_penalty,
    );

    let confirmed = &config.thresholds.confirmed_entry;
    let probe = &config.thresholds.probe_entry;

    let (decision, position_multiplier) = if !blocks.is_empty() {
        (Decision::Reject, 0.0)
    } else if safety_score >= confirmed.safety_score
        && creator_score >= confirmed.creator_score
        && demand_score >= confirmed.demand_score
        && narrative_score >= confirmed.narrative_score
        && exit_liquidity_usd >= confirmed.exit_liquidity_usd
        && holder_concentration <= confirmed.holder_concentration
        && strong_clusters >= confirmed.strong_clusters
    {
        (Decision::ConfirmedEntry, confirmed.position_multiplier)
    } else if token.creator.history_score.is_none()
        && safety_score >= probe.safety_score
        && demand_score >= probe.demand_score
        && narrative_score >= probe.narrative_score
        && token.narrative.global_event_match
        && exit_liquidity_usd >= probe.exit_liquidity_usd
    {
        (Decision::ProbeEntry, probe.position_multiplier)
    } else {
        (Decision::Observe, 0.0)
    };

    RiskSnapshot {
        mint: token.mint.clone(),
        hard_blocks: blocks,
        safety_score: safety_score.round() as i64,
        creator_score: creator_score.round() as i64,
        demand_score: demand_score.round() as i64,
        narrative_score: narrative_score.round() as i64,
        holder_concentration: (holder_concentration * 10_000.0).round() / 10_000.0,
        sell_pressure: sell_pressure as i64,
        exit_liquidity_usd: (exit_liquidity_usd * 100.0).round() / 100.0,
        graduation_probability: graduation_probability.round() as i64,
        independent_strong_clusters: strong_clusters,
        decision,
        position_multiplier,
        scoring_config_version: config.version,
    }
}

pub fn apply_event(replay_state: &mut ReplayState, event: &Event, config: &ScoringConfig) -> RiskSnapshot {
    let token = replay_state
        .tokens
        .entry(event.mint.clone())
        .or_insert_with(|| TokenState::new(event.mint.clone()));

    match &event.payload {
        EventPayload::TokenCreated {
            creator_cluster_id,
            creator_history_score,
            mint_authority_active,
            freeze_authority_active,
            transfer_hook,
            transfer_fee_bps,
            permanent_delegate,
            non_transferable,
            default_frozen,
            restricted_transfer_mechanism,
            unsupported_token_program,
        } => {
            token.technical.created = true;
            token.creator.cluster_id = creator_cluster_id.clone();
            token.creator.history_score = *creator_history_score;
            token.technical.mint_authority_active = *mint_authority_active;
            token.technical.freeze_authority_active = *freeze_authority_active;
            token.technical.transfer_hook = *transfer_hook;
            token.technical.transfer_fee_bps = *transfer_fee_bps;
            token.technical.permanent_delegate = *permanent_delegate;
            token.technical.non_transferable = *non_transferable;
            token.technical.default_frozen = *default_frozen;
            token.technical.restricted_transfer_mechanism = *restricted_transfer_mechanism;
            token.technical.unsupported_token_program = *unsupported_token_program;
        }
        EventPayload::MetadataCreated { social_links } => {
            let mut unique: Vec<String> = social_links.clone();
            unique.sort();
            unique.dedup();
            token.metadata.created = true;
            token.metadata.social_links = unique;
        }
        EventPayload::MintTo { initial_supply } => {
            // Initial supply creation is normal; any later mint is fail-closed.
            token.technical.post_launch_mint |= !initial_supply;
        }
        EventPayload::AuthorityChanged { authority, active } => match authority {
            Authority::Mint => token.technical.mint_authority_active = *active,
            Authority::Freeze => token.technical.freeze_authority_active = *active,
        },
        EventPayload::PoolCreated { pool_id, exit_liquidity_usd } => {
            token
                .pools
                .insert(pool_id.clone(), PoolState { exit_liquidity_usd: *exit_liquidity_usd });
        }
        EventPayload::CurveCreated => {
            token.lifecycle.curve_created = true;
        }
        EventPayload::LiquidityAdded { pool_id, amount_usd } => {
            let pool = token.pools.entry(pool_id.clone()).or_default();
            pool.exit_liquidity_usd += amount_usd;
        }
        EventPayload::LiquidityRemoved { pool_id, amount_usd, all_liquidity_removed } => {
            let pool = token.pools.entry(pool_id.clone()).or_default();
            pool.exit_liquidity_usd = (pool.exit_liquidity_usd - amount_usd).max(0.0);
            token.lifecycle.liquidity_removed |= all_liquidity_removed;
        }
        EventPayload::Buy { buyer_cluster_id, buyer_quality, amount_usd } => {
            token.flow.buy_usd += amount_usd;
            if let Some(cluster_id) = buyer_cluster_id {
                let buyer = token.flow.buyers.entry(cluster_id.clone()).or_default();
                buyer.quality = buyer.quality.max(*buyer_quality);
                buyer.net_buy_usd += amount_usd;
            }
        }
        EventPayload::Sell { seller_cluster_id, amount_usd } => {
            token.flow.sell_usd += amount_usd;
            if let Some(cluster_id) = seller_cluster_id {
                if let Some(buyer) = token.flow.buyers.get_mut(cluster_id) {
                    buyer.net_buy_usd -= amount_usd;
                }
            }
        }
        EventPayload::TokenTransfer => {
            token.transfer_count += 1;
        }
        EventPayload::HolderSnapshot { holders } => {
            let mut sorted: Vec<HolderShare> = holders.clone();
            sorted.sort_by(|a, b| a.cluster_id.cmp(&b.cluster_id));
            token.holder_snapshot = sorted;
        }
        EventPayload::NarrativeUpdated {
            mention_acceleration,
            authors_quality,
            semantic_match,
            coordination_risk,
            global_event_match,
        } => {
            token.narrative = NarrativeState {
                mention_acceleration: *mention_acceleration,
                authors_quality: *authors_quality,
                semantic_match: *semantic_match,
                coordination_risk: *coordination_risk,
                global_event_match: *global_event_match,
            };
        }
        EventPayload::Graduation => {
            token.lifecycle.graduated = true;
        }
        EventPayload::Migration => {
            token.lifecycle.migrated = true;
        }
    }

    replay_state.applied_events += 1;
    snapshot(replay_state.tokens.get(&event.mint).unwrap(), config)
}
