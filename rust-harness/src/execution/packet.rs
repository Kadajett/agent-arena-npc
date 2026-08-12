use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::world::TilePosition;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ActionPacket {
    pub id: Uuid,
    pub decision_id: Uuid,
    pub frame_revision: u64,
    pub strategic_revision: u64,
    pub scene: Option<String>,
    pub created_at: DateTime<Utc>,
    pub proposal: TacticalProposal,
}

impl ActionPacket {
    #[must_use]
    pub fn from_proposal(
        decision_id: Uuid,
        frame_revision: u64,
        strategic_revision: u64,
        scene: Option<String>,
        proposal: TacticalProposal,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            decision_id,
            frame_revision,
            strategic_revision,
            scene,
            created_at: Utc::now(),
            proposal,
        }
    }
}

/// Model-owned tactical output. The runtime adds every identity and revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TacticalProposal {
    pub intent: TacticalIntent,
    #[schemars(length(max = 8))]
    pub actions: Vec<TacticalAction>,
    #[schemars(range(min = 100, max = 5_000))]
    pub valid_for_ms: u64,
    #[schemars(length(max = 7))]
    pub abort_if: Vec<AbortCondition>,
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TacticalProposalSemanticError {
    #[error("continue intent must contain no actions")]
    ContinueHasActions,
    #[error("non-continue intent must contain a matching action")]
    MissingMatchingAction,
    #[error("valid_for_ms must be between 100 and 5000")]
    InvalidLifetime,
    #[error("a tactical proposal may contain at most eight actions")]
    TooManyActions,
    #[error("a tactical proposal may contain at most seven abort conditions")]
    TooManyAbortConditions,
}

impl TacticalProposal {
    /// Verify that the structured intent and action list describe the same operation.
    ///
    /// # Errors
    /// Returns a stable semantic error when a model emits contradictory structured output.
    pub fn validate_semantics(&self) -> Result<(), TacticalProposalSemanticError> {
        if !(100..=5_000).contains(&self.valid_for_ms) {
            return Err(TacticalProposalSemanticError::InvalidLifetime);
        }
        if self.actions.len() > 8 {
            return Err(TacticalProposalSemanticError::TooManyActions);
        }
        if self.abort_if.len() > 7 {
            return Err(TacticalProposalSemanticError::TooManyAbortConditions);
        }
        if self.intent == TacticalIntent::Continue {
            return if self.actions.is_empty() {
                Ok(())
            } else {
                Err(TacticalProposalSemanticError::ContinueHasActions)
            };
        }
        let matches = self.actions.iter().any(|action| match self.intent {
            TacticalIntent::Continue => false,
            TacticalIntent::Attack => matches!(
                action,
                TacticalAction::Attack { .. } | TacticalAction::UseSkill { .. }
            ),
            TacticalIntent::UseSkill => matches!(action, TacticalAction::UseSkill { .. }),
            TacticalIntent::UseItem => matches!(action, TacticalAction::UseItem { .. }),
            TacticalIntent::Loot => matches!(action, TacticalAction::PickUp { .. }),
            TacticalIntent::Reposition | TacticalIntent::Disengage => {
                matches!(
                    action,
                    TacticalAction::MoveTo { .. }
                        | TacticalAction::SetTactics {
                            style: TacticalStyle::Flee,
                            ..
                        }
                )
            }
            TacticalIntent::Stop => matches!(action, TacticalAction::Stop),
        });
        if matches {
            Ok(())
        } else {
            Err(TacticalProposalSemanticError::MissingMatchingAction)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TacticalIntent {
    Continue,
    Attack,
    UseSkill,
    UseItem,
    Loot,
    Reposition,
    Disengage,
    Stop,
}

impl TacticalIntent {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Continue => "continue",
            Self::Attack => "attack",
            Self::UseSkill => "use_skill",
            Self::UseItem => "use_item",
            Self::Loot => "loot",
            Self::Reposition => "reposition",
            Self::Disengage => "disengage",
            Self::Stop => "stop",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TacticalAction {
    MoveTo {
        tile_x: i32,
        tile_y: i32,
    },
    Attack {
        target_id: String,
    },
    UseSkill {
        skill_id: String,
        target_id: Option<String>,
    },
    UseItem {
        item_id: String,
    },
    PickUp {
        drop_id: String,
    },
    SetTactics {
        style: TacticalStyle,
        mode: TacticalMode,
    },
    Stop,
}

impl TacticalAction {
    pub fn destination(&self) -> Option<TilePosition> {
        match self {
            Self::MoveTo { tile_x, tile_y } => Some(TilePosition {
                x: *tile_x,
                y: *tile_y,
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TacticalStyle {
    CloseUp,
    LongRange,
    DuckAndWeave,
    Flee,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TacticalMode {
    SemiAuto,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AbortCondition {
    HealthCritical,
    PathBlocked,
    NewHostile,
    StrategicIntentChanged,
    TargetUnavailable,
    SceneChanged,
    PlayerDied,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_proposal_schema_contains_no_runtime_identity_or_revision_fields() {
        let schema = serde_json::to_string(&schemars::schema_for!(TacticalProposal))
            .expect("proposal schema");

        assert!(!schema.contains("agent_id"));
        assert!(!schema.contains("packet_id"));
        assert!(!schema.contains("decision_id"));
        assert!(!schema.contains("frame_revision"));
        assert!(!schema.contains("strategic_revision"));
        assert!(!schema.contains("created_at"));
    }

    #[test]
    fn runtime_enriches_a_proposal_with_causal_metadata() {
        let decision_id = Uuid::new_v4();
        let packet = ActionPacket::from_proposal(
            decision_id,
            18,
            4,
            Some("spider-nest".to_owned()),
            TacticalProposal {
                intent: TacticalIntent::Stop,
                actions: vec![TacticalAction::Stop],
                valid_for_ms: 500,
                abort_if: Vec::new(),
                rationale: None,
            },
        );

        assert_eq!(packet.decision_id, decision_id);
        assert_eq!(packet.frame_revision, 18);
        assert_eq!(packet.strategic_revision, 4);
        assert_eq!(packet.scene.as_deref(), Some("spider-nest"));
    }

    #[test]
    fn rejects_an_intent_that_only_describes_an_absent_action() {
        let proposal = TacticalProposal {
            intent: TacticalIntent::Reposition,
            actions: Vec::new(),
            valid_for_ms: 500,
            abort_if: Vec::new(),
            rationale: None,
        };

        assert_eq!(
            proposal.validate_semantics(),
            Err(TacticalProposalSemanticError::MissingMatchingAction)
        );
    }

    #[test]
    fn schema_and_semantics_reject_zero_lifetime() {
        let mut proposal = TacticalProposal {
            intent: TacticalIntent::Stop,
            actions: vec![TacticalAction::Stop],
            valid_for_ms: 0,
            abort_if: Vec::new(),
            rationale: None,
        };
        assert_eq!(
            proposal.validate_semantics(),
            Err(TacticalProposalSemanticError::InvalidLifetime)
        );
        let schema =
            serde_json::to_value(schemars::schema_for!(TacticalProposal)).expect("proposal schema");
        assert_eq!(schema["properties"]["valid_for_ms"]["minimum"], 100);
        assert_eq!(schema["properties"]["valid_for_ms"]["maximum"], 5_000);

        proposal.valid_for_ms = 1_000;
        assert_eq!(proposal.validate_semantics(), Ok(()));
    }

    #[test]
    fn attack_intent_accepts_an_offensive_skill_action() {
        let proposal = TacticalProposal {
            intent: TacticalIntent::Attack,
            actions: vec![TacticalAction::UseSkill {
                skill_id: "slash".to_owned(),
                target_id: Some("spider-92".to_owned()),
            }],
            valid_for_ms: 1_000,
            abort_if: Vec::new(),
            rationale: None,
        };

        assert_eq!(proposal.validate_semantics(), Ok(()));
    }

    #[test]
    fn strict_json_rejects_unknown_root_and_action_fields() {
        let root_error = serde_json::from_value::<TacticalProposal>(serde_json::json!({
            "intent": "continue",
            "actions": [],
            "valid_for_ms": 1000,
            "abort_if": [],
            "rationale": null,
            "invented_control": "move_anyway"
        }))
        .expect_err("unknown root field must fail");
        assert!(root_error.to_string().contains("unknown field"));

        let action_error = serde_json::from_value::<TacticalProposal>(serde_json::json!({
            "intent": "reposition",
            "actions": [{
                "type": "move_to",
                "tile_x": 4,
                "tile_y": 8,
                "agent_id": "somebody-else"
            }],
            "valid_for_ms": 1000,
            "abort_if": [],
            "rationale": null
        }))
        .expect_err("unknown action field must fail");
        assert!(action_error.to_string().contains("unknown field"));
    }
}
