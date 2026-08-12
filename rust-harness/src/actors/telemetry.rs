use std::sync::Arc;

use ractor::{Actor, ActorProcessingErr, ActorRef};

use crate::{
    execution::movement::{MovementTelemetry, NavigationMissionState, NavigationMissionTelemetry},
    execution::outcome::{OutcomeStatus, PacketTerminalStatus},
    observability::{AnalyticsEvent, AnalyticsSink, EventLevel},
    runtime::messages::{TelemetryEvent, TelemetryMsg, TelemetrySnapshot},
    world::perception::PerceptionSummary,
};

pub struct TelemetryActor;

pub struct TelemetryActorArgs {
    pub character_id: String,
    pub sink: Arc<dyn AnalyticsSink>,
}

pub struct TelemetryActorState {
    snapshot: TelemetrySnapshot,
    args: TelemetryActorArgs,
}

impl Actor for TelemetryActor {
    type Msg = TelemetryMsg;
    type State = TelemetryActorState;
    type Arguments = TelemetryActorArgs;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(TelemetryActorState {
            snapshot: TelemetrySnapshot::default(),
            args,
        })
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            TelemetryMsg::Record(event) => {
                update_snapshot(&mut state.snapshot, &event);
                state
                    .args
                    .sink
                    .record(to_analytics_event(&state.args.character_id, &event));
            }
            TelemetryMsg::Snapshot(reply) => {
                if !reply.is_closed() {
                    reply.send(state.snapshot.clone())?;
                }
            }
            TelemetryMsg::Shutdown => myself.stop(Some("player runtime shutdown".to_owned())),
        }
        Ok(())
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "every telemetry event counter is kept in one exhaustive audit point"
)]
fn update_snapshot(snapshot: &mut TelemetrySnapshot, event: &TelemetryEvent) {
    snapshot.events_recorded = snapshot.events_recorded.saturating_add(1);
    match event {
        TelemetryEvent::ActorFailed { .. } => {
            snapshot.actor_failures = snapshot.actor_failures.saturating_add(1);
        }
        TelemetryEvent::StrategyPublished { .. } => {
            snapshot.strategies_published = snapshot.strategies_published.saturating_add(1);
        }
        TelemetryEvent::StrategicInferenceStarted { .. } => {
            snapshot.strategic_inferences_started =
                snapshot.strategic_inferences_started.saturating_add(1);
        }
        TelemetryEvent::StrategicInferenceCoalesced { .. } => {
            snapshot.strategic_inferences_coalesced =
                snapshot.strategic_inferences_coalesced.saturating_add(1);
        }
        TelemetryEvent::StrategicInferenceDeferred { .. } => {
            snapshot.strategic_inferences_deferred =
                snapshot.strategic_inferences_deferred.saturating_add(1);
        }
        TelemetryEvent::StrategicInferenceSuperseded { .. } => {
            snapshot.strategic_inferences_superseded =
                snapshot.strategic_inferences_superseded.saturating_add(1);
        }
        TelemetryEvent::StrategicInferenceCompleted { .. } => {
            snapshot.strategic_inferences_completed =
                snapshot.strategic_inferences_completed.saturating_add(1);
        }
        TelemetryEvent::StrategicInferenceFailed { .. } => {
            snapshot.strategic_inferences_failed =
                snapshot.strategic_inferences_failed.saturating_add(1);
        }
        TelemetryEvent::TacticalWakeRequested { .. } => {
            snapshot.tactical_wakes_requested = snapshot.tactical_wakes_requested.saturating_add(1);
        }
        TelemetryEvent::TacticalWakeSuppressed { .. } => {
            snapshot.tactical_wakes_suppressed =
                snapshot.tactical_wakes_suppressed.saturating_add(1);
        }
        TelemetryEvent::TacticalWakeDeferred { .. } => {
            snapshot.tactical_wakes_deferred = snapshot.tactical_wakes_deferred.saturating_add(1);
        }
        TelemetryEvent::TacticalWakeCoalesced { .. } => {
            snapshot.tactical_wakes_coalesced = snapshot.tactical_wakes_coalesced.saturating_add(1);
        }
        TelemetryEvent::TacticalHeartbeatGenerated { .. } => {
            snapshot.tactical_heartbeats_generated =
                snapshot.tactical_heartbeats_generated.saturating_add(1);
        }
        TelemetryEvent::TacticalDecisionStarted { .. } => {
            snapshot.tactical_inferences_started =
                snapshot.tactical_inferences_started.saturating_add(1);
        }
        TelemetryEvent::TacticalDecisionCompleted { .. } => {
            snapshot.tactical_inferences_completed =
                snapshot.tactical_inferences_completed.saturating_add(1);
        }
        TelemetryEvent::TacticalDecisionSuperseded { .. } => {
            snapshot.tactical_inferences_superseded =
                snapshot.tactical_inferences_superseded.saturating_add(1);
        }
        TelemetryEvent::TacticalDecisionFailed { .. } => {
            snapshot.tactical_inferences_failed =
                snapshot.tactical_inferences_failed.saturating_add(1);
        }
        TelemetryEvent::TacticalPacketReleaseDecided {
            release_policy,
            released,
            ..
        } => {
            snapshot.tactical_packet_release_decisions =
                snapshot.tactical_packet_release_decisions.saturating_add(1);
            match release_policy {
                crate::runtime::tactical_schedule::PacketRelease::RecordOnly => {
                    snapshot.tactical_packets_record_only =
                        snapshot.tactical_packets_record_only.saturating_add(1);
                }
                crate::runtime::tactical_schedule::PacketRelease::RequireControlGate => {
                    snapshot.tactical_packets_control_gated =
                        snapshot.tactical_packets_control_gated.saturating_add(1);
                }
                crate::runtime::tactical_schedule::PacketRelease::Release if *released => {
                    snapshot.tactical_packets_released =
                        snapshot.tactical_packets_released.saturating_add(1);
                }
                crate::runtime::tactical_schedule::PacketRelease::Release => {}
            }
        }
        TelemetryEvent::PacketAccepted { .. } => {
            snapshot.packets_accepted = snapshot.packets_accepted.saturating_add(1);
        }
        TelemetryEvent::PacketRejected { .. } => {
            snapshot.packets_rejected = snapshot.packets_rejected.saturating_add(1);
        }
        TelemetryEvent::PacketTerminal { status, .. } => match status {
            PacketTerminalStatus::Completed => {
                snapshot.packets_completed = snapshot.packets_completed.saturating_add(1);
            }
            PacketTerminalStatus::Failed | PacketTerminalStatus::Aborted => {
                snapshot.packets_failed = snapshot.packets_failed.saturating_add(1);
            }
            PacketTerminalStatus::Cancelled => {
                snapshot.packets_cancelled = snapshot.packets_cancelled.saturating_add(1);
            }
            PacketTerminalStatus::Superseded => {
                snapshot.packets_superseded = snapshot.packets_superseded.saturating_add(1);
            }
        },
        TelemetryEvent::ActionStarted { .. } => {
            snapshot.actions_started = snapshot.actions_started.saturating_add(1);
        }
        TelemetryEvent::ActionTerminal { outcome, .. } => match outcome.status {
            OutcomeStatus::Accepted => {
                snapshot.actions_accepted = snapshot.actions_accepted.saturating_add(1);
            }
            OutcomeStatus::Succeeded => {
                snapshot.actions_succeeded = snapshot.actions_succeeded.saturating_add(1);
            }
            OutcomeStatus::Failed | OutcomeStatus::Rejected => {
                snapshot.actions_failed = snapshot.actions_failed.saturating_add(1);
            }
            _ => {}
        },
        TelemetryEvent::Movement { fact, .. } => match fact {
            MovementTelemetry::Requested { .. } => {
                snapshot.movement_requests = snapshot.movement_requests.saturating_add(1);
            }
            MovementTelemetry::Progress { .. } => {
                snapshot.movement_progress_observations =
                    snapshot.movement_progress_observations.saturating_add(1);
            }
            MovementTelemetry::Arrival { .. } | MovementTelemetry::SceneTransition { .. } => {
                snapshot.movement_arrivals = snapshot.movement_arrivals.saturating_add(1);
            }
            MovementTelemetry::Stall { .. } => {
                snapshot.movement_stalls = snapshot.movement_stalls.saturating_add(1);
            }
            MovementTelemetry::Stop { succeeded, .. } => {
                snapshot.movement_stops = snapshot.movement_stops.saturating_add(1);
                if !succeeded {
                    snapshot.movement_stop_failures =
                        snapshot.movement_stop_failures.saturating_add(1);
                }
            }
        },
        TelemetryEvent::NavigationMission { fact, .. } => match fact {
            NavigationMissionTelemetry::Started { .. } => {
                snapshot.navigation_missions_started =
                    snapshot.navigation_missions_started.saturating_add(1);
            }
            NavigationMissionTelemetry::AttemptStarted { attempt_kind, .. } => {
                snapshot.navigation_attempts = snapshot.navigation_attempts.saturating_add(1);
                match attempt_kind {
                    crate::execution::movement::NavigationAttemptKind::MoveTo => {
                        snapshot.navigation_move_to_attempts =
                            snapshot.navigation_move_to_attempts.saturating_add(1);
                    }
                    crate::execution::movement::NavigationAttemptKind::EnterDoor => {
                        snapshot.navigation_door_attempts =
                            snapshot.navigation_door_attempts.saturating_add(1);
                    }
                    _ => {}
                }
            }
            NavigationMissionTelemetry::Paused { reason_code, .. }
                if reason_code == "tactical_preemption" =>
            {
                snapshot.navigation_preemptions = snapshot.navigation_preemptions.saturating_add(1);
            }
            NavigationMissionTelemetry::RetryScheduled { .. } => {
                snapshot.navigation_retries = snapshot.navigation_retries.saturating_add(1);
            }
            NavigationMissionTelemetry::Terminal { state, .. } => match state {
                NavigationMissionState::Arrived => {
                    snapshot.navigation_missions_arrived =
                        snapshot.navigation_missions_arrived.saturating_add(1);
                }
                NavigationMissionState::Failed => {
                    snapshot.navigation_missions_failed =
                        snapshot.navigation_missions_failed.saturating_add(1);
                }
                NavigationMissionState::Superseded => {
                    snapshot.navigation_missions_superseded =
                        snapshot.navigation_missions_superseded.saturating_add(1);
                }
                _ => {}
            },
            _ => {}
        },
        _ => {}
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "the exhaustive event mapping is one stable, auditable telemetry interface"
)]
fn to_analytics_event(character_id: &str, event: &TelemetryEvent) -> AnalyticsEvent {
    match event {
        TelemetryEvent::ActorStarted(actor) => {
            AnalyticsEvent::new("actor.started", EventLevel::Info)
                .character(character_id)
                .attribute("actor", format!("{actor:?}"))
        }
        TelemetryEvent::ActorFailed { actor, reason } => {
            AnalyticsEvent::new("actor.failed", EventLevel::Error)
                .character(character_id)
                .attribute("actor", format!("{actor:?}"))
                .attribute("reason", reason.clone())
        }
        TelemetryEvent::ActorTerminated { actor, reason } => {
            let mut event = AnalyticsEvent::new("actor.terminated", EventLevel::Warn)
                .character(character_id)
                .attribute("actor", format!("{actor:?}"));
            if let Some(reason) = reason {
                event = event.attribute("reason", reason.clone());
            }
            event
        }
        TelemetryEvent::FramePublished {
            observation_cycle_id,
            observation_cycle_sequence,
            frame_revision,
            perception_revision,
            strategic_revision,
            inventory_revision,
            map_revision,
            summary,
        } => frame_published_event(
            character_id,
            [
                *frame_revision,
                *perception_revision,
                *strategic_revision,
                *inventory_revision,
                *map_revision,
            ],
            summary,
            *observation_cycle_id,
            *observation_cycle_sequence,
        ),
        TelemetryEvent::PerceptionRejected {
            observation_cycle_id,
            observation_cycle_sequence,
            error_class,
        } => AnalyticsEvent::new("perception.snapshot_rejected", EventLevel::Warn)
            .character(character_id)
            .correlation(*observation_cycle_id)
            .attribute("observation_cycle_id", observation_cycle_id.to_string())
            .attribute("observation_cycle_sequence", *observation_cycle_sequence)
            .attribute("error_class", error_class.clone()),
        TelemetryEvent::StrategyPublished {
            decision_id,
            input_revision,
            revision,
            objective_chars,
            subgoal_count,
            priority_count,
            constraint_count,
            preferred_target_count,
            navigation_scene,
            navigation_tile_known,
        } => AnalyticsEvent::new("strategy.published", EventLevel::Info)
            .character(character_id)
            .correlation(*decision_id)
            .attribute("decision_id", decision_id.to_string())
            .attribute("input_revision", *input_revision)
            .attribute("strategic_revision", *revision)
            .attribute(
                "objective_chars",
                u64::try_from(*objective_chars).unwrap_or(u64::MAX),
            )
            .attribute(
                "subgoal_count",
                u64::try_from(*subgoal_count).unwrap_or(u64::MAX),
            )
            .attribute(
                "priority_count",
                u64::try_from(*priority_count).unwrap_or(u64::MAX),
            )
            .attribute(
                "constraint_count",
                u64::try_from(*constraint_count).unwrap_or(u64::MAX),
            )
            .attribute(
                "preferred_target_count",
                u64::try_from(*preferred_target_count).unwrap_or(u64::MAX),
            )
            .attribute(
                "navigation_scene",
                navigation_scene.as_deref().unwrap_or(""),
            )
            .attribute("navigation_tile_known", *navigation_tile_known),
        TelemetryEvent::StrategicRecallStarted {
            recall_id,
            input_revision,
            base_strategic_revision,
            query_chars,
        } => strategic_event(
            "strategic.recall_started",
            EventLevel::Debug,
            character_id,
            *recall_id,
            *input_revision,
            *base_strategic_revision,
        )
        .attribute(
            "query_chars",
            u64::try_from(*query_chars).unwrap_or(u64::MAX),
        ),
        TelemetryEvent::StrategicRecallCompleted {
            recall_id,
            input_revision,
            base_strategic_revision,
            duration_ms,
            semantic_count,
            relationship_count,
            episode_count,
            plan_step_count,
        } => strategic_event(
            "strategic.recall_completed",
            EventLevel::Debug,
            character_id,
            *recall_id,
            *input_revision,
            *base_strategic_revision,
        )
        .attribute("duration_ms", *duration_ms)
        .attribute(
            "semantic_count",
            u64::try_from(*semantic_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "relationship_count",
            u64::try_from(*relationship_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "episode_count",
            u64::try_from(*episode_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "plan_step_count",
            u64::try_from(*plan_step_count).unwrap_or(u64::MAX),
        ),
        TelemetryEvent::StrategicRecallFailed {
            recall_id,
            input_revision,
            base_strategic_revision,
            duration_ms,
            error_class,
        } => strategic_event(
            "strategic.recall_failed",
            EventLevel::Warn,
            character_id,
            *recall_id,
            *input_revision,
            *base_strategic_revision,
        )
        .attribute("duration_ms", *duration_ms)
        .attribute("error_class", error_class.clone()),
        TelemetryEvent::StrategicPlanChanged {
            decision_id,
            plan_revision,
            step_count,
            retained_step_count,
            blocked,
            completion_claimed,
        } => AnalyticsEvent::new("strategic.plan_changed", EventLevel::Info)
            .character(character_id)
            .correlation(*decision_id)
            .attribute("decision_id", decision_id.to_string())
            .attribute("plan_revision", *plan_revision)
            .attribute("step_count", u64::try_from(*step_count).unwrap_or(u64::MAX))
            .attribute(
                "retained_step_count",
                u64::try_from(*retained_step_count).unwrap_or(u64::MAX),
            )
            .attribute("blocked", *blocked)
            .attribute("completion_claimed", *completion_claimed),
        TelemetryEvent::StrategicPlanStepAdvanced {
            correlation_id,
            plan_revision,
            transition,
            tries,
            evidence_count,
        } => AnalyticsEvent::new("strategic.plan_step_advanced", EventLevel::Info)
            .character(character_id)
            .correlation(*correlation_id)
            .attribute("correlation_id", correlation_id.to_string())
            .attribute("plan_revision", *plan_revision)
            .attribute("transition", transition.clone())
            .attribute("tries", u64::from(*tries))
            .attribute(
                "evidence_count",
                u64::try_from(*evidence_count).unwrap_or(u64::MAX),
            ),
        TelemetryEvent::StrategicNavigationArrivalObserved {
            mission_id,
            decision_id,
            strategic_revision,
            destination_scene,
            arrived_scene,
            destination_tile_known,
            arrived_tile_known,
            attempts,
        } => AnalyticsEvent::new("strategic.navigation_arrival_observed", EventLevel::Info)
            .character(character_id)
            .correlation(*mission_id)
            .attribute("mission_id", mission_id.to_string())
            .attribute("decision_id", decision_id.to_string())
            .attribute("strategic_revision", *strategic_revision)
            .attribute("destination_scene", destination_scene.clone())
            .attribute("arrived_scene", arrived_scene.as_deref().unwrap_or(""))
            .attribute("destination_tile_known", *destination_tile_known)
            .attribute("arrived_tile_known", *arrived_tile_known)
            .attribute("attempts", u64::from(*attempts)),
        TelemetryEvent::StrategicInferenceStarted {
            decision_id,
            input_revision,
            base_strategic_revision,
            moment_count,
            frame_revision,
            scene,
            visible_entity_count,
            visible_hostile_count,
            exit_count,
            recent_scene_transition_count,
            consecutive_failures_before_call,
            last_successful_inference_age_ms,
        } => strategic_event(
            "strategic.inference_started",
            EventLevel::Info,
            character_id,
            *decision_id,
            *input_revision,
            *base_strategic_revision,
        )
        .attribute(
            "moment_count",
            u64::try_from(*moment_count).unwrap_or(u64::MAX),
        )
        .attribute("frame_revision", *frame_revision)
        .attribute("scene", scene.as_deref().unwrap_or(""))
        .attribute(
            "visible_entity_count",
            u64::try_from(*visible_entity_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "visible_hostile_count",
            u64::try_from(*visible_hostile_count).unwrap_or(u64::MAX),
        )
        .attribute("exit_count", u64::try_from(*exit_count).unwrap_or(u64::MAX))
        .attribute(
            "recent_scene_transition_count",
            u64::try_from(*recent_scene_transition_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "consecutive_failures_before_call",
            u64::from(*consecutive_failures_before_call),
        )
        .attribute(
            "last_successful_inference_known",
            last_successful_inference_age_ms.is_some(),
        )
        .attribute(
            "last_successful_inference_age_ms",
            last_successful_inference_age_ms.unwrap_or(0),
        ),
        TelemetryEvent::StrategicInferenceCoalesced {
            decision_id,
            active_input_revision,
            base_strategic_revision,
            pending_input_revision,
            pending_moment_count,
        } => strategic_event(
            "strategic.inference_coalesced",
            EventLevel::Debug,
            character_id,
            *decision_id,
            *active_input_revision,
            *base_strategic_revision,
        )
        .attribute("pending_input_revision", *pending_input_revision)
        .attribute(
            "pending_moment_count",
            u64::try_from(*pending_moment_count).unwrap_or(u64::MAX),
        ),
        TelemetryEvent::StrategicInferenceDeferred {
            schedule_id,
            input_revision,
            base_strategic_revision,
            pending_moment_count,
            eligible_after_ms,
        } => AnalyticsEvent::new("strategic.inference_deferred", EventLevel::Debug)
            .character(character_id)
            .correlation(*schedule_id)
            .attribute("schedule_id", schedule_id.to_string())
            .attribute("input_revision", *input_revision)
            .attribute("base_strategic_revision", *base_strategic_revision)
            .attribute(
                "pending_moment_count",
                u64::try_from(*pending_moment_count).unwrap_or(u64::MAX),
            )
            .attribute("eligible_after_ms", *eligible_after_ms),
        TelemetryEvent::StrategicInferenceSuperseded {
            decision_id,
            input_revision,
            base_strategic_revision,
            duration_ms,
            reason_code,
        } => strategic_event(
            "strategic.inference_superseded",
            EventLevel::Info,
            character_id,
            *decision_id,
            *input_revision,
            *base_strategic_revision,
        )
        .attribute("duration_ms", *duration_ms)
        .attribute("reason_code", reason_code.clone()),
        TelemetryEvent::StrategicInferenceCompleted {
            decision_id,
            input_revision,
            base_strategic_revision,
            published_revision,
            duration_ms,
            newer_input_pending,
            speech_suppressed_as_stale,
            interaction_suppressed_as_stale,
        } => strategic_event(
            "strategic.inference_completed",
            EventLevel::Info,
            character_id,
            *decision_id,
            *input_revision,
            *base_strategic_revision,
        )
        .attribute("duration_ms", *duration_ms)
        .attribute("published", published_revision.is_some())
        .attribute("published_revision", published_revision.unwrap_or(0))
        .attribute("newer_input_pending", *newer_input_pending)
        .attribute("speech_suppressed_as_stale", *speech_suppressed_as_stale)
        .attribute(
            "interaction_suppressed_as_stale",
            *interaction_suppressed_as_stale,
        ),
        TelemetryEvent::StrategicInferenceFailed {
            decision_id,
            input_revision,
            base_strategic_revision,
            duration_ms,
            error_class,
            consecutive_failures,
            retry_after_ms,
            last_successful_inference_age_ms,
            previous_intent_retained,
        } => strategic_event(
            "strategic.inference_failed",
            EventLevel::Warn,
            character_id,
            *decision_id,
            *input_revision,
            *base_strategic_revision,
        )
        .attribute("duration_ms", *duration_ms)
        .attribute("error_class", error_class.clone())
        .attribute("consecutive_failures", u64::from(*consecutive_failures))
        .attribute("retry_after_ms", *retry_after_ms)
        .attribute(
            "last_successful_inference_known",
            last_successful_inference_age_ms.is_some(),
        )
        .attribute(
            "last_successful_inference_age_ms",
            last_successful_inference_age_ms.unwrap_or(0),
        )
        .attribute("previous_intent_retained", *previous_intent_retained),
        TelemetryEvent::TacticalWakeRequested {
            signal_id,
            frame_revision,
            strategic_revision,
            reason,
            activity,
        } => AnalyticsEvent::new("tactical.wake_requested", EventLevel::Debug)
            .character(character_id)
            .correlation(*signal_id)
            .attribute("signal_id", signal_id.to_string())
            .attribute("frame_revision", *frame_revision)
            .attribute("strategic_revision", *strategic_revision)
            .attribute("wake_reason", reason.as_str())
            .attribute("activity", activity.as_str()),
        TelemetryEvent::TacticalWakeSuppressed {
            signal_id,
            frame_revision,
            strategic_revision,
            reason,
        } => AnalyticsEvent::new("tactical.wake_suppressed", EventLevel::Debug)
            .character(character_id)
            .correlation(*signal_id)
            .attribute("signal_id", signal_id.to_string())
            .attribute("frame_revision", *frame_revision)
            .attribute("strategic_revision", *strategic_revision)
            .attribute("suppression_reason", reason.as_str()),
        TelemetryEvent::TacticalWakeDeferred {
            signal_id,
            frame_revision,
            strategic_revision,
            reason,
            eligible_after_ms,
            coalesced_reason_count,
        } => AnalyticsEvent::new("tactical.wake_deferred", EventLevel::Debug)
            .character(character_id)
            .correlation(*signal_id)
            .attribute("signal_id", signal_id.to_string())
            .attribute("frame_revision", *frame_revision)
            .attribute("strategic_revision", *strategic_revision)
            .attribute("deferral_reason", reason.as_str())
            .attribute("eligible_after_ms_known", eligible_after_ms.is_some())
            .attribute("eligible_after_ms", eligible_after_ms.unwrap_or(0))
            .attribute(
                "coalesced_reason_count",
                u64::try_from(*coalesced_reason_count).unwrap_or(u64::MAX),
            ),
        TelemetryEvent::TacticalWakeCoalesced {
            signal_id,
            frame_revision,
            strategic_revision,
            pending_frame_revision,
            pending_strategic_revision,
            coalesced_reason_count,
        } => AnalyticsEvent::new("tactical.wake_coalesced", EventLevel::Debug)
            .character(character_id)
            .correlation(*signal_id)
            .attribute("signal_id", signal_id.to_string())
            .attribute("frame_revision", *frame_revision)
            .attribute("strategic_revision", *strategic_revision)
            .attribute("pending_frame_revision", *pending_frame_revision)
            .attribute("pending_strategic_revision", *pending_strategic_revision)
            .attribute(
                "coalesced_reason_count",
                u64::try_from(*coalesced_reason_count).unwrap_or(u64::MAX),
            ),
        TelemetryEvent::TacticalHeartbeatGenerated {
            signal_id,
            frame_revision,
            strategic_revision,
            activity,
        } => AnalyticsEvent::new("tactical.heartbeat_generated", EventLevel::Debug)
            .character(character_id)
            .correlation(*signal_id)
            .attribute("signal_id", signal_id.to_string())
            .attribute("frame_revision", *frame_revision)
            .attribute("strategic_revision", *strategic_revision)
            .attribute("activity", activity.as_str()),
        TelemetryEvent::TacticalDecisionStarted {
            trigger_signal_id,
            decision_id,
            scheduler_inference_id,
            frame_revision,
            strategic_revision,
            wake_reasons,
        } => AnalyticsEvent::new("tactical.inference_started", EventLevel::Info)
            .character(character_id)
            .correlation(*decision_id)
            .attribute("trigger_signal_id", trigger_signal_id.to_string())
            .attribute("decision_id", decision_id.to_string())
            .attribute("scheduler_inference_id", *scheduler_inference_id)
            .attribute("frame_revision", *frame_revision)
            .attribute("strategic_revision", *strategic_revision)
            .attribute(
                "wake_reason_count",
                u64::try_from(wake_reasons.len()).unwrap_or(u64::MAX),
            )
            .attribute(
                "wake_reasons",
                wake_reasons
                    .iter()
                    .map(|reason| reason.as_str())
                    .collect::<Vec<_>>()
                    .join("|"),
            ),
        TelemetryEvent::TacticalDecisionSuperseded {
            decision_id,
            frame_revision,
            strategic_revision,
            duration_ms,
            reason_code,
        } => AnalyticsEvent::new("tactical.inference_superseded", EventLevel::Info)
            .character(character_id)
            .correlation(*decision_id)
            .attribute("decision_id", decision_id.to_string())
            .attribute("frame_revision", *frame_revision)
            .attribute("strategic_revision", *strategic_revision)
            .attribute("duration_ms", *duration_ms)
            .attribute("reason_code", reason_code.clone()),
        TelemetryEvent::TacticalDecisionCompleted {
            decision_id,
            frame_revision,
            strategic_revision,
            action_count,
            action_plan,
            intent,
            duration_ms,
        } => AnalyticsEvent::new("tactical.inference_completed", EventLevel::Info)
            .character(character_id)
            .correlation(*decision_id)
            .attribute("decision_id", decision_id.to_string())
            .attribute("frame_revision", *frame_revision)
            .attribute("strategic_revision", *strategic_revision)
            .attribute("duration_ms", *duration_ms)
            .attribute("intent", intent.as_str())
            .attribute("action_plan", action_plan.clone())
            .attribute(
                "action_count",
                u64::try_from(*action_count).unwrap_or(u64::MAX),
            ),
        TelemetryEvent::TacticalDecisionFailed {
            decision_id,
            frame_revision,
            strategic_revision,
            duration_ms,
            error_class,
        } => AnalyticsEvent::new("tactical.inference_failed", EventLevel::Warn)
            .character(character_id)
            .correlation(*decision_id)
            .attribute("decision_id", decision_id.to_string())
            .attribute("frame_revision", *frame_revision)
            .attribute("strategic_revision", *strategic_revision)
            .attribute("duration_ms", *duration_ms)
            .attribute("error_class", error_class.clone()),
        TelemetryEvent::TacticalPacketReleaseDecided {
            decision_id,
            packet_id,
            frame_revision,
            strategic_revision,
            rollout_mode,
            release_policy,
            action_count,
            intent,
            released,
            reason_code,
        } => AnalyticsEvent::new("tactical.packet_release_decided", EventLevel::Info)
            .character(character_id)
            .correlation(*decision_id)
            .attribute("decision_id", decision_id.to_string())
            .attribute("packet_id", packet_id.to_string())
            .attribute("frame_revision", *frame_revision)
            .attribute("strategic_revision", *strategic_revision)
            .attribute("rollout_mode", rollout_mode.clone())
            .attribute("release_policy", release_policy.as_str())
            .attribute("intent", intent.as_str())
            .attribute(
                "action_count",
                u64::try_from(*action_count).unwrap_or(u64::MAX),
            )
            .attribute("released", *released)
            .attribute("reason_code", reason_code.clone()),
        TelemetryEvent::PacketAccepted {
            packet_id,
            decision_id,
            frame_revision,
            strategic_revision,
        } => AnalyticsEvent::new("body.packet_accepted", EventLevel::Info)
            .character(character_id)
            .correlation(*packet_id)
            .attribute("packet_id", packet_id.to_string())
            .attribute("decision_id", decision_id.to_string())
            .attribute("frame_revision", *frame_revision)
            .attribute("strategic_revision", *strategic_revision),
        TelemetryEvent::PacketRejected {
            packet_id,
            decision_id,
            frame_revision,
            strategic_revision,
            reason,
        } => AnalyticsEvent::new("body.packet_rejected", EventLevel::Warn)
            .character(character_id)
            .correlation(*packet_id)
            .attribute("packet_id", packet_id.to_string())
            .attribute("decision_id", decision_id.to_string())
            .attribute("frame_revision", *frame_revision)
            .attribute("strategic_revision", *strategic_revision)
            .attribute("reason", reason.clone()),
        TelemetryEvent::PacketTerminal {
            packet_id,
            decision_id,
            frame_revision,
            strategic_revision,
            status,
            reason_code,
            superseded_by,
        } => {
            let mut event = AnalyticsEvent::new(
                format!("body.packet_{}", status.as_str()),
                if matches!(status, PacketTerminalStatus::Completed) {
                    EventLevel::Info
                } else {
                    EventLevel::Warn
                },
            )
            .character(character_id)
            .correlation(*packet_id)
            .attribute("packet_id", packet_id.to_string())
            .attribute("decision_id", decision_id.to_string())
            .attribute("frame_revision", *frame_revision)
            .attribute("strategic_revision", *strategic_revision)
            .attribute("status", status.as_str());
            if let Some(reason_code) = reason_code {
                event = event.attribute("reason_code", reason_code.clone());
            }
            if let Some(superseded_by) = superseded_by {
                event = event.attribute("superseded_by", superseded_by.to_string());
            }
            event
        }
        TelemetryEvent::ActionStarted {
            context,
            action_kind,
        } => execution_event(
            "body.action_started",
            EventLevel::Info,
            character_id,
            context,
        )
        .attribute("action_kind", action_kind.clone()),
        TelemetryEvent::ActionTerminal {
            outcome,
            session_generation,
        } => execution_event(
            match outcome.status {
                OutcomeStatus::Succeeded => "body.action_succeeded",
                OutcomeStatus::Failed => "body.action_failed",
                OutcomeStatus::Rejected => "body.action_rejected",
                OutcomeStatus::Cancelled => "body.action_cancelled",
                OutcomeStatus::Superseded => "body.action_superseded",
                OutcomeStatus::Accepted => "body.action_accepted",
            },
            if matches!(
                outcome.status,
                OutcomeStatus::Accepted | OutcomeStatus::Succeeded
            ) {
                EventLevel::Info
            } else {
                EventLevel::Warn
            },
            character_id,
            &crate::execution::gateway::ExecutionContext {
                session_generation: *session_generation,
                decision_id: outcome.decision_id,
                packet_id: outcome.packet_id,
                action_id: outcome.action_id,
                action_index: outcome.action_index,
                frame_revision: outcome.source_frame_revision,
                strategic_revision: outcome.strategic_revision,
            },
        )
        .attribute("action_kind", outcome.action_kind.clone())
        .attribute(
            "status",
            format!("{:?}", outcome.status).to_ascii_lowercase(),
        )
        .attribute("duration_ms", outcome.duration_ms)
        .attribute(
            "resulting_frame_revision_known",
            outcome.resulting_frame_revision.is_some(),
        )
        .attribute(
            "resulting_frame_revision",
            outcome.resulting_frame_revision.unwrap_or(0),
        )
        .attribute(
            "reason_code",
            outcome.reason_code.clone().unwrap_or_default(),
        ),
        TelemetryEvent::Movement { context, fact } => movement_event(character_id, context, fact),
        TelemetryEvent::NavigationMission { decision_id, fact } => {
            navigation_mission_event(character_id, *decision_id, fact)
        }
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "every navigation fact maps to one explicit structured analytics event"
)]
fn navigation_mission_event(
    character_id: &str,
    decision_id: uuid::Uuid,
    fact: &NavigationMissionTelemetry,
) -> AnalyticsEvent {
    let base = |name, level, mission_id: uuid::Uuid| {
        AnalyticsEvent::new(name, level)
            .character(character_id)
            .correlation(mission_id)
            .attribute("decision_id", decision_id.to_string())
            .attribute("mission_id", mission_id.to_string())
    };
    match fact {
        NavigationMissionTelemetry::Started {
            mission_id,
            recorded_at,
            destination_scene,
            destination_tile,
            route_waypoints,
        } => base(
            "body.navigation_mission_started",
            EventLevel::Info,
            *mission_id,
        )
        .attribute("recorded_at", recorded_at.to_rfc3339())
        .attribute("destination_scene", destination_scene.clone())
        .attribute(
            "destination_tile_x",
            destination_tile.map_or(0, |tile| tile.x),
        )
        .attribute(
            "destination_tile_y",
            destination_tile.map_or(0, |tile| tile.y),
        )
        .attribute("destination_tile_known", destination_tile.is_some())
        .attribute(
            "route_waypoints",
            u64::try_from(*route_waypoints).unwrap_or(u64::MAX),
        ),
        NavigationMissionTelemetry::AttemptStarted {
            mission_id,
            attempt_id,
            recorded_at,
            attempt_number,
            attempt_kind,
            scene,
            target_tile,
        } => base(
            "body.navigation_attempt_started",
            EventLevel::Info,
            *mission_id,
        )
        .attribute("attempt_id", attempt_id.to_string())
        .attribute("recorded_at", recorded_at.to_rfc3339())
        .attribute("attempt_number", u64::from(*attempt_number))
        .attribute(
            "attempt_kind",
            format!("{attempt_kind:?}").to_ascii_lowercase(),
        )
        .attribute("scene", scene.as_deref().unwrap_or(""))
        .attribute("target_tile_x", target_tile.x)
        .attribute("target_tile_y", target_tile.y),
        NavigationMissionTelemetry::Paused {
            mission_id,
            recorded_at,
            reason_code,
        } => base(
            "body.navigation_mission_paused",
            EventLevel::Info,
            *mission_id,
        )
        .attribute("recorded_at", recorded_at.to_rfc3339())
        .attribute("reason_code", reason_code.clone()),
        NavigationMissionTelemetry::Resumed {
            mission_id,
            recorded_at,
            scene,
            attempt_number,
        } => base(
            "body.navigation_mission_resumed",
            EventLevel::Info,
            *mission_id,
        )
        .attribute("recorded_at", recorded_at.to_rfc3339())
        .attribute("scene", scene.as_deref().unwrap_or(""))
        .attribute("attempt_number", u64::from(*attempt_number)),
        NavigationMissionTelemetry::DuplicateSuppressed {
            mission_id,
            recorded_at,
            strategic_revision,
        } => base(
            "body.navigation_duplicate_suppressed",
            EventLevel::Debug,
            *mission_id,
        )
        .attribute("recorded_at", recorded_at.to_rfc3339())
        .attribute("strategic_revision", *strategic_revision),
        NavigationMissionTelemetry::WaypointReached {
            mission_id,
            recorded_at,
            waypoint_index,
            scene,
            position_tile,
        } => base(
            "body.navigation_waypoint_reached",
            EventLevel::Info,
            *mission_id,
        )
        .attribute("recorded_at", recorded_at.to_rfc3339())
        .attribute(
            "waypoint_index",
            u64::try_from(*waypoint_index).unwrap_or(u64::MAX),
        )
        .attribute("scene", scene.as_deref().unwrap_or(""))
        .attribute("position_tile_x", position_tile.map_or(0, |tile| tile.x))
        .attribute("position_tile_y", position_tile.map_or(0, |tile| tile.y)),
        NavigationMissionTelemetry::RetryScheduled {
            mission_id,
            recorded_at,
            attempt_number,
            reason_code,
        } => base(
            "body.navigation_retry_scheduled",
            EventLevel::Warn,
            *mission_id,
        )
        .attribute("recorded_at", recorded_at.to_rfc3339())
        .attribute("attempt_number", u64::from(*attempt_number))
        .attribute("reason_code", reason_code.clone()),
        NavigationMissionTelemetry::Terminal {
            mission_id,
            recorded_at,
            state,
            reason_code,
            scene,
            position_tile,
            attempts,
        } => base(
            "body.navigation_mission_terminal",
            if *state == NavigationMissionState::Arrived {
                EventLevel::Info
            } else {
                EventLevel::Warn
            },
            *mission_id,
        )
        .attribute("recorded_at", recorded_at.to_rfc3339())
        .attribute("state", format!("{state:?}").to_ascii_lowercase())
        .attribute("reason_code", reason_code.clone().unwrap_or_default())
        .attribute("scene", scene.as_deref().unwrap_or(""))
        .attribute("position_tile_x", position_tile.map_or(0, |tile| tile.x))
        .attribute("position_tile_y", position_tile.map_or(0, |tile| tile.y))
        .attribute("attempts", u64::from(*attempts)),
    }
}

fn strategic_event(
    name: &'static str,
    level: EventLevel,
    character_id: &str,
    decision_id: uuid::Uuid,
    input_revision: u64,
    base_strategic_revision: u64,
) -> AnalyticsEvent {
    AnalyticsEvent::new(name, level)
        .character(character_id)
        .correlation(decision_id)
        .attribute("decision_id", decision_id.to_string())
        .attribute("input_revision", input_revision)
        .attribute("base_strategic_revision", base_strategic_revision)
}

fn movement_event(
    character_id: &str,
    context: &crate::execution::gateway::ExecutionContext,
    fact: &MovementTelemetry,
) -> AnalyticsEvent {
    match fact {
        MovementTelemetry::Requested {
            requested_at,
            origin_tile,
            destination_tile,
        } => movement_requested_event(
            character_id,
            context,
            *requested_at,
            *origin_tile,
            *destination_tile,
        ),
        MovementTelemetry::Progress {
            observed_at,
            frame_revision,
            position_tile,
            distance_from_previous_millipixels,
            observed_distance_millipixels,
            remaining_tile_distance,
        } => movement_progress_event(
            character_id,
            context,
            *observed_at,
            *frame_revision,
            *position_tile,
            *distance_from_previous_millipixels,
            *observed_distance_millipixels,
            *remaining_tile_distance,
        ),
        MovementTelemetry::Arrival {
            observed_at,
            frame_revision,
            position_tile,
            evidence,
        } => movement_arrival_event(
            character_id,
            context,
            *observed_at,
            *frame_revision,
            *position_tile,
            *evidence,
        ),
        MovementTelemetry::SceneTransition {
            observed_at,
            frame_revision,
            from_scene,
            to_scene,
            position_tile,
        } => movement_scene_transition_event(
            character_id,
            context,
            *observed_at,
            *frame_revision,
            from_scene.as_deref(),
            to_scene.as_deref(),
            *position_tile,
        ),
        MovementTelemetry::Stall {
            observed_at,
            frame_revision,
            position_tile,
            observations_without_progress,
        } => movement_stall_event(
            character_id,
            context,
            *observed_at,
            *frame_revision,
            *position_tile,
            *observations_without_progress,
        ),
        MovementTelemetry::Stop {
            recorded_at,
            stop_action_id,
            reason_code,
            succeeded,
        } => execution_event(
            "body.movement_stop",
            if *succeeded {
                EventLevel::Info
            } else {
                EventLevel::Warn
            },
            character_id,
            context,
        )
        .attribute("recorded_at", recorded_at.to_rfc3339())
        .attribute("stop_action_id", stop_action_id.to_string())
        .attribute("reason_code", reason_code.clone())
        .attribute("succeeded", *succeeded),
    }
}

fn movement_stall_event(
    character_id: &str,
    context: &crate::execution::gateway::ExecutionContext,
    observed_at: chrono::DateTime<chrono::Utc>,
    frame_revision: Option<u64>,
    position_tile: Option<crate::world::TilePosition>,
    observations_without_progress: u32,
) -> AnalyticsEvent {
    execution_event(
        "body.movement_stall",
        EventLevel::Warn,
        character_id,
        context,
    )
    .attribute("observed_at", observed_at.to_rfc3339())
    .attribute("observed_frame_revision_known", frame_revision.is_some())
    .attribute("observed_frame_revision", frame_revision.unwrap_or(0))
    .attribute("position_known", position_tile.is_some())
    .attribute(
        "tile_x",
        position_tile.map_or(0, |position| i64::from(position.x)),
    )
    .attribute(
        "tile_y",
        position_tile.map_or(0, |position| i64::from(position.y)),
    )
    .attribute(
        "observations_without_progress",
        u64::from(observations_without_progress),
    )
}

fn movement_arrival_event(
    character_id: &str,
    context: &crate::execution::gateway::ExecutionContext,
    observed_at: chrono::DateTime<chrono::Utc>,
    frame_revision: Option<u64>,
    position_tile: Option<crate::world::TilePosition>,
    evidence: crate::execution::movement::ArrivalEvidence,
) -> AnalyticsEvent {
    execution_event(
        "body.movement_arrival",
        EventLevel::Info,
        character_id,
        context,
    )
    .attribute("observed_at", observed_at.to_rfc3339())
    .attribute("observed_frame_revision_known", frame_revision.is_some())
    .attribute("observed_frame_revision", frame_revision.unwrap_or(0))
    .attribute("position_known", position_tile.is_some())
    .attribute(
        "tile_x",
        position_tile.map_or(0, |position| i64::from(position.x)),
    )
    .attribute(
        "tile_y",
        position_tile.map_or(0, |position| i64::from(position.y)),
    )
    .attribute("evidence", evidence.as_str())
}

fn movement_scene_transition_event(
    character_id: &str,
    context: &crate::execution::gateway::ExecutionContext,
    observed_at: chrono::DateTime<chrono::Utc>,
    frame_revision: u64,
    from_scene: Option<&str>,
    to_scene: Option<&str>,
    position_tile: Option<crate::world::TilePosition>,
) -> AnalyticsEvent {
    execution_event(
        "body.movement_scene_transition",
        EventLevel::Info,
        character_id,
        context,
    )
    .attribute("observed_at", observed_at.to_rfc3339())
    .attribute("observed_frame_revision", frame_revision)
    .attribute("from_scene_known", from_scene.is_some())
    .attribute("from_scene", from_scene.unwrap_or_default())
    .attribute("to_scene_known", to_scene.is_some())
    .attribute("to_scene", to_scene.unwrap_or_default())
    .attribute("position_known", position_tile.is_some())
    .attribute(
        "tile_x",
        position_tile.map_or(0, |position| i64::from(position.x)),
    )
    .attribute(
        "tile_y",
        position_tile.map_or(0, |position| i64::from(position.y)),
    )
}

fn movement_requested_event(
    character_id: &str,
    context: &crate::execution::gateway::ExecutionContext,
    requested_at: chrono::DateTime<chrono::Utc>,
    origin_tile: Option<crate::world::TilePosition>,
    destination_tile: crate::world::TilePosition,
) -> AnalyticsEvent {
    execution_event(
        "body.movement_requested",
        EventLevel::Info,
        character_id,
        context,
    )
    .attribute("requested_at", requested_at.to_rfc3339())
    .attribute("origin_known", origin_tile.is_some())
    .attribute(
        "origin_tile_x",
        origin_tile.map_or(0, |position| i64::from(position.x)),
    )
    .attribute(
        "origin_tile_y",
        origin_tile.map_or(0, |position| i64::from(position.y)),
    )
    .attribute("destination_tile_x", i64::from(destination_tile.x))
    .attribute("destination_tile_y", i64::from(destination_tile.y))
}

#[allow(
    clippy::too_many_arguments,
    reason = "the movement reducer supplies one fixed set of physical progress facts"
)]
fn movement_progress_event(
    character_id: &str,
    context: &crate::execution::gateway::ExecutionContext,
    observed_at: chrono::DateTime<chrono::Utc>,
    frame_revision: u64,
    position_tile: Option<crate::world::TilePosition>,
    distance_from_previous_millipixels: Option<u64>,
    observed_distance_millipixels: u64,
    remaining_tile_distance: Option<u32>,
) -> AnalyticsEvent {
    execution_event(
        "body.movement_progress",
        EventLevel::Debug,
        character_id,
        context,
    )
    .attribute("observed_at", observed_at.to_rfc3339())
    .attribute("observed_frame_revision", frame_revision)
    .attribute("position_known", position_tile.is_some())
    .attribute(
        "tile_x",
        position_tile.map_or(0, |position| i64::from(position.x)),
    )
    .attribute(
        "tile_y",
        position_tile.map_or(0, |position| i64::from(position.y)),
    )
    .attribute(
        "distance_from_previous_millipixels_known",
        distance_from_previous_millipixels.is_some(),
    )
    .attribute(
        "distance_from_previous_millipixels",
        distance_from_previous_millipixels.unwrap_or(0),
    )
    .attribute(
        "observed_distance_millipixels",
        observed_distance_millipixels,
    )
    .attribute(
        "remaining_tile_distance_known",
        remaining_tile_distance.is_some(),
    )
    .attribute(
        "remaining_tile_distance",
        u64::from(remaining_tile_distance.unwrap_or(0)),
    )
}

fn execution_event(
    name: &'static str,
    level: EventLevel,
    character_id: &str,
    context: &crate::execution::gateway::ExecutionContext,
) -> AnalyticsEvent {
    AnalyticsEvent::new(name, level)
        .character(character_id)
        .correlation(context.action_id)
        .attribute("session_generation", context.session_generation)
        .attribute("decision_id", context.decision_id.to_string())
        .attribute("packet_id", context.packet_id.to_string())
        .attribute("action_id", context.action_id.to_string())
        .attribute(
            "action_index",
            u64::try_from(context.action_index).unwrap_or(u64::MAX),
        )
        .attribute("frame_revision", context.frame_revision)
        .attribute("strategic_revision", context.strategic_revision)
}

fn frame_published_event(
    character_id: &str,
    revisions: [u64; 5],
    summary: &PerceptionSummary,
    observation_cycle_id: Option<uuid::Uuid>,
    observation_cycle_sequence: Option<u64>,
) -> AnalyticsEvent {
    let mut event = AnalyticsEvent::new("perception.frame_published", EventLevel::Debug)
        .character(character_id)
        .attribute("observation_cycle_known", observation_cycle_id.is_some())
        .attribute("frame_revision", revisions[0])
        .attribute("perception_revision", revisions[1])
        .attribute("strategic_revision", revisions[2])
        .attribute("inventory_revision", revisions[3])
        .attribute("map_revision", revisions[4])
        .attribute("material_change", summary.material_change)
        .attribute(
            "derived_event_count",
            u64::try_from(summary.derived_event_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "backend_event_count",
            u64::try_from(summary.backend_event_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "visible_entity_count",
            u64::try_from(summary.visible_entity_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "visible_hostile_count",
            u64::try_from(summary.visible_hostile_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "hostiles_targeting_self_count",
            u64::try_from(summary.hostiles_targeting_self_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "nearest_hostile_distance_known",
            summary.nearest_hostile_distance_mill_tiles.is_some(),
        )
        .attribute(
            "nearest_hostile_distance_mill_tiles",
            u64::from(summary.nearest_hostile_distance_mill_tiles.unwrap_or(0)),
        )
        .attribute(
            "visible_player_count",
            u64::try_from(summary.visible_player_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "visible_npc_count",
            u64::try_from(summary.visible_npc_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "visible_merchant_count",
            u64::try_from(summary.visible_merchant_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "visible_enemy_count",
            u64::try_from(summary.visible_enemy_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "visible_unknown_count",
            u64::try_from(summary.visible_unknown_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "drop_count",
            u64::try_from(summary.drop_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "positioned_drop_count",
            u64::try_from(summary.positioned_drop_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "unpositioned_drop_count",
            u64::try_from(summary.unpositioned_drop_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "carried_item_count",
            u64::try_from(summary.carried_item_count).unwrap_or(u64::MAX),
        )
        .attribute("carried_item_units", summary.carried_item_units)
        .attribute(
            "door_count",
            u64::try_from(summary.door_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "locked_door_count",
            u64::try_from(summary.locked_door_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "unknown_lock_door_count",
            u64::try_from(summary.unknown_lock_door_count).unwrap_or(u64::MAX),
        );
    if let Some(cycle_id) = observation_cycle_id {
        event = event
            .correlation(cycle_id)
            .attribute("observation_cycle_id", cycle_id.to_string());
    }
    if let Some(sequence) = observation_cycle_sequence {
        event = event.attribute("observation_cycle_sequence", sequence);
    }
    add_frame_context(event, summary)
}

#[allow(
    clippy::items_after_test_module,
    reason = "the shared frame helper remains below its focused lineage tests during concurrent edits"
)]
fn add_frame_context(event: AnalyticsEvent, summary: &PerceptionSummary) -> AnalyticsEvent {
    event
        .attribute("scene_known", summary.scene.is_some())
        .attribute("scene", summary.scene.clone().unwrap_or_default())
        .attribute("position_tile_known", summary.position_tile.is_some())
        .attribute(
            "position_tile_x",
            i64::from(summary.position_tile.map_or(0, |position| position.x)),
        )
        .attribute(
            "position_tile_y",
            i64::from(summary.position_tile.map_or(0, |position| position.y)),
        )
        .attribute("alive_known", summary.alive.is_some())
        .attribute("alive", summary.alive.unwrap_or(false))
        .attribute("recently_died_known", summary.recently_died.is_some())
        .attribute("recently_died", summary.recently_died.unwrap_or(false))
        .attribute(
            "reported_total_object_count_known",
            summary.reported_total_object_count.is_some(),
        )
        .attribute(
            "reported_total_object_count",
            u64::from(summary.reported_total_object_count.unwrap_or(0)),
        )
        .attribute(
            "object_list_truncated_known",
            summary.object_list_truncated.is_some(),
        )
        .attribute(
            "object_list_truncated",
            summary.object_list_truncated.unwrap_or(false),
        )
        .attribute(
            "new_dialogue_count",
            u64::try_from(summary.new_dialogue_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "new_scene_chat_count",
            u64::try_from(summary.new_scene_chat_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "new_global_chat_count",
            u64::try_from(summary.new_global_chat_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "new_private_chat_count",
            u64::try_from(summary.new_private_chat_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "new_team_chat_count",
            u64::try_from(summary.new_team_chat_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "new_unknown_chat_count",
            u64::try_from(summary.new_unknown_chat_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "new_melody_count",
            u64::try_from(summary.new_melody_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "filtered_chat_count",
            u64::try_from(summary.filtered_chat_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "reachable_exit_count",
            u64::try_from(summary.reachable_exit_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "nearest_exit_path_length_known",
            summary.nearest_exit_path_length.is_some(),
        )
        .attribute(
            "nearest_exit_path_length",
            u64::from(summary.nearest_exit_path_length.unwrap_or(0)),
        )
        .attribute(
            "local_waypoint_count",
            u64::try_from(summary.local_waypoint_count).unwrap_or(u64::MAX),
        )
        .attribute(
            "farthest_waypoint_path_length_known",
            summary.farthest_waypoint_path_length.is_some(),
        )
        .attribute(
            "farthest_waypoint_path_length",
            u64::from(summary.farthest_waypoint_path_length.unwrap_or(0)),
        )
        .attribute(
            "map_tile_count",
            u64::try_from(summary.map_tile_count).unwrap_or(u64::MAX),
        )
        .attribute("health_known", summary.health.is_some())
        .attribute("health", i64::from(summary.health.unwrap_or(0)))
        .attribute("max_health_known", summary.max_health.is_some())
        .attribute("max_health", i64::from(summary.max_health.unwrap_or(0)))
        .attribute("combat_active_known", summary.combat_active.is_some())
        .attribute("combat_active", summary.combat_active.unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::tactical_schedule::{
        DeferralReason, PacketRelease, TacticalActivity, TacticalWakeReason,
    };

    #[test]
    fn frame_and_rejection_events_keep_observation_cycle_lineage() {
        let cycle_id = uuid::Uuid::from_u128(42);
        let summary = PerceptionSummary {
            position_tile: Some(crate::world::TilePosition { x: 5, y: 7 }),
            ..PerceptionSummary::default()
        };
        let frame = to_analytics_event(
            "cassian",
            &TelemetryEvent::FramePublished {
                observation_cycle_id: Some(cycle_id),
                observation_cycle_sequence: Some(7),
                frame_revision: 11,
                perception_revision: 12,
                strategic_revision: 13,
                inventory_revision: 14,
                map_revision: 15,
                summary: Box::new(summary),
            },
        );
        assert_eq!(frame.correlation_id, Some(cycle_id));
        assert_eq!(frame.attributes["observation_cycle_known"], true);
        assert_eq!(
            frame.attributes["observation_cycle_id"],
            cycle_id.to_string()
        );
        assert_eq!(frame.attributes["observation_cycle_sequence"], 7);
        assert_eq!(frame.attributes["position_tile_known"], true);
        assert_eq!(frame.attributes["position_tile_x"], 5);
        assert_eq!(frame.attributes["position_tile_y"], 7);

        let rejected = to_analytics_event(
            "cassian",
            &TelemetryEvent::PerceptionRejected {
                observation_cycle_id: cycle_id,
                observation_cycle_sequence: 7,
                error_class: "invalid_coordinate".to_owned(),
            },
        );
        assert_eq!(rejected.correlation_id, Some(cycle_id));
        assert_eq!(
            rejected.attributes["observation_cycle_id"],
            cycle_id.to_string()
        );
        assert_eq!(rejected.attributes["observation_cycle_sequence"], 7);
        assert_eq!(rejected.attributes["error_class"], "invalid_coordinate");
    }

    #[test]
    fn synthetic_frame_is_explicitly_not_an_observation_cycle() {
        let frame = to_analytics_event(
            "cassian",
            &TelemetryEvent::FramePublished {
                observation_cycle_id: None,
                observation_cycle_sequence: None,
                frame_revision: 1,
                perception_revision: 1,
                strategic_revision: 1,
                inventory_revision: 1,
                map_revision: 1,
                summary: Box::new(PerceptionSummary::default()),
            },
        );
        assert_eq!(frame.correlation_id, None);
        assert_eq!(frame.attributes["observation_cycle_known"], false);
        assert!(!frame.attributes.contains_key("observation_cycle_id"));
        assert!(!frame.attributes.contains_key("observation_cycle_sequence"));
    }

    #[test]
    fn tactical_lineage_uses_safe_causal_fields_only() {
        let signal_id = uuid::Uuid::from_u128(51);
        let decision_id = uuid::Uuid::from_u128(52);
        let started = to_analytics_event(
            "cassian",
            &TelemetryEvent::TacticalDecisionStarted {
                trigger_signal_id: signal_id,
                decision_id,
                scheduler_inference_id: 9,
                frame_revision: 100,
                strategic_revision: 12,
                wake_reasons: vec![
                    TacticalWakeReason::DamageTaken,
                    TacticalWakeReason::HostileSpawned,
                ],
            },
        );

        assert_eq!(started.name, "tactical.inference_started");
        assert_eq!(started.correlation_id, Some(decision_id));
        assert_eq!(
            started.attributes["trigger_signal_id"],
            signal_id.to_string()
        );
        assert_eq!(started.attributes["decision_id"], decision_id.to_string());
        assert_eq!(started.attributes["frame_revision"], 100);
        assert_eq!(started.attributes["strategic_revision"], 12);
        assert_eq!(started.attributes["wake_reason_count"], 2);
        assert_eq!(
            started.attributes["wake_reasons"],
            "damage_taken|hostile_spawned"
        );
        assert!(!started.attributes.contains_key("prompt"));
        assert!(!started.attributes.contains_key("model_output"));
        assert!(!started.attributes.contains_key("rationale"));
    }

    #[test]
    fn tactical_policy_events_have_bounded_dimensions_and_update_counters() {
        let signal_id = uuid::Uuid::from_u128(61);
        let decision_id = uuid::Uuid::from_u128(62);
        let packet_id = uuid::Uuid::from_u128(63);
        let deferred = TelemetryEvent::TacticalWakeDeferred {
            signal_id,
            frame_revision: 20,
            strategic_revision: 3,
            reason: DeferralReason::GlobalRateLimit,
            eligible_after_ms: Some(125),
            coalesced_reason_count: 2,
        };
        let analytics = to_analytics_event("cassian", &deferred);
        assert_eq!(analytics.name, "tactical.wake_deferred");
        assert_eq!(analytics.attributes["deferral_reason"], "global_rate_limit");
        assert_eq!(analytics.attributes["eligible_after_ms"], 125);

        let mut snapshot = TelemetrySnapshot::default();
        update_snapshot(
            &mut snapshot,
            &TelemetryEvent::TacticalWakeRequested {
                signal_id,
                frame_revision: 20,
                strategic_revision: 3,
                reason: TacticalWakeReason::DamageTaken,
                activity: TacticalActivity::ActiveCombat,
            },
        );
        update_snapshot(&mut snapshot, &deferred);
        update_snapshot(
            &mut snapshot,
            &TelemetryEvent::TacticalPacketReleaseDecided {
                decision_id,
                packet_id,
                frame_revision: 20,
                strategic_revision: 3,
                rollout_mode: "shadow".to_owned(),
                release_policy: PacketRelease::RecordOnly,
                action_count: 2,
                intent: crate::execution::packet::TacticalIntent::Attack,
                released: false,
                reason_code: "record_only".to_owned(),
            },
        );

        assert_eq!(snapshot.events_recorded, 3);
        assert_eq!(snapshot.tactical_wakes_requested, 1);
        assert_eq!(snapshot.tactical_wakes_deferred, 1);
        assert_eq!(snapshot.tactical_packet_release_decisions, 1);
        assert_eq!(snapshot.tactical_packets_record_only, 1);
        assert_eq!(snapshot.tactical_packets_released, 0);
    }
}
