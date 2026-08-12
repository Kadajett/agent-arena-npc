use std::collections::HashMap;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EpisodeSummary {
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub scene: String,
    pub summary: String,
    pub kills: u32,
    pub damage_dealt: i64,
    pub damage_received: i64,
    pub loot_collected: HashMap<String, u32>,
}
