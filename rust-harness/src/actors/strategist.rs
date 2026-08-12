use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::Utc;

use ractor::{Actor, ActorProcessingErr, ActorRef};

use crate::{
    brain::{
        Brain, BrainCallContext,
        strategic_input::{
            STRATEGIC_INPUT_PROTOCOL_VERSION, StrategicInput, StrategicMoment, StrategicMomentKind,
            StrategicNavigationArrival, StrategicWorldSnapshot,
        },
        strategic_output::{StrategicAction, StrategicProposal},
    },
    execution::movement::NavigationMissionRequest,
    memory::recall::{RecallLimits, RecallQuery},
    runtime::{
        blackboard::HotBlackboard,
        messages::{
            BodyMsg, MemoryMsg, StrategicInferenceResult, StrategicInteractionRequest,
            StrategicRecallResult, StrategicSpeechRequest, StrategistMsg, StrategistStatus,
            TacticianMsg, TelemetryEvent, TelemetryMsg,
        },
    },
    world::dialogue::{DialogueChannel, DialogueLine},
};

const MAX_PENDING_MOMENTS: usize = 32;
const FAILURE_BACKOFF_BASE: Duration = Duration::from_secs(5);
const FAILURE_BACKOFF_MAX: Duration = Duration::from_mins(1);
// A first local FastEmbed recall may initialize ONNX and populate the derived
// index. This work remains outside the actor handler and tactical path, so give
// it enough time to complete once rather than discarding a healthy cold start.
const MEMORY_RECALL_TIMEOUT_MS: u64 = 60_000;

pub struct StrategistActor;

pub struct StrategistActorArgs {
    pub blackboard: Arc<HotBlackboard>,
    pub tactician: ActorRef<TacticianMsg>,
    pub body: ActorRef<BodyMsg>,
    pub memory: ActorRef<MemoryMsg>,
    pub telemetry: ActorRef<TelemetryMsg>,
    pub minimum_interval: Duration,
}

pub struct StrategistActorState {
    args: StrategistActorArgs,
    brain: Option<Arc<dyn Brain<StrategicInput, StrategicProposal>>>,
    character_id: Option<String>,
    persona: Option<String>,
    pending_moments: VecDeque<StrategicMoment>,
    pending_navigation: VecDeque<crate::brain::strategic_intent::NavigationGoal>,
    input_revision: u64,
    active_recall: Option<ActiveRecall>,
    active_inference: Option<ActiveInference>,
    inferences_started: u64,
    inferences_coalesced: u64,
    inferences_failed: u64,
    consecutive_inference_failures: u32,
    last_successful_inference_at: Option<Instant>,
    next_inference_not_before: Option<Instant>,
    schedule_tick_armed: bool,
}

struct ActiveInference {
    decision_id: uuid::Uuid,
    input_revision: u64,
    base_strategic_revision: u64,
    started_at: Instant,
    moments: Vec<StrategicMoment>,
    input_scene: Option<String>,
    working: crate::memory::working::WorkingMemory,
    abort_handle: tokio::task::AbortHandle,
}

struct ActiveRecall {
    recall_id: uuid::Uuid,
    input_revision: u64,
    base_strategic_revision: u64,
    started_at: Instant,
    moments: Vec<StrategicMoment>,
    character_id: String,
    persona: String,
    world: StrategicWorldSnapshot,
    input_scene: Option<String>,
}

impl Actor for StrategistActor {
    type Msg = StrategistMsg;
    type State = StrategistActorState;
    type Arguments = StrategistActorArgs;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(StrategistActorState {
            args,
            brain: None,
            character_id: None,
            persona: None,
            pending_moments: VecDeque::new(),
            pending_navigation: VecDeque::new(),
            input_revision: 0,
            active_recall: None,
            active_inference: None,
            inferences_started: 0,
            inferences_coalesced: 0,
            inferences_failed: 0,
            consecutive_inference_failures: 0,
            last_successful_inference_at: None,
            next_inference_not_before: None,
            schedule_tick_armed: false,
        })
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            StrategistMsg::InstallBrain {
                character_id,
                persona,
                brain,
            } => {
                state.character_id = Some(character_id);
                state.persona = Some(persona);
                state.brain = Some(brain);
                start_if_ready(&myself, state);
            }
            StrategistMsg::WorldMoment(summary) => {
                wake(&myself, state, StrategicMomentKind::World, &summary);
            }
            StrategistMsg::GoalBlocked(summary) => {
                record_navigation_fact(
                    &state.args.memory,
                    "navigation",
                    "navigation blocked",
                    &summary,
                );
                wake(&myself, state, StrategicMomentKind::GoalBlocked, &summary);
            }
            StrategistMsg::NavigationArrived(arrival) => {
                record_arrival_fact(&state.args.memory, &arrival);
                wake_navigation_arrived(&myself, state, arrival);
            }
            StrategistMsg::PersonSpoke(line) => {
                wake_dialogue(&myself, state, &line);
            }
            StrategistMsg::EpisodeFinished(episode) => {
                wake(
                    &myself,
                    state,
                    StrategicMomentKind::EpisodeFinished,
                    &episode.summary,
                );
            }
            StrategistMsg::Reflect => {
                wake(
                    &myself,
                    state,
                    StrategicMomentKind::Reflection,
                    "Periodic strategic reflection requested.",
                );
            }
            StrategistMsg::ScheduleTick => {
                state.schedule_tick_armed = false;
                start_if_ready(&myself, state);
            }
            StrategistMsg::RecallCompleted(result) => finish_recall(&myself, state, result),
            StrategistMsg::InferenceCompleted(result) => {
                finish_inference(&myself, state, result);
            }
            StrategistMsg::ReplaceTactician(tactician) => state.args.tactician = tactician,
            StrategistMsg::Health(reply) => {
                if !reply.is_closed() {
                    reply.send(StrategistStatus {
                        latest_revision: state.args.blackboard.strategic_revision(),
                        queued_moments: state.pending_moments.len(),
                        inference_in_flight: state.active_inference.is_some()
                            || state.active_recall.is_some(),
                        input_revision: state.input_revision,
                        inferences_started: state.inferences_started,
                        inferences_coalesced: state.inferences_coalesced,
                        inferences_failed: state.inferences_failed,
                        consecutive_inference_failures: state.consecutive_inference_failures,
                        last_successful_inference_age_ms: last_successful_inference_age_ms(state),
                    })?;
                }
            }
            StrategistMsg::Shutdown => {
                if let Some(active) = state.active_inference.take() {
                    active.abort_handle.abort();
                    record(
                        state,
                        TelemetryEvent::StrategicInferenceSuperseded {
                            decision_id: active.decision_id,
                            input_revision: active.input_revision,
                            base_strategic_revision: active.base_strategic_revision,
                            duration_ms: duration_millis(active.started_at.elapsed()),
                            reason_code: "runtime_shutdown".to_owned(),
                        },
                    );
                }
                myself.stop(Some("player runtime shutdown".to_owned()));
            }
        }
        Ok(())
    }
}

fn wake(
    myself: &ActorRef<StrategistMsg>,
    state: &mut StrategistActorState,
    kind: StrategicMomentKind,
    summary: &str,
) {
    state.input_revision = state.input_revision.saturating_add(1);
    push_bounded(
        &mut state.pending_moments,
        StrategicMoment {
            kind,
            summary: summary.to_owned(),
            speaker: None,
            dialogue_channel: None,
            navigation_arrival: None,
        },
    );
    if let Some(active) = &state.active_inference {
        state.inferences_coalesced = state.inferences_coalesced.saturating_add(1);
        record(
            state,
            TelemetryEvent::StrategicInferenceCoalesced {
                decision_id: active.decision_id,
                active_input_revision: active.input_revision,
                base_strategic_revision: active.base_strategic_revision,
                pending_input_revision: state.input_revision,
                pending_moment_count: state.pending_moments.len(),
            },
        );
        return;
    }
    start_if_ready(myself, state);
}

fn wake_dialogue(
    myself: &ActorRef<StrategistMsg>,
    state: &mut StrategistActorState,
    line: &DialogueLine,
) {
    state.input_revision = state.input_revision.saturating_add(1);
    push_bounded(
        &mut state.pending_moments,
        StrategicMoment {
            kind: StrategicMomentKind::PersonSpoke,
            summary: line.message.clone(),
            speaker: Some(line.from.clone()),
            dialogue_channel: Some(dialogue_channel_name(line.channel).to_owned()),
            navigation_arrival: None,
        },
    );
    if let Some(active) = &state.active_inference {
        state.inferences_coalesced = state.inferences_coalesced.saturating_add(1);
        record(
            state,
            TelemetryEvent::StrategicInferenceCoalesced {
                decision_id: active.decision_id,
                active_input_revision: active.input_revision,
                base_strategic_revision: active.base_strategic_revision,
                pending_input_revision: state.input_revision,
                pending_moment_count: state.pending_moments.len(),
            },
        );
    } else {
        start_if_ready(myself, state);
    }
}

fn wake_navigation_arrived(
    myself: &ActorRef<StrategistMsg>,
    state: &mut StrategistActorState,
    arrival: crate::execution::movement::NavigationArrival,
) {
    let next = state.pending_navigation.pop_front();
    if let Some(next) = next {
        let frame_revision = state.args.blackboard.frame().revision;
        let strategic_revision = state.args.blackboard.strategic_revision();
        let destination_name = next.destination.as_ref().map_or_else(
            || next.scene.clone(),
            |destination| destination.name.clone(),
        );
        let _ = state
            .args
            .body
            .send_message(BodyMsg::PursueNavigation(NavigationMissionRequest {
                decision_id: uuid::Uuid::new_v4(),
                frame_revision,
                strategic_revision,
                destination_scene: next.scene,
                destination_tile: next.destination.and_then(|destination| destination.tile),
                destination_name,
                reason: next.reason,
                route: Vec::new(),
            }));
    } else {
        // A completed mission must not remain advertised as active strategic
        // navigation. Clearing it lets the next checkpoint choose a genuinely
        // new destination instead of repeatedly “investigating” the same tile.
        let current = state.args.blackboard.strategy();
        if current.navigation_goal.is_some() {
            let mut cleared = (*current).clone();
            cleared.revision = state.args.blackboard.strategic_revision().saturating_add(1);
            cleared.navigation_goal = None;
            let cleared = Arc::new(cleared);
            if state.args.blackboard.publish_strategy(cleared.clone()) {
                let _ = state
                    .args
                    .tactician
                    .send_message(TacticianMsg::StrategyUpdated(cleared.clone()));
                let _ = state
                    .args
                    .memory
                    .send_message(MemoryMsg::UpdateStrategicIntent((*cleared).clone()));
                record(
                    state,
                    TelemetryEvent::StrategyPublished {
                        decision_id: uuid::Uuid::new_v4(),
                        input_revision: state.input_revision,
                        revision: cleared.revision,
                        objective_chars: cleared.objective.chars().count(),
                        subgoal_count: cleared.subgoals.len(),
                        priority_count: cleared.priorities.len(),
                        constraint_count: cleared.constraints.len(),
                        preferred_target_count: cleared.preferred_targets.len(),
                        navigation_scene: None,
                        navigation_tile_known: false,
                    },
                );
            }
        }
    }
    state.input_revision = state.input_revision.saturating_add(1);
    push_bounded(
        &mut state.pending_moments,
        StrategicMoment {
            kind: StrategicMomentKind::NavigationArrived,
            summary: "The body reached its assigned navigation destination.".to_owned(),
            speaker: None,
            dialogue_channel: None,
            navigation_arrival: Some(StrategicNavigationArrival {
                destination_scene: arrival.destination_scene,
                destination_tile: arrival.destination_tile,
                destination_name: arrival.destination_name,
                arrived_scene: arrival.arrived_scene,
                arrived_tile: arrival.arrived_tile,
                attempts: arrival.attempts,
            }),
        },
    );
    if let Some(active) = &state.active_inference {
        state.inferences_coalesced = state.inferences_coalesced.saturating_add(1);
        record(
            state,
            TelemetryEvent::StrategicInferenceCoalesced {
                decision_id: active.decision_id,
                active_input_revision: active.input_revision,
                base_strategic_revision: active.base_strategic_revision,
                pending_input_revision: state.input_revision,
                pending_moment_count: state.pending_moments.len(),
            },
        );
    } else {
        start_if_ready(myself, state);
    }
}

fn record_arrival_fact(
    memory: &ActorRef<MemoryMsg>,
    arrival: &crate::execution::movement::NavigationArrival,
) {
    let destination = if arrival.destination_name.trim().is_empty() {
        "assigned destination"
    } else {
        arrival.destination_name.as_str()
    };
    let arrived_tile = arrival.arrived_tile.map_or_else(
        || "unknown tile".to_owned(),
        |tile| format!("({}, {})", tile.x, tile.y),
    );
    let arrived_scene = arrival.arrived_scene.as_deref().unwrap_or("unknown scene");
    let summary = format!(
        "Navigation completed: reached {destination} in scene {} at {arrived_tile} after {} attempt(s). Treat this destination as visited unless new evidence changes the goal.",
        arrived_scene, arrival.attempts
    );
    record_navigation_fact(memory, "navigation completed", arrived_scene, &summary);
}

fn record_navigation_fact(memory: &ActorRef<MemoryMsg>, kind: &str, scene: &str, summary: &str) {
    let now = Utc::now();
    let _ = memory.send_message(MemoryMsg::RecordEpisode(
        crate::world::episodes::EpisodeSummary {
            started_at: now,
            ended_at: now,
            scene: scene.to_owned(),
            summary: format!("{kind}: {summary}"),
            kills: 0,
            damage_dealt: 0,
            damage_received: 0,
            loot_collected: Default::default(),
        },
    ));
}

fn start_if_ready(myself: &ActorRef<StrategistMsg>, state: &mut StrategistActorState) {
    if state.active_recall.is_some()
        || state.active_inference.is_some()
        || state.pending_moments.is_empty()
        || state.brain.is_none()
    {
        return;
    }
    if let Some(not_before) = state.next_inference_not_before {
        let now = Instant::now();
        if not_before > now {
            if !state.schedule_tick_armed {
                let delay = not_before.saturating_duration_since(now);
                state.schedule_tick_armed = true;
                let schedule_id = uuid::Uuid::new_v4();
                record(
                    state,
                    TelemetryEvent::StrategicInferenceDeferred {
                        schedule_id,
                        input_revision: state.input_revision,
                        base_strategic_revision: state.args.blackboard.strategic_revision(),
                        pending_moment_count: state.pending_moments.len(),
                        eligible_after_ms: duration_millis(delay),
                    },
                );
                drop(myself.send_after(delay, || StrategistMsg::ScheduleTick));
            }
            return;
        }
        state.next_inference_not_before = None;
    }
    let (Some(character_id), Some(persona)) = (state.character_id.clone(), state.persona.clone())
    else {
        return;
    };

    let moments = state.pending_moments.drain(..).collect::<Vec<_>>();
    let input_revision = state.input_revision;
    let current_intent = state.args.blackboard.strategy();
    let base_strategic_revision = current_intent.revision;
    let decision_id = uuid::Uuid::new_v4();
    let world = StrategicWorldSnapshot::from(&*state.args.blackboard.frame());
    let reply_to = myself.clone();
    let recall_id = decision_id;
    let started_at = Instant::now();
    state.schedule_tick_armed = false;
    let query_text = recall_query_text(&current_intent, &moments, &world);
    record(
        state,
        TelemetryEvent::StrategicRecallStarted {
            recall_id,
            input_revision,
            base_strategic_revision,
            query_chars: query_text.chars().count(),
        },
    );
    let memory = state.args.memory.clone();
    let scene = world.scene.clone();
    let visible_people = world
        .visible_entities
        .iter()
        .filter(|entity| entity.hostile != Some(true))
        .map(|entity| format!("{} {}", entity.id, entity.label))
        .collect();
    tokio::spawn(async move {
        let query = RecallQuery {
            recall_id,
            text: query_text,
            scene,
            visible_people,
            limits: RecallLimits::default(),
        };
        let result =
            match ractor::call_t!(memory, MemoryMsg::Recall, MEMORY_RECALL_TIMEOUT_MS, query) {
                Ok(result) => result,
                Err(_) => Err("memory_recall_unavailable".to_owned()),
            };
        let _ = reply_to.send_message(StrategistMsg::RecallCompleted(StrategicRecallResult {
            recall_id,
            input_revision,
            base_strategic_revision,
            result,
        }));
    });
    state.active_recall = Some(ActiveRecall {
        recall_id,
        input_revision,
        base_strategic_revision,
        started_at,
        moments,
        character_id,
        persona,
        input_scene: world.scene.clone(),
        world,
    });
}

#[allow(
    clippy::too_many_lines,
    reason = "recall completion records the full causal transition into non-blocking inference"
)]
fn finish_recall(
    myself: &ActorRef<StrategistMsg>,
    state: &mut StrategistActorState,
    completion: StrategicRecallResult,
) {
    let Some(active) = state.active_recall.take() else {
        return;
    };
    let duration_ms = duration_millis(active.started_at.elapsed());
    if active.recall_id != completion.recall_id
        || active.base_strategic_revision != state.args.blackboard.strategic_revision()
        || active.input_scene != state.args.blackboard.frame().self_state.scene
    {
        restore_moments(state, active.moments);
        start_if_ready(myself, state);
        return;
    }
    let Ok(recall) = completion.result else {
        restore_moments(state, active.moments);
        record(
            state,
            TelemetryEvent::StrategicRecallFailed {
                recall_id: active.recall_id,
                input_revision: active.input_revision,
                base_strategic_revision: active.base_strategic_revision,
                duration_ms,
                error_class: "memory_actor".to_owned(),
            },
        );
        return;
    };
    record(
        state,
        TelemetryEvent::StrategicRecallCompleted {
            recall_id: active.recall_id,
            input_revision: active.input_revision,
            base_strategic_revision: active.base_strategic_revision,
            duration_ms,
            semantic_count: recall.semantic_memories.len(),
            relationship_count: recall.relationships.len(),
            episode_count: recall.episode_summaries.len(),
            plan_step_count: recall.working.plan.len(),
        },
    );
    let Some(brain) = state.brain.clone() else {
        return;
    };
    let working = recall.working.clone();
    let input = StrategicInput {
        protocol_version: STRATEGIC_INPUT_PROTOCOL_VERSION,
        character_id: active.character_id.clone(),
        persona: active.persona,
        current_intent: (*state.args.blackboard.strategy()).clone(),
        memory: recall,
        world: active.world.clone(),
        moments: active.moments.clone(),
    };
    let context = BrainCallContext {
        decision_id: active.recall_id,
        character_id: Some(active.character_id),
        frame_revision: Some(input.world.frame_revision),
        strategic_revision: Some(active.base_strategic_revision),
    };
    let reply_to = myself.clone();
    let decision_id = active.recall_id;
    state.inferences_started = state.inferences_started.saturating_add(1);
    record(
        state,
        TelemetryEvent::StrategicInferenceStarted {
            decision_id,
            input_revision: active.input_revision,
            base_strategic_revision: active.base_strategic_revision,
            moment_count: input.moments.len(),
            frame_revision: input.world.frame_revision,
            scene: input.world.scene.clone(),
            visible_entity_count: input.world.visible_entities.len(),
            visible_hostile_count: input
                .world
                .visible_entities
                .iter()
                .filter(|e| e.hostile == Some(true))
                .count(),
            exit_count: input.world.exits.len(),
            recent_scene_transition_count: input.world.recent_scene_transitions.len(),
            consecutive_failures_before_call: state.consecutive_inference_failures,
            last_successful_inference_age_ms: last_successful_inference_age_ms(state),
        },
    );
    let input_revision = active.input_revision;
    let base_strategic_revision = active.base_strategic_revision;
    let inference = tokio::spawn(async move {
        let result = brain
            .decide_with_context(&input, &context)
            .await
            .map_err(|error| error.to_string());
        let _ = reply_to.send_message(StrategistMsg::InferenceCompleted(
            StrategicInferenceResult {
                decision_id,
                input_revision,
                base_strategic_revision,
                result,
            },
        ));
    });
    state.active_inference = Some(ActiveInference {
        decision_id,
        input_revision,
        base_strategic_revision,
        started_at: Instant::now(),
        moments: active.moments,
        input_scene: active.input_scene,
        working,
        abort_handle: inference.abort_handle(),
    });
}

fn recall_query_text(
    intent: &crate::brain::strategic_intent::StrategicIntent,
    moments: &[StrategicMoment],
    world: &StrategicWorldSnapshot,
) -> String {
    let moment_text = moments
        .iter()
        .map(|moment| moment.summary.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let entities = world
        .visible_entities
        .iter()
        .map(|entity| entity.label.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "{} {} {} {}",
        intent.objective,
        intent.subgoals.join(" "),
        moment_text,
        entities
    )
}

#[allow(
    clippy::too_many_lines,
    reason = "inference completion atomically reconciles staleness, failure backoff, publication, and follow-up scheduling"
)]
fn finish_inference(
    myself: &ActorRef<StrategistMsg>,
    state: &mut StrategistActorState,
    completion: StrategicInferenceResult,
) {
    let StrategicInferenceResult {
        decision_id,
        input_revision,
        base_strategic_revision,
        result,
    } = completion;
    let Some(active) = state.active_inference.take() else {
        return;
    };
    if active.decision_id != decision_id {
        state.active_inference = Some(active);
        record(
            state,
            TelemetryEvent::StrategicInferenceFailed {
                decision_id,
                input_revision,
                base_strategic_revision,
                duration_ms: 0,
                error_class: "unexpected_completion".to_owned(),
                consecutive_failures: state.consecutive_inference_failures,
                retry_after_ms: 0,
                last_successful_inference_age_ms: last_successful_inference_age_ms(state),
                previous_intent_retained: true,
            },
        );
        return;
    }

    let duration_ms = duration_millis(active.started_at.elapsed());
    let current_scene = state.args.blackboard.frame().self_state.scene.clone();
    if active.input_scene != current_scene {
        restore_moments(state, active.moments);
        record(
            state,
            TelemetryEvent::StrategicInferenceSuperseded {
                decision_id,
                input_revision,
                base_strategic_revision,
                duration_ms,
                reason_code: "scene_changed".to_owned(),
            },
        );
        start_if_ready(myself, state);
        return;
    }
    if base_strategic_revision != state.args.blackboard.strategic_revision() {
        restore_moments(state, active.moments);
        record(
            state,
            TelemetryEvent::StrategicInferenceSuperseded {
                decision_id,
                input_revision,
                base_strategic_revision,
                duration_ms,
                reason_code: "newer_input".to_owned(),
            },
        );
        start_if_ready(myself, state);
        return;
    }

    let newer_input_pending = input_revision != state.input_revision;
    let (result, failure_class) = match result {
        Ok(proposal) => match proposal.validate_semantics() {
            Ok(()) => (Ok(proposal), "none"),
            Err(error) => (Err(error.to_string()), "proposal_semantics"),
        },
        Err(error) => (Err(error), "model_or_parse"),
    };
    if let Ok(mut proposal) = result {
        state.consecutive_inference_failures = 0;
        state.last_successful_inference_at = Some(Instant::now());
        let continue_thinking = proposal.continue_thinking;
        let speech_suppressed_as_stale = newer_input_pending && proposal.speech.take().is_some();
        let interaction_suppressed_as_stale = newer_input_pending
            && (proposal.interaction_target_id.take().is_some()
                || proposal
                    .actions
                    .iter()
                    .any(|action| matches!(action, StrategicAction::Interact { .. })));
        publish_proposal(
            state,
            decision_id,
            input_revision,
            base_strategic_revision,
            proposal,
            &active.moments,
            &active.working,
            duration_ms,
            newer_input_pending,
            speech_suppressed_as_stale,
            interaction_suppressed_as_stale,
        );
        state.next_inference_not_before = Some(Instant::now() + state.args.minimum_interval);
        if newer_input_pending {
            start_if_ready(myself, state);
        } else if continue_thinking {
            let delay = state.args.minimum_interval;
            drop(myself.send_after(delay, || StrategistMsg::Reflect));
        }
    } else {
        state.inferences_failed = state.inferences_failed.saturating_add(1);
        state.consecutive_inference_failures =
            state.consecutive_inference_failures.saturating_add(1);
        let retry_after = failure_backoff(state.consecutive_inference_failures);
        state.next_inference_not_before = Some(Instant::now() + retry_after);
        restore_moments(state, active.moments);
        record(
            state,
            TelemetryEvent::StrategicInferenceFailed {
                decision_id,
                input_revision,
                base_strategic_revision,
                duration_ms,
                error_class: failure_class.to_owned(),
                consecutive_failures: state.consecutive_inference_failures,
                retry_after_ms: duration_millis(retry_after),
                last_successful_inference_age_ms: last_successful_inference_age_ms(state),
                previous_intent_retained: true,
            },
        );
        start_if_ready(myself, state);
    }
}

fn failure_backoff(consecutive_failures: u32) -> Duration {
    let exponent = consecutive_failures.saturating_sub(1).min(4);
    FAILURE_BACKOFF_BASE
        .saturating_mul(1_u32 << exponent)
        .min(FAILURE_BACKOFF_MAX)
}

fn last_successful_inference_age_ms(state: &StrategistActorState) -> Option<u64> {
    state
        .last_successful_inference_at
        .map(|completed_at| duration_millis(completed_at.elapsed()))
}

#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "publication telemetry carries the complete strategic causal context"
)]
fn publish_proposal(
    state: &mut StrategistActorState,
    decision_id: uuid::Uuid,
    input_revision: u64,
    base_strategic_revision: u64,
    proposal: StrategicProposal,
    moments: &[StrategicMoment],
    working: &crate::memory::working::WorkingMemory,
    duration_ms: u64,
    newer_input_pending: bool,
    speech_suppressed_as_stale: bool,
    interaction_suppressed_as_stale: bool,
) {
    let speech = proposal.speech.clone();
    let speech_channel = proposal.speech_channel.clone();
    let strategic_thought = if proposal.progress_summary.trim().is_empty() {
        proposal.objective.clone()
    } else {
        proposal.progress_summary.clone()
    };
    // `actions` is the preferred strict-JSON operation channel. The legacy
    // field remains accepted so older strategist transcripts can still act.
    let interaction_target_id = proposal
        .actions
        .iter()
        .map(|action| match action {
            StrategicAction::Interact { target_id } => target_id.clone(),
            StrategicAction::QueueDuel => String::new(),
        })
        .next()
        .filter(|target| !target.is_empty())
        .or_else(|| proposal.interaction_target_id.clone());
    let queue_duel = proposal
        .actions
        .iter()
        .any(|action| matches!(action, StrategicAction::QueueDuel));
    let navigation_goal = proposal.navigation_goal.clone();
    state.pending_navigation = proposal.navigation_queue.iter().cloned().skip(1).collect();
    let current = state.args.blackboard.strategy();
    let navigation_changed = navigation_goal != current.navigation_goal;
    let working_update = proposal.working_update(working);
    let plan_changed = working_update.plan_revision != working.plan_revision;
    let retained_step_count = working_update
        .plan
        .iter()
        .filter(|step| working.plan.iter().any(|old| old.what == step.what))
        .count();
    let published_revision = if proposal.materially_differs_from(&current) {
        let revision = state.args.blackboard.strategic_revision().saturating_add(1);
        let intent = Arc::new(proposal.into_intent(revision));
        if state.args.blackboard.publish_strategy(intent.clone()) {
            let _ = state
                .args
                .tactician
                .send_message(TacticianMsg::StrategyUpdated(intent.clone()));
            let _ = state
                .args
                .memory
                .send_message(MemoryMsg::ApplyStrategicPlan {
                    update: working_update.clone(),
                    intent: (*intent).clone(),
                });
            record(
                state,
                TelemetryEvent::StrategyPublished {
                    decision_id,
                    input_revision,
                    revision,
                    objective_chars: intent.objective.chars().count(),
                    subgoal_count: intent.subgoals.len(),
                    priority_count: intent.priorities.len(),
                    constraint_count: intent.constraints.len(),
                    preferred_target_count: intent.preferred_targets.len(),
                    navigation_scene: intent
                        .navigation_goal
                        .as_ref()
                        .map(|goal| goal.scene.clone()),
                    navigation_tile_known: intent
                        .navigation_goal
                        .as_ref()
                        .and_then(|goal| goal.destination.as_ref())
                        .and_then(|destination| destination.tile)
                        .is_some(),
                },
            );
            Some(revision)
        } else {
            None
        }
    } else if plan_changed {
        let _ = state
            .args
            .memory
            .send_message(MemoryMsg::ApplyStrategicPlan {
                update: working_update.clone(),
                intent: (*current).clone(),
            });
        None
    } else {
        None
    };
    if plan_changed {
        record(
            state,
            TelemetryEvent::StrategicPlanChanged {
                decision_id,
                plan_revision: working_update.plan_revision,
                step_count: working_update.plan.len(),
                retained_step_count,
                blocked: working_update.blocked_reason.is_some(),
                completion_claimed: working_update.goal_completion_claimed,
            },
        );
    }
    let has_body_action = navigation_changed && published_revision.is_some()
        || speech.is_some()
        || interaction_target_id.is_some()
        || queue_duel;
    if has_body_action {
        // Keep the spectator-facing action lineage compatible with the legacy
        // harness: a concise strategic thought is recorded before the body
        // receives the corresponding operation. This is an assessment, not
        // private chain-of-thought.
        let _ = state.args.body.send_message(BodyMsg::Think(
            crate::runtime::messages::StrategicThoughtRequest {
                decision_id,
                frame_revision: state.args.blackboard.frame().revision,
                strategic_revision: published_revision.unwrap_or(base_strategic_revision),
                thought: strategic_thought,
            },
        ));
    }
    if navigation_changed
        && published_revision.is_some()
        && let Some(goal) = navigation_goal.as_ref()
    {
        let destination_name = goal.destination.as_ref().map_or_else(
            || goal.scene.clone(),
            |destination| destination.name.clone(),
        );
        let _ = state
            .args
            .body
            .send_message(BodyMsg::PursueNavigation(NavigationMissionRequest {
                decision_id,
                frame_revision: state.args.blackboard.frame().revision,
                strategic_revision: published_revision.unwrap_or(base_strategic_revision),
                destination_scene: goal.scene.clone(),
                destination_tile: goal
                    .destination
                    .as_ref()
                    .and_then(|destination| destination.tile),
                destination_name,
                reason: goal.reason.clone(),
                route: Vec::new(),
            }));
    }
    if let Some(request) = speech_request(
        decision_id,
        state.args.blackboard.frame().revision,
        published_revision.unwrap_or(base_strategic_revision),
        speech,
        speech_channel.as_deref(),
        moments,
    ) {
        let _ = state.args.body.send_message(BodyMsg::Speak(request));
    }
    if let Some(target_id) = interaction_target_id {
        let _ = state
            .args
            .body
            .send_message(BodyMsg::Interact(StrategicInteractionRequest {
                decision_id,
                frame_revision: state.args.blackboard.frame().revision,
                strategic_revision: published_revision.unwrap_or(base_strategic_revision),
                target_id,
            }));
    }
    if queue_duel {
        let _ = state.args.body.send_message(BodyMsg::QueueDuel(
            crate::runtime::messages::StrategicDuelRequest {
                decision_id,
                frame_revision: state.args.blackboard.frame().revision,
                strategic_revision: published_revision.unwrap_or(base_strategic_revision),
            },
        ));
    }
    record(
        state,
        TelemetryEvent::StrategicInferenceCompleted {
            decision_id,
            input_revision,
            base_strategic_revision,
            published_revision,
            duration_ms,
            newer_input_pending,
            speech_suppressed_as_stale,
            interaction_suppressed_as_stale,
        },
    );
}

fn speech_request(
    decision_id: uuid::Uuid,
    frame_revision: u64,
    strategic_revision: u64,
    speech: Option<String>,
    requested_channel: Option<&str>,
    moments: &[StrategicMoment],
) -> Option<StrategicSpeechRequest> {
    let message = speech?.trim().to_owned();
    if message.is_empty() {
        return None;
    }
    let moment = moments
        .iter()
        .rev()
        .find(|moment| moment.kind == StrategicMomentKind::PersonSpoke);
    let channel_name = requested_channel
        .or_else(|| moment.and_then(|moment| moment.dialogue_channel.as_deref()))
        .unwrap_or("scene");
    let channel = match channel_name {
        "scene" => crate::execution::gateway::BodySpeechChannel::Scene,
        "global" => crate::execution::gateway::BodySpeechChannel::Global,
        "private" => crate::execution::gateway::BodySpeechChannel::Private,
        _ => return None,
    };
    let to_player = (channel == crate::execution::gateway::BodySpeechChannel::Private)
        .then(|| moment.and_then(|moment| moment.speaker.clone()))
        .flatten();
    if channel == crate::execution::gateway::BodySpeechChannel::Private && to_player.is_none() {
        return None;
    }
    Some(StrategicSpeechRequest {
        decision_id,
        frame_revision,
        strategic_revision,
        message,
        channel,
        to_player,
    })
}

const fn dialogue_channel_name(channel: DialogueChannel) -> &'static str {
    match channel {
        DialogueChannel::Scene => "scene",
        DialogueChannel::Global => "global",
        DialogueChannel::Private => "private",
        DialogueChannel::Team => "team",
        DialogueChannel::Unknown => "unknown",
    }
}

fn restore_moments(state: &mut StrategistActorState, moments: Vec<StrategicMoment>) {
    let pending = state.pending_moments.drain(..).collect::<Vec<_>>();
    for moment in moments.into_iter().chain(pending) {
        push_bounded(&mut state.pending_moments, moment);
    }
}

fn push_bounded(moments: &mut VecDeque<StrategicMoment>, moment: StrategicMoment) {
    // Perception can publish a world update every cycle. Keep the newest world
    // snapshot, but retain dialogue, failures, arrivals, and episode facts in
    // order so the strategist does not spend its entire budget on stale frames.
    if moment.kind == StrategicMomentKind::World {
        moments.retain(|existing| existing.kind != StrategicMomentKind::World);
    }
    if moments.len() == MAX_PENDING_MOMENTS {
        moments.pop_front();
    }
    moments.push_back(moment);
}

fn duration_millis(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn record(state: &StrategistActorState, event: TelemetryEvent) {
    let _ = state
        .args
        .telemetry
        .send_message(TelemetryMsg::Record(event));
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use ractor::Actor;
    use tokio::sync::{Semaphore, mpsc};

    use super::*;
    use crate::{
        actors::telemetry::{TelemetryActor, TelemetryActorArgs},
        brain::{strategic_intent::StrategicIntent, strategic_output::StrategicProposal},
        observability::RecordingAnalyticsSink,
        runtime::messages::TacticianStatus,
    };

    struct TacticianProbe;

    impl Actor for TacticianProbe {
        type Msg = TacticianMsg;
        type State = mpsc::UnboundedSender<Arc<crate::brain::strategic_intent::StrategicIntent>>;
        type Arguments = Self::State;

        async fn pre_start(
            &self,
            _myself: ActorRef<Self::Msg>,
            state: Self::Arguments,
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(state)
        }

        async fn handle(
            &self,
            myself: ActorRef<Self::Msg>,
            message: Self::Msg,
            state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            match message {
                TacticianMsg::StrategyUpdated(intent) => {
                    let _ = state.send(intent);
                }
                TacticianMsg::Health(reply) if !reply.is_closed() => {
                    reply.send(TacticianStatus {
                        inference_in_flight: false,
                        latest_frame_revision: 0,
                        latest_strategic_revision: 0,
                        decisions_started: 0,
                        stale_decisions_discarded: 0,
                    })?;
                }
                TacticianMsg::Shutdown => myself.stop(None),
                _ => {}
            }
            Ok(())
        }
    }

    struct BodyProbe;

    impl Actor for BodyProbe {
        type Msg = BodyMsg;
        type State = ();
        type Arguments = ();

        async fn pre_start(
            &self,
            _myself: ActorRef<Self::Msg>,
            (): Self::Arguments,
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }

        async fn handle(
            &self,
            myself: ActorRef<Self::Msg>,
            message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            if matches!(message, BodyMsg::Shutdown) {
                myself.stop(None);
            }
            Ok(())
        }
    }

    struct MemoryProbe;

    impl Actor for MemoryProbe {
        type Msg = MemoryMsg;
        type State = ();
        type Arguments = ();

        async fn pre_start(
            &self,
            _myself: ActorRef<Self::Msg>,
            (): Self::Arguments,
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }

        async fn handle(
            &self,
            myself: ActorRef<Self::Msg>,
            message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            match message {
                MemoryMsg::Recall(_, reply) if !reply.is_closed() => {
                    reply.send(Ok(crate::memory::recall::StrategicRecall::default()))?;
                }
                MemoryMsg::Shutdown => myself.stop(None),
                _ => {}
            }
            Ok(())
        }
    }

    struct ControlledBrain {
        calls: Mutex<Vec<StrategicInput>>,
        permits: Arc<Semaphore>,
        proposal: StrategicProposal,
    }

    #[async_trait]
    impl Brain<StrategicInput, StrategicProposal> for ControlledBrain {
        async fn decide(&self, input: &StrategicInput) -> anyhow::Result<StrategicProposal> {
            self.calls.lock().expect("calls lock").push(input.clone());
            self.permits.acquire().await?.forget();
            Ok(self.proposal.clone())
        }
    }

    #[derive(Default)]
    struct FailingBrain {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl Brain<StrategicInput, StrategicProposal> for FailingBrain {
        async fn decide(&self, _input: &StrategicInput) -> anyhow::Result<StrategicProposal> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            anyhow::bail!("provider unavailable: sensitive detail")
        }
    }

    struct Fixture {
        strategist: ActorRef<StrategistMsg>,
        blackboard: Arc<HotBlackboard>,
        analytics: Arc<RecordingAnalyticsSink>,
        updates: mpsc::UnboundedReceiver<Arc<crate::brain::strategic_intent::StrategicIntent>>,
    }

    async fn fixture() -> Fixture {
        fixture_with_interval(Duration::ZERO).await
    }

    async fn fixture_with_interval(minimum_interval: Duration) -> Fixture {
        let initial = StrategicIntent {
            revision: 7,
            objective: "Keep watch.".to_owned(),
            ..StrategicIntent::default()
        };
        let blackboard = Arc::new(HotBlackboard::new(initial));
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let (telemetry, _) = Actor::spawn(
            None,
            TelemetryActor,
            TelemetryActorArgs {
                character_id: "guy".to_owned(),
                sink: analytics.clone(),
            },
        )
        .await
        .expect("telemetry starts");
        let (updates_tx, updates) = mpsc::unbounded_channel();
        let (tactician, _) = Actor::spawn(None, TacticianProbe, updates_tx)
            .await
            .expect("tactician probe starts");
        let (body, _) = Actor::spawn(None, BodyProbe, ())
            .await
            .expect("body probe starts");
        let (memory, _) = Actor::spawn(None, MemoryProbe, ())
            .await
            .expect("memory probe starts");
        let (strategist, _) = Actor::spawn(
            None,
            StrategistActor,
            StrategistActorArgs {
                blackboard: blackboard.clone(),
                tactician,
                body,
                memory,
                telemetry,
                minimum_interval,
            },
        )
        .await
        .expect("strategist starts");
        Fixture {
            strategist,
            blackboard,
            analytics,
            updates,
        }
    }

    #[tokio::test]
    async fn pending_moments_wait_for_the_configured_minimum_interval() {
        let fixture = fixture_with_interval(Duration::from_millis(80)).await;
        let permits = Arc::new(Semaphore::new(0));
        let brain = Arc::new(ControlledBrain {
            calls: Mutex::new(Vec::new()),
            permits: permits.clone(),
            proposal: StrategicProposal::from(&*fixture.blackboard.strategy()),
        });
        fixture
            .strategist
            .send_message(StrategistMsg::InstallBrain {
                character_id: "generic-agent".to_owned(),
                persona: "A test persona.".to_owned(),
                brain: brain.clone(),
            })
            .expect("install brain");
        fixture
            .strategist
            .send_message(StrategistMsg::Reflect)
            .expect("first wake");
        wait_until(|| brain.calls.lock().expect("calls").len() == 1).await;
        fixture
            .strategist
            .send_message(StrategistMsg::WorldMoment("A later fact.".to_owned()))
            .expect("pending wake");

        permits.add_permits(1);
        wait_until(|| {
            fixture
                .analytics
                .events()
                .iter()
                .any(|event| event.name == "strategic.inference_deferred")
        })
        .await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(brain.calls.lock().expect("calls").len(), 1);

        wait_until(|| brain.calls.lock().expect("calls").len() == 2).await;
        let deferred = fixture
            .analytics
            .events()
            .into_iter()
            .find(|event| event.name == "strategic.inference_deferred")
            .expect("deferred event");
        assert!(deferred.correlation_id.is_some());
        assert_eq!(deferred.attributes["pending_moment_count"], 1);
        assert!(
            deferred.attributes["eligible_after_ms"]
                .as_u64()
                .unwrap_or(0)
                > 0
        );
        permits.add_permits(1);
    }

    #[tokio::test]
    async fn navigation_arrival_is_a_typed_complete_strategic_moment() {
        let fixture = fixture().await;
        let permits = Arc::new(Semaphore::new(0));
        let brain = Arc::new(ControlledBrain {
            calls: Mutex::new(Vec::new()),
            permits: permits.clone(),
            proposal: StrategicProposal::from(&*fixture.blackboard.strategy()),
        });
        fixture
            .strategist
            .send_message(StrategistMsg::InstallBrain {
                character_id: "generic-agent".to_owned(),
                persona: "A test persona.".to_owned(),
                brain: brain.clone(),
            })
            .expect("install brain");
        let destination_name = "the complete name of a destination ".repeat(30);
        fixture
            .strategist
            .send_message(StrategistMsg::NavigationArrived(
                crate::execution::movement::NavigationArrival {
                    mission_id: uuid::Uuid::new_v4(),
                    decision_id: uuid::Uuid::new_v4(),
                    strategic_revision: 7,
                    destination_scene: "town".to_owned(),
                    destination_tile: Some(crate::world::TilePosition { x: 17, y: 13 }),
                    destination_name: destination_name.clone(),
                    arrived_scene: Some("town".to_owned()),
                    arrived_tile: Some(crate::world::TilePosition { x: 17, y: 13 }),
                    attempts: 3,
                },
            ))
            .expect("arrival sent");

        wait_until(|| brain.calls.lock().expect("calls").len() == 1).await;
        let calls = brain.calls.lock().expect("calls");
        let moment = &calls[0].moments[0];
        assert_eq!(moment.kind, StrategicMomentKind::NavigationArrived);
        let arrival = moment.navigation_arrival.as_ref().expect("typed arrival");
        assert_eq!(arrival.destination_name, destination_name);
        assert_eq!(arrival.attempts, 3);
        drop(calls);
        permits.add_permits(1);
    }

    async fn status(actor: &ActorRef<StrategistMsg>) -> StrategistStatus {
        ractor::call_t!(actor, StrategistMsg::Health, 250).expect("health RPC succeeds")
    }

    async fn wait_until(mut condition: impl FnMut() -> bool) {
        for _ in 0..100 {
            if condition() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("condition did not become true");
    }

    #[tokio::test]
    async fn inference_does_not_block_mailbox_and_new_wakes_coalesce() {
        let mut fixture = fixture().await;
        let permits = Arc::new(Semaphore::new(0));
        let mut proposal = StrategicProposal::from(&*fixture.blackboard.strategy());
        proposal.objective = "Find paid work in town.".to_owned();
        proposal.speech = Some("I will answer the old moment.".to_owned());
        let brain = Arc::new(ControlledBrain {
            calls: Mutex::new(Vec::new()),
            permits: permits.clone(),
            proposal,
        });
        fixture
            .strategist
            .send_message(StrategistMsg::InstallBrain {
                character_id: "guy".to_owned(),
                persona: "Gruff but loyal.".to_owned(),
                brain: brain.clone(),
            })
            .expect("install brain");
        fixture
            .strategist
            .send_message(StrategistMsg::WorldMoment(
                "A job board appeared.".to_owned(),
            ))
            .expect("first wake");
        wait_until(|| brain.calls.lock().expect("calls").len() == 1).await;

        fixture
            .strategist
            .send_message(StrategistMsg::GoalBlocked("The road is shut.".to_owned()))
            .expect("coalesced wake");
        fixture
            .strategist
            .send_message(StrategistMsg::PersonSpoke(DialogueLine {
                channel: DialogueChannel::Scene,
                kind: crate::world::dialogue::DialogueKind::Speech,
                backend_message_type: Some(1),
                from: "Barnaby".to_owned(),
                message: "Barnaby offered work.".to_owned(),
                received_at: None,
            }))
            .expect("coalesced wake");
        let while_blocked = status(&fixture.strategist).await;
        assert!(while_blocked.inference_in_flight);
        assert_eq!(while_blocked.input_revision, 3);
        assert_eq!(while_blocked.inferences_coalesced, 2);

        permits.add_permits(1);
        wait_until(|| brain.calls.lock().expect("calls").len() == 2).await;
        wait_until(|| fixture.blackboard.strategic_revision() == 8).await;
        let published = fixture.updates.recv().await.expect("tactician update");
        assert_eq!(published.revision, 8);
        assert_eq!(published.objective, "Find paid work in town.");
        let second_input = brain.calls.lock().expect("calls")[1].clone();
        assert_eq!(second_input.current_intent.revision, 8);
        assert_eq!(second_input.moments.len(), 2);

        permits.add_permits(1);
        wait_until(|| {
            fixture
                .analytics
                .events()
                .iter()
                .filter(|event| event.name == "strategic.inference_completed")
                .count()
                == 2
        })
        .await;
        let finished = status(&fixture.strategist).await;
        assert!(!finished.inference_in_flight);
        assert_eq!(finished.inferences_started, 2);

        wait_until(|| {
            fixture
                .analytics
                .events()
                .iter()
                .any(|event| event.name == "strategic.inference_completed")
        })
        .await;
        assert!(fixture.analytics.events().iter().any(|event| {
            event.name == "strategic.inference_completed"
                && event.attributes["newer_input_pending"] == true
                && event.attributes["speech_suppressed_as_stale"] == true
        }));
        for event in fixture.analytics.events().into_iter().filter(|event| {
            event.name.starts_with("strategic.") || event.name == "strategy.published"
        }) {
            assert!(event.correlation_id.is_some());
            assert!(event.attributes.contains_key("decision_id"));
            assert!(!event.attributes.contains_key("prompt"));
            assert!(!event.attributes.contains_key("model_output"));
        }
    }

    #[tokio::test]
    async fn model_failure_is_isolated_and_not_retried_in_a_loop() {
        let fixture = fixture().await;
        let brain = Arc::new(FailingBrain::default());
        fixture
            .strategist
            .send_message(StrategistMsg::InstallBrain {
                character_id: "guy".to_owned(),
                persona: "Gruff but loyal.".to_owned(),
                brain: brain.clone(),
            })
            .expect("install brain");
        fixture
            .strategist
            .send_message(StrategistMsg::Reflect)
            .expect("wake");

        wait_until(|| brain.calls.load(Ordering::SeqCst) == 1).await;
        wait_until(|| {
            fixture
                .analytics
                .events()
                .iter()
                .any(|event| event.name == "strategic.inference_failed")
        })
        .await;
        let failed = status(&fixture.strategist).await;
        assert!(!failed.inference_in_flight);
        assert_eq!(failed.inferences_failed, 1);
        assert_eq!(failed.queued_moments, 1);
        assert_eq!(fixture.blackboard.strategic_revision(), 7);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert_eq!(brain.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn shutdown_terminalizes_an_in_flight_strategic_decision() {
        let fixture = fixture().await;
        let permits = Arc::new(Semaphore::new(0));
        let brain = Arc::new(ControlledBrain {
            calls: Mutex::new(Vec::new()),
            permits: permits.clone(),
            proposal: StrategicProposal::from(&*fixture.blackboard.strategy()),
        });
        fixture
            .strategist
            .send_message(StrategistMsg::InstallBrain {
                character_id: "guy".to_owned(),
                persona: "Gruff but loyal.".to_owned(),
                brain: brain.clone(),
            })
            .expect("install brain");
        fixture
            .strategist
            .send_message(StrategistMsg::Reflect)
            .expect("wake");
        wait_until(|| brain.calls.lock().expect("calls").len() == 1).await;

        fixture
            .strategist
            .send_message(StrategistMsg::Shutdown)
            .expect("request shutdown");
        wait_until(|| {
            fixture.analytics.events().iter().any(|event| {
                event.name == "strategic.inference_superseded"
                    && event.attributes["reason_code"] == "runtime_shutdown"
            })
        })
        .await;
        permits.add_permits(1);
    }
}
