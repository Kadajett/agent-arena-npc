use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The evidence class carried by a durable character memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryEvidence {
    Confirmed,
    Firsthand,
    Hearsay,
    StrategicBelief,
    MigratedUnknown,
}

/// A durable, concrete memory that can become a document in the local Rig index.
///
/// `SQLite` is authoritative. The vector index is derived from these records and
/// can be rebuilt without changing the record identifiers or provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SemanticMemoryRecord {
    pub memory_id: Uuid,
    pub kind: String,
    pub subject: String,
    pub summary: String,
    pub evidence: MemoryEvidence,
    pub source: String,
    pub source_id: Option<String>,
    pub occurred_at: Option<DateTime<Utc>>,
    pub recorded_at: DateTime<Utc>,
}
