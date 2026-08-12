use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::world::TilePosition;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ActionOutcome {
    pub packet_id: Uuid,
    pub decision_id: Uuid,
    pub action_id: Uuid,
    pub action_index: usize,
    pub action_kind: String,
    pub started_at: DateTime<Utc>,
    pub recorded_at: DateTime<Utc>,
    pub duration_ms: u64,
    pub status: OutcomeStatus,
    pub reason_code: Option<String>,
    pub detail: String,
    /// The requested movement destination, when this outcome concerns movement.
    ///
    /// This fact lets the tactician avoid a destination that just failed. It is
    /// runtime-owned and never inferred from model prose.
    #[serde(default)]
    pub destination_tile: Option<TilePosition>,
    pub source_frame_revision: u64,
    pub strategic_revision: u64,
    pub resulting_frame_revision: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeStatus {
    Accepted,
    Succeeded,
    Failed,
    Rejected,
    Cancelled,
    Superseded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PacketTerminalStatus {
    Completed,
    Failed,
    Aborted,
    Cancelled,
    Superseded,
}

impl PacketTerminalStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Aborted => "aborted",
            Self::Cancelled => "cancelled",
            Self::Superseded => "superseded",
        }
    }
}
