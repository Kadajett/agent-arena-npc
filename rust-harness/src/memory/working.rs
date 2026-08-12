use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::brain::strategic_intent::StrategicIntent;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkingMemory {
    pub goal: Option<Goal>,
    pub plan: Vec<PlanStep>,
    /// Runtime-owned revision of the durable plan, independent of world state.
    #[serde(default)]
    pub plan_revision: u64,
    /// Short factual assessment of progress against the current goal.
    #[serde(default)]
    pub progress_summary: String,
    /// Conditions that should wake the strategist before its periodic reflection.
    #[serde(default)]
    pub reevaluate_when: Vec<String>,
    /// Why the plan as a whole cannot currently advance, when known.
    #[serde(default)]
    pub blocked_reason: Option<String>,
    /// Whether supplied evidence satisfies the goal's completion condition.
    #[serde(default)]
    pub goal_complete: bool,
    pub todo: Vec<TodoItem>,
    pub notes: Vec<WorkingNote>,
    /// Last accepted long-horizon direction. Older stores omit this field.
    #[serde(default)]
    pub strategic_intent: Option<StrategicIntent>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_working_memory_without_strategy_still_deserializes() {
        let memory: WorkingMemory = serde_json::from_value(serde_json::json!({
            "goal": null,
            "plan": [],
            "todo": [],
            "notes": []
        }))
        .expect("legacy working memory");

        assert!(memory.strategic_intent.is_none());
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Goal {
    pub aim: String,
    pub done: Option<String>,
    pub why: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlanStep {
    /// Stable runtime identity. Migrated legacy steps receive one when next reconciled.
    #[serde(default)]
    pub step_id: Option<uuid::Uuid>,
    pub what: String,
    pub status: WorkStatus,
    pub note: Option<String>,
    pub tries: u32,
    /// Observable evidence that makes this step complete.
    #[serde(default)]
    pub done_when: Option<String>,
    /// Facts observed while attempting the step. This must not contain hidden reasoning.
    #[serde(default)]
    pub evidence: Vec<String>,
    /// Conditions that should cause this step to be reconsidered.
    #[serde(default)]
    pub reevaluate_when: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TodoItem {
    pub what: String,
    pub status: WorkStatus,
    pub note: Option<String>,
    pub asked_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkingNote {
    pub text: String,
    pub recorded_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkStatus {
    Next,
    Doing,
    Done,
    Blocked,
}

/// Runtime-observed progress for one durable plan step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanProgressUpdate {
    pub correlation_id: uuid::Uuid,
    pub expected_plan_revision: u64,
    pub step_id: uuid::Uuid,
    pub transition: PlanStepTransition,
    pub evidence: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanStepTransition {
    Started,
    Completed,
    Blocked,
}
