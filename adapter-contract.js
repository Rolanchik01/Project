/**
 * Two independent Geyser/Yellowstone gRPC feeds will both deliver the same
 * on-chain instruction. This module gives the ingestion layer one place to
 * decide "have I already seen this exact instruction" before it ever
 * reaches the recorder or the risk engine.
 *
 * The identity of one on-chain instruction is (venue, signature,
 * instructionIndex) — the same transaction can carry several instructions
 * for the same venue, so signature alone is not enough.
 */

export function eventDedupeKey(event) {
  return `${event.venue}:${event.signature}:${event.instructionIndex}`;
}

/**
 * Pure batch dedup: given a set of already-collected events (e.g. two
 * recorded NDJSON files merged for replay), keep one copy per instruction —
 * the one with the earliest observedAtNs, since that is the feed that saw
 * it first and is the more useful latency sample.
 */
export function dedupeEvents(events) {
  const seen = new Map();
  for (const event of events) {
    const key = eventDedupeKey(event);
    const existing = seen.get(key);
    if (!existing || BigInt(event.observedAtNs) < BigInt(existing.observedAtNs)) {
      seen.set(key, event);
    }
  }
  return [...seen.values()];
}

/**
 * Streaming dedup for live ingestion: two feeds push events as they arrive
 * and only the first copy of each instruction should be forwarded
 * downstream (to the recorder / risk engine). Unlike dedupeEvents, this
 * keeps whichever copy arrives first in wall-clock order, since that is
 * what a live pipeline actually has to decide with.
 */
export class StreamDeduplicator {
  #seen = new Set();

  /** Returns true if the event is new and should be forwarded; false if it's a duplicate. */
  admit(event) {
    const key = eventDedupeKey(event);
    if (this.#seen.has(key)) return false;
    this.#seen.add(key);
    return true;
  }

  size() {
    return this.#seen.size;
  }
}
