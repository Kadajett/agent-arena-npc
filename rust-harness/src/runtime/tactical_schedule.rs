//! Pure scheduling policy for tactical inference.
//!
//! The scheduler has no clock, actor, model, or game dependency. Its caller
//! supplies monotonic timestamps and the latest revisions. This keeps timing
//! tests deterministic and lets an actor choose how to arm timers.

use std::{collections::BTreeSet, fmt, str::FromStr, time::Duration};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Controls how far a tactical decision may progress through the runtime.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TacticalRolloutMode {
    /// Collect perception and scheduling facts, but do not call a model.
    #[default]
    ObserveOnly,
    /// Call the model and record its proposal, but never release a packet.
    Shadow,
    /// Release a packet only through an external, explicit control gate.
    Controlled,
    /// Release a validated packet without an additional rollout gate.
    Full,
}

impl TacticalRolloutMode {
    pub const fn allows_inference(self) -> bool {
        !matches!(self, Self::ObserveOnly)
    }

    pub const fn packet_release(self) -> PacketRelease {
        match self {
            Self::ObserveOnly | Self::Shadow => PacketRelease::RecordOnly,
            Self::Controlled => PacketRelease::RequireControlGate,
            Self::Full => PacketRelease::Release,
        }
    }
}

impl fmt::Display for TacticalRolloutMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ObserveOnly => "observe_only",
            Self::Shadow => "shadow",
            Self::Controlled => "controlled",
            Self::Full => "full",
        })
    }
}

impl FromStr for TacticalRolloutMode {
    type Err = ParseRolloutModeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "observe_only" => Ok(Self::ObserveOnly),
            "shadow" => Ok(Self::Shadow),
            "controlled" => Ok(Self::Controlled),
            "full" => Ok(Self::Full),
            _ => Err(ParseRolloutModeError(value.to_owned())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("unknown tactical rollout mode `{0}`")]
pub struct ParseRolloutModeError(String);

/// Describes what may happen to a model proposal in a rollout mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketRelease {
    RecordOnly,
    RequireControlGate,
    Release,
}

impl PacketRelease {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RecordOnly => "record_only",
            Self::RequireControlGate => "require_control_gate",
            Self::Release => "release",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TacticalActivity {
    Idle,
    ActiveCombat,
}

impl TacticalActivity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::ActiveCombat => "active_combat",
        }
    }
}

/// The pair of revisions captured by one inference request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TacticalSnapshot {
    pub frame_revision: u64,
    pub strategic_revision: u64,
}

impl TacticalSnapshot {
    fn relation_to(self, current: Self) -> SnapshotRelation {
        if self.frame_revision < current.frame_revision
            || self.strategic_revision < current.strategic_revision
        {
            SnapshotRelation::Older
        } else if self == current {
            SnapshotRelation::Same
        } else {
            SnapshotRelation::Newer
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapshotRelation {
    Older,
    Same,
    Newer,
}

/// Factual reasons that can wake tactical inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TacticalWakeReason {
    DamageTaken,
    HostileSpawned,
    HostileDespawned,
    TargetDied,
    HealthThresholdChanged,
    MovementFailed,
    ActionRefused,
    LootAppeared,
    StrategyChanged,
    CombatStarted,
    CombatEnded,
    Heartbeat,
    Forced,
}

impl TacticalWakeReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DamageTaken => "damage_taken",
            Self::HostileSpawned => "hostile_spawned",
            Self::HostileDespawned => "hostile_despawned",
            Self::TargetDied => "target_died",
            Self::HealthThresholdChanged => "health_threshold_changed",
            Self::MovementFailed => "movement_failed",
            Self::ActionRefused => "action_refused",
            Self::LootAppeared => "loot_appeared",
            Self::StrategyChanged => "strategy_changed",
            Self::CombatStarted => "combat_started",
            Self::CombatEnded => "combat_ended",
            Self::Heartbeat => "heartbeat",
            Self::Forced => "forced",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TacticalWake {
    pub snapshot: TacticalSnapshot,
    pub activity: TacticalActivity,
    pub reason: TacticalWakeReason,
}

/// Validated rate and heartbeat configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TacticalScheduleConfig {
    global_min_interval: Duration,
    idle_min_interval: Option<Duration>,
    active_combat_heartbeat: Duration,
}

impl TacticalScheduleConfig {
    /// Builds a schedule from frequency ceilings.
    ///
    /// `idle_hz` may be zero to disable idle heartbeat inference. A non-zero
    /// idle ceiling may not exceed the global ceiling.
    ///
    /// # Errors
    ///
    /// Returns an error when either frequency is invalid, the idle ceiling is
    /// above the global ceiling, or the combat heartbeat is zero.
    pub fn from_hz(
        max_hz: f64,
        idle_hz: f64,
        active_combat_heartbeat: Duration,
    ) -> Result<Self, TacticalScheduleConfigError> {
        if !max_hz.is_finite() || max_hz <= 0.0 {
            return Err(TacticalScheduleConfigError::InvalidMaxHz);
        }
        if !idle_hz.is_finite() || idle_hz < 0.0 {
            return Err(TacticalScheduleConfigError::InvalidIdleHz);
        }
        if idle_hz > max_hz {
            return Err(TacticalScheduleConfigError::IdleExceedsMaximum);
        }
        if active_combat_heartbeat.is_zero() {
            return Err(TacticalScheduleConfigError::ZeroCombatHeartbeat);
        }

        Ok(Self {
            global_min_interval: ceiling_interval(max_hz),
            idle_min_interval: if idle_hz > 0.0 {
                Some(ceiling_interval(idle_hz))
            } else {
                None
            },
            active_combat_heartbeat,
        })
    }

    pub const fn global_min_interval(&self) -> Duration {
        self.global_min_interval
    }

    pub const fn idle_min_interval(&self) -> Option<Duration> {
        self.idle_min_interval
    }

    pub const fn active_combat_heartbeat(&self) -> Duration {
        self.active_combat_heartbeat
    }
}

fn ceiling_interval(hz: f64) -> Duration {
    let exact_seconds = 1.0 / hz;
    let rounded = Duration::try_from_secs_f64(exact_seconds).unwrap_or(Duration::MAX);
    if rounded.as_secs_f64() < exact_seconds {
        rounded.saturating_add(Duration::from_nanos(1))
    } else {
        rounded
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TacticalScheduleConfigError {
    #[error("tactical maximum frequency must be finite and greater than zero")]
    InvalidMaxHz,
    #[error("tactical idle frequency must be finite and non-negative")]
    InvalidIdleHz,
    #[error("tactical idle frequency may not exceed the maximum frequency")]
    IdleExceedsMaximum,
    #[error("active-combat heartbeat must be greater than zero")]
    ZeroCombatHeartbeat,
}

/// Permission to start exactly one model call for one captured snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InferencePermit {
    pub inference_id: u64,
    pub snapshot: TacticalSnapshot,
    pub reasons: BTreeSet<TacticalWakeReason>,
    pub started_at: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferralReason {
    InferenceInFlight,
    GlobalRateLimit,
    IdleRateLimit,
}

impl DeferralReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InferenceInFlight => "inference_in_flight",
            Self::GlobalRateLimit => "global_rate_limit",
            Self::IdleRateLimit => "idle_rate_limit",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppressionReason {
    ObserveOnly,
    OlderSnapshot,
    DuplicateSnapshot,
    NoSnapshot,
    NotDue,
}

impl SuppressionReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ObserveOnly => "observe_only",
            Self::OlderSnapshot => "older_snapshot",
            Self::DuplicateSnapshot => "duplicate_snapshot",
            Self::NoSnapshot => "no_snapshot",
            Self::NotDue => "not_due",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TacticalScheduleEffect {
    Start(InferencePermit),
    Deferred {
        reason: DeferralReason,
        eligible_at: Option<Duration>,
        pending_snapshot: TacticalSnapshot,
        coalesced_reasons: BTreeSet<TacticalWakeReason>,
    },
    Suppressed(SuppressionReason),
}

/// Monotonic scheduling facts suitable for metrics and structured logs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TacticalScheduleFacts {
    pub wake_signals: u64,
    pub heartbeats_generated: u64,
    pub inferences_started: u64,
    pub wakes_coalesced: u64,
    pub wakes_suppressed: u64,
    pub rate_limited: u64,
    pub stale_completions: u64,
    pub unexpected_completions: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TacticalScheduleDecision {
    pub effect: TacticalScheduleEffect,
    pub facts: TacticalScheduleFacts,
    pub next_due_at: Option<Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InferenceResultDisposition {
    Current,
    Superseded,
    Unexpected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InferenceCompletion {
    pub disposition: InferenceResultDisposition,
    pub follow_up: TacticalScheduleDecision,
}

#[derive(Debug, Clone)]
struct PendingWake {
    snapshot: TacticalSnapshot,
    reasons: BTreeSet<TacticalWakeReason>,
}

/// Stateful, deterministic tactical scheduling policy.
#[derive(Debug, Clone)]
pub struct TacticalScheduler {
    config: TacticalScheduleConfig,
    rollout_mode: TacticalRolloutMode,
    activity: TacticalActivity,
    latest_snapshot: Option<TacticalSnapshot>,
    pending: Option<PendingWake>,
    in_flight: Option<InferencePermit>,
    last_started_at: Option<Duration>,
    next_heartbeat_at: Option<Duration>,
    next_inference_id: u64,
    facts: TacticalScheduleFacts,
}

impl TacticalScheduler {
    pub fn new(config: TacticalScheduleConfig, rollout_mode: TacticalRolloutMode) -> Self {
        Self {
            config,
            rollout_mode,
            activity: TacticalActivity::Idle,
            latest_snapshot: None,
            pending: None,
            in_flight: None,
            last_started_at: None,
            next_heartbeat_at: None,
            next_inference_id: 1,
            facts: TacticalScheduleFacts::default(),
        }
    }

    pub const fn rollout_mode(&self) -> TacticalRolloutMode {
        self.rollout_mode
    }

    pub fn set_rollout_mode(&mut self, mode: TacticalRolloutMode, now: Duration) {
        self.rollout_mode = mode;
        if !mode.allows_inference() {
            self.pending = None;
        }
        self.arm_heartbeat(now);
    }

    pub const fn activity(&self) -> TacticalActivity {
        self.activity
    }

    pub const fn facts(&self) -> TacticalScheduleFacts {
        self.facts
    }

    pub fn in_flight(&self) -> Option<&InferencePermit> {
        self.in_flight.as_ref()
    }

    pub fn pending_snapshot(&self) -> Option<TacticalSnapshot> {
        self.pending.as_ref().map(|pending| pending.snapshot)
    }

    /// Records a material wake and either starts, defers, coalesces, or
    /// suppresses it. Only the newest revision pair is retained.
    pub fn request(&mut self, now: Duration, wake: TacticalWake) -> TacticalScheduleDecision {
        self.facts.wake_signals = self.facts.wake_signals.saturating_add(1);

        if let Some(latest) = self.latest_snapshot {
            match wake.snapshot.relation_to(latest) {
                SnapshotRelation::Older => {
                    self.facts.wakes_suppressed = self.facts.wakes_suppressed.saturating_add(1);
                    return self.decision(TacticalScheduleEffect::Suppressed(
                        SuppressionReason::OlderSnapshot,
                    ));
                }
                SnapshotRelation::Same if wake.reason != TacticalWakeReason::Heartbeat => {
                    if self.merge_same_snapshot_reason(wake.reason) {
                        return if self.in_flight.is_some() {
                            self.deferred_for_in_flight()
                        } else {
                            self.try_start(now)
                        };
                    }
                    self.facts.wakes_suppressed = self.facts.wakes_suppressed.saturating_add(1);
                    return self.decision(TacticalScheduleEffect::Suppressed(
                        SuppressionReason::DuplicateSnapshot,
                    ));
                }
                SnapshotRelation::Same | SnapshotRelation::Newer => {}
            }
        }

        self.update_activity(now, wake.activity);
        self.latest_snapshot = Some(wake.snapshot);

        if !self.rollout_mode.allows_inference() {
            self.facts.wakes_suppressed = self.facts.wakes_suppressed.saturating_add(1);
            return self.decision(TacticalScheduleEffect::Suppressed(
                SuppressionReason::ObserveOnly,
            ));
        }

        let first_pending_behind_inference = self.in_flight.is_some() && self.pending.is_none();
        self.coalesce_pending(wake.snapshot, wake.reason);
        if first_pending_behind_inference {
            self.facts.wakes_coalesced = self.facts.wakes_coalesced.saturating_add(1);
        }
        self.try_start(now)
    }

    /// Advances heartbeat and deferred work using a caller-supplied monotonic
    /// timestamp. Calling this early has no side effects beyond a suppression
    /// fact.
    pub fn poll(&mut self, now: Duration) -> TacticalScheduleDecision {
        if self.pending.is_some() && self.in_flight.is_none() {
            return self.try_start(now);
        }

        let Some(snapshot) = self.latest_snapshot else {
            self.facts.wakes_suppressed = self.facts.wakes_suppressed.saturating_add(1);
            return self.decision(TacticalScheduleEffect::Suppressed(
                SuppressionReason::NoSnapshot,
            ));
        };

        if self.next_heartbeat_at.is_some_and(|due| now >= due) {
            self.facts.heartbeats_generated = self.facts.heartbeats_generated.saturating_add(1);
            self.advance_heartbeat(now);
            return self.request(
                now,
                TacticalWake {
                    snapshot,
                    activity: self.activity,
                    reason: TacticalWakeReason::Heartbeat,
                },
            );
        }

        self.facts.wakes_suppressed = self.facts.wakes_suppressed.saturating_add(1);
        self.decision(TacticalScheduleEffect::Suppressed(
            SuppressionReason::NotDue,
        ))
    }

    /// Completes the current model call.
    ///
    /// A newer perception frame does not by itself supersede the result. The
    /// body validates the proposed packet against the latest material facts.
    /// A strategic revision change does supersede it because the proposal was
    /// made under direction that is no longer current.
    pub fn complete(&mut self, now: Duration, inference_id: u64) -> InferenceCompletion {
        let Some(completed) = self.in_flight.as_ref() else {
            self.facts.unexpected_completions = self.facts.unexpected_completions.saturating_add(1);
            return InferenceCompletion {
                disposition: InferenceResultDisposition::Unexpected,
                follow_up: self.decision(TacticalScheduleEffect::Suppressed(
                    SuppressionReason::NotDue,
                )),
            };
        };

        if completed.inference_id != inference_id {
            self.facts.unexpected_completions = self.facts.unexpected_completions.saturating_add(1);
            return InferenceCompletion {
                disposition: InferenceResultDisposition::Unexpected,
                follow_up: self.decision(TacticalScheduleEffect::Suppressed(
                    SuppressionReason::NotDue,
                )),
            };
        }

        let Some(completed) = self.in_flight.take() else {
            self.facts.unexpected_completions = self.facts.unexpected_completions.saturating_add(1);
            return InferenceCompletion {
                disposition: InferenceResultDisposition::Unexpected,
                follow_up: self.decision(TacticalScheduleEffect::Suppressed(
                    SuppressionReason::NotDue,
                )),
            };
        };
        let disposition = if self.latest_snapshot.is_some_and(|latest| {
            latest.strategic_revision != completed.snapshot.strategic_revision
        }) {
            self.facts.stale_completions = self.facts.stale_completions.saturating_add(1);
            InferenceResultDisposition::Superseded
        } else {
            InferenceResultDisposition::Current
        };

        let follow_up = if self.pending.is_some() {
            self.try_start(now)
        } else if self.next_heartbeat_at.is_some_and(|due| now >= due) {
            self.poll(now)
        } else {
            self.decision(TacticalScheduleEffect::Suppressed(
                SuppressionReason::NotDue,
            ))
        };

        InferenceCompletion {
            disposition,
            follow_up,
        }
    }

    /// The next timestamp at which polling can produce useful work.
    pub fn next_due_at(&self) -> Option<Duration> {
        let pending_due = if self.pending.is_some() && self.in_flight.is_none() {
            self.rate_limit_eligible_at()
        } else {
            None
        };
        match (pending_due, self.next_heartbeat_at) {
            (Some(left), Some(right)) => Some(left.min(right)),
            (Some(due), None) | (None, Some(due)) => Some(due),
            (None, None) => None,
        }
    }

    fn update_activity(&mut self, now: Duration, activity: TacticalActivity) {
        if self.activity != activity || self.next_heartbeat_at.is_none() {
            self.activity = activity;
            self.arm_heartbeat(now);
        }
    }

    fn arm_heartbeat(&mut self, now: Duration) {
        self.next_heartbeat_at = self.heartbeat_interval().map(|interval| now + interval);
    }

    fn advance_heartbeat(&mut self, now: Duration) {
        let Some(interval) = self.heartbeat_interval() else {
            self.next_heartbeat_at = None;
            return;
        };
        let previous = self.next_heartbeat_at.unwrap_or(now);
        self.next_heartbeat_at = Some(previous.max(now) + interval);
    }

    fn heartbeat_interval(&self) -> Option<Duration> {
        match self.activity {
            TacticalActivity::Idle => self.config.idle_min_interval,
            TacticalActivity::ActiveCombat => Some(self.config.active_combat_heartbeat),
        }
    }

    fn merge_same_snapshot_reason(&mut self, reason: TacticalWakeReason) -> bool {
        if let Some(pending) = self.pending.as_mut() {
            let inserted = pending.reasons.insert(reason);
            if inserted {
                self.facts.wakes_coalesced = self.facts.wakes_coalesced.saturating_add(1);
            }
            return inserted;
        }
        if let Some(in_flight) = self.in_flight.as_ref()
            && !in_flight.reasons.contains(&reason)
        {
            self.pending = Some(PendingWake {
                snapshot: in_flight.snapshot,
                reasons: BTreeSet::from([reason]),
            });
            self.facts.wakes_coalesced = self.facts.wakes_coalesced.saturating_add(1);
            return true;
        }
        false
    }

    fn coalesce_pending(&mut self, snapshot: TacticalSnapshot, reason: TacticalWakeReason) {
        match self.pending.as_mut() {
            Some(pending) if pending.snapshot == snapshot => {
                if pending.reasons.insert(reason) {
                    self.facts.wakes_coalesced = self.facts.wakes_coalesced.saturating_add(1);
                }
            }
            Some(pending) => {
                pending.snapshot = snapshot;
                pending.reasons.clear();
                pending.reasons.insert(reason);
                self.facts.wakes_coalesced = self.facts.wakes_coalesced.saturating_add(1);
            }
            None => {
                self.pending = Some(PendingWake {
                    snapshot,
                    reasons: BTreeSet::from([reason]),
                });
            }
        }
    }

    fn deferred_for_in_flight(&self) -> TacticalScheduleDecision {
        let pending = self.pending.as_ref();
        let in_flight = self
            .in_flight
            .as_ref()
            .expect("deferred_for_in_flight requires an active inference");
        let snapshot = pending.map_or_else(|| in_flight.snapshot, |work| work.snapshot);
        let reasons =
            pending.map_or_else(|| in_flight.reasons.clone(), |work| work.reasons.clone());
        self.decision(TacticalScheduleEffect::Deferred {
            reason: DeferralReason::InferenceInFlight,
            eligible_at: None,
            pending_snapshot: snapshot,
            coalesced_reasons: reasons,
        })
    }

    fn try_start(&mut self, now: Duration) -> TacticalScheduleDecision {
        let pending = self
            .pending
            .as_ref()
            .expect("try_start requires pending work");

        if self.in_flight.is_some() {
            return self.decision(TacticalScheduleEffect::Deferred {
                reason: DeferralReason::InferenceInFlight,
                eligible_at: None,
                pending_snapshot: pending.snapshot,
                coalesced_reasons: pending.reasons.clone(),
            });
        }

        if let Some((reason, eligible_at)) = self.rate_limit(now) {
            self.facts.rate_limited = self.facts.rate_limited.saturating_add(1);
            return self.decision(TacticalScheduleEffect::Deferred {
                reason,
                eligible_at: Some(eligible_at),
                pending_snapshot: pending.snapshot,
                coalesced_reasons: pending.reasons.clone(),
            });
        }

        let pending = self.pending.take().expect("checked above");
        let permit = InferencePermit {
            inference_id: self.next_inference_id,
            snapshot: pending.snapshot,
            reasons: pending.reasons,
            started_at: now,
        };
        self.next_inference_id = self.next_inference_id.saturating_add(1);
        self.last_started_at = Some(now);
        self.facts.inferences_started = self.facts.inferences_started.saturating_add(1);
        self.in_flight = Some(permit.clone());
        self.arm_heartbeat(now);
        self.decision(TacticalScheduleEffect::Start(permit))
    }

    fn rate_limit(&self, now: Duration) -> Option<(DeferralReason, Duration)> {
        let last_started = self.last_started_at?;
        let global_eligible = last_started + self.config.global_min_interval;
        let (reason, eligible_at) = if self.activity == TacticalActivity::Idle {
            if let Some(idle_interval) = self.config.idle_min_interval {
                let idle_eligible = last_started + idle_interval;
                if idle_eligible > global_eligible {
                    (DeferralReason::IdleRateLimit, idle_eligible)
                } else {
                    (DeferralReason::GlobalRateLimit, global_eligible)
                }
            } else {
                (DeferralReason::GlobalRateLimit, global_eligible)
            }
        } else {
            (DeferralReason::GlobalRateLimit, global_eligible)
        };
        (now < eligible_at).then_some((reason, eligible_at))
    }

    fn rate_limit_eligible_at(&self) -> Option<Duration> {
        let last_started = self.last_started_at?;
        let global_eligible = last_started + self.config.global_min_interval;
        Some(match (self.activity, self.config.idle_min_interval) {
            (TacticalActivity::Idle, Some(idle)) => global_eligible.max(last_started + idle),
            _ => global_eligible,
        })
    }

    fn decision(&self, effect: TacticalScheduleEffect) -> TacticalScheduleDecision {
        TacticalScheduleDecision {
            effect,
            facts: self.facts,
            next_due_at: self.next_due_at(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: Duration = Duration::from_secs(1);

    fn scheduler(mode: TacticalRolloutMode) -> TacticalScheduler {
        TacticalScheduler::new(
            TacticalScheduleConfig::from_hz(5.0, 0.2, Duration::from_millis(500)).unwrap(),
            mode,
        )
    }

    fn wake(frame: u64, activity: TacticalActivity, reason: TacticalWakeReason) -> TacticalWake {
        TacticalWake {
            snapshot: TacticalSnapshot {
                frame_revision: frame,
                strategic_revision: 1,
            },
            activity,
            reason,
        }
    }

    fn started(decision: &TacticalScheduleDecision) -> &InferencePermit {
        let TacticalScheduleEffect::Start(permit) = &decision.effect else {
            panic!("expected inference start, got {:?}", decision.effect);
        };
        permit
    }

    #[test]
    fn validates_frequency_ceilings() {
        assert_eq!(
            TacticalScheduleConfig::from_hz(0.0, 0.0, SECOND),
            Err(TacticalScheduleConfigError::InvalidMaxHz)
        );
        assert_eq!(
            TacticalScheduleConfig::from_hz(5.0, 6.0, SECOND),
            Err(TacticalScheduleConfigError::IdleExceedsMaximum)
        );
        assert_eq!(
            TacticalScheduleConfig::from_hz(5.0, 0.0, Duration::ZERO),
            Err(TacticalScheduleConfigError::ZeroCombatHeartbeat)
        );

        let config = TacticalScheduleConfig::from_hz(5.0, 0.2, SECOND).unwrap();
        assert_eq!(config.global_min_interval(), Duration::from_millis(200));
        assert_eq!(config.idle_min_interval(), Some(Duration::from_secs(5)));
    }

    #[test]
    fn rollout_modes_have_explicit_inference_and_release_semantics() {
        assert!(!TacticalRolloutMode::ObserveOnly.allows_inference());
        assert_eq!(
            TacticalRolloutMode::ObserveOnly.packet_release(),
            PacketRelease::RecordOnly
        );
        assert!(TacticalRolloutMode::Shadow.allows_inference());
        assert_eq!(
            TacticalRolloutMode::Shadow.packet_release(),
            PacketRelease::RecordOnly
        );
        assert_eq!(
            TacticalRolloutMode::Controlled.packet_release(),
            PacketRelease::RequireControlGate
        );
        assert_eq!(
            TacticalRolloutMode::Full.packet_release(),
            PacketRelease::Release
        );

        for mode in [
            TacticalRolloutMode::ObserveOnly,
            TacticalRolloutMode::Shadow,
            TacticalRolloutMode::Controlled,
            TacticalRolloutMode::Full,
        ] {
            assert_eq!(mode.to_string().parse(), Ok(mode));
        }
    }

    #[test]
    fn observe_only_records_but_never_starts_inference() {
        let mut schedule = scheduler(TacticalRolloutMode::ObserveOnly);
        let decision = schedule.request(
            Duration::ZERO,
            wake(
                1,
                TacticalActivity::ActiveCombat,
                TacticalWakeReason::CombatStarted,
            ),
        );
        assert_eq!(
            decision.effect,
            TacticalScheduleEffect::Suppressed(SuppressionReason::ObserveOnly)
        );
        assert_eq!(decision.facts.wake_signals, 1);
        assert_eq!(decision.facts.inferences_started, 0);
    }

    #[test]
    fn global_max_hz_defers_new_combat_work_without_dropping_it() {
        let mut schedule = scheduler(TacticalRolloutMode::Shadow);
        let first = schedule.request(
            Duration::ZERO,
            wake(
                1,
                TacticalActivity::ActiveCombat,
                TacticalWakeReason::CombatStarted,
            ),
        );
        let first_id = started(&first).inference_id;
        let completion = schedule.complete(Duration::from_millis(50), first_id);
        assert_eq!(completion.disposition, InferenceResultDisposition::Current);

        let deferred = schedule.request(
            Duration::from_millis(100),
            wake(
                2,
                TacticalActivity::ActiveCombat,
                TacticalWakeReason::DamageTaken,
            ),
        );
        assert_eq!(
            deferred.effect,
            TacticalScheduleEffect::Deferred {
                reason: DeferralReason::GlobalRateLimit,
                eligible_at: Some(Duration::from_millis(200)),
                pending_snapshot: TacticalSnapshot {
                    frame_revision: 2,
                    strategic_revision: 1,
                },
                coalesced_reasons: BTreeSet::from([TacticalWakeReason::DamageTaken]),
            }
        );

        assert_eq!(
            schedule.poll(Duration::from_millis(199)).effect,
            deferred.effect
        );
        let due = schedule.poll(Duration::from_millis(200));
        assert_eq!(started(&due).snapshot.frame_revision, 2);
    }

    #[test]
    fn idle_ceiling_is_stricter_than_global_ceiling() {
        let mut schedule = scheduler(TacticalRolloutMode::Shadow);
        let first = schedule.request(
            Duration::ZERO,
            wake(
                1,
                TacticalActivity::Idle,
                TacticalWakeReason::StrategyChanged,
            ),
        );
        let first_id = started(&first).inference_id;
        schedule.complete(Duration::from_millis(20), first_id);

        let deferred = schedule.request(
            SECOND,
            wake(2, TacticalActivity::Idle, TacticalWakeReason::LootAppeared),
        );
        assert!(matches!(
            deferred.effect,
            TacticalScheduleEffect::Deferred {
                reason: DeferralReason::IdleRateLimit,
                eligible_at: Some(due),
                ..
            } if due == Duration::from_secs(5)
        ));
        assert_eq!(
            started(&schedule.poll(Duration::from_secs(5)))
                .snapshot
                .frame_revision,
            2
        );
    }

    #[test]
    fn active_combat_heartbeat_starts_only_when_due() {
        let mut schedule = scheduler(TacticalRolloutMode::Shadow);
        let initial = schedule.request(
            Duration::ZERO,
            wake(
                7,
                TacticalActivity::ActiveCombat,
                TacticalWakeReason::CombatStarted,
            ),
        );
        let initial_id = started(&initial).inference_id;
        schedule.complete(Duration::from_millis(100), initial_id);

        assert_eq!(
            schedule.poll(Duration::from_millis(499)).effect,
            TacticalScheduleEffect::Suppressed(SuppressionReason::NotDue)
        );
        let heartbeat = schedule.poll(Duration::from_millis(500));
        let permit = started(&heartbeat);
        assert_eq!(permit.snapshot.frame_revision, 7);
        assert_eq!(
            permit.reasons,
            BTreeSet::from([TacticalWakeReason::Heartbeat])
        );
        assert_eq!(heartbeat.facts.heartbeats_generated, 1);
    }

    #[test]
    fn latest_value_replaces_intermediate_frames_during_inference() {
        let mut schedule = scheduler(TacticalRolloutMode::Shadow);
        let first = schedule.request(
            Duration::ZERO,
            wake(
                10,
                TacticalActivity::ActiveCombat,
                TacticalWakeReason::CombatStarted,
            ),
        );
        let first_id = started(&first).inference_id;

        assert!(matches!(
            schedule
                .request(
                    Duration::from_millis(20),
                    wake(
                        11,
                        TacticalActivity::ActiveCombat,
                        TacticalWakeReason::DamageTaken
                    ),
                )
                .effect,
            TacticalScheduleEffect::Deferred {
                reason: DeferralReason::InferenceInFlight,
                pending_snapshot: TacticalSnapshot {
                    frame_revision: 11,
                    ..
                },
                ..
            }
        ));
        schedule.request(
            Duration::from_millis(40),
            wake(
                12,
                TacticalActivity::ActiveCombat,
                TacticalWakeReason::HostileSpawned,
            ),
        );
        assert_eq!(schedule.pending_snapshot().unwrap().frame_revision, 12);

        let completion = schedule.complete(Duration::from_millis(250), first_id);
        assert_eq!(completion.disposition, InferenceResultDisposition::Current);
        assert_eq!(started(&completion.follow_up).snapshot.frame_revision, 12);
        assert_eq!(completion.follow_up.facts.stale_completions, 0);
        assert_eq!(completion.follow_up.facts.wakes_coalesced, 2);
    }

    #[test]
    fn a_new_strategic_revision_supersedes_in_flight_tactical_work() {
        let mut schedule = scheduler(TacticalRolloutMode::Shadow);
        let first = schedule.request(
            Duration::ZERO,
            wake(
                10,
                TacticalActivity::ActiveCombat,
                TacticalWakeReason::CombatStarted,
            ),
        );
        let first_id = started(&first).inference_id;
        schedule.request(
            Duration::from_millis(20),
            TacticalWake {
                snapshot: TacticalSnapshot {
                    frame_revision: 11,
                    strategic_revision: 2,
                },
                activity: TacticalActivity::ActiveCombat,
                reason: TacticalWakeReason::StrategyChanged,
            },
        );

        let completion = schedule.complete(Duration::from_millis(250), first_id);

        assert_eq!(
            completion.disposition,
            InferenceResultDisposition::Superseded
        );
        assert_eq!(completion.follow_up.facts.stale_completions, 1);
        assert_eq!(
            started(&completion.follow_up).snapshot.strategic_revision,
            2
        );
    }

    #[test]
    fn old_and_duplicate_signals_are_suppressed() {
        let mut schedule = scheduler(TacticalRolloutMode::Shadow);
        let first = schedule.request(
            Duration::ZERO,
            wake(
                4,
                TacticalActivity::ActiveCombat,
                TacticalWakeReason::DamageTaken,
            ),
        );
        let first_id = started(&first).inference_id;

        let duplicate = schedule.request(
            Duration::from_millis(10),
            wake(
                4,
                TacticalActivity::ActiveCombat,
                TacticalWakeReason::DamageTaken,
            ),
        );
        assert_eq!(
            duplicate.effect,
            TacticalScheduleEffect::Suppressed(SuppressionReason::DuplicateSnapshot)
        );
        let older = schedule.request(
            Duration::from_millis(20),
            wake(3, TacticalActivity::Idle, TacticalWakeReason::CombatEnded),
        );
        assert_eq!(
            older.effect,
            TacticalScheduleEffect::Suppressed(SuppressionReason::OlderSnapshot)
        );
        assert_eq!(schedule.activity(), TacticalActivity::ActiveCombat);
        assert_eq!(schedule.in_flight().unwrap().inference_id, first_id);
    }

    #[test]
    fn unexpected_completion_does_not_clear_current_inference() {
        let mut schedule = scheduler(TacticalRolloutMode::Shadow);
        let first = schedule.request(
            Duration::ZERO,
            wake(1, TacticalActivity::Idle, TacticalWakeReason::Forced),
        );
        let first_id = started(&first).inference_id;

        let completion = schedule.complete(Duration::from_millis(5), first_id + 99);
        assert_eq!(
            completion.disposition,
            InferenceResultDisposition::Unexpected
        );
        assert_eq!(schedule.in_flight().unwrap().inference_id, first_id);
        assert_eq!(completion.follow_up.facts.unexpected_completions, 1);
    }

    #[test]
    fn zero_idle_hz_disables_idle_heartbeat() {
        let config = TacticalScheduleConfig::from_hz(5.0, 0.0, SECOND).unwrap();
        let mut schedule = TacticalScheduler::new(config, TacticalRolloutMode::Shadow);
        let first = schedule.request(
            Duration::ZERO,
            wake(1, TacticalActivity::Idle, TacticalWakeReason::Forced),
        );
        let first_id = started(&first).inference_id;
        schedule.complete(Duration::from_millis(10), first_id);
        assert_eq!(schedule.next_due_at(), None);
        assert_eq!(
            schedule.poll(Duration::from_mins(1)).effect,
            TacticalScheduleEffect::Suppressed(SuppressionReason::NotDue)
        );
    }
}
