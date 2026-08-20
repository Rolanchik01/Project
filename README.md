import { AdapterRegistry } from '../src/adapter-contract.js';
import { EventKind, Venue } from '../src/domain.js';

export const version = 'pump-layout-2026-08';
export const registry = () => new AdapterRegistry().register(Venue.PUMP, version);

export const base = (id, slot, kind, payload) => ({
  id, slot, observedAtNs: String(slot * 1_000), signature: `sig-${slot}`, instructionIndex: 0,
  venue: Venue.PUMP, programVersion: version, kind, payload: { mint: 'MintA', ...payload },
});

export function confirmedCandidateEvents() {
  return [
    base('token', 1, EventKind.TOKEN_CREATED, { creatorClusterId: 'creator', creatorHistoryScore: 0.84 }),
    base('pool', 2, EventKind.POOL_CREATED, { poolId: 'pool-1', initialLiquidityUsd: 12_000 }),
    base('holders', 3, EventKind.HOLDER_SNAPSHOT, { holders: [{ clusterId: 'a', share: 0.19 }, { clusterId: 'b', share: 0.13 }] }),
    base('buy-1', 4, EventKind.BUY, { buyerClusterId: 'smart-a', buyerQuality: 0.91, amountUsd: 3_000 }),
    base('buy-2', 5, EventKind.BUY, { buyerClusterId: 'smart-b', buyerQuality: 0.88, amountUsd: 2_000 }),
    base('narrative', 6, EventKind.NARRATIVE_UPDATED, { mentionAcceleration: 0.82, authorsQuality: 0.78, semanticMatch: true, coordinationRisk: 0.08 }),
  ];
}
