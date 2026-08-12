use std::{collections::HashSet, sync::Arc};

use ractor::{
    Actor, ActorProcessingErr, ActorRef, ActorStatus, SupervisionEvent, concurrency::JoinHandle,
};

use crate::{
    actors::{
        body::{BodyActor, BodyActorArgs},
        memory::{MemoryActor, MemoryActorArgs},
        perception::{PerceptionActor, PerceptionActorArgs},
        strategist::{StrategistActor, StrategistActorArgs},
        tactician::{TacticianActor, TacticianActorArgs},
        telemetry::{TelemetryActor, TelemetryActorArgs},
    },
    brain::{
        Brain, strategic_input::StrategicInput, strategic_output::StrategicProposal,
        tactical_input::TacticalInput, tactical_output::TacticalProposal,
    },
    character::CharacterSheet,
    config::HarnessConfig,
    execution::{gateway::BodyGateway, packet::ActionPacket},
    memory::{store::MemoryStore, working::WorkingMemory},
    observability::{AnalyticsEvent, AnalyticsSink, EventLevel},
    runtime::{
        blackboard::HotBlackboard,
        control_gate::{
            ControlledPacketError, ControlledPacketReceipt, ControlledPacketRequest,
            LiveMutationGate, assert_packet_limits, assert_runtime, runtime_packet,
        },
        messages::{
            ActorKind, BodyMsg, MemoryMsg, PerceptionMsg, PlayerRuntimeStatus, PlayerSupervisorMsg,
            StrategistMsg, TacticianMsg, TelemetryEvent, TelemetryMsg,
        },
        perception_pump::{
            PerceptionPumpArgs, PerceptionPumpConfig, PerceptionPumpHandle, PerceptionSource,
            start_perception_pump,
        },
    },
};

pub struct PlayerSupervisor;

pub struct PlayerSupervisorArgs {
    pub runtime_prefix: String,
    pub config: Arc<HarnessConfig>,
    pub character: Arc<CharacterSheet>,
    pub blackboard: Arc<HotBlackboard>,
    pub tactician_brain: Arc<dyn Brain<TacticalInput, TacticalProposal>>,
    pub strategist_brain: Option<Arc<dyn Brain<StrategicInput, StrategicProposal>>>,
    pub memory_store: Arc<dyn MemoryStore>,
    pub body_gateway: Arc<dyn BodyGateway>,
    pub session_generation: u64,
    pub body_connected: bool,
    pub perception_source: Option<Arc<dyn PerceptionSource>>,
    pub analytics: Arc<dyn AnalyticsSink>,
}

pub struct PlayerSupervisorState {
    args: PlayerSupervisorArgs,
    body: ActorRef<BodyMsg>,
    perception: ActorRef<PerceptionMsg>,
    tactician: ActorRef<TacticianMsg>,
    strategist: ActorRef<StrategistMsg>,
    memory: ActorRef<MemoryMsg>,
    telemetry: ActorRef<TelemetryMsg>,
    perception_pump: Option<PerceptionPumpHandle>,
    tactician_generation: u64,
    failures_observed: u64,
    live_mutation_gate: LiveMutationGate,
    shutting_down: bool,
    shutdown_terminated: HashSet<ActorKind>,
}

impl Actor for PlayerSupervisor {
    type Msg = PlayerSupervisorMsg;
    type State = PlayerSupervisorState;
    type Arguments = PlayerSupervisorArgs;

    #[allow(
        clippy::too_many_lines,
        reason = "one ordered startup keeps actor dependency wiring and ownership auditable"
    )]
    async fn pre_start(
        &self,
        myself: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        let prefix = &args.runtime_prefix;
        let supervisor = myself.get_cell();

        let (telemetry, _) = Actor::spawn_linked(
            Some(format!("{prefix}-telemetry")),
            TelemetryActor,
            TelemetryActorArgs {
                character_id: args.character.id.clone(),
                sink: args.analytics.clone(),
            },
            supervisor.clone(),
        )
        .await?;
        let (body, _) = Actor::spawn_linked(
            Some(format!("{prefix}-body")),
            BodyActor,
            BodyActorArgs {
                character: args.character.clone(),
                blackboard: args.blackboard.clone(),
                gateway: args.body_gateway.clone(),
                session_generation: args.session_generation,
                connected: args.body_connected,
                telemetry: telemetry.clone(),
            },
            supervisor.clone(),
        )
        .await?;
        let tactician = spawn_tactician(
            prefix,
            0,
            &args,
            &body,
            &telemetry,
            myself.clone(),
            supervisor.clone(),
        )
        .await?;
        let initial_working = WorkingMemory {
            goal: args.character.initial_goal.clone(),
            ..WorkingMemory::default()
        };
        let (memory, _) = Actor::spawn_linked(
            Some(format!("{prefix}-memory")),
            MemoryActor,
            MemoryActorArgs {
                character_id: args.character.id.clone(),
                initial_working,
                store: args.memory_store.clone(),
                telemetry: telemetry.clone(),
            },
            supervisor.clone(),
        )
        .await?;
        let (strategist, _) = Actor::spawn_linked(
            Some(format!("{prefix}-strategist")),
            StrategistActor,
            StrategistActorArgs {
                blackboard: args.blackboard.clone(),
                tactician: tactician.clone(),
                body: body.clone(),
                memory: memory.clone(),
                telemetry: telemetry.clone(),
                minimum_interval: args.config.models.strategist_min_interval,
            },
            supervisor.clone(),
        )
        .await?;
        if let Some(brain) = &args.strategist_brain {
            strategist.send_message(StrategistMsg::InstallBrain {
                character_id: args.character.id.clone(),
                persona: args.character.persona.clone(),
                brain: brain.clone(),
            })?;
            strategist.send_message(StrategistMsg::Reflect)?;
        }
        let (perception, _) = Actor::spawn_linked(
            Some(format!("{prefix}-perception")),
            PerceptionActor,
            PerceptionActorArgs {
                blackboard: args.blackboard.clone(),
                body: body.clone(),
                tactician: tactician.clone(),
                strategist: strategist.clone(),
                player_name: args.character.player_name.clone(),
                telemetry: telemetry.clone(),
            },
            supervisor.clone(),
        )
        .await?;
        body.send_message(BodyMsg::ReplacePerception(perception.clone()))?;
        let perception_pump = args
            .perception_source
            .as_ref()
            .map(|source| {
                start_perception_pump(PerceptionPumpArgs {
                    character_id: args.character.id.clone(),
                    source: source.clone(),
                    blackboard: args.blackboard.clone(),
                    perception: perception.clone(),
                    analytics: args.analytics.clone(),
                    config: PerceptionPumpConfig {
                        interval: args.config.runtime.perception_interval,
                        map_radius: args.config.runtime.perception_map_radius,
                        inventory_every_cycles: args
                            .config
                            .runtime
                            .perception_inventory_every_cycles,
                    },
                })
            })
            .transpose()
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        for actor in [
            ActorKind::Telemetry,
            ActorKind::Body,
            ActorKind::Tactician,
            ActorKind::Perception,
            ActorKind::Strategist,
            ActorKind::Memory,
        ] {
            let _ =
                telemetry.send_message(TelemetryMsg::Record(TelemetryEvent::ActorStarted(actor)));
        }

        tracing::info!(
            character_id = %args.character.id,
            tactical_max_hz = args.config.runtime.tactical_max_hz,
            "player supervision subtree started"
        );
        let live_mutation_gate = LiveMutationGate::new(args.config.runtime.live_action_budget);
        Ok(PlayerSupervisorState {
            args,
            body,
            perception,
            tactician,
            strategist,
            memory,
            telemetry,
            perception_pump,
            tactician_generation: 0,
            failures_observed: 0,
            live_mutation_gate,
            shutting_down: false,
            shutdown_terminated: HashSet::new(),
        })
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            PlayerSupervisorMsg::PerceptionInput(input) => {
                state
                    .perception
                    .send_message(PerceptionMsg::Observation(input))?;
            }
            PlayerSupervisorMsg::SessionInvalidated { generation, reason } => {
                state
                    .args
                    .blackboard
                    .invalidate_before(state.args.blackboard.frame().revision.saturating_add(1));
                state.body.send_message(BodyMsg::CancelCurrentAction(
                    crate::runtime::messages::ActionCancelReason::AbortCondition(format!(
                        "session_generation_{generation}:{reason}"
                    )),
                ))?;
                state
                    .body
                    .send_message(BodyMsg::SessionGenerationChanged(generation))?;
            }
            PlayerSupervisorMsg::Health(reply) => {
                if !reply.is_closed() {
                    let actors = [
                        (ActorKind::Body, active(&state.body)),
                        (ActorKind::Perception, active(&state.perception)),
                        (ActorKind::Tactician, active(&state.tactician)),
                        (ActorKind::Strategist, active(&state.strategist)),
                        (ActorKind::Memory, active(&state.memory)),
                        (ActorKind::Telemetry, active(&state.telemetry)),
                    ];
                    reply.send(PlayerRuntimeStatus {
                        character_id: state.args.character.id.clone(),
                        running: actors
                            .into_iter()
                            .filter_map(|(kind, is_active)| is_active.then_some(kind))
                            .collect::<HashSet<_>>(),
                        failures_observed: state.failures_observed,
                    })?;
                }
            }
            PlayerSupervisorMsg::BodyHealth(reply) => {
                let status = ractor::call_t!(state.body, BodyMsg::Health, 1_000)?;
                if !reply.is_closed() {
                    reply.send(status)?;
                }
            }
            PlayerSupervisorMsg::TelemetryHealth(reply) => {
                reply_telemetry_snapshot(state, reply).await?;
            }
            PlayerSupervisorMsg::ValidateControlledPacket(request, reply) => {
                let result = process_controlled_packet(state, request, false).await;
                if !reply.is_closed() {
                    reply.send(result)?;
                }
            }
            PlayerSupervisorMsg::SubmitControlledPacket(request, reply) => {
                let result = process_controlled_packet(state, request, true).await;
                if !reply.is_closed() {
                    reply.send(result)?;
                }
            }
            PlayerSupervisorMsg::SubmitModelPacket(packet) => {
                process_model_packet(state, packet).await;
            }
            PlayerSupervisorMsg::ActivateSafetyFallback(reason_code, reply) => {
                let result = ractor::call_t!(
                    state.body,
                    BodyMsg::ActivateSafetyFallback,
                    2_500,
                    reason_code
                )
                .unwrap_or_else(|_| {
                    Err(crate::execution::gateway::BodyGatewayError {
                        class: "body_unavailable".to_owned(),
                    })
                });
                if !reply.is_closed() {
                    reply.send(result)?;
                }
            }
            #[cfg(test)]
            PlayerSupervisorMsg::FailTacticianForTest => {
                state.tactician.send_message(TacticianMsg::FailForTest)?;
            }
            PlayerSupervisorMsg::Shutdown => {
                state.shutting_down = true;
                if let Some(pump) = state.perception_pump.take() {
                    pump.shutdown().await?;
                }
                state.body.send_message(BodyMsg::Shutdown)?;
                state.perception.send_message(PerceptionMsg::Shutdown)?;
                state.tactician.send_message(TacticianMsg::Shutdown)?;
                state.strategist.send_message(StrategistMsg::Shutdown)?;
                state.memory.send_message(MemoryMsg::Shutdown)?;
            }
        }
        Ok(())
    }

    async fn handle_supervisor_evt(
        &self,
        myself: ActorRef<Self::Msg>,
        message: SupervisionEvent,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            SupervisionEvent::ActorFailed(actor, error) => {
                state.failures_observed += 1;
                let kind = actor_kind(actor.get_id(), state);
                let reason = error.to_string();
                tracing::error!(?kind, %reason, "player child actor failed");
                let _ = state.args.blackboard.current_packet();
                let _ = state.telemetry.send_message(TelemetryMsg::Record(
                    TelemetryEvent::ActorFailed {
                        actor: kind,
                        reason,
                    },
                ));

                if !state.shutting_down && kind == ActorKind::Tactician {
                    state.tactician_generation += 1;
                    let replacement = spawn_tactician(
                        &state.args.runtime_prefix,
                        state.tactician_generation,
                        &state.args,
                        &state.body,
                        &state.telemetry,
                        myself.clone(),
                        myself.get_cell(),
                    )
                    .await?;
                    state.tactician = replacement.clone();
                    let _ = state
                        .perception
                        .send_message(PerceptionMsg::ReplaceTactician(replacement.clone()));
                    let _ = state
                        .strategist
                        .send_message(StrategistMsg::ReplaceTactician(replacement));
                }
            }
            SupervisionEvent::ActorTerminated(actor, _, _reason) if state.shutting_down => {
                let kind = actor_kind(actor.get_id(), state);
                state.shutdown_terminated.insert(kind);
                if kind == ActorKind::Telemetry {
                    myself.stop(Some("player runtime shutdown".to_owned()));
                } else if [
                    ActorKind::Body,
                    ActorKind::Perception,
                    ActorKind::Tactician,
                    ActorKind::Strategist,
                    ActorKind::Memory,
                ]
                .iter()
                .all(|kind| state.shutdown_terminated.contains(kind))
                {
                    state.telemetry.send_message(TelemetryMsg::Shutdown)?;
                }
            }
            SupervisionEvent::ActorTerminated(actor, _, reason) => {
                let kind = actor_kind(actor.get_id(), state);
                tracing::warn!(
                    actor = ?actor.get_name(),
                    ?reason,
                    "player child actor terminated"
                );
                let _ = state.telemetry.send_message(TelemetryMsg::Record(
                    TelemetryEvent::ActorTerminated {
                        actor: kind,
                        reason: reason.clone(),
                    },
                ));
            }
            SupervisionEvent::ActorStarted(actor) => {
                tracing::debug!(actor = ?actor.get_name(), "player child actor started");
            }
            SupervisionEvent::ProcessGroupChanged(_) => {}
        }
        Ok(())
    }
}

async fn reply_telemetry_snapshot(
    state: &PlayerSupervisorState,
    reply: ractor::RpcReplyPort<crate::runtime::messages::TelemetrySnapshot>,
) -> Result<(), ActorProcessingErr> {
    let snapshot = ractor::call_t!(state.telemetry, TelemetryMsg::Snapshot, 1_000)?;
    if !reply.is_closed() {
        reply.send(snapshot)?;
    }
    Ok(())
}

async fn process_controlled_packet(
    state: &mut PlayerSupervisorState,
    request: ControlledPacketRequest,
    release: bool,
) -> Result<ControlledPacketReceipt, ControlledPacketError> {
    let frame = state.args.blackboard.frame();
    let packet = runtime_packet(
        frame.revision,
        state.args.blackboard.strategic_revision(),
        frame.self_state.scene.clone(),
        request.proposal.clone(),
    );
    let action_count = packet.proposal.actions.len();
    let result = async {
        if release {
            state.live_mutation_gate.authorize(
                &state.args.config.runtime,
                &state.args.character,
                frame.self_state.scene.as_deref(),
                &request,
            )?;
        } else {
            assert_runtime(
                &state.args.character,
                frame.self_state.scene.as_deref(),
                &request,
            )?;
            assert_packet_limits(&state.args.config.runtime, &request.proposal)?;
        }

        match ractor::call_t!(state.body, BodyMsg::ValidateTactical, 1_000, packet.clone()) {
            Ok(Ok(())) => {}
            Ok(Err(reason)) => return Err(ControlledPacketError::BodyRejected(reason)),
            Err(_) => return Err(ControlledPacketError::BodyUnavailable),
        }

        let remaining = if release {
            let remaining = state.live_mutation_gate.consume(action_count)?;
            state
                .body
                .send_message(BodyMsg::ExecuteTactical(packet.clone()))
                .map_err(|_| ControlledPacketError::BodyUnavailable)?;
            remaining
        } else {
            state.live_mutation_gate.remaining_actions()
        };

        Ok(ControlledPacketReceipt {
            packet_id: packet.id,
            decision_id: packet.decision_id,
            frame_revision: packet.frame_revision,
            strategic_revision: packet.strategic_revision,
            action_count,
            remaining_live_action_budget: remaining,
            live_action_budget_unlimited: state.live_mutation_gate.is_unlimited(),
            released: release,
        })
    }
    .await;

    record_controlled_packet_decision(state, &packet, release, &result);
    result
}

async fn process_model_packet(state: &mut PlayerSupervisorState, packet: ActionPacket) {
    let frame = state.args.blackboard.frame();
    let action_count = packet.proposal.actions.len();
    let result = async {
        state.live_mutation_gate.authorize_model_packet(
            &state.args.config.runtime,
            &state.args.character,
            frame.self_state.scene.as_deref(),
            &packet.proposal,
        )?;
        match ractor::call_t!(state.body, BodyMsg::ValidateTactical, 1_000, packet.clone()) {
            Ok(Ok(())) => {}
            Ok(Err(reason)) => return Err(ControlledPacketError::BodyRejected(reason)),
            Err(_) => return Err(ControlledPacketError::BodyUnavailable),
        }
        let remaining = state.live_mutation_gate.consume(action_count)?;
        state
            .body
            .send_message(BodyMsg::ExecuteTactical(packet.clone()))
            .map_err(|_| ControlledPacketError::BodyUnavailable)?;
        Ok(remaining)
    }
    .await;

    let (released, reason_code, body_rejection_code, level, remaining) = match result {
        Ok(remaining) => (true, "released", None, EventLevel::Info, remaining),
        Err(ref error) => (
            false,
            error.reason_code(),
            error.body_rejection_code(),
            EventLevel::Warn,
            state.live_mutation_gate.remaining_actions(),
        ),
    };
    state.args.analytics.record(
        AnalyticsEvent::new("runtime.model_packet_decided", level)
            .character(&state.args.character.id)
            .correlation(packet.decision_id)
            .attribute("decision_id", packet.decision_id.to_string())
            .attribute("packet_id", packet.id.to_string())
            .attribute("frame_revision", packet.frame_revision)
            .attribute("strategic_revision", packet.strategic_revision)
            .attribute(
                "action_count",
                u64::try_from(action_count).unwrap_or(u64::MAX),
            )
            .attribute("valid_for_ms", packet.proposal.valid_for_ms)
            .attribute("released", released)
            .attribute("reason_code", reason_code)
            .attribute("body_rejection_known", body_rejection_code.is_some())
            .attribute("body_rejection_code", body_rejection_code.unwrap_or(""))
            .attribute("remaining_live_action_budget", remaining.unwrap_or(0))
            .attribute(
                "live_action_budget_unlimited",
                state.live_mutation_gate.is_unlimited(),
            ),
    );
}

fn record_controlled_packet_decision(
    state: &PlayerSupervisorState,
    packet: &crate::execution::packet::ActionPacket,
    release_requested: bool,
    result: &Result<ControlledPacketReceipt, ControlledPacketError>,
) {
    let (released, reason_code, body_rejection_code, level) = match result {
        Ok(receipt) => (
            receipt.released,
            if receipt.released {
                "released"
            } else {
                "validated"
            },
            None,
            EventLevel::Info,
        ),
        Err(error) => (
            false,
            error.reason_code(),
            error.body_rejection_code(),
            EventLevel::Warn,
        ),
    };
    state.args.analytics.record(
        AnalyticsEvent::new("runtime.controlled_packet_decided", level)
            .character(&state.args.character.id)
            .correlation(packet.decision_id)
            .attribute("decision_id", packet.decision_id.to_string())
            .attribute("packet_id", packet.id.to_string())
            .attribute("frame_revision", packet.frame_revision)
            .attribute("strategic_revision", packet.strategic_revision)
            .attribute(
                "action_count",
                u64::try_from(packet.proposal.actions.len()).unwrap_or(u64::MAX),
            )
            .attribute("valid_for_ms", packet.proposal.valid_for_ms)
            .attribute("release_requested", release_requested)
            .attribute("released", released)
            .attribute("reason_code", reason_code)
            .attribute("body_rejection_known", body_rejection_code.is_some())
            .attribute("body_rejection_code", body_rejection_code.unwrap_or(""))
            .attribute(
                "remaining_live_action_budget",
                state.live_mutation_gate.remaining_actions().unwrap_or(0),
            )
            .attribute(
                "live_action_budget_unlimited",
                state.live_mutation_gate.is_unlimited(),
            ),
    );
}

async fn spawn_tactician(
    prefix: &str,
    generation: u64,
    args: &PlayerSupervisorArgs,
    body: &ActorRef<BodyMsg>,
    telemetry: &ActorRef<TelemetryMsg>,
    packet_sink: ActorRef<PlayerSupervisorMsg>,
    supervisor: ractor::ActorCell,
) -> Result<ActorRef<TacticianMsg>, ractor::SpawnErr> {
    let (actor, _join): (ActorRef<TacticianMsg>, JoinHandle<()>) = Actor::spawn_linked(
        Some(format!("{prefix}-tactician-{generation}")),
        TacticianActor,
        TacticianActorArgs {
            character_id: args.character.id.clone(),
            blackboard: args.blackboard.clone(),
            body: body.clone(),
            packet_sink,
            telemetry: telemetry.clone(),
            brain: args.tactician_brain.clone(),
            schedule_config: crate::runtime::tactical_schedule::TacticalScheduleConfig::from_hz(
                args.config.runtime.tactical_max_hz,
                args.config.runtime.idle_tactical_hz,
                std::time::Duration::from_secs(1),
            )
            .expect("runtime tactical rates were validated by HarnessConfig"),
            rollout_mode: args.config.runtime.tactical_rollout_mode,
        },
        supervisor,
    )
    .await?;
    Ok(actor)
}

fn active<M>(actor: &ActorRef<M>) -> bool {
    matches!(
        actor.get_status(),
        ActorStatus::Starting | ActorStatus::Running | ActorStatus::Upgrading
    )
}

fn actor_kind(id: ractor::ActorId, state: &PlayerSupervisorState) -> ActorKind {
    if id == state.body.get_id() {
        ActorKind::Body
    } else if id == state.perception.get_id() {
        ActorKind::Perception
    } else if id == state.tactician.get_id() {
        ActorKind::Tactician
    } else if id == state.strategist.get_id() {
        ActorKind::Strategist
    } else if id == state.memory.get_id() {
        ActorKind::Memory
    } else {
        ActorKind::Telemetry
    }
}
