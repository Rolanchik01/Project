//! Shared fixtures, ported from test/helpers.js.
#![allow(dead_code)]

use momentum_core::adapter_contract::AdapterRegistry;
use momentum_core::domain::{Event, EventPayload, HolderShare, Venue};

pub const VERSION: &str = "pump-layout-2026-08";

pub fn registry() -> AdapterRegistry {
    AdapterRegistry::new().register(Venue::Pump, VERSION)
}

pub fn base(id: &str, slot: u64, payload: EventPayload) -> Event {
    Event {
        id: id.to_string(),
        slot,
        observed_at_ns: slot * 1_000,
        signature: format!("sig-{slot}"),
        instruction_index: 0,
        venue: Venue::Pump,
        program_version: VERSION.to_string(),
        mint: "MintA".to_string(),
        payload,
    }
}

pub fn confirmed_candidate_events() -> Vec<Event> {
    vec![
        base(
            "token",
            1,
            EventPayload::TokenCreated {
                creator_cluster_id: Some("creator".to_string()),
                creator_history_score: Some(0.84),
                mint_authority_active: false,
                freeze_authority_active: false,
                transfer_hook: false,
                transfer_fee_bps: 0,
                unsupported_token_program: false,
            },
        ),
        base(
            "pool",
            2,
            EventPayload::PoolCreated { pool_id: "pool-1".to_string(), exit_liquidity_usd: 12_000.0 },
        ),
        base(
            "holders",
            3,
            EventPayload::HolderSnapshot {
                holders: vec![
                    HolderShare { cluster_id: "a".to_string(), share: 0.19 },
                    HolderShare { cluster_id: "b".to_string(), share: 0.13 },
                ],
            },
        ),
        base(
            "buy-1",
            4,
            EventPayload::Buy {
                buyer_cluster_id: Some("smart-a".to_string()),
                buyer_quality: 0.91,
                amount_usd: 3_000.0,
            },
        ),
        base(
            "buy-2",
            5,
            EventPayload::Buy {
                buyer_cluster_id: Some("smart-b".to_string()),
                buyer_quality: 0.88,
                amount_usd: 2_000.0,
            },
        ),
        base(
            "narrative",
            6,
            EventPayload::NarrativeUpdated {
                mention_acceleration: 0.82,
                authors_quality: 0.78,
                semantic_match: true,
                coordination_risk: 0.08,
                global_event_match: false,
            },
        ),
    ]
}
