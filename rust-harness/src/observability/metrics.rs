use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionTrace {
    pub decision_id: Uuid,
    pub frame_revision: u64,
    pub strategic_revision: u64,
    pub prompt_version: String,
    pub model_id: String,
    pub latency_ms: u64,
}
