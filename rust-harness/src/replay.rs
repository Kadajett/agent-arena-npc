use std::collections::HashSet;

use chrono::Utc;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    brain::tactical_frame::TacticalFrame,
    character::Capability,
    execution::{
        packet::{
            ActionPacket, TacticalAction, TacticalIntent, TacticalMode, TacticalProposal,
            TacticalStyle,
        },
        validator::{ValidationContext, validate_packet},
    },
};

pub const TACTICAL_REPLAY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TacticalReplayFixture {
    pub schema_version: u32,
    pub case_id: String,
    pub frame: TacticalFrame,
    pub capabilities: HashSet<Capability>,
    pub scripted_proposal: TacticalProposal,
    pub expectation: ReplayExpectation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReplayExpectation {
    pub allowed_intents: Vec<TacticalIntent>,
    pub required_actions: Vec<ActionMatcher>,
    pub forbidden_actions: Vec<ActionMatcher>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActionMatcher {
    MoveTo {
        tile_x: Option<i32>,
        tile_y: Option<i32>,
    },
    Attack {
        target_id: Option<String>,
    },
    UseSkill {
        skill_id: Option<String>,
        target_id: Option<String>,
    },
    UseItem {
        item_id: Option<String>,
    },
    PickUp {
        drop_id: Option<String>,
    },
    SetTactics {
        style: Option<TacticalStyle>,
        mode: Option<TacticalMode>,
    },
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReplayEvaluation {
    pub passed: bool,
    pub semantics_check: ReplayCheck,
    pub packet_check: ReplayCheck,
    pub validation_reason_code: Option<String>,
    pub intent_check: ReplayCheck,
    pub missing_required_actions: Vec<ActionMatcher>,
    pub present_forbidden_actions: Vec<ActionMatcher>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReplayCheck {
    Passed,
    Failed,
}

/// Evaluate one tactical proposal against deterministic fixture facts.
///
/// This function never calls a model or MCP. A proposal passes only when its
/// schema semantics, complete runtime packet validation, and fixture assertions
/// all pass.
#[must_use]
pub fn evaluate_proposal(
    fixture: &TacticalReplayFixture,
    proposal: &TacticalProposal,
) -> ReplayEvaluation {
    let semantics = proposal.validate_semantics();
    let packet = ActionPacket::from_proposal(
        uuid::Uuid::new_v4(),
        fixture.frame.revision,
        fixture.frame.strategic_intent.revision,
        fixture.frame.self_state.scene.clone(),
        proposal.clone(),
    );
    let validation = validate_packet(
        &packet,
        &ValidationContext {
            minimum_valid_frame_revision: fixture.frame.revision,
            current_strategic_revision: fixture.frame.strategic_intent.revision,
            now: Utc::now(),
            capabilities: &fixture.capabilities,
            frame: &fixture.frame,
        },
    );
    let missing_required_actions = fixture
        .expectation
        .required_actions
        .iter()
        .filter(|matcher| {
            !proposal
                .actions
                .iter()
                .any(|action| matcher.matches(action))
        })
        .cloned()
        .collect::<Vec<_>>();
    let present_forbidden_actions = fixture
        .expectation
        .forbidden_actions
        .iter()
        .filter(|matcher| {
            proposal
                .actions
                .iter()
                .any(|action| matcher.matches(action))
        })
        .cloned()
        .collect::<Vec<_>>();
    let intent_allowed = fixture
        .expectation
        .allowed_intents
        .contains(&proposal.intent);
    let semantics_check = check(semantics.is_ok());
    let packet_check = check(validation.is_ok());
    let validation_reason_code = validation
        .err()
        .map(|error| error.reason_code().to_owned())
        .or_else(|| semantics.err().map(|error| error.to_string()));
    ReplayEvaluation {
        passed: semantics_check == ReplayCheck::Passed
            && packet_check == ReplayCheck::Passed
            && intent_allowed
            && missing_required_actions.is_empty()
            && present_forbidden_actions.is_empty(),
        semantics_check,
        packet_check,
        validation_reason_code,
        intent_check: check(intent_allowed),
        missing_required_actions,
        present_forbidden_actions,
    }
}

const fn check(passed: bool) -> ReplayCheck {
    if passed {
        ReplayCheck::Passed
    } else {
        ReplayCheck::Failed
    }
}

impl ActionMatcher {
    fn matches(&self, action: &TacticalAction) -> bool {
        match (self, action) {
            (
                Self::MoveTo { tile_x, tile_y },
                TacticalAction::MoveTo {
                    tile_x: actual_x,
                    tile_y: actual_y,
                },
            ) => optional_eq(*tile_x, *actual_x) && optional_eq(*tile_y, *actual_y),
            (Self::Attack { target_id }, TacticalAction::Attack { target_id: actual }) => {
                optional_str_eq(target_id.as_deref(), actual)
            }
            (
                Self::UseSkill {
                    skill_id,
                    target_id,
                },
                TacticalAction::UseSkill {
                    skill_id: actual_skill,
                    target_id: actual_target,
                },
            ) => {
                optional_str_eq(skill_id.as_deref(), actual_skill)
                    && target_id
                        .as_ref()
                        .is_none_or(|expected| actual_target.as_deref() == Some(expected.as_str()))
            }
            (Self::UseItem { item_id }, TacticalAction::UseItem { item_id: actual }) => {
                optional_str_eq(item_id.as_deref(), actual)
            }
            (Self::PickUp { drop_id }, TacticalAction::PickUp { drop_id: actual }) => {
                optional_str_eq(drop_id.as_deref(), actual)
            }
            (
                Self::SetTactics { style, mode },
                TacticalAction::SetTactics {
                    style: actual_style,
                    mode: actual_mode,
                },
            ) => optional_eq(*style, *actual_style) && optional_eq(*mode, *actual_mode),
            (Self::Stop, TacticalAction::Stop) => true,
            _ => false,
        }
    }
}

fn optional_eq<T: Copy + PartialEq>(expected: Option<T>, actual: T) -> bool {
    expected.is_none_or(|expected| expected == actual)
}

fn optional_str_eq(expected: Option<&str>, actual: &str) -> bool {
    expected.is_none_or(|expected| expected == actual)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{brain::strategic_intent::StrategicIntent, execution::packet::AbortCondition};

    fn fixture() -> TacticalReplayFixture {
        let mut frame = TacticalFrame::empty(StrategicIntent {
            revision: 7,
            ..StrategicIntent::default()
        });
        frame.revision = 12;
        frame.self_state.scene = Some("test".to_owned());
        frame.self_state.alive = Some(true);
        TacticalReplayFixture {
            schema_version: TACTICAL_REPLAY_SCHEMA_VERSION,
            case_id: "critical-no-heal".to_owned(),
            frame,
            capabilities: HashSet::from([Capability::Fight]),
            scripted_proposal: TacticalProposal {
                intent: TacticalIntent::Disengage,
                actions: vec![TacticalAction::SetTactics {
                    style: TacticalStyle::Flee,
                    mode: TacticalMode::SemiAuto,
                }],
                valid_for_ms: 1_000,
                abort_if: vec![AbortCondition::PlayerDied],
                rationale: None,
            },
            expectation: ReplayExpectation {
                allowed_intents: vec![TacticalIntent::Disengage],
                required_actions: vec![ActionMatcher::SetTactics {
                    style: Some(TacticalStyle::Flee),
                    mode: None,
                }],
                forbidden_actions: vec![ActionMatcher::Attack { target_id: None }],
            },
        }
    }

    #[test]
    fn scripted_fixture_passes_semantics_validation_and_assertions() {
        let fixture = fixture();
        let result = evaluate_proposal(&fixture, &fixture.scripted_proposal);
        assert_eq!(result.validation_reason_code, None);
        assert!(result.passed);
    }

    #[test]
    fn plausible_intent_with_forbidden_attack_fails() {
        let mut fixture = fixture();
        fixture
            .scripted_proposal
            .actions
            .push(TacticalAction::Attack {
                target_id: "invented".to_owned(),
            });
        let result = evaluate_proposal(&fixture, &fixture.scripted_proposal);
        assert!(!result.passed);
        assert_eq!(result.packet_check, ReplayCheck::Failed);
        assert_eq!(
            result.validation_reason_code.as_deref(),
            Some("unknown_target")
        );
        assert_eq!(result.present_forbidden_actions.len(), 1);
    }
}
