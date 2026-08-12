use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    brain::strategic_intent::StrategicIntent,
    execution::outcome::ActionOutcome,
    world::{
        Position, TilePosition,
        combat::CombatSnapshot,
        events::GameEvent,
        map::{LocalMap, ReachableExit, ReachableWaypoint},
    },
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TacticalFrame {
    pub revision: u64,
    pub perception_revision: u64,
    pub inventory_revision: u64,
    pub generated_at: DateTime<Utc>,
    pub self_state: SelfState,
    pub combat: CombatSnapshot,
    pub census: VisibilityCensus,
    pub nearby_entities: Vec<VisibleEntity>,
    pub nearby_drops: Vec<Drop>,
    pub map: LocalMap,
    pub exits: Vec<ReachableExit>,
    #[serde(default)]
    pub local_waypoints: Vec<ReachableWaypoint>,
    pub recent_events: Vec<GameEvent>,
    pub recent_actions: Vec<ActionOutcome>,
    pub strategic_intent: StrategicIntent,
}

impl TacticalFrame {
    pub fn empty(strategic_intent: StrategicIntent) -> Self {
        Self {
            revision: 0,
            perception_revision: 0,
            inventory_revision: 0,
            generated_at: Utc::now(),
            self_state: SelfState::default(),
            combat: CombatSnapshot::default(),
            census: VisibilityCensus::default(),
            nearby_entities: Vec::new(),
            nearby_drops: Vec::new(),
            map: LocalMap::default(),
            exits: Vec::new(),
            local_waypoints: Vec::new(),
            recent_events: Vec::new(),
            recent_actions: Vec::new(),
            strategic_intent,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VisibilityCensus {
    pub reported_total_players: Option<u32>,
    pub listed_other_players: usize,
    pub reported_total_objects: Option<u32>,
    pub listed_objects: usize,
    pub object_list_truncated: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SelfState {
    pub scene: Option<String>,
    pub position: Option<Position>,
    pub health: Option<i32>,
    pub max_health: Option<i32>,
    pub level: Option<i32>,
    pub experience: Option<i64>,
    pub class_path: Option<String>,
    pub alive: Option<bool>,
    pub recently_died: Option<bool>,
    pub moving: Option<bool>,
    pub combat_actions: Vec<CombatActionAvailability>,
    pub inventory: Vec<CarriedItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CombatActionAvailability {
    pub id: String,
    pub available: Option<bool>,
    pub cooldown_remaining_ms: Option<u64>,
    pub target_kind: TargetKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    None,
    SelfTarget,
    Entity,
    Position,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CarriedItem {
    pub id: String,
    pub label: String,
    pub quantity: u32,
    pub usable: Option<bool>,
    pub equipment: Option<bool>,
    pub equipped: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct VisibleEntity {
    pub id: String,
    pub backend_object_id: Option<i64>,
    pub label: String,
    pub kind: EntityKind,
    pub tile: Option<TilePosition>,
    pub relative: Option<TilePosition>,
    pub distance: Option<f32>,
    pub alive: Option<bool>,
    pub is_merchant: Option<bool>,
    pub interactable: Option<bool>,
    pub hostile: Option<bool>,
    pub targeting_you: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Player,
    Npc,
    Enemy,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Drop {
    pub id: String,
    pub item_id: Option<String>,
    pub label: Option<String>,
    pub tile: Option<TilePosition>,
    pub relative: Option<TilePosition>,
    pub distance: Option<f32>,
}
