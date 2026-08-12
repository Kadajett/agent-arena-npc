use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::TilePosition;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LocalMap {
    pub revision: u64,
    pub origin_tile_x: i32,
    pub origin_tile_y: i32,
    pub width: usize,
    pub height: usize,
    pub tiles: Vec<MapTile>,
    pub doors: Vec<Doorway>,
    pub ascii: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MapTile {
    pub position: TilePosition,
    pub kind: TileKind,
    pub walkable: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TileKind {
    Traversable,
    Blocked,
    Door,
    LockedDoor,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Doorway {
    pub tile: TilePosition,
    pub destination_scene: Option<String>,
    pub label: Option<String>,
    pub locked: Option<bool>,
    pub lock_known: Option<bool>,
    pub required_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReachableExit {
    pub tile: TilePosition,
    pub destination_scene: Option<String>,
    pub label: Option<String>,
    pub path_length_tiles: u32,
}

/// One exact, locally reachable waypoint offered to the tactical model.
///
/// Waypoints are physical facts derived from the structured map. They do not
/// imply that the character should travel in any direction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReachableWaypoint {
    pub tile: TilePosition,
    pub direction: CardinalDirection,
    pub path_length_tiles: u32,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CardinalDirection {
    North,
    East,
    South,
    West,
}
