use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    brain::{
        strategic_intent::StrategicIntent,
        tactical_frame::{EntityKind, TacticalFrame},
    },
    memory::recall::StrategicRecall,
    world::{TilePosition, events::GameEventKind},
};

const MAX_STRATEGIC_ENTITIES: usize = 32;
const MAX_STRATEGIC_EXITS: usize = 16;
const MAX_STRATEGIC_TRANSITIONS: usize = 8;
pub const STRATEGIC_INPUT_PROTOCOL_VERSION: u32 = 1;

/// Bounded, self-contained facts for one long-horizon decision.
///
/// Event summaries are data supplied by the runtime. They are not instructions
/// and they deliberately do not expose a game gateway or conversation transcript.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StrategicInput {
    pub protocol_version: u32,
    pub character_id: String,
    pub persona: String,
    pub current_intent: StrategicIntent,
    /// Typed current work plus bounded, relevant durable memories.
    pub memory: StrategicRecall,
    pub world: StrategicWorldSnapshot,
    pub moments: Vec<StrategicMoment>,
}

/// Small authoritative world view for one long-horizon decision.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StrategicWorldSnapshot {
    pub frame_revision: u64,
    pub scene: Option<String>,
    pub tile: Option<TilePosition>,
    pub health: Option<i32>,
    pub max_health: Option<i32>,
    pub combat_active: Option<bool>,
    pub visible_entities: Vec<StrategicVisibleEntity>,
    pub visible_drop_count: usize,
    pub carried_item_count: usize,
    pub exits: Vec<StrategicExit>,
    pub recent_scene_transitions: Vec<StrategicSceneTransition>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StrategicVisibleEntity {
    pub id: String,
    pub label: String,
    pub kind: EntityKind,
    pub tile: Option<TilePosition>,
    pub distance: Option<f32>,
    pub hostile: Option<bool>,
    pub is_merchant: Option<bool>,
    pub interactable: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StrategicExit {
    pub tile: TilePosition,
    pub destination_scene: Option<String>,
    pub label: Option<String>,
    pub path_length_tiles: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StrategicSceneTransition {
    pub kind: GameEventKind,
    pub scene: Option<String>,
}

impl From<&TacticalFrame> for StrategicWorldSnapshot {
    fn from(frame: &TacticalFrame) -> Self {
        let visible_entities = frame
            .nearby_entities
            .iter()
            .take(MAX_STRATEGIC_ENTITIES)
            .map(|entity| StrategicVisibleEntity {
                id: entity.id.clone(),
                label: entity.label.clone(),
                kind: entity.kind,
                tile: entity.tile,
                distance: entity.distance,
                hostile: entity.hostile,
                is_merchant: entity.is_merchant,
                interactable: entity.interactable,
            })
            .collect();
        let exits = frame
            .exits
            .iter()
            .take(MAX_STRATEGIC_EXITS)
            .map(|exit| StrategicExit {
                tile: exit.tile,
                destination_scene: exit.destination_scene.clone(),
                label: exit.label.clone(),
                path_length_tiles: exit.path_length_tiles,
            })
            .collect();
        let recent_scene_transitions = frame
            .recent_events
            .iter()
            .rev()
            .filter(|event| {
                matches!(
                    event.kind,
                    GameEventKind::SceneEntered | GameEventKind::SceneLeft
                )
            })
            .take(MAX_STRATEGIC_TRANSITIONS)
            .map(|event| StrategicSceneTransition {
                kind: event.kind,
                scene: event.detail.clone(),
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        Self {
            frame_revision: frame.revision,
            scene: frame.self_state.scene.clone(),
            tile: frame.self_state.position.map(|position| position.tile),
            health: frame.self_state.health,
            max_health: frame.self_state.max_health,
            combat_active: frame.combat.active,
            visible_entities,
            visible_drop_count: frame.nearby_drops.len(),
            carried_item_count: frame.self_state.inventory.len(),
            exits,
            recent_scene_transitions,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StrategicMoment {
    pub kind: StrategicMomentKind,
    pub summary: String,
    pub speaker: Option<String>,
    pub dialogue_channel: Option<String>,
    pub navigation_arrival: Option<StrategicNavigationArrival>,
}

/// Model-visible physical completion facts for a navigation mission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StrategicNavigationArrival {
    pub destination_scene: String,
    pub destination_tile: Option<TilePosition>,
    pub destination_name: String,
    pub arrived_scene: Option<String>,
    pub arrived_tile: Option<TilePosition>,
    pub attempts: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StrategicMomentKind {
    World,
    GoalBlocked,
    PersonSpoke,
    NavigationArrived,
    EpisodeFinished,
    Reflection,
}
