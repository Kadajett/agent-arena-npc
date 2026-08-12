use async_trait::async_trait;

use crate::{
    memory::{
        recall::{RecallQuery, StrategicRecall},
        relationships::{Relationship, RelationshipUpdate},
        semantic::SemanticMemoryRecord,
        working::WorkingMemory,
    },
    world::episodes::EpisodeSummary,
};

#[async_trait]
pub trait MemoryStore: Send + Sync {
    async fn load_working(&self, character_id: &str) -> anyhow::Result<WorkingMemory>;
    async fn save_working(&self, character_id: &str, memory: &WorkingMemory) -> anyhow::Result<()>;
    async fn record_episode(
        &self,
        character_id: &str,
        episode: &EpisodeSummary,
    ) -> anyhow::Result<()>;
    async fn apply_relationship(
        &self,
        character_id: &str,
        update: &RelationshipUpdate,
    ) -> anyhow::Result<Relationship>;
    async fn load_relationships(&self, character_id: &str) -> anyhow::Result<Vec<Relationship>>;
    async fn load_episodes(&self, character_id: &str) -> anyhow::Result<Vec<EpisodeSummary>>;
    async fn record_semantic(
        &self,
        character_id: &str,
        memory: &SemanticMemoryRecord,
    ) -> anyhow::Result<()>;
    async fn load_semantic(&self, character_id: &str) -> anyhow::Result<Vec<SemanticMemoryRecord>>;
    /// Retrieve bounded memories relevant to one strategic decision.
    async fn recall(
        &self,
        character_id: &str,
        query: &RecallQuery,
    ) -> anyhow::Result<StrategicRecall>;
}
