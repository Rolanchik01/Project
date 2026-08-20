/**
 * Stage 0 baseline scoring thresholds. These are the same numbers that used
 * to be hardcoded in risk-engine.js, pulled out so different threshold sets
 * can be walk-forward backtested without editing code (see README, "Границы
 * Stage 0"). They are starting research values, not a trading recommendation.
 *
 * `narrative.socialLinksBonus` has no historical precedent to pull from — it
 * is a placeholder estimate (smaller than semanticMatchBonus, since a linked
 * social account is weaker evidence than a matched narrative) and must be
 * confirmed by backtest before it drives real position sizing.
 */
export const DEFAULT_SCORING_CONFIG = Object.freeze({
  version: 'stage0-baseline-2026-08',
  safety: Object.freeze({
    base: 92,
    concentrationFreeThreshold: 0.35,
    concentrationPenaltyPerUnit: 120,
  }),
  creator: Object.freeze({
    unknownScore: 45,
  }),
  demand: Object.freeze({
    base: 18,
    buySellDeltaWeight: 0.65,
    strongClusterWeight: 15,
    sellPressureWeight: 0.35,
    strongClusterQualityThreshold: 0.75,
  }),
  narrative: Object.freeze({
    mentionAccelerationWeight: 60,
    authorsQualityWeight: 30,
    semanticMatchBonus: 10,
    socialLinksBonus: 5,
    coordinationRiskPenalty: 55,
  }),
  graduation: Object.freeze({
    demandWeight: 0.25,
    narrativeWeight: 0.2,
    exitLiquidityDivisor: 250,
    exitLiquidityCap: 20,
    strongClusterWeight: 4,
    sellPressurePenalty: 0.2,
    concentrationPenalty: 25,
  }),
  thresholds: Object.freeze({
    confirmedEntry: Object.freeze({
      safetyScore: 75,
      creatorScore: 60,
      demandScore: 55,
      narrativeScore: 45,
      exitLiquidityUsd: 5_000,
      holderConcentration: 0.35,
      strongClusters: 2,
      positionMultiplier: 1,
    }),
    probeEntry: Object.freeze({
      safetyScore: 85,
      demandScore: 45,
      narrativeScore: 65,
      exitLiquidityUsd: 8_000,
      positionMultiplier: 0.2,
    }),
  }),
});
