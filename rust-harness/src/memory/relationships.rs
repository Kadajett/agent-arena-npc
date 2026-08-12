use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Relationship {
    pub person_id: String,
    pub display_name: String,
    pub trust: f32,
    pub opinion: String,
    pub last_updated: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RelationshipUpdate {
    pub person_id: String,
    pub display_name: String,
    pub trust_delta: f32,
    pub reason: String,
}
