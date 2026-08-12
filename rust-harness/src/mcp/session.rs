use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use thiserror::Error;
use tokio::sync::{RwLock, broadcast};
use uuid::Uuid;

use crate::{
    character::CharacterSheet,
    mcp::{
        client::ArenaGateway,
        protocol::{ImplementationInfo, InitializeResult, PROTOCOL_VERSION, ToolListResult},
        transport::{McpError, McpTransport},
        types::{
            AgentList, DisconnectResult, LoginResult, RegisteredAgent, RegistrationResult,
            WatchCodeResult,
        },
    },
    observability::{AnalyticsEvent, AnalyticsSink, EventLevel},
};

const CLIENT_NAME: &str = "agent-arena-npc-rust-harness";
const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    Connected { generation: u64, agent_id: String },
    Disconnected { generation: u64 },
    Reconnected { generation: u64, agent_id: String },
    DecisionsInvalidated { generation: u64, reason: String },
}

#[derive(Debug, Clone, Default)]
struct SessionState {
    generation: u64,
    agent: Option<RegisteredAgent>,
    analytics_character_id: Option<String>,
    connected: bool,
}

pub struct ArenaSession {
    transport: Arc<dyn McpTransport>,
    analytics: Arc<dyn AnalyticsSink>,
    state: RwLock<SessionState>,
    events: broadcast::Sender<SessionEvent>,
}

pub struct ConnectedArena {
    pub gateway: ArenaGateway,
    pub agent: RegisteredAgent,
    pub generation: u64,
}

#[derive(Debug, Error)]
pub enum SessionError {
    #[error(transparent)]
    Mcp(#[from] McpError),
    #[error("session response for {operation} was incompatible: {source}")]
    Decode {
        operation: &'static str,
        source: serde_json::Error,
    },
    #[error("registration returned no usable agent identity")]
    MissingAgentIdentity,
    #[error("MCP reconnect failed after {attempts} attempts: {last_error}")]
    ReconnectExhausted { attempts: u32, last_error: String },
}

impl ArenaSession {
    pub fn new(transport: Arc<dyn McpTransport>, analytics: Arc<dyn AnalyticsSink>) -> Self {
        let (events, _) = broadcast::channel(32);
        Self {
            transport,
            analytics,
            state: RwLock::new(SessionState::default()),
            events,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    pub async fn generation(&self) -> u64 {
        self.state.read().await.generation
    }

    /// Initialize MCP, register or locate the character, and log its body in.
    ///
    /// # Errors
    ///
    /// Returns an error for protocol, registration, login, or decode failures.
    pub async fn connect(
        &self,
        character: &CharacterSheet,
    ) -> Result<ConnectedArena, SessionError> {
        match self.establish(character, false).await {
            Ok(connected) => Ok(connected),
            Err(error) => {
                self.analytics.record(
                    AnalyticsEvent::new("mcp.session_connect_failed", EventLevel::Error)
                        .character(&character.id)
                        .attribute("error_class", session_error_class(&error)),
                );
                Err(error)
            }
        }
    }

    /// Rebuild a lost MCP session with bounded exponential backoff.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::ReconnectExhausted`] after all attempts fail.
    pub async fn reconnect(
        &self,
        character: &CharacterSheet,
        max_attempts: u32,
        initial_backoff: Duration,
    ) -> Result<ConnectedArena, SessionError> {
        let attempts = max_attempts.max(1);
        let mut delay = initial_backoff;
        let mut last_error = String::new();
        for attempt in 1..=attempts {
            self.analytics.record(
                AnalyticsEvent::new("mcp.session_reconnect_attempted", EventLevel::Warn)
                    .character(&character.id)
                    .attribute("attempt", attempt),
            );
            match self.establish(character, true).await {
                Ok(connected) => return Ok(connected),
                Err(error) => {
                    last_error = error.to_string();
                    self.analytics.record(
                        AnalyticsEvent::new("mcp.session_reconnect_failed", EventLevel::Warn)
                            .character(&character.id)
                            .attribute("attempt", attempt)
                            .attribute("error", last_error.clone()),
                    );
                    if attempt < attempts {
                        tokio::time::sleep(delay).await;
                        delay = delay.saturating_mul(2).min(Duration::from_secs(8));
                    }
                }
            }
        }
        Err(SessionError::ReconnectExhausted {
            attempts,
            last_error,
        })
    }

    /// Disconnect the currently bound character and clear local session state.
    ///
    /// # Errors
    ///
    /// Returns an error if the server refuses or cannot receive the disconnect.
    pub async fn disconnect(&self) -> Result<Option<DisconnectResult>, SessionError> {
        let (agent_id, generation, analytics_character_id) = {
            let state = self.state.read().await;
            (
                state.agent.as_ref().map(|agent| agent.id.clone()),
                state.generation,
                state.analytics_character_id.clone(),
            )
        };
        let Some(agent_id) = agent_id else {
            self.transport.reset_session().await;
            return Ok(None);
        };
        let result = match self
            .session_tool(
                "arena_disconnect",
                json!({ "agent_id": agent_id }),
                Uuid::new_v4(),
            )
            .await
        {
            Ok(result) => Some(result),
            Err(SessionError::Mcp(error)) if error.is_session_loss() => {
                self.analytics.record(with_character(
                    AnalyticsEvent::new("mcp.session_disconnect_already_lost", EventLevel::Warn)
                        .attribute("generation", generation)
                        .attribute("error_class", error.class()),
                    analytics_character_id.as_deref(),
                ));
                None
            }
            Err(error) => return Err(error),
        };
        {
            let mut state = self.state.write().await;
            state.connected = false;
        }
        self.transport.reset_session().await;
        let _ = self.events.send(SessionEvent::Disconnected { generation });
        self.analytics.record(with_character(
            AnalyticsEvent::new("mcp.session_disconnected", EventLevel::Info)
                .attribute("generation", generation),
            analytics_character_id.as_deref(),
        ));
        Ok(result)
    }

    /// Create a viewer code for the bound body.
    ///
    /// # Errors
    ///
    /// Returns an error when no agent is bound or the MCP operation fails.
    pub async fn create_watch_code(&self) -> Result<WatchCodeResult, SessionError> {
        let agent_id = self
            .state
            .read()
            .await
            .agent
            .as_ref()
            .map(|agent| agent.id.clone())
            .ok_or(SessionError::MissingAgentIdentity)?;
        self.session_tool(
            "arena_create_watch_code",
            json!({ "agent_id": agent_id }),
            Uuid::new_v4(),
        )
        .await
    }

    /// List every tool advertised by the connected MCP server.
    ///
    /// This method follows MCP pagination so compatibility diagnostics can detect
    /// new production commands before model or actor code tries to use them.
    ///
    /// # Errors
    ///
    /// Returns an error when the protocol request fails or its result cannot decode.
    pub async fn list_tool_names(&self) -> Result<Vec<String>, SessionError> {
        let correlation_id = Uuid::new_v4();
        let mut cursor: Option<String> = None;
        let mut names = Vec::new();
        loop {
            let params = cursor
                .as_ref()
                .map_or_else(|| json!({}), |cursor| json!({ "cursor": cursor }));
            let page: ToolListResult = self
                .request_typed("tools/list", params, correlation_id)
                .await?;
            names.extend(page.tools.into_iter().map(|tool| tool.name));
            let Some(next_cursor) = page.next_cursor.filter(|value| !value.is_empty()) else {
                break;
            };
            cursor = Some(next_cursor);
        }
        names.sort_unstable();
        names.dedup();
        Ok(names)
    }

    async fn establish(
        &self,
        character: &CharacterSheet,
        reconnect: bool,
    ) -> Result<ConnectedArena, SessionError> {
        let started = Instant::now();
        let correlation_id = Uuid::new_v4();
        self.transport.reset_session().await;
        self.state.write().await.analytics_character_id = Some(character.id.clone());
        self.analytics.record(
            AnalyticsEvent::new("mcp.session_connecting", EventLevel::Info)
                .character(&character.id)
                .correlation(correlation_id)
                .attribute("reconnect", reconnect),
        );
        let initialized = self.initialize(correlation_id).await?;
        self.transport
            .set_protocol_version(&initialized.protocol_version)
            .await;
        self.transport
            .notify("notifications/initialized", json!({}), correlation_id)
            .await?;
        let agent = self.find_or_register(character, correlation_id).await?;
        if agent.id.trim().is_empty() {
            return Err(SessionError::MissingAgentIdentity);
        }
        let _: LoginResult = self
            .session_tool(
                "arena_login",
                json!({ "agent_id": agent.id }),
                correlation_id,
            )
            .await?;

        Ok(self
            .finish_establish(
                character,
                agent,
                reconnect,
                correlation_id,
                started,
                &initialized,
            )
            .await)
    }

    async fn initialize(&self, correlation_id: Uuid) -> Result<InitializeResult, SessionError> {
        self.request_typed(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": ImplementationInfo {
                    name: CLIENT_NAME.to_owned(),
                    version: CLIENT_VERSION.to_owned(),
                }
            }),
            correlation_id,
        )
        .await
    }

    async fn find_or_register(
        &self,
        character: &CharacterSheet,
        correlation_id: Uuid,
    ) -> Result<RegisteredAgent, SessionError> {
        let agents: AgentList = self
            .session_tool("arena_list_agents", json!({}), correlation_id)
            .await?;
        if let Some(agent) = agents
            .agents
            .into_iter()
            .find(|agent| agent.player_name == character.player_name)
        {
            return Ok(agent);
        }
        let registered: RegistrationResult = self
            .session_tool(
                "arena_register_agent",
                json!({
                    "agent_name": character.id,
                    "player_name": character.player_name,
                    "class_path": character.class_path.as_deref().unwrap_or("journeyman"),
                    "selected_scene": character.home_scene,
                    "idempotency_key": format!(
                        "npc-{}-v{}",
                        character.id,
                        character.registration_version
                    ),
                }),
                correlation_id,
            )
            .await?;
        Ok(registered.agent)
    }

    async fn finish_establish(
        &self,
        character: &CharacterSheet,
        agent: RegisteredAgent,
        reconnect: bool,
        correlation_id: Uuid,
        started: Instant,
        initialized: &InitializeResult,
    ) -> ConnectedArena {
        let generation = {
            let mut state = self.state.write().await;
            state.generation += 1;
            state.agent = Some(agent.clone());
            state.analytics_character_id = Some(character.id.clone());
            state.connected = true;
            state.generation
        };
        let gateway = ArenaGateway::for_character(
            self.transport.clone(),
            &agent.id,
            &character.id,
            character.capabilities.clone(),
            self.analytics.clone(),
        );
        let event = if reconnect {
            SessionEvent::Reconnected {
                generation,
                agent_id: agent.id.clone(),
            }
        } else {
            SessionEvent::Connected {
                generation,
                agent_id: agent.id.clone(),
            }
        };
        let _ = self.events.send(event);
        if reconnect {
            let _ = self.events.send(SessionEvent::DecisionsInvalidated {
                generation,
                reason: "MCP session reconnected".to_owned(),
            });
            self.analytics.record(
                AnalyticsEvent::new("runtime.decisions_invalidated", EventLevel::Warn)
                    .character(&character.id)
                    .correlation(correlation_id)
                    .attribute("generation", generation)
                    .attribute("reason", "mcp_session_reconnected"),
            );
        }
        self.analytics.record(
            AnalyticsEvent::new(
                if reconnect {
                    "mcp.session_reconnected"
                } else {
                    "mcp.session_connected"
                },
                EventLevel::Info,
            )
            .character(&character.id)
            .correlation(correlation_id)
            .attribute("generation", generation)
            .attribute("duration_ms", elapsed_ms(started))
            .attribute(
                "server_name",
                initialized
                    .server_info
                    .as_ref()
                    .map(|info| info.name.clone())
                    .unwrap_or_default(),
            )
            .attribute("protocol_version", initialized.protocol_version.clone()),
        );
        ConnectedArena {
            gateway,
            agent,
            generation,
        }
    }

    async fn session_tool<T: DeserializeOwned>(
        &self,
        tool: &'static str,
        arguments: Value,
        correlation_id: Uuid,
    ) -> Result<T, SessionError> {
        let started = Instant::now();
        let analytics_character_id = self.state.read().await.analytics_character_id.clone();
        self.analytics.record(with_character(
            AnalyticsEvent::new("mcp.session_tool_started", EventLevel::Debug)
                .correlation(correlation_id)
                .attribute("tool", tool),
            analytics_character_id.as_deref(),
        ));
        let result = match self
            .transport
            .call_tool(tool, arguments, correlation_id)
            .await
        {
            Ok(result) => result,
            Err(error) => {
                self.analytics.record(with_character(
                    AnalyticsEvent::new("mcp.session_tool_failed", EventLevel::Warn)
                        .correlation(correlation_id)
                        .attribute("tool", tool)
                        .attribute("duration_ms", elapsed_ms(started))
                        .attribute("error_class", error.class()),
                    analytics_character_id.as_deref(),
                ));
                return Err(error.into());
            }
        };
        let decoded = match serde_json::from_value(result) {
            Ok(decoded) => decoded,
            Err(source) => {
                self.analytics.record(with_character(
                    AnalyticsEvent::new("mcp.session_tool_decode_failed", EventLevel::Warn)
                        .correlation(correlation_id)
                        .attribute("tool", tool)
                        .attribute("duration_ms", elapsed_ms(started)),
                    analytics_character_id.as_deref(),
                ));
                return Err(SessionError::Decode {
                    operation: tool,
                    source,
                });
            }
        };
        self.analytics.record(with_character(
            AnalyticsEvent::new("mcp.session_tool_completed", EventLevel::Debug)
                .correlation(correlation_id)
                .attribute("tool", tool)
                .attribute("duration_ms", elapsed_ms(started)),
            analytics_character_id.as_deref(),
        ));
        Ok(decoded)
    }

    async fn request_typed<T: DeserializeOwned>(
        &self,
        operation: &'static str,
        params: Value,
        correlation_id: Uuid,
    ) -> Result<T, SessionError> {
        let result = self
            .transport
            .request(operation, params, correlation_id)
            .await?;
        serde_json::from_value(result).map_err(|source| SessionError::Decode { operation, source })
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn session_error_class(error: &SessionError) -> &'static str {
    match error {
        SessionError::Mcp(error) => error.class(),
        SessionError::Decode { .. } => "decode",
        SessionError::MissingAgentIdentity => "missing_agent_identity",
        SessionError::ReconnectExhausted { .. } => "reconnect_exhausted",
    }
}

fn with_character(event: AnalyticsEvent, character_id: Option<&str>) -> AnalyticsEvent {
    match character_id {
        Some(character_id) => event.character(character_id),
        None => event,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, VecDeque},
        sync::Mutex,
    };

    use async_trait::async_trait;

    use crate::{config::HarnessConfig, observability::RecordingAnalyticsSink};

    use super::*;

    #[derive(Default)]
    struct ScriptedTransport {
        requests: Mutex<Vec<(String, Value)>>,
        responses: Mutex<VecDeque<Value>>,
        session: RwLock<Option<String>>,
    }

    #[async_trait]
    impl McpTransport for ScriptedTransport {
        async fn request(
            &self,
            method: &str,
            params: Value,
            _correlation_id: Uuid,
        ) -> Result<Value, McpError> {
            self.requests
                .lock()
                .expect("requests")
                .push((method.to_owned(), params));
            self.responses
                .lock()
                .expect("responses")
                .pop_front()
                .ok_or_else(|| McpError::Protocol {
                    message: "test response missing".to_owned(),
                })
        }

        async fn notify(
            &self,
            method: &str,
            params: Value,
            _correlation_id: Uuid,
        ) -> Result<(), McpError> {
            self.requests
                .lock()
                .expect("requests")
                .push((method.to_owned(), params));
            Ok(())
        }

        async fn reset_session(&self) {
            *self.session.write().await = None;
        }

        async fn session_id(&self) -> Option<String> {
            self.session.read().await.clone()
        }
    }

    fn character() -> CharacterSheet {
        let values = HashMap::from([
            ("ARENA_API_KEY", "arena"),
            ("OPENROUTER_API_KEY", "router"),
            (
                "NPC_CHARACTER_SHEET_PATH",
                concat!(env!("CARGO_MANIFEST_DIR"), "/characters/guy.json"),
            ),
        ]);
        let config = HarnessConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
            .expect("config");
        config.character_sheet().expect("Guy")
    }

    fn tool(body: &Value) -> Value {
        json!({ "content": [{"type": "text", "text": body.to_string()}] })
    }

    #[tokio::test]
    async fn connects_existing_character_in_protocol_order() {
        let transport = Arc::new(ScriptedTransport::default());
        transport.responses.lock().expect("responses").extend([
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "serverInfo": {"name": "arena", "version": "1"}
            }),
            tool(&json!({"agents": [{"id": "agent-guy", "playerName": "Guy"}]})),
            tool(&json!({"loggedIn": true})),
        ]);
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let session = ArenaSession::new(transport.clone(), analytics.clone());

        let connected = session.connect(&character()).await.expect("connect");
        assert_eq!(connected.agent.id, "agent-guy");
        let calls = transport.requests.lock().expect("requests");
        assert_eq!(calls[0].0, "initialize");
        assert_eq!(calls[1].0, "notifications/initialized");
        assert_eq!(calls[2].1["name"], "arena_list_agents");
        assert_eq!(calls[3].1["name"], "arena_login");
        assert_eq!(calls[3].1["arguments"], json!({"agent_id": "agent-guy"}));
        assert!(
            analytics
                .events()
                .iter()
                .any(|event| event.name == "mcp.session_connected")
        );
    }

    #[tokio::test]
    async fn registration_uses_stable_idempotency_key() {
        let transport = Arc::new(ScriptedTransport::default());
        transport.responses.lock().expect("responses").extend([
            json!({"protocolVersion": PROTOCOL_VERSION, "capabilities": {}}),
            tool(&json!({"agents": []})),
            tool(&json!({"agent": {"id": "new-guy", "playerName": "Guy"}})),
            tool(&json!({"loggedIn": true})),
        ]);
        let session = ArenaSession::new(
            transport.clone(),
            Arc::new(RecordingAnalyticsSink::default()),
        );

        session.connect(&character()).await.expect("connect");
        let calls = transport.requests.lock().expect("requests");
        let registration = &calls[3].1["arguments"];
        assert_eq!(registration["idempotency_key"], "npc-guy-v1");
        assert_eq!(registration["agent_name"], "guy");
        assert_eq!(registration["player_name"], "Guy");
    }

    #[tokio::test]
    async fn reconnect_notifies_runtime_to_invalidate_decisions() {
        let transport = Arc::new(ScriptedTransport::default());
        transport.responses.lock().expect("responses").extend([
            json!({"protocolVersion": PROTOCOL_VERSION, "capabilities": {}}),
            tool(&json!({"agents": [{"id": "agent-guy", "playerName": "Guy"}]})),
            tool(&json!({"loggedIn": true})),
        ]);
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let session = ArenaSession::new(transport, analytics.clone());
        let mut events = session.subscribe();

        session
            .reconnect(&character(), 1, Duration::ZERO)
            .await
            .expect("reconnect");
        let first = events.recv().await.expect("reconnected event");
        let second = events.recv().await.expect("invalidation event");
        assert!(matches!(first, SessionEvent::Reconnected { .. }));
        assert!(matches!(second, SessionEvent::DecisionsInvalidated { .. }));
        assert!(
            analytics
                .events()
                .iter()
                .any(|event| event.name == "runtime.decisions_invalidated")
        );
    }

    #[tokio::test]
    async fn session_tool_and_connect_failures_have_terminal_events() {
        let transport = Arc::new(ScriptedTransport::default());
        transport
            .responses
            .lock()
            .expect("responses")
            .push_back(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {}
            }));
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let session = ArenaSession::new(transport, analytics.clone());

        assert!(
            session.connect(&character()).await.is_err(),
            "missing list response must fail"
        );
        let names = analytics
            .events()
            .into_iter()
            .map(|event| event.name)
            .collect::<Vec<_>>();
        assert!(names.iter().any(|name| name == "mcp.session_tool_failed"));
        assert!(
            names
                .iter()
                .any(|name| name == "mcp.session_connect_failed")
        );
    }
}
