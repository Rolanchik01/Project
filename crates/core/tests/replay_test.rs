//! Ported from test/replay.test.js.
mod support;

use momentum_core::adapter_contract::AdapterVersionMismatch;
use momentum_core::domain::EventPayload;
use momentum_core::replay::replay;
use momentum_core::risk_engine::{Decision, HardBlock};
use support::{base, confirmed_candidate_events, registry};

#[test]
fn replay_is_deterministic_and_confirms_a_safe_candidate_backed_by_two_independent_clusters() {
    let normal = replay(confirmed_candidate_events(), &registry()).unwrap();
    let mut reversed = confirmed_candidate_events();
    reversed.reverse();
    let shuffled = replay(reversed, &registry()).unwrap();

    let first = &normal.timeline.last().unwrap().risk;
    let second = &shuffled.timeline.last().unwrap().risk;
    assert_eq!(first, second);
    assert_eq!(first.decision, Decision::ConfirmedEntry);
    assert_eq!(first.independent_strong_clusters, 2);
    assert!(first.hard_blocks.is_empty());
}

#[test]
fn a_new_creator_with_a_strong_matched_global_narrative_receives_only_probe_size() {
    let events = vec![
        base(
            "token",
            1,
            EventPayload::TokenCreated {
                creator_cluster_id: Some("unknown".to_string()),
                creator_history_score: None,
                mint_authority_active: false,
                freeze_authority_active: false,
                transfer_hook: false,
                transfer_fee_bps: 0,
                permanent_delegate: false,
                non_transferable: false,
                default_frozen: false,
                unsupported_token_program: false,
            },
        ),
        base("pool", 2, EventPayload::PoolCreated { pool_id: "pool-1".to_string(), exit_liquidity_usd: 10_000.0 }),
        base(
            "holders",
            3,
            EventPayload::HolderSnapshot {
                holders: vec![momentum_core::domain::HolderShare { cluster_id: "a".to_string(), share: 0.2 }],
            },
        ),
        base(
            "buy",
            4,
            EventPayload::Buy { buyer_cluster_id: Some("smart-a".to_string()), buyer_quality: 0.9, amount_usd: 2_000.0 },
        ),
        base(
            "narrative",
            5,
            EventPayload::NarrativeUpdated {
                mention_acceleration: 0.95,
                authors_quality: 0.85,
                semantic_match: true,
                coordination_risk: 0.05,
                global_event_match: true,
            },
        ),
    ];
    let outcome = replay(events, &registry()).unwrap();
    let risk = &outcome.timeline.last().unwrap().risk;
    assert_eq!(risk.decision, Decision::ProbeEntry);
    assert_eq!(risk.position_multiplier, 0.2);
}

#[test]
fn a_permanent_delegate_extension_rejects_the_token_even_with_no_transfer_hook_or_fee() {
    // Found during Stage 1 Token-2022 research, not in the Stage 0 JS
    // reference this domain model was ported from: a permanent delegate
    // can move or burn any holder's tokens at will, just as dangerous as a
    // transfer hook, so it must gate the same hard block.
    let events = vec![base(
        "token",
        1,
        EventPayload::TokenCreated {
            creator_cluster_id: Some("creator".to_string()),
            creator_history_score: Some(0.84),
            mint_authority_active: false,
            freeze_authority_active: false,
            transfer_hook: false,
            transfer_fee_bps: 0,
            permanent_delegate: true,
            non_transferable: false,
            default_frozen: false,
            unsupported_token_program: false,
        },
    )];
    let outcome = replay(events, &registry()).unwrap();
    let risk = &outcome.timeline.last().unwrap().risk;
    assert_eq!(risk.decision, Decision::Reject);
    assert_eq!(risk.hard_blocks, vec![HardBlock::RestrictedTransferMechanism]);
}

#[test]
fn a_newly_enabled_freeze_authority_rejects_the_token_immediately() {
    let mut events = confirmed_candidate_events();
    events.push(base("freeze", 7, EventPayload::AuthorityChanged { authority: momentum_core::domain::Authority::Freeze, active: true }));
    let outcome = replay(events, &registry()).unwrap();
    let risk = &outcome.timeline.last().unwrap().risk;
    assert_eq!(risk.decision, Decision::Reject);
    assert_eq!(risk.hard_blocks, vec![HardBlock::FreezeAuthorityActive]);
}

#[test]
fn a_post_launch_mint_is_a_fail_closed_safety_event() {
    let mut events = confirmed_candidate_events();
    events.push(base("mint-after-launch", 7, EventPayload::MintTo { initial_supply: false }));
    let outcome = replay(events, &registry()).unwrap();
    let risk = &outcome.timeline.last().unwrap().risk;
    assert_eq!(risk.decision, Decision::Reject);
    assert_eq!(risk.hard_blocks, vec![HardBlock::PostLaunchMint]);
}

#[test]
fn a_protocol_version_mismatch_halts_the_venue_before_a_decision_can_be_emitted() {
    let mut events = confirmed_candidate_events();
    let mut event = events.remove(0);
    event.program_version = "unexpected-layout".to_string();
    let result = replay(vec![event], &registry());
    assert!(matches!(result, Err(AdapterVersionMismatch { .. })));
}

#[test]
fn a_verifiable_social_link_nudges_the_narrative_score_without_substituting_for_it() {
    let without_link = confirmed_candidate_events();
    let mut with_link = confirmed_candidate_events();
    with_link.pop();
    with_link.push(base(
        "narrative",
        6,
        EventPayload::NarrativeUpdated {
            mention_acceleration: 0.82,
            authors_quality: 0.78,
            semantic_match: true,
            coordination_risk: 0.08,
            global_event_match: false,
        },
    ));
    with_link.push(base("metadata", 7, EventPayload::MetadataCreated { social_links: vec!["https://x.com/example".to_string()] }));

    let baseline = replay(without_link, &registry()).unwrap();
    let with_social = replay(with_link, &registry()).unwrap();
    assert!(with_social.timeline.last().unwrap().risk.narrative_score > baseline.timeline.last().unwrap().risk.narrative_score);
}
