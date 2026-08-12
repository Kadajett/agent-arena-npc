use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::world::TilePosition;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StrategicIntent {
    pub revision: u64,
    pub objective: String,
    pub subgoals: Vec<String>,
    pub priorities: Vec<Priority>,
    pub constraints: Vec<String>,
    pub risk_tolerance: f32,
    pub preferred_targets: Vec<String>,
    pub avoid: Vec<String>,
    pub navigation_goal: Option<NavigationGoal>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl Default for StrategicIntent {
    fn default() -> Self {
        Self {
            revision: 0,
            objective: "Stay alive while waiting for strategic direction.".to_owned(),
            subgoals: Vec::new(),
            priorities: vec![Priority::Survival],
            constraints: Vec::new(),
            risk_tolerance: 0.0,
            preferred_targets: Vec::new(),
            avoid: Vec::new(),
            navigation_goal: None,
            expires_at: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Survival,
    Objective,
    Loot,
    Kills,
    Social,
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NavigationGoal {
    pub scene: String,
    pub destination: Option<NamedDestination>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NamedDestination {
    pub name: String,
    pub tile: Option<TilePosition>,
}
