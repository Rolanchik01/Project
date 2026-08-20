import { orderedEvents, createReplayState } from './domain.js';
import { applyEvent } from './risk-engine.js';
import { DEFAULT_SCORING_CONFIG } from './scoring-config.js';

/**
 * Runs the paper-only state machine. A version mismatch throws and stops
 * replay. `config` is a versioned scoring config (see scoring-config.js) —
 * pass a different one to compare threshold sets without editing code.
 */
export function replay(events, adapterRegistry, config = DEFAULT_SCORING_CONFIG) {
  const state = createReplayState();
  const timeline = [];
  for (const event of orderedEvents(events)) {
    adapterRegistry.assertCompatible(event);
    const risk = applyEvent(state, event, config);
    timeline.push({ eventId: event.id, slot: event.slot, kind: event.kind, risk });
  }
  return { state, timeline };
}
