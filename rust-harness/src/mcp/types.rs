use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    mcp::observation::{ObservedItem, ObservedParty},
    world::PixelPosition,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MoveDirection {
    Up,
    Down,
    Left,
    Right,
}

/// Chat channels currently accepted by `arena_say`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatChannel {
    Scene,
    Global,
    Private,
}

impl ChatChannel {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Scene => "scene",
            Self::Global => "global",
            Self::Private => "private",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MelodyInstrument {
    Lute,
    Flute,
    Horn,
    Bell,
}

impl MelodyInstrument {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lute => "lute",
            Self::Flute => "flute",
            Self::Horn => "horn",
            Self::Bell => "bell",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TradeSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TacticsStyle {
    CloseUp,
    LongRange,
    DuckAndWeave,
    Flee,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TacticsMode {
    SemiAuto,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CombatTarget {
    Object { object_index: String },
    Player { session_id: String, player_id: i64 },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapObservation {
    #[serde(skip)]
    pub requested_radius: Option<u32>,
    pub grid_available: Option<bool>,
    pub map: Option<String>,
    pub scene_name: Option<String>,
    pub scene_size: Option<SceneSize>,
    #[serde(default)]
    pub doors: Vec<MapDoor>,
    pub origin: Option<PixelPosition>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneSize {
    pub width_tiles: Option<u32>,
    pub height_tiles: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurveyResult {
    pub scene_name: Option<String>,
    pub within: Option<u32>,
    pub scene_size: Option<SceneSize>,
    pub grid_available: Option<bool>,
    pub counts: Option<SurveyCounts>,
    #[serde(default)]
    pub ways_out: Vec<SurveyWayOut>,
    #[serde(default)]
    pub survey: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurveyCounts {
    pub enemies: Option<u32>,
    pub people: Option<u32>,
    pub readables: Option<u32>,
    pub drops: Option<u32>,
    pub other_players: Option<u32>,
    pub ways_out: Option<u32>,
    pub beyond_range: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurveyWayOut {
    pub leads_to: Option<String>,
    pub locked: Option<bool>,
    pub lock_known: Option<bool>,
    pub required_key: Option<String>,
    pub enter_at: Option<SurveyTile>,
    #[serde(default)]
    pub tiles: Vec<SurveyTile>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurveyTile {
    pub row: i32,
    pub column: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapDoor {
    pub x: Option<f32>,
    pub y: Option<f32>,
    #[serde(alias = "column")]
    pub tile_x: Option<i32>,
    #[serde(alias = "row")]
    pub tile_y: Option<i32>,
    pub leads_to: Option<String>,
    pub label: Option<String>,
    pub locked: Option<bool>,
    pub lock_known: Option<bool>,
    pub required_key: Option<String>,
}

macro_rules! simple_result {
    ($name:ident { $($field:ident: $ty:ty),* $(,)? }) => {
        #[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub struct $name {
            $(pub $field: $ty,)*
            pub reason: Option<String>,
            pub message: Option<String>,
        }
    };
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SayResult {
    pub accepted: Option<bool>,
    pub said: Option<bool>,
    pub channel: Option<String>,
    pub to_player: Option<String>,
    pub reason: Option<String>,
    pub message: Option<String>,
}
simple_result!(PlayMelodyResult {
    accepted: Option<bool>,
    played: Option<bool>,
    instrument: Option<String>,
    times: Option<u8>,
    note_count: Option<u8>,
    notes: Option<String>,
    note: Option<String>
});
simple_result!(FeelResult { updated: Option<bool> });
simple_result!(MoveResult {
    accepted: Option<bool>,
    moved: Option<bool>,
    moving: Option<bool>,
    arrived: Option<bool>,
    came_to_rest: Option<bool>,
    x: Option<f32>,
    y: Option<f32>,
    tile_x: Option<i32>,
    tile_y: Option<i32>
});
simple_result!(PathResult {
    reachable: Option<bool>,
    path_length_tiles: Option<u32>
});
simple_result!(StopResult { stopped: Option<bool> });
simple_result!(UnstickResult {
    moved: Option<bool>,
    scene: Option<String>
});
simple_result!(DoorResult {
    entered: Option<bool>,
    scene: Option<String>
});
simple_result!(CombatResult {
    accepted: Option<bool>,
    action_type: Option<String>,
    target_kind: Option<String>,
    target_object_index: Option<String>,
    target_session_id: Option<String>,
    target_player_id: Option<i64>,
    // Compatibility fields for gateways that return an immediate outcome.
    attacked: Option<bool>,
    damage: Option<i64>,
    target_killed: Option<bool>
});
simple_result!(UseItemResult {
    used: Option<bool>,
    remaining: Option<u32>
});
simple_result!(EquipResult {
    changed: Option<bool>,
    item: Option<String>,
    equipped: Option<bool>,
    confirmed: Option<bool>
});
simple_result!(TacticsResult {
    style: Option<String>,
    style_is_own_choice: Option<bool>,
    mode: Option<String>
});
simple_result!(ThinkResult {
    recorded: Option<bool>,
    note: Option<String>
});
simple_result!(PickupResult {
    picked_up: Option<bool>,
    item: Option<String>
});
simple_result!(DisconnectResult { disconnected: Option<bool> });

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogueResult {
    pub opened: bool,
    pub object_id: Option<i64>,
    pub title: Option<String>,
    pub content: Option<String>,
    pub options: Option<std::collections::BTreeMap<String, String>>,
    pub reason: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EndDialogueResult {
    pub ended: Option<bool>,
    pub closed: Option<bool>,
    pub object_id: Option<i64>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchResult {
    pub id: Option<String>,
    pub status: String,
    #[serde(default)]
    pub participants: Vec<MatchParticipant>,
    pub winner_agent_id: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchParticipant {
    pub agent_id: Option<String>,
    pub player_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditBalance {
    pub balance: i64,
    pub currency: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditHistory {
    #[serde(default, alias = "history")]
    pub entries: Vec<CreditEntry>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditEntry {
    pub amount: Option<i64>,
    pub balance: Option<i64>,
    pub reason: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryQuery {
    pub after: Option<u64>,
    pub before: Option<u64>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub limit: Option<u16>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub generated_at: Option<String>,
    pub player: String,
    pub cursor: u64,
    pub oldest: u64,
    pub has_more: bool,
    #[serde(default)]
    pub summary: HistorySummary,
    #[serde(default)]
    pub events: Vec<HistoryEvent>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HistorySummary {
    pub event_count: u64,
    pub first_at: Option<String>,
    pub last_at: Option<String>,
    #[serde(default)]
    pub scenes: Vec<String>,
    #[serde(default)]
    pub counts: BTreeMap<String, u64>,
    #[serde(default)]
    pub combat: HistoryCombatSummary,
    #[serde(default)]
    pub movement: HistoryMovementSummary,
    #[serde(default)]
    pub items: HistoryItemSummary,
    #[serde(default)]
    pub progression: HistoryProgressionSummary,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HistoryCombatSummary {
    pub damage_dealt: i64,
    pub damage_taken: i64,
    pub deaths: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HistoryMovementSummary {
    pub commands: u64,
    pub failed: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HistoryItemSummary {
    pub bought: u64,
    pub sold: u64,
    pub used: u64,
    pub picked_up: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HistoryProgressionSummary {
    pub experience_gained: i64,
    pub levels_gained: u64,
    pub skills_learned: u64,
}

/// One lossless event at the external MCP boundary.
///
/// The world deliberately returns unknown future engine rows. Common lineage
/// fields are typed, while `fields` preserves the remainder without teaching
/// actor messages to depend on arbitrary JSON.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HistoryEvent {
    pub id: u64,
    pub at: Option<String>,
    pub scene: Option<String>,
    pub category: Option<String>,
    pub event_type: String,
    #[serde(rename = "decisionId")]
    pub decision_id: Option<String>,
    pub tool: Option<String>,
    pub success: Option<bool>,
    pub amount: Option<i64>,
    pub actor: Option<String>,
    #[serde(rename = "actorId")]
    pub actor_id: Option<String>,
    pub target: Option<String>,
    #[serde(default)]
    pub raw: Value,
    #[serde(default)]
    pub data: Value,
    #[serde(flatten)]
    pub fields: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartyActionResult {
    pub invited: Option<bool>,
    pub responded: Option<bool>,
    pub accepted: Option<bool>,
    pub confirmed: Option<bool>,
    pub changed: Option<bool>,
    pub target_player_id: Option<i64>,
    pub target_player_name: Option<String>,
    pub from_player_id: Option<i64>,
    pub removed_player_id: Option<i64>,
    pub party: Option<ObservedParty>,
    pub reason: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryResult {
    #[serde(default, alias = "items", alias = "inventory")]
    pub carrying: Vec<ObservedItem>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeListing {
    pub opened: bool,
    pub merchant: Option<String>,
    pub side: Option<String>,
    #[serde(default)]
    pub offers: Vec<TradeOffer>,
    pub reason: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeOffer {
    pub key: Option<String>,
    pub label: String,
    pub price: Option<ItemAmount>,
    pub payout: Option<ItemAmount>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeResult {
    pub traded: bool,
    pub merchant: Option<String>,
    pub item: Option<TradeItem>,
    pub quantity: Option<u32>,
    pub price: Option<ItemAmount>,
    pub payout: Option<ItemAmount>,
    pub reason: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradeItem {
    pub key: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemAmount {
    pub item_key: String,
    pub quantity: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentList {
    #[serde(default)]
    pub agents: Vec<RegisteredAgent>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredAgent {
    pub id: String,
    pub agent_name: Option<String>,
    pub player_name: String,
    pub class_path: Option<String>,
    pub selected_scene: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrationResult {
    pub agent: RegisteredAgent,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResult {
    pub logged_in: Option<bool>,
    pub agent: Option<RegisteredAgent>,
    pub session_id: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchCodeResult {
    pub code: Option<String>,
    pub watch_code: Option<String>,
    pub expires_at: Option<String>,
}
