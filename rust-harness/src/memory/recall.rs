use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    memory::{relationships::Relationship, semantic::SemanticMemoryRecord, working::WorkingMemory},
    world::episodes::EpisodeSummary,
};

/// A bounded, character-scoped long-term-memory lookup.
///
/// The strategist supplies facts that describe its current decision. The memory
/// subsystem owns retrieval and provenance; callers never query `SQLite` directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecallQuery {
    pub recall_id: uuid::Uuid,
    pub text: String,
    pub scene: Option<String>,
    pub visible_people: Vec<String>,
    pub limits: RecallLimits,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecallLimits {
    pub semantic_memories: usize,
    pub relationships: usize,
    pub episodes: usize,
}

impl Default for RecallLimits {
    fn default() -> Self {
        Self {
            semantic_memories: 8,
            relationships: 8,
            episodes: 6,
        }
    }
}

/// Typed memory supplied to one strategic decision.
///
/// Working state is always included in full. Long-tail collections are bounded
/// and selected by the memory store for the current query.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StrategicRecall {
    pub working: WorkingMemory,
    pub semantic_memories: Vec<SemanticMemoryRecord>,
    pub relationships: Vec<Relationship>,
    pub episode_summaries: Vec<EpisodeSummary>,
}
