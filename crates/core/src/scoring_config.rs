//! Stage 0 baseline scoring thresholds, ported unchanged from
//! `src/scoring-config.js`. Starting research values, not a trading
//! recommendation — see README, "Границы Stage 0" / "Stage 0 boundaries".

#[derive(Debug, Clone, Copy)]
pub struct SafetyConfig {
    pub base: f64,
    pub concentration_free_threshold: f64,
    pub concentration_penalty_per_unit: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct CreatorConfig {
    pub unknown_score: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct DemandConfig {
    pub base: f64,
    pub buy_sell_delta_weight: f64,
    pub strong_cluster_weight: f64,
    pub sell_pressure_weight: f64,
    pub strong_cluster_quality_threshold: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct NarrativeConfig {
    pub mention_acceleration_weight: f64,
    pub authors_quality_weight: f64,
    pub semantic_match_bonus: f64,
    pub social_links_bonus: f64,
    pub coordination_risk_penalty: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct GraduationConfig {
    pub demand_weight: f64,
    pub narrative_weight: f64,
    pub exit_liquidity_divisor: f64,
    pub exit_liquidity_cap: f64,
    pub strong_cluster_weight: f64,
    pub sell_pressure_penalty: f64,
    pub concentration_penalty: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct EntryThresholds {
    pub safety_score: f64,
    pub creator_score: f64,
    pub demand_score: f64,
    pub narrative_score: f64,
    pub exit_liquidity_usd: f64,
    pub holder_concentration: f64,
    pub strong_clusters: u32,
    pub position_multiplier: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct ProbeThresholds {
    pub safety_score: f64,
    pub demand_score: f64,
    pub narrative_score: f64,
    pub exit_liquidity_usd: f64,
    pub position_multiplier: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct Thresholds {
    pub confirmed_entry: EntryThresholds,
    pub probe_entry: ProbeThresholds,
}

#[derive(Debug, Clone, Copy)]
pub struct ScoringConfig {
    pub version: &'static str,
    pub safety: SafetyConfig,
    pub creator: CreatorConfig,
    pub demand: DemandConfig,
    pub narrative: NarrativeConfig,
    pub graduation: GraduationConfig,
    pub thresholds: Thresholds,
}

pub const DEFAULT_SCORING_CONFIG: ScoringConfig = ScoringConfig {
    version: "stage0-baseline-2026-08",
    safety: SafetyConfig {
        base: 92.0,
        concentration_free_threshold: 0.35,
        concentration_penalty_per_unit: 120.0,
    },
    creator: CreatorConfig { unknown_score: 45.0 },
    demand: DemandConfig {
        base: 18.0,
        buy_sell_delta_weight: 0.65,
        strong_cluster_weight: 15.0,
        sell_pressure_weight: 0.35,
        strong_cluster_quality_threshold: 0.75,
    },
    narrative: NarrativeConfig {
        mention_acceleration_weight: 60.0,
        authors_quality_weight: 30.0,
        semantic_match_bonus: 10.0,
        social_links_bonus: 5.0,
        coordination_risk_penalty: 55.0,
    },
    graduation: GraduationConfig {
        demand_weight: 0.25,
        narrative_weight: 0.2,
        exit_liquidity_divisor: 250.0,
        exit_liquidity_cap: 20.0,
        strong_cluster_weight: 4.0,
        sell_pressure_penalty: 0.2,
        concentration_penalty: 25.0,
    },
    thresholds: Thresholds {
        confirmed_entry: EntryThresholds {
            safety_score: 75.0,
            creator_score: 60.0,
            demand_score: 55.0,
            narrative_score: 45.0,
            exit_liquidity_usd: 5_000.0,
            holder_concentration: 0.35,
            strong_clusters: 2,
            position_multiplier: 1.0,
        },
        probe_entry: ProbeThresholds {
            safety_score: 85.0,
            demand_score: 45.0,
            narrative_score: 65.0,
            exit_liquidity_usd: 8_000.0,
            position_multiplier: 0.2,
        },
    },
};
