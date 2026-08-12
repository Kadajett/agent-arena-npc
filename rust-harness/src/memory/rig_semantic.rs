use std::{collections::HashMap, sync::Arc, time::Instant};

use async_trait::async_trait;
use rig_core::{
    embeddings::{Embed, EmbedError, EmbeddingsBuilder, TextEmbedder},
    vector_store::{
        VectorStoreIndex,
        in_memory_store::{InMemoryVectorIndex, InMemoryVectorStore},
        request::{Filter, SearchFilter, VectorSearchRequest},
    },
};
use rig_fastembed::{Client as FastembedClient, EmbeddingModel, FastembedModel};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::sync::Mutex;

use crate::{
    memory::{
        recall::{RecallQuery, StrategicRecall},
        relationships::{Relationship, RelationshipUpdate},
        semantic::SemanticMemoryRecord,
        store::MemoryStore,
        working::WorkingMemory,
    },
    observability::{AnalyticsEvent, AnalyticsSink, EventLevel},
    world::episodes::EpisodeSummary,
};

/// Versioned settings for the derived local semantic index.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalRagConfig {
    pub enabled: bool,
    pub minimum_score: f64,
}

impl Default for LocalRagConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            minimum_score: 0.25,
        }
    }
}

const INDEX_VERSION: &str = "rig-local-rag-v1";
const EMBEDDING_MODEL: &str = "fastembed/all-minilm-l6-v2-q";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct MemoryDocument {
    id: String,
    character_id: String,
    memory_kind: String,
    text: String,
    payload: String,
}

impl Embed for MemoryDocument {
    fn embed(&self, embedder: &mut TextEmbedder) -> Result<(), EmbedError> {
        // Character identity and typed payload are metadata, not embedding text.
        embedder.embed(self.text.clone());
        Ok(())
    }
}

type RigIndex = InMemoryVectorIndex<EmbeddingModel, MemoryDocument>;

enum CharacterIndex {
    Empty,
    Ready(Arc<RigIndex>),
}

#[derive(Default)]
struct IndexState {
    model: Option<EmbeddingModel>,
    characters: HashMap<String, CharacterIndex>,
}

/// A durable-memory adapter that adds Rig semantic retrieval at the
/// `MemoryStore` seam.
///
/// The wrapped store remains authoritative. The FastEmbed/Rig index is a local
/// derived cache, built lazily per character and invalidated after semantic
/// writes. If model initialization, embedding, or vector search fails, recall
/// returns the wrapped store's deterministic lexical result and emits an
/// explicit degraded-mode event.
pub struct RigSemanticMemoryStore {
    inner: Arc<dyn MemoryStore>,
    analytics: Arc<dyn AnalyticsSink>,
    config: LocalRagConfig,
    state: Mutex<IndexState>,
}

impl RigSemanticMemoryStore {
    pub fn new(
        inner: Arc<dyn MemoryStore>,
        analytics: Arc<dyn AnalyticsSink>,
        config: LocalRagConfig,
    ) -> Self {
        Self {
            inner,
            analytics,
            config,
            state: Mutex::new(IndexState::default()),
        }
    }

    async fn semantic_recall(
        &self,
        character_id: &str,
        query: &RecallQuery,
    ) -> anyhow::Result<StrategicRecall> {
        let semantic_limit = query.limits.semantic_memories.min(16);
        let relationship_limit = query.limits.relationships.min(16);
        let episode_limit = query.limits.episodes.min(12);
        let requested = semantic_limit + relationship_limit + episode_limit;
        if requested == 0 {
            return Ok(StrategicRecall::default());
        }

        let started = Instant::now();
        self.analytics.record(
            AnalyticsEvent::new("memory.rag_retrieval_started", EventLevel::Debug)
                .character(character_id)
                .correlation(query.recall_id)
                .attribute("index_version", INDEX_VERSION)
                .attribute("embedding_model", EMBEDDING_MODEL)
                .attribute("requested_count", usize_to_u64(requested)),
        );

        let index = self.index_for(character_id, query.recall_id).await?;
        let Some(index) = index else {
            self.record_retrieval_completed(
                character_id,
                query,
                started,
                &StrategicRecall::default(),
                None,
                None,
            );
            return Ok(StrategicRecall::default());
        };

        let semantic_query = format!(
            "{}\nscene: {}\nvisible people: {}",
            query.text,
            query.scene.as_deref().unwrap_or("unknown"),
            query.visible_people.join(", ")
        );
        let request = VectorSearchRequest::builder()
            .query(semantic_query)
            // Search the complete per-character derived index once, then apply
            // the caller's independent type limits without re-embedding the
            // same query three times.
            .samples(usize_to_u64(index.len()))
            .threshold(self.config.minimum_score)
            .filter(Filter::eq(
                "character_id",
                serde_json::Value::String(character_id.to_owned()),
            ))
            .build();
        let handle = tokio::runtime::Handle::current();
        let mut results = tokio::task::spawn_blocking(move || {
            handle.block_on(index.top_n::<MemoryDocument>(request))
        })
        .await
        .map_err(|_| anyhow::anyhow!("local RAG retrieval task failed"))??;
        results.sort_by(|left, right| right.0.total_cmp(&left.0));

        let min_score = results.iter().map(|(score, _, _)| *score).reduce(f64::min);
        let max_score = results.iter().map(|(score, _, _)| *score).reduce(f64::max);
        let mut recall = StrategicRecall::default();
        for (_, _, document) in results {
            match document.memory_kind.as_str() {
                "semantic" if recall.semantic_memories.len() < semantic_limit => {
                    recall.semantic_memories.push(decode(&document.payload)?);
                }
                "relationship" if recall.relationships.len() < relationship_limit => {
                    recall.relationships.push(decode(&document.payload)?);
                }
                "episode" if recall.episode_summaries.len() < episode_limit => {
                    recall.episode_summaries.push(decode(&document.payload)?);
                }
                _ => {}
            }
        }
        self.record_retrieval_completed(
            character_id,
            query,
            started,
            &recall,
            min_score,
            max_score,
        );
        Ok(recall)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the complete derived-index lifecycle stays behind the one MemoryStore recall seam so callers cannot observe partial index state"
    )]
    async fn index_for(
        &self,
        character_id: &str,
        correlation_id: uuid::Uuid,
    ) -> anyhow::Result<Option<Arc<RigIndex>>> {
        {
            let state = self.state.lock().await;
            if let Some(entry) = state.characters.get(character_id) {
                return Ok(match entry {
                    CharacterIndex::Empty => None,
                    CharacterIndex::Ready(index) => Some(index.clone()),
                });
            }
        }

        let started = Instant::now();
        self.analytics.record(
            AnalyticsEvent::new("memory.rag_index_build_started", EventLevel::Info)
                .character(character_id)
                .correlation(correlation_id)
                .attribute("index_version", INDEX_VERSION)
                .attribute("embedding_model", EMBEDDING_MODEL),
        );
        let (semantic, relationships, episodes) = match tokio::try_join!(
            self.inner.load_semantic(character_id),
            self.inner.load_relationships(character_id),
            self.inner.load_episodes(character_id),
        ) {
            Ok(records) => records,
            Err(error) => {
                self.record_index_failed(character_id, correlation_id, started, "durable_source");
                return Err(error);
            }
        };
        let mut documents =
            Vec::with_capacity(semantic.len() + relationships.len() + episodes.len());
        for record in semantic {
            documents.push(MemoryDocument {
                id: record.memory_id.to_string(),
                character_id: character_id.to_owned(),
                memory_kind: "semantic".to_owned(),
                text: format!(
                    "kind: {}\nsubject: {}\nsummary: {}\nevidence: {:?}\nsource: {}",
                    record.kind, record.subject, record.summary, record.evidence, record.source
                ),
                payload: serde_json::to_string(&record)?,
            });
        }
        for record in relationships {
            documents.push(MemoryDocument {
                id: format!("relationship:{}", record.person_id),
                character_id: character_id.to_owned(),
                memory_kind: "relationship".to_owned(),
                text: format!(
                    "person: {}\nname: {}\nopinion: {}\ntrust: {}",
                    record.person_id, record.display_name, record.opinion, record.trust
                ),
                payload: serde_json::to_string(&record)?,
            });
        }
        for record in episodes {
            documents.push(MemoryDocument {
                id: format!(
                    "episode:{}:{}:{}",
                    record.started_at.timestamp_millis(),
                    record.ended_at.timestamp_millis(),
                    record.scene
                ),
                character_id: character_id.to_owned(),
                memory_kind: "episode".to_owned(),
                text: format!("scene: {}\nsummary: {}", record.scene, record.summary),
                payload: serde_json::to_string(&record)?,
            });
        }
        if documents.is_empty() {
            self.state
                .lock()
                .await
                .characters
                .insert(character_id.to_owned(), CharacterIndex::Empty);
            self.record_index_completed(character_id, correlation_id, started, 0);
            return Ok(None);
        }

        let model = match self.embedding_model().await {
            Ok(model) => model,
            Err(error) => {
                self.record_index_failed(character_id, correlation_id, started, "model_init");
                return Err(error);
            }
        };
        let document_count = documents.len();
        let handle = tokio::runtime::Handle::current();
        let model_for_index = model.clone();
        let index_result = tokio::task::spawn_blocking(move || {
            handle.block_on(async move {
                let embeddings = EmbeddingsBuilder::new(model_for_index.clone())
                    .documents(documents)?
                    .build()
                    .await?;
                let store = InMemoryVectorStore::from_documents_with_id_f(embeddings, |document| {
                    document.id.clone()
                });
                Ok::<RigIndex, anyhow::Error>(store.index(model_for_index))
            })
        })
        .await;
        let index = match index_result {
            Ok(Ok(index)) => index,
            Ok(Err(error)) => {
                self.record_index_failed(
                    character_id,
                    correlation_id,
                    started,
                    "embedding_or_index",
                );
                return Err(error);
            }
            Err(_) => {
                self.record_index_failed(character_id, correlation_id, started, "embedding_task");
                anyhow::bail!("local RAG index task failed");
            }
        };
        let index = Arc::new(index);
        self.state.lock().await.characters.insert(
            character_id.to_owned(),
            CharacterIndex::Ready(index.clone()),
        );
        self.record_index_completed(character_id, correlation_id, started, document_count);
        Ok(Some(index))
    }

    async fn embedding_model(&self) -> anyhow::Result<EmbeddingModel> {
        {
            let state = self.state.lock().await;
            if let Some(model) = &state.model {
                return Ok(model.clone());
            }
        }
        let model = tokio::task::spawn_blocking(|| {
            FastembedClient::new().embedding_model(&FastembedModel::AllMiniLML6V2Q)
        })
        .await
        .map_err(|_| anyhow::anyhow!("local embedding model initialization task failed"))??;
        self.state.lock().await.model = Some(model.clone());
        Ok(model)
    }

    async fn invalidate(&self, character_id: &str) {
        let invalidated = self
            .state
            .lock()
            .await
            .characters
            .remove(character_id)
            .is_some();
        if invalidated {
            self.analytics.record(
                AnalyticsEvent::new("memory.rag_index_invalidated", EventLevel::Debug)
                    .character(character_id)
                    .attribute("index_version", INDEX_VERSION)
                    .attribute("embedding_model", EMBEDDING_MODEL)
                    .attribute("reason", "semantic_memory_write"),
            );
        }
    }

    fn record_index_completed(
        &self,
        character_id: &str,
        correlation_id: uuid::Uuid,
        started: Instant,
        document_count: usize,
    ) {
        self.analytics.record(
            AnalyticsEvent::new("memory.rag_index_build_completed", EventLevel::Info)
                .character(character_id)
                .correlation(correlation_id)
                .attribute("index_version", INDEX_VERSION)
                .attribute("embedding_model", EMBEDDING_MODEL)
                .attribute("duration_ms", elapsed_ms(started))
                .attribute("document_count", usize_to_u64(document_count)),
        );
    }

    fn record_index_failed(
        &self,
        character_id: &str,
        correlation_id: uuid::Uuid,
        started: Instant,
        reason: &'static str,
    ) {
        self.analytics.record(
            AnalyticsEvent::new("memory.rag_index_build_failed", EventLevel::Warn)
                .character(character_id)
                .correlation(correlation_id)
                .attribute("index_version", INDEX_VERSION)
                .attribute("embedding_model", EMBEDDING_MODEL)
                .attribute("duration_ms", elapsed_ms(started))
                .attribute("error_class", reason),
        );
    }

    fn record_retrieval_completed(
        &self,
        character_id: &str,
        query: &RecallQuery,
        started: Instant,
        recall: &StrategicRecall,
        min_score: Option<f64>,
        max_score: Option<f64>,
    ) {
        let mut event = AnalyticsEvent::new("memory.rag_retrieval_completed", EventLevel::Debug)
            .character(character_id)
            .correlation(query.recall_id)
            .attribute("index_version", INDEX_VERSION)
            .attribute("embedding_model", EMBEDDING_MODEL)
            .attribute("duration_ms", elapsed_ms(started))
            .attribute(
                "requested_count",
                usize_to_u64(
                    query.limits.semantic_memories.min(16)
                        + query.limits.relationships.min(16)
                        + query.limits.episodes.min(12),
                ),
            )
            .attribute(
                "returned_count",
                usize_to_u64(
                    recall.semantic_memories.len()
                        + recall.relationships.len()
                        + recall.episode_summaries.len(),
                ),
            )
            .attribute(
                "semantic_count",
                usize_to_u64(recall.semantic_memories.len()),
            )
            .attribute(
                "relationship_count",
                usize_to_u64(recall.relationships.len()),
            )
            .attribute(
                "episode_count",
                usize_to_u64(recall.episode_summaries.len()),
            );
        if let Some(score) = min_score {
            event = event.attribute("minimum_score", score);
        }
        if let Some(score) = max_score {
            event = event.attribute("maximum_score", score);
        }
        self.analytics.record(event);
    }

    fn record_fallback(
        &self,
        character_id: &str,
        query: &RecallQuery,
        reason: &'static str,
        duration_ms: u64,
    ) {
        self.analytics.record(
            AnalyticsEvent::new("memory.rag_retrieval_fallback", EventLevel::Warn)
                .character(character_id)
                .correlation(query.recall_id)
                .attribute("index_version", INDEX_VERSION)
                .attribute("embedding_model", EMBEDDING_MODEL)
                .attribute("fallback", "deterministic_lexical")
                .attribute("reason", reason)
                .attribute("duration_ms", duration_ms),
        );
    }
}

#[async_trait]
impl MemoryStore for RigSemanticMemoryStore {
    async fn load_working(&self, character_id: &str) -> anyhow::Result<WorkingMemory> {
        self.inner.load_working(character_id).await
    }

    async fn save_working(&self, character_id: &str, memory: &WorkingMemory) -> anyhow::Result<()> {
        self.inner.save_working(character_id, memory).await
    }

    async fn record_episode(
        &self,
        character_id: &str,
        episode: &EpisodeSummary,
    ) -> anyhow::Result<()> {
        self.inner.record_episode(character_id, episode).await?;
        self.invalidate(character_id).await;
        Ok(())
    }

    async fn apply_relationship(
        &self,
        character_id: &str,
        update: &RelationshipUpdate,
    ) -> anyhow::Result<Relationship> {
        let relationship = self.inner.apply_relationship(character_id, update).await?;
        self.invalidate(character_id).await;
        Ok(relationship)
    }

    async fn load_relationships(&self, character_id: &str) -> anyhow::Result<Vec<Relationship>> {
        self.inner.load_relationships(character_id).await
    }

    async fn load_episodes(&self, character_id: &str) -> anyhow::Result<Vec<EpisodeSummary>> {
        self.inner.load_episodes(character_id).await
    }

    async fn record_semantic(
        &self,
        character_id: &str,
        memory: &SemanticMemoryRecord,
    ) -> anyhow::Result<()> {
        self.inner.record_semantic(character_id, memory).await?;
        self.invalidate(character_id).await;
        Ok(())
    }

    async fn load_semantic(&self, character_id: &str) -> anyhow::Result<Vec<SemanticMemoryRecord>> {
        self.inner.load_semantic(character_id).await
    }

    async fn recall(
        &self,
        character_id: &str,
        query: &RecallQuery,
    ) -> anyhow::Result<StrategicRecall> {
        // The inner call supplies authoritative working state and bounded
        // relationship/episode recall. Its semantic result is also the explicit
        // deterministic fallback if the derived Rig index is unavailable.
        let mut recall = self.inner.recall(character_id, query).await?;
        if !self.config.enabled {
            self.record_fallback(character_id, query, "disabled", 0);
            return Ok(recall);
        }
        let started = Instant::now();
        match self.semantic_recall(character_id, query).await {
            Ok(semantic) => {
                recall.semantic_memories = semantic.semantic_memories;
                recall.relationships = semantic.relationships;
                recall.episode_summaries = semantic.episode_summaries;
            }
            Err(_) => self.record_fallback(
                character_id,
                query,
                "index_or_embedding_unavailable",
                elapsed_ms(started),
            ),
        }
        Ok(recall)
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn decode<T: DeserializeOwned>(payload: &str) -> anyhow::Result<T> {
    serde_json::from_str(payload).map_err(anyhow::Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        memory::{recall::RecallLimits, semantic::MemoryEvidence, sqlite_store::SqliteMemoryStore},
        observability::RecordingAnalyticsSink,
    };
    use chrono::Utc;
    use rig_core::{
        embeddings::EmbeddingsBuilder,
        test_utils::MockEmbeddingModel,
        vector_store::{VectorStoreIndex, in_memory_store::InMemoryVectorStore},
    };

    #[tokio::test]
    async fn rig_vector_index_enforces_character_metadata_filter() {
        let documents = ["cassian", "guy"]
            .into_iter()
            .map(|character_id| MemoryDocument {
                id: format!("{character_id}-memory"),
                character_id: character_id.to_owned(),
                memory_kind: "semantic".to_owned(),
                text: "a legendary song hidden in the forest".to_owned(),
                payload: "{}".to_owned(),
            })
            .collect::<Vec<_>>();
        let embeddings = EmbeddingsBuilder::new(MockEmbeddingModel)
            .documents(documents)
            .expect("documents are embeddable")
            .build()
            .await
            .expect("mock embeddings build");
        let index = InMemoryVectorStore::from_documents_with_id_f(embeddings, |document| {
            document.id.clone()
        })
        .index(MockEmbeddingModel);
        let results = index
            .top_n::<MemoryDocument>(
                VectorSearchRequest::builder()
                    .query("legendary forest song")
                    .samples(10)
                    .filter(Filter::eq(
                        "character_id",
                        serde_json::Value::String("cassian".to_owned()),
                    ))
                    .build(),
            )
            .await
            .expect("Rig vector search");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].2.character_id, "cassian");
    }

    #[tokio::test]
    async fn disabled_local_rag_uses_lexical_fallback_without_logging_memory_text() {
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let sqlite: Arc<dyn MemoryStore> = Arc::new(
            SqliteMemoryStore::open_in_memory(analytics.clone())
                .await
                .expect("memory store opens"),
        );
        sqlite
            .record_semantic(
                "cassian",
                &SemanticMemoryRecord {
                    memory_id: uuid::Uuid::new_v4(),
                    kind: "treasure_clue".to_owned(),
                    subject: "secret golden peacock".to_owned(),
                    summary: "private-memory-marker is behind the moon gate".to_owned(),
                    evidence: MemoryEvidence::Firsthand,
                    source: "test".to_owned(),
                    source_id: None,
                    occurred_at: None,
                    recorded_at: Utc::now(),
                },
            )
            .await
            .expect("memory persists");
        let store = RigSemanticMemoryStore::new(
            sqlite,
            analytics.clone(),
            LocalRagConfig {
                enabled: false,
                minimum_score: 0.25,
            },
        );
        let recall = store
            .recall(
                "cassian",
                &RecallQuery {
                    recall_id: uuid::Uuid::new_v4(),
                    text: "find the golden peacock".to_owned(),
                    scene: None,
                    visible_people: Vec::new(),
                    limits: RecallLimits {
                        semantic_memories: 2,
                        relationships: 0,
                        episodes: 0,
                    },
                },
            )
            .await
            .expect("fallback recall succeeds");

        assert_eq!(recall.semantic_memories.len(), 1);
        let events = analytics.events();
        assert!(
            events
                .iter()
                .any(|event| event.name == "memory.rag_retrieval_fallback")
        );
        let serialized = serde_json::to_string(&events).expect("events serialize");
        assert!(!serialized.contains("private-memory-marker"));
        assert!(!serialized.contains("secret golden peacock"));
    }
}
