//! Ported from `src/replay.js`. Runs the paper-only state machine over an
//! event batch; a version mismatch stops replay via `Err` instead of a
//! thrown exception. `replay_with_config` takes an explicit scoring config
//! (Rust has no default-parameter sugar) to compare threshold sets without
//! editing code.

use crate::adapter_contract::{AdapterRegistry, AdapterVersionMismatch};
use crate::domain::{ordered_events, Event, EventKind, ReplayState};
use crate::risk_engine::{apply_event, RiskSnapshot};
use crate::scoring_config::{ScoringConfig, DEFAULT_SCORING_CONFIG};

#[derive(Debug, Clone)]
pub struct TimelineEntry {
    pub event_id: String,
    pub slot: u64,
    pub kind: EventKind,
    pub risk: RiskSnapshot,
}

#[derive(Debug)]
pub struct ReplayOutcome {
    pub state: ReplayState,
    pub timeline: Vec<TimelineEntry>,
}

pub fn replay(events: Vec<Event>, adapter_registry: &AdapterRegistry) -> Result<ReplayOutcome, AdapterVersionMismatch> {
    replay_with_config(events, adapter_registry, &DEFAULT_SCORING_CONFIG)
}

pub fn replay_with_config(
    events: Vec<Event>,
    adapter_registry: &AdapterRegistry,
    config: &ScoringConfig,
) -> Result<ReplayOutcome, AdapterVersionMismatch> {
    let mut state = ReplayState::new();
    let mut timeline = Vec::with_capacity(events.len());
    for event in ordered_events(events) {
        adapter_registry.assert_compatible(&event)?;
        let risk = apply_event(&mut state, &event, config);
        timeline.push(TimelineEntry {
            event_id: event.id.clone(),
            slot: event.slot,
            kind: event.kind(),
            risk,
        });
    }
    Ok(ReplayOutcome { state, timeline })
}
