use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::brain::strategic_intent::{NavigationGoal, Priority, StrategicIntent};
use crate::memory::working::{Goal, PlanStep, WorkStatus, WorkingMemory};

/// Model-authored strategic content.
///
/// Revisions are intentionally absent. The actor converts an accepted proposal
/// into [`StrategicIntent`] and assigns the next runtime-owned revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StrategicProposal {
    /// Agentic strategist hint to continue its self-directed checkpoint loop.
    /// The actor owns the actual scheduling and may still coalesce or reject it.
    #[serde(default)]
    pub continue_thinking: bool,
    /// Durable end condition and motivation. This is not a tactical instruction.
    pub goal: Goal,
    /// Ordered multi-step plan. The runtime owns the plan revision.
    #[schemars(length(max = 12))]
    pub plan: Vec<ProposedPlanStep>,
    /// Factual progress assessment; do not include private chain-of-thought.
    #[schemars(length(max = 500))]
    pub progress_summary: String,
    /// Material events that should cause this plan to be reconsidered.
    #[schemars(length(max = 12))]
    pub reevaluate_when: Vec<String>,
    /// Why no plan step can currently advance, if that is true.
    pub blocked_reason: Option<String>,
    /// True only when supplied evidence satisfies `goal.done`.
    pub goal_completion_claimed: bool,
    pub objective: String,
    pub subgoals: Vec<String>,
    pub priorities: Vec<Priority>,
    pub constraints: Vec<String>,
    pub risk_tolerance: f32,
    pub preferred_targets: Vec<String>,
    pub avoid: Vec<String>,
    pub navigation_goal: Option<NavigationGoal>,
    /// Ordered destinations the body may pursue without another model call.
    #[serde(default)]
    #[schemars(length(max = 5))]
    pub navigation_queue: Vec<NavigationGoal>,
    /// Immediate strategic operation on a visible interactable object.
    #[schemars(length(max = 4))]
    pub actions: Vec<StrategicAction>,
    /// An immediate reply to the newest person-spoke moment.
    ///
    /// The runtime selects the channel and private recipient from perception.
    #[schemars(length(max = 140))]
    pub speech: Option<String>,
    /// Requested channel for proactive speech; runtime validates the channel.
    #[serde(default)]
    pub speech_channel: Option<String>,
    /// Exact visible NPC entity ID to open dialogue with now.
    pub interaction_target_id: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum StrategicAction {
    Interact { target_id: String },
    QueueDuel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum StrategicProposalSemanticError {
    #[error("goal aim and objective must not be blank")]
    BlankObjective,
    #[error("plan may contain at most 12 steps")]
    TooManyPlanSteps,
    #[error("plan step descriptions must not be blank")]
    BlankPlanStep,
    #[error("reevaluate_when may contain at most 12 conditions")]
    TooManyReevaluationConditions,
    #[error("risk_tolerance must be finite and between zero and one")]
    InvalidRiskTolerance,
    #[error("speech may contain at most 140 characters")]
    SpeechTooLong,
    #[error("interaction_target_id must not be blank")]
    BlankInteractionTarget,
    #[error("strategic action target must not be blank")]
    BlankActionTarget,
}

impl StrategicProposal {
    /// Validate invariants that JSON Schema describes but Serde does not enforce.
    ///
    /// # Errors
    /// Returns a stable error when structurally valid model JSON violates the
    /// strategic protocol's semantic bounds.
    pub fn validate_semantics(&self) -> Result<(), StrategicProposalSemanticError> {
        if self.goal.aim.trim().is_empty() || self.objective.trim().is_empty() {
            return Err(StrategicProposalSemanticError::BlankObjective);
        }
        if self.plan.len() > 12 {
            return Err(StrategicProposalSemanticError::TooManyPlanSteps);
        }
        if self.plan.iter().any(|step| step.what.trim().is_empty()) {
            return Err(StrategicProposalSemanticError::BlankPlanStep);
        }
        if self.reevaluate_when.len() > 12 {
            return Err(StrategicProposalSemanticError::TooManyReevaluationConditions);
        }
        if !self.risk_tolerance.is_finite() || !(0.0..=1.0).contains(&self.risk_tolerance) {
            return Err(StrategicProposalSemanticError::InvalidRiskTolerance);
        }
        if self
            .speech
            .as_ref()
            .is_some_and(|speech| speech.chars().count() > 140)
        {
            return Err(StrategicProposalSemanticError::SpeechTooLong);
        }
        if self
            .interaction_target_id
            .as_ref()
            .is_some_and(|target| target.trim().is_empty())
        {
            return Err(StrategicProposalSemanticError::BlankInteractionTarget);
        }
        if self.actions.iter().any(|action| match action {
            StrategicAction::Interact { target_id } => target_id.trim().is_empty(),
            StrategicAction::QueueDuel => false,
        }) {
            return Err(StrategicProposalSemanticError::BlankActionTarget);
        }
        Ok(())
    }

    #[must_use]
    pub fn into_intent(self, revision: u64) -> StrategicIntent {
        StrategicIntent {
            revision,
            objective: self.objective,
            subgoals: self.subgoals,
            priorities: self.priorities,
            constraints: self.constraints,
            risk_tolerance: self.risk_tolerance,
            preferred_targets: self.preferred_targets,
            avoid: self.avoid,
            navigation_goal: self.navigation_goal,
            expires_at: self.expires_at,
        }
    }

    /// Convert accepted model content into an atomic durable working-state update.
    #[must_use]
    pub fn working_update(&self, current: &WorkingMemory) -> StrategicWorkingUpdate {
        let plan = self
            .plan
            .iter()
            .map(|proposed| {
                current
                    .plan
                    .iter()
                    .find(|step| step.what == proposed.what)
                    .map_or_else(
                        || PlanStep {
                            step_id: Some(uuid::Uuid::new_v4()),
                            what: proposed.what.clone(),
                            status: WorkStatus::Next,
                            note: proposed.note.clone(),
                            tries: 0,
                            done_when: proposed.done_when.clone(),
                            evidence: Vec::new(),
                            reevaluate_when: proposed.reevaluate_when.clone(),
                        },
                        |existing| PlanStep {
                            step_id: existing.step_id.or_else(|| Some(uuid::Uuid::new_v4())),
                            what: existing.what.clone(),
                            status: existing.status,
                            note: proposed.note.clone().or_else(|| existing.note.clone()),
                            tries: existing.tries,
                            done_when: proposed
                                .done_when
                                .clone()
                                .or_else(|| existing.done_when.clone()),
                            evidence: existing.evidence.clone(),
                            reevaluate_when: proposed.reevaluate_when.clone(),
                        },
                    )
            })
            .collect();
        let materially_changed = current.goal.as_ref() != Some(&self.goal)
            || current.plan != plan
            || current.progress_summary != self.progress_summary
            || current.reevaluate_when != self.reevaluate_when
            || current.blocked_reason != self.blocked_reason;
        StrategicWorkingUpdate {
            goal: self.goal.clone(),
            plan,
            plan_revision: if materially_changed {
                current.plan_revision.saturating_add(1)
            } else {
                current.plan_revision
            },
            progress_summary: self.progress_summary.clone(),
            reevaluate_when: self.reevaluate_when.clone(),
            blocked_reason: self.blocked_reason.clone(),
            // Completion is a model claim until a deterministic reducer records
            // evidence and advances the durable state.
            goal_completion_claimed: self.goal_completion_claimed,
        }
    }

    #[must_use]
    pub fn materially_differs_from(&self, current: &StrategicIntent) -> bool {
        self.objective != current.objective
            || self.subgoals != current.subgoals
            || self.priorities != current.priorities
            || self.constraints != current.constraints
            || self.risk_tolerance.to_bits() != current.risk_tolerance.to_bits()
            || self.preferred_targets != current.preferred_targets
            || self.avoid != current.avoid
            || self.navigation_goal != current.navigation_goal
            || self.expires_at != current.expires_at
    }
}

impl From<&StrategicIntent> for StrategicProposal {
    fn from(intent: &StrategicIntent) -> Self {
        Self {
            continue_thinking: false,
            goal: Goal {
                aim: intent.objective.clone(),
                done: None,
                why: None,
            },
            plan: Vec::new(),
            progress_summary: String::new(),
            reevaluate_when: Vec::new(),
            blocked_reason: None,
            goal_completion_claimed: false,
            objective: intent.objective.clone(),
            subgoals: intent.subgoals.clone(),
            priorities: intent.priorities.clone(),
            constraints: intent.constraints.clone(),
            risk_tolerance: intent.risk_tolerance,
            preferred_targets: intent.preferred_targets.clone(),
            avoid: intent.avoid.clone(),
            navigation_goal: intent.navigation_goal.clone(),
            navigation_queue: Vec::new(),
            actions: Vec::new(),
            speech: None,
            speech_channel: None,
            interaction_target_id: None,
            expires_at: intent.expires_at,
        }
    }
}

/// Model-authored planning content after the runtime assigns a plan revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrategicWorkingUpdate {
    pub goal: Goal,
    pub plan: Vec<PlanStep>,
    pub plan_revision: u64,
    pub progress_summary: String,
    pub reevaluate_when: Vec<String>,
    pub blocked_reason: Option<String>,
    pub goal_completion_claimed: bool,
}

/// Model-authored content for one durable plan step.
///
/// Status, attempt counts, IDs, and evidence are deliberately absent. They are
/// execution facts owned by the runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProposedPlanStep {
    pub what: String,
    pub done_when: Option<String>,
    pub note: Option<String>,
    pub reevaluate_when: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::working::{PlanStep, WorkStatus, WorkingMemory};

    #[test]
    fn model_schema_has_no_runtime_revision() {
        let schema = serde_json::to_value(schemars::schema_for!(StrategicProposal))
            .expect("schema serializes");

        assert!(schema.pointer("/properties/revision").is_none());
        assert!(
            schema["properties"]["speech"]
                .to_string()
                .contains(r#""maxLength":140"#)
        );
    }

    #[test]
    fn actor_can_stamp_a_proposal_without_changing_content() {
        let current = StrategicIntent::default();
        let proposal = StrategicProposal::from(&current);

        assert!(!proposal.materially_differs_from(&current));
        assert_eq!(proposal.into_intent(19).revision, 19);
    }

    #[test]
    fn model_plan_cannot_erase_attempts_or_fabricate_evidence_and_completion() {
        let current = WorkingMemory {
            goal: Some(Goal {
                aim: "Find the bell".to_owned(),
                done: Some("The bell is held".to_owned()),
                why: None,
            }),
            plan: vec![PlanStep {
                step_id: Some(uuid::Uuid::new_v4()),
                what: "Search the harbor".to_owned(),
                status: WorkStatus::Doing,
                note: Some("Started east".to_owned()),
                tries: 4,
                done_when: Some("The bell is visible".to_owned()),
                evidence: vec!["The west pier was empty".to_owned()],
                reevaluate_when: vec!["A sailor gives a lead".to_owned()],
            }],
            plan_revision: 8,
            ..WorkingMemory::default()
        };
        let mut proposal = StrategicProposal::from(&StrategicIntent::default());
        proposal.goal = current.goal.clone().expect("goal");
        proposal.plan = vec![ProposedPlanStep {
            what: "Search the harbor".to_owned(),
            done_when: Some("The bell is visible".to_owned()),
            note: None,
            reevaluate_when: Vec::new(),
        }];
        proposal.goal_completion_claimed = true;

        let update = proposal.working_update(&current);

        assert_eq!(update.plan[0].tries, 4);
        assert_eq!(update.plan[0].step_id, current.plan[0].step_id);
        assert_eq!(update.plan[0].status, WorkStatus::Doing);
        assert_eq!(update.plan[0].evidence, ["The west pier was empty"]);
        assert!(update.goal_completion_claimed);
        assert!(!current.goal_complete);
        let schema = serde_json::to_string(&schemars::schema_for!(StrategicProposal))
            .expect("schema serializes");
        assert!(!schema.contains(r#"\"tries\""#));
        assert!(!schema.contains(r#"\"evidence\""#));
        assert!(!schema.contains(r#"\"status\""#));
    }

    #[test]
    fn strict_json_and_semantics_reject_unowned_or_invalid_content() {
        let unknown = serde_json::from_value::<StrategicProposal>(serde_json::json!({
            "goal": {"aim": "Explore", "done": null, "why": null},
            "plan": [{
                "what": "Walk north",
                "done_when": null,
                "note": null,
                "reevaluate_when": [],
                "status": "done"
            }],
            "progress_summary": "Not started",
            "reevaluate_when": [],
            "blocked_reason": null,
            "goal_completion_claimed": false,
            "objective": "Explore",
            "subgoals": [],
            "priorities": ["objective"],
            "constraints": [],
            "risk_tolerance": 0.5,
            "preferred_targets": [],
            "avoid": [],
            "navigation_goal": null,
            "actions": [],
            "speech": null,
            "interaction_target_id": null,
            "expires_at": null
        }))
        .expect_err("model-owned plan status must fail strict parsing");
        assert!(unknown.to_string().contains("unknown field"));

        let mut proposal = StrategicProposal::from(&StrategicIntent::default());
        proposal.risk_tolerance = 1.5;
        assert_eq!(
            proposal.validate_semantics(),
            Err(StrategicProposalSemanticError::InvalidRiskTolerance)
        );
    }
}
