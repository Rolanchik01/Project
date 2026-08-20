import assert from 'node:assert/strict';
import test from 'node:test';
import { AdapterVersionMismatch } from '../src/adapter-contract.js';
import { EventKind } from '../src/domain.js';
import { replay } from '../src/replay.js';
import { base, registry, confirmedCandidateEvents } from './helpers.js';

test('replay is deterministic and confirms a safe candidate backed by two independent clusters', () => {
  const normal = replay(confirmedCandidateEvents(), registry());
  const shuffled = replay([...confirmedCandidateEvents()].reverse(), registry());
  const first = normal.timeline.at(-1).risk;
  const second = shuffled.timeline.at(-1).risk;
  assert.deepEqual(first, second);
  assert.equal(first.decision, 'confirmed_entry');
  assert.equal(first.independentStrongClusters, 2);
  assert.equal(first.hardBlocks.length, 0);
});

test('a new creator with a strong matched global narrative receives only probe size', () => {
  const events = [
    base('token', 1, EventKind.TOKEN_CREATED, { creatorClusterId: 'unknown', creatorHistoryScore: null }),
    base('pool', 2, EventKind.POOL_CREATED, { poolId: 'pool-1', initialLiquidityUsd: 10_000 }),
    base('holders', 3, EventKind.HOLDER_SNAPSHOT, { holders: [{ clusterId: 'a', share: 0.2 }] }),
    base('buy', 4, EventKind.BUY, { buyerClusterId: 'smart-a', buyerQuality: 0.9, amountUsd: 2_000 }),
    base('narrative', 5, EventKind.NARRATIVE_UPDATED, { mentionAcceleration: 0.95, authorsQuality: 0.85, semanticMatch: true, globalEventMatch: true, coordinationRisk: 0.05 }),
  ];
  const risk = replay(events, registry()).timeline.at(-1).risk;
  assert.equal(risk.decision, 'probe_entry');
  assert.equal(risk.positionMultiplier, 0.2);
});

test('a newly enabled freeze authority rejects the token immediately', () => {
  const events = [
    ...confirmedCandidateEvents(),
    base('freeze', 7, EventKind.AUTHORITY_CHANGED, { authority: 'freeze', active: true }),
  ];
  const risk = replay(events, registry()).timeline.at(-1).risk;
  assert.equal(risk.decision, 'reject');
  assert.deepEqual(risk.hardBlocks, ['freeze_authority_active']);
});

test('a post-launch mint is a fail-closed safety event', () => {
  const events = [
    ...confirmedCandidateEvents(),
    base('mint-after-launch', 7, EventKind.MINT_TO, { amount: '1000000', initialSupply: false }),
  ];
  const risk = replay(events, registry()).timeline.at(-1).risk;
  assert.equal(risk.decision, 'reject');
  assert.deepEqual(risk.hardBlocks, ['post_launch_mint']);
});

test('a protocol version mismatch halts the venue before a decision can be emitted', () => {
  const [event] = confirmedCandidateEvents();
  event.programVersion = 'unexpected-layout';
  assert.throws(() => replay([event], registry()), AdapterVersionMismatch);
});

test('a verifiable social link nudges the narrative score without substituting for it', () => {
  const withoutLink = confirmedCandidateEvents();
  const withLink = [
    ...confirmedCandidateEvents().slice(0, -1),
    base('narrative', 6, EventKind.NARRATIVE_UPDATED, { mentionAcceleration: 0.82, authorsQuality: 0.78, semanticMatch: true, coordinationRisk: 0.08 }),
    base('metadata', 7, EventKind.METADATA_CREATED, { socialLinks: ['https://x.com/example'] }),
  ];
  const baseline = replay(withoutLink, registry()).timeline.at(-1).risk;
  const withSocial = replay(withLink, registry()).timeline.at(-1).risk;
  assert.ok(withSocial.narrativeScore > baseline.narrativeScore);
});
