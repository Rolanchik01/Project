export const Venue = Object.freeze({
  PUMP: 'pump',
  PUMP_SWAP: 'pumpswap',
  RAYDIUM_CPMM: 'raydium_cpmm',
  RAYDIUM_CLMM: 'raydium_clmm',
  RAYDIUM_LAUNCH_LAB: 'raydium_launch_lab',
  METEORA_DLMM: 'meteora_dlmm',
});

export const EventKind = Object.freeze({
  TOKEN_CREATED: 'TokenCreated',
  METADATA_CREATED: 'MetadataCreated',
  MINT_TO: 'MintTo',
  AUTHORITY_CHANGED: 'AuthorityChanged',
  POOL_CREATED: 'PoolCreated',
  CURVE_CREATED: 'CurveCreated',
  BUY: 'Buy',
  SELL: 'Sell',
  TOKEN_TRANSFER: 'TokenTransfer',
  GRADUATION: 'Graduation',
  MIGRATION: 'Migration',
  LIQUIDITY_ADDED: 'LiquidityAdded',
  LIQUIDITY_REMOVED: 'LiquidityRemoved',
  HOLDER_SNAPSHOT: 'HolderSnapshot',
  NARRATIVE_UPDATED: 'NarrativeUpdated',
});

const validKinds = new Set(Object.values(EventKind));

/** Validates the portable, NDJSON-safe event envelope used by every venue adapter. */
export function validateEvent(event) {
  const required = ['id', 'slot', 'observedAtNs', 'signature', 'instructionIndex', 'venue', 'programVersion', 'kind', 'payload'];
  for (const key of required) {
    if (event[key] === undefined || event[key] === null) throw new Error(`Event is missing ${key}`);
  }
  if (!validKinds.has(event.kind)) throw new Error(`Unsupported event kind: ${event.kind}`);
  if (!Number.isSafeInteger(event.slot) || event.slot < 0) throw new Error('slot must be a non-negative safe integer');
  if (!Number.isSafeInteger(event.instructionIndex) || event.instructionIndex < 0) throw new Error('instructionIndex must be a non-negative safe integer');
  if (!/^[0-9]+$/.test(String(event.observedAtNs))) throw new Error('observedAtNs must be a decimal nanosecond string');
  if (!event.payload.mint) throw new Error('payload.mint is required for token-scoped replay');
  return event;
}

export function eventOrder(left, right) {
  if (left.slot !== right.slot) return left.slot - right.slot;
  const timeOrder = BigInt(left.observedAtNs) < BigInt(right.observedAtNs) ? -1 : BigInt(left.observedAtNs) > BigInt(right.observedAtNs) ? 1 : 0;
  if (timeOrder !== 0) return timeOrder;
  if (left.signature !== right.signature) return left.signature.localeCompare(right.signature);
  if (left.instructionIndex !== right.instructionIndex) return left.instructionIndex - right.instructionIndex;
  return left.id.localeCompare(right.id);
}

export function orderedEvents(events) {
  return events.map(validateEvent).toSorted(eventOrder);
}

export function createTokenState(mint) {
  return {
    mint,
    creator: { clusterId: null, historyScore: null },
    technical: {
      created: false,
      mintAuthorityActive: false,
      freezeAuthorityActive: false,
      postLaunchMint: false,
      transferHook: false,
      transferFeeBps: 0,
      unsupportedTokenProgram: false,
    },
    metadata: { created: false, socialLinks: [] },
    pools: {},
    holderSnapshot: [],
    flow: { buyUsd: 0, sellUsd: 0, buyers: {} },
    narrative: { mentionAcceleration: 0, authorsQuality: 0, semanticMatch: false, coordinationRisk: 0, globalEventMatch: false },
    lifecycle: { curveCreated: false, graduated: false, migrated: false, liquidityRemoved: false },
    transferCount: 0,
  };
}

export function createReplayState() {
  return { tokens: {}, halts: {}, appliedEvents: 0 };
}
