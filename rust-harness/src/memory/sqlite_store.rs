use std::{path::Path, sync::Arc, time::Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio_rusqlite::{
    Connection,
    rusqlite::{self, OptionalExtension},
};

use crate::{
    memory::{
        recall::{RecallLimits, RecallQuery, StrategicRecall},
        relationships::{Relationship, RelationshipUpdate},
        semantic::{MemoryEvidence, SemanticMemoryRecord},
        store::MemoryStore,
        working::WorkingMemory,
    },
    observability::{AnalyticsEvent, AnalyticsSink, EventLevel},
    world::episodes::EpisodeSummary,
};

const SCHEMA_VERSION: i64 = 1;

/// Durable typed character memory backed by a local `SQLite` database.
///
/// The same database may also contain Rig conversation-memory tables. Each
/// adapter owns its schema name and uses an independent `SQLite` connection.
#[derive(Clone)]
pub struct SqliteMemoryStore {
    connection: Connection,
    analytics: Arc<dyn AnalyticsSink>,
}

impl SqliteMemoryStore {
    /// Open or create the typed memory database.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot open or initialize the database.
    pub async fn open(
        path: impl AsRef<Path>,
        analytics: Arc<dyn AnalyticsSink>,
    ) -> anyhow::Result<Self> {
        let connection = Connection::open(path).await?;
        Self::initialize(connection, analytics).await
    }

    /// Open an isolated typed memory database for tests.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot initialize the database.
    #[cfg(test)]
    pub async fn open_in_memory(analytics: Arc<dyn AnalyticsSink>) -> anyhow::Result<Self> {
        let connection = Connection::open_in_memory().await?;
        Self::initialize(connection, analytics).await
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the ordered schema is kept together so its ownership and foreign keys remain auditable"
    )]
    async fn initialize(
        connection: Connection,
        analytics: Arc<dyn AnalyticsSink>,
    ) -> anyhow::Result<Self> {
        connection
            .call(|connection| {
                connection.busy_timeout(std::time::Duration::from_secs(5))?;
                connection.execute_batch(
                    "PRAGMA foreign_keys = ON;
                     PRAGMA journal_mode = WAL;
                     CREATE TABLE IF NOT EXISTS memory_schema (
                         name TEXT PRIMARY KEY NOT NULL,
                         version INTEGER NOT NULL
                     );
                     CREATE TABLE IF NOT EXISTS working_memory (
                         character_id TEXT PRIMARY KEY NOT NULL,
                         memory_json TEXT NOT NULL,
                         updated_at TEXT NOT NULL
                     );
                     CREATE TABLE IF NOT EXISTS episode_memories (
                         episode_id INTEGER PRIMARY KEY AUTOINCREMENT,
                         character_id TEXT NOT NULL,
                         started_at TEXT NOT NULL,
                         ended_at TEXT NOT NULL,
                         scene TEXT NOT NULL,
                         summary TEXT NOT NULL,
                         episode_json TEXT NOT NULL,
                         recorded_at TEXT NOT NULL,
                         UNIQUE(character_id, started_at, ended_at, scene, summary)
                     );
                     CREATE INDEX IF NOT EXISTS episode_memories_character_time
                         ON episode_memories(character_id, ended_at);
                     CREATE TABLE IF NOT EXISTS relationships (
                         character_id TEXT NOT NULL,
                         person_id TEXT NOT NULL,
                         display_name TEXT NOT NULL,
                         trust REAL NOT NULL,
                         opinion TEXT NOT NULL,
                         last_updated TEXT NOT NULL,
                         PRIMARY KEY(character_id, person_id)
                     );
                     CREATE TABLE IF NOT EXISTS relationship_evidence (
                         evidence_id INTEGER PRIMARY KEY AUTOINCREMENT,
                         character_id TEXT NOT NULL,
                         person_id TEXT NOT NULL,
                         trust_delta REAL NOT NULL,
                         reason TEXT NOT NULL,
                         recorded_at TEXT NOT NULL,
                         FOREIGN KEY(character_id, person_id)
                             REFERENCES relationships(character_id, person_id)
                     );
                     CREATE INDEX IF NOT EXISTS relationship_evidence_lookup
                         ON relationship_evidence(character_id, person_id, evidence_id);
                     CREATE TABLE IF NOT EXISTS semantic_memories (
                         memory_id TEXT PRIMARY KEY NOT NULL,
                         character_id TEXT NOT NULL,
                         kind TEXT NOT NULL,
                         subject TEXT NOT NULL,
                         summary TEXT NOT NULL,
                         evidence TEXT NOT NULL,
                         source TEXT NOT NULL,
                         source_id TEXT,
                         occurred_at TEXT,
                         recorded_at TEXT NOT NULL
                     );
                     CREATE INDEX IF NOT EXISTS semantic_memories_character_time
                         ON semantic_memories(character_id, recorded_at);",
                )?;
                connection.execute(
                    "INSERT INTO memory_schema(name, version) VALUES ('typed', ?1)
                     ON CONFLICT(name) DO NOTHING",
                    [SCHEMA_VERSION],
                )?;
                let version: i64 = connection.query_row(
                    "SELECT version FROM memory_schema WHERE name = 'typed'",
                    [],
                    |row| row.get(0),
                )?;
                if version != SCHEMA_VERSION {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                Ok::<(), rusqlite::Error>(())
            })
            .await?;
        Ok(Self {
            connection,
            analytics,
        })
    }

    fn started(&self, character_id: &str, operation: &'static str) -> Instant {
        self.analytics.record(
            AnalyticsEvent::new("memory.typed_operation_started", EventLevel::Debug)
                .character(character_id)
                .attribute("operation", operation),
        );
        Instant::now()
    }

    fn completed(
        &self,
        character_id: &str,
        operation: &'static str,
        started: Instant,
        record_count: usize,
    ) {
        self.analytics.record(
            AnalyticsEvent::new("memory.typed_operation_completed", EventLevel::Debug)
                .character(character_id)
                .attribute("operation", operation)
                .attribute("duration_ms", elapsed_ms(started))
                .attribute("record_count", usize_to_u64(record_count)),
        );
    }

    fn failed(&self, character_id: &str, operation: &'static str, started: Instant) {
        self.analytics.record(
            AnalyticsEvent::new("memory.typed_operation_failed", EventLevel::Warn)
                .character(character_id)
                .attribute("operation", operation)
                .attribute("duration_ms", elapsed_ms(started))
                .attribute("error_class", "sqlite_or_serialization"),
        );
    }
}

#[async_trait]
#[allow(
    clippy::too_many_lines,
    reason = "the typed store implementation keeps all transaction boundaries visible in one auditable adapter"
)]
impl MemoryStore for SqliteMemoryStore {
    async fn load_working(&self, character_id: &str) -> anyhow::Result<WorkingMemory> {
        const OPERATION: &str = "load_working";
        let started = self.started(character_id, OPERATION);
        let character = character_id.to_owned();
        let row = self
            .connection
            .call(move |connection| {
                let mut statement = connection
                    .prepare("SELECT memory_json FROM working_memory WHERE character_id = ?1")?;
                let mut rows = statement.query([character])?;
                rows.next()?.map(|row| row.get::<_, String>(0)).transpose()
            })
            .await;
        let result = match row {
            Ok(Some(json)) => serde_json::from_str(&json).map_err(anyhow::Error::from),
            Ok(None) => Ok(WorkingMemory::default()),
            Err(error) => Err(anyhow::Error::from(error)),
        };
        match &result {
            Ok(memory) => {
                self.completed(character_id, OPERATION, started, memory_entry_count(memory));
            }
            Err(_) => self.failed(character_id, OPERATION, started),
        }
        result
    }

    async fn save_working(&self, character_id: &str, memory: &WorkingMemory) -> anyhow::Result<()> {
        const OPERATION: &str = "save_working";
        let started = self.started(character_id, OPERATION);
        let json = match serde_json::to_string(memory) {
            Ok(json) => json,
            Err(error) => {
                self.failed(character_id, OPERATION, started);
                return Err(error.into());
            }
        };
        let character = character_id.to_owned();
        let updated_at = Utc::now().to_rfc3339();
        let result = self
            .connection
            .call(move |connection| {
                connection.execute(
                    "INSERT INTO working_memory(character_id, memory_json, updated_at)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(character_id) DO UPDATE SET
                         memory_json = excluded.memory_json,
                         updated_at = excluded.updated_at",
                    rusqlite::params![character, json, updated_at],
                )?;
                Ok::<(), rusqlite::Error>(())
            })
            .await;
        match result {
            Ok(()) => {
                self.completed(character_id, OPERATION, started, memory_entry_count(memory));
                Ok(())
            }
            Err(error) => {
                self.failed(character_id, OPERATION, started);
                Err(error.into())
            }
        }
    }

    async fn record_episode(
        &self,
        character_id: &str,
        episode: &EpisodeSummary,
    ) -> anyhow::Result<()> {
        const OPERATION: &str = "record_episode";
        let started = self.started(character_id, OPERATION);
        let payload = serde_json::to_string(episode)?;
        let character = character_id.to_owned();
        let episode = episode.clone();
        let result = self
            .connection
            .call(move |connection| {
                connection.execute(
                    "INSERT INTO episode_memories(
                         character_id, started_at, ended_at, scene, summary,
                         episode_json, recorded_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(character_id, started_at, ended_at, scene, summary)
                     DO NOTHING",
                    rusqlite::params![
                        character,
                        episode.started_at.to_rfc3339(),
                        episode.ended_at.to_rfc3339(),
                        episode.scene,
                        episode.summary,
                        payload,
                        Utc::now().to_rfc3339(),
                    ],
                )
            })
            .await;
        match result {
            Ok(written) => {
                self.completed(character_id, OPERATION, started, written);
                Ok(())
            }
            Err(error) => {
                self.failed(character_id, OPERATION, started);
                Err(error.into())
            }
        }
    }

    async fn apply_relationship(
        &self,
        character_id: &str,
        update: &RelationshipUpdate,
    ) -> anyhow::Result<Relationship> {
        const OPERATION: &str = "apply_relationship";
        let started = self.started(character_id, OPERATION);
        let character = character_id.to_owned();
        let update = update.clone();
        let result = self
            .connection
            .call(move |connection| {
                let transaction = connection.transaction()?;
                let current = transaction
                    .query_row(
                        "SELECT trust FROM relationships
                         WHERE character_id = ?1 AND person_id = ?2",
                        rusqlite::params![character, update.person_id],
                        |row| row.get::<_, f32>(0),
                    )
                    .optional()?
                    .unwrap_or(0.0);
                let relationship = Relationship {
                    person_id: update.person_id,
                    display_name: update.display_name,
                    trust: (current + update.trust_delta).clamp(-1.0, 1.0),
                    opinion: update.reason.clone(),
                    last_updated: Utc::now(),
                };
                transaction.execute(
                    "INSERT INTO relationships(
                         character_id, person_id, display_name, trust, opinion, last_updated
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(character_id, person_id) DO UPDATE SET
                         display_name = excluded.display_name,
                         trust = excluded.trust,
                         opinion = excluded.opinion,
                         last_updated = excluded.last_updated",
                    rusqlite::params![
                        character,
                        relationship.person_id,
                        relationship.display_name,
                        relationship.trust,
                        relationship.opinion,
                        relationship.last_updated.to_rfc3339(),
                    ],
                )?;
                transaction.execute(
                    "INSERT INTO relationship_evidence(
                         character_id, person_id, trust_delta, reason, recorded_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![
                        character,
                        relationship.person_id,
                        update.trust_delta,
                        update.reason,
                        relationship.last_updated.to_rfc3339(),
                    ],
                )?;
                transaction.commit()?;
                Ok::<Relationship, rusqlite::Error>(relationship)
            })
            .await;
        match result {
            Ok(relationship) => {
                self.completed(character_id, OPERATION, started, 1);
                Ok(relationship)
            }
            Err(error) => {
                self.failed(character_id, OPERATION, started);
                Err(error.into())
            }
        }
    }

    async fn load_relationships(&self, character_id: &str) -> anyhow::Result<Vec<Relationship>> {
        const OPERATION: &str = "load_relationships";
        let started = self.started(character_id, OPERATION);
        let character = character_id.to_owned();
        let result = self
            .connection
            .call(move |connection| {
                let mut statement = connection.prepare(
                    "SELECT person_id, display_name, trust, opinion, last_updated
                     FROM relationships WHERE character_id = ?1 ORDER BY person_id",
                )?;
                statement
                    .query_map([character], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, f32>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, rusqlite::Error>>()
            })
            .await;
        let relationships = match result {
            Ok(rows) => rows
                .into_iter()
                .map(|(person_id, display_name, trust, opinion, last_updated)| {
                    Ok(Relationship {
                        person_id,
                        display_name,
                        trust,
                        opinion,
                        last_updated: parse_timestamp(&last_updated)?,
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>(),
            Err(error) => Err(error.into()),
        };
        match &relationships {
            Ok(rows) => self.completed(character_id, OPERATION, started, rows.len()),
            Err(_) => self.failed(character_id, OPERATION, started),
        }
        relationships
    }

    async fn load_episodes(&self, character_id: &str) -> anyhow::Result<Vec<EpisodeSummary>> {
        const OPERATION: &str = "load_episodes";
        let started = self.started(character_id, OPERATION);
        let character = character_id.to_owned();
        let result = self
            .connection
            .call(move |connection| {
                let mut statement = connection.prepare(
                    "SELECT episode_json FROM episode_memories
                     WHERE character_id = ?1 ORDER BY ended_at, episode_id",
                )?;
                statement
                    .query_map([character], |row| row.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, rusqlite::Error>>()
            })
            .await;
        let episodes = match result {
            Ok(rows) => rows
                .into_iter()
                .map(|json| serde_json::from_str(&json).map_err(anyhow::Error::from))
                .collect::<anyhow::Result<Vec<_>>>(),
            Err(error) => Err(error.into()),
        };
        match &episodes {
            Ok(rows) => self.completed(character_id, OPERATION, started, rows.len()),
            Err(_) => self.failed(character_id, OPERATION, started),
        }
        episodes
    }

    async fn record_semantic(
        &self,
        character_id: &str,
        memory: &SemanticMemoryRecord,
    ) -> anyhow::Result<()> {
        const OPERATION: &str = "record_semantic";
        let started = self.started(character_id, OPERATION);
        let character = character_id.to_owned();
        let memory = memory.clone();
        let result = self
            .connection
            .call(move |connection| {
                connection.execute(
                    "INSERT INTO semantic_memories(
                         memory_id, character_id, kind, subject, summary, evidence,
                         source, source_id, occurred_at, recorded_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                     ON CONFLICT(memory_id) DO NOTHING",
                    rusqlite::params![
                        memory.memory_id.to_string(),
                        character,
                        memory.kind,
                        memory.subject,
                        memory.summary,
                        evidence_name(memory.evidence),
                        memory.source,
                        memory.source_id,
                        memory.occurred_at.map(|value| value.to_rfc3339()),
                        memory.recorded_at.to_rfc3339(),
                    ],
                )
            })
            .await;
        match result {
            Ok(written) => {
                self.completed(character_id, OPERATION, started, written);
                Ok(())
            }
            Err(error) => {
                self.failed(character_id, OPERATION, started);
                Err(error.into())
            }
        }
    }

    async fn load_semantic(&self, character_id: &str) -> anyhow::Result<Vec<SemanticMemoryRecord>> {
        const OPERATION: &str = "load_semantic";
        let started = self.started(character_id, OPERATION);
        let character = character_id.to_owned();
        let result = self
            .connection
            .call(move |connection| {
                let mut statement = connection.prepare(
                    "SELECT memory_id, kind, subject, summary, evidence, source,
                            source_id, occurred_at, recorded_at
                     FROM semantic_memories
                     WHERE character_id = ?1 ORDER BY recorded_at, memory_id",
                )?;
                statement
                    .query_map([character], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, Option<String>>(6)?,
                            row.get::<_, Option<String>>(7)?,
                            row.get::<_, String>(8)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, rusqlite::Error>>()
            })
            .await;
        let memories = match result {
            Ok(rows) => rows
                .into_iter()
                .map(
                    |(
                        memory_id,
                        kind,
                        subject,
                        summary,
                        evidence,
                        source,
                        source_id,
                        occurred_at,
                        recorded_at,
                    )| {
                        Ok(SemanticMemoryRecord {
                            memory_id: uuid::Uuid::parse_str(&memory_id)?,
                            kind,
                            subject,
                            summary,
                            evidence: parse_evidence(&evidence)?,
                            source,
                            source_id,
                            occurred_at: occurred_at.as_deref().map(parse_timestamp).transpose()?,
                            recorded_at: parse_timestamp(&recorded_at)?,
                        })
                    },
                )
                .collect::<anyhow::Result<Vec<_>>>(),
            Err(error) => Err(error.into()),
        };
        match &memories {
            Ok(rows) => self.completed(character_id, OPERATION, started, rows.len()),
            Err(_) => self.failed(character_id, OPERATION, started),
        }
        memories
    }

    async fn recall(
        &self,
        character_id: &str,
        query: &RecallQuery,
    ) -> anyhow::Result<StrategicRecall> {
        const OPERATION: &str = "recall";
        const CANDIDATE_LIMIT: i64 = 64;
        let started = self.started(character_id, OPERATION);
        let character = character_id.to_owned();
        let rows = self
            .connection
            .call(move |connection| {
                let working = connection
                    .query_row(
                        "SELECT memory_json FROM working_memory WHERE character_id = ?1",
                        [&character],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?;

                let relationships = {
                    let mut statement = connection.prepare(
                        "SELECT person_id, display_name, trust, opinion, last_updated
                         FROM relationships WHERE character_id = ?1
                         ORDER BY last_updated DESC LIMIT ?2",
                    )?;
                    statement
                        .query_map(rusqlite::params![character, CANDIDATE_LIMIT], |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, f32>(2)?,
                                row.get::<_, String>(3)?,
                                row.get::<_, String>(4)?,
                            ))
                        })?
                        .collect::<Result<Vec<_>, rusqlite::Error>>()?
                };
                let semantic = {
                    let mut statement = connection.prepare(
                        "SELECT memory_id, kind, subject, summary, evidence, source,
                                source_id, occurred_at, recorded_at
                         FROM semantic_memories WHERE character_id = ?1
                         ORDER BY recorded_at DESC LIMIT ?2",
                    )?;
                    statement
                        .query_map(rusqlite::params![character, CANDIDATE_LIMIT], |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, String>(3)?,
                                row.get::<_, String>(4)?,
                                row.get::<_, String>(5)?,
                                row.get::<_, Option<String>>(6)?,
                                row.get::<_, Option<String>>(7)?,
                                row.get::<_, String>(8)?,
                            ))
                        })?
                        .collect::<Result<Vec<_>, rusqlite::Error>>()?
                };
                let episodes = {
                    let mut statement = connection.prepare(
                        "SELECT episode_json FROM episode_memories
                         WHERE character_id = ?1 ORDER BY ended_at DESC LIMIT ?2",
                    )?;
                    statement
                        .query_map(rusqlite::params![character, CANDIDATE_LIMIT], |row| {
                            row.get::<_, String>(0)
                        })?
                        .collect::<Result<Vec<_>, rusqlite::Error>>()?
                };
                Ok::<_, rusqlite::Error>((working, relationships, semantic, episodes))
            })
            .await;

        let result = (|| -> anyhow::Result<StrategicRecall> {
            let (working_json, relationship_rows, semantic_rows, episode_rows) = rows?;
            let working = working_json
                .map(|json| serde_json::from_str(&json))
                .transpose()?
                .unwrap_or_default();
            let relationships = relationship_rows
                .into_iter()
                .map(|(person_id, display_name, trust, opinion, last_updated)| {
                    Ok(Relationship {
                        person_id,
                        display_name,
                        trust,
                        opinion,
                        last_updated: parse_timestamp(&last_updated)?,
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            let semantic_memories = semantic_rows
                .into_iter()
                .map(
                    |(
                        memory_id,
                        kind,
                        subject,
                        summary,
                        evidence,
                        source,
                        source_id,
                        occurred_at,
                        recorded_at,
                    )| {
                        Ok(SemanticMemoryRecord {
                            memory_id: uuid::Uuid::parse_str(&memory_id)?,
                            kind,
                            subject,
                            summary,
                            evidence: parse_evidence(&evidence)?,
                            source,
                            source_id,
                            occurred_at: occurred_at.as_deref().map(parse_timestamp).transpose()?,
                            recorded_at: parse_timestamp(&recorded_at)?,
                        })
                    },
                )
                .collect::<anyhow::Result<Vec<_>>>()?;
            let episode_summaries = episode_rows
                .into_iter()
                .map(|json| serde_json::from_str(&json))
                .collect::<Result<Vec<EpisodeSummary>, _>>()?;

            Ok(rank_recall(
                working,
                relationships,
                semantic_memories,
                episode_summaries,
                query,
            ))
        })();
        match &result {
            Ok(recall) => self.completed(
                character_id,
                OPERATION,
                started,
                recall.relationships.len()
                    + recall.semantic_memories.len()
                    + recall.episode_summaries.len(),
            ),
            Err(_) => self.failed(character_id, OPERATION, started),
        }
        result
    }
}

fn rank_recall(
    working: WorkingMemory,
    relationships: Vec<Relationship>,
    semantic_memories: Vec<SemanticMemoryRecord>,
    episode_summaries: Vec<EpisodeSummary>,
    query: &RecallQuery,
) -> StrategicRecall {
    let limits = bounded_limits(query.limits);
    let terms = recall_terms(&query.text);
    let visible = query
        .visible_people
        .iter()
        .flat_map(|person| recall_terms(person))
        .collect::<std::collections::HashSet<_>>();

    let mut relationships = relationships
        .into_iter()
        .filter_map(|record| {
            let searchable = format!(
                "{} {} {}",
                record.person_id, record.display_name, record.opinion
            );
            let score = overlap_score(&searchable, &terms)
                + overlap_score(&searchable, &visible).saturating_mul(4);
            (score > 0).then_some((score, record))
        })
        .collect::<Vec<_>>();
    relationships.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| right.1.last_updated.cmp(&left.1.last_updated))
    });

    let mut semantic_memories = semantic_memories
        .into_iter()
        .filter_map(|record| {
            let evidence_weight = match record.evidence {
                MemoryEvidence::Confirmed | MemoryEvidence::Firsthand => 2,
                MemoryEvidence::Hearsay | MemoryEvidence::StrategicBelief => 1,
                MemoryEvidence::MigratedUnknown => 0,
            };
            let subject_overlap = overlap_score(&record.subject, &terms);
            let summary_overlap = overlap_score(&record.summary, &terms);
            let metadata_overlap =
                overlap_score(&format!("{} {}", record.kind, record.source), &terms);
            let overlap = subject_overlap + summary_overlap + metadata_overlap;
            (overlap > 0).then_some((
                subject_overlap.saturating_mul(6)
                    + summary_overlap.saturating_mul(3)
                    + metadata_overlap
                    + evidence_weight,
                record,
            ))
        })
        .collect::<Vec<_>>();
    semantic_memories.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| right.1.recorded_at.cmp(&left.1.recorded_at))
    });

    let mut episode_summaries = episode_summaries
        .into_iter()
        .filter_map(|episode| {
            let overlap = overlap_score(&episode.summary, &terms);
            let scene_match = query
                .scene
                .as_ref()
                .is_some_and(|scene| scene == &episode.scene);
            (overlap > 0 || scene_match).then_some((
                overlap.saturating_mul(3) + usize::from(scene_match).saturating_mul(4),
                episode,
            ))
        })
        .collect::<Vec<_>>();
    episode_summaries.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| right.1.ended_at.cmp(&left.1.ended_at))
    });

    StrategicRecall {
        working,
        relationships: relationships
            .into_iter()
            .take(limits.relationships)
            .map(|(_, record)| record)
            .collect(),
        semantic_memories: semantic_memories
            .into_iter()
            .take(limits.semantic_memories)
            .map(|(_, record)| record)
            .collect(),
        episode_summaries: episode_summaries
            .into_iter()
            .take(limits.episodes)
            .map(|(_, record)| record)
            .collect(),
    }
}

fn bounded_limits(limits: RecallLimits) -> RecallLimits {
    RecallLimits {
        semantic_memories: limits.semantic_memories.min(16),
        relationships: limits.relationships.min(16),
        episodes: limits.episodes.min(12),
    }
}

fn recall_terms(value: &str) -> std::collections::HashSet<String> {
    const STOP_WORDS: &[&str] = &[
        "and",
        "the",
        "for",
        "with",
        "that",
        "this",
        "from",
        "into",
        "while",
        "what",
        "when",
        "where",
        "have",
        "has",
        "was",
        "were",
        "current",
        "character",
    ];
    value
        .split(|character: char| !character.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|term| term.len() >= 3 && !STOP_WORDS.contains(&term.as_str()))
        .collect()
}

fn overlap_score(value: &str, terms: &std::collections::HashSet<String>) -> usize {
    recall_terms(value).intersection(terms).count()
}

fn memory_entry_count(memory: &WorkingMemory) -> usize {
    usize::from(memory.goal.is_some()) + memory.plan.len() + memory.todo.len() + memory.notes.len()
}

fn evidence_name(evidence: MemoryEvidence) -> &'static str {
    match evidence {
        MemoryEvidence::Confirmed => "confirmed",
        MemoryEvidence::Firsthand => "firsthand",
        MemoryEvidence::Hearsay => "hearsay",
        MemoryEvidence::StrategicBelief => "strategic_belief",
        MemoryEvidence::MigratedUnknown => "migrated_unknown",
    }
}

fn parse_evidence(value: &str) -> anyhow::Result<MemoryEvidence> {
    match value {
        "confirmed" => Ok(MemoryEvidence::Confirmed),
        "firsthand" => Ok(MemoryEvidence::Firsthand),
        "hearsay" => Ok(MemoryEvidence::Hearsay),
        "strategic_belief" => Ok(MemoryEvidence::StrategicBelief),
        "migrated_unknown" => Ok(MemoryEvidence::MigratedUnknown),
        _ => anyhow::bail!("unknown semantic memory evidence class"),
    }
}

fn parse_timestamp(value: &str) -> anyhow::Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc))
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::TimeZone;

    use super::*;
    use crate::{
        memory::working::{Goal, PlanStep, WorkStatus},
        observability::RecordingAnalyticsSink,
    };

    #[tokio::test]
    async fn working_memory_survives_reopen_and_remains_character_scoped() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("memory.sqlite3");
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let store = SqliteMemoryStore::open(&path, analytics.clone())
            .await
            .expect("open memory store");
        let memory = WorkingMemory {
            goal: Some(Goal {
                aim: "Find the lost song".to_owned(),
                done: Some("The score is recovered".to_owned()),
                why: None,
            }),
            plan: vec![PlanStep {
                step_id: Some(uuid::Uuid::new_v4()),
                what: "Ask at the inn".to_owned(),
                status: WorkStatus::Doing,
                note: None,
                tries: 1,
                done_when: None,
                evidence: Vec::new(),
                reevaluate_when: Vec::new(),
            }],
            ..WorkingMemory::default()
        };
        store
            .save_working("cassian", &memory)
            .await
            .expect("save working memory");
        drop(store);

        let reopened = SqliteMemoryStore::open(&path, analytics)
            .await
            .expect("reopen memory store");
        assert_eq!(
            reopened
                .load_working("cassian")
                .await
                .expect("load working memory"),
            memory
        );
        assert_eq!(
            reopened
                .load_working("guy")
                .await
                .expect("load other character"),
            WorkingMemory::default()
        );
    }

    #[tokio::test]
    async fn relationship_updates_are_transactional_and_bounded() {
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let store = SqliteMemoryStore::open_in_memory(analytics)
            .await
            .expect("open memory store");
        for delta in [0.75, 0.75] {
            store
                .apply_relationship(
                    "cassian",
                    &RelationshipUpdate {
                        person_id: "rival-1".to_owned(),
                        display_name: "A Worthy Rival".to_owned(),
                        trust_delta: delta,
                        reason: "Kept their word".to_owned(),
                    },
                )
                .await
                .expect("apply relationship update");
        }

        let relationships = store
            .load_relationships("cassian")
            .await
            .expect("load relationships");
        assert_eq!(relationships.len(), 1);
        assert!((relationships[0].trust - 1.0).abs() < f32::EPSILON);
        assert!(
            store
                .load_relationships("guy")
                .await
                .expect("load other relationships")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn semantic_memory_is_idempotent_and_keeps_provenance() {
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let store = SqliteMemoryStore::open_in_memory(analytics)
            .await
            .expect("open memory store");
        let record = SemanticMemoryRecord {
            memory_id: uuid::Uuid::new_v4(),
            kind: "treasure_clue".to_owned(),
            subject: "sunken bell".to_owned(),
            summary: "An inn visitor claimed the bell is east of town.".to_owned(),
            evidence: MemoryEvidence::Hearsay,
            source: "dialogue".to_owned(),
            source_id: Some("event-91".to_owned()),
            occurred_at: Some(Utc.with_ymd_and_hms(2026, 8, 12, 1, 2, 3).unwrap()),
            recorded_at: Utc.with_ymd_and_hms(2026, 8, 12, 1, 2, 4).unwrap(),
        };
        store
            .record_semantic("cassian", &record)
            .await
            .expect("record semantic memory");
        store
            .record_semantic("cassian", &record)
            .await
            .expect("record semantic memory again");

        assert_eq!(
            store
                .load_semantic("cassian")
                .await
                .expect("load semantic memory"),
            vec![record]
        );
    }

    #[tokio::test]
    async fn episodes_are_idempotent_and_telemetry_excludes_content() {
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let store = SqliteMemoryStore::open_in_memory(analytics.clone())
            .await
            .expect("open memory store");
        let episode = EpisodeSummary {
            started_at: Utc.with_ymd_and_hms(2026, 8, 12, 1, 0, 0).unwrap(),
            ended_at: Utc.with_ymd_and_hms(2026, 8, 12, 1, 1, 0).unwrap(),
            scene: "secret-scene".to_owned(),
            summary: "private episode summary".to_owned(),
            kills: 1,
            damage_dealt: 10,
            damage_received: 2,
            loot_collected: HashMap::new(),
        };
        store
            .record_episode("cassian", &episode)
            .await
            .expect("record episode");
        store
            .record_episode("cassian", &episode)
            .await
            .expect("record duplicate episode");
        assert_eq!(
            store
                .load_episodes("cassian")
                .await
                .expect("load persisted episodes"),
            vec![episode.clone()]
        );
        assert!(
            store
                .load_episodes("guy")
                .await
                .expect("load other character episodes")
                .is_empty()
        );

        let serialized = serde_json::to_string(&analytics.events()).expect("serialize telemetry");
        assert!(!serialized.contains("private episode summary"));
        assert!(!serialized.contains("secret-scene"));
        assert!(serialized.contains("memory.typed_operation_completed"));
    }

    #[tokio::test]
    async fn strategic_recall_is_relevant_bounded_and_keeps_working_state_complete() {
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let store = SqliteMemoryStore::open_in_memory(analytics)
            .await
            .expect("open memory store");
        let working = WorkingMemory {
            goal: Some(Goal {
                aim: "Find the sunken bell".to_owned(),
                done: None,
                why: None,
            }),
            ..WorkingMemory::default()
        };
        store
            .save_working("cassian", &working)
            .await
            .expect("save working");
        for (subject, summary) in [
            ("sunken bell", "The sunken bell is east of town."),
            ("bakery", "The baker makes rye loaves."),
            ("bell keeper", "The bell keeper visits the harbor."),
        ] {
            store
                .record_semantic(
                    "cassian",
                    &SemanticMemoryRecord {
                        memory_id: uuid::Uuid::new_v4(),
                        kind: "discovery".to_owned(),
                        subject: subject.to_owned(),
                        summary: summary.to_owned(),
                        evidence: MemoryEvidence::Firsthand,
                        source: "test".to_owned(),
                        source_id: None,
                        occurred_at: None,
                        recorded_at: Utc::now(),
                    },
                )
                .await
                .expect("record semantic");
        }
        let recall = store
            .recall(
                "cassian",
                &RecallQuery {
                    recall_id: uuid::Uuid::new_v4(),
                    text: "Find the sunken bell at the harbor".to_owned(),
                    scene: None,
                    visible_people: Vec::new(),
                    limits: RecallLimits {
                        semantic_memories: 1,
                        relationships: 0,
                        episodes: 0,
                    },
                },
            )
            .await
            .expect("recall");

        assert_eq!(recall.working, working);
        assert_eq!(recall.semantic_memories.len(), 1);
        assert_eq!(recall.semantic_memories[0].subject, "sunken bell");
        assert!(recall.relationships.is_empty());
        assert!(recall.episode_summaries.is_empty());
    }
}
