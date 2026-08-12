use std::{collections::VecDeque, sync::Arc};

use num_traits::ToPrimitive;
use ractor::{Actor, ActorProcessingErr, ActorRef};

use crate::{
    brain::tactical_frame::{EntityKind, TacticalFrame},
    execution::{movement::NavigationMissionRequest, outcome::ActionOutcome},
    runtime::{
        blackboard::HotBlackboard,
        messages::{
            BodyMsg, PerceptionMsg, PerceptionStatus, StrategistMsg, TacticianMsg, TelemetryEvent,
            TelemetryMsg,
        },
    },
    world::{
        events::GameEvent,
        perception::{PerceptionEngine, PerceptionSummary},
    },
};

const MAX_RECENT_EVENTS: usize = 256;
const MAX_RECENT_OUTCOMES: usize = 64;

pub struct PerceptionActor;

pub struct PerceptionActorArgs {
    pub blackboard: Arc<HotBlackboard>,
    pub body: ActorRef<crate::runtime::messages::BodyMsg>,
    pub tactician: ActorRef<TacticianMsg>,
    pub strategist: ActorRef<StrategistMsg>,
    pub player_name: String,
    pub telemetry: ActorRef<TelemetryMsg>,
}

pub struct PerceptionActorState {
    args: PerceptionActorArgs,
    events: VecDeque<GameEvent>,
    outcomes: VecDeque<ActionOutcome>,
    engine: PerceptionEngine,
    frames_published: u64,
    snapshots_rejected: u64,
    last_strategist_scene: Option<String>,
    initial_navigation_resumed: bool,
}

impl Actor for PerceptionActor {
    type Msg = PerceptionMsg;
    type State = PerceptionActorState;
    type Arguments = PerceptionActorArgs;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(PerceptionActorState {
            args,
            events: VecDeque::new(),
            outcomes: VecDeque::new(),
            engine: PerceptionEngine::default(),
            frames_published: 0,
            snapshots_rejected: 0,
            last_strategist_scene: None,
            initial_navigation_resumed: false,
        })
    }

    #[allow(
        clippy::too_many_lines,
        reason = "perception message ownership remains visible in one actor handler"
    )]
    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            PerceptionMsg::Observation(input) => {
                let observation_cycle_id = input.observation_cycle_id;
                let observation_cycle_sequence = input.observation_cycle_sequence;
                match state.engine.update(*input) {
                    Ok(update) => {
                        let new_scene = update.frame.self_state.scene.clone();
                        if new_scene != state.last_strategist_scene {
                            let summary = match (&state.last_strategist_scene, &new_scene) {
                                (Some(previous), Some(current)) => {
                                    format!("Scene changed from {previous} to {current}.")
                                }
                                (None, Some(current)) => format!("Current scene is {current}."),
                                (Some(previous), None) => {
                                    format!("Scene {previous} was left; current scene is unknown.")
                                }
                                (None, None) => String::new(),
                            };
                            if !summary.is_empty() {
                                let _ = state
                                    .args
                                    .strategist
                                    .send_message(StrategistMsg::WorldMoment(summary));
                            }
                            state.last_strategist_scene = new_scene;
                        }
                        for line in &update.new_dialogue {
                            if !line.from.eq_ignore_ascii_case(&state.args.player_name) {
                                let _ = state
                                    .args
                                    .strategist
                                    .send_message(StrategistMsg::PersonSpoke(line.clone()));
                            }
                        }
                        publish_frame(
                            &Arc::new(update.frame),
                            Box::new(update.summary),
                            Some((
                                update.observation_cycle_id,
                                update.observation_cycle_sequence,
                            )),
                            state,
                        );
                    }
                    Err(error) => {
                        state.snapshots_rejected += 1;
                        let _ = state.args.telemetry.send_message(TelemetryMsg::Record(
                            TelemetryEvent::PerceptionRejected {
                                observation_cycle_id,
                                observation_cycle_sequence,
                                error_class: perception_error_class(&error).to_owned(),
                            },
                        ));
                    }
                }
            }
            PerceptionMsg::PublishFrame(frame) => {
                publish_frame(
                    &frame,
                    Box::new(summarize_published_frame(&frame)),
                    None,
                    state,
                );
            }
            PerceptionMsg::BackendEvent(event) => {
                push_bounded(&mut state.events, event.clone(), MAX_RECENT_EVENTS);
                state.engine.record_backend_event(event);
            }
            PerceptionMsg::ActionOutcome(outcome) => {
                push_bounded(&mut state.outcomes, outcome.clone(), MAX_RECENT_OUTCOMES);
                state.engine.record_action(outcome);
            }
            PerceptionMsg::NavigationBlocked {
                mission_id,
                reason_code,
                attempts,
            } => {
                tracing::warn!(
                    %mission_id,
                    %reason_code,
                    attempts,
                    "forwarding blocked navigation mission to strategist"
                );
                let _ = state.args.strategist.send_message(StrategistMsg::GoalBlocked(
                    format!(
                        "navigation mission {mission_id} failed after {attempts} attempts: {reason_code}"
                    ),
                ));
            }
            PerceptionMsg::NavigationArrived(arrival) => {
                tracing::info!(
                    mission_id = %arrival.mission_id,
                    destination = %arrival.destination_name,
                    attempts = arrival.attempts,
                    "forwarding navigation arrival to strategist"
                );
                let _ = state.args.telemetry.send_message(TelemetryMsg::Record(
                    TelemetryEvent::StrategicNavigationArrivalObserved {
                        mission_id: arrival.mission_id,
                        decision_id: arrival.decision_id,
                        strategic_revision: arrival.strategic_revision,
                        destination_scene: arrival.destination_scene.clone(),
                        arrived_scene: arrival.arrived_scene.clone(),
                        destination_tile_known: arrival.destination_tile.is_some(),
                        arrived_tile_known: arrival.arrived_tile.is_some(),
                        attempts: arrival.attempts,
                    },
                ));
                let _ = state
                    .args
                    .strategist
                    .send_message(StrategistMsg::NavigationArrived(arrival));
            }
            PerceptionMsg::Tick => {}
            PerceptionMsg::ReplaceTactician(tactician) => state.args.tactician = tactician,
            PerceptionMsg::Health(reply) => {
                if !reply.is_closed() {
                    reply.send(PerceptionStatus {
                        frames_published: state.frames_published,
                        snapshots_rejected: state.snapshots_rejected,
                        latest_perception_revision: state.args.blackboard.perception_revision(),
                        buffered_events: state.events.len(),
                    })?;
                }
            }
            PerceptionMsg::Shutdown => myself.stop(Some("player runtime shutdown".to_owned())),
        }
        Ok(())
    }
}

fn entity_count(frame: &TacticalFrame, kind: EntityKind) -> usize {
    frame
        .nearby_entities
        .iter()
        .filter(|entity| entity.kind == kind)
        .count()
}

fn publish_frame(
    frame: &Arc<TacticalFrame>,
    summary: Box<PerceptionSummary>,
    observation_cycle: Option<(uuid::Uuid, u64)>,
    state: &mut PerceptionActorState,
) {
    if !state.args.blackboard.publish_frame(frame.clone()) {
        tracing::warn!(
            perception_revision = frame.perception_revision,
            "ignored out-of-order tactical frame"
        );
        return;
    }
    state.frames_published += 1;
    let _ = state
        .args
        .tactician
        .send_message(TacticianMsg::FrameUpdated(frame.clone()));
    let _ = state
        .args
        .body
        .send_message(BodyMsg::FrameUpdated(frame.clone()));
    if !state.initial_navigation_resumed {
        state.initial_navigation_resumed = true;
        if let Some(request) = restored_navigation_request(frame) {
            let _ = state
                .args
                .body
                .send_message(BodyMsg::PursueNavigation(request));
        }
    }
    let _ =
        state
            .args
            .telemetry
            .send_message(TelemetryMsg::Record(TelemetryEvent::FramePublished {
                observation_cycle_id: observation_cycle.map(|cycle| cycle.0),
                observation_cycle_sequence: observation_cycle.map(|cycle| cycle.1),
                frame_revision: frame.revision,
                perception_revision: frame.perception_revision,
                strategic_revision: frame.strategic_intent.revision,
                inventory_revision: frame.inventory_revision,
                map_revision: frame.map.revision,
                summary,
            }));
}

fn restored_navigation_request(frame: &TacticalFrame) -> Option<NavigationMissionRequest> {
    let goal = frame.strategic_intent.navigation_goal.as_ref()?;
    let destination_name = goal.destination.as_ref().map_or_else(
        || goal.scene.clone(),
        |destination| destination.name.clone(),
    );
    let correlation_name = format!(
        "restored-navigation:{}:{}:{:?}",
        frame.strategic_intent.revision,
        goal.scene,
        goal.destination
            .as_ref()
            .and_then(|destination| destination.tile)
    );
    Some(NavigationMissionRequest {
        decision_id: uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, correlation_name.as_bytes()),
        frame_revision: frame.revision,
        strategic_revision: frame.strategic_intent.revision,
        destination_scene: goal.scene.clone(),
        destination_tile: goal
            .destination
            .as_ref()
            .and_then(|destination| destination.tile),
        destination_name,
        reason: goal.reason.clone(),
        route: Vec::new(),
    })
}

fn summarize_published_frame(frame: &TacticalFrame) -> PerceptionSummary {
    PerceptionSummary {
        scene: frame.self_state.scene.clone(),
        position_tile: frame.self_state.position.map(|position| position.tile),
        alive: frame.self_state.alive,
        recently_died: frame.self_state.recently_died,
        material_change: true,
        derived_event_count: 0,
        backend_event_count: 0,
        visible_entity_count: frame.nearby_entities.len(),
        visible_hostile_count: frame
            .nearby_entities
            .iter()
            .filter(|entity| entity.hostile == Some(true))
            .count(),
        hostiles_targeting_self_count: frame
            .nearby_entities
            .iter()
            .filter(|entity| entity.hostile == Some(true) && entity.targeting_you == Some(true))
            .count(),
        nearest_hostile_distance_mill_tiles: frame
            .nearby_entities
            .iter()
            .filter(|entity| entity.hostile == Some(true))
            .filter_map(|entity| entity.distance)
            .filter(|distance| distance.is_finite() && *distance >= 0.0)
            .min_by(f32::total_cmp)
            .and_then(|distance| (distance * 1_000.0).round().to_u32()),
        visible_player_count: entity_count(frame, EntityKind::Player),
        visible_npc_count: entity_count(frame, EntityKind::Npc),
        visible_merchant_count: frame
            .nearby_entities
            .iter()
            .filter(|entity| entity.is_merchant == Some(true))
            .count(),
        visible_enemy_count: entity_count(frame, EntityKind::Enemy),
        visible_unknown_count: entity_count(frame, EntityKind::Unknown),
        drop_count: frame.nearby_drops.len(),
        positioned_drop_count: frame
            .nearby_drops
            .iter()
            .filter(|drop| drop.tile.is_some())
            .count(),
        unpositioned_drop_count: frame
            .nearby_drops
            .iter()
            .filter(|drop| drop.tile.is_none())
            .count(),
        carried_item_count: frame.self_state.inventory.len(),
        carried_item_units: frame
            .self_state
            .inventory
            .iter()
            .map(|item| u64::from(item.quantity))
            .sum(),
        door_count: frame.map.doors.len(),
        locked_door_count: frame
            .map
            .doors
            .iter()
            .filter(|door| door.locked == Some(true))
            .count(),
        unknown_lock_door_count: frame
            .map
            .doors
            .iter()
            .filter(|door| door.locked.is_none())
            .count(),
        reported_total_object_count: frame.census.reported_total_objects,
        object_list_truncated: frame.census.object_list_truncated,
        new_dialogue_count: 0,
        new_scene_chat_count: 0,
        new_global_chat_count: 0,
        new_private_chat_count: 0,
        new_team_chat_count: 0,
        new_unknown_chat_count: 0,
        new_melody_count: 0,
        filtered_chat_count: 0,
        reachable_exit_count: frame.exits.len(),
        nearest_exit_path_length: frame.exits.iter().map(|exit| exit.path_length_tiles).min(),
        local_waypoint_count: frame.local_waypoints.len(),
        farthest_waypoint_path_length: frame
            .local_waypoints
            .iter()
            .map(|waypoint| waypoint.path_length_tiles)
            .max(),
        map_tile_count: frame.map.tiles.len(),
        health: frame.self_state.health,
        max_health: frame.self_state.max_health,
        combat_active: frame.combat.active,
    }
}

fn perception_error_class(error: &crate::world::perception::PerceptionError) -> &'static str {
    match error {
        crate::world::perception::PerceptionError::InvalidCoordinate { .. } => "invalid_coordinate",
        crate::world::perception::PerceptionError::MapTooLarge => "map_too_large",
    }
}

fn push_bounded<T>(queue: &mut VecDeque<T>, value: T, maximum: usize) {
    if queue.len() == maximum {
        queue.pop_front();
    }
    queue.push_back(value);
}

#[cfg(test)]
mod tests {
    use crate::brain::{
        strategic_intent::{NamedDestination, NavigationGoal, StrategicIntent},
        tactical_frame::TacticalFrame,
    };

    use super::restored_navigation_request;

    #[test]
    fn restored_navigation_becomes_a_body_owned_destination_mission() {
        let mut intent = StrategicIntent {
            revision: 17,
            ..StrategicIntent::default()
        };
        intent.navigation_goal = Some(NavigationGoal {
            scene: "reldens-town".to_owned(),
            destination: Some(NamedDestination {
                name: "town square".to_owned(),
                tile: Some(crate::world::TilePosition { x: 21, y: 14 }),
            }),
            reason: "meet the merchant".to_owned(),
        });
        let mut frame = TacticalFrame::empty(intent);
        frame.revision = 93;

        let request = restored_navigation_request(&frame).expect("restored navigation mission");

        assert_eq!(request.frame_revision, 93);
        assert_eq!(request.strategic_revision, 17);
        assert_eq!(request.destination_scene, "reldens-town");
        assert_eq!(
            request.destination_tile,
            Some(crate::world::TilePosition { x: 21, y: 14 })
        );
        assert_eq!(request.destination_name, "town square");
        assert_eq!(request.reason, "meet the merchant");
        assert!(request.route.is_empty());
    }

    #[test]
    fn frame_without_navigation_does_not_create_a_mission() {
        let frame = TacticalFrame::empty(StrategicIntent::default());
        assert!(restored_navigation_request(&frame).is_none());
    }
}
