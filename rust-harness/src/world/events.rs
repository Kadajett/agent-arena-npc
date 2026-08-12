use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::TilePosition;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GameEvent {
    pub sequence: Option<u64>,
    pub observed_at: DateTime<Utc>,
    pub origin: GameEventOrigin,
    pub kind: GameEventKind,
    pub entity_id: Option<String>,
    pub amount: Option<i64>,
    pub tile: Option<TilePosition>,
    pub detail: Option<String>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum GameEventOrigin {
    Backend,
    Derived,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum GameEventKind {
    DamageTaken,
    DamageDealt,
    Heal,
    ItemUsed,
    EnemySeen,
    EnemySpawned,
    EnemyDespawned,
    TargetChanged,
    TargetKilled,
    LootDropped,
    LootPickedUp,
    MovementStarted,
    MovementStopped,
    MovementFailed,
    SceneEntered,
    SceneLeft,
    PlayerDied,
    PlayerRespawned,
    LevelChanged,
    ExperienceChanged,
}
