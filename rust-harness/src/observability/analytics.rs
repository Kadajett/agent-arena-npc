use std::sync::{Arc, Mutex, OnceLock};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

/// One structured, append-only fact emitted by the harness.
///
/// Event names and common dimensions are stable. `attributes` carries
/// event-specific, non-secret dimensions without making the event pipeline
/// depend on every actor's domain types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalyticsEvent {
    pub process_run_id: Uuid,
    pub event_id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub name: String,
    pub level: EventLevel,
    pub character_id: Option<String>,
    pub correlation_id: Option<Uuid>,
    pub attributes: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl AnalyticsEvent {
    pub fn new(name: impl Into<String>, level: EventLevel) -> Self {
        Self {
            process_run_id: process_run_id(),
            event_id: Uuid::new_v4(),
            occurred_at: Utc::now(),
            name: name.into(),
            level,
            character_id: None,
            correlation_id: None,
            attributes: Map::new(),
        }
    }

    #[must_use]
    pub fn character(mut self, character_id: impl Into<String>) -> Self {
        self.character_id = Some(character_id.into());
        self
    }

    #[must_use]
    pub fn correlation(mut self, correlation_id: Uuid) -> Self {
        self.correlation_id = Some(correlation_id);
        self
    }

    #[must_use]
    pub fn attribute(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }
}

/// One stable identifier shared by every analytics event in this process.
#[must_use]
pub fn process_run_id() -> Uuid {
    static PROCESS_RUN_ID: OnceLock<Uuid> = OnceLock::new();
    *PROCESS_RUN_ID.get_or_init(Uuid::new_v4)
}

/// Append-only analytics boundary. Implementations must never block gameplay.
pub trait AnalyticsSink: Send + Sync {
    fn record(&self, event: AnalyticsEvent);
}

/// Production sink backed by the process's structured tracing subscriber.
#[derive(Debug, Default)]
pub struct TracingAnalyticsSink;

impl AnalyticsSink for TracingAnalyticsSink {
    fn record(&self, event: AnalyticsEvent) {
        let attributes = Value::Object(event.attributes).to_string();
        let character_id = event.character_id.as_deref().unwrap_or("");
        let correlation_id = event
            .correlation_id
            .map(|id| id.to_string())
            .unwrap_or_default();
        match event.level {
            EventLevel::Debug => tracing::debug!(
                target: "harness.analytics",
                process_run_id = %event.process_run_id,
                event_id = %event.event_id,
                event_name = %event.name,
                occurred_at = %event.occurred_at,
                character_id,
                correlation_id,
                attributes,
                "analytics_event"
            ),
            EventLevel::Info => tracing::info!(
                target: "harness.analytics",
                process_run_id = %event.process_run_id,
                event_id = %event.event_id,
                event_name = %event.name,
                occurred_at = %event.occurred_at,
                character_id,
                correlation_id,
                attributes,
                "analytics_event"
            ),
            EventLevel::Warn => tracing::warn!(
                target: "harness.analytics",
                process_run_id = %event.process_run_id,
                event_id = %event.event_id,
                event_name = %event.name,
                occurred_at = %event.occurred_at,
                character_id,
                correlation_id,
                attributes,
                "analytics_event"
            ),
            EventLevel::Error => tracing::error!(
                target: "harness.analytics",
                process_run_id = %event.process_run_id,
                event_id = %event.event_id,
                event_name = %event.name,
                occurred_at = %event.occurred_at,
                character_id,
                correlation_id,
                attributes,
                "analytics_event"
            ),
        }
    }
}

/// In-memory sink used by contract and causality tests.
#[derive(Debug, Default)]
pub struct RecordingAnalyticsSink {
    events: Mutex<Vec<AnalyticsEvent>>,
}

impl RecordingAnalyticsSink {
    pub fn events(&self) -> Vec<AnalyticsEvent> {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl AnalyticsSink for RecordingAnalyticsSink {
    fn record(&self, event: AnalyticsEvent) {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
    }
}

pub fn tracing_sink() -> Arc<dyn AnalyticsSink> {
    Arc::new(TracingAnalyticsSink)
}

/// Replace configured secrets before an external error reaches logs or events.
pub fn redact(input: &str, secrets: &[&str]) -> String {
    secrets
        .iter()
        .filter(|secret| !secret.is_empty())
        .fold(input.to_owned(), |safe, secret| {
            safe.replace(secret, "[REDACTED]")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_every_occurrence_of_each_secret() {
        assert_eq!(
            redact("token-a then token-b and token-a", &["token-a", "token-b"]),
            "[REDACTED] then [REDACTED] and [REDACTED]"
        );
    }

    #[test]
    fn every_event_in_one_process_has_the_same_run_id() {
        let first = AnalyticsEvent::new("first", EventLevel::Info);
        let second = AnalyticsEvent::new("second", EventLevel::Debug);

        assert_eq!(first.process_run_id, second.process_run_id);
        assert_eq!(first.process_run_id, process_run_id());
        assert_ne!(first.event_id, second.event_id);
    }
}
