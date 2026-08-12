use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    brain::tactical_frame::TacticalFrame,
    world::{Position, TilePosition},
};

/// A destination-level navigation request.
///
/// Callers describe where the character should end up. The body owns path
/// preflight, MCP dispatch, progress monitoring, retries, and scene
/// transitions. Runtime IDs are deliberately absent and are assigned when the
/// body accepts the request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NavigationMissionRequest {
    pub decision_id: Uuid,
    pub frame_revision: u64,
    pub strategic_revision: u64,
    pub destination_scene: String,
    pub destination_tile: Option<TilePosition>,
    pub destination_name: String,
    pub reason: String,
    #[serde(default)]
    pub route: Vec<NavigationWaypoint>,
}

/// One optional route hint for a longer navigation mission.
///
/// A waypoint names a tile in `scene`. When `transition_to_scene` is present,
/// the body treats that tile as a door and uses `arena_enter_door`. Ordinary
/// waypoint and final-destination tiles use `arena_move_to`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NavigationWaypoint {
    pub scene: String,
    pub tile: TilePosition,
    pub transition_to_scene: Option<String>,
}

/// Authoritative completion fact for one destination-level mission.
///
/// Arrival proves physical position only. It does not prove that a strategic
/// plan step such as an inspection, conversation, or pickup is complete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NavigationArrival {
    pub mission_id: Uuid,
    pub decision_id: Uuid,
    pub strategic_revision: u64,
    pub destination_scene: String,
    pub destination_tile: Option<TilePosition>,
    pub destination_name: String,
    pub arrived_scene: Option<String>,
    pub arrived_tile: Option<TilePosition>,
    pub attempts: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NavigationMissionState {
    Active,
    Paused,
    Arrived,
    Failed,
    Cancelled,
    Superseded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NavigationAttemptKind {
    PathPreflight,
    MoveTo,
    EnterDoor,
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NavigationMissionTelemetry {
    Started {
        mission_id: Uuid,
        recorded_at: DateTime<Utc>,
        destination_scene: String,
        destination_tile: Option<TilePosition>,
        route_waypoints: usize,
    },
    AttemptStarted {
        mission_id: Uuid,
        attempt_id: Uuid,
        recorded_at: DateTime<Utc>,
        attempt_number: u32,
        attempt_kind: NavigationAttemptKind,
        scene: Option<String>,
        target_tile: TilePosition,
    },
    Paused {
        mission_id: Uuid,
        recorded_at: DateTime<Utc>,
        reason_code: String,
    },
    Resumed {
        mission_id: Uuid,
        recorded_at: DateTime<Utc>,
        scene: Option<String>,
        attempt_number: u32,
    },
    DuplicateSuppressed {
        mission_id: Uuid,
        recorded_at: DateTime<Utc>,
        strategic_revision: u64,
    },
    WaypointReached {
        mission_id: Uuid,
        recorded_at: DateTime<Utc>,
        waypoint_index: usize,
        scene: Option<String>,
        position_tile: Option<TilePosition>,
    },
    RetryScheduled {
        mission_id: Uuid,
        recorded_at: DateTime<Utc>,
        attempt_number: u32,
        reason_code: String,
    },
    Terminal {
        mission_id: Uuid,
        recorded_at: DateTime<Utc>,
        state: NavigationMissionState,
        reason_code: Option<String>,
        scene: Option<String>,
        position_tile: Option<TilePosition>,
        attempts: u32,
    },
}

/// Runtime identity for one movement action.
///
/// The model does not supply any of these identifiers. They connect movement
/// observations to the decision, packet, and action that requested the move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MovementOwnership {
    pub movement_id: Uuid,
    pub decision_id: Uuid,
    pub packet_id: Uuid,
    pub action_index: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MovementRequest {
    pub ownership: MovementOwnership,
    pub destination: TilePosition,
    pub requested_scene: Option<String>,
    pub requested_at: DateTime<Utc>,
    pub start_position: Option<Position>,
}

/// Thresholds used to classify observed physical facts.
///
/// These values do not select a destination or a recovery action. They only
/// define when two observed positions count as progress and when a lack of
/// progress counts as a stall.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MovementObservationRules {
    pub minimum_progress_pixels: f32,
    pub stalled_after_observations: u32,
}

impl Default for MovementObservationRules {
    fn default() -> Self {
        Self {
            minimum_progress_pixels: 1.0,
            stalled_after_observations: 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MovementState {
    Requested,
    Moving,
    Arrived,
    Stalled,
    Blocked,
    Cancelled,
    Interrupted,
    SceneTransition,
}

impl MovementState {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Arrived
                | Self::Stalled
                | Self::Blocked
                | Self::Cancelled
                | Self::Interrupted
                | Self::SceneTransition
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PathPreflightStatus {
    NotChecked,
    Reachable,
    Unreachable,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PathPreflightFact {
    pub status: PathPreflightStatus,
    pub checked_at: Option<DateTime<Utc>>,
    pub path_length_tiles: Option<u32>,
}

impl Default for PathPreflightFact {
    fn default() -> Self {
        Self {
            status: PathPreflightStatus::NotChecked,
            checked_at: None,
            path_length_tiles: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MovementObservationFact {
    pub observed_at: DateTime<Utc>,
    pub scene: Option<String>,
    pub position: Option<Position>,
    pub backend_reports_moving: Option<bool>,
    pub made_progress: bool,
    pub distance_from_previous_pixels: Option<f32>,
    pub remaining_tile_distance: Option<u32>,
    pub reached_destination: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ArrivalFact {
    pub observed_at: DateTime<Utc>,
    pub position: Option<Position>,
    pub evidence: ArrivalEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArrivalEvidence {
    BackendResult,
    PerceptionFrame,
}

impl ArrivalEvidence {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BackendResult => "backend_result",
            Self::PerceptionFrame => "perception_frame",
        }
    }
}

/// Stable, typed movement facts emitted to telemetry.
///
/// These deliberately contain only causal identifiers and reduced physical
/// facts. Backend payloads never cross the telemetry boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MovementTelemetry {
    Requested {
        requested_at: DateTime<Utc>,
        origin_tile: Option<TilePosition>,
        destination_tile: TilePosition,
    },
    Progress {
        observed_at: DateTime<Utc>,
        frame_revision: u64,
        position_tile: Option<TilePosition>,
        distance_from_previous_millipixels: Option<u64>,
        observed_distance_millipixels: u64,
        remaining_tile_distance: Option<u32>,
    },
    Arrival {
        observed_at: DateTime<Utc>,
        frame_revision: Option<u64>,
        position_tile: Option<TilePosition>,
        evidence: ArrivalEvidence,
    },
    SceneTransition {
        observed_at: DateTime<Utc>,
        frame_revision: u64,
        from_scene: Option<String>,
        to_scene: Option<String>,
        position_tile: Option<TilePosition>,
    },
    Stall {
        observed_at: DateTime<Utc>,
        frame_revision: Option<u64>,
        position_tile: Option<TilePosition>,
        observations_without_progress: u32,
    },
    Stop {
        recorded_at: DateTime<Utc>,
        stop_action_id: Uuid,
        reason_code: String,
        succeeded: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SceneTransitionFact {
    pub observed_at: DateTime<Utc>,
    pub from_scene: Option<String>,
    pub to_scene: Option<String>,
    pub position: Option<Position>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BlockedReason {
    PathPreflightUnreachable,
    PathInvalidated,
    BackendRejected,
    NoReachableRecovery,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BlockedFact {
    pub observed_at: DateTime<Utc>,
    pub reason: BlockedReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CancellationReason {
    PacketSuperseded,
    TacticalStop,
    RuntimeShutdown,
    SessionInvalidated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CancellationFact {
    pub cancelled_at: DateTime<Utc>,
    pub reason: CancellationReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InterruptionReason {
    Combat,
    PlayerDied,
    HealthChanged,
    NewHostile,
    StrategicConstraintChanged,
    TargetUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InterruptionFact {
    pub interrupted_at: DateTime<Utc>,
    pub reason: InterruptionReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryKind {
    AlternatePath,
    AdjacentReachableTile,
    StopAndRetry,
    DoorApproach,
    Unstick,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryResult {
    Started,
    Succeeded,
    Failed,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RecoveryAttemptFact {
    pub attempt_id: Uuid,
    pub attempted_at: DateTime<Utc>,
    pub kind: RecoveryKind,
    pub target: Option<TilePosition>,
    pub result: RecoveryResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UnstickCooldownFacts {
    pub cooldown_ms: u64,
    pub last_successful_unstick_at: Option<DateTime<Utc>>,
    pub available_at: Option<DateTime<Utc>>,
    pub remaining_ms: u64,
    pub available: bool,
}

/// Cooldown history that can be carried across movement actions.
///
/// Recording an attempt does not authorize it. This type only records whether
/// a previously selected unstick operation succeeded and calculates the
/// resulting time facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UnstickCooldown {
    pub cooldown_ms: u64,
    pub last_successful_unstick_at: Option<DateTime<Utc>>,
}

impl UnstickCooldown {
    #[must_use]
    pub const fn new(cooldown_ms: u64) -> Self {
        Self {
            cooldown_ms,
            last_successful_unstick_at: None,
        }
    }

    pub fn record_attempt(&mut self, attempt: &RecoveryAttemptFact) {
        if attempt.kind == RecoveryKind::Unstick
            && attempt.result == RecoveryResult::Succeeded
            && self
                .last_successful_unstick_at
                .is_none_or(|previous| attempt.attempted_at > previous)
        {
            self.last_successful_unstick_at = Some(attempt.attempted_at);
        }
    }

    #[must_use]
    pub fn facts(&self, now: DateTime<Utc>) -> UnstickCooldownFacts {
        let available_at = self.last_successful_unstick_at.and_then(|last| {
            last.checked_add_signed(chrono::Duration::milliseconds(
                i64::try_from(self.cooldown_ms).unwrap_or(i64::MAX),
            ))
        });
        let remaining_ms = match (self.last_successful_unstick_at, available_at) {
            (None, _) => 0,
            (Some(_), None) => u64::MAX,
            (Some(_), Some(available_at)) => {
                u64::try_from((available_at - now).num_milliseconds().max(0)).unwrap_or(u64::MAX)
            }
        };
        UnstickCooldownFacts {
            cooldown_ms: self.cooldown_ms,
            last_successful_unstick_at: self.last_successful_unstick_at,
            available_at,
            remaining_ms,
            available: remaining_ms == 0,
        }
    }
}

/// Frame-driven physical movement facts for one requested action.
///
/// This type never chooses to move, retry, recover, or unstick. Callers record
/// commands and their authoritative results; this reducer classifies only the
/// resulting observations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MovementProgress {
    pub request: MovementRequest,
    pub state: MovementState,
    pub path_preflight: PathPreflightFact,
    pub started_at: Option<DateTime<Utc>>,
    pub latest_observed_at: Option<DateTime<Utc>>,
    pub latest_position: Option<Position>,
    pub latest_position_observed_at: Option<DateTime<Utc>>,
    pub last_progress_at: Option<DateTime<Utc>>,
    pub observed_distance_pixels: f32,
    pub progress_observations: u32,
    pub observations_without_progress: u32,
    /// Frames with no tile change. Pixel jitter at a blocked doorway must not
    /// keep a movement request alive indefinitely.
    pub observations_without_tile_change: u32,
    pub last_observation: Option<MovementObservationFact>,
    pub arrival: Option<ArrivalFact>,
    pub scene_transition: Option<SceneTransitionFact>,
    pub blocked: Option<BlockedFact>,
    pub cancellation: Option<CancellationFact>,
    pub interruption: Option<InterruptionFact>,
    pub recovery_attempts: Vec<RecoveryAttemptFact>,
    pub unstick_cooldown: UnstickCooldown,
}

impl MovementProgress {
    #[must_use]
    pub fn new(request: MovementRequest, unstick_cooldown_ms: u64) -> Self {
        Self {
            latest_position: request.start_position,
            request,
            state: MovementState::Requested,
            path_preflight: PathPreflightFact::default(),
            started_at: None,
            latest_observed_at: None,
            latest_position_observed_at: None,
            last_progress_at: None,
            observed_distance_pixels: 0.0,
            progress_observations: 0,
            observations_without_progress: 0,
            observations_without_tile_change: 0,
            last_observation: None,
            arrival: None,
            scene_transition: None,
            blocked: None,
            cancellation: None,
            interruption: None,
            recovery_attempts: Vec::new(),
            unstick_cooldown: UnstickCooldown::new(unstick_cooldown_ms),
        }
    }

    /// Record the backend's path-preflight response.
    ///
    /// An explicit unreachable response is a terminal blocked fact. Missing
    /// backend data remains unknown instead of being treated as reachable.
    pub fn record_path_preflight(
        &mut self,
        checked_at: DateTime<Utc>,
        reachable: Option<bool>,
        path_length_tiles: Option<u32>,
    ) {
        if self.state.is_terminal() {
            return;
        }
        self.path_preflight = PathPreflightFact {
            status: match reachable {
                Some(true) => PathPreflightStatus::Reachable,
                Some(false) => PathPreflightStatus::Unreachable,
                None => PathPreflightStatus::Unknown,
            },
            checked_at: Some(checked_at),
            path_length_tiles,
        };
        if reachable == Some(false) {
            self.mark_blocked(checked_at, BlockedReason::PathPreflightUnreachable);
        }
    }

    /// Record that the backend accepted the movement command.
    pub fn record_started(&mut self, started_at: DateTime<Utc>) {
        if self.state == MovementState::Requested {
            self.started_at = Some(started_at);
            self.state = MovementState::Moving;
        }
    }

    /// Record an explicit backend arrival without treating command acceptance
    /// or partial motion as arrival evidence.
    pub fn record_backend_arrival(&mut self, observed_at: DateTime<Utc>) {
        if self.state.is_terminal() {
            return;
        }
        self.arrival = Some(ArrivalFact {
            observed_at,
            position: self.latest_position,
            evidence: ArrivalEvidence::BackendResult,
        });
        self.state = MovementState::Arrived;
    }

    /// Reduce one authoritative tactical frame into movement facts.
    pub fn observe_frame(&mut self, frame: &TacticalFrame, rules: MovementObservationRules) {
        if self.state.is_terminal() {
            return;
        }

        let observed_at = frame.generated_at;
        let scene = frame.self_state.scene.clone();
        let position = frame.self_state.position;
        let backend_reports_moving = frame.self_state.moving;
        self.latest_observed_at = Some(observed_at);

        if scene_changed(self.request.requested_scene.as_deref(), scene.as_deref()) {
            self.scene_transition = Some(SceneTransitionFact {
                observed_at,
                from_scene: self.request.requested_scene.clone(),
                to_scene: scene,
                position,
            });
            if position.is_some() {
                self.latest_position_observed_at = Some(observed_at);
            }
            self.latest_position = position.or(self.latest_position);
            self.state = MovementState::SceneTransition;
            return;
        }

        let minimum_progress = rules.minimum_progress_pixels.max(f32::EPSILON);
        let distance_from_previous_pixels = position
            .zip(self.latest_position)
            .map(|(current, previous)| pixel_distance(previous, current));
        let made_progress =
            distance_from_previous_pixels.is_some_and(|distance| distance >= minimum_progress);
        let reached_destination =
            position.is_some_and(|current| current.tile == self.request.destination);
        let remaining_tile_distance = position.map(|current| {
            current
                .tile
                .x
                .abs_diff(self.request.destination.x)
                .saturating_add(current.tile.y.abs_diff(self.request.destination.y))
        });
        let tile_changed = position
            .zip(self.latest_position)
            .is_some_and(|(current, previous)| current.tile != previous.tile);

        if made_progress {
            self.progress_observations = self.progress_observations.saturating_add(1);
            self.last_progress_at = Some(observed_at);
            self.observed_distance_pixels += distance_from_previous_pixels.unwrap_or(0.0);
        }
        if made_progress && backend_reports_moving != Some(false) {
            self.observations_without_progress = 0;
        } else if position.is_some() {
            self.observations_without_progress =
                self.observations_without_progress.saturating_add(1);
        }
        if tile_changed {
            self.observations_without_tile_change = 0;
        } else if position.is_some() {
            self.observations_without_tile_change =
                self.observations_without_tile_change.saturating_add(1);
        }
        if position.is_some() {
            self.latest_position_observed_at = Some(observed_at);
        }
        self.latest_position = position.or(self.latest_position);
        self.last_observation = Some(MovementObservationFact {
            observed_at,
            scene,
            position,
            backend_reports_moving,
            made_progress,
            distance_from_previous_pixels,
            remaining_tile_distance,
            reached_destination,
        });

        if let Some(position) = position.filter(|_| reached_destination) {
            self.arrival = Some(ArrivalFact {
                observed_at,
                position: Some(position),
                evidence: ArrivalEvidence::PerceptionFrame,
            });
            self.state = MovementState::Arrived;
        } else if self.state == MovementState::Moving
            && rules.stalled_after_observations > 0
            && self.observations_without_progress >= rules.stalled_after_observations
        {
            self.state = MovementState::Stalled;
        } else if self.state == MovementState::Moving && self.observations_without_tile_change >= 12
        {
            // The backend can report moving while the character only jitters
            // against a collision. Treat prolonged tile stagnation as a stall.
            self.state = MovementState::Stalled;
        }
    }

    pub fn mark_blocked(&mut self, observed_at: DateTime<Utc>, reason: BlockedReason) {
        if self.state.is_terminal() {
            return;
        }
        self.blocked = Some(BlockedFact {
            observed_at,
            reason,
        });
        self.state = MovementState::Blocked;
    }

    pub fn cancel(&mut self, cancelled_at: DateTime<Utc>, reason: CancellationReason) {
        if self.state.is_terminal() {
            return;
        }
        self.cancellation = Some(CancellationFact {
            cancelled_at,
            reason,
        });
        self.state = MovementState::Cancelled;
    }

    pub fn interrupt(&mut self, interrupted_at: DateTime<Utc>, reason: InterruptionReason) {
        if self.state.is_terminal() {
            return;
        }
        self.interruption = Some(InterruptionFact {
            interrupted_at,
            reason,
        });
        self.state = MovementState::Interrupted;
    }

    /// Record a recovery operation selected by a caller.
    ///
    /// A successful unstick starts the cooldown. Failed and unavailable
    /// attempts do not consume it.
    pub fn record_recovery_attempt(&mut self, attempt: RecoveryAttemptFact) {
        self.unstick_cooldown.record_attempt(&attempt);
        self.recovery_attempts.push(attempt);
    }

    #[must_use]
    pub fn unstick_cooldown_facts(&self, now: DateTime<Utc>) -> UnstickCooldownFacts {
        self.unstick_cooldown.facts(now)
    }
}

fn scene_changed(requested: Option<&str>, observed: Option<&str>) -> bool {
    requested.is_some() && observed.is_some() && requested != observed
}

fn pixel_distance(left: Position, right: Position) -> f32 {
    let delta_x = right.pixel.x - left.pixel.x;
    let delta_y = right.pixel.y - left.pixel.y;
    delta_x.hypot(delta_y)
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;

    use super::*;
    use crate::{
        brain::strategic_intent::StrategicIntent,
        world::{PixelPosition, Position},
    };

    const HOUR_MS: u64 = 60 * 60 * 1_000;

    fn at(second: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(second, 0).expect("valid test timestamp")
    }

    fn position(pixel_x: f32, pixel_y: f32, tile_x: i32, tile_y: i32) -> Position {
        Position {
            pixel: PixelPosition {
                x: pixel_x,
                y: pixel_y,
            },
            tile: TilePosition {
                x: tile_x,
                y: tile_y,
            },
        }
    }

    fn request() -> MovementRequest {
        MovementRequest {
            ownership: MovementOwnership {
                movement_id: Uuid::from_u128(1),
                decision_id: Uuid::from_u128(2),
                packet_id: Uuid::from_u128(3),
                action_index: 4,
            },
            destination: TilePosition { x: 2, y: 0 },
            requested_scene: Some("town".to_owned()),
            requested_at: at(10),
            start_position: Some(position(16.0, 16.0, 0, 0)),
        }
    }

    fn frame(
        second: i64,
        scene: Option<&str>,
        position: Option<Position>,
        moving: Option<bool>,
    ) -> TacticalFrame {
        let mut frame = TacticalFrame::empty(StrategicIntent::default());
        frame.generated_at = at(second);
        frame.self_state.scene = scene.map(str::to_owned);
        frame.self_state.position = position;
        frame.self_state.moving = moving;
        frame
    }

    #[test]
    fn preserves_runtime_movement_and_action_ownership() {
        let progress = MovementProgress::new(request(), HOUR_MS);

        assert_eq!(progress.request.ownership.movement_id, Uuid::from_u128(1));
        assert_eq!(progress.request.ownership.decision_id, Uuid::from_u128(2));
        assert_eq!(progress.request.ownership.packet_id, Uuid::from_u128(3));
        assert_eq!(progress.request.ownership.action_index, 4);
    }

    #[test]
    fn unreachable_preflight_is_a_blocked_fact() {
        let mut progress = MovementProgress::new(request(), HOUR_MS);

        progress.record_path_preflight(at(11), Some(false), None);

        assert_eq!(progress.state, MovementState::Blocked);
        assert_eq!(
            progress.path_preflight.status,
            PathPreflightStatus::Unreachable
        );
        assert_eq!(
            progress.blocked.as_ref().map(|fact| fact.reason),
            Some(BlockedReason::PathPreflightUnreachable)
        );
        progress.record_started(at(12));
        assert_eq!(
            progress.started_at, None,
            "terminal movement cannot restart"
        );
    }

    #[test]
    fn frames_record_pixel_progress_and_authoritative_arrival() {
        let mut progress = MovementProgress::new(request(), HOUR_MS);
        progress.record_path_preflight(at(11), Some(true), Some(2));
        progress.record_started(at(12));

        progress.observe_frame(
            &frame(
                13,
                Some("town"),
                Some(position(32.0, 16.0, 1, 0)),
                Some(true),
            ),
            MovementObservationRules::default(),
        );
        assert_eq!(progress.state, MovementState::Moving);
        assert_eq!(progress.progress_observations, 1);
        assert_eq!(progress.observations_without_progress, 0);
        assert_eq!(progress.last_progress_at, Some(at(13)));
        assert_eq!(progress.latest_position_observed_at, Some(at(13)));
        assert!((progress.observed_distance_pixels - 16.0).abs() < f32::EPSILON);
        let observation = progress.last_observation.as_ref().expect("movement fact");
        assert_eq!(observation.distance_from_previous_pixels, Some(16.0));
        assert_eq!(observation.remaining_tile_distance, Some(1));

        let destination = position(80.0, 16.0, 2, 0);
        progress.observe_frame(
            &frame(14, Some("town"), Some(destination), Some(false)),
            MovementObservationRules::default(),
        );

        assert_eq!(progress.state, MovementState::Arrived);
        assert_eq!(
            progress.arrival.as_ref().and_then(|fact| fact.position),
            Some(destination)
        );
        assert_eq!(
            progress.arrival.as_ref().map(|fact| fact.evidence),
            Some(ArrivalEvidence::PerceptionFrame)
        );
        assert_eq!(progress.latest_position, Some(destination));
        assert_eq!(progress.latest_position_observed_at, Some(at(14)));
        assert!((progress.observed_distance_pixels - 64.0).abs() < f32::EPSILON);
    }

    #[test]
    fn only_an_explicit_backend_arrival_marks_arrival() {
        let mut progress = MovementProgress::new(request(), HOUR_MS);
        progress.record_started(at(11));

        assert_eq!(progress.state, MovementState::Moving);
        assert!(progress.arrival.is_none());

        progress.record_backend_arrival(at(12));

        assert_eq!(progress.state, MovementState::Arrived);
        assert_eq!(
            progress.arrival.as_ref().map(|fact| fact.evidence),
            Some(ArrivalEvidence::BackendResult)
        );
    }

    #[test]
    fn repeated_unchanged_frames_classify_a_stall() {
        let mut progress = MovementProgress::new(request(), HOUR_MS);
        progress.record_started(at(11));
        let unchanged = Some(position(16.0, 16.0, 0, 0));
        let rules = MovementObservationRules {
            minimum_progress_pixels: 1.0,
            stalled_after_observations: 3,
        };

        progress.observe_frame(&frame(12, Some("town"), unchanged, Some(true)), rules);
        progress.observe_frame(&frame(13, Some("town"), unchanged, Some(true)), rules);
        assert_eq!(progress.state, MovementState::Moving);
        progress.observe_frame(&frame(14, Some("town"), unchanged, Some(true)), rules);

        assert_eq!(progress.state, MovementState::Stalled);
        assert_eq!(progress.observations_without_progress, 3);
    }

    #[test]
    fn pixel_jitter_without_tile_change_classifies_a_doorway_stall() {
        let mut progress = MovementProgress::new(request(), HOUR_MS);
        progress.record_started(at(11));
        let rules = MovementObservationRules {
            minimum_progress_pixels: 1.0,
            stalled_after_observations: 100,
        };

        for index in 0..12 {
            let pixel = 16.0 + (index % 2) as f32 * 3.0;
            progress.observe_frame(
                &frame(
                    12 + i64::from(index),
                    Some("town"),
                    Some(position(pixel, 16.0, 0, 0)),
                    Some(true),
                ),
                rules,
            );
        }

        assert_eq!(progress.state, MovementState::Stalled);
        assert_eq!(progress.observations_without_tile_change, 12);
    }

    #[test]
    fn a_stopped_path_counts_toward_stall_classification() {
        let mut progress = MovementProgress::new(request(), HOUR_MS);
        progress.record_started(at(11));
        let rules = MovementObservationRules {
            minimum_progress_pixels: 1.0,
            stalled_after_observations: 2,
        };

        progress.observe_frame(
            &frame(
                12,
                Some("town"),
                Some(position(32.0, 16.0, 1, 0)),
                Some(false),
            ),
            rules,
        );
        assert_eq!(progress.state, MovementState::Moving);
        progress.observe_frame(
            &frame(
                13,
                Some("town"),
                Some(position(32.0, 16.0, 1, 0)),
                Some(false),
            ),
            rules,
        );

        assert_eq!(progress.state, MovementState::Stalled);
    }

    #[test]
    fn scene_change_is_classified_separately_from_arrival() {
        let mut progress = MovementProgress::new(request(), HOUR_MS);
        progress.record_started(at(11));

        progress.observe_frame(
            &frame(
                12,
                Some("forest"),
                Some(position(16.0, 16.0, 0, 0)),
                Some(false),
            ),
            MovementObservationRules::default(),
        );

        assert_eq!(progress.state, MovementState::SceneTransition);
        let transition = progress.scene_transition.expect("transition fact");
        assert_eq!(transition.from_scene.as_deref(), Some("town"));
        assert_eq!(transition.to_scene.as_deref(), Some("forest"));
        assert_eq!(progress.latest_position_observed_at, Some(at(12)));
    }

    #[test]
    fn cancellation_and_interruption_are_distinct_terminal_facts() {
        let mut cancelled = MovementProgress::new(request(), HOUR_MS);
        cancelled.record_started(at(11));
        cancelled.cancel(at(12), CancellationReason::PacketSuperseded);
        assert_eq!(cancelled.state, MovementState::Cancelled);
        assert_eq!(
            cancelled.cancellation.as_ref().map(|fact| fact.reason),
            Some(CancellationReason::PacketSuperseded)
        );

        let mut interrupted = MovementProgress::new(request(), HOUR_MS);
        interrupted.record_started(at(11));
        interrupted.interrupt(at(12), InterruptionReason::Combat);
        assert_eq!(interrupted.state, MovementState::Interrupted);
        assert_eq!(
            interrupted.interruption.as_ref().map(|fact| fact.reason),
            Some(InterruptionReason::Combat)
        );
    }

    #[test]
    fn recovery_attempts_are_facts_and_only_successful_unstick_starts_cooldown() {
        let mut progress = MovementProgress::new(request(), HOUR_MS);
        progress.record_recovery_attempt(RecoveryAttemptFact {
            attempt_id: Uuid::from_u128(10),
            attempted_at: at(20),
            kind: RecoveryKind::AdjacentReachableTile,
            target: Some(TilePosition { x: 1, y: 1 }),
            result: RecoveryResult::Failed,
        });
        progress.record_recovery_attempt(RecoveryAttemptFact {
            attempt_id: Uuid::from_u128(11),
            attempted_at: at(30),
            kind: RecoveryKind::Unstick,
            target: None,
            result: RecoveryResult::Failed,
        });
        assert!(progress.unstick_cooldown_facts(at(31)).available);

        progress.record_recovery_attempt(RecoveryAttemptFact {
            attempt_id: Uuid::from_u128(12),
            attempted_at: at(40),
            kind: RecoveryKind::Unstick,
            target: None,
            result: RecoveryResult::Succeeded,
        });
        let halfway = progress.unstick_cooldown_facts(at(40) + TimeDelta::minutes(30));
        assert!(!halfway.available);
        assert_eq!(halfway.remaining_ms, 30 * 60 * 1_000);
        assert_eq!(halfway.available_at, Some(at(40) + TimeDelta::hours(1)));

        let elapsed = progress.unstick_cooldown_facts(at(40) + TimeDelta::hours(1));
        assert!(elapsed.available);
        assert_eq!(elapsed.remaining_ms, 0);
        assert_eq!(progress.recovery_attempts.len(), 3);
        assert_eq!(
            progress.recovery_attempts[2].attempt_id,
            Uuid::from_u128(12)
        );

        let carried_cooldown = progress.unstick_cooldown.clone();
        let mut later_movement = MovementProgress::new(request(), HOUR_MS);
        later_movement.unstick_cooldown = carried_cooldown;
        assert!(
            !later_movement
                .unstick_cooldown_facts(at(40) + TimeDelta::minutes(45))
                .available
        );
    }

    #[test]
    fn missing_position_does_not_create_false_stall_evidence() {
        let mut progress = MovementProgress::new(request(), HOUR_MS);
        progress.record_started(at(11));

        for second in 12..20 {
            progress.observe_frame(
                &frame(second, Some("town"), None, None),
                MovementObservationRules::default(),
            );
        }

        assert_eq!(progress.state, MovementState::Moving);
        assert_eq!(progress.observations_without_progress, 0);
    }
}
