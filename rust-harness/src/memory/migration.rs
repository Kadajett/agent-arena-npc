use std::{path::Path, sync::Arc};

use chrono::Utc;
use rig_core::{completion::Message, memory::ConversationMemory};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_rusqlite::{Connection, rusqlite};
use uuid::Uuid;

use crate::{
    memory::{
        semantic::{MemoryEvidence, SemanticMemoryRecord},
        sqlite_conversation::SqliteConversationMemory,
        sqlite_store::SqliteMemoryStore,
        store::MemoryStore,
        working::{Goal, PlanStep, TodoItem, WorkStatus, WorkingMemory},
    },
    observability::{AnalyticsEvent, AnalyticsSink, EventLevel},
};

#[derive(Debug, Clone)]
pub struct MastraMigrationOptions<'a> {
    pub character_id: &'a str,
    pub source_database: &'a Path,
    pub destination_database: &'a Path,
    pub legacy_conversations_file: Option<&'a Path>,
    pub visited_file: Option<&'a Path>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct MigrationReport {
    pub source_messages_read: usize,
    pub conversation_messages_written: usize,
    pub source_observations_archived: usize,
    pub source_rows_archived: usize,
    pub semantic_memories_written: usize,
    pub working_memory_written: bool,
    pub unsupported_messages_archived: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct LegacyMessage {
    id: String,
    thread_id: String,
    content: String,
    role: String,
    message_type: String,
    created_at: String,
    resource_id: Option<String>,
}

#[derive(Debug, Clone)]
struct LegacyObservation {
    id: String,
    payload: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyWorkingMemory {
    goal: Option<LegacyGoal>,
    #[serde(default)]
    plan: Vec<LegacyPlanStep>,
    #[serde(default)]
    todo: Vec<LegacyTodo>,
    #[serde(default)]
    people: Vec<Value>,
    #[serde(default)]
    places: Vec<Value>,
    #[serde(default)]
    goings_on: Vec<Value>,
    #[serde(default)]
    own_business: Vec<Value>,
    #[serde(default)]
    opinions: Vec<Value>,
    #[serde(default)]
    lately: Vec<Value>,
    #[serde(default)]
    notes: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct LegacyGoal {
    aim: String,
    done: Option<String>,
    why: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LegacyPlanStep {
    what: String,
    status: String,
    note: Option<String>,
    #[serde(default)]
    tries: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyTodo {
    what: String,
    status: String,
    note: Option<String>,
    asked_by: Option<String>,
}

/// Migrate one online-safe snapshot of a Mastra database into the Rust store.
///
/// Every source message and observational-memory row is archived unchanged.
/// Supported text turns are also converted to Rig messages. Working goal,
/// plan, and todo state is typed; other working-memory domains become semantic
/// records with explicit migrated-unknown provenance.
///
/// # Errors
///
/// Returns an error for a failed source integrity check, a previously completed
/// migration, malformed required working state, or any source/destination IO.
#[allow(
    clippy::too_many_lines,
    reason = "one transaction-oriented migration keeps archive and conversion counts causally aligned"
)]
pub async fn migrate_mastra_memory(
    options: &MastraMigrationOptions<'_>,
    analytics: Arc<dyn AnalyticsSink>,
) -> anyhow::Result<MigrationReport> {
    anyhow::ensure!(
        options.source_database != options.destination_database,
        "source and destination databases must differ"
    );
    let character_id = options.character_id.to_owned();
    let source = Connection::open(options.source_database).await?;
    assert_source_integrity(&source).await?;
    let source_bytes = std::fs::metadata(options.source_database)?.len();
    let (working_rows, messages, observations) = read_source(&source).await?;
    anyhow::ensure!(
        working_rows.len() <= 1,
        "expected at most one Mastra resource row"
    );

    let typed_store =
        SqliteMemoryStore::open(options.destination_database, analytics.clone()).await?;
    let conversation = SqliteConversationMemory::open(
        options.destination_database,
        &character_id,
        analytics.clone(),
    )
    .await?;
    let archive = Connection::open(options.destination_database).await?;
    initialize_archive(&archive).await?;
    refuse_completed_migration(&archive, &character_id).await?;

    analytics.record(
        AnalyticsEvent::new("memory.migration_started", EventLevel::Info)
            .character(&character_id)
            .attribute("source_bytes", source_bytes)
            .attribute("source_message_count", usize_to_u64(messages.len()))
            .attribute("source_observation_count", usize_to_u64(observations.len())),
    );

    let mut report = MigrationReport {
        source_messages_read: messages.len(),
        source_observations_archived: observations.len(),
        ..MigrationReport::default()
    };
    if let Some((resource_id, raw)) = working_rows.first() {
        archive_row(&archive, &character_id, "working_memory", resource_id, raw).await?;
        report.source_rows_archived += 1;
        let legacy: LegacyWorkingMemory = serde_json::from_str(raw)?;
        let working = convert_working(&legacy, &mut report.warnings);
        typed_store.save_working(&character_id, &working).await?;
        report.working_memory_written = true;
        for memory in semantic_records(&character_id, &legacy) {
            typed_store.record_semantic(&character_id, &memory).await?;
            report.semantic_memories_written += 1;
        }
    }

    let mut converted = Vec::new();
    for message in &messages {
        let archive_payload = serde_json::to_string(&json!({
            "thread_id": message.thread_id,
            "content": message.content,
            "role": message.role,
            "type": message.message_type,
            "created_at": message.created_at,
            "resource_id": message.resource_id,
        }))?;
        archive_row(
            &archive,
            &character_id,
            "mastra_message",
            &message.id,
            &archive_payload,
        )
        .await?;
        report.source_rows_archived += 1;
        if let Some(rig_message) = convert_message(message) {
            converted.push(rig_message);
        } else {
            report.unsupported_messages_archived += 1;
        }
    }
    for chunk in converted.chunks(500) {
        conversation.append("strategist", chunk.to_vec()).await?;
    }
    report.conversation_messages_written = converted.len();

    for observation in &observations {
        archive_row(
            &archive,
            &character_id,
            "mastra_observation",
            &observation.id,
            &observation.payload,
        )
        .await?;
        report.source_rows_archived += 1;
    }
    for (kind, path) in [
        (
            "legacy_conversations_file",
            options.legacy_conversations_file,
        ),
        ("visited_file", options.visited_file),
    ] {
        if let Some(path) = path {
            let payload = std::fs::read_to_string(path)?;
            archive_row(
                &archive,
                &character_id,
                kind,
                &path.display().to_string(),
                &payload,
            )
            .await?;
            report.source_rows_archived += 1;
        }
    }
    complete_migration(&archive, &character_id, source_bytes, &report).await?;
    analytics.record(
        AnalyticsEvent::new("memory.migration_completed", EventLevel::Info)
            .character(&character_id)
            .attribute(
                "source_messages_read",
                usize_to_u64(report.source_messages_read),
            )
            .attribute(
                "conversation_messages_written",
                usize_to_u64(report.conversation_messages_written),
            )
            .attribute(
                "source_rows_archived",
                usize_to_u64(report.source_rows_archived),
            )
            .attribute(
                "semantic_memories_written",
                usize_to_u64(report.semantic_memories_written),
            )
            .attribute("warning_count", usize_to_u64(report.warnings.len())),
    );
    Ok(report)
}

async fn assert_source_integrity(source: &Connection) -> anyhow::Result<()> {
    let result = source
        .call(|connection| {
            connection.query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0))
        })
        .await?;
    anyhow::ensure!(result == "ok", "source SQLite quick_check failed: {result}");
    Ok(())
}

type LegacySourceRows = (
    Vec<(String, String)>,
    Vec<LegacyMessage>,
    Vec<LegacyObservation>,
);

async fn read_source(source: &Connection) -> anyhow::Result<LegacySourceRows> {
    Ok(source
        .call(|connection| {
            let working = {
                let mut statement = connection.prepare(
                    "SELECT id, workingMemory FROM mastra_resources WHERE workingMemory IS NOT NULL",
                )?;
                statement
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<Result<Vec<_>, rusqlite::Error>>()?
            };
            let messages = {
                let mut statement = connection.prepare(
                    "SELECT id, thread_id, content, role, type, createdAt, resourceId
                     FROM mastra_messages ORDER BY createdAt, id",
                )?;
                statement
                    .query_map([], |row| {
                        Ok(LegacyMessage {
                            id: row.get(0)?,
                            thread_id: row.get(1)?,
                            content: row.get(2)?,
                            role: row.get(3)?,
                            message_type: row.get(4)?,
                            created_at: row.get(5)?,
                            resource_id: row.get(6)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, rusqlite::Error>>()?
            };
            let observations = {
                let mut statement = connection.prepare(
                    "SELECT id, json_object(
                        'lookupKey', lookupKey,
                        'scope', scope,
                        'resourceId', resourceId,
                        'threadId', threadId,
                        'activeObservations', activeObservations,
                        'activeObservationsPendingUpdate', activeObservationsPendingUpdate,
                        'originType', originType,
                        'config', config,
                        'generationCount', generationCount,
                        'lastObservedAt', lastObservedAt,
                        'lastReflectionAt', lastReflectionAt,
                        'pendingMessageTokens', pendingMessageTokens,
                        'totalTokensObserved', totalTokensObserved
                     ) FROM mastra_observational_memory ORDER BY id",
                )?;
                statement
                    .query_map([], |row| Ok(LegacyObservation { id: row.get(0)?, payload: row.get(1)? }))?
                    .collect::<Result<Vec<_>, rusqlite::Error>>()?
            };
            Ok::<_, rusqlite::Error>((working, messages, observations))
        })
        .await?)
}

fn convert_working(legacy: &LegacyWorkingMemory, warnings: &mut Vec<String>) -> WorkingMemory {
    WorkingMemory {
        goal: legacy.goal.as_ref().map(|goal| Goal {
            aim: goal.aim.clone(),
            done: goal.done.clone(),
            why: goal.why.clone(),
        }),
        plan: legacy
            .plan
            .iter()
            .map(|step| PlanStep {
                step_id: None,
                what: step.what.clone(),
                status: work_status(&step.status, warnings),
                note: step.note.clone(),
                tries: step.tries,
                done_when: None,
                evidence: Vec::new(),
                reevaluate_when: Vec::new(),
            })
            .collect(),
        todo: legacy
            .todo
            .iter()
            .map(|item| TodoItem {
                what: item.what.clone(),
                status: work_status(&item.status, warnings),
                note: item.note.clone(),
                asked_by: item.asked_by.clone(),
            })
            .collect(),
        notes: Vec::new(),
        plan_revision: 0,
        progress_summary: String::new(),
        reevaluate_when: Vec::new(),
        blocked_reason: None,
        goal_complete: false,
        strategic_intent: None,
    }
}

fn work_status(status: &str, warnings: &mut Vec<String>) -> WorkStatus {
    match status.to_ascii_lowercase().as_str() {
        "next" | "pending" => WorkStatus::Next,
        "doing" | "in_progress" | "active" => WorkStatus::Doing,
        "done" | "completed" => WorkStatus::Done,
        "blocked" => WorkStatus::Blocked,
        other => {
            warnings.push(format!(
                "unknown work status {other:?} was preserved in the archive and mapped to blocked"
            ));
            WorkStatus::Blocked
        }
    }
}

fn semantic_records(character_id: &str, legacy: &LegacyWorkingMemory) -> Vec<SemanticMemoryRecord> {
    let domains = [
        ("person", &legacy.people),
        ("place", &legacy.places),
        ("going_on", &legacy.goings_on),
        ("own_business", &legacy.own_business),
        ("opinion", &legacy.opinions),
        ("recent_memory", &legacy.lately),
        ("note", &legacy.notes),
    ];
    let mut records = Vec::new();
    for (kind, values) in domains {
        for (index, value) in values.iter().enumerate() {
            let summary = semantic_summary(value);
            let subject = semantic_subject(kind, value, index);
            let source_id = format!("working:{kind}:{index}");
            let stable_name = format!("{character_id}|{source_id}|{summary}");
            records.push(SemanticMemoryRecord {
                memory_id: Uuid::new_v5(&Uuid::NAMESPACE_OID, stable_name.as_bytes()),
                kind: kind.to_owned(),
                subject,
                summary,
                evidence: MemoryEvidence::MigratedUnknown,
                source: "mastra_working_memory".to_owned(),
                source_id: Some(source_id),
                occurred_at: None,
                recorded_at: Utc::now(),
            });
        }
    }
    records
}

fn semantic_summary(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return text.to_owned();
    }
    serde_json::to_string(value).unwrap_or_else(|_| "unreadable migrated value".to_owned())
}

fn semantic_subject(kind: &str, value: &Value, index: usize) -> String {
    for key in ["name", "where", "subject"] {
        if let Some(subject) = value.get(key).and_then(Value::as_str) {
            return subject.to_owned();
        }
    }
    format!("{kind}-{index}")
}

fn convert_message(message: &LegacyMessage) -> Option<Message> {
    let content: Value = serde_json::from_str(&message.content).ok()?;
    let mut texts = Vec::new();
    if let Some(text) = content.get("content").and_then(Value::as_str)
        && !text.trim().is_empty()
    {
        texts.push(text.trim().to_owned());
    }
    if let Some(parts) = content.get("parts").and_then(Value::as_array) {
        for part in parts {
            if part.get("type").and_then(Value::as_str) == Some("text")
                && let Some(text) = part.get("text").and_then(Value::as_str)
                && !text.trim().is_empty()
                && !texts.iter().any(|existing| existing == text.trim())
            {
                texts.push(text.trim().to_owned());
            }
        }
    }
    let text = texts.join("\n");
    if text.is_empty() {
        return None;
    }
    match message.role.as_str() {
        "user" => Some(Message::user(text)),
        "assistant" => Some(Message::assistant(text)),
        _ => None,
    }
}

async fn initialize_archive(connection: &Connection) -> anyhow::Result<()> {
    connection
        .call(|connection| {
            connection.execute_batch(
                "CREATE TABLE IF NOT EXISTS legacy_migration_archive (
                    character_id TEXT NOT NULL,
                    source_kind TEXT NOT NULL,
                    source_id TEXT NOT NULL,
                    payload TEXT NOT NULL,
                    archived_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    PRIMARY KEY(character_id, source_kind, source_id)
                 );
                 CREATE TABLE IF NOT EXISTS legacy_migration_runs (
                    character_id TEXT PRIMARY KEY NOT NULL,
                    source_bytes INTEGER NOT NULL,
                    report_json TEXT NOT NULL,
                    completed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                 );",
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await?;
    Ok(())
}

async fn refuse_completed_migration(
    connection: &Connection,
    character_id: &str,
) -> anyhow::Result<()> {
    let character_id = character_id.to_owned();
    let exists = connection
        .call(move |connection| {
            connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM legacy_migration_runs WHERE character_id = ?1)",
                [character_id],
                |row| row.get::<_, bool>(0),
            )
        })
        .await?;
    anyhow::ensure!(
        !exists,
        "a completed migration already exists for this character"
    );
    Ok(())
}

async fn archive_row(
    connection: &Connection,
    character_id: &str,
    source_kind: &str,
    source_id: &str,
    payload: &str,
) -> anyhow::Result<()> {
    let values = (
        character_id.to_owned(),
        source_kind.to_owned(),
        source_id.to_owned(),
        payload.to_owned(),
    );
    connection
        .call(move |connection| {
            connection.execute(
                "INSERT INTO legacy_migration_archive(character_id, source_kind, source_id, payload)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(character_id, source_kind, source_id) DO NOTHING",
                rusqlite::params![values.0, values.1, values.2, values.3],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await?;
    Ok(())
}

async fn complete_migration(
    connection: &Connection,
    character_id: &str,
    source_bytes: u64,
    report: &MigrationReport,
) -> anyhow::Result<()> {
    let character_id = character_id.to_owned();
    let source_bytes = i64::try_from(source_bytes).unwrap_or(i64::MAX);
    let report = serde_json::to_string(report)?;
    connection
        .call(move |connection| {
            connection.execute(
                "INSERT INTO legacy_migration_runs(character_id, source_bytes, report_json)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![character_id, source_bytes, report],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await?;
    Ok(())
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use rig_core::memory::ConversationMemory;

    use super::*;
    use crate::observability::RecordingAnalyticsSink;

    #[tokio::test]
    async fn migration_converts_supported_memory_and_archives_every_source_row() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let source_path = directory.path().join("legacy.sqlite");
        let destination_path = directory.path().join("rust.sqlite");
        create_source_fixture(&source_path).await;
        let analytics = Arc::new(RecordingAnalyticsSink::default());

        let report = migrate_mastra_memory(
            &MastraMigrationOptions {
                character_id: "orin",
                source_database: &source_path,
                destination_database: &destination_path,
                legacy_conversations_file: None,
                visited_file: None,
            },
            analytics.clone(),
        )
        .await
        .expect("migrate fixture");

        assert_eq!(report.source_messages_read, 3);
        assert_eq!(report.conversation_messages_written, 2);
        assert_eq!(report.unsupported_messages_archived, 1);
        assert_eq!(report.source_observations_archived, 1);
        assert_eq!(report.source_rows_archived, 5);
        assert_eq!(report.semantic_memories_written, 3);
        assert!(report.working_memory_written);
        let store = SqliteMemoryStore::open(&destination_path, analytics.clone())
            .await
            .expect("open migrated typed store");
        let working = store.load_working("orin").await.expect("load working");
        assert_eq!(working.goal.expect("goal").aim, "Map the north road");
        assert_eq!(working.plan[0].status, WorkStatus::Doing);
        assert_eq!(
            store
                .load_semantic("orin")
                .await
                .expect("semantic memory")
                .len(),
            3
        );
        let conversation = SqliteConversationMemory::open(&destination_path, "orin", analytics)
            .await
            .expect("open migrated conversation");
        assert_eq!(
            conversation
                .load("strategist")
                .await
                .expect("conversation")
                .len(),
            2
        );

        let rerun = migrate_mastra_memory(
            &MastraMigrationOptions {
                character_id: "orin",
                source_database: &source_path,
                destination_database: &destination_path,
                legacy_conversations_file: None,
                visited_file: None,
            },
            Arc::new(RecordingAnalyticsSink::default()),
        )
        .await
        .expect_err("completed migration must not duplicate messages");
        assert!(rerun.to_string().contains("completed migration"));
    }

    async fn create_source_fixture(path: &Path) {
        let connection = Connection::open(path).await.expect("source fixture");
        connection
            .call(|connection| {
                connection.execute_batch(
                    "CREATE TABLE mastra_resources (
                        id TEXT PRIMARY KEY, workingMemory TEXT, metadata TEXT,
                        createdAt TEXT, updatedAt TEXT
                     );
                     CREATE TABLE mastra_messages (
                        id TEXT PRIMARY KEY, thread_id TEXT, content TEXT, role TEXT,
                        type TEXT, createdAt TEXT, resourceId TEXT
                     );
                     CREATE TABLE mastra_observational_memory (
                        id TEXT PRIMARY KEY, lookupKey TEXT, scope TEXT, resourceId TEXT,
                        threadId TEXT, activeObservations TEXT,
                        activeObservationsPendingUpdate TEXT, originType TEXT, config TEXT,
                        generationCount INTEGER, lastObservedAt TEXT, lastReflectionAt TEXT,
                        pendingMessageTokens INTEGER, totalTokensObserved INTEGER
                     );",
                )?;
                let working = json!({
                    "goal": {"aim":"Map the north road","done":"Return with a map","why":"Travelers need it","setAt":"now"},
                    "plan": [{"what":"Walk north","status":"doing","note":null,"tries":1}],
                    "todo": [],
                    "people": [{"name":"Mira","about":"Runs the inn","feeling":"trusted","why":"helped","lastSeen":"today"}],
                    "places": [{"where":"Inn","what":"Safe room","how":"visited","who":"Mira","settled":true,"vouched":1,"doubted":0}],
                    "goingsOn": ["The road is busy"],
                    "ownBusiness": [], "opinions": [], "lately": [], "notes": []
                })
                .to_string();
                connection.execute(
                    "INSERT INTO mastra_resources VALUES (?1, ?2, NULL, ?3, ?3)",
                    rusqlite::params!["resource", working, "2026-01-01"],
                )?;
                let messages = [
                    ("m1", "user", json!({"format":2,"parts":[{"type":"text","text":"Mira asked for a map."}]}).to_string()),
                    ("m2", "assistant", json!({"format":2,"parts":[{"type":"text","text":"I will map the road."},{"type":"reasoning","reasoning":"private"}]}).to_string()),
                    ("m3", "assistant", json!({"format":2,"parts":[{"type":"tool-invocation","toolInvocation":{}}]}).to_string()),
                ];
                for (id, role, content) in messages {
                    connection.execute(
                        "INSERT INTO mastra_messages VALUES (?1, 'life', ?2, ?3, 'v2', ?4, 'resource')",
                        rusqlite::params![id, content, role, format!("2026-01-01T00:00:0{id}Z")],
                    )?;
                }
                connection.execute(
                    "INSERT INTO mastra_observational_memory VALUES (
                        'o1','life','resource','resource','life','raw observation',NULL,
                        'agent','{}',1,'2026-01-01',NULL,0,10
                     )",
                    [],
                )?;
                Ok::<_, rusqlite::Error>(())
            })
            .await
            .expect("populate source fixture");
    }
}
