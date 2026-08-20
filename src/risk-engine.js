import { EventKind, createTokenState } from './domain.js';
import { DEFAULT_SCORING_CONFIG } from './scoring-config.js';

const clamp = (value, min = 0, max = 100) => Math.max(min, Math.min(max, value));

function poolExitLiquidity(token) {
  return Object.values(token.pools).reduce((sum, pool) => sum + Math.max(0, pool.exitLiquidityUsd || 0), 0);
}

function hardBlocks(token) {
  const blocks = [];
  if (!token.technical.created) blocks.push('token_not_created');
  if (token.technical.mintAuthorityActive) blocks.push('mint_authority_active');
  if (token.technical.freezeAuthorityActive) blocks.push('freeze_authority_active');
  if (token.technical.postLaunchMint) blocks.push('post_launch_mint');
  if (token.technical.transferHook || token.technical.transferFeeBps > 0) blocks.push('restricted_transfer_mechanism');
  if (token.technical.unsupportedTokenProgram) blocks.push('unsupported_token_program');
  if (token.lifecycle.liquidityRemoved) blocks.push('liquidity_removed');
  return blocks;
}

export function snapshot(token, config = DEFAULT_SCORING_CONFIG) {
  const exitLiquidityUsd = poolExitLiquidity(token);
  const blocks = hardBlocks(token);
  const holderConcentration = token.holderSnapshot.length === 0
    ? 1
    : Math.max(...token.holderSnapshot.map((holder) => holder.share));
  const totalFlow = token.flow.buyUsd + token.flow.sellUsd;
  const sellPressure = totalFlow === 0 ? 0 : Math.round((token.flow.sellUsd / totalFlow) * 100);
  const strongClusters = Object.values(token.flow.buyers)
    .filter((buyer) => buyer.quality >= config.demand.strongClusterQualityThreshold && buyer.netBuyUsd > 0).length;

  const safetyScore = blocks.length > 0 ? 0 : clamp(
    config.safety.base - (holderConcentration > config.safety.concentrationFreeThreshold
      ? (holderConcentration - config.safety.concentrationFreeThreshold) * config.safety.concentrationPenaltyPerUnit
      : 0),
  );
  const creatorScore = token.creator.historyScore === null
    ? config.creator.unknownScore
    : clamp(token.creator.historyScore * 100);
  const demandScore = clamp(
    config.demand.base
      + Math.sqrt(Math.max(0, token.flow.buyUsd - token.flow.sellUsd)) * config.demand.buySellDeltaWeight
      + strongClusters * config.demand.strongClusterWeight
      - sellPressure * config.demand.sellPressureWeight,
  );
  // socialLinks presence is a small, verifiable signal (the metadata account actually links out);
  // it is not a substitute for the full X/narrative engine that lands in Stage 5.
  const hasSocialLinks = token.metadata.socialLinks.length > 0;
  const narrativeScore = clamp(
    token.narrative.mentionAcceleration * config.narrative.mentionAccelerationWeight
      + token.narrative.authorsQuality * config.narrative.authorsQualityWeight
      + (token.narrative.semanticMatch ? config.narrative.semanticMatchBonus : 0)
      + (hasSocialLinks ? config.narrative.socialLinksBonus : 0)
      - token.narrative.coordinationRisk * config.narrative.coordinationRiskPenalty,
  );
  const graduationProbability = clamp(
    config.graduation.demandWeight * demandScore
      + config.graduation.narrativeWeight * narrativeScore
      + Math.min(exitLiquidityUsd / config.graduation.exitLiquidityDivisor, config.graduation.exitLiquidityCap)
      + strongClusters * config.graduation.strongClusterWeight
      - sellPressure * config.graduation.sellPressurePenalty
      - holderConcentration * config.graduation.concentrationPenalty,
  );

  const confirmed = config.thresholds.confirmedEntry;
  const probe = config.thresholds.probeEntry;

  let decision = 'observe';
  let positionMultiplier = 0;
  if (blocks.length > 0) {
    decision = 'reject';
  } else if (
    safetyScore >= confirmed.safetyScore && creatorScore >= confirmed.creatorScore
    && demandScore >= confirmed.demandScore && narrativeScore >= confirmed.narrativeScore
    && exitLiquidityUsd >= confirmed.exitLiquidityUsd && holderConcentration <= confirmed.holderConcentration
    && strongClusters >= confirmed.strongClusters
  ) {
    decision = 'confirmed_entry';
    positionMultiplier = confirmed.positionMultiplier;
  } else if (
    token.creator.historyScore === null && safetyScore >= probe.safetyScore && demandScore >= probe.demandScore
    && narrativeScore >= probe.narrativeScore && token.narrative.globalEventMatch
    && exitLiquidityUsd >= probe.exitLiquidityUsd
  ) {
    decision = 'probe_entry';
    positionMultiplier = probe.positionMultiplier;
  }

  return {
    mint: token.mint,
    hardBlocks: blocks,
    safetyScore: Math.round(safetyScore),
    creatorScore: Math.round(creatorScore),
    demandScore: Math.round(demandScore),
    narrativeScore: Math.round(narrativeScore),
    holderConcentration: Number(holderConcentration.toFixed(4)),
    sellPressure,
    exitLiquidityUsd: Number(exitLiquidityUsd.toFixed(2)),
    graduationProbability: Math.round(graduationProbability),
    independentStrongClusters: strongClusters,
    decision,
    positionMultiplier,
    scoringConfigVersion: config.version,
  };
}

export function applyEvent(replayState, event, config = DEFAULT_SCORING_CONFIG) {
  const { payload } = event;
  const token = replayState.tokens[payload.mint] ?? createTokenState(payload.mint);
  replayState.tokens[payload.mint] = token;

  switch (event.kind) {
    case EventKind.TOKEN_CREATED:
      token.technical.created = true;
      token.creator = { clusterId: payload.creatorClusterId ?? null, historyScore: payload.creatorHistoryScore ?? null };
      Object.assign(token.technical, {
        mintAuthorityActive: Boolean(payload.mintAuthorityActive),
        freezeAuthorityActive: Boolean(payload.freezeAuthorityActive),
        transferHook: Boolean(payload.transferHook),
        transferFeeBps: Number(payload.transferFeeBps ?? 0),
        unsupportedTokenProgram: Boolean(payload.unsupportedTokenProgram),
      });
      break;
    case EventKind.METADATA_CREATED:
      token.metadata = {
        created: true,
        socialLinks: [...new Set(payload.socialLinks ?? [])].sort(),
      };
      break;
    case EventKind.MINT_TO:
      // Initial supply creation is normal; any later mint is a fail-closed event.
      token.technical.postLaunchMint ||= !Boolean(payload.initialSupply);
      break;
    case EventKind.AUTHORITY_CHANGED:
      if (payload.authority === 'mint') token.technical.mintAuthorityActive = Boolean(payload.active);
      if (payload.authority === 'freeze') token.technical.freezeAuthorityActive = Boolean(payload.active);
      break;
    case EventKind.POOL_CREATED:
      token.pools[payload.poolId] = { exitLiquidityUsd: Number(payload.exitLiquidityUsd ?? payload.initialLiquidityUsd ?? 0) };
      break;
    case EventKind.CURVE_CREATED:
      token.lifecycle.curveCreated = true;
      break;
    case EventKind.LIQUIDITY_ADDED:
      token.pools[payload.poolId] ??= { exitLiquidityUsd: 0 };
      token.pools[payload.poolId].exitLiquidityUsd += Number(payload.amountUsd ?? 0);
      break;
    case EventKind.LIQUIDITY_REMOVED:
      token.pools[payload.poolId] ??= { exitLiquidityUsd: 0 };
      token.pools[payload.poolId].exitLiquidityUsd = Math.max(0, token.pools[payload.poolId].exitLiquidityUsd - Number(payload.amountUsd ?? 0));
      token.lifecycle.liquidityRemoved ||= Boolean(payload.allLiquidityRemoved);
      break;
    case EventKind.BUY:
      token.flow.buyUsd += Number(payload.amountUsd ?? 0);
      if (payload.buyerClusterId) {
        const buyer = token.flow.buyers[payload.buyerClusterId] ?? { quality: 0, netBuyUsd: 0 };
        buyer.quality = Math.max(buyer.quality, Number(payload.buyerQuality ?? 0));
        buyer.netBuyUsd += Number(payload.amountUsd ?? 0);
        token.flow.buyers[payload.buyerClusterId] = buyer;
      }
      break;
    case EventKind.SELL:
      token.flow.sellUsd += Number(payload.amountUsd ?? 0);
      if (payload.sellerClusterId && token.flow.buyers[payload.sellerClusterId]) {
        token.flow.buyers[payload.sellerClusterId].netBuyUsd -= Number(payload.amountUsd ?? 0);
      }
      break;
    case EventKind.TOKEN_TRANSFER:
      token.transferCount += 1;
      break;
    case EventKind.HOLDER_SNAPSHOT:
      token.holderSnapshot = [...(payload.holders ?? [])]
        .map(({ clusterId, share }) => ({ clusterId, share: Number(share) }))
        .sort((a, b) => a.clusterId.localeCompare(b.clusterId));
      break;
    case EventKind.NARRATIVE_UPDATED:
      token.narrative = {
        mentionAcceleration: Number(payload.mentionAcceleration ?? 0),
        authorsQuality: Number(payload.authorsQuality ?? 0),
        semanticMatch: Boolean(payload.semanticMatch),
        coordinationRisk: Number(payload.coordinationRisk ?? 0),
        globalEventMatch: Boolean(payload.globalEventMatch),
      };
      break;
    case EventKind.GRADUATION:
      token.lifecycle.graduated = true;
      break;
    case EventKind.MIGRATION:
      token.lifecycle.migrated = true;
      break;
    default:
      break;
  }
  replayState.appliedEvents += 1;
  return snapshot(token, config);
}
