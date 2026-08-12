use agent_arena_npc_harness::replay::{ReplayCheck, TacticalReplayFixture, evaluate_proposal};

const CRITICAL_NO_HEAL: &str = include_str!("../fixtures/combat/critical-no-heal.json");

#[test]
fn committed_critical_health_fixture_passes_with_its_scripted_brain() {
    let fixture: TacticalReplayFixture =
        serde_json::from_str(CRITICAL_NO_HEAL).expect("complete replay fixture");
    let evaluation = evaluate_proposal(&fixture, &fixture.scripted_proposal);

    assert_eq!(evaluation.semantics_check, ReplayCheck::Passed);
    assert_eq!(evaluation.packet_check, ReplayCheck::Passed);
    assert!(evaluation.passed);
}

#[test]
fn fixture_schema_rejects_a_missing_authoritative_frame() {
    let mut fixture: serde_json::Value =
        serde_json::from_str(CRITICAL_NO_HEAL).expect("fixture JSON");
    fixture
        .as_object_mut()
        .expect("fixture object")
        .remove("frame");

    assert!(serde_json::from_value::<TacticalReplayFixture>(fixture).is_err());
}
