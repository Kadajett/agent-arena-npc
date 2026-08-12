use std::sync::Arc;

use ractor::{Actor, ActorProcessingErr, ActorRef};

use crate::{
    memory::{
        store::MemoryStore,
        working::{PlanStepTransition, WorkingMemory},
    },
    runtime::messages::{MemoryMsg, MemoryStatus, TelemetryEvent, TelemetryMsg},
};

pub struct MemoryActor;

pub struct MemoryActorArgs {
    pub character_id: String,
    pub initial_working: WorkingMemory,
    pub store: Arc<dyn MemoryStore>,
    pub telemetry: ActorRef<TelemetryMsg>,
}

pub struct MemoryActorState {
    character_id: String,
    working: WorkingMemory,
    store: Arc<dyn MemoryStore>,
    episodes_recorded: u64,
    relationships_recorded: u64,
    writes_failed: u64,
    telemetry: ActorRef<TelemetryMsg>,
}

impl Actor for MemoryActor {
    type Msg = MemoryMsg;
    type State = MemoryActorState;
    type Arguments = MemoryActorArgs;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        let mut working = args.store.load_working(&args.character_id).await?;
        if working == WorkingMemory::default() && args.initial_working != WorkingMemory::default() {
            args.store
                .save_working(&args.character_id, &args.initial_working)
                .await?;
            working = args.initial_working;
        }
        Ok(MemoryActorState {
            character_id: args.character_id,
            working,
            store: args.store,
            episodes_recorded: 0,
            relationships_recorded: 0,
            writes_failed: 0,
            telemetry: args.telemetry,
        })
    }

    #[allow(
        clippy::too_many_lines,
        reason = "each typed memory message is handled at the actor's single mutation boundary"
    )]
    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            MemoryMsg::RecordRelationship(update) => {
                if state
                    .store
                    .apply_relationship(&state.character_id, &update)
                    .await
                    .is_ok()
                {
                    state.relationships_recorded += 1;
                } else {
                    state.writes_failed += 1;
                }
            }
            MemoryMsg::RecordEpisode(episode) => {
                if state
                    .store
                    .record_episode(&state.character_id, &episode)
                    .await
                    .is_ok()
                {
                    state.episodes_recorded += 1;
                } else {
                    state.writes_failed += 1;
                }
            }
            MemoryMsg::UpdateStrategicIntent(intent) => {
                let mut working = state.working.clone();
                working.strategic_intent = Some(intent);
                if state
                    .store
                    .save_working(&state.character_id, &working)
                    .await
                    .is_ok()
                {
                    state.working = working;
                } else {
                    state.writes_failed += 1;
                }
            }
            MemoryMsg::ApplyStrategicPlan { update, intent } => {
                let mut working = state.working.clone();
                let goal_changed = working.goal.as_ref() != Some(&update.goal);
                working.goal = Some(update.goal);
                working.plan = update.plan;
                working.plan_revision = update.plan_revision;
                working.progress_summary = update.progress_summary;
                working.reevaluate_when = update.reevaluate_when;
                working.blocked_reason = update.blocked_reason;
                // A model may claim completion, but only runtime-observed evidence
                // may set the durable completion fact. A new goal starts incomplete.
                if goal_changed {
                    working.goal_complete = false;
                }
                working.strategic_intent = Some(intent);
                if state
                    .store
                    .save_working(&state.character_id, &working)
                    .await
                    .is_ok()
                {
                    state.working = working;
                } else {
                    state.writes_failed += 1;
                }
            }
            MemoryMsg::Recall(query, reply) => {
                let result = state
                    .store
                    .recall(&state.character_id, &query)
                    .await
                    .map_err(|_| "memory_recall_failed".to_owned());
                if !reply.is_closed() {
                    reply.send(result)?;
                }
            }
            MemoryMsg::AdvancePlanStep(update) => {
                if update.expected_plan_revision != state.working.plan_revision {
                    return Ok(());
                }
                let mut working = state.working.clone();
                let Some(step) = working
                    .plan
                    .iter_mut()
                    .find(|step| step.step_id == Some(update.step_id))
                else {
                    return Ok(());
                };
                let transition = match update.transition {
                    PlanStepTransition::Started => {
                        step.status = crate::memory::working::WorkStatus::Doing;
                        step.tries = step.tries.saturating_add(1);
                        "started"
                    }
                    PlanStepTransition::Completed => {
                        step.status = crate::memory::working::WorkStatus::Done;
                        "completed"
                    }
                    PlanStepTransition::Blocked => {
                        step.status = crate::memory::working::WorkStatus::Blocked;
                        "blocked"
                    }
                };
                if !update.evidence.trim().is_empty() {
                    if step.evidence.len() == 16 {
                        step.evidence.remove(0);
                    }
                    step.evidence.push(update.evidence);
                }
                let tries = step.tries;
                let evidence_count = step.evidence.len();
                if state
                    .store
                    .save_working(&state.character_id, &working)
                    .await
                    .is_ok()
                {
                    state.working = working;
                    let _ = state.telemetry.send_message(TelemetryMsg::Record(
                        TelemetryEvent::StrategicPlanStepAdvanced {
                            correlation_id: update.correlation_id,
                            plan_revision: update.expected_plan_revision,
                            transition: transition.to_owned(),
                            tries,
                            evidence_count,
                        },
                    ));
                } else {
                    state.writes_failed += 1;
                }
            }
            MemoryMsg::ReplaceWorking(working) => {
                if state
                    .store
                    .save_working(&state.character_id, &working)
                    .await
                    .is_ok()
                {
                    state.working = working;
                } else {
                    state.writes_failed += 1;
                }
            }
            MemoryMsg::ReadWorking(reply) => {
                if !reply.is_closed() {
                    reply.send(state.working.clone())?;
                }
            }
            MemoryMsg::Health(reply) => {
                if !reply.is_closed() {
                    reply.send(MemoryStatus {
                        episodes_recorded: state.episodes_recorded,
                        relationships_recorded: state.relationships_recorded,
                        writes_failed: state.writes_failed,
                    })?;
                }
            }
            MemoryMsg::Shutdown => myself.stop(Some("player runtime shutdown".to_owned())),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        actors::telemetry::{TelemetryActor, TelemetryActorArgs},
        memory::{
            sqlite_store::SqliteMemoryStore,
            working::{Goal, PlanStep, WorkStatus},
        },
        observability::RecordingAnalyticsSink,
    };

    #[tokio::test]
    async fn working_state_persists_and_reloads_after_actor_restart() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("memory.sqlite3");
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let (telemetry, _) = Actor::spawn(
            None,
            TelemetryActor,
            TelemetryActorArgs {
                character_id: "cassian".to_owned(),
                sink: analytics.clone(),
            },
        )
        .await
        .expect("telemetry starts");
        let first_store = Arc::new(
            SqliteMemoryStore::open(&path, analytics.clone())
                .await
                .expect("open first store"),
        );
        let (first, first_join) = Actor::spawn(
            None,
            MemoryActor,
            MemoryActorArgs {
                character_id: "cassian".to_owned(),
                initial_working: WorkingMemory::default(),
                store: first_store,
                telemetry: telemetry.clone(),
            },
        )
        .await
        .expect("spawn first memory actor");
        let expected = WorkingMemory {
            goal: Some(Goal {
                aim: "Recover a legendary song".to_owned(),
                done: Some("The score is safe".to_owned()),
                why: Some("Art demands it".to_owned()),
            }),
            plan: vec![PlanStep {
                step_id: Some(uuid::Uuid::new_v4()),
                what: "Question the inn patrons".to_owned(),
                status: WorkStatus::Doing,
                note: None,
                tries: 1,
                done_when: None,
                evidence: Vec::new(),
                reevaluate_when: Vec::new(),
            }],
            strategic_intent: Some(crate::brain::strategic_intent::StrategicIntent {
                revision: 3,
                objective: "Recover the legendary song without getting trapped.".to_owned(),
                ..crate::brain::strategic_intent::StrategicIntent::default()
            }),
            ..WorkingMemory::default()
        };
        first
            .send_message(MemoryMsg::ReplaceWorking(expected.clone()))
            .expect("replace working state");
        assert_eq!(
            ractor::call_t!(first, MemoryMsg::ReadWorking, 1_000).expect("read first actor state"),
            expected
        );
        first
            .send_message(MemoryMsg::Shutdown)
            .expect("stop first actor");
        first_join.await.expect("join first actor");

        let second_store = Arc::new(
            SqliteMemoryStore::open(&path, analytics)
                .await
                .expect("reopen store"),
        );
        let (second, second_join) = Actor::spawn(
            None,
            MemoryActor,
            MemoryActorArgs {
                character_id: "cassian".to_owned(),
                initial_working: WorkingMemory::default(),
                store: second_store,
                telemetry,
            },
        )
        .await
        .expect("spawn second memory actor");
        assert_eq!(
            ractor::call_t!(second, MemoryMsg::ReadWorking, 1_000)
                .expect("read reloaded actor state"),
            expected
        );
        let status = ractor::call_t!(second, MemoryMsg::Health, 1_000).expect("read memory health");
        assert_eq!(status.writes_failed, 0);
        second
            .send_message(MemoryMsg::Shutdown)
            .expect("stop second actor");
        second_join.await.expect("join second actor");
    }

    #[tokio::test]
    async fn runtime_plan_progress_preserves_evidence_but_telemetry_redacts_it() {
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let store = Arc::new(
            SqliteMemoryStore::open_in_memory(analytics.clone())
                .await
                .expect("open store"),
        );
        let (telemetry, _) = Actor::spawn(
            None,
            TelemetryActor,
            TelemetryActorArgs {
                character_id: "cassian".to_owned(),
                sink: analytics.clone(),
            },
        )
        .await
        .expect("telemetry starts");
        let step_id = uuid::Uuid::new_v4();
        let initial = WorkingMemory {
            plan: vec![PlanStep {
                step_id: Some(step_id),
                what: "Reach the north gate".to_owned(),
                status: WorkStatus::Next,
                note: None,
                tries: 2,
                done_when: Some("The north road is visible".to_owned()),
                evidence: Vec::new(),
                reevaluate_when: Vec::new(),
            }],
            plan_revision: 4,
            ..WorkingMemory::default()
        };
        let (memory, _) = Actor::spawn(
            None,
            MemoryActor,
            MemoryActorArgs {
                character_id: "cassian".to_owned(),
                initial_working: initial,
                store,
                telemetry,
            },
        )
        .await
        .expect("memory starts");
        memory
            .send_message(MemoryMsg::AdvancePlanStep(
                crate::memory::working::PlanProgressUpdate {
                    correlation_id: uuid::Uuid::new_v4(),
                    expected_plan_revision: 4,
                    step_id,
                    transition: crate::memory::working::PlanStepTransition::Started,
                    evidence: "private exact route detail".to_owned(),
                },
            ))
            .expect("advance step");
        let working =
            ractor::call_t!(memory, MemoryMsg::ReadWorking, 1_000).expect("read progressed memory");

        assert_eq!(working.plan[0].status, WorkStatus::Doing);
        assert_eq!(working.plan[0].tries, 3);
        assert_eq!(working.plan[0].evidence, ["private exact route detail"]);
        let events = analytics.events();
        let event = events
            .iter()
            .find(|event| event.name == "strategic.plan_step_advanced")
            .expect("plan advancement telemetry");
        assert_eq!(event.attributes["tries"], 3);
        assert!(
            !serde_json::to_string(event)
                .expect("event json")
                .contains("private exact")
        );
    }
}
