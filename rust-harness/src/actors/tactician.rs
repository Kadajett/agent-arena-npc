use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use ractor::{Actor, ActorProcessingErr, ActorRef};

use crate::{
    brain::{
        Brain, BrainCallContext, strategic_intent::StrategicIntent, tactical_frame::TacticalFrame,
        tactical_input::TacticalInput, tactical_output::TacticalProposal,
    },
    execution::packet::ActionPacket,
    runtime::{
        blackboard::HotBlackboard,
        messages::{
            BodyMsg, PlayerSupervisorMsg, TacticalDecisionResult, TacticianMsg, TacticianStatus,
            TelemetryEvent, TelemetryMsg,
        },
        tactical_schedule::{
            InferencePermit, InferenceResultDisposition, PacketRelease, TacticalActivity,
            TacticalRolloutMode, TacticalScheduleConfig, TacticalScheduleDecision,
            TacticalScheduleEffect, TacticalScheduleFacts, TacticalScheduler, TacticalSnapshot,
            TacticalWake, TacticalWakeReason,
        },
    },
};

pub struct TacticianActor;

pub struct TacticianActorArgs {
    pub character_id: String,
    pub blackboard: Arc<HotBlackboard>,
    pub body: ActorRef<BodyMsg>,
    pub packet_sink: ActorRef<PlayerSupervisorMsg>,
    pub telemetry: ActorRef<TelemetryMsg>,
    pub brain: Arc<dyn Brain<TacticalInput, TacticalProposal>>,
    pub schedule_config: TacticalScheduleConfig,
    pub rollout_mode: TacticalRolloutMode,
}

pub struct TacticianActorState {
    args: TacticianActorArgs,
    latest_frame: Arc<TacticalFrame>,
    latest_strategy: Arc<StrategicIntent>,
    scheduler: TacticalScheduler,
    epoch: Instant,
    active_inference: Option<ActiveInference>,
    pending_signal_id: Option<uuid::Uuid>,
    decisions_started: u64,
    stale_decisions_discarded: u64,
}

#[derive(Debug)]
struct ActiveInference {
    scheduler_inference_id: u64,
    decision_id: uuid::Uuid,
    frame_revision: u64,
    strategic_revision: u64,
    started_at: Instant,
    abort_handle: tokio::task::AbortHandle,
}

impl Actor for TacticianActor {
    type Msg = TacticianMsg;
    type State = TacticianActorState;
    type Arguments = TacticianActorArgs;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        let scheduler = TacticalScheduler::new(args.schedule_config.clone(), args.rollout_mode);
        Ok(TacticianActorState {
            latest_frame: args.blackboard.frame(),
            latest_strategy: args.blackboard.strategy(),
            args,
            scheduler,
            epoch: Instant::now(),
            active_inference: None,
            pending_signal_id: None,
            decisions_started: 0,
            stale_decisions_discarded: 0,
        })
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            TacticianMsg::FrameUpdated(frame) => {
                let prior = state.latest_frame.clone();
                state.latest_frame = frame;
                if state.latest_frame.revision != prior.revision {
                    let wake = TacticalWake {
                        snapshot: snapshot(state),
                        activity: activity(&state.latest_frame),
                        reason: frame_wake_reason(&prior, &state.latest_frame),
                    };
                    request_schedule(&myself, state, wake);
                }
            }
            TacticianMsg::StrategyUpdated(strategy) => {
                state.latest_strategy = strategy;
                request_schedule(
                    &myself,
                    state,
                    TacticalWake {
                        snapshot: snapshot(state),
                        activity: activity(&state.latest_frame),
                        reason: TacticalWakeReason::StrategyChanged,
                    },
                );
            }
            TacticianMsg::ForceDecision(_trigger) => {
                request_schedule(
                    &myself,
                    state,
                    TacticalWake {
                        snapshot: snapshot(state),
                        activity: activity(&state.latest_frame),
                        reason: TacticalWakeReason::Forced,
                    },
                );
            }
            TacticianMsg::ScheduleTick => {
                poll_schedule(&myself, state);
            }
            TacticianMsg::DecisionCompleted(decision) => {
                finish_decision(&myself, state, decision);
            }
            TacticianMsg::ReplaceBody(body) => state.args.body = body,
            TacticianMsg::Health(reply) => {
                if !reply.is_closed() {
                    reply.send(TacticianStatus {
                        inference_in_flight: state.scheduler.in_flight().is_some(),
                        latest_frame_revision: state.latest_frame.revision,
                        latest_strategic_revision: state.latest_strategy.revision,
                        decisions_started: state.decisions_started,
                        stale_decisions_discarded: state.stale_decisions_discarded,
                    })?;
                }
            }
            #[cfg(test)]
            TacticianMsg::FailForTest => {
                return Err(std::io::Error::other("injected tactician failure").into());
            }
            TacticianMsg::Shutdown => {
                if let Some(active) = state.active_inference.take() {
                    active.abort_handle.abort();
                    record(
                        state,
                        TelemetryEvent::TacticalDecisionSuperseded {
                            decision_id: active.decision_id,
                            frame_revision: active.frame_revision,
                            strategic_revision: active.strategic_revision,
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

fn request_schedule(
    myself: &ActorRef<TacticianMsg>,
    state: &mut TacticianActorState,
    wake: TacticalWake,
) {
    let signal_id = uuid::Uuid::new_v4();
    record(
        state,
        TelemetryEvent::TacticalWakeRequested {
            signal_id,
            frame_revision: wake.snapshot.frame_revision,
            strategic_revision: wake.snapshot.strategic_revision,
            reason: wake.reason,
            activity: wake.activity,
        },
    );
    let facts_before = state.scheduler.facts();
    let decision = state.scheduler.request(now(state), wake);
    apply_schedule(
        myself,
        state,
        signal_id,
        wake.snapshot,
        facts_before,
        decision,
        true,
    );
    arm_next_tick(myself, state);
}

fn poll_schedule(myself: &ActorRef<TacticianMsg>, state: &mut TacticianActorState) {
    let signal_id = state.pending_signal_id.unwrap_or_else(uuid::Uuid::new_v4);
    let facts_before = state.scheduler.facts();
    let decision = state.scheduler.poll(now(state));
    let heartbeat_generated =
        decision.facts.heartbeats_generated > facts_before.heartbeats_generated;
    if heartbeat_generated {
        let snapshot = snapshot(state);
        record(
            state,
            TelemetryEvent::TacticalHeartbeatGenerated {
                signal_id,
                frame_revision: snapshot.frame_revision,
                strategic_revision: snapshot.strategic_revision,
                activity: state.scheduler.activity(),
            },
        );
        record(
            state,
            TelemetryEvent::TacticalWakeRequested {
                signal_id,
                frame_revision: snapshot.frame_revision,
                strategic_revision: snapshot.strategic_revision,
                reason: TacticalWakeReason::Heartbeat,
                activity: state.scheduler.activity(),
            },
        );
    }
    apply_schedule(
        myself,
        state,
        signal_id,
        snapshot(state),
        facts_before,
        decision,
        heartbeat_generated,
    );
    arm_next_tick(myself, state);
}

#[allow(
    clippy::too_many_arguments,
    reason = "schedule telemetry needs the complete causal context for one policy evaluation"
)]
fn apply_schedule(
    myself: &ActorRef<TacticianMsg>,
    state: &mut TacticianActorState,
    signal_id: uuid::Uuid,
    signal_snapshot: TacticalSnapshot,
    facts_before: TacticalScheduleFacts,
    decision: TacticalScheduleDecision,
    track_as_pending_signal: bool,
) {
    let coalesced = decision.facts.wakes_coalesced > facts_before.wakes_coalesced;
    if coalesced {
        let (pending, reason_count) = pending_facts(&decision.effect, signal_snapshot);
        record(
            state,
            TelemetryEvent::TacticalWakeCoalesced {
                signal_id,
                frame_revision: signal_snapshot.frame_revision,
                strategic_revision: signal_snapshot.strategic_revision,
                pending_frame_revision: pending.frame_revision,
                pending_strategic_revision: pending.strategic_revision,
                coalesced_reason_count: reason_count,
            },
        );
    }

    match decision.effect {
        TacticalScheduleEffect::Start(permit) => {
            let trigger_signal_id = state.pending_signal_id.take().unwrap_or(signal_id);
            start_decision(myself, state, trigger_signal_id, &permit);
        }
        TacticalScheduleEffect::Deferred {
            reason,
            eligible_at,
            pending_snapshot,
            coalesced_reasons,
        } => {
            if track_as_pending_signal {
                state.pending_signal_id = Some(signal_id);
            }
            record(
                state,
                TelemetryEvent::TacticalWakeDeferred {
                    signal_id,
                    frame_revision: pending_snapshot.frame_revision,
                    strategic_revision: pending_snapshot.strategic_revision,
                    reason,
                    eligible_after_ms: eligible_at
                        .map(|due| duration_millis(due.saturating_sub(now(state)))),
                    coalesced_reason_count: coalesced_reasons.len(),
                },
            );
        }
        TacticalScheduleEffect::Suppressed(reason) => record(
            state,
            TelemetryEvent::TacticalWakeSuppressed {
                signal_id,
                frame_revision: signal_snapshot.frame_revision,
                strategic_revision: signal_snapshot.strategic_revision,
                reason,
            },
        ),
    }
}

fn pending_facts(
    effect: &TacticalScheduleEffect,
    fallback: TacticalSnapshot,
) -> (TacticalSnapshot, usize) {
    match effect {
        TacticalScheduleEffect::Start(permit) => (permit.snapshot, permit.reasons.len()),
        TacticalScheduleEffect::Deferred {
            pending_snapshot,
            coalesced_reasons,
            ..
        } => (*pending_snapshot, coalesced_reasons.len()),
        TacticalScheduleEffect::Suppressed(_) => (fallback, 0),
    }
}

fn start_decision(
    myself: &ActorRef<TacticianMsg>,
    state: &mut TacticianActorState,
    trigger_signal_id: uuid::Uuid,
    permit: &InferencePermit,
) {
    if permit.snapshot != snapshot(state) {
        return;
    }
    let mut frame = (*state.latest_frame).clone();
    frame.strategic_intent = (*state.latest_strategy).clone();
    let input = TacticalInput::from(&frame);
    let frame_revision = input.frame_revision;
    let strategic_revision = input.strategic_revision;
    let decision_id = uuid::Uuid::new_v4();
    let brain = state.args.brain.clone();
    let brain_context = BrainCallContext {
        decision_id,
        character_id: Some(state.args.character_id.clone()),
        frame_revision: Some(frame_revision),
        strategic_revision: Some(strategic_revision),
    };
    let reply_to = myself.clone();

    let started_at = Instant::now();
    state.decisions_started = state.decisions_started.saturating_add(1);
    record(
        state,
        TelemetryEvent::TacticalDecisionStarted {
            trigger_signal_id,
            decision_id,
            scheduler_inference_id: permit.inference_id,
            frame_revision,
            strategic_revision,
            wake_reasons: permit.reasons.iter().copied().collect(),
        },
    );
    let inference = tokio::spawn(async move {
        let result = brain
            .decide_with_context(&input, &brain_context)
            .await
            .map_err(|error| error.to_string());
        let _ = reply_to.send_message(TacticianMsg::DecisionCompleted(TacticalDecisionResult {
            decision_id,
            frame_revision,
            strategic_revision,
            result,
        }));
    });
    state.active_inference = Some(ActiveInference {
        scheduler_inference_id: permit.inference_id,
        decision_id,
        frame_revision,
        strategic_revision,
        started_at,
        abort_handle: inference.abort_handle(),
    });
}

fn finish_decision(
    myself: &ActorRef<TacticianMsg>,
    state: &mut TacticianActorState,
    decision: TacticalDecisionResult,
) {
    let Some(active) = state.active_inference.take() else {
        return;
    };
    if active.decision_id != decision.decision_id {
        state.active_inference = Some(active);
        record(
            state,
            TelemetryEvent::TacticalDecisionFailed {
                decision_id: decision.decision_id,
                frame_revision: decision.frame_revision,
                strategic_revision: decision.strategic_revision,
                duration_ms: 0,
                error_class: "unexpected_completion".to_owned(),
            },
        );
        return;
    }
    let duration_ms = duration_millis(active.started_at.elapsed());
    let facts_before = state.scheduler.facts();
    let completion = state
        .scheduler
        .complete(now(state), active.scheduler_inference_id);
    let outdated = completion.disposition != InferenceResultDisposition::Current
        || decision.strategic_revision != state.latest_strategy.revision;
    if outdated {
        state.stale_decisions_discarded = state.stale_decisions_discarded.saturating_add(1);
        record(
            state,
            TelemetryEvent::TacticalDecisionSuperseded {
                decision_id: decision.decision_id,
                frame_revision: decision.frame_revision,
                strategic_revision: decision.strategic_revision,
                duration_ms,
                reason_code: if completion.disposition == InferenceResultDisposition::Superseded {
                    "scheduler_superseded"
                } else {
                    "revision_changed"
                }
                .to_owned(),
            },
        );
    } else {
        record_current_decision(state, decision, duration_ms);
    }
    apply_completion_follow_up(myself, state, facts_before, completion.follow_up);
}

fn record_current_decision(
    state: &TacticianActorState,
    decision: TacticalDecisionResult,
    duration_ms: u64,
) {
    let Ok(proposal) = decision.result else {
        record(
            state,
            TelemetryEvent::TacticalDecisionFailed {
                decision_id: decision.decision_id,
                frame_revision: decision.frame_revision,
                strategic_revision: decision.strategic_revision,
                duration_ms,
                error_class: "model_or_parse".to_owned(),
            },
        );
        return;
    };
    if proposal.validate_semantics().is_err() {
        record(
            state,
            TelemetryEvent::TacticalDecisionFailed {
                decision_id: decision.decision_id,
                frame_revision: decision.frame_revision,
                strategic_revision: decision.strategic_revision,
                duration_ms,
                error_class: "proposal_semantics".to_owned(),
            },
        );
        return;
    }

    let release = state.scheduler.rollout_mode().packet_release();
    let action_count = proposal.actions.len();
    let action_plan = proposal
        .actions
        .iter()
        .take(8)
        .map(action_fact)
        .collect::<Vec<_>>()
        .join("|");
    let intent = proposal.intent;
    record(
        state,
        TelemetryEvent::TacticalDecisionCompleted {
            decision_id: decision.decision_id,
            frame_revision: decision.frame_revision,
            strategic_revision: decision.strategic_revision,
            action_count,
            action_plan,
            intent,
            duration_ms,
        },
    );
    let packet = ActionPacket::from_proposal(
        decision.decision_id,
        decision.frame_revision,
        decision.strategic_revision,
        state.latest_frame.self_state.scene.clone(),
        proposal,
    );
    let packet_id = packet.id;
    let (released, reason_code) = release_packet(state, release, packet);
    record(
        state,
        TelemetryEvent::TacticalPacketReleaseDecided {
            decision_id: decision.decision_id,
            packet_id,
            frame_revision: decision.frame_revision,
            strategic_revision: decision.strategic_revision,
            rollout_mode: state.scheduler.rollout_mode().to_string(),
            release_policy: release,
            action_count,
            intent,
            released,
            reason_code: reason_code.to_owned(),
        },
    );
}

fn action_fact(action: &crate::execution::packet::TacticalAction) -> String {
    use crate::execution::packet::TacticalAction;
    match action {
        TacticalAction::MoveTo { tile_x, tile_y } => format!("move_to:{tile_x},{tile_y}"),
        TacticalAction::Attack { .. } => "attack".to_owned(),
        TacticalAction::UseSkill { .. } => "use_skill".to_owned(),
        TacticalAction::UseItem { .. } => "use_item".to_owned(),
        TacticalAction::PickUp { .. } => "pick_up".to_owned(),
        TacticalAction::SetTactics { style, mode } => {
            format!("set_tactics:{style:?},{mode:?}").to_ascii_lowercase()
        }
        TacticalAction::Stop => "stop".to_owned(),
    }
}

fn release_packet(
    state: &TacticianActorState,
    release: PacketRelease,
    packet: ActionPacket,
) -> (bool, &'static str) {
    match release {
        PacketRelease::RecordOnly => (false, "record_only"),
        PacketRelease::RequireControlGate => (false, "control_gate_required"),
        PacketRelease::Release if packet.proposal.actions.is_empty() => (false, "no_actions"),
        PacketRelease::Release => {
            match state
                .args
                .packet_sink
                .send_message(PlayerSupervisorMsg::SubmitModelPacket(packet))
            {
                Ok(()) => (false, "runtime_gate_pending"),
                Err(_) => (false, "runtime_gate_mailbox_closed"),
            }
        }
    }
}

fn apply_completion_follow_up(
    myself: &ActorRef<TacticianMsg>,
    state: &mut TacticianActorState,
    facts_before: TacticalScheduleFacts,
    follow_up: TacticalScheduleDecision,
) {
    let signal_id = uuid::Uuid::new_v4();
    let heartbeat_generated =
        follow_up.facts.heartbeats_generated > facts_before.heartbeats_generated;
    if heartbeat_generated {
        let latest = snapshot(state);
        record(
            state,
            TelemetryEvent::TacticalHeartbeatGenerated {
                signal_id,
                frame_revision: latest.frame_revision,
                strategic_revision: latest.strategic_revision,
                activity: state.scheduler.activity(),
            },
        );
        record(
            state,
            TelemetryEvent::TacticalWakeRequested {
                signal_id,
                frame_revision: latest.frame_revision,
                strategic_revision: latest.strategic_revision,
                reason: TacticalWakeReason::Heartbeat,
                activity: state.scheduler.activity(),
            },
        );
    }
    apply_schedule(
        myself,
        state,
        signal_id,
        snapshot(state),
        facts_before,
        follow_up,
        heartbeat_generated,
    );
    arm_next_tick(myself, state);
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn arm_next_tick(myself: &ActorRef<TacticianMsg>, state: &TacticianActorState) {
    if let Some(due) = state.scheduler.next_due_at() {
        let delay = due.saturating_sub(now(state)).max(Duration::from_millis(1));
        drop(myself.send_after(delay, || TacticianMsg::ScheduleTick));
    }
}

fn snapshot(state: &TacticianActorState) -> TacticalSnapshot {
    TacticalSnapshot {
        frame_revision: state.latest_frame.revision,
        strategic_revision: state.latest_strategy.revision,
    }
}

fn activity(frame: &TacticalFrame) -> TacticalActivity {
    if frame.combat.active == Some(true) {
        TacticalActivity::ActiveCombat
    } else {
        TacticalActivity::Idle
    }
}

fn frame_wake_reason(previous: &TacticalFrame, current: &TacticalFrame) -> TacticalWakeReason {
    if current.combat.damage_received_last_five_seconds
        != previous.combat.damage_received_last_five_seconds
    {
        TacticalWakeReason::DamageTaken
    } else if current.combat.active == Some(true) && previous.combat.active != Some(true) {
        TacticalWakeReason::CombatStarted
    } else if current.combat.active != Some(true) && previous.combat.active == Some(true) {
        TacticalWakeReason::CombatEnded
    } else if current.combat.current_hostiles > previous.combat.current_hostiles {
        TacticalWakeReason::HostileSpawned
    } else if current.combat.current_hostiles < previous.combat.current_hostiles {
        TacticalWakeReason::HostileDespawned
    } else if current.nearby_drops.len() > previous.nearby_drops.len() {
        TacticalWakeReason::LootAppeared
    } else {
        TacticalWakeReason::Forced
    }
}

fn now(state: &TacticianActorState) -> Duration {
    state.epoch.elapsed()
}

fn record(state: &TacticianActorState, event: TelemetryEvent) {
    let _ = state
        .args
        .telemetry
        .send_message(TelemetryMsg::Record(event));
}
