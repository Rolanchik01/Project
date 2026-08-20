import assert from 'node:assert/strict';
import test from 'node:test';
import { eventDedupeKey, dedupeEvents, StreamDeduplicator } from '../src/dedup.js';

const feedEvent = (overrides) => ({
  venue: 'pump', signature: 'sig-1', instructionIndex: 0, observedAtNs: '1000', payload: 'a', ...overrides,
});

test('eventDedupeKey identifies one on-chain instruction by venue, signature and instructionIndex', () => {
  assert.equal(eventDedupeKey(feedEvent({})), 'pump:sig-1:0');
});

test('dedupeEvents keeps the copy with the earliest observedAtNs across two feeds', () => {
  const fromFeedA = feedEvent({ observedAtNs: '2000', payload: 'feed-a' });
  const fromFeedB = feedEvent({ observedAtNs: '1500', payload: 'feed-b' });
  const [winner] = dedupeEvents([fromFeedA, fromFeedB]);
  assert.equal(winner.payload, 'feed-b');
});

test('dedupeEvents treats different instructions in the same transaction as distinct', () => {
  const first = feedEvent({ instructionIndex: 0 });
  const second = feedEvent({ instructionIndex: 1 });
  assert.equal(dedupeEvents([first, second]).length, 2);
});

test('StreamDeduplicator admits the first copy of an instruction and rejects the duplicate', () => {
  const dedup = new StreamDeduplicator();
  const event = feedEvent({});
  assert.equal(dedup.admit(event), true);
  assert.equal(dedup.admit(event), false);
  assert.equal(dedup.size(), 1);
});

test('StreamDeduplicator treats different signatures as independent instructions', () => {
  const dedup = new StreamDeduplicator();
  assert.equal(dedup.admit(feedEvent({ signature: 'sig-1' })), true);
  assert.equal(dedup.admit(feedEvent({ signature: 'sig-2' })), true);
  assert.equal(dedup.size(), 2);
});
