use std::{path::Path, sync::Arc, time::Instant};

use rig_core::{
    completion::Message,
    memory::{ConversationMemory, MemoryError},
    wasm_compat::WasmBoxedFuture,
};
use rig_memory::{HeuristicTokenCounter, PolicyMemory, TokenWindowMemory};
use tokio_rusqlite::{Connection, rusqlite};

use crate::observability::{AnalyticsEvent, AnalyticsSink, EventLevel};

const SCHEMA_VERSION: i64 = 1;

/// A durable local adapter for Rig-managed strategist conversations.
///
/// The adapter stores complete Rig messages. This includes tool calls and tool
/// results when the strategist gains tools. Loaded history can be shaped with
/// `rig-memory` without changing the durable record.
#[derive(Clone)]
pub struct SqliteConversationMemory {
    connection: Connection,
    character_id: String,
    analytics: Arc<dyn AnalyticsSink>,
}

impl SqliteConversationMemory {
    /// Open or create a local `SQLite` conversation database.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot open or initialize the database.
    pub async fn open(
        path: impl AsRef<Path>,
        character_id: impl Into<String>,
        analytics: Arc<dyn AnalyticsSink>,
    ) -> anyhow::Result<Self> {
        let connection = Connection::open(path).await?;
        Self::initialize(connection, character_id.into(), analytics).await
    }

    /// Open an isolated in-memory `SQLite` database for tests.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot initialize the database.
    #[cfg(test)]
    pub async fn open_in_memory(
        character_id: impl Into<String>,
        analytics: Arc<dyn AnalyticsSink>,
    ) -> anyhow::Result<Self> {
        let connection = Connection::open_in_memory().await?;
        Self::initialize(connection, character_id.into(), analytics).await
    }

    async fn initialize(
        connection: Connection,
        character_id: String,
        analytics: Arc<dyn AnalyticsSink>,
    ) -> anyhow::Result<Self> {
        connection
            .call(|connection| {
                connection.busy_timeout(std::time::Duration::from_secs(5))?;
                connection.execute_batch(
                    "PRAGMA foreign_keys = ON;
                     CREATE TABLE IF NOT EXISTS memory_schema (
                         name TEXT PRIMARY KEY NOT NULL,
                         version INTEGER NOT NULL
                     );
                     CREATE TABLE IF NOT EXISTS conversation_messages (
                         message_id INTEGER PRIMARY KEY AUTOINCREMENT,
                         conversation_id TEXT NOT NULL,
                         message_json TEXT NOT NULL,
                         created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                     );
                     CREATE INDEX IF NOT EXISTS conversation_messages_lookup
                         ON conversation_messages(conversation_id, message_id);",
                )?;
                connection.execute(
                    "INSERT INTO memory_schema(name, version) VALUES ('conversation', ?1)
                     ON CONFLICT(name) DO UPDATE SET version = excluded.version
                     WHERE memory_schema.version = excluded.version",
                    [SCHEMA_VERSION],
                )?;
                let version: i64 = connection.query_row(
                    "SELECT version FROM memory_schema WHERE name = 'conversation'",
                    [],
                    |row| row.get(0),
                )?;
                if version != SCHEMA_VERSION {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                Ok(())
            })
            .await?;
        Ok(Self {
            connection,
            character_id,
            analytics,
        })
    }

    fn record_started(&self, operation: &'static str) {
        self.analytics.record(
            AnalyticsEvent::new("memory.conversation_operation_started", EventLevel::Debug)
                .character(&self.character_id)
                .attribute("operation", operation),
        );
    }

    fn record_completed(
        &self,
        operation: &'static str,
        started: Instant,
        message_count: usize,
        serialized_bytes: usize,
    ) {
        self.analytics.record(
            AnalyticsEvent::new("memory.conversation_operation_completed", EventLevel::Debug)
                .character(&self.character_id)
                .attribute("operation", operation)
                .attribute("duration_ms", elapsed_ms(started))
                .attribute("message_count", usize_to_u64(message_count))
                .attribute("serialized_bytes", usize_to_u64(serialized_bytes)),
        );
    }

    fn record_failed(&self, operation: &'static str, started: Instant, error_class: &'static str) {
        self.analytics.record(
            AnalyticsEvent::new("memory.conversation_operation_failed", EventLevel::Warn)
                .character(&self.character_id)
                .attribute("operation", operation)
                .attribute("duration_ms", elapsed_ms(started))
                .attribute("error_class", error_class),
        );
    }
}

/// Apply Rig's token-window policy to a durable conversation adapter.
///
/// The `SQLite` record stays complete. Rig receives only the newest messages
/// that fit within `max_tokens`.
pub fn bounded_conversation_memory(
    memory: SqliteConversationMemory,
    max_tokens: usize,
) -> PolicyMemory<SqliteConversationMemory, TokenWindowMemory> {
    PolicyMemory::new(
        memory,
        TokenWindowMemory::new(max_tokens, HeuristicTokenCounter::openai()),
    )
}

impl ConversationMemory for SqliteConversationMemory {
    fn load<'a>(
        &'a self,
        conversation_id: &'a str,
    ) -> WasmBoxedFuture<'a, Result<Vec<Message>, MemoryError>> {
        Box::pin(async move {
            const OPERATION: &str = "load";
            let started = Instant::now();
            self.record_started(OPERATION);
            let conversation_id = conversation_id.to_owned();
            let rows = match self
                .connection
                .call(move |connection| {
                    let mut statement = connection.prepare(
                        "SELECT message_json FROM conversation_messages
                         WHERE conversation_id = ?1 ORDER BY message_id ASC",
                    )?;
                    statement
                        .query_map([conversation_id], |row| row.get::<_, String>(0))?
                        .collect::<Result<Vec<_>, rusqlite::Error>>()
                })
                .await
            {
                Ok(rows) => rows,
                Err(error) => {
                    self.record_failed(OPERATION, started, "sqlite");
                    return Err(MemoryError::backend(error));
                }
            };
            let serialized_bytes = rows.iter().map(String::len).sum();
            let messages = match rows
                .iter()
                .map(|row| serde_json::from_str(row))
                .collect::<Result<Vec<Message>, serde_json::Error>>()
            {
                Ok(messages) => messages,
                Err(error) => {
                    self.record_failed(OPERATION, started, "deserialize");
                    return Err(MemoryError::backend(error));
                }
            };
            self.record_completed(OPERATION, started, messages.len(), serialized_bytes);
            Ok(messages)
        })
    }

    fn append<'a>(
        &'a self,
        conversation_id: &'a str,
        messages: Vec<Message>,
    ) -> WasmBoxedFuture<'a, Result<(), MemoryError>> {
        Box::pin(async move {
            const OPERATION: &str = "append";
            let started = Instant::now();
            self.record_started(OPERATION);
            let message_count = messages.len();
            let rows = match messages
                .iter()
                .map(serde_json::to_string)
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(rows) => rows,
                Err(error) => {
                    self.record_failed(OPERATION, started, "serialize");
                    return Err(MemoryError::backend(error));
                }
            };
            let serialized_bytes = rows.iter().map(String::len).sum();
            let conversation_id = conversation_id.to_owned();
            let result = self
                .connection
                .call(move |connection| {
                    let transaction = connection.transaction()?;
                    {
                        let mut statement = transaction.prepare(
                            "INSERT INTO conversation_messages(conversation_id, message_json)
                             VALUES (?1, ?2)",
                        )?;
                        for row in rows {
                            statement.execute(rusqlite::params![conversation_id, row])?;
                        }
                    }
                    transaction.commit()
                })
                .await;
            if let Err(error) = result {
                self.record_failed(OPERATION, started, "sqlite");
                return Err(MemoryError::backend(error));
            }
            self.record_completed(OPERATION, started, message_count, serialized_bytes);
            Ok(())
        })
    }

    fn clear<'a>(
        &'a self,
        conversation_id: &'a str,
    ) -> WasmBoxedFuture<'a, Result<(), MemoryError>> {
        Box::pin(async move {
            const OPERATION: &str = "clear";
            let started = Instant::now();
            self.record_started(OPERATION);
            let conversation_id = conversation_id.to_owned();
            let result = self
                .connection
                .call(move |connection| {
                    connection.execute(
                        "DELETE FROM conversation_messages WHERE conversation_id = ?1",
                        [conversation_id],
                    )
                })
                .await;
            match result {
                Ok(deleted) => {
                    self.record_completed(OPERATION, started, deleted, 0);
                    Ok(())
                }
                Err(error) => {
                    self.record_failed(OPERATION, started, "sqlite");
                    Err(MemoryError::backend(error))
                }
            }
        })
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use rig_core::{
        OneOrMany,
        memory::ConversationMemory,
        message::{
            AssistantContent, ToolCall, ToolFunction, ToolResult, ToolResultContent, UserContent,
        },
    };

    use super::*;
    use crate::observability::RecordingAnalyticsSink;

    #[tokio::test]
    async fn persists_rig_messages_across_database_reopen() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("memory.sqlite3");
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let first = SqliteConversationMemory::open(&path, "cassian", analytics.clone())
            .await
            .expect("open first adapter");
        first
            .append(
                "strategist:cassian:main",
                vec![Message::user("A private fact"), Message::assistant("Noted")],
            )
            .await
            .expect("append conversation");
        drop(first);

        let reopened = SqliteConversationMemory::open(&path, "cassian", analytics)
            .await
            .expect("reopen adapter");
        let loaded = reopened
            .load("strategist:cassian:main")
            .await
            .expect("load conversation");

        assert_eq!(loaded.len(), 2);
        assert!(matches!(
            &loaded[0],
            Message::User { content }
                if matches!(content.first_ref(), UserContent::Text(text) if text.text == "A private fact")
        ));
        assert!(matches!(
            &loaded[1],
            Message::Assistant { content, .. }
                if matches!(content.first_ref(), AssistantContent::Text(text) if text.text == "Noted")
        ));
    }

    #[tokio::test]
    async fn rig_token_policy_bounds_loaded_history_without_deleting_it() {
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let memory = SqliteConversationMemory::open_in_memory("cassian", analytics)
            .await
            .expect("open adapter");
        memory
            .append(
                "strategist:cassian:main",
                vec![
                    Message::user("old ".repeat(200)),
                    Message::assistant("old reply ".repeat(200)),
                    Message::user("recent question"),
                    Message::assistant("recent answer"),
                ],
            )
            .await
            .expect("append conversation");

        let bounded = bounded_conversation_memory(memory.clone(), 32);
        let visible = bounded
            .load("strategist:cassian:main")
            .await
            .expect("load bounded history");
        let durable = memory
            .load("strategist:cassian:main")
            .await
            .expect("load durable history");

        assert_eq!(visible.len(), 2);
        assert_eq!(durable.len(), 4);
    }

    #[tokio::test]
    async fn clear_removes_only_the_selected_conversation() {
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let memory = SqliteConversationMemory::open_in_memory("cassian", analytics)
            .await
            .expect("open adapter");
        memory
            .append("one", vec![Message::user("first")])
            .await
            .expect("append first");
        memory
            .append("two", vec![Message::user("second")])
            .await
            .expect("append second");

        memory.clear("one").await.expect("clear first");

        assert!(memory.load("one").await.expect("load first").is_empty());
        assert_eq!(memory.load("two").await.expect("load second").len(), 1);
    }

    #[tokio::test]
    async fn configured_policy_preserves_a_tool_call_and_its_result() {
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let memory = SqliteConversationMemory::open_in_memory("cassian", analytics)
            .await
            .expect("open adapter");
        let tool_call = Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::ToolCall(ToolCall::new(
                "call-1".to_owned(),
                ToolFunction::new("recall".to_owned(), serde_json::json!({"kind": "place"})),
            ))),
        };
        let tool_result = Message::User {
            content: OneOrMany::one(UserContent::ToolResult(ToolResult {
                id: "call-1".to_owned(),
                call_id: None,
                content: OneOrMany::one(ToolResultContent::text("one place")),
            })),
        };
        memory
            .append(
                "strategist:cassian:main",
                vec![
                    Message::user("old ".repeat(500)),
                    tool_call,
                    tool_result,
                    Message::assistant("I remember the place."),
                ],
            )
            .await
            .expect("append conversation");

        let visible = bounded_conversation_memory(memory, 64)
            .load("strategist:cassian:main")
            .await
            .expect("load bounded history");

        assert_eq!(visible.len(), 3);
        assert!(matches!(
            &visible[0],
            Message::Assistant { content, .. }
                if matches!(content.first_ref(), AssistantContent::ToolCall(call) if call.id == "call-1")
        ));
        assert!(matches!(
            &visible[1],
            Message::User { content }
                if matches!(content.first_ref(), UserContent::ToolResult(result) if result.id == "call-1")
        ));
    }

    #[tokio::test]
    async fn analytics_records_counts_but_not_conversation_content_or_id() {
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let memory = SqliteConversationMemory::open_in_memory("cassian", analytics.clone())
            .await
            .expect("open adapter");
        memory
            .append(
                "strategist:cassian:private-thread",
                vec![Message::user("secret phrase")],
            )
            .await
            .expect("append conversation");

        let events = analytics.events();
        let completed = events
            .iter()
            .find(|event| {
                event.name == "memory.conversation_operation_completed"
                    && event.attributes["operation"] == "append"
            })
            .expect("append completion event");
        let encoded = serde_json::to_string(&events).expect("encode events");

        assert_eq!(completed.character_id.as_deref(), Some("cassian"));
        assert_eq!(completed.attributes["message_count"], 1);
        assert!(!encoded.contains("secret phrase"));
        assert!(!encoded.contains("private-thread"));
    }
}
