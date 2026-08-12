use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    brain::{
        strategic_intent::StrategicIntent,
        tactical_frame::{
            Drop, EntityKind, SelfState, TacticalFrame, VisibilityCensus, VisibleEntity,
        },
    },
    execution::outcome::OutcomeStatus,
    world::{
        TilePosition,
        events::GameEventKind,
        map::{Doorway, ReachableExit, ReachableWaypoint},
    },
};

const MAX_RECENT_EVENTS: usize = 20;
const MAX_RECENT_ACTIONS: usize = 10;
const TACTICAL_PREEMPTION_EVENT_WINDOW_MS: i64 = 2_000;
pub const TACTICAL_INPUT_PROTOCOL_VERSION: u32 = 1;

/// Compact, factual model input derived from a full authoritative tactical frame.
///
/// The full frame remains available to deterministic runtime code. In particular,
/// structured map tiles never enter the model prompt: the model receives the same
/// map as compact ASCII plus exact entities, drops, doors, and reachable exits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TacticalInput {
    pub protocol_version: u32,
    pub frame_revision: u64,
    pub strategic_revision: u64,
    pub self_state: SelfState,
    pub combat: crate::world::combat::CombatSnapshot,
    pub census: VisibilityCensus,
    pub nearby_entities: Vec<VisibleEntity>,
    pub entity_approaches: Vec<TacticalEntityApproach>,
    pub nearby_drops: Vec<Drop>,
    pub local_map: TacticalMap,
    pub exits: Vec<ReachableExit>,
    pub local_waypoints: Vec<ReachableWaypoint>,
    pub recent_events: Vec<TacticalEvent>,
    pub recent_actions: Vec<TacticalActionOutcome>,
    pub strategic_intent: StrategicIntent,
    pub movement_control: MovementControl,
}

/// Typed runtime ownership for local movement.
///
/// This is coordination state, not gameplay advice. The body owns quiet
/// destination travel. The tactician can request preemption only when the
/// current frame contains one of the listed immediate facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MovementControl {
    pub owner: MovementControlOwner,
    pub state: MovementControlState,
    pub strategic_revision: u64,
    pub destination_scene: Option<String>,
    pub destination_tile: Option<TilePosition>,
    pub tactical_preemption_allowed_for: Vec<TacticalPreemptionFact>,
    /// Immediate authoritative facts present in this frame.
    ///
    /// The executor uses the same reducer when it validates a proposed
    /// movement override. This prevents prompt/runtime policy drift.
    pub tactical_preemption_facts_present: Vec<TacticalPreemptionFact>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MovementControlOwner {
    BodyStrategicNavigation,
    Tactician,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MovementControlState {
    Assigned,
    Available,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TacticalPreemptionFact {
    CombatActive,
    HostileTargetingSelf,
    DamageTaken,
    MovementFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TacticalMap {
    pub revision: u64,
    pub origin_tile_x: i32,
    pub origin_tile_y: i32,
    pub width: usize,
    pub height: usize,
    pub doors: Vec<Doorway>,
    pub ascii: String,
}

/// Exact reachable tiles next to a visible entity. The model still selects the entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TacticalEntityApproach {
    pub entity_id: String,
    pub entity_kind: EntityKind,
    pub candidates: Vec<ReachableApproachTile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReachableApproachTile {
    pub tile: TilePosition,
    pub path_length_tiles: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TacticalEvent {
    pub sequence: Option<u64>,
    pub age_ms: u64,
    pub kind: GameEventKind,
    pub entity_id: Option<String>,
    pub amount: Option<i64>,
    pub tile: Option<TilePosition>,
    /// Bounded authoritative context, such as the entered or departed scene.
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TacticalActionOutcome {
    pub action_kind: String,
    pub duration_ms: u64,
    pub status: OutcomeStatus,
    pub reason_code: Option<String>,
    pub detail: String,
    pub destination_tile: Option<TilePosition>,
}

impl From<&TacticalFrame> for TacticalInput {
    fn from(frame: &TacticalFrame) -> Self {
        let most_recent_left_scene = frame
            .recent_events
            .iter()
            .rev()
            .find(|event| event.kind == GameEventKind::SceneLeft)
            .and_then(|event| event.detail.as_deref());
        let strategy_requires_return = most_recent_left_scene.is_some_and(|left_scene| {
            frame
                .strategic_intent
                .navigation_goal
                .as_ref()
                .is_some_and(|goal| goal.scene == left_scene)
        });
        Self {
            protocol_version: TACTICAL_INPUT_PROTOCOL_VERSION,
            frame_revision: frame.revision,
            strategic_revision: frame.strategic_intent.revision,
            self_state: frame.self_state.clone(),
            combat: frame.combat.clone(),
            census: frame.census.clone(),
            nearby_entities: frame.nearby_entities.clone(),
            entity_approaches: entity_approaches(frame),
            nearby_drops: frame.nearby_drops.clone(),
            local_map: TacticalMap {
                revision: frame.map.revision,
                origin_tile_x: frame.map.origin_tile_x,
                origin_tile_y: frame.map.origin_tile_y,
                width: frame.map.width,
                height: frame.map.height,
                doors: frame.map.doors.clone(),
                ascii: frame.map.ascii.clone(),
            },
            exits: frame
                .exits
                .iter()
                .filter(|exit| {
                    strategy_requires_return
                        || exit.destination_scene.as_deref() != most_recent_left_scene
                })
                .cloned()
                .collect(),
            local_waypoints: frame.local_waypoints.clone(),
            recent_events: tail(&frame.recent_events, MAX_RECENT_EVENTS)
                .iter()
                .map(|event| TacticalEvent {
                    sequence: event.sequence,
                    age_ms: u64::try_from(
                        frame
                            .generated_at
                            .signed_duration_since(event.observed_at)
                            .num_milliseconds()
                            .max(0),
                    )
                    .unwrap_or(u64::MAX),
                    kind: event.kind,
                    entity_id: event.entity_id.clone(),
                    amount: event.amount,
                    tile: event.tile,
                    detail: event.detail.clone(),
                })
                .collect(),
            recent_actions: tail(&frame.recent_actions, MAX_RECENT_ACTIONS)
                .iter()
                .map(|outcome| TacticalActionOutcome {
                    action_kind: outcome.action_kind.clone(),
                    duration_ms: outcome.duration_ms,
                    status: outcome.status,
                    reason_code: outcome.reason_code.clone(),
                    detail: outcome.detail.clone(),
                    destination_tile: outcome.destination_tile,
                })
                .collect(),
            strategic_intent: frame.strategic_intent.clone(),
            movement_control: movement_control(frame),
        }
    }
}

fn movement_control(frame: &TacticalFrame) -> MovementControl {
    frame.strategic_intent.navigation_goal.as_ref().map_or_else(
        || MovementControl {
            owner: MovementControlOwner::Tactician,
            state: MovementControlState::Available,
            strategic_revision: frame.strategic_intent.revision,
            destination_scene: None,
            destination_tile: None,
            tactical_preemption_allowed_for: Vec::new(),
            tactical_preemption_facts_present: Vec::new(),
        },
        |goal| MovementControl {
            owner: MovementControlOwner::BodyStrategicNavigation,
            state: MovementControlState::Assigned,
            strategic_revision: frame.strategic_intent.revision,
            destination_scene: Some(goal.scene.clone()),
            destination_tile: goal
                .destination
                .as_ref()
                .and_then(|destination| destination.tile),
            tactical_preemption_allowed_for: vec![
                TacticalPreemptionFact::CombatActive,
                TacticalPreemptionFact::HostileTargetingSelf,
                TacticalPreemptionFact::DamageTaken,
                TacticalPreemptionFact::MovementFailure,
            ],
            tactical_preemption_facts_present: tactical_preemption_facts(frame),
        },
    )
}

/// Reduce the authoritative frame into facts that permit tactical movement preemption.
///
/// The result contains facts only. It does not decide whether the character
/// should move, fight, heal, or flee.
#[must_use]
pub fn tactical_preemption_facts(frame: &TacticalFrame) -> Vec<TacticalPreemptionFact> {
    let mut facts = Vec::new();
    if frame.combat.active == Some(true) {
        facts.push(TacticalPreemptionFact::CombatActive);
    }
    if frame
        .nearby_entities
        .iter()
        .any(|entity| entity.hostile == Some(true) && entity.targeting_you == Some(true))
    {
        facts.push(TacticalPreemptionFact::HostileTargetingSelf);
    }
    if frame.recent_events.iter().any(|event| {
        event.kind == GameEventKind::DamageTaken
            && (0..=TACTICAL_PREEMPTION_EVENT_WINDOW_MS).contains(
                &frame
                    .generated_at
                    .signed_duration_since(event.observed_at)
                    .num_milliseconds(),
            )
    }) {
        facts.push(TacticalPreemptionFact::DamageTaken);
    }
    if frame.recent_events.iter().any(|event| {
        event.kind == GameEventKind::MovementFailed
            && (0..=TACTICAL_PREEMPTION_EVENT_WINDOW_MS).contains(
                &frame
                    .generated_at
                    .signed_duration_since(event.observed_at)
                    .num_milliseconds(),
            )
    }) || frame.recent_actions.iter().any(|outcome| {
        outcome.status == OutcomeStatus::Failed
            && outcome.action_kind == "move_to"
            && (0..=TACTICAL_PREEMPTION_EVENT_WINDOW_MS).contains(
                &frame
                    .generated_at
                    .signed_duration_since(outcome.recorded_at)
                    .num_milliseconds(),
            )
    }) {
        facts.push(TacticalPreemptionFact::MovementFailure);
    }
    facts
}

fn entity_approaches(frame: &TacticalFrame) -> Vec<TacticalEntityApproach> {
    let Some(start) = frame.self_state.position.map(|position| position.tile) else {
        return Vec::new();
    };
    let paths = crate::world::perception::local_path_lengths(&frame.map, start);
    frame
        .nearby_entities
        .iter()
        .filter(|entity| entity.alive != Some(false))
        .filter_map(|entity| {
            let entity_tile = entity.tile?;
            let mut candidates = crate::world::perception::cardinal_neighbors(entity_tile)
                .into_iter()
                .filter(|tile| *tile != start)
                .filter_map(|tile| {
                    paths
                        .get(&tile)
                        .copied()
                        .map(|path_length_tiles| ReachableApproachTile {
                            tile,
                            path_length_tiles,
                        })
                })
                .collect::<Vec<_>>();
            candidates.sort_by_key(|candidate| {
                (
                    candidate.path_length_tiles,
                    candidate.tile.y,
                    candidate.tile.x,
                )
            });
            candidates.truncate(4);
            (!candidates.is_empty()).then(|| TacticalEntityApproach {
                entity_id: entity.id.clone(),
                entity_kind: entity.kind,
                candidates,
            })
        })
        .collect()
}

fn tail<T>(values: &[T], limit: usize) -> &[T] {
    &values[values.len().saturating_sub(limit)..]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        brain::strategic_intent::StrategicIntent,
        execution::outcome::ActionOutcome,
        world::{
            TilePosition,
            map::{LocalMap, MapTile, TileKind},
        },
    };

    #[test]
    fn excludes_structured_tiles_but_keeps_ascii_and_exact_world_lists() {
        let mut frame = TacticalFrame::empty(StrategicIntent::default());
        frame.revision = 18;
        frame.map = LocalMap {
            revision: 4,
            origin_tile_x: 10,
            origin_tile_y: 20,
            width: 33,
            height: 33,
            tiles: (0..1_089)
                .map(|offset| MapTile {
                    position: TilePosition {
                        x: offset % 33,
                        y: offset / 33,
                    },
                    kind: TileKind::Traversable,
                    walkable: Some(true),
                })
                .collect(),
            doors: Vec::new(),
            ascii: "...\n.@S\n...".to_owned(),
        };
        frame.nearby_entities.push(VisibleEntity {
            id: "enemy-1".to_owned(),
            backend_object_id: Some(1),
            label: "Spider".to_owned(),
            kind: crate::brain::tactical_frame::EntityKind::Enemy,
            tile: Some(TilePosition { x: 12, y: 21 }),
            relative: Some(TilePosition { x: 1, y: 0 }),
            distance: Some(1.0),
            alive: Some(true),
            is_merchant: Some(false),
            interactable: Some(false),
            hostile: Some(true),
            targeting_you: Some(true),
        });
        frame.nearby_drops.push(Drop {
            id: "drop-1".to_owned(),
            item_id: Some("silk".to_owned()),
            label: Some("Spider Silk".to_owned()),
            tile: Some(TilePosition { x: 11, y: 21 }),
            relative: Some(TilePosition { x: 0, y: 0 }),
            distance: Some(0.0),
        });
        frame.local_waypoints.push(ReachableWaypoint {
            tile: TilePosition { x: 13, y: 21 },
            direction: crate::world::map::CardinalDirection::East,
            path_length_tiles: 3,
        });

        let full = serde_json::to_vec(&frame).expect("full frame serializes");
        let input = TacticalInput::from(&frame);
        let compact = serde_json::to_vec(&input).expect("input serializes");
        let value: serde_json::Value = serde_json::from_slice(&compact).expect("valid JSON");

        assert!(value["local_map"].get("tiles").is_none());
        assert_eq!(value["local_map"]["ascii"], "...\n.@S\n...");
        assert_eq!(value["nearby_entities"][0]["id"], "enemy-1");
        assert_eq!(value["nearby_drops"][0]["id"], "drop-1");
        assert_eq!(value["local_waypoints"][0]["direction"], "east");
        assert_eq!(value["local_waypoints"][0]["tile"]["x"], 13);
        assert!(compact.len() * 10 < full.len());
    }

    #[test]
    fn keeps_failed_movement_destination_in_compact_model_input() {
        let now = chrono::Utc::now();
        let mut frame = TacticalFrame::empty(StrategicIntent::default());
        frame.recent_actions.push(ActionOutcome {
            packet_id: uuid::Uuid::new_v4(),
            decision_id: uuid::Uuid::new_v4(),
            action_id: uuid::Uuid::new_v4(),
            action_index: 0,
            action_kind: "move_to".to_owned(),
            started_at: now,
            recorded_at: now,
            duration_ms: 1_750,
            status: OutcomeStatus::Failed,
            reason_code: Some("movement_stalled".to_owned()),
            detail: "movement_stalled".to_owned(),
            destination_tile: Some(TilePosition { x: 16, y: 20 }),
            source_frame_revision: 7,
            strategic_revision: 3,
            resulting_frame_revision: Some(8),
        });

        let input = TacticalInput::from(&frame);

        assert_eq!(
            input.recent_actions[0].destination_tile,
            Some(TilePosition { x: 16, y: 20 })
        );
        assert_eq!(
            input.recent_actions[0].reason_code.as_deref(),
            Some("movement_stalled")
        );
    }

    #[test]
    fn keeps_complete_scene_transition_names_in_tactical_input() {
        let now = chrono::Utc::now();
        let mut frame = TacticalFrame::empty(StrategicIntent::default());
        frame.generated_at = now;
        frame.recent_events.push(crate::world::events::GameEvent {
            sequence: Some(8),
            observed_at: now,
            origin: crate::world::events::GameEventOrigin::Derived,
            kind: crate::world::events::GameEventKind::SceneLeft,
            entity_id: None,
            amount: None,
            tile: None,
            detail: Some("town-square".to_owned()),
        });
        frame.recent_events.push(crate::world::events::GameEvent {
            sequence: Some(9),
            observed_at: now,
            origin: crate::world::events::GameEventOrigin::Derived,
            kind: crate::world::events::GameEventKind::SceneEntered,
            entity_id: None,
            amount: None,
            tile: None,
            detail: Some("bot-forest".to_owned()),
        });

        let input = TacticalInput::from(&frame);

        assert_eq!(
            input.recent_events[0].detail.as_deref(),
            Some("town-square")
        );
        assert_eq!(input.recent_events[1].detail.as_deref(), Some("bot-forest"));
    }

    #[test]
    fn movement_control_separates_policy_from_current_preemption_facts() {
        let now = chrono::Utc::now();
        let mut frame = TacticalFrame::empty(StrategicIntent::default());
        frame.generated_at = now;
        frame.strategic_intent.navigation_goal =
            Some(crate::brain::strategic_intent::NavigationGoal {
                scene: "bot-forest".to_owned(),
                destination: None,
                reason: "find the northern trail".to_owned(),
            });

        let quiet = TacticalInput::from(&frame).movement_control;
        assert_eq!(quiet.owner, MovementControlOwner::BodyStrategicNavigation);
        assert_eq!(quiet.state, MovementControlState::Assigned);
        assert!(!quiet.tactical_preemption_allowed_for.is_empty());
        assert!(quiet.tactical_preemption_facts_present.is_empty());

        frame.recent_events.push(crate::world::events::GameEvent {
            sequence: Some(10),
            observed_at: now,
            origin: crate::world::events::GameEventOrigin::Backend,
            kind: GameEventKind::DamageTaken,
            entity_id: Some("spider-1".to_owned()),
            amount: Some(8),
            tile: None,
            detail: None,
        });
        let damaged = TacticalInput::from(&frame).movement_control;
        assert_eq!(
            damaged.tactical_preemption_facts_present,
            [TacticalPreemptionFact::DamageTaken]
        );
    }

    #[test]
    fn compact_projection_keeps_whole_record_text_instead_of_chopping_strings() {
        let now = chrono::Utc::now();
        let detail = "x".repeat(700);
        let mut frame = TacticalFrame::empty(StrategicIntent::default());
        frame.generated_at = now;
        frame.recent_events.push(crate::world::events::GameEvent {
            sequence: Some(11),
            observed_at: now,
            origin: crate::world::events::GameEventOrigin::Backend,
            kind: GameEventKind::SceneEntered,
            entity_id: None,
            amount: None,
            tile: None,
            detail: Some(detail.clone()),
        });

        let input = TacticalInput::from(&frame);

        assert_eq!(
            input.recent_events[0].detail.as_deref(),
            Some(detail.as_str())
        );
    }

    #[test]
    fn supplies_reachable_adjacent_tiles_for_model_selected_entities() {
        let mut frame = TacticalFrame::empty(StrategicIntent::default());
        frame.self_state.position = Some(crate::world::Position {
            pixel: crate::world::PixelPosition { x: 32.0, y: 32.0 },
            tile: TilePosition { x: 1, y: 1 },
        });
        frame.map = LocalMap {
            revision: 1,
            origin_tile_x: 0,
            origin_tile_y: 0,
            width: 5,
            height: 3,
            tiles: (0..3)
                .flat_map(|y| {
                    (0..5).map(move |x| MapTile {
                        position: TilePosition { x, y },
                        kind: TileKind::Traversable,
                        walkable: Some(true),
                    })
                })
                .collect(),
            doors: Vec::new(),
            ascii: ".....\n.@.S.\n.....".to_owned(),
        };
        frame.nearby_entities.push(VisibleEntity {
            id: "npc-7".to_owned(),
            backend_object_id: Some(7),
            label: "Archivist".to_owned(),
            kind: crate::brain::tactical_frame::EntityKind::Npc,
            tile: Some(TilePosition { x: 3, y: 1 }),
            relative: Some(TilePosition { x: 2, y: 0 }),
            distance: Some(2.0),
            alive: Some(true),
            is_merchant: Some(false),
            interactable: Some(true),
            hostile: Some(false),
            targeting_you: Some(false),
        });

        let input = TacticalInput::from(&frame);

        assert_eq!(input.entity_approaches.len(), 1);
        assert_eq!(input.entity_approaches[0].entity_id, "npc-7");
        assert!(
            input.entity_approaches[0]
                .candidates
                .iter()
                .any(|candidate| candidate.tile == TilePosition { x: 2, y: 1 })
        );
        assert!(
            input.entity_approaches[0]
                .candidates
                .iter()
                .all(|candidate| candidate.tile != TilePosition { x: 3, y: 1 })
        );
    }
}
