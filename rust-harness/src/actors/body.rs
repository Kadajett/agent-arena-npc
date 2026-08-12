use std::{sync::Arc, time::Instant};

use chrono::{DateTime, Utc};
use num_traits::ToPrimitive;
use ractor::{Actor, ActorProcessingErr, ActorRef};

use crate::{
    character::{Capability, CharacterSheet},
    execution::{
        gateway::{BodyCommand, BodyGateway, BodySpeechChannel, ExecutionContext},
        invalidation::{
            ExecutionValidityFacts, MaterialComparison, MaterialInvalidationFact,
            compare_material_state,
        },
        movement::{
            MovementObservationRules, MovementOwnership, MovementProgress, MovementRequest,
            MovementState, MovementTelemetry, NavigationArrival, NavigationAttemptKind,
            NavigationMissionRequest, NavigationMissionState, NavigationMissionTelemetry,
        },
        outcome::{ActionOutcome, OutcomeStatus, PacketTerminalStatus},
        packet::{ActionPacket, TacticalAction},
        validator::{ValidationContext, validate_action, validate_packet, validate_packet_header},
    },
    runtime::{
        blackboard::HotBlackboard,
        messages::{
            ActionCancelReason, ActionExecutionCompleted, BodyMsg, BodyStatus, PerceptionMsg,
            SafetyFallbackCompleted, SafetyFallbackResult, StrategicDuelCompleted,
            StrategicDuelRequest, StrategicInteractionCompleted, StrategicInteractionRequest,
            StrategicSpeechCompleted, StrategicSpeechRequest, TelemetryEvent, TelemetryMsg,
        },
    },
};

pub struct BodyActor;

pub struct BodyActorArgs {
    pub character: Arc<CharacterSheet>,
    pub blackboard: Arc<HotBlackboard>,
    pub gateway: Arc<dyn BodyGateway>,
    pub session_generation: u64,
    pub connected: bool,
    pub telemetry: ActorRef<TelemetryMsg>,
}

struct ActiveAction {
    context: ExecutionContext,
    kind: String,
    started_at: DateTime<Utc>,
    movement: Option<MovementProgress>,
}

struct ActiveMovementStop {
    context: ExecutionContext,
    started_at: DateTime<Utc>,
    reason_code: String,
}

struct ActivePacket {
    packet: ActionPacket,
    accepted_frame: Arc<crate::brain::tactical_frame::TacticalFrame>,
    accepted_intent: Arc<crate::brain::strategic_intent::StrategicIntent>,
    next_action_index: usize,
    in_flight: Option<ActiveAction>,
}

struct ActiveNavigationMission {
    id: uuid::Uuid,
    request: NavigationMissionRequest,
    state: NavigationMissionState,
    waypoint_index: usize,
    attempt_number: u32,
    retry_pending: bool,
    /// Index of the same-destination doorway tile to try next. Some Reldens
    /// doors occupy more than one collision tile, and the first tile can be
    /// rejected when the player is standing on an overlapping pixel boundary.
    door_retry_index: usize,
    recovery_tile: Option<crate::world::TilePosition>,
    in_flight: Option<ActiveAction>,
}

pub struct BodyActorState {
    args: BodyActorArgs,
    perception: Option<ActorRef<PerceptionMsg>>,
    active: Option<ActivePacket>,
    navigation: Option<ActiveNavigationMission>,
    movement_stop: Option<ActiveMovementStop>,
    status: BodyStatus,
}

impl Actor for BodyActor {
    type Msg = BodyMsg;
    type State = BodyActorState;
    type Arguments = BodyActorArgs;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        let connected = args.connected;
        Ok(BodyActorState {
            args,
            perception: None,
            active: None,
            navigation: None,
            movement_stop: None,
            status: BodyStatus {
                connected,
                current_packet_id: None,
                accepted_packets: 0,
                rejected_packets: 0,
                last_terminal_packet_id: None,
                last_terminal_status: None,
                active_navigation_mission_id: None,
                navigation_state: None,
            },
        })
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            BodyMsg::Think(request) => start_thought(&myself, request, state),
            BodyMsg::Speak(request) => start_speech(&myself, request, state),
            BodyMsg::SpeechCompleted(completed) => finish_speech(&completed, state),
            BodyMsg::Interact(request) => start_interaction(&myself, &request, state),
            BodyMsg::InteractionCompleted(completed) => finish_interaction(&completed, state),
            BodyMsg::QueueDuel(request) => start_duel(&myself, &request, state),
            BodyMsg::DuelQueued(completed) => finish_duel(&completed, state),
            BodyMsg::ExecuteTactical(packet) => accept_packet(&myself, packet, state),
            BodyMsg::PursueNavigation(request) => {
                accept_navigation_mission(&myself, request, state);
            }
            BodyMsg::NavigationActionCompleted(completed) => {
                finish_navigation_action(&myself, &completed, state);
            }
            BodyMsg::ValidateTactical(packet, reply) => {
                if !reply.is_closed() {
                    let frame = state.args.blackboard.frame();
                    let context = validation_context(state, &frame);
                    reply.send(validate_packet(&packet, &context))?;
                }
            }
            BodyMsg::ActionCompleted(completed) => finish_action(&myself, &completed, state),
            BodyMsg::MovementStopCompleted(completed) => {
                finish_movement_stop(&myself, completed, state);
            }
            BodyMsg::ActivateSafetyFallback(reason_code, reply) => {
                start_safety_fallback(&myself, reason_code, reply, state);
            }
            BodyMsg::SafetyFallbackCompleted(completed) => {
                finish_safety_fallback(completed, state);
            }
            BodyMsg::FrameUpdated(frame) => handle_frame_update(&myself, &frame, state),
            BodyMsg::SessionGenerationChanged(generation) => {
                cancel_navigation(&myself, "session_generation_changed", state);
                state.args.session_generation = generation;
                state.status.connected = true;
            }
            BodyMsg::CancelCurrentAction(reason) => {
                let frame_revision = state.args.blackboard.frame().revision;
                cancel_active(&myself, reason, frame_revision, state);
            }
            BodyMsg::ReplacePerception(perception) => state.perception = Some(perception),
            BodyMsg::Health(reply) => {
                if !reply.is_closed() {
                    state.status.active_navigation_mission_id =
                        state.navigation.as_ref().map(|mission| mission.id);
                    state.status.navigation_state =
                        state.navigation.as_ref().map(|mission| mission.state);
                    reply.send(state.status.clone())?;
                }
            }
            BodyMsg::Shutdown => {
                let frame_revision = state.args.blackboard.frame().revision;
                cancel_active(&myself, ActionCancelReason::Shutdown, frame_revision, state);
                cancel_navigation(&myself, "runtime_shutdown", state);
                myself.stop(Some("player runtime shutdown".to_owned()));
            }
        }
        Ok(())
    }
}

fn start_thought(
    myself: &ActorRef<BodyMsg>,
    request: crate::runtime::messages::StrategicThoughtRequest,
    state: &BodyActorState,
) {
    let context = ExecutionContext {
        session_generation: state.args.session_generation,
        decision_id: request.decision_id,
        packet_id: uuid::Uuid::new_v4(),
        action_id: uuid::Uuid::new_v4(),
        action_index: 0,
        frame_revision: request.frame_revision,
        strategic_revision: request.strategic_revision,
    };
    let action_kind = "think".to_owned();
    record(
        state,
        TelemetryEvent::ActionStarted {
            context,
            action_kind: action_kind.clone(),
        },
    );
    let invalid = !state
        .args
        .character
        .capabilities
        .contains(&Capability::Purpose)
        || request.thought.trim().is_empty();
    if invalid {
        publish_outcome(
            state,
            action_outcome(
                &context,
                &action_kind,
                Utc::now(),
                0,
                OutcomeStatus::Failed,
                Some("strategic_thought_rejected".to_owned()),
                state.args.blackboard.frame().revision,
                None,
            ),
        );
        return;
    }
    let gateway = state.args.gateway.clone();
    let reply_to = myself.clone();
    let started_at = Utc::now();
    tokio::spawn(async move {
        let started = Instant::now();
        let result = gateway
            .execute(
                BodyCommand::Think {
                    thought: request.thought,
                },
                context,
            )
            .await;
        let completed = ActionExecutionCompleted {
            context,
            action_kind,
            started_at,
            duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            result,
        };
        let _ = reply_to.send_message(BodyMsg::ActionCompleted(completed));
    });
}

fn start_speech(
    myself: &ActorRef<BodyMsg>,
    request: StrategicSpeechRequest,
    state: &BodyActorState,
) {
    let context = ExecutionContext {
        session_generation: state.args.session_generation,
        decision_id: request.decision_id,
        packet_id: uuid::Uuid::new_v4(),
        action_id: uuid::Uuid::new_v4(),
        action_index: 0,
        frame_revision: request.frame_revision,
        strategic_revision: request.strategic_revision,
    };
    let action_kind = format!("say_{}", speech_channel_name(request.channel));
    record(
        state,
        TelemetryEvent::ActionStarted {
            context,
            action_kind: action_kind.clone(),
        },
    );
    let invalid = !state
        .args
        .character
        .capabilities
        .contains(&Capability::Speak)
        || request.message.trim().is_empty()
        || request.message.chars().count() > 140
        || (request.channel == BodySpeechChannel::Private && request.to_player.is_none());
    if invalid {
        let outcome = action_outcome(
            &context,
            &action_kind,
            Utc::now(),
            0,
            OutcomeStatus::Failed,
            Some("strategic_speech_rejected".to_owned()),
            state.args.blackboard.frame().revision,
            None,
        );
        publish_outcome(state, outcome);
        return;
    }
    let command = BodyCommand::Say {
        message: request.message,
        channel: request.channel,
        to_player: request.to_player,
    };
    let gateway = state.args.gateway.clone();
    let reply_to = myself.clone();
    let started_at = Utc::now();
    tokio::spawn(async move {
        let started = Instant::now();
        let result = gateway.execute(command, context).await;
        let _ = reply_to.send_message(BodyMsg::SpeechCompleted(StrategicSpeechCompleted {
            context,
            started_at,
            duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            result,
        }));
    });
}

fn finish_speech(completed: &StrategicSpeechCompleted, state: &BodyActorState) {
    let (status, reason) = match &completed.result {
        Ok(result) if result.accepted != Some(false) => (OutcomeStatus::Succeeded, None),
        Ok(_) => (
            OutcomeStatus::Failed,
            Some("strategic_speech_backend_refused".to_owned()),
        ),
        Err(error) => (OutcomeStatus::Failed, Some(error.class.clone())),
    };
    let outcome = action_outcome(
        &completed.context,
        "strategic_speech",
        completed.started_at,
        completed.duration_ms,
        status,
        reason,
        state.args.blackboard.frame().revision,
        None,
    );
    publish_outcome(state, outcome);
}

fn start_interaction(
    myself: &ActorRef<BodyMsg>,
    request: &StrategicInteractionRequest,
    state: &BodyActorState,
) {
    let context = ExecutionContext {
        session_generation: state.args.session_generation,
        decision_id: request.decision_id,
        packet_id: uuid::Uuid::new_v4(),
        action_id: uuid::Uuid::new_v4(),
        action_index: 0,
        frame_revision: request.frame_revision,
        strategic_revision: request.strategic_revision,
    };
    record(
        state,
        TelemetryEvent::ActionStarted {
            context,
            action_kind: "interact".to_owned(),
        },
    );
    let frame = state.args.blackboard.frame();
    let target = frame
        .nearby_entities
        .iter()
        .find(|entity| entity.id == request.target_id);
    let object_id = target.and_then(|entity| entity.backend_object_id);
    let valid_target = target.is_some_and(|entity| {
        !matches!(
            entity.kind,
            crate::brain::tactical_frame::EntityKind::Player
                | crate::brain::tactical_frame::EntityKind::Enemy
        ) && entity.interactable == Some(true)
            && entity.alive != Some(false)
            && entity.distance.is_some_and(|distance| distance <= 2.0)
    });
    let invalid = !state
        .args
        .character
        .capabilities
        .contains(&Capability::TalkToFolk)
        || request.frame_revision != frame.revision
        || object_id.is_none()
        || !valid_target;
    if invalid {
        publish_outcome(
            state,
            action_outcome(
                &context,
                "interact",
                Utc::now(),
                0,
                OutcomeStatus::Failed,
                Some("strategic_interaction_rejected".to_owned()),
                frame.revision,
                None,
            ),
        );
        return;
    }
    let gateway = state.args.gateway.clone();
    let reply_to = myself.clone();
    let started_at = Utc::now();
    tokio::spawn(async move {
        let started = Instant::now();
        let result = gateway
            .execute(
                BodyCommand::TalkTo {
                    object_id: object_id.expect("validated object id"),
                },
                context,
            )
            .await;
        let _ = reply_to.send_message(BodyMsg::InteractionCompleted(
            StrategicInteractionCompleted {
                context,
                started_at,
                duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                result,
            },
        ));
    });
}

fn start_duel(myself: &ActorRef<BodyMsg>, request: &StrategicDuelRequest, state: &BodyActorState) {
    let context = ExecutionContext {
        session_generation: state.args.session_generation,
        decision_id: request.decision_id,
        packet_id: uuid::Uuid::new_v4(),
        action_id: uuid::Uuid::new_v4(),
        action_index: 0,
        frame_revision: request.frame_revision,
        strategic_revision: request.strategic_revision,
    };
    let frame = state.args.blackboard.frame();
    let scene_name = frame.self_state.scene.clone();
    let valid = state
        .args
        .character
        .capabilities
        .contains(&Capability::Duel)
        && request.frame_revision == frame.revision
        && scene_name
            .as_ref()
            .is_some_and(|scene| !scene.trim().is_empty());
    record(
        state,
        TelemetryEvent::ActionStarted {
            context,
            action_kind: "queue_duel".to_owned(),
        },
    );
    if !valid {
        publish_outcome(
            state,
            action_outcome(
                &context,
                "queue_duel",
                Utc::now(),
                0,
                OutcomeStatus::Failed,
                Some("strategic_duel_rejected".to_owned()),
                frame.revision,
                None,
            ),
        );
        return;
    }
    let gateway = state.args.gateway.clone();
    let reply_to = myself.clone();
    let started_at = Utc::now();
    tokio::spawn(async move {
        let started = Instant::now();
        let result = gateway
            .execute(
                BodyCommand::QueueDuel {
                    scene_name: scene_name.expect("validated scene name"),
                },
                context,
            )
            .await;
        let _ = reply_to.send_message(BodyMsg::DuelQueued(StrategicDuelCompleted {
            context,
            started_at,
            duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            result,
        }));
    });
}

fn finish_duel(completed: &StrategicDuelCompleted, state: &BodyActorState) {
    let (status, reason) = match &completed.result {
        Ok(result) if result.accepted != Some(false) => (OutcomeStatus::Succeeded, None),
        Ok(_) => (OutcomeStatus::Failed, Some("backend_refused".to_owned())),
        Err(error) => (OutcomeStatus::Failed, Some(error.class.clone())),
    };
    publish_outcome(
        state,
        action_outcome(
            &completed.context,
            "queue_duel",
            completed.started_at,
            completed.duration_ms,
            status,
            reason,
            state.args.blackboard.frame().revision,
            None,
        ),
    );
}

fn finish_interaction(completed: &StrategicInteractionCompleted, state: &BodyActorState) {
    let (status, reason) = match &completed.result {
        Ok(result) if result.accepted != Some(false) => (OutcomeStatus::Succeeded, None),
        Ok(_) => (
            OutcomeStatus::Failed,
            Some("strategic_interaction_backend_refused".to_owned()),
        ),
        Err(error) => (OutcomeStatus::Failed, Some(error.class.clone())),
    };
    publish_outcome(
        state,
        action_outcome(
            &completed.context,
            "interact",
            completed.started_at,
            completed.duration_ms,
            status,
            reason,
            state.args.blackboard.frame().revision,
            None,
        ),
    );
}

const fn speech_channel_name(channel: BodySpeechChannel) -> &'static str {
    match channel {
        BodySpeechChannel::Scene => "scene",
        BodySpeechChannel::Global => "global",
        BodySpeechChannel::Private => "private",
    }
}

fn start_safety_fallback(
    myself: &ActorRef<BodyMsg>,
    reason_code: String,
    reply: ractor::RpcReplyPort<
        Result<SafetyFallbackResult, crate::execution::gateway::BodyGatewayError>,
    >,
    state: &mut BodyActorState,
) {
    let frame = state.args.blackboard.frame();
    cancel_active(
        myself,
        ActionCancelReason::AbortCondition(format!("safety_fallback:{reason_code}")),
        frame.revision,
        state,
    );
    let context = ExecutionContext {
        session_generation: state.args.session_generation,
        decision_id: uuid::Uuid::new_v4(),
        packet_id: uuid::Uuid::new_v4(),
        action_id: uuid::Uuid::new_v4(),
        action_index: 0,
        frame_revision: frame.revision,
        strategic_revision: state.args.blackboard.strategic_revision(),
    };
    let command = BodyCommand::SetTactics {
        style: crate::execution::packet::TacticalStyle::Flee,
        mode: crate::execution::packet::TacticalMode::SemiAuto,
    };
    let action_kind = "safety_fallback_set_tactics_flee".to_owned();
    let started_at = Utc::now();
    record(
        state,
        TelemetryEvent::ActionStarted {
            context,
            action_kind,
        },
    );
    let gateway = state.args.gateway.clone();
    let reply_to = myself.clone();
    tokio::spawn(async move {
        let started = Instant::now();
        let result = gateway.execute(command, context).await;
        let _ = reply_to.send_message(BodyMsg::SafetyFallbackCompleted(SafetyFallbackCompleted {
            context,
            started_at,
            duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            reason_code,
            result,
            reply,
        }));
    });
}

fn finish_safety_fallback(completed: SafetyFallbackCompleted, state: &BodyActorState) {
    let (status, terminal_reason) = match &completed.result {
        Ok(result) if result.accepted != Some(false) => (OutcomeStatus::Succeeded, None),
        Ok(_) => (
            OutcomeStatus::Failed,
            Some("safety_fallback_backend_refused".to_owned()),
        ),
        Err(error) => (OutcomeStatus::Failed, Some(error.class.clone())),
    };
    publish_outcome(
        state,
        action_outcome(
            &completed.context,
            "safety_fallback_set_tactics_flee",
            completed.started_at,
            completed.duration_ms,
            status,
            terminal_reason.clone(),
            state.args.blackboard.frame().revision,
            None,
        ),
    );
    let result = SafetyFallbackResult {
        context: completed.context,
        duration_ms: completed.duration_ms,
        status,
        reason_code: terminal_reason,
    };
    if !completed.reply.is_closed() {
        let _ = completed.reply.send(match completed.result {
            Ok(_) => Ok(result),
            Err(error) => Err(error),
        });
    }
}

fn accept_packet(myself: &ActorRef<BodyMsg>, packet: ActionPacket, state: &mut BodyActorState) {
    let frame = state.args.blackboard.frame();
    let context = validation_context(state, &frame);
    if let Err(error) = validate_packet(&packet, &context) {
        state.status.rejected_packets += 1;
        record(
            state,
            TelemetryEvent::PacketRejected {
                packet_id: packet.id,
                decision_id: packet.decision_id,
                frame_revision: packet.frame_revision,
                strategic_revision: packet.strategic_revision,
                reason: error.to_string(),
            },
        );
        return;
    }

    pause_navigation(myself, "tactical_preemption", state);
    supersede_active(myself, packet.id, state);

    state.status.accepted_packets += 1;
    state.status.current_packet_id = Some(packet.id);
    state
        .args
        .blackboard
        .set_current_packet(Some(Arc::new(packet.clone())));
    record(
        state,
        TelemetryEvent::PacketAccepted {
            packet_id: packet.id,
            decision_id: packet.decision_id,
            frame_revision: packet.frame_revision,
            strategic_revision: packet.strategic_revision,
        },
    );
    state.active = Some(ActivePacket {
        packet,
        accepted_frame: frame,
        accepted_intent: state.args.blackboard.strategy(),
        next_action_index: 0,
        in_flight: None,
    });
    start_next_action(myself, state);
}

fn accept_navigation_mission(
    myself: &ActorRef<BodyMsg>,
    request: NavigationMissionRequest,
    state: &mut BodyActorState,
) {
    let frame = state.args.blackboard.frame();
    let valid = state
        .args
        .character
        .capabilities
        .contains(&Capability::Walk)
        && frame.self_state.alive != Some(false)
        && request.frame_revision <= frame.revision
        && request.strategic_revision == state.args.blackboard.strategic_revision()
        && !request.destination_scene.trim().is_empty();
    if !valid {
        let mission_id = uuid::Uuid::new_v4();
        record_navigation(
            state,
            request.decision_id,
            NavigationMissionTelemetry::Terminal {
                mission_id,
                recorded_at: Utc::now(),
                state: NavigationMissionState::Failed,
                reason_code: Some("navigation_request_rejected".to_owned()),
                scene: frame.self_state.scene.clone(),
                position_tile: frame.self_state.position.map(|position| position.tile),
                attempts: 0,
            },
        );
        return;
    }

    if let Some(existing) = state.navigation.as_ref()
        && existing.request.strategic_revision == request.strategic_revision
        && existing.request.destination_scene == request.destination_scene
        && existing.request.destination_tile == request.destination_tile
        && existing.request.destination_name == request.destination_name
        && existing.request.reason == request.reason
        && existing.request.route == request.route
    {
        record_navigation(
            state,
            request.decision_id,
            NavigationMissionTelemetry::DuplicateSuppressed {
                mission_id: existing.id,
                recorded_at: Utc::now(),
                strategic_revision: request.strategic_revision,
            },
        );
        return;
    }

    supersede_navigation(myself, state);
    let mission_id = uuid::Uuid::new_v4();
    record_navigation(
        state,
        request.decision_id,
        NavigationMissionTelemetry::Started {
            mission_id,
            recorded_at: Utc::now(),
            destination_scene: request.destination_scene.clone(),
            destination_tile: request.destination_tile,
            route_waypoints: request.route.len(),
        },
    );
    state.navigation = Some(ActiveNavigationMission {
        id: mission_id,
        request,
        state: if state.active.is_some() || state.movement_stop.is_some() {
            NavigationMissionState::Paused
        } else {
            NavigationMissionState::Active
        },
        waypoint_index: 0,
        attempt_number: 0,
        retry_pending: false,
        door_retry_index: 0,
        recovery_tile: None,
        in_flight: None,
    });
    if state.active.is_some() || state.movement_stop.is_some() {
        let mission = state.navigation.as_ref().expect("navigation installed");
        record_navigation(
            state,
            mission.request.decision_id,
            NavigationMissionTelemetry::Paused {
                mission_id,
                recorded_at: Utc::now(),
                reason_code: "tactical_action_active".to_owned(),
            },
        );
    } else {
        drive_navigation(myself, state);
    }
}

fn supersede_navigation(myself: &ActorRef<BodyMsg>, state: &mut BodyActorState) {
    let Some(mut previous) = state.navigation.take() else {
        return;
    };
    if let Some(action) = previous.in_flight.take() {
        begin_movement_stop(
            myself,
            &action,
            "navigation_superseded",
            action.context.frame_revision,
            state,
        );
    }
    let frame = state.args.blackboard.frame();
    record_navigation(
        state,
        previous.request.decision_id,
        NavigationMissionTelemetry::Terminal {
            mission_id: previous.id,
            recorded_at: Utc::now(),
            state: NavigationMissionState::Superseded,
            reason_code: Some("new_navigation_mission".to_owned()),
            scene: frame.self_state.scene.clone(),
            position_tile: frame.self_state.position.map(|position| position.tile),
            attempts: previous.attempt_number,
        },
    );
}

fn cancel_navigation(myself: &ActorRef<BodyMsg>, reason_code: &str, state: &mut BodyActorState) {
    let Some(mut mission) = state.navigation.take() else {
        return;
    };
    if let Some(action) = mission.in_flight.take() {
        begin_movement_stop(
            myself,
            &action,
            reason_code,
            action.context.frame_revision,
            state,
        );
    }
    navigation_terminal(
        mission,
        NavigationMissionState::Cancelled,
        Some(reason_code.to_owned()),
        state,
    );
}

fn pause_navigation(myself: &ActorRef<BodyMsg>, reason_code: &str, state: &mut BodyActorState) {
    let Some(mut mission) = state.navigation.take() else {
        return;
    };
    if mission.state == NavigationMissionState::Paused {
        state.navigation = Some(mission);
        return;
    }
    if let Some(action) = mission.in_flight.take() {
        begin_movement_stop(
            myself,
            &action,
            reason_code,
            action.context.frame_revision,
            state,
        );
    }
    mission.state = NavigationMissionState::Paused;
    record_navigation(
        state,
        mission.request.decision_id,
        NavigationMissionTelemetry::Paused {
            mission_id: mission.id,
            recorded_at: Utc::now(),
            reason_code: reason_code.to_owned(),
        },
    );
    state.navigation = Some(mission);
}

#[allow(
    clippy::too_many_lines,
    reason = "movement reduction and terminalization remain one atomic actor-state transition"
)]
fn handle_frame_update(
    myself: &ActorRef<BodyMsg>,
    frame: &Arc<crate::brain::tactical_frame::TacticalFrame>,
    state: &mut BodyActorState,
) {
    handle_navigation_frame(myself, frame, state);
    let mut movement_events = Vec::new();
    let mut movement_terminal = None;
    if let Some(action) = state
        .active
        .as_mut()
        .and_then(|active| active.in_flight.as_mut())
        && let Some(movement) = action.movement.as_mut()
    {
        let previous_state = movement.state;
        if movement.state == MovementState::Requested && frame.self_state.moving == Some(true) {
            movement.record_started(frame.generated_at);
        }
        movement.observe_frame(frame, MovementObservationRules::default());
        if movement
            .last_observation
            .as_ref()
            .is_some_and(|observation| observation.made_progress)
        {
            let observation = movement.last_observation.as_ref().expect("checked above");
            movement_events.push((
                action.context,
                MovementTelemetry::Progress {
                    observed_at: observation.observed_at,
                    frame_revision: frame.revision,
                    position_tile: observation.position.map(|position| position.tile),
                    distance_from_previous_millipixels: observation
                        .distance_from_previous_pixels
                        .map(millipixels),
                    observed_distance_millipixels: millipixels(movement.observed_distance_pixels),
                    remaining_tile_distance: observation.remaining_tile_distance,
                },
            ));
        }
        if previous_state != movement.state {
            match movement.state {
                MovementState::Arrived => {
                    let arrival = movement
                        .arrival
                        .as_ref()
                        .expect("arrived movement has fact");
                    movement_events.push((
                        action.context,
                        MovementTelemetry::Arrival {
                            observed_at: arrival.observed_at,
                            frame_revision: Some(frame.revision),
                            position_tile: arrival.position.map(|position| position.tile),
                            evidence: arrival.evidence,
                        },
                    ));
                    movement_terminal = Some((OutcomeStatus::Succeeded, None));
                }
                MovementState::Stalled => {
                    movement_events.push((
                        action.context,
                        MovementTelemetry::Stall {
                            observed_at: frame.generated_at,
                            frame_revision: Some(frame.revision),
                            position_tile: movement.latest_position.map(|position| position.tile),
                            observations_without_progress: movement.observations_without_progress,
                        },
                    ));
                    movement_terminal =
                        Some((OutcomeStatus::Failed, Some("movement_stalled".to_owned())));
                }
                MovementState::SceneTransition => {
                    let transition = movement
                        .scene_transition
                        .as_ref()
                        .expect("scene-transition movement has fact");
                    movement_events.push((
                        action.context,
                        MovementTelemetry::SceneTransition {
                            observed_at: transition.observed_at,
                            frame_revision: frame.revision,
                            from_scene: transition.from_scene.clone(),
                            to_scene: transition.to_scene.clone(),
                            position_tile: transition.position.map(|position| position.tile),
                        },
                    ));
                    // A scene transition is successful arrival only when this action was
                    // explicitly translated to the typed door operation. An unexpected
                    // scene change during an ordinary move is still useful telemetry, but
                    // the material-invalidation pass below must cancel that packet.
                    if action.kind == "enter_door" {
                        movement_terminal = Some((OutcomeStatus::Succeeded, None));
                    }
                }
                _ => {}
            }
        }
    }
    for (context, fact) in movement_events {
        record(state, TelemetryEvent::Movement { context, fact });
    }
    if let Some((status, reason_code)) = movement_terminal {
        finish_observed_movement(myself, status, reason_code, frame.revision, state);
        return;
    }
    let Some(active) = state.active.as_ref() else {
        return;
    };
    let mut remaining_packet = active.packet.clone();
    remaining_packet.proposal.actions = active.packet.proposal.actions[active
        .next_action_index
        .min(active.packet.proposal.actions.len())..]
        .to_vec();
    let current_intent = state.args.blackboard.strategy();
    let report =
        compare_material_state(&MaterialComparison {
            packet: &remaining_packet,
            accepted_frame: &active.accepted_frame,
            accepted_intent: &active.accepted_intent,
            current_frame: frame,
            current_intent: &current_intent,
            health_critical_at_or_below: None,
            execution: active.in_flight.as_ref().map_or_else(
                ExecutionValidityFacts::default,
                |action| {
                    action.movement.as_ref().map_or_else(
                        ExecutionValidityFacts::default,
                        |movement| ExecutionValidityFacts {
                            path_preflight: Some(movement.path_preflight.status),
                            movement_state: Some(movement.state),
                        },
                    )
                },
            ),
        });
    let integrity_invalidated = report.facts.iter().any(|fact| {
        !matches!(
            fact,
            MaterialInvalidationFact::NewHostile { .. }
                | MaterialInvalidationFact::HealthBecameCritical { .. }
        )
    });
    if integrity_invalidated || !report.triggered_abort_conditions.is_empty() {
        state.args.blackboard.invalidate_before(frame.revision);
        cancel_active(
            myself,
            ActionCancelReason::AbortCondition("material_invalidation".to_owned()),
            frame.revision,
            state,
        );
    }
}

#[derive(Debug, Clone, Copy)]
struct ResolvedNavigationTarget {
    tile: crate::world::TilePosition,
    enter_door: bool,
}

fn resolve_navigation_target(
    mission: &ActiveNavigationMission,
    frame: &crate::brain::tactical_frame::TacticalFrame,
) -> Result<Option<ResolvedNavigationTarget>, &'static str> {
    if let Some(tile) = mission.recovery_tile {
        return Ok(Some(ResolvedNavigationTarget {
            tile,
            enter_door: false,
        }));
    }
    let current_scene = frame.self_state.scene.as_deref();
    if let Some(waypoint) = mission.request.route.get(mission.waypoint_index) {
        if current_scene != Some(waypoint.scene.as_str()) {
            return Err("navigation_route_scene_mismatch");
        }
        // A routed door may occupy more than one tile. Once the requested
        // tile has failed, prefer another backend-reported exit for the same
        // destination before repeating the original pixel overlap.
        if let Some(destination_scene) = waypoint.transition_to_scene.as_deref() {
            let exits = frame
                .exits
                .iter()
                .filter(|exit| exit.destination_scene.as_deref() == Some(destination_scene))
                .collect::<Vec<_>>();
            if let Some(exit) = exits
                .get(mission.door_retry_index)
                .or_else(|| exits.first())
            {
                return Ok(Some(ResolvedNavigationTarget {
                    tile: exit.tile,
                    enter_door: true,
                }));
            }
        }
        return Ok(Some(ResolvedNavigationTarget {
            tile: waypoint.tile,
            enter_door: waypoint.transition_to_scene.is_some(),
        }));
    }
    if current_scene == Some(mission.request.destination_scene.as_str()) {
        return Ok(mission
            .request
            .destination_tile
            .map(|tile| ResolvedNavigationTarget {
                tile,
                enter_door: false,
            }));
    }
    let exits = frame
        .exits
        .iter()
        .filter(|exit| {
            exit.destination_scene.as_deref() == Some(mission.request.destination_scene.as_str())
        })
        .collect::<Vec<_>>();
    let exit = exits
        .get(mission.door_retry_index)
        .or_else(|| exits.first());
    exit.map_or(Err("navigation_route_unknown"), |exit| {
        Ok(Some(ResolvedNavigationTarget {
            tile: exit.tile,
            enter_door: true,
        }))
    })
}

fn drive_navigation(myself: &ActorRef<BodyMsg>, state: &mut BodyActorState) {
    if state.active.is_some() || state.movement_stop.is_some() {
        return;
    }
    let Some(mut mission) = state.navigation.take() else {
        return;
    };
    if mission.in_flight.is_some() {
        state.navigation = Some(mission);
        return;
    }
    if mission.state == NavigationMissionState::Paused {
        mission.state = NavigationMissionState::Active;
        let frame = state.args.blackboard.frame();
        record_navigation(
            state,
            mission.request.decision_id,
            NavigationMissionTelemetry::Resumed {
                mission_id: mission.id,
                recorded_at: Utc::now(),
                scene: frame.self_state.scene.clone(),
                attempt_number: mission.attempt_number,
            },
        );
    }
    let frame = state.args.blackboard.frame();
    if mission.request.strategic_revision != state.args.blackboard.strategic_revision() {
        navigation_terminal(
            mission,
            NavigationMissionState::Cancelled,
            Some("strategic_intent_changed".to_owned()),
            state,
        );
        return;
    }
    let target = match resolve_navigation_target(&mission, &frame) {
        Ok(Some(target)) => target,
        Ok(None) => {
            navigation_terminal(mission, NavigationMissionState::Arrived, None, state);
            return;
        }
        Err(reason) => {
            navigation_terminal(
                mission,
                NavigationMissionState::Failed,
                Some(reason.to_owned()),
                state,
            );
            return;
        }
    };
    if frame
        .self_state
        .position
        .is_some_and(|position| position.tile == target.tile)
        && !target.enter_door
    {
        navigation_waypoint_reached(myself, mission, false, state);
        return;
    }
    mission.attempt_number = mission.attempt_number.saturating_add(1);
    mission.retry_pending = false;
    let recovery_directional = mission.recovery_tile.is_some();
    state.navigation = Some(mission);
    if target.enter_door {
        dispatch_navigation_command(
            myself,
            BodyCommand::EnterDoor {
                destination: target.tile,
            },
            target.tile,
            NavigationAttemptKind::EnterDoor,
            true,
            state,
        );
    } else {
        let command = if recovery_directional {
            BodyCommand::MoveDirection {
                direction: crate::mcp::types::MoveDirection::Down,
            }
        } else {
            BodyCommand::MoveTo {
                destination: target.tile,
            }
        };
        dispatch_navigation_command(
            myself,
            command,
            target.tile,
            NavigationAttemptKind::MoveTo,
            true,
            state,
        );
    }
}

fn dispatch_navigation_command(
    myself: &ActorRef<BodyMsg>,
    command: BodyCommand,
    destination: crate::world::TilePosition,
    attempt_kind: NavigationAttemptKind,
    track_movement: bool,
    state: &mut BodyActorState,
) {
    let Some(mut mission) = state.navigation.take() else {
        return;
    };
    let frame = state.args.blackboard.frame();
    let context = ExecutionContext {
        session_generation: state.args.session_generation,
        decision_id: mission.request.decision_id,
        packet_id: mission.id,
        action_id: uuid::Uuid::new_v4(),
        action_index: mission.waypoint_index,
        frame_revision: frame.revision,
        strategic_revision: mission.request.strategic_revision,
    };
    let kind = command.kind().to_owned();
    let started_at = Utc::now();
    let movement = track_movement.then(|| {
        MovementProgress::new(
            MovementRequest {
                ownership: MovementOwnership {
                    movement_id: context.action_id,
                    decision_id: context.decision_id,
                    packet_id: context.packet_id,
                    action_index: context.action_index,
                },
                destination,
                requested_scene: frame.self_state.scene.clone(),
                requested_at: started_at,
                start_position: frame.self_state.position,
            },
            60_000,
        )
    });
    mission.in_flight = Some(ActiveAction {
        context,
        kind: kind.clone(),
        started_at,
        movement,
    });
    record_navigation(
        state,
        mission.request.decision_id,
        NavigationMissionTelemetry::AttemptStarted {
            mission_id: mission.id,
            attempt_id: context.action_id,
            recorded_at: started_at,
            attempt_number: mission.attempt_number,
            attempt_kind,
            scene: frame.self_state.scene.clone(),
            target_tile: destination,
        },
    );
    record(
        state,
        TelemetryEvent::ActionStarted {
            context,
            action_kind: kind.clone(),
        },
    );
    if track_movement {
        record_movement_request(
            state,
            context,
            started_at,
            frame.self_state.position.map(|position| position.tile),
            destination,
        );
    }
    state.navigation = Some(mission);
    let gateway = state.args.gateway.clone();
    let reply_to = myself.clone();
    tokio::spawn(async move {
        let started = Instant::now();
        let result = gateway.execute(command, context).await;
        let _ = reply_to.send_message(BodyMsg::NavigationActionCompleted(
            ActionExecutionCompleted {
                context,
                action_kind: kind,
                started_at,
                duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                result,
            },
        ));
    });
}

fn finish_observed_movement(
    myself: &ActorRef<BodyMsg>,
    status: OutcomeStatus,
    reason_code: Option<String>,
    resulting_frame_revision: u64,
    state: &mut BodyActorState,
) {
    let Some(mut active) = state.active.take() else {
        return;
    };
    let Some(action) = active.in_flight.take() else {
        state.active = Some(active);
        return;
    };
    if action.movement.is_none() {
        active.in_flight = Some(action);
        state.active = Some(active);
        return;
    }

    let duration_ms = u64::try_from((Utc::now() - action.started_at).num_milliseconds().max(0))
        .unwrap_or(u64::MAX);
    let destination_tile = action
        .movement
        .as_ref()
        .map(|movement| movement.request.destination);
    publish_outcome(
        state,
        action_outcome(
            &action.context,
            &action.kind,
            action.started_at,
            duration_ms,
            status,
            reason_code.clone(),
            resulting_frame_revision,
            destination_tile,
        ),
    );

    if status == OutcomeStatus::Succeeded {
        active.next_action_index += 1;
        state.active = Some(active);
        start_next_action(myself, state);
    } else {
        begin_movement_stop(
            myself,
            &action,
            reason_code.as_deref().unwrap_or("movement_failed"),
            resulting_frame_revision,
            state,
        );
        terminal(
            state,
            &active.packet,
            PacketTerminalStatus::Failed,
            reason_code,
            None,
        );
        clear_current(state);
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "validation, ownership, movement facts, and async dispatch form one atomic action-start boundary"
)]
fn start_next_action(myself: &ActorRef<BodyMsg>, state: &mut BodyActorState) {
    if state.movement_stop.is_some() {
        return;
    }
    let Some(mut active) = state.active.take() else {
        return;
    };
    if active.in_flight.is_some() {
        state.active = Some(active);
        return;
    }
    if active.next_action_index >= active.packet.proposal.actions.len() {
        terminal(
            state,
            &active.packet,
            PacketTerminalStatus::Completed,
            None,
            None,
        );
        clear_current(state);
        drive_navigation(myself, state);
        return;
    }

    let frame = state.args.blackboard.frame();
    let validation = validation_context(state, &frame);
    let action = &active.packet.proposal.actions[active.next_action_index];
    if let Err(error) = validate_packet_header(&active.packet, &validation)
        .and_then(|()| validate_action(action, &validation))
    {
        terminal(
            state,
            &active.packet,
            PacketTerminalStatus::Aborted,
            Some(validation_code(&error)),
            None,
        );
        clear_current(state);
        drive_navigation(myself, state);
        return;
    }

    // Room changes are strategic decisions. The fast brain may reposition,
    // flee, heal, and fight inside the current scene, but it must not turn a
    // tile-level move into an unsolicited door transition. Door missions are
    // issued only by the strategist through `PursueNavigation`.
    if matches!(action, TacticalAction::MoveTo { .. })
        && action
            .destination()
            .is_some_and(|destination| frame.exits.iter().any(|exit| exit.tile == destination))
    {
        terminal(
            state,
            &active.packet,
            PacketTerminalStatus::Aborted,
            Some("tactical_room_change_requires_strategist".to_owned()),
            None,
        );
        clear_current(state);
        drive_navigation(myself, state);
        return;
    }

    let context = ExecutionContext {
        session_generation: state.args.session_generation,
        decision_id: active.packet.decision_id,
        packet_id: active.packet.id,
        action_id: uuid::Uuid::new_v4(),
        action_index: active.next_action_index,
        frame_revision: frame.revision,
        strategic_revision: active.packet.strategic_revision,
    };
    let command = command_for(action, &frame);
    let kind = command.kind().to_owned();
    let started_at = Utc::now();
    let destination = action.destination();
    let movement = destination.map(|destination| {
        MovementProgress::new(
            MovementRequest {
                ownership: MovementOwnership {
                    movement_id: context.action_id,
                    decision_id: context.decision_id,
                    packet_id: context.packet_id,
                    action_index: context.action_index,
                },
                destination,
                requested_scene: frame.self_state.scene.clone(),
                requested_at: started_at,
                start_position: frame.self_state.position,
            },
            60_000,
        )
    });
    active.in_flight = Some(ActiveAction {
        context,
        kind: kind.clone(),
        started_at,
        movement,
    });
    state.active = Some(active);
    record(
        state,
        TelemetryEvent::ActionStarted {
            context,
            action_kind: kind.clone(),
        },
    );
    if let Some(destination_tile) = destination {
        record_movement_request(
            state,
            context,
            started_at,
            frame.self_state.position.map(|position| position.tile),
            destination_tile,
        );
    }

    let gateway = state.args.gateway.clone();
    let reply_to = myself.clone();
    tokio::spawn(async move {
        let started = Instant::now();
        let result = gateway.execute(command, context).await;
        let _ = reply_to.send_message(BodyMsg::ActionCompleted(ActionExecutionCompleted {
            context,
            action_kind: kind,
            started_at,
            duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            result,
        }));
    });
}

fn record_movement_request(
    state: &BodyActorState,
    context: ExecutionContext,
    requested_at: chrono::DateTime<Utc>,
    origin_tile: Option<crate::world::TilePosition>,
    destination_tile: crate::world::TilePosition,
) {
    record(
        state,
        TelemetryEvent::Movement {
            context,
            fact: MovementTelemetry::Requested {
                requested_at,
                origin_tile,
                destination_tile,
            },
        },
    );
}

#[allow(
    clippy::too_many_lines,
    reason = "MCP and perception completion cases share one auditable reconciliation point"
)]
fn finish_action(
    myself: &ActorRef<BodyMsg>,
    completed: &ActionExecutionCompleted,
    state: &mut BodyActorState,
) {
    let Some(mut active) = state.active.take() else {
        tracing::debug!(
            action_id = %completed.context.action_id,
            packet_id = %completed.context.packet_id,
            decision_id = %completed.context.decision_id,
            session_generation = completed.context.session_generation,
            action_kind = completed.action_kind,
            reason = "no_active_packet",
            "ignored orphaned body completion"
        );
        return;
    };
    if active
        .in_flight
        .as_ref()
        .is_none_or(|action| action.context.action_id != completed.context.action_id)
    {
        state.active = Some(active);
        tracing::debug!(
            action_id = %completed.context.action_id,
            packet_id = %completed.context.packet_id,
            decision_id = %completed.context.decision_id,
            session_generation = completed.context.session_generation,
            action_kind = completed.action_kind,
            reason = "action_id_mismatch",
            "ignored superseded body completion"
        );
        return;
    }
    if active
        .in_flight
        .as_ref()
        .is_some_and(|action| action.movement.is_some())
    {
        let now = Utc::now();
        let movement = active
            .in_flight
            .as_mut()
            .and_then(|action| action.movement.as_mut())
            .expect("movement checked above");
        let resolution = match &completed.result {
            Ok(result) => {
                movement.record_path_preflight(now, result.reachable, result.path_length_tiles);
                let backend_tile_matches =
                    result.tile_x.zip(result.tile_y).is_some_and(|(x, y)| {
                        x == movement.request.destination.x && y == movement.request.destination.y
                    });
                if result.arrived == Some(true) && backend_tile_matches {
                    movement.record_backend_arrival(now);
                    MovementCompletion::Succeeded
                } else if result.arrived == Some(true) {
                    // Some backend responses report a pixel-level arrival or
                    // came-to-rest before the requested tile is authoritative.
                    // Never close the mission on that weaker evidence.
                    movement.record_started(completed.started_at);
                    MovementCompletion::AwaitPerception
                } else if result.reachable == Some(false) {
                    MovementCompletion::Failed("path_unreachable".to_owned())
                } else if result.accepted == Some(false) {
                    MovementCompletion::Failed("backend_refused".to_owned())
                } else if result.arrived == Some(false)
                    && (result.came_to_rest == Some(true) || result.moving == Some(false))
                {
                    MovementCompletion::Stalled
                } else if result.accepted == Some(true)
                    || result.moved == Some(true)
                    || result.moving == Some(true)
                    || result.arrived == Some(false)
                {
                    movement.record_started(completed.started_at);
                    MovementCompletion::AwaitPerception
                } else {
                    MovementCompletion::Failed("movement_completion_unconfirmed".to_owned())
                }
            }
            Err(error) => MovementCompletion::Failed(error.class.clone()),
        };

        match resolution {
            MovementCompletion::AwaitPerception => {
                state.active = Some(active);
                return;
            }
            MovementCompletion::Succeeded => {
                let arrival = movement.arrival.as_ref().expect("backend arrival recorded");
                record(
                    state,
                    TelemetryEvent::Movement {
                        context: completed.context,
                        fact: MovementTelemetry::Arrival {
                            observed_at: arrival.observed_at,
                            frame_revision: None,
                            position_tile: arrival.position.map(|position| position.tile),
                            evidence: arrival.evidence,
                        },
                    },
                );
                complete_finished_action(
                    myself,
                    active,
                    completed,
                    OutcomeStatus::Succeeded,
                    None,
                    false,
                    state,
                );
                return;
            }
            MovementCompletion::Stalled => {
                record(
                    state,
                    TelemetryEvent::Movement {
                        context: completed.context,
                        fact: MovementTelemetry::Stall {
                            observed_at: now,
                            frame_revision: None,
                            position_tile: movement.latest_position.map(|position| position.tile),
                            observations_without_progress: movement.observations_without_progress,
                        },
                    },
                );
                complete_finished_action(
                    myself,
                    active,
                    completed,
                    OutcomeStatus::Failed,
                    Some("movement_stalled".to_owned()),
                    true,
                    state,
                );
                return;
            }
            MovementCompletion::Failed(reason_code) => {
                complete_finished_action(
                    myself,
                    active,
                    completed,
                    OutcomeStatus::Failed,
                    Some(reason_code),
                    false,
                    state,
                );
                return;
            }
        }
    }

    let command_only_acceptance = matches!(completed.action_kind.as_str(), "attack" | "use_skill");
    let (status, reason_code) = match &completed.result {
        Ok(result) if result.accepted != Some(false) && command_only_acceptance => {
            (OutcomeStatus::Accepted, None)
        }
        Ok(result) if result.accepted != Some(false) => (OutcomeStatus::Succeeded, None),
        Ok(_) => (OutcomeStatus::Failed, Some("backend_refused".to_owned())),
        Err(error) => (OutcomeStatus::Failed, Some(error.class.clone())),
    };
    let destination_tile = active
        .in_flight
        .as_ref()
        .and_then(|action| action.movement.as_ref())
        .map(|movement| movement.request.destination);
    active.in_flight = None;
    let outcome = action_outcome(
        &completed.context,
        &completed.action_kind,
        completed.started_at,
        completed.duration_ms,
        status,
        reason_code.clone(),
        state.args.blackboard.frame().revision,
        destination_tile,
    );
    publish_outcome(state, outcome);

    if matches!(status, OutcomeStatus::Succeeded | OutcomeStatus::Accepted) {
        active.next_action_index += 1;
        state.active = Some(active);
        start_next_action(myself, state);
    } else {
        terminal(
            state,
            &active.packet,
            PacketTerminalStatus::Failed,
            reason_code,
            None,
        );
        clear_current(state);
        drive_navigation(myself, state);
    }
}

enum MovementCompletion {
    AwaitPerception,
    Succeeded,
    Stalled,
    Failed(String),
}

#[allow(
    clippy::too_many_lines,
    reason = "preflight and movement completions share one auditable navigation reconciliation point"
)]
fn finish_navigation_action(
    myself: &ActorRef<BodyMsg>,
    completed: &ActionExecutionCompleted,
    state: &mut BodyActorState,
) {
    let Some(mut mission) = state.navigation.take() else {
        tracing::debug!(
            action_id = %completed.context.action_id,
            mission_id = %completed.context.packet_id,
            reason = "no_active_navigation_mission",
            "ignored orphaned navigation completion"
        );
        return;
    };
    let Some(action) = mission.in_flight.take() else {
        state.navigation = Some(mission);
        return;
    };
    if action.context.action_id != completed.context.action_id {
        mission.in_flight = Some(action);
        state.navigation = Some(mission);
        return;
    }

    let now = Utc::now();
    let resolution = match &completed.result {
        Ok(result)
            if result.arrived == Some(true)
                && action.movement.as_ref().is_some_and(|movement| {
                    result.tile_x.zip(result.tile_y).is_none_or(|(x, y)| {
                        x == movement.request.destination.x && y == movement.request.destination.y
                    })
                }) =>
        {
            MovementCompletion::Succeeded
        }
        Ok(result) if result.arrived == Some(true) => {
            // Pixel-level backend arrival is not authoritative for a tile
            // mission. Keep observing until the requested tile is reported.
            if let Some(movement) = action.movement.as_ref() {
                let mut movement = movement.clone();
                movement.record_started(completed.started_at);
                mission.in_flight = Some(ActiveAction {
                    movement: Some(movement),
                    ..action
                });
            }
            state.navigation = Some(mission);
            return;
        }
        Ok(result) if result.reachable == Some(false) => {
            MovementCompletion::Failed("path_unreachable".to_owned())
        }
        Ok(result) if result.accepted == Some(false) => {
            MovementCompletion::Failed("backend_refused".to_owned())
        }
        Ok(result)
            if result.arrived == Some(false)
                && (result.came_to_rest == Some(true) || result.moving == Some(false)) =>
        {
            MovementCompletion::Stalled
        }
        Ok(result)
            if result.accepted == Some(true)
                || result.moved == Some(true)
                || result.moving == Some(true)
                || result.arrived == Some(false) =>
        {
            if let Some(movement) = action.movement.as_ref() {
                let mut movement = movement.clone();
                movement.record_started(completed.started_at);
                mission.in_flight = Some(ActiveAction {
                    movement: Some(movement),
                    ..action
                });
            } else {
                mission.in_flight = Some(action);
            }
            state.navigation = Some(mission);
            return;
        }
        Ok(_) => MovementCompletion::Failed("movement_completion_unconfirmed".to_owned()),
        Err(error) => MovementCompletion::Failed(error.class.clone()),
    };

    match resolution {
        MovementCompletion::Succeeded => {
            publish_outcome(
                state,
                action_outcome(
                    &completed.context,
                    &completed.action_kind,
                    completed.started_at,
                    completed.duration_ms,
                    OutcomeStatus::Succeeded,
                    None,
                    state.args.blackboard.frame().revision,
                    action
                        .movement
                        .as_ref()
                        .map(|movement| movement.request.destination),
                ),
            );
            navigation_waypoint_reached(myself, mission, true, state);
        }
        MovementCompletion::Stalled => {
            publish_outcome(
                state,
                action_outcome(
                    &completed.context,
                    &completed.action_kind,
                    completed.started_at,
                    completed.duration_ms,
                    OutcomeStatus::Failed,
                    Some("movement_stalled".to_owned()),
                    state.args.blackboard.frame().revision,
                    action
                        .movement
                        .as_ref()
                        .map(|movement| movement.request.destination),
                ),
            );
            schedule_navigation_retry(myself, mission, action, "movement_stalled", now, state);
        }
        MovementCompletion::Failed(reason) => {
            publish_outcome(
                state,
                action_outcome(
                    &completed.context,
                    &completed.action_kind,
                    completed.started_at,
                    completed.duration_ms,
                    OutcomeStatus::Failed,
                    Some(reason.clone()),
                    state.args.blackboard.frame().revision,
                    action
                        .movement
                        .as_ref()
                        .map(|movement| movement.request.destination),
                ),
            );
            navigation_terminal(mission, NavigationMissionState::Failed, Some(reason), state);
        }
        MovementCompletion::AwaitPerception => unreachable!("handled before match"),
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "progress, arrival, transition, and stall facts are reduced atomically"
)]
fn handle_navigation_frame(
    myself: &ActorRef<BodyMsg>,
    frame: &Arc<crate::brain::tactical_frame::TacticalFrame>,
    state: &mut BodyActorState,
) {
    let Some(mut mission) = state.navigation.take() else {
        return;
    };
    if mission.request.strategic_revision != state.args.blackboard.strategic_revision() {
        if let Some(action) = mission.in_flight.take() {
            begin_movement_stop(
                myself,
                &action,
                "strategic_intent_changed",
                frame.revision,
                state,
            );
        }
        navigation_terminal(
            mission,
            NavigationMissionState::Cancelled,
            Some("strategic_intent_changed".to_owned()),
            state,
        );
        return;
    }
    if mission.state == NavigationMissionState::Paused || state.active.is_some() {
        state.navigation = Some(mission);
        return;
    }
    let (previous_state, movement_state, context, action_kind, progress_fact, from_scene, stalled) = {
        let Some(action) = mission.in_flight.as_mut() else {
            state.navigation = Some(mission);
            drive_navigation(myself, state);
            return;
        };
        let Some(movement) = action.movement.as_mut() else {
            state.navigation = Some(mission);
            return;
        };
        let previous_state = movement.state;
        movement.observe_frame(frame, MovementObservationRules::default());
        let progress_fact = movement.last_observation.as_ref().and_then(|observation| {
            observation
                .made_progress
                .then(|| MovementTelemetry::Progress {
                    observed_at: observation.observed_at,
                    frame_revision: frame.revision,
                    position_tile: observation.position.map(|position| position.tile),
                    distance_from_previous_millipixels: observation
                        .distance_from_previous_pixels
                        .map(millipixels),
                    observed_distance_millipixels: millipixels(movement.observed_distance_pixels),
                    remaining_tile_distance: observation.remaining_tile_distance,
                })
        });
        (
            previous_state,
            movement.state,
            action.context,
            action.kind.clone(),
            progress_fact,
            movement.request.requested_scene.clone(),
            movement.observations_without_progress,
        )
    };
    if let Some(fact) = progress_fact {
        record(state, TelemetryEvent::Movement { context, fact });
    }
    if previous_state == movement_state {
        state.navigation = Some(mission);
        return;
    }
    match movement_state {
        MovementState::Arrived => {
            let action = mission.in_flight.take().expect("active movement");
            publish_outcome(
                state,
                action_outcome(
                    &action.context,
                    &action.kind,
                    action.started_at,
                    u64::try_from((Utc::now() - action.started_at).num_milliseconds().max(0))
                        .unwrap_or(u64::MAX),
                    OutcomeStatus::Succeeded,
                    None,
                    frame.revision,
                    action
                        .movement
                        .as_ref()
                        .map(|value| value.request.destination),
                ),
            );
            navigation_waypoint_reached(myself, mission, false, state);
        }
        MovementState::SceneTransition if action_kind == "enter_door" => {
            let action = mission.in_flight.take().expect("active movement");
            record(
                state,
                TelemetryEvent::Movement {
                    context,
                    fact: MovementTelemetry::SceneTransition {
                        observed_at: frame.generated_at,
                        frame_revision: frame.revision,
                        from_scene,
                        to_scene: frame.self_state.scene.clone(),
                        position_tile: frame.self_state.position.map(|position| position.tile),
                    },
                },
            );
            publish_outcome(
                state,
                action_outcome(
                    &action.context,
                    &action.kind,
                    action.started_at,
                    u64::try_from((Utc::now() - action.started_at).num_milliseconds().max(0))
                        .unwrap_or(u64::MAX),
                    OutcomeStatus::Succeeded,
                    None,
                    frame.revision,
                    action
                        .movement
                        .as_ref()
                        .map(|value| value.request.destination),
                ),
            );
            navigation_waypoint_reached(myself, mission, false, state);
        }
        MovementState::Stalled => {
            let action = mission.in_flight.take().expect("active movement");
            record(
                state,
                TelemetryEvent::Movement {
                    context,
                    fact: MovementTelemetry::Stall {
                        observed_at: frame.generated_at,
                        frame_revision: Some(frame.revision),
                        position_tile: frame.self_state.position.map(|position| position.tile),
                        observations_without_progress: stalled,
                    },
                },
            );
            schedule_navigation_retry(
                myself,
                mission,
                action,
                "movement_stalled",
                frame.generated_at,
                state,
            );
        }
        _ => state.navigation = Some(mission),
    }
}

fn navigation_waypoint_reached(
    myself: &ActorRef<BodyMsg>,
    mut mission: ActiveNavigationMission,
    backend_confirmed_arrival: bool,
    state: &mut BodyActorState,
) {
    let frame = state.args.blackboard.frame();
    if mission.recovery_tile.is_some() {
        mission.recovery_tile = None;
        state.navigation = Some(mission);
        drive_navigation(myself, state);
        return;
    }
    let reached_index = mission.waypoint_index;
    if mission.waypoint_index < mission.request.route.len() {
        mission.waypoint_index += 1;
    }
    record_navigation(
        state,
        mission.request.decision_id,
        NavigationMissionTelemetry::WaypointReached {
            mission_id: mission.id,
            recorded_at: Utc::now(),
            waypoint_index: reached_index,
            scene: frame.self_state.scene.clone(),
            position_tile: frame.self_state.position.map(|position| position.tile),
        },
    );
    let at_destination_scene =
        frame.self_state.scene.as_deref() == Some(mission.request.destination_scene.as_str());
    let at_destination_tile = backend_confirmed_arrival
        || mission.request.destination_tile.is_none_or(|destination| {
            frame
                .self_state
                .position
                .is_some_and(|position| position.tile == destination)
        });
    if mission.waypoint_index >= mission.request.route.len()
        && at_destination_scene
        && at_destination_tile
    {
        navigation_terminal(mission, NavigationMissionState::Arrived, None, state);
    } else {
        state.navigation = Some(mission);
        drive_navigation(myself, state);
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "the caller transfers completed attempt ownership at this terminal boundary"
)]
fn schedule_navigation_retry(
    myself: &ActorRef<BodyMsg>,
    mut mission: ActiveNavigationMission,
    action: ActiveAction,
    reason_code: &str,
    recorded_at: DateTime<Utc>,
    state: &mut BodyActorState,
) {
    const MAX_NAVIGATION_ATTEMPTS: u32 = 3;
    if mission.attempt_number >= MAX_NAVIGATION_ATTEMPTS {
        navigation_terminal(
            mission,
            NavigationMissionState::Failed,
            Some("movement_stalled_after_local_retries".to_owned()),
            state,
        );
        return;
    }
    mission.retry_pending = true;
    if action.kind == "move_to" {
        let frame = state.args.blackboard.frame();
        let current = frame.self_state.position.map(|position| position.tile);
        let destination = mission.request.destination_tile;
        let mut candidates = current
            .map(crate::world::perception::cardinal_neighbors)
            .map_or_else(Vec::new, |neighbors| neighbors.into_iter().collect())
            // Prefer the tile immediately below, matching the common doorway
            // overlap in the Inn, then any walkable neighbor that approaches
            // the strategic destination.
            ;
        candidates.sort_by_key(|tile| {
            let down_bias = if current.is_some_and(|origin| tile.x == origin.x && tile.y > origin.y)
            {
                0_u8
            } else {
                1
            };
            let distance = destination.map_or(0, |goal| {
                tile.x
                    .abs_diff(goal.x)
                    .saturating_add(tile.y.abs_diff(goal.y))
            });
            (down_bias, distance)
        });
        let preferred_down = current.map(|origin| crate::world::TilePosition {
            x: origin.x,
            // Arena's tile Y axis increases toward the top of the map; this
            // is the engine's "down" direction.
            y: origin.y.saturating_sub(1),
        });
        mission.recovery_tile =
            preferred_down.filter(|tile| {
                !frame.map.tiles.iter().any(|candidate| {
                    candidate.position == *tile && candidate.walkable == Some(false)
                })
            });
        mission.recovery_tile = mission.recovery_tile.or_else(|| {
            candidates.into_iter().find(|tile| {
                frame.map.tiles.iter().any(|candidate| {
                    // Unknown collision data remains eligible for one bounded
                    // recovery attempt; only an explicit blocked tile is
                    // rejected. Door-adjacent backend payloads often omit it.
                    candidate.position == *tile && candidate.walkable != Some(false)
                })
            })
        });
        if mission.recovery_tile.is_none() {
            // If the map omitted the adjacent tile entirely, still make one
            // bounded directional-style attempt below the actor. The gateway
            // remains authoritative and will reject an actually invalid tile.
            mission.recovery_tile = current.map(|origin| crate::world::TilePosition {
                x: origin.x,
                y: origin.y.saturating_sub(1),
            });
        }
    }
    if action.kind == "enter_door" {
        // Prefer another doorway tile leading to the same scene on the next
        // attempt. This preserves destination-level intent while recovering
        // from multi-tile door/pixel overlap. If the backend exposes only one
        // tile, the existing bounded retry still applies to that tile.
        mission.door_retry_index = mission.door_retry_index.saturating_add(1);
    }
    record_navigation(
        state,
        mission.request.decision_id,
        NavigationMissionTelemetry::RetryScheduled {
            mission_id: mission.id,
            recorded_at,
            attempt_number: mission.attempt_number.saturating_add(1),
            reason_code: reason_code.to_owned(),
        },
    );
    state.navigation = Some(mission);
    begin_movement_stop(
        myself,
        &action,
        "navigation_local_retry",
        state.args.blackboard.frame().revision,
        state,
    );
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "terminalization deliberately consumes the removed actor-owned mission"
)]
fn navigation_terminal(
    mission: ActiveNavigationMission,
    terminal_state: NavigationMissionState,
    reason_code: Option<String>,
    state: &mut BodyActorState,
) {
    let frame = state.args.blackboard.frame();
    if terminal_state == NavigationMissionState::Arrived
        && let Some(perception) = &state.perception
    {
        let _ = perception.send_message(PerceptionMsg::NavigationArrived(NavigationArrival {
            mission_id: mission.id,
            decision_id: mission.request.decision_id,
            strategic_revision: mission.request.strategic_revision,
            destination_scene: mission.request.destination_scene.clone(),
            destination_tile: mission.request.destination_tile,
            destination_name: mission.request.destination_name.clone(),
            arrived_scene: frame.self_state.scene.clone(),
            arrived_tile: frame.self_state.position.map(|position| position.tile),
            attempts: mission.attempt_number,
        }));
    }
    if terminal_state == NavigationMissionState::Failed
        && let (Some(perception), Some(reason)) = (&state.perception, reason_code.as_ref())
    {
        let _ = perception.send_message(PerceptionMsg::NavigationBlocked {
            mission_id: mission.id,
            reason_code: reason.clone(),
            attempts: mission.attempt_number,
        });
    }
    record_navigation(
        state,
        mission.request.decision_id,
        NavigationMissionTelemetry::Terminal {
            mission_id: mission.id,
            recorded_at: Utc::now(),
            state: terminal_state,
            reason_code,
            scene: frame.self_state.scene.clone(),
            position_tile: frame.self_state.position.map(|position| position.tile),
            attempts: mission.attempt_number,
        },
    );
}

fn record_navigation(
    state: &BodyActorState,
    decision_id: uuid::Uuid,
    fact: NavigationMissionTelemetry,
) {
    record(
        state,
        TelemetryEvent::NavigationMission { decision_id, fact },
    );
}

fn complete_finished_action(
    myself: &ActorRef<BodyMsg>,
    mut active: ActivePacket,
    completed: &ActionExecutionCompleted,
    status: OutcomeStatus,
    reason_code: Option<String>,
    stop_movement: bool,
    state: &mut BodyActorState,
) {
    let action = active.in_flight.take().expect("matched in-flight action");
    let destination_tile = action
        .movement
        .as_ref()
        .map(|movement| movement.request.destination);
    publish_outcome(
        state,
        action_outcome(
            &completed.context,
            &completed.action_kind,
            completed.started_at,
            completed.duration_ms,
            status,
            reason_code.clone(),
            state.args.blackboard.frame().revision,
            destination_tile,
        ),
    );
    if status == OutcomeStatus::Succeeded {
        active.next_action_index += 1;
        state.active = Some(active);
        start_next_action(myself, state);
    } else {
        if stop_movement {
            begin_movement_stop(
                myself,
                &action,
                reason_code.as_deref().unwrap_or("movement_failed"),
                state.args.blackboard.frame().revision,
                state,
            );
        }
        terminal(
            state,
            &active.packet,
            PacketTerminalStatus::Failed,
            reason_code,
            None,
        );
        clear_current(state);
        drive_navigation(myself, state);
    }
}

fn supersede_active(
    myself: &ActorRef<BodyMsg>,
    superseded_by: uuid::Uuid,
    state: &mut BodyActorState,
) {
    let Some(previous) = state.active.take() else {
        return;
    };
    terminal(
        state,
        &previous.packet,
        PacketTerminalStatus::Superseded,
        Some("newer_packet".to_owned()),
        Some(superseded_by),
    );
    if let Some(action) = previous.in_flight {
        let outcome = action_outcome(
            &action.context,
            &action.kind,
            action.started_at,
            0,
            OutcomeStatus::Superseded,
            Some("newer_packet".to_owned()),
            state.args.blackboard.frame().revision,
            action
                .movement
                .as_ref()
                .map(|movement| movement.request.destination),
        );
        publish_outcome(state, outcome);
        let frame_revision = state.args.blackboard.frame().revision;
        begin_movement_stop(myself, &action, "newer_packet", frame_revision, state);
    }
}

fn cancel_active(
    myself: &ActorRef<BodyMsg>,
    reason: ActionCancelReason,
    frame_revision: u64,
    state: &mut BodyActorState,
) {
    let Some(active) = state.active.take() else {
        return;
    };
    let reason_code = match reason {
        ActionCancelReason::Preempted => "preempted".to_owned(),
        ActionCancelReason::AbortCondition(reason) => format!("abort:{reason}"),
        ActionCancelReason::Shutdown => "shutdown".to_owned(),
    };
    if let Some(action) = active.in_flight {
        let outcome = action_outcome(
            &action.context,
            &action.kind,
            action.started_at,
            0,
            OutcomeStatus::Cancelled,
            Some(reason_code.clone()),
            state.args.blackboard.frame().revision,
            action
                .movement
                .as_ref()
                .map(|movement| movement.request.destination),
        );
        publish_outcome(state, outcome);
        begin_movement_stop(myself, &action, &reason_code, frame_revision, state);
    }
    terminal(
        state,
        &active.packet,
        PacketTerminalStatus::Cancelled,
        Some(reason_code),
        None,
    );
    clear_current(state);
}

fn begin_movement_stop(
    myself: &ActorRef<BodyMsg>,
    interrupted_action: &ActiveAction,
    reason_code: &str,
    frame_revision: u64,
    state: &mut BodyActorState,
) {
    if !matches!(interrupted_action.kind.as_str(), "move_to" | "enter_door")
        || state.movement_stop.is_some()
        || interrupted_action
            .movement
            .as_ref()
            .is_some_and(|movement| movement.state == MovementState::Arrived)
    {
        return;
    }

    let context = ExecutionContext {
        action_id: uuid::Uuid::new_v4(),
        frame_revision,
        strategic_revision: state.args.blackboard.strategic_revision(),
        session_generation: state.args.session_generation,
        ..interrupted_action.context
    };
    let started_at = Utc::now();
    let action_kind = "movement_cancel_stop".to_owned();
    state.movement_stop = Some(ActiveMovementStop {
        context,
        started_at,
        reason_code: reason_code.to_owned(),
    });
    record(
        state,
        TelemetryEvent::ActionStarted {
            context,
            action_kind: action_kind.clone(),
        },
    );

    let gateway = state.args.gateway.clone();
    let reply_to = myself.clone();
    tokio::spawn(async move {
        let started = Instant::now();
        let result = gateway.execute(BodyCommand::Stop, context).await;
        let _ = reply_to.send_message(BodyMsg::MovementStopCompleted(ActionExecutionCompleted {
            context,
            action_kind,
            started_at,
            duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            result,
        }));
    });
}

fn finish_movement_stop(
    myself: &ActorRef<BodyMsg>,
    completed: ActionExecutionCompleted,
    state: &mut BodyActorState,
) {
    let Some(stop) = state.movement_stop.take() else {
        tracing::debug!(
            action_id = %completed.context.action_id,
            packet_id = %completed.context.packet_id,
            decision_id = %completed.context.decision_id,
            session_generation = completed.context.session_generation,
            reason = "no_active_movement_stop",
            "ignored orphaned movement stop completion"
        );
        return;
    };
    if stop.context.action_id != completed.context.action_id {
        state.movement_stop = Some(stop);
        tracing::debug!(
            action_id = %completed.context.action_id,
            packet_id = %completed.context.packet_id,
            decision_id = %completed.context.decision_id,
            session_generation = completed.context.session_generation,
            reason = "movement_stop_action_id_mismatch",
            "ignored superseded movement stop completion"
        );
        return;
    }

    let (status, backend_reason) = match completed.result {
        Ok(result) if result.accepted == Some(true) || result.stopped == Some(true) => {
            (OutcomeStatus::Succeeded, None)
        }
        Ok(_) => (OutcomeStatus::Failed, Some("backend_refused".to_owned())),
        Err(error) => (OutcomeStatus::Failed, Some(error.class)),
    };
    let reason_code = backend_reason.or(Some(stop.reason_code));
    record(
        state,
        TelemetryEvent::Movement {
            context: completed.context,
            fact: MovementTelemetry::Stop {
                recorded_at: Utc::now(),
                stop_action_id: completed.context.action_id,
                reason_code: reason_code.clone().unwrap_or_default(),
                succeeded: status == OutcomeStatus::Succeeded,
            },
        },
    );
    let outcome = action_outcome(
        &completed.context,
        &completed.action_kind,
        stop.started_at,
        completed.duration_ms,
        status,
        reason_code,
        state.args.blackboard.frame().revision,
        None,
    );
    publish_outcome(state, outcome);
    start_next_action(myself, state);
    drive_navigation(myself, state);
}

fn command_for(
    action: &TacticalAction,
    frame: &crate::brain::tactical_frame::TacticalFrame,
) -> BodyCommand {
    match action {
        TacticalAction::MoveTo { tile_x, tile_y } => {
            let destination = crate::world::TilePosition {
                x: *tile_x,
                y: *tile_y,
            };
            if frame.exits.iter().any(|exit| exit.tile == destination) {
                BodyCommand::EnterDoor { destination }
            } else {
                BodyCommand::MoveTo { destination }
            }
        }
        TacticalAction::Attack { target_id } => BodyCommand::Attack {
            target_object_index: target_id.clone(),
        },
        TacticalAction::UseSkill {
            skill_id,
            target_id,
        } => BodyCommand::UseSkill {
            skill_id: skill_id.clone(),
            target_object_index: target_id.clone(),
        },
        TacticalAction::UseItem { item_id } => BodyCommand::UseItem {
            item_id: item_id.clone(),
        },
        TacticalAction::PickUp { drop_id } => BodyCommand::PickUp {
            drop_id: drop_id.clone(),
        },
        TacticalAction::SetTactics { style, mode } => BodyCommand::SetTactics {
            style: *style,
            mode: *mode,
        },
        TacticalAction::Stop => BodyCommand::Stop,
    }
}

fn validation_context<'a>(
    state: &'a BodyActorState,
    frame: &'a crate::brain::tactical_frame::TacticalFrame,
) -> ValidationContext<'a> {
    ValidationContext {
        minimum_valid_frame_revision: state.args.blackboard.minimum_valid_frame_revision(),
        current_strategic_revision: state.args.blackboard.strategic_revision(),
        now: Utc::now(),
        capabilities: &state.args.character.capabilities,
        frame,
    }
}

fn validation_code(error: &crate::execution::validator::ActionRejected) -> String {
    match error {
        crate::execution::validator::ActionRejected::StaleFrame { .. } => "stale_frame",
        crate::execution::validator::ActionRejected::StaleStrategy { .. } => "stale_strategy",
        crate::execution::validator::ActionRejected::PlayerUnavailable => "player_unavailable",
        crate::execution::validator::ActionRejected::MissingCombatHealth => "missing_combat_health",
        crate::execution::validator::ActionRejected::MissingCapability(_) => "missing_capability",
        crate::execution::validator::ActionRejected::UnknownTarget(_) => "unknown_target",
        crate::execution::validator::ActionRejected::UnknownDrop(_) => "unknown_drop",
        crate::execution::validator::ActionRejected::UnavailableItem(_) => "unavailable_item",
        crate::execution::validator::ActionRejected::UnavailableSkill(_) => "unavailable_skill",
        crate::execution::validator::ActionRejected::UnknownDestination { .. } => {
            "unknown_destination"
        }
        crate::execution::validator::ActionRejected::BlockedDestination { .. } => {
            "blocked_destination"
        }
        crate::execution::validator::ActionRejected::StrategicNavigationOwnsMovement => {
            "strategic_navigation_owns_movement"
        }
        crate::execution::validator::ActionRejected::EmptyPacket => "empty_packet",
        crate::execution::validator::ActionRejected::InvalidLifetime => "invalid_lifetime",
        crate::execution::validator::ActionRejected::Expired => "expired",
        crate::execution::validator::ActionRejected::SceneChanged { .. } => "scene_changed",
    }
    .to_owned()
}

fn millipixels(pixels: f32) -> u64 {
    if pixels.is_finite() && pixels > 0.0 {
        (f64::from(pixels) * 1_000.0)
            .round()
            .to_u64()
            .unwrap_or(u64::MAX)
    } else {
        0
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "the audit record keeps causal, timing, terminal, and movement facts explicit"
)]
fn action_outcome(
    context: &ExecutionContext,
    action_kind: &str,
    started_at: DateTime<Utc>,
    duration_ms: u64,
    status: OutcomeStatus,
    reason_code: Option<String>,
    resulting_frame_revision: u64,
    destination_tile: Option<crate::world::TilePosition>,
) -> ActionOutcome {
    ActionOutcome {
        packet_id: context.packet_id,
        decision_id: context.decision_id,
        action_id: context.action_id,
        action_index: context.action_index,
        action_kind: action_kind.to_owned(),
        started_at,
        recorded_at: Utc::now(),
        duration_ms,
        status,
        detail: reason_code.clone().unwrap_or_else(|| match status {
            OutcomeStatus::Accepted => "mcp_command_accepted_effect_unconfirmed".to_owned(),
            _ => "mcp_operation_completed".to_owned(),
        }),
        destination_tile,
        reason_code,
        source_frame_revision: context.frame_revision,
        strategic_revision: context.strategic_revision,
        resulting_frame_revision: Some(resulting_frame_revision),
    }
}

fn publish_outcome(state: &BodyActorState, outcome: ActionOutcome) {
    record(
        state,
        TelemetryEvent::ActionTerminal {
            outcome: outcome.clone(),
            session_generation: state.args.session_generation,
        },
    );
    if let Some(perception) = &state.perception {
        let _ = perception.send_message(PerceptionMsg::ActionOutcome(outcome));
    }
}

fn terminal(
    state: &mut BodyActorState,
    packet: &ActionPacket,
    status: PacketTerminalStatus,
    reason_code: Option<String>,
    superseded_by: Option<uuid::Uuid>,
) {
    state.status.last_terminal_packet_id = Some(packet.id);
    state.status.last_terminal_status = Some(status);
    record(
        state,
        TelemetryEvent::PacketTerminal {
            packet_id: packet.id,
            decision_id: packet.decision_id,
            frame_revision: packet.frame_revision,
            strategic_revision: packet.strategic_revision,
            status,
            reason_code,
            superseded_by,
        },
    );
}

fn clear_current(state: &mut BodyActorState) {
    state.active = None;
    state.status.current_packet_id = None;
    state.args.blackboard.set_current_packet(None);
}

fn record(state: &BodyActorState, event: TelemetryEvent) {
    let _ = state
        .args
        .telemetry
        .send_message(TelemetryMsg::Record(event));
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use async_trait::async_trait;
    use ractor::Actor;

    use super::*;
    use crate::{
        actors::telemetry::{TelemetryActor, TelemetryActorArgs},
        brain::strategic_intent::StrategicIntent,
        config::HarnessConfig,
        execution::{
            gateway::{BodyCommandResult, BodyGatewayError},
            packet::{TacticalIntent, TacticalProposal},
        },
        observability::RecordingAnalyticsSink,
    };

    #[derive(Default)]
    struct RecordingGateway {
        calls: Mutex<Vec<(BodyCommand, ExecutionContext)>>,
        call_number: AtomicUsize,
        delay_first: bool,
        move_result: Option<BodyCommandResult>,
        move_error_class: Option<String>,
    }

    #[async_trait]
    impl BodyGateway for RecordingGateway {
        async fn execute(
            &self,
            command: BodyCommand,
            context: ExecutionContext,
        ) -> Result<BodyCommandResult, BodyGatewayError> {
            let is_move = matches!(
                command,
                BodyCommand::MoveTo { .. } | BodyCommand::EnterDoor { .. }
            );
            self.calls
                .lock()
                .expect("recording gateway lock")
                .push((command, context));
            let number = self.call_number.fetch_add(1, Ordering::SeqCst);
            if self.delay_first && number == 0 {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            if is_move {
                if let Some(class) = &self.move_error_class {
                    return Err(BodyGatewayError {
                        class: class.clone(),
                    });
                }
                if let Some(result) = &self.move_result {
                    return Ok(result.clone());
                }
            }
            Ok(BodyCommandResult {
                accepted: Some(true),
                stopped: Some(true),
                ..BodyCommandResult::default()
            })
        }
    }

    fn config() -> HarnessConfig {
        let values = HashMap::from([
            ("ARENA_API_KEY", "test-arena-key"),
            ("OPENROUTER_API_KEY", "test-router-key"),
        ]);
        HarnessConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
            .expect("test configuration")
    }

    fn blackboard() -> Arc<HotBlackboard> {
        let strategy = StrategicIntent {
            revision: 4,
            ..StrategicIntent::default()
        };
        let blackboard = Arc::new(HotBlackboard::new(strategy.clone()));
        let mut frame = crate::brain::tactical_frame::TacticalFrame::empty(strategy);
        frame.revision = 10;
        frame.perception_revision = 1;
        frame.self_state.alive = Some(true);
        frame.self_state.scene = Some("town".to_owned());
        frame.self_state.position = Some(crate::world::Position {
            pixel: crate::world::PixelPosition { x: 0.0, y: 0.0 },
            tile: crate::world::TilePosition { x: 0, y: 0 },
        });
        frame
            .nearby_entities
            .push(crate::brain::tactical_frame::VisibleEntity {
                id: "enemy-1".to_owned(),
                backend_object_id: Some(1),
                label: "Test Enemy".to_owned(),
                kind: crate::brain::tactical_frame::EntityKind::Enemy,
                tile: Some(crate::world::TilePosition { x: 1, y: 0 }),
                relative: Some(crate::world::TilePosition { x: 1, y: 0 }),
                distance: Some(1.0),
                alive: Some(true),
                is_merchant: None,
                interactable: Some(false),
                hostile: Some(true),
                targeting_you: Some(false),
            });
        frame.map.tiles = [
            crate::world::TilePosition { x: 2, y: 0 },
            crate::world::TilePosition { x: 4, y: 8 },
        ]
        .into_iter()
        .map(|position| crate::world::map::MapTile {
            position,
            kind: crate::world::map::TileKind::Traversable,
            walkable: Some(true),
        })
        .collect();
        assert!(blackboard.publish_frame(Arc::new(frame)));
        blackboard
    }

    fn blackboard_with_exit() -> Arc<HotBlackboard> {
        let blackboard = blackboard();
        let mut frame = blackboard.frame().as_ref().clone();
        frame.revision = 11;
        frame.perception_revision = 2;
        frame.map.tiles.push(crate::world::map::MapTile {
            position: crate::world::TilePosition { x: 9, y: 9 },
            kind: crate::world::map::TileKind::Door,
            walkable: Some(true),
        });
        frame.exits.push(crate::world::map::ReachableExit {
            tile: crate::world::TilePosition { x: 9, y: 9 },
            destination_scene: Some("forest".to_owned()),
            label: Some("forest door".to_owned()),
            path_length_tiles: 12,
        });
        assert!(blackboard.publish_frame(Arc::new(frame)));
        blackboard
    }

    fn packet(action_count: usize) -> ActionPacket {
        packet_with_actions(vec![TacticalAction::Stop; action_count])
    }

    fn packet_with_actions(actions: Vec<TacticalAction>) -> ActionPacket {
        ActionPacket::from_proposal(
            uuid::Uuid::new_v4(),
            10,
            4,
            Some("town".to_owned()),
            TacticalProposal {
                intent: TacticalIntent::Stop,
                actions,
                valid_for_ms: 10_000,
                abort_if: Vec::new(),
                rationale: None,
            },
        )
    }

    async fn spawn_body(
        gateway: Arc<RecordingGateway>,
    ) -> (
        ActorRef<BodyMsg>,
        ractor::concurrency::JoinHandle<()>,
        Arc<RecordingAnalyticsSink>,
    ) {
        spawn_body_with_blackboard(gateway, blackboard()).await
    }

    async fn spawn_body_with_blackboard(
        gateway: Arc<RecordingGateway>,
        blackboard: Arc<HotBlackboard>,
    ) -> (
        ActorRef<BodyMsg>,
        ractor::concurrency::JoinHandle<()>,
        Arc<RecordingAnalyticsSink>,
    ) {
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let (telemetry, _telemetry_join) = Actor::spawn(
            None,
            TelemetryActor,
            TelemetryActorArgs {
                character_id: "cassian".to_owned(),
                sink: analytics.clone(),
            },
        )
        .await
        .expect("telemetry starts");
        let character = Arc::new(
            crate::character::CharacterSheet::from_file(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("characters/cassian.json"),
                &config(),
            )
            .expect("Cassian sheet"),
        );
        let (body, join) = Actor::spawn(
            None,
            BodyActor,
            BodyActorArgs {
                character,
                blackboard,
                gateway,
                session_generation: 7,
                connected: true,
                telemetry,
            },
        )
        .await
        .expect("body starts");
        (body, join, analytics)
    }

    async fn wait_for_event(analytics: &RecordingAnalyticsSink, name: &str) {
        for _ in 0..100 {
            if analytics.events().iter().any(|event| event.name == name) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("timed out waiting for {name}");
    }

    async fn wait_for_calls(gateway: &RecordingGateway, count: usize) {
        for _ in 0..100 {
            if gateway.calls.lock().expect("calls").len() >= count {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("timed out waiting for {count} gateway calls");
    }

    fn movement_frame(
        revision: u64,
        pixel_x: f32,
        tile_x: i32,
        moving: bool,
    ) -> Arc<crate::brain::tactical_frame::TacticalFrame> {
        let strategy = StrategicIntent {
            revision: 4,
            ..StrategicIntent::default()
        };
        let mut frame = crate::brain::tactical_frame::TacticalFrame::empty(strategy);
        frame.revision = revision;
        frame.perception_revision = revision.saturating_sub(9);
        frame.self_state.alive = Some(true);
        frame.self_state.scene = Some("town".to_owned());
        frame.self_state.moving = Some(moving);
        frame.self_state.position = Some(crate::world::Position {
            pixel: crate::world::PixelPosition { x: pixel_x, y: 0.0 },
            tile: crate::world::TilePosition { x: tile_x, y: 0 },
        });
        Arc::new(frame)
    }

    #[tokio::test]
    async fn tactical_room_change_is_rejected_for_strategist_owned_doors() {
        let gateway = Arc::new(RecordingGateway {
            move_result: Some(BodyCommandResult {
                accepted: Some(true),
                moved: Some(true),
                arrived: Some(true),
                came_to_rest: Some(true),
                ..BodyCommandResult::default()
            }),
            ..RecordingGateway::default()
        });
        let blackboard = blackboard_with_exit();
        let (body, join, analytics) = spawn_body_with_blackboard(gateway.clone(), blackboard).await;
        let packet = ActionPacket::from_proposal(
            uuid::Uuid::new_v4(),
            11,
            4,
            Some("town".to_owned()),
            TacticalProposal {
                intent: TacticalIntent::Reposition,
                actions: vec![TacticalAction::MoveTo {
                    tile_x: 9,
                    tile_y: 9,
                }],
                valid_for_ms: 10_000,
                abort_if: Vec::new(),
                rationale: None,
            },
        );

        body.send_message(BodyMsg::ExecuteTactical(packet))
            .expect("packet sent");
        wait_for_event(&analytics, "body.packet_aborted").await;
        assert!(analytics.events().iter().any(|event| {
            event.name == "body.packet_aborted"
                && event.attributes["reason_code"] == "tactical_room_change_requires_strategist"
        }));
        assert!(gateway.calls.lock().expect("calls").is_empty());
        body.send_message(BodyMsg::Shutdown).expect("shutdown");
        join.await.expect("body joins");
    }

    #[tokio::test]
    async fn tactical_room_change_does_not_start_a_door_call() {
        let gateway = Arc::new(RecordingGateway {
            delay_first: true,
            move_result: Some(BodyCommandResult {
                accepted: Some(true),
                moved: Some(true),
                arrived: Some(true),
                ..BodyCommandResult::default()
            }),
            ..RecordingGateway::default()
        });
        let blackboard = blackboard_with_exit();
        let (body, join, analytics) =
            spawn_body_with_blackboard(gateway.clone(), blackboard.clone()).await;
        let packet = ActionPacket::from_proposal(
            uuid::Uuid::new_v4(),
            11,
            4,
            Some("town".to_owned()),
            TacticalProposal {
                intent: TacticalIntent::Reposition,
                actions: vec![TacticalAction::MoveTo {
                    tile_x: 9,
                    tile_y: 9,
                }],
                valid_for_ms: 10_000,
                abort_if: Vec::new(),
                rationale: None,
            },
        );
        body.send_message(BodyMsg::ExecuteTactical(packet))
            .expect("packet sent");
        wait_for_event(&analytics, "body.packet_aborted").await;
        assert!(gateway.calls.lock().expect("calls").is_empty());

        body.send_message(BodyMsg::Shutdown).expect("shutdown");
        join.await.expect("body joins");
    }

    #[tokio::test]
    async fn executes_packet_actions_in_order_with_one_causal_chain_each() {
        let gateway = Arc::new(RecordingGateway::default());
        let (body, join, analytics) = spawn_body(gateway.clone()).await;
        let packet = packet(2);
        let packet_id = packet.id;
        let decision_id = packet.decision_id;
        body.send_message(BodyMsg::ExecuteTactical(packet))
            .expect("packet sent");
        wait_for_event(&analytics, "body.packet_completed").await;

        let first_action_id = {
            let calls = gateway.calls.lock().expect("calls");
            assert_eq!(calls.len(), 2);
            for (index, (command, context)) in calls.iter().enumerate() {
                assert_eq!(command, &BodyCommand::Stop);
                assert_eq!(context.packet_id, packet_id);
                assert_eq!(context.decision_id, decision_id);
                assert_eq!(context.action_index, index);
                assert_eq!(context.session_generation, 7);
            }
            assert_ne!(calls[0].1.action_id, calls[1].1.action_id);
            calls[0].1.action_id
        };

        let events = analytics.events();
        let action_events = events
            .iter()
            .filter(|event| event.name == "body.action_succeeded")
            .collect::<Vec<_>>();
        assert_eq!(action_events.len(), 2);
        assert_eq!(action_events[0].correlation_id, Some(first_action_id));

        body.send_message(BodyMsg::Shutdown).expect("shutdown sent");
        join.await.expect("body stops");
    }

    #[tokio::test]
    async fn an_accepted_attack_is_not_reported_as_a_confirmed_success() {
        let gateway = Arc::new(RecordingGateway::default());
        let (body, join, analytics) = spawn_body(gateway).await;
        let packet = packet_with_actions(vec![TacticalAction::Attack {
            target_id: "enemy-1".to_owned(),
        }]);

        body.send_message(BodyMsg::ExecuteTactical(packet))
            .expect("packet sent");
        wait_for_event(&analytics, "body.packet_completed").await;

        let events = analytics.events();
        let accepted = events
            .iter()
            .find(|event| event.name == "body.action_accepted")
            .expect("accepted attack telemetry");
        assert_eq!(accepted.attributes["action_kind"], "attack");
        assert!(!events.iter().any(|event| {
            event.name == "body.action_succeeded" && event.attributes["action_kind"] == "attack"
        }));

        body.send_message(BodyMsg::Shutdown).expect("shutdown sent");
        join.await.expect("body stops");
    }

    #[tokio::test]
    async fn safety_fallback_uses_character_bound_flee_before_shutdown() {
        let gateway = Arc::new(RecordingGateway::default());
        let (body, join, analytics) = spawn_body(gateway.clone()).await;

        let result = ractor::call_t!(
            body,
            BodyMsg::ActivateSafetyFallback,
            1_000,
            "combat_health_unknown".to_owned()
        )
        .expect("body answers")
        .expect("fallback succeeds");

        assert_eq!(result.status, OutcomeStatus::Succeeded);
        {
            let calls = gateway.calls.lock().expect("calls");
            assert_eq!(calls.len(), 1);
            assert_eq!(
                calls[0].0,
                BodyCommand::SetTactics {
                    style: crate::execution::packet::TacticalStyle::Flee,
                    mode: crate::execution::packet::TacticalMode::SemiAuto,
                }
            );
            assert_eq!(calls[0].1.action_id, result.context.action_id);
        }
        wait_for_event(&analytics, "body.action_succeeded").await;
        let action = analytics
            .events()
            .into_iter()
            .find(|event| event.name == "body.action_succeeded")
            .expect("fallback action telemetry");
        assert_eq!(
            action.attributes["action_kind"],
            "safety_fallback_set_tactics_flee"
        );

        body.send_message(BodyMsg::Shutdown).expect("shutdown sent");
        join.await.expect("body stops");
    }

    #[tokio::test]
    async fn a_new_packet_supersedes_in_flight_work_without_blocking_the_mailbox() {
        let gateway = Arc::new(RecordingGateway {
            delay_first: true,
            ..RecordingGateway::default()
        });
        let (body, join, analytics) = spawn_body(gateway.clone()).await;
        let first = packet(2);
        let first_id = first.id;
        body.send_message(BodyMsg::ExecuteTactical(first))
            .expect("first packet sent");
        for _ in 0..100 {
            if !gateway.calls.lock().expect("calls").is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        let second = packet(1);
        let second_id = second.id;
        body.send_message(BodyMsg::ExecuteTactical(second))
            .expect("replacement packet sent");
        wait_for_event(&analytics, "body.packet_completed").await;

        let events = analytics.events();
        let superseded = events
            .iter()
            .find(|event| {
                event.name == "body.packet_superseded"
                    && event.attributes["packet_id"] == first_id.to_string()
            })
            .expect("old packet superseded");
        assert_eq!(
            superseded.attributes["superseded_by"],
            second_id.to_string()
        );
        assert!(events.iter().any(|event| {
            event.name == "body.packet_completed"
                && event.attributes["packet_id"] == second_id.to_string()
        }));
        assert_eq!(gateway.calls.lock().expect("calls").len(), 2);

        body.send_message(BodyMsg::Shutdown).expect("shutdown sent");
        join.await.expect("body stops");
    }

    #[tokio::test]
    async fn a_material_frame_change_cancels_remaining_packet_actions() {
        let gateway = Arc::new(RecordingGateway {
            delay_first: true,
            ..RecordingGateway::default()
        });
        let (body, join, analytics) = spawn_body(gateway.clone()).await;
        let packet = packet(2);
        let packet_id = packet.id;
        body.send_message(BodyMsg::ExecuteTactical(packet))
            .expect("packet sent");
        for _ in 0..100 {
            if !gateway.calls.lock().expect("calls").is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        let strategy = StrategicIntent {
            revision: 4,
            ..StrategicIntent::default()
        };
        let mut changed = crate::brain::tactical_frame::TacticalFrame::empty(strategy);
        changed.revision = 11;
        changed.perception_revision = 2;
        changed.self_state.alive = Some(true);
        changed.self_state.scene = Some("forest".to_owned());
        body.send_message(BodyMsg::FrameUpdated(Arc::new(changed)))
            .expect("changed frame sent");
        wait_for_event(&analytics, "body.packet_cancelled").await;
        tokio::time::sleep(Duration::from_millis(125)).await;

        assert_eq!(gateway.calls.lock().expect("calls").len(), 1);
        assert!(analytics.events().iter().any(|event| {
            event.name == "body.packet_cancelled"
                && event.attributes["packet_id"] == packet_id.to_string()
                && event.attributes["reason_code"] == "abort:material_invalidation"
        }));

        body.send_message(BodyMsg::Shutdown).expect("shutdown sent");
        join.await.expect("body stops");
    }

    #[tokio::test]
    async fn active_move_is_stopped_before_replacement_packet_runs() {
        let gateway = Arc::new(RecordingGateway {
            delay_first: true,
            ..RecordingGateway::default()
        });
        let (body, join, analytics) = spawn_body(gateway.clone()).await;
        let moving = packet_with_actions(vec![TacticalAction::MoveTo {
            tile_x: 4,
            tile_y: 8,
        }]);
        let moving_packet_id = moving.id;
        let moving_decision_id = moving.decision_id;
        body.send_message(BodyMsg::ExecuteTactical(moving))
            .expect("moving packet sent");
        wait_for_calls(&gateway, 1).await;

        let replacement = packet(1);
        let replacement_packet_id = replacement.id;
        body.send_message(BodyMsg::ExecuteTactical(replacement))
            .expect("replacement packet sent");
        wait_for_event(&analytics, "body.packet_completed").await;

        let cancellation_stop_id = {
            let calls = gateway.calls.lock().expect("calls");
            assert_eq!(calls.len(), 3);
            assert!(matches!(calls[0].0, BodyCommand::MoveTo { .. }));
            assert_eq!(calls[1].0, BodyCommand::Stop);
            assert_eq!(calls[2].0, BodyCommand::Stop);
            assert_eq!(calls[1].1.packet_id, moving_packet_id);
            assert_eq!(calls[1].1.decision_id, moving_decision_id);
            assert_eq!(calls[1].1.action_index, calls[0].1.action_index);
            assert_eq!(calls[1].1.session_generation, calls[0].1.session_generation);
            assert_ne!(calls[1].1.action_id, calls[0].1.action_id);
            assert_eq!(calls[2].1.packet_id, replacement_packet_id);
            calls[1].1.action_id
        };

        assert!(analytics.events().iter().any(|event| {
            event.name == "body.action_succeeded"
                && event.correlation_id == Some(cancellation_stop_id)
                && event.attributes["action_kind"] == "movement_cancel_stop"
                && event.attributes["reason_code"] == "newer_packet"
        }));

        body.send_message(BodyMsg::Shutdown).expect("shutdown sent");
        join.await.expect("body stops");
    }

    #[tokio::test]
    async fn late_move_completion_is_ignored_after_preemption() {
        let gateway = Arc::new(RecordingGateway {
            delay_first: true,
            ..RecordingGateway::default()
        });
        let (body, join, analytics) = spawn_body(gateway.clone()).await;
        let moving = packet_with_actions(vec![
            TacticalAction::MoveTo {
                tile_x: 4,
                tile_y: 8,
            },
            TacticalAction::Stop,
        ]);
        body.send_message(BodyMsg::ExecuteTactical(moving))
            .expect("moving packet sent");
        wait_for_calls(&gateway, 1).await;
        let move_action_id = gateway.calls.lock().expect("calls")[0].1.action_id;

        body.send_message(BodyMsg::ExecuteTactical(packet(1)))
            .expect("replacement packet sent");
        wait_for_event(&analytics, "body.packet_completed").await;
        tokio::time::sleep(Duration::from_millis(125)).await;

        let events = analytics.events();
        assert!(events.iter().any(|event| {
            event.name == "body.action_superseded" && event.correlation_id == Some(move_action_id)
        }));
        assert!(!events.iter().any(|event| {
            event.name == "body.action_succeeded" && event.correlation_id == Some(move_action_id)
        }));
        assert_eq!(gateway.calls.lock().expect("calls").len(), 3);

        body.send_message(BodyMsg::Shutdown).expect("shutdown sent");
        join.await.expect("body stops");
    }

    #[tokio::test]
    async fn material_invalidation_stops_move_without_recovery_escalation() {
        let gateway = Arc::new(RecordingGateway {
            delay_first: true,
            ..RecordingGateway::default()
        });
        let (body, join, analytics) = spawn_body(gateway.clone()).await;
        let moving = packet_with_actions(vec![TacticalAction::MoveTo {
            tile_x: 4,
            tile_y: 8,
        }]);
        body.send_message(BodyMsg::ExecuteTactical(moving))
            .expect("moving packet sent");
        wait_for_calls(&gateway, 1).await;

        let strategy = StrategicIntent {
            revision: 4,
            ..StrategicIntent::default()
        };
        let mut changed = crate::brain::tactical_frame::TacticalFrame::empty(strategy);
        changed.revision = 11;
        changed.perception_revision = 2;
        changed.self_state.alive = Some(true);
        changed.self_state.scene = Some("forest".to_owned());
        body.send_message(BodyMsg::FrameUpdated(Arc::new(changed)))
            .expect("changed frame sent");
        wait_for_calls(&gateway, 2).await;
        wait_for_event(&analytics, "body.packet_cancelled").await;
        tokio::time::sleep(Duration::from_millis(125)).await;

        {
            let calls = gateway.calls.lock().expect("calls");
            assert_eq!(calls.len(), 2, "preemption must not add recovery calls");
            assert!(matches!(calls[0].0, BodyCommand::MoveTo { .. }));
            assert_eq!(calls[1].0, BodyCommand::Stop);
        }
        assert!(analytics.events().iter().any(|event| {
            event.name == "body.action_succeeded"
                && event.attributes["action_kind"] == "movement_cancel_stop"
                && event.attributes["reason_code"] == "abort:material_invalidation"
        }));

        body.send_message(BodyMsg::Shutdown).expect("shutdown sent");
        join.await.expect("body stops");
    }

    #[tokio::test]
    async fn partial_move_completion_does_not_advance_before_arrival() {
        let gateway = Arc::new(RecordingGateway {
            move_result: Some(BodyCommandResult {
                accepted: Some(true),
                moved: Some(true),
                moving: Some(true),
                arrived: Some(false),
                ..BodyCommandResult::default()
            }),
            ..RecordingGateway::default()
        });
        let (body, join, analytics) = spawn_body(gateway.clone()).await;
        let moving = packet_with_actions(vec![
            TacticalAction::MoveTo {
                tile_x: 2,
                tile_y: 0,
            },
            TacticalAction::Stop,
        ]);
        body.send_message(BodyMsg::ExecuteTactical(moving))
            .expect("packet sent");
        wait_for_calls(&gateway, 1).await;
        body.send_message(BodyMsg::FrameUpdated(movement_frame(11, 16.0, 1, true)))
            .expect("progress frame sent");
        wait_for_event(&analytics, "body.movement_progress").await;

        assert_eq!(gateway.calls.lock().expect("calls").len(), 1);
        assert!(!analytics.events().iter().any(|event| {
            event.name == "body.action_succeeded" && event.attributes["action_kind"] == "move_to"
        }));
        assert!(
            !analytics
                .events()
                .iter()
                .any(|event| event.name == "body.packet_completed")
        );

        body.send_message(BodyMsg::Shutdown).expect("shutdown sent");
        join.await.expect("body stops");
    }

    #[tokio::test]
    async fn perception_arrival_is_defensible_success_and_advances_packet() {
        let gateway = Arc::new(RecordingGateway {
            move_result: Some(BodyCommandResult {
                accepted: Some(true),
                moved: Some(true),
                moving: Some(true),
                arrived: Some(false),
                ..BodyCommandResult::default()
            }),
            ..RecordingGateway::default()
        });
        let (body, join, analytics) = spawn_body(gateway.clone()).await;
        body.send_message(BodyMsg::ExecuteTactical(packet_with_actions(vec![
            TacticalAction::MoveTo {
                tile_x: 2,
                tile_y: 0,
            },
            TacticalAction::Stop,
        ])))
        .expect("packet sent");
        wait_for_calls(&gateway, 1).await;
        body.send_message(BodyMsg::FrameUpdated(movement_frame(11, 16.0, 1, true)))
            .expect("progress frame sent");
        body.send_message(BodyMsg::FrameUpdated(movement_frame(12, 32.0, 2, false)))
            .expect("arrival frame sent");
        wait_for_event(&analytics, "body.packet_completed").await;

        assert_eq!(gateway.calls.lock().expect("calls").len(), 2);
        assert!(analytics.events().iter().any(|event| {
            event.name == "body.movement_arrival"
                && event.attributes["evidence"] == "perception_frame"
        }));
        assert!(analytics.events().iter().any(|event| {
            event.name == "body.action_succeeded" && event.attributes["action_kind"] == "move_to"
        }));

        body.send_message(BodyMsg::Shutdown).expect("shutdown sent");
        join.await.expect("body stops");
    }

    #[tokio::test]
    async fn explicit_move_failure_fails_packet_without_advancing() {
        let gateway = Arc::new(RecordingGateway {
            move_result: Some(BodyCommandResult {
                accepted: Some(false),
                arrived: Some(false),
                ..BodyCommandResult::default()
            }),
            ..RecordingGateway::default()
        });
        let (body, join, analytics) = spawn_body(gateway.clone()).await;
        body.send_message(BodyMsg::ExecuteTactical(packet_with_actions(vec![
            TacticalAction::MoveTo {
                tile_x: 2,
                tile_y: 0,
            },
            TacticalAction::Stop,
        ])))
        .expect("packet sent");
        wait_for_event(&analytics, "body.packet_failed").await;

        assert_eq!(gateway.calls.lock().expect("calls").len(), 1);
        assert!(analytics.events().iter().any(|event| {
            event.name == "body.action_failed"
                && event.attributes["action_kind"] == "move_to"
                && event.attributes["reason_code"] == "backend_refused"
        }));

        body.send_message(BodyMsg::Shutdown).expect("shutdown sent");
        join.await.expect("body stops");
    }

    #[tokio::test]
    async fn perception_stall_stops_once_and_ignores_concurrent_move_completion() {
        let gateway = Arc::new(RecordingGateway {
            delay_first: true,
            move_result: Some(BodyCommandResult {
                accepted: Some(true),
                moved: Some(true),
                moving: Some(true),
                arrived: Some(false),
                ..BodyCommandResult::default()
            }),
            ..RecordingGateway::default()
        });
        let (body, join, analytics) = spawn_body(gateway.clone()).await;
        body.send_message(BodyMsg::ExecuteTactical(packet_with_actions(vec![
            TacticalAction::MoveTo {
                tile_x: 2,
                tile_y: 0,
            },
            TacticalAction::Stop,
        ])))
        .expect("packet sent");
        wait_for_calls(&gateway, 1).await;
        for revision in 11..=13 {
            body.send_message(BodyMsg::FrameUpdated(movement_frame(
                revision, 0.0, 0, true,
            )))
            .expect("stalled frame sent");
        }
        wait_for_event(&analytics, "body.movement_stop").await;
        tokio::time::sleep(Duration::from_millis(125)).await;

        {
            let calls = gateway.calls.lock().expect("calls");
            assert_eq!(calls.len(), 2, "only move and its safety stop may execute");
            assert!(matches!(calls[0].0, BodyCommand::MoveTo { .. }));
            assert_eq!(calls[1].0, BodyCommand::Stop);
        }
        let events = analytics.events();
        assert_eq!(
            events
                .iter()
                .filter(|event| {
                    event.name == "body.action_failed"
                        && event.attributes["action_kind"] == "move_to"
                })
                .count(),
            1
        );
        assert!(!events.iter().any(|event| {
            event.name == "body.action_succeeded" && event.attributes["action_kind"] == "move_to"
        }));

        body.send_message(BodyMsg::Shutdown).expect("shutdown sent");
        join.await.expect("body stops");
    }

    fn navigation_request() -> NavigationMissionRequest {
        NavigationMissionRequest {
            decision_id: uuid::Uuid::new_v4(),
            frame_revision: 10,
            strategic_revision: 4,
            destination_scene: "town".to_owned(),
            destination_tile: Some(crate::world::TilePosition { x: 2, y: 0 }),
            destination_name: "town square".to_owned(),
            reason: "meet the merchant".to_owned(),
            route: Vec::new(),
        }
    }

    #[tokio::test]
    async fn destination_mission_uses_the_gateway_move_to_seam() {
        let gateway = Arc::new(RecordingGateway {
            move_result: Some(BodyCommandResult {
                accepted: Some(true),
                moved: Some(true),
                arrived: Some(true),
                came_to_rest: Some(true),
                ..BodyCommandResult::default()
            }),
            ..RecordingGateway::default()
        });
        let (body, join, analytics) = spawn_body(gateway.clone()).await;
        body.send_message(BodyMsg::PursueNavigation(navigation_request()))
            .expect("mission sent");
        wait_for_event(&analytics, "body.navigation_mission_terminal").await;

        {
            let calls = gateway.calls.lock().expect("calls");
            assert!(matches!(
                calls.as_slice(),
                [(BodyCommand::MoveTo { destination }, _)]
                    if *destination == crate::world::TilePosition { x: 2, y: 0 }
            ));
        }
        let terminal = analytics
            .events()
            .into_iter()
            .find(|event| event.name == "body.navigation_mission_terminal")
            .expect("terminal mission event");
        assert_eq!(terminal.attributes["state"], "arrived");

        body.send_message(BodyMsg::Shutdown).expect("shutdown sent");
        join.await.expect("body stops");
    }

    #[tokio::test]
    async fn identical_navigation_mission_is_idempotent() {
        let gateway = Arc::new(RecordingGateway {
            delay_first: true,
            move_result: Some(BodyCommandResult {
                accepted: Some(true),
                moving: Some(true),
                arrived: Some(false),
                ..BodyCommandResult::default()
            }),
            ..RecordingGateway::default()
        });
        let (body, join, analytics) = spawn_body(gateway.clone()).await;
        let first = navigation_request();
        let duplicate = NavigationMissionRequest {
            decision_id: uuid::Uuid::new_v4(),
            ..first.clone()
        };
        body.send_message(BodyMsg::PursueNavigation(first))
            .expect("first mission sent");
        wait_for_calls(&gateway, 1).await;
        body.send_message(BodyMsg::PursueNavigation(duplicate))
            .expect("duplicate mission sent");
        wait_for_event(&analytics, "body.navigation_duplicate_suppressed").await;

        assert_eq!(gateway.calls.lock().expect("calls").len(), 1);
        assert!(
            !analytics
                .events()
                .iter()
                .any(|event| event.name == "body.navigation_mission_terminal")
        );

        body.send_message(BodyMsg::Shutdown).expect("shutdown sent");
        join.await.expect("body stops");
    }
}
