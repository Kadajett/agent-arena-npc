use std::{collections::HashSet, sync::Arc};

use ractor::RpcReplyPort;

use crate::{
    brain::{
        Brain,
        strategic_input::StrategicInput,
        strategic_intent::StrategicIntent,
        strategic_output::{StrategicProposal, StrategicWorkingUpdate},
        tactical_frame::TacticalFrame,
    },
    execution::{
        gateway::{BodyCommandResult, BodyGatewayError, ExecutionContext},
        movement::{
            MovementTelemetry, NavigationArrival, NavigationMissionRequest,
            NavigationMissionTelemetry,
        },
        outcome::{ActionOutcome, PacketTerminalStatus},
        packet::{ActionPacket, TacticalProposal},
    },
    memory::{
        recall::{RecallQuery, StrategicRecall},
        relationships::RelationshipUpdate,
        working::{PlanProgressUpdate, WorkingMemory},
    },
    world::{
        episodes::EpisodeSummary,
        events::GameEvent,
        perception::{PerceptionInput, PerceptionSummary},
    },
};

use super::control_gate::{
    ControlledPacketError, ControlledPacketReceipt, ControlledPacketRequest,
};
use super::tactical_schedule::{
    DeferralReason, PacketRelease, SuppressionReason, TacticalActivity, TacticalWakeReason,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActorKind {
    Body,
    Perception,
    Tactician,
    Strategist,
    Memory,
    Telemetry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerRuntimeStatus {
    pub character_id: String,
    pub running: HashSet<ActorKind>,
    pub failures_observed: u64,
}

impl PlayerRuntimeStatus {
    pub fn is_running(&self, actor: ActorKind) -> bool {
        self.running.contains(&actor)
    }
}

pub enum PlayerSupervisorMsg {
    PerceptionInput(Box<PerceptionInput>),
    SessionInvalidated {
        generation: u64,
        reason: String,
    },
    Health(RpcReplyPort<PlayerRuntimeStatus>),
    BodyHealth(RpcReplyPort<BodyStatus>),
    TelemetryHealth(RpcReplyPort<TelemetrySnapshot>),
    ValidateControlledPacket(
        ControlledPacketRequest,
        RpcReplyPort<Result<ControlledPacketReceipt, ControlledPacketError>>,
    ),
    SubmitControlledPacket(
        ControlledPacketRequest,
        RpcReplyPort<Result<ControlledPacketReceipt, ControlledPacketError>>,
    ),
    SubmitModelPacket(ActionPacket),
    ActivateSafetyFallback(
        String,
        RpcReplyPort<Result<SafetyFallbackResult, BodyGatewayError>>,
    ),
    #[cfg(test)]
    FailTacticianForTest,
    Shutdown,
}

pub enum BodyMsg {
    Think(StrategicThoughtRequest),
    Speak(StrategicSpeechRequest),
    SpeechCompleted(StrategicSpeechCompleted),
    Interact(StrategicInteractionRequest),
    InteractionCompleted(StrategicInteractionCompleted),
    QueueDuel(StrategicDuelRequest),
    DuelQueued(StrategicDuelCompleted),
    ExecuteTactical(ActionPacket),
    PursueNavigation(NavigationMissionRequest),
    NavigationActionCompleted(ActionExecutionCompleted),
    ValidateTactical(
        ActionPacket,
        RpcReplyPort<Result<(), crate::execution::validator::ActionRejected>>,
    ),
    ActionCompleted(ActionExecutionCompleted),
    MovementStopCompleted(ActionExecutionCompleted),
    ActivateSafetyFallback(
        String,
        RpcReplyPort<Result<SafetyFallbackResult, BodyGatewayError>>,
    ),
    SafetyFallbackCompleted(SafetyFallbackCompleted),
    FrameUpdated(Arc<TacticalFrame>),
    SessionGenerationChanged(u64),
    CancelCurrentAction(ActionCancelReason),
    ReplacePerception(ractor::ActorRef<PerceptionMsg>),
    Health(RpcReplyPort<BodyStatus>),
    Shutdown,
}

/// A private spectator-visible thought emitted before a strategic body action.
/// The text is reduced model output, never a chain-of-thought transcript.
#[derive(Debug, Clone)]
pub struct StrategicThoughtRequest {
    pub decision_id: uuid::Uuid,
    pub frame_revision: u64,
    pub strategic_revision: u64,
    pub thought: String,
}

#[derive(Debug, Clone)]
pub struct StrategicSpeechRequest {
    pub decision_id: uuid::Uuid,
    pub frame_revision: u64,
    pub strategic_revision: u64,
    pub message: String,
    pub channel: crate::execution::gateway::BodySpeechChannel,
    pub to_player: Option<String>,
}

#[derive(Debug)]
pub struct StrategicSpeechCompleted {
    pub context: ExecutionContext,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub duration_ms: u64,
    pub result: Result<BodyCommandResult, BodyGatewayError>,
}

#[derive(Debug, Clone)]
pub struct StrategicInteractionRequest {
    pub decision_id: uuid::Uuid,
    pub frame_revision: u64,
    pub strategic_revision: u64,
    pub target_id: String,
}

#[derive(Debug)]
pub struct StrategicInteractionCompleted {
    pub context: ExecutionContext,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub duration_ms: u64,
    pub result: Result<BodyCommandResult, BodyGatewayError>,
}

#[derive(Debug, Clone)]
pub struct StrategicDuelRequest {
    pub decision_id: uuid::Uuid,
    pub frame_revision: u64,
    pub strategic_revision: u64,
}

#[derive(Debug)]
pub struct StrategicDuelCompleted {
    pub context: ExecutionContext,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub duration_ms: u64,
    pub result: Result<BodyCommandResult, BodyGatewayError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafetyFallbackResult {
    pub context: ExecutionContext,
    pub duration_ms: u64,
    pub status: crate::execution::outcome::OutcomeStatus,
    pub reason_code: Option<String>,
}

pub struct SafetyFallbackCompleted {
    pub context: ExecutionContext,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub duration_ms: u64,
    pub reason_code: String,
    pub result: Result<BodyCommandResult, BodyGatewayError>,
    pub reply: RpcReplyPort<Result<SafetyFallbackResult, BodyGatewayError>>,
}

#[derive(Debug)]
pub struct ActionExecutionCompleted {
    pub context: ExecutionContext,
    pub action_kind: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub duration_ms: u64,
    pub result: Result<BodyCommandResult, BodyGatewayError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyStatus {
    pub connected: bool,
    pub current_packet_id: Option<uuid::Uuid>,
    pub accepted_packets: u64,
    pub rejected_packets: u64,
    pub last_terminal_packet_id: Option<uuid::Uuid>,
    pub last_terminal_status: Option<PacketTerminalStatus>,
    pub active_navigation_mission_id: Option<uuid::Uuid>,
    pub navigation_state: Option<crate::execution::movement::NavigationMissionState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionCancelReason {
    Preempted,
    AbortCondition(String),
    Shutdown,
}

pub enum PerceptionMsg {
    Observation(Box<PerceptionInput>),
    PublishFrame(Arc<TacticalFrame>),
    BackendEvent(GameEvent),
    ActionOutcome(ActionOutcome),
    NavigationBlocked {
        mission_id: uuid::Uuid,
        reason_code: String,
        attempts: u32,
    },
    NavigationArrived(NavigationArrival),
    Tick,
    ReplaceTactician(ractor::ActorRef<TacticianMsg>),
    Health(RpcReplyPort<PerceptionStatus>),
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerceptionStatus {
    pub frames_published: u64,
    pub snapshots_rejected: u64,
    pub latest_perception_revision: u64,
    pub buffered_events: usize,
}

pub enum TacticianMsg {
    FrameUpdated(Arc<TacticalFrame>),
    StrategyUpdated(Arc<StrategicIntent>),
    ForceDecision(TacticalTrigger),
    DecisionCompleted(TacticalDecisionResult),
    ScheduleTick,
    ReplaceBody(ractor::ActorRef<BodyMsg>),
    Health(RpcReplyPort<TacticianStatus>),
    #[cfg(test)]
    FailForTest,
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TacticalTrigger {
    MaterialFrame,
    StrategyChanged,
    Heartbeat,
    ActionFailed,
}

#[derive(Debug)]
pub struct TacticalDecisionResult {
    pub decision_id: uuid::Uuid,
    pub frame_revision: u64,
    pub strategic_revision: u64,
    pub result: Result<TacticalProposal, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TacticianStatus {
    pub inference_in_flight: bool,
    pub latest_frame_revision: u64,
    pub latest_strategic_revision: u64,
    pub decisions_started: u64,
    pub stale_decisions_discarded: u64,
}

pub enum StrategistMsg {
    InstallBrain {
        character_id: String,
        persona: String,
        brain: Arc<dyn Brain<StrategicInput, StrategicProposal>>,
    },
    WorldMoment(String),
    EpisodeFinished(EpisodeSummary),
    GoalBlocked(String),
    NavigationArrived(NavigationArrival),
    PersonSpoke(crate::world::dialogue::DialogueLine),
    Reflect,
    ScheduleTick,
    RecallCompleted(StrategicRecallResult),
    InferenceCompleted(StrategicInferenceResult),
    ReplaceTactician(ractor::ActorRef<TacticianMsg>),
    Health(RpcReplyPort<StrategistStatus>),
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrategistStatus {
    pub latest_revision: u64,
    pub queued_moments: usize,
    pub inference_in_flight: bool,
    pub input_revision: u64,
    pub inferences_started: u64,
    pub inferences_coalesced: u64,
    pub inferences_failed: u64,
    pub consecutive_inference_failures: u32,
    pub last_successful_inference_age_ms: Option<u64>,
}

#[derive(Debug)]
pub struct StrategicInferenceResult {
    pub decision_id: uuid::Uuid,
    pub input_revision: u64,
    pub base_strategic_revision: u64,
    pub result: Result<StrategicProposal, String>,
}

#[derive(Debug)]
pub struct StrategicRecallResult {
    pub recall_id: uuid::Uuid,
    pub input_revision: u64,
    pub base_strategic_revision: u64,
    pub result: Result<StrategicRecall, String>,
}

pub enum MemoryMsg {
    RecordRelationship(RelationshipUpdate),
    RecordEpisode(EpisodeSummary),
    UpdateStrategicIntent(StrategicIntent),
    ApplyStrategicPlan {
        update: StrategicWorkingUpdate,
        intent: StrategicIntent,
    },
    Recall(RecallQuery, RpcReplyPort<Result<StrategicRecall, String>>),
    AdvancePlanStep(PlanProgressUpdate),
    ReplaceWorking(WorkingMemory),
    ReadWorking(RpcReplyPort<WorkingMemory>),
    Health(RpcReplyPort<MemoryStatus>),
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryStatus {
    pub episodes_recorded: u64,
    pub relationships_recorded: u64,
    pub writes_failed: u64,
}

pub enum TelemetryMsg {
    Record(TelemetryEvent),
    Snapshot(RpcReplyPort<TelemetrySnapshot>),
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TelemetryEvent {
    ActorStarted(ActorKind),
    ActorFailed {
        actor: ActorKind,
        reason: String,
    },
    ActorTerminated {
        actor: ActorKind,
        reason: Option<String>,
    },
    FramePublished {
        observation_cycle_id: Option<uuid::Uuid>,
        observation_cycle_sequence: Option<u64>,
        frame_revision: u64,
        perception_revision: u64,
        strategic_revision: u64,
        inventory_revision: u64,
        map_revision: u64,
        summary: Box<PerceptionSummary>,
    },
    PerceptionRejected {
        observation_cycle_id: uuid::Uuid,
        observation_cycle_sequence: u64,
        error_class: String,
    },
    StrategyPublished {
        decision_id: uuid::Uuid,
        input_revision: u64,
        revision: u64,
        objective_chars: usize,
        subgoal_count: usize,
        priority_count: usize,
        constraint_count: usize,
        preferred_target_count: usize,
        navigation_scene: Option<String>,
        navigation_tile_known: bool,
    },
    StrategicRecallStarted {
        recall_id: uuid::Uuid,
        input_revision: u64,
        base_strategic_revision: u64,
        query_chars: usize,
    },
    StrategicRecallCompleted {
        recall_id: uuid::Uuid,
        input_revision: u64,
        base_strategic_revision: u64,
        duration_ms: u64,
        semantic_count: usize,
        relationship_count: usize,
        episode_count: usize,
        plan_step_count: usize,
    },
    StrategicRecallFailed {
        recall_id: uuid::Uuid,
        input_revision: u64,
        base_strategic_revision: u64,
        duration_ms: u64,
        error_class: String,
    },
    StrategicPlanChanged {
        decision_id: uuid::Uuid,
        plan_revision: u64,
        step_count: usize,
        retained_step_count: usize,
        blocked: bool,
        completion_claimed: bool,
    },
    StrategicPlanStepAdvanced {
        correlation_id: uuid::Uuid,
        plan_revision: u64,
        transition: String,
        tries: u32,
        evidence_count: usize,
    },
    StrategicNavigationArrivalObserved {
        mission_id: uuid::Uuid,
        decision_id: uuid::Uuid,
        strategic_revision: u64,
        destination_scene: String,
        arrived_scene: Option<String>,
        destination_tile_known: bool,
        arrived_tile_known: bool,
        attempts: u32,
    },
    StrategicInferenceStarted {
        decision_id: uuid::Uuid,
        input_revision: u64,
        base_strategic_revision: u64,
        moment_count: usize,
        frame_revision: u64,
        scene: Option<String>,
        visible_entity_count: usize,
        visible_hostile_count: usize,
        exit_count: usize,
        recent_scene_transition_count: usize,
        consecutive_failures_before_call: u32,
        last_successful_inference_age_ms: Option<u64>,
    },
    StrategicInferenceCoalesced {
        decision_id: uuid::Uuid,
        active_input_revision: u64,
        base_strategic_revision: u64,
        pending_input_revision: u64,
        pending_moment_count: usize,
    },
    StrategicInferenceDeferred {
        schedule_id: uuid::Uuid,
        input_revision: u64,
        base_strategic_revision: u64,
        pending_moment_count: usize,
        eligible_after_ms: u64,
    },
    StrategicInferenceSuperseded {
        decision_id: uuid::Uuid,
        input_revision: u64,
        base_strategic_revision: u64,
        duration_ms: u64,
        reason_code: String,
    },
    StrategicInferenceCompleted {
        decision_id: uuid::Uuid,
        input_revision: u64,
        base_strategic_revision: u64,
        published_revision: Option<u64>,
        duration_ms: u64,
        newer_input_pending: bool,
        speech_suppressed_as_stale: bool,
        interaction_suppressed_as_stale: bool,
    },
    StrategicInferenceFailed {
        decision_id: uuid::Uuid,
        input_revision: u64,
        base_strategic_revision: u64,
        duration_ms: u64,
        error_class: String,
        consecutive_failures: u32,
        retry_after_ms: u64,
        last_successful_inference_age_ms: Option<u64>,
        previous_intent_retained: bool,
    },
    TacticalWakeRequested {
        signal_id: uuid::Uuid,
        frame_revision: u64,
        strategic_revision: u64,
        reason: TacticalWakeReason,
        activity: TacticalActivity,
    },
    TacticalWakeSuppressed {
        signal_id: uuid::Uuid,
        frame_revision: u64,
        strategic_revision: u64,
        reason: SuppressionReason,
    },
    TacticalWakeDeferred {
        signal_id: uuid::Uuid,
        frame_revision: u64,
        strategic_revision: u64,
        reason: DeferralReason,
        eligible_after_ms: Option<u64>,
        coalesced_reason_count: usize,
    },
    TacticalWakeCoalesced {
        signal_id: uuid::Uuid,
        frame_revision: u64,
        strategic_revision: u64,
        pending_frame_revision: u64,
        pending_strategic_revision: u64,
        coalesced_reason_count: usize,
    },
    TacticalHeartbeatGenerated {
        signal_id: uuid::Uuid,
        frame_revision: u64,
        strategic_revision: u64,
        activity: TacticalActivity,
    },
    TacticalDecisionStarted {
        trigger_signal_id: uuid::Uuid,
        decision_id: uuid::Uuid,
        scheduler_inference_id: u64,
        frame_revision: u64,
        strategic_revision: u64,
        wake_reasons: Vec<TacticalWakeReason>,
    },
    TacticalDecisionSuperseded {
        decision_id: uuid::Uuid,
        frame_revision: u64,
        strategic_revision: u64,
        duration_ms: u64,
        reason_code: String,
    },
    TacticalDecisionCompleted {
        decision_id: uuid::Uuid,
        frame_revision: u64,
        strategic_revision: u64,
        action_count: usize,
        action_plan: String,
        intent: crate::execution::packet::TacticalIntent,
        duration_ms: u64,
    },
    TacticalDecisionFailed {
        decision_id: uuid::Uuid,
        frame_revision: u64,
        strategic_revision: u64,
        duration_ms: u64,
        error_class: String,
    },
    TacticalPacketReleaseDecided {
        decision_id: uuid::Uuid,
        packet_id: uuid::Uuid,
        frame_revision: u64,
        strategic_revision: u64,
        rollout_mode: String,
        release_policy: PacketRelease,
        action_count: usize,
        intent: crate::execution::packet::TacticalIntent,
        released: bool,
        reason_code: String,
    },
    PacketAccepted {
        packet_id: uuid::Uuid,
        decision_id: uuid::Uuid,
        frame_revision: u64,
        strategic_revision: u64,
    },
    PacketRejected {
        packet_id: uuid::Uuid,
        decision_id: uuid::Uuid,
        frame_revision: u64,
        strategic_revision: u64,
        reason: String,
    },
    PacketTerminal {
        packet_id: uuid::Uuid,
        decision_id: uuid::Uuid,
        frame_revision: u64,
        strategic_revision: u64,
        status: crate::execution::outcome::PacketTerminalStatus,
        reason_code: Option<String>,
        superseded_by: Option<uuid::Uuid>,
    },
    ActionStarted {
        context: ExecutionContext,
        action_kind: String,
    },
    ActionTerminal {
        outcome: ActionOutcome,
        session_generation: u64,
    },
    Movement {
        context: ExecutionContext,
        fact: MovementTelemetry,
    },
    NavigationMission {
        decision_id: uuid::Uuid,
        fact: NavigationMissionTelemetry,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TelemetrySnapshot {
    pub events_recorded: u64,
    pub actor_failures: u64,
    pub packets_accepted: u64,
    pub packets_rejected: u64,
    pub packets_completed: u64,
    pub packets_failed: u64,
    pub packets_cancelled: u64,
    pub packets_superseded: u64,
    pub actions_started: u64,
    pub actions_accepted: u64,
    pub actions_succeeded: u64,
    pub actions_failed: u64,
    pub movement_progress_observations: u64,
    pub movement_requests: u64,
    pub movement_arrivals: u64,
    pub movement_stalls: u64,
    pub movement_stops: u64,
    pub movement_stop_failures: u64,
    pub navigation_missions_started: u64,
    pub navigation_missions_arrived: u64,
    pub navigation_missions_failed: u64,
    pub navigation_missions_superseded: u64,
    pub navigation_attempts: u64,
    pub navigation_move_to_attempts: u64,
    pub navigation_door_attempts: u64,
    pub navigation_preemptions: u64,
    pub navigation_retries: u64,
    pub tactical_wakes_requested: u64,
    pub tactical_wakes_suppressed: u64,
    pub tactical_wakes_deferred: u64,
    pub tactical_wakes_coalesced: u64,
    pub tactical_heartbeats_generated: u64,
    pub tactical_inferences_started: u64,
    pub tactical_inferences_completed: u64,
    pub tactical_inferences_superseded: u64,
    pub tactical_inferences_failed: u64,
    pub tactical_packet_release_decisions: u64,
    pub tactical_packets_record_only: u64,
    pub tactical_packets_control_gated: u64,
    pub tactical_packets_released: u64,
    pub strategic_inferences_started: u64,
    pub strategic_inferences_completed: u64,
    pub strategic_inferences_coalesced: u64,
    pub strategic_inferences_deferred: u64,
    pub strategic_inferences_superseded: u64,
    pub strategic_inferences_failed: u64,
    pub strategies_published: u64,
}
