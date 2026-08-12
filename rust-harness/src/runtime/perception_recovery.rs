use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::{
    character::CharacterSheet,
    mcp::{
        ArenaGateway,
        client::GatewayError,
        observation::Observation,
        session::ArenaSession,
        types::{InventoryResult, MapObservation},
    },
    observability::{AnalyticsEvent, AnalyticsSink, EventLevel},
    runtime::perception_pump::PerceptionSource,
};

/// Read-only production source that repairs a lost MCP HTTP session once and
/// retries the failed perception operation.
///
/// Concurrent observation, map, and inventory reads share one recovery lock.
/// The first failed read reconnects; the others observe the newer generation
/// and retry without starting duplicate login sequences.
pub struct ReconnectingPerceptionSource {
    gateway: ArenaGateway,
    session: Arc<ArenaSession>,
    character: Arc<CharacterSheet>,
    max_attempts: u32,
    initial_backoff: Duration,
    recovery: Mutex<()>,
    analytics: Arc<dyn AnalyticsSink>,
}

impl ReconnectingPerceptionSource {
    #[must_use]
    pub fn new(
        gateway: ArenaGateway,
        session: Arc<ArenaSession>,
        character: Arc<CharacterSheet>,
        max_attempts: u32,
        initial_backoff: Duration,
        analytics: Arc<dyn AnalyticsSink>,
    ) -> Self {
        Self {
            gateway,
            session,
            character,
            max_attempts,
            initial_backoff,
            recovery: Mutex::new(()),
            analytics,
        }
    }

    async fn recover_if_session_lost(
        &self,
        operation: &'static str,
        observed_generation: u64,
        error: &GatewayError,
    ) -> bool {
        let GatewayError::Mcp(error) = error else {
            return false;
        };
        if !error.is_session_loss() {
            return false;
        }
        self.analytics.record(
            AnalyticsEvent::new("mcp.session_loss_detected", EventLevel::Warn)
                .character(&self.character.id)
                .attribute("operation", operation)
                .attribute("observed_generation", observed_generation)
                .attribute("error_class", error.class()),
        );
        let _guard = self.recovery.lock().await;
        let current_generation = self.session.generation().await;
        if current_generation > observed_generation {
            self.analytics.record(
                AnalyticsEvent::new("mcp.session_recovery_joined", EventLevel::Info)
                    .character(&self.character.id)
                    .attribute("operation", operation)
                    .attribute("observed_generation", observed_generation)
                    .attribute("current_generation", current_generation),
            );
            return true;
        }
        if let Ok(connected) = self
            .session
            .reconnect(&self.character, self.max_attempts, self.initial_backoff)
            .await
        {
            self.analytics.record(
                AnalyticsEvent::new("mcp.session_recovery_completed", EventLevel::Info)
                    .character(&self.character.id)
                    .attribute("operation", operation)
                    .attribute("previous_generation", observed_generation)
                    .attribute("generation", connected.generation),
            );
            true
        } else {
            self.analytics.record(
                AnalyticsEvent::new("mcp.session_recovery_exhausted", EventLevel::Error)
                    .character(&self.character.id)
                    .attribute("operation", operation)
                    .attribute("generation", observed_generation)
                    .attribute("error_class", "reconnect_exhausted"),
            );
            false
        }
    }
}

#[async_trait]
impl PerceptionSource for ReconnectingPerceptionSource {
    async fn observe(&self) -> Result<Observation, GatewayError> {
        let generation = self.session.generation().await;
        match self.gateway.observe().await {
            Ok(observation) => Ok(observation),
            Err(error)
                if self
                    .recover_if_session_lost("observe", generation, &error)
                    .await =>
            {
                self.gateway.observe().await
            }
            Err(error) => Err(error),
        }
    }

    async fn render_map(&self, radius: u32) -> Result<MapObservation, GatewayError> {
        let generation = self.session.generation().await;
        match self.gateway.render_map(radius).await {
            Ok(map) => Ok(map),
            Err(error)
                if self
                    .recover_if_session_lost("render_map", generation, &error)
                    .await =>
            {
                self.gateway.render_map(radius).await
            }
            Err(error) => Err(error),
        }
    }

    async fn inventory(&self) -> Result<InventoryResult, GatewayError> {
        let generation = self.session.generation().await;
        match self.gateway.inventory().await {
            Ok(inventory) => Ok(inventory),
            Err(error)
                if self
                    .recover_if_session_lost("inventory", generation, &error)
                    .await =>
            {
                self.gateway.inventory().await
            }
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, VecDeque},
        sync::Mutex as StdMutex,
    };

    use serde_json::{Value, json};
    use tokio::sync::RwLock;
    use uuid::Uuid;

    use super::*;
    use crate::{
        HarnessConfig,
        mcp::{
            session::SessionEvent,
            transport::{McpError, McpTransport},
        },
        observability::RecordingAnalyticsSink,
    };

    #[derive(Default)]
    struct ScriptedTransport {
        responses: StdMutex<VecDeque<Result<Value, McpError>>>,
        session: RwLock<Option<String>>,
    }

    #[async_trait]
    impl McpTransport for ScriptedTransport {
        async fn request(
            &self,
            _method: &str,
            _params: Value,
            _correlation_id: Uuid,
        ) -> Result<Value, McpError> {
            self.responses
                .lock()
                .expect("responses")
                .pop_front()
                .expect("scripted response")
        }

        async fn notify(
            &self,
            _method: &str,
            _params: Value,
            _correlation_id: Uuid,
        ) -> Result<(), McpError> {
            Ok(())
        }

        async fn reset_session(&self) {
            *self.session.write().await = None;
        }

        async fn session_id(&self) -> Option<String> {
            self.session.read().await.clone()
        }
    }

    fn tool(body: &Value) -> Value {
        json!({"content": [{"type": "text", "text": body.to_string()}]})
    }

    fn character() -> CharacterSheet {
        let values = HashMap::from([
            ("ARENA_API_KEY", "arena"),
            (
                "NPC_CHARACTER_SHEET_PATH",
                concat!(env!("CARGO_MANIFEST_DIR"), "/characters/guy.json"),
            ),
        ]);
        HarnessConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
            .expect("config")
            .character_sheet()
            .expect("character")
    }

    #[tokio::test]
    async fn one_failed_read_reconnects_invalid_http_session_and_retries() {
        let transport = Arc::new(ScriptedTransport::default());
        let initialize = || {
            Ok(json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "serverInfo": {"name": "arena", "version": "1"}
            }))
        };
        let listed = || {
            Ok(tool(
                &json!({"agents": [{"id": "agent-guy", "playerName": "Guy"}]}),
            ))
        };
        let logged_in = || Ok(tool(&json!({"loggedIn": true})));
        transport.responses.lock().expect("responses").extend([
            initialize(),
            listed(),
            logged_in(),
            Err(McpError::HttpStatus {
                status: 400,
                message: "Invalid or missing MCP session ID.".to_owned(),
            }),
            initialize(),
            listed(),
            logged_in(),
            Ok(tool(&json!({}))),
        ]);
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let session = Arc::new(ArenaSession::new(transport, analytics.clone()));
        let character = Arc::new(character());
        let mut events = session.subscribe();
        let connected = session.connect(&character).await.expect("initial connect");
        let source = ReconnectingPerceptionSource::new(
            connected.gateway,
            session.clone(),
            character,
            1,
            Duration::ZERO,
            analytics.clone(),
        );

        source
            .observe()
            .await
            .expect("read retries after reconnect");

        assert_eq!(session.generation().await, 2);
        assert!(matches!(
            events.recv().await.expect("connected"),
            SessionEvent::Connected { generation: 1, .. }
        ));
        assert!(matches!(
            events.recv().await.expect("reconnected"),
            SessionEvent::Reconnected { generation: 2, .. }
        ));
        assert!(matches!(
            events.recv().await.expect("invalidated"),
            SessionEvent::DecisionsInvalidated { generation: 2, .. }
        ));
        let names = analytics
            .events()
            .into_iter()
            .map(|event| event.name)
            .collect::<Vec<_>>();
        assert!(names.contains(&"mcp.session_loss_detected".to_owned()));
        assert!(names.contains(&"mcp.session_recovery_completed".to_owned()));
    }
}
