use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Context;
use ractor::{Actor, ActorRef, concurrency::JoinHandle};

use crate::{
    brain::{
        models::{
            DisabledBrain, ModelBackgroundTasks, ModelCallObservability, ModelUsageLedger,
            ModelUsageTotals, OpenRouterJsonBrain,
        },
        prompts::{STRATEGIST_V6, STRATEGIST_V6_VERSION, TACTICIAN_V12, TACTICIAN_V12_VERSION},
        strategic_agentic::{RigStrategicAgent, RigStrategicProposalBrain},
        strategic_input::StrategicInput,
        strategic_intent::StrategicIntent,
        strategic_output::StrategicProposal,
        tactical_input::TacticalInput,
        tactical_output::TacticalProposal,
    },
    character::CharacterSheet,
    config::{HarnessConfig, ModelReasoningConfig, RuntimeConfig},
    execution::gateway::{BodyGateway, DisabledBodyGateway},
    mcp::{ArenaGateway, session::SessionEvent},
    memory::{
        sqlite_conversation::{SqliteConversationMemory, bounded_conversation_memory},
        sqlite_store::SqliteMemoryStore,
        store::MemoryStore,
        working::{WorkStatus, WorkingMemory},
    },
    observability::{AnalyticsEvent, AnalyticsSink, EventLevel, tracing_sink},
    runtime::{
        blackboard::HotBlackboard,
        control_gate::{ControlledPacketError, ControlledPacketReceipt, ControlledPacketRequest},
        messages::{
            BodyStatus, PlayerRuntimeStatus, PlayerSupervisorMsg, SafetyFallbackResult,
            TelemetrySnapshot,
        },
        perception_pump::PerceptionSource,
        safety::{RuntimeSafetyStop, evaluate_runtime_safety},
        supervisor::{PlayerSupervisor, PlayerSupervisorArgs},
    },
};

pub struct PlayerRuntime {
    supervisor: ActorRef<PlayerSupervisorMsg>,
    join: JoinHandle<()>,
    analytics: Arc<dyn AnalyticsSink>,
    character_id: String,
    runtime_id: uuid::Uuid,
    blackboard: Arc<HotBlackboard>,
    runtime_config: RuntimeConfig,
    model_background_tasks: Arc<ModelBackgroundTasks>,
    tactician_usage: Arc<ModelUsageLedger>,
    strategist_usage: Arc<ModelUsageLedger>,
    started_at: Instant,
    session_monitor: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeRunSummary {
    pub runtime_id: uuid::Uuid,
    pub character_id: String,
    pub shutdown_reason: String,
    pub connected_duration_ms: u64,
    pub tactician_usage: ModelUsageTotals,
    pub strategist_usage: ModelUsageTotals,
    pub total_usage: ModelUsageTotals,
    pub projected_cost_per_24_connected_hours_usd: f64,
    pub action_success_rate: Option<f64>,
    pub packet_completion_rate: Option<f64>,
    pub telemetry: TelemetrySnapshot,
    pub body: BodyStatus,
}

#[cfg(feature = "local-rag")]
fn configure_semantic_memory(
    store: Arc<dyn MemoryStore>,
    analytics: Arc<dyn AnalyticsSink>,
    character_id: &str,
    enabled: bool,
    minimum_score: f64,
) -> Arc<dyn MemoryStore> {
    analytics.record(
        AnalyticsEvent::new("memory.rag_configured", EventLevel::Info)
            .character(character_id)
            .attribute("enabled", enabled)
            .attribute("implementation", "rig_fastembed_in_memory")
            .attribute("index_version", "rig-local-rag-v1")
            .attribute("embedding_model", "fastembed/all-minilm-l6-v2-q")
            .attribute("minimum_score", minimum_score),
    );
    Arc::new(crate::memory::rig_semantic::RigSemanticMemoryStore::new(
        store,
        analytics,
        crate::memory::rig_semantic::LocalRagConfig {
            enabled,
            minimum_score,
        },
    ))
}

#[cfg(not(feature = "local-rag"))]
#[allow(
    clippy::needless_pass_by_value,
    reason = "the no-feature compatibility seam mirrors the enabled implementation"
)]
fn configure_semantic_memory(
    store: Arc<dyn MemoryStore>,
    analytics: Arc<dyn AnalyticsSink>,
    character_id: &str,
    enabled: bool,
    minimum_score: f64,
) -> Arc<dyn MemoryStore> {
    analytics.record(
        AnalyticsEvent::new(
            "memory.rag_feature_unavailable",
            if enabled {
                EventLevel::Warn
            } else {
                EventLevel::Info
            },
        )
        .character(character_id)
        .attribute("configured_enabled", enabled)
        .attribute("fallback", "deterministic_lexical")
        .attribute("reason", "binary_built_without_local_rag_feature")
        .attribute("minimum_score", minimum_score),
    );
    store
}

impl PlayerRuntime {
    /// Start one supervised player actor subtree.
    ///
    /// # Errors
    ///
    /// Returns an error if the supervisor or any child actor fails during
    /// startup.
    pub async fn start(config: HarnessConfig, character: CharacterSheet) -> anyhow::Result<Self> {
        Self::start_with_analytics(config, character, tracing_sink()).await
    }

    /// Start the runtime with an injected analytics sink.
    ///
    /// # Errors
    ///
    /// Returns an error if the supervisor or any child actor fails during
    /// startup.
    pub async fn start_with_analytics(
        config: HarnessConfig,
        character: CharacterSheet,
        analytics: Arc<dyn AnalyticsSink>,
    ) -> anyhow::Result<Self> {
        Self::start_with_dependencies(
            config,
            character,
            Arc::new(DisabledBodyGateway),
            0,
            false,
            None,
            None,
            analytics,
        )
        .await
    }

    /// Start a player runtime whose `BodyActor` owns a connected character gateway.
    ///
    /// # Errors
    ///
    /// Returns an error if the supervised actor subtree cannot start.
    pub async fn start_connected_with_analytics(
        config: HarnessConfig,
        character: CharacterSheet,
        gateway: ArenaGateway,
        session_generation: u64,
        analytics: Arc<dyn AnalyticsSink>,
    ) -> anyhow::Result<Self> {
        let gateway = Arc::new(gateway);
        Self::start_with_dependencies(
            config,
            character,
            gateway.clone(),
            session_generation,
            true,
            Some(gateway),
            None,
            analytics,
        )
        .await
    }

    /// Start a connected runtime and consume session invalidation events.
    ///
    /// # Errors
    ///
    /// Returns an error if the supervised actor subtree cannot start.
    pub async fn start_connected_with_session_events(
        config: HarnessConfig,
        character: CharacterSheet,
        gateway: ArenaGateway,
        session_generation: u64,
        session_events: tokio::sync::broadcast::Receiver<SessionEvent>,
        analytics: Arc<dyn AnalyticsSink>,
    ) -> anyhow::Result<Self> {
        let gateway = Arc::new(gateway);
        Self::start_with_dependencies(
            config,
            character,
            gateway.clone(),
            session_generation,
            true,
            Some(gateway),
            Some(session_events),
            analytics,
        )
        .await
    }

    /// Start a connected runtime with an independently recoverable perception source.
    ///
    /// # Errors
    ///
    /// Returns an error if the supervised actor subtree cannot start.
    pub async fn start_connected_with_session_events_and_source(
        config: HarnessConfig,
        character: CharacterSheet,
        gateway: ArenaGateway,
        perception_source: Arc<dyn PerceptionSource>,
        session_generation: u64,
        session_events: tokio::sync::broadcast::Receiver<SessionEvent>,
        analytics: Arc<dyn AnalyticsSink>,
    ) -> anyhow::Result<Self> {
        Self::start_with_dependencies(
            config,
            character,
            Arc::new(gateway),
            session_generation,
            true,
            Some(perception_source),
            Some(session_events),
            analytics,
        )
        .await
    }

    #[allow(
        clippy::too_many_arguments,
        clippy::too_many_lines,
        reason = "this private constructor makes every runtime dependency and startup stage explicit"
    )]
    async fn start_with_dependencies(
        config: HarnessConfig,
        character: CharacterSheet,
        body_gateway: Arc<dyn BodyGateway>,
        session_generation: u64,
        body_connected: bool,
        perception_source: Option<Arc<dyn PerceptionSource>>,
        session_events: Option<tokio::sync::broadcast::Receiver<SessionEvent>>,
        analytics: Arc<dyn AnalyticsSink>,
    ) -> anyhow::Result<Self> {
        let model_background_tasks = Arc::new(ModelBackgroundTasks::default());
        let tactician_usage = Arc::new(ModelUsageLedger::default());
        let strategist_usage = Arc::new(ModelUsageLedger::default());
        let brain = build_tactician_brain(
            &config,
            &character,
            &analytics,
            model_background_tasks.clone(),
            tactician_usage.clone(),
        )?;
        let sqlite_memory_store: Arc<dyn MemoryStore> = Arc::new(
            SqliteMemoryStore::open(&config.memory_path, analytics.clone())
                .await
                .with_context(|| {
                    format!(
                        "failed to open typed memory store at {}",
                        config.memory_path.display()
                    )
                })?,
        );
        let memory_store = configure_semantic_memory(
            sqlite_memory_store,
            analytics.clone(),
            &character.id,
            config.local_rag_enabled,
            config.local_rag_minimum_score,
        );
        let persisted_working = memory_store
            .load_working(&character.id)
            .await
            .context("failed to load persisted working memory before actor startup")?;
        let initial_strategy = initial_strategy(&character, &persisted_working);
        let blackboard = Arc::new(HotBlackboard::new(initial_strategy));
        let strategist_conversation_memory: Option<Arc<dyn rig_core::memory::ConversationMemory>> =
            if config.models.strategist_enabled && character.remembers {
                let memory = SqliteConversationMemory::open(
                    &config.memory_path,
                    &character.id,
                    analytics.clone(),
                )
                .await
                .with_context(|| {
                    format!(
                        "failed to open Rig conversation memory at {}",
                        config.memory_path.display()
                    )
                })?;
                Some(Arc::new(bounded_conversation_memory(
                    memory,
                    config.models.strategist_memory_max_tokens,
                )))
            } else {
                None
            };
        let strategist_brain = build_strategist_brain(
            &config,
            &character,
            &analytics,
            model_background_tasks.clone(),
            strategist_usage.clone(),
            strategist_conversation_memory,
        )?;
        let runtime_config = config.runtime.clone();
        let character = Arc::new(character);
        let config = Arc::new(config);
        let runtime_id = uuid::Uuid::new_v4();
        let runtime_prefix = format!("{}-{runtime_id}", character.id);
        let character_id = character.id.clone();
        let (supervisor, join) = Actor::spawn(
            Some(format!("{runtime_prefix}-player-supervisor")),
            PlayerSupervisor,
            PlayerSupervisorArgs {
                runtime_prefix,
                config,
                character,
                blackboard: blackboard.clone(),
                tactician_brain: brain,
                strategist_brain,
                memory_store,
                body_gateway,
                session_generation,
                body_connected,
                perception_source,
                analytics: analytics.clone(),
            },
        )
        .await
        .context("failed to start player supervisor")?;
        analytics.record(
            AnalyticsEvent::new("runtime.started", EventLevel::Info)
                .character(&character_id)
                .correlation(runtime_id)
                .attribute("runtime_id", runtime_id.to_string()),
        );
        let session_monitor = session_events.map(|events| {
            start_session_monitor(
                events,
                supervisor.clone(),
                analytics.clone(),
                character_id.clone(),
            )
        });
        Ok(Self {
            supervisor,
            join,
            analytics,
            character_id,
            runtime_id,
            blackboard,
            runtime_config,
            model_background_tasks,
            tactician_usage,
            strategist_usage,
            started_at: Instant::now(),
            session_monitor,
        })
    }

    /// Query which actors in the player subtree are currently running.
    ///
    /// # Errors
    ///
    /// Returns an error when the supervisor mailbox is unavailable or does not
    /// answer within one second.
    pub async fn status(&self) -> anyhow::Result<PlayerRuntimeStatus> {
        ractor::call_t!(self.supervisor, PlayerSupervisorMsg::Health, 1_000)
            .context("player supervisor health check failed")
    }

    /// Query the executor's current and last terminal packet state.
    ///
    /// # Errors
    ///
    /// Returns an error when either the supervisor or body mailbox is
    /// unavailable or does not answer within one second.
    pub async fn body_status(&self) -> anyhow::Result<BodyStatus> {
        ractor::call_t!(self.supervisor, PlayerSupervisorMsg::BodyHealth, 1_000)
            .context("body actor health check failed")
    }

    /// Query the actor-owned operational counters.
    ///
    /// # Errors
    ///
    /// Returns an error when either the supervisor or telemetry mailbox is
    /// unavailable or does not answer within one second.
    pub async fn telemetry_snapshot(&self) -> anyhow::Result<TelemetrySnapshot> {
        ractor::call_t!(self.supervisor, PlayerSupervisorMsg::TelemetryHealth, 1_000)
            .context("telemetry actor snapshot failed")
    }

    /// Build a point-in-time summary of this connected runtime.
    ///
    /// # Errors
    ///
    /// Returns an error when the body or telemetry actor cannot provide its
    /// current state.
    pub async fn run_summary(
        &self,
        shutdown_reason: impl Into<String>,
    ) -> anyhow::Result<RuntimeRunSummary> {
        let duration_ms = u64::try_from(self.started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        let tactician_usage = self.tactician_usage.totals_for(&self.character_id);
        let strategist_usage = self.strategist_usage.totals_for(&self.character_id);
        let total_usage = combine_usage(&tactician_usage, &strategist_usage);
        let connected_hours = self.started_at.elapsed().as_secs_f64() / 3_600.0;
        let projected_cost = if connected_hours > 0.0 {
            total_usage.exact_cost_usd / connected_hours * 24.0
        } else {
            0.0
        };
        Ok(RuntimeRunSummary {
            runtime_id: self.runtime_id,
            character_id: self.character_id.clone(),
            shutdown_reason: shutdown_reason.into(),
            connected_duration_ms: duration_ms,
            tactician_usage,
            strategist_usage,
            total_usage,
            projected_cost_per_24_connected_hours_usd: projected_cost,
            telemetry: self.telemetry_snapshot().await?,
            body: self.body_status().await?,
            action_success_rate: None,
            packet_completion_rate: None,
        }
        .with_rates())
    }

    /// Return the latest immutable tactical frame for diagnostic assertions.
    pub fn tactical_frame(&self) -> Arc<crate::brain::tactical_frame::TacticalFrame> {
        self.blackboard.frame()
    }

    /// Wait until a full live rollout reaches a fail-closed safety condition.
    pub async fn wait_for_safety_stop(&self) -> RuntimeSafetyStop {
        let mut revisions = self.blackboard.subscribe_frames();
        let mut tactical_usage_revisions = self.tactician_usage.subscribe();
        let mut strategic_usage_revisions = self.strategist_usage.subscribe();
        loop {
            let frame = self.blackboard.frame();
            if let Some(stop) = evaluate_runtime_safety(&self.runtime_config, &frame) {
                self.record_safety_stop(stop, &frame);
                return stop;
            }
            if let Some(stop) = self.evaluate_model_cost_safety() {
                self.record_safety_stop(stop, &frame);
                return stop;
            }
            tokio::select! {
                result = revisions.changed() => {
                    if result.is_err() {
                        let stop = RuntimeSafetyStop::PerceptionUpdatesUnavailable;
                        self.record_safety_stop(stop, &frame);
                        return stop;
                    }
                }
                _ = tactical_usage_revisions.changed() => {}
                _ = strategic_usage_revisions.changed() => {}
            }
        }
    }

    fn evaluate_model_cost_safety(&self) -> Option<RuntimeSafetyStop> {
        let totals = self.total_model_usage();
        crate::runtime::safety::evaluate_model_cost_safety(
            self.runtime_config.run_max_openrouter_cost_usd,
            &totals,
        )
    }

    fn record_safety_stop(
        &self,
        stop: RuntimeSafetyStop,
        frame: &crate::brain::tactical_frame::TacticalFrame,
    ) {
        self.analytics.record(
            AnalyticsEvent::new("runtime.safety_stop_triggered", EventLevel::Error)
                .character(&self.character_id)
                .correlation(self.runtime_id)
                .attribute("runtime_id", self.runtime_id.to_string())
                .attribute("reason_code", stop.reason_code())
                .attribute("frame_revision", frame.revision)
                .attribute("strategic_revision", frame.strategic_intent.revision)
                .attribute("combat_active", frame.combat.active == Some(true))
                .attribute("health_known", frame.self_state.health.is_some())
                .attribute("max_health_known", frame.self_state.max_health.is_some())
                .attribute("scene_known", frame.self_state.scene.is_some())
                .attribute("scene", frame.self_state.scene.as_deref().unwrap_or(""))
                .attribute(
                    "model_cost_limit_usd",
                    self.runtime_config
                        .run_max_openrouter_cost_usd
                        .unwrap_or_default(),
                )
                .attribute("model_cost_usd", self.total_model_usage().exact_cost_usd),
        );
    }

    fn total_model_usage(&self) -> ModelUsageTotals {
        combine_usage(
            &self.tactician_usage.totals_for(&self.character_id),
            &self.strategist_usage.totals_for(&self.character_id),
        )
    }

    /// Put the backend reflex layer into semi-automatic flee mode before a
    /// safety-triggered disconnect.
    ///
    /// # Errors
    ///
    /// Returns an error when the supervisor or body cannot confirm the
    /// character-bound `set_tactics(flee, semi_auto)` operation within three
    /// seconds.
    pub async fn activate_safety_fallback(
        &self,
        stop: RuntimeSafetyStop,
    ) -> anyhow::Result<SafetyFallbackResult> {
        self.analytics.record(
            AnalyticsEvent::new("runtime.safety_fallback_started", EventLevel::Warn)
                .character(&self.character_id)
                .correlation(self.runtime_id)
                .attribute("runtime_id", self.runtime_id.to_string())
                .attribute("reason_code", stop.reason_code())
                .attribute("fallback_style", "flee")
                .attribute("fallback_mode", "semi_auto"),
        );
        let result = ractor::call_t!(
            self.supervisor,
            PlayerSupervisorMsg::ActivateSafetyFallback,
            3_000,
            stop.reason_code().to_owned()
        )
        .context("safety fallback body call failed")?
        .map_err(|error| anyhow::anyhow!(error))?;
        anyhow::ensure!(
            result.status == crate::execution::outcome::OutcomeStatus::Succeeded,
            "safety fallback body action ended with status {:?}",
            result.status
        );
        self.analytics.record(
            AnalyticsEvent::new("runtime.safety_fallback_completed", EventLevel::Warn)
                .character(&self.character_id)
                .correlation(result.context.action_id)
                .attribute("runtime_id", self.runtime_id.to_string())
                .attribute("reason_code", stop.reason_code())
                .attribute("decision_id", result.context.decision_id.to_string())
                .attribute("packet_id", result.context.packet_id.to_string())
                .attribute("action_id", result.context.action_id.to_string())
                .attribute("frame_revision", result.context.frame_revision)
                .attribute("strategic_revision", result.context.strategic_revision)
                .attribute("duration_ms", result.duration_ms)
                .attribute("status", format!("{:?}", result.status).to_lowercase())
                .attribute(
                    "terminal_reason_code",
                    result.reason_code.as_deref().unwrap_or(""),
                ),
        );
        Ok(result)
    }

    /// Validate a runtime-created packet through the real `BodyActor` without
    /// consuming live-action budget or executing it.
    ///
    /// # Errors
    ///
    /// Returns [`ControlledPacketError`] when runtime assertions, packet
    /// limits, body validation, or supervisor communication fail.
    pub async fn validate_tactical_packet(
        &self,
        request: ControlledPacketRequest,
    ) -> Result<ControlledPacketReceipt, ControlledPacketError> {
        ractor::call_t!(
            self.supervisor,
            PlayerSupervisorMsg::ValidateControlledPacket,
            2_000,
            request
        )
        .map_err(|_| ControlledPacketError::SupervisorUnavailable)?
    }

    /// Submit exactly one explicitly asserted proposal through the controlled
    /// live-mutation gate and the real `BodyActor`.
    ///
    /// # Errors
    ///
    /// Returns [`ControlledPacketError`] unless every live-mutation gate and
    /// body validation check passes and the packet can be queued for execution.
    pub async fn submit_controlled_packet(
        &self,
        request: ControlledPacketRequest,
    ) -> Result<ControlledPacketReceipt, ControlledPacketError> {
        ractor::call_t!(
            self.supervisor,
            PlayerSupervisorMsg::SubmitControlledPacket,
            2_000,
            request
        )
        .map_err(|_| ControlledPacketError::SupervisorUnavailable)?
    }

    /// Gracefully stop the entire player subtree.
    ///
    /// # Errors
    ///
    /// Returns an error if the shutdown message cannot be delivered, the
    /// supervisor task fails, or shutdown exceeds five seconds.
    pub async fn shutdown(self) -> anyhow::Result<RuntimeRunSummary> {
        self.shutdown_with_reason("api_request").await
    }

    /// Gracefully stop the player subtree and emit its terminal run summary.
    ///
    /// # Errors
    ///
    /// Returns an error if the pre-shutdown snapshot, shutdown request, actor
    /// join, or accounting drain fails.
    pub async fn shutdown_with_reason(
        mut self,
        shutdown_reason: impl Into<String>,
    ) -> anyhow::Result<RuntimeRunSummary> {
        let started = std::time::Instant::now();
        let shutdown_reason = shutdown_reason.into();
        let mut summary = self.run_summary(shutdown_reason.clone()).await?;
        self.analytics.record(
            AnalyticsEvent::new("runtime.shutdown_started", EventLevel::Info)
                .character(&self.character_id)
                .correlation(self.runtime_id),
        );
        if let Some(monitor) = self.session_monitor.take() {
            monitor.abort();
        }
        self.supervisor
            .send_message(PlayerSupervisorMsg::Shutdown)
            .context("failed to request player shutdown")?;
        tokio::time::timeout(Duration::from_secs(5), self.join)
            .await
            .context("player supervisor did not stop within five seconds")?
            .context("player supervisor task failed")?;
        let accounting = self
            .model_background_tasks
            .drain(Duration::from_secs(35))
            .await;
        summary.tactician_usage = self.tactician_usage.totals_for(&self.character_id);
        summary.strategist_usage = self.strategist_usage.totals_for(&self.character_id);
        summary.total_usage = combine_usage(&summary.tactician_usage, &summary.strategist_usage);
        let connected_hours =
            Duration::from_millis(summary.connected_duration_ms).as_secs_f64() / 3_600.0;
        summary.projected_cost_per_24_connected_hours_usd = if connected_hours > 0.0 {
            summary.total_usage.exact_cost_usd / connected_hours * 24.0
        } else {
            0.0
        };
        summary = summary.with_rates();
        self.analytics.record(
            AnalyticsEvent::new("model.background_tasks_drained", EventLevel::Info)
                .character(&self.character_id)
                .correlation(self.runtime_id)
                .attribute(
                    "completed",
                    u64::try_from(accounting.completed).unwrap_or(u64::MAX),
                )
                .attribute(
                    "failed",
                    u64::try_from(accounting.failed).unwrap_or(u64::MAX),
                )
                .attribute(
                    "aborted",
                    u64::try_from(accounting.aborted).unwrap_or(u64::MAX),
                )
                .attribute(
                    "active_model_calls_remaining",
                    u64::try_from(accounting.active_model_calls_remaining).unwrap_or(u64::MAX),
                ),
        );
        self.analytics.record(run_summary_event(&summary));
        self.analytics.record(
            AnalyticsEvent::new("runtime.shutdown_completed", EventLevel::Info)
                .character(&self.character_id)
                .correlation(self.runtime_id)
                .attribute(
                    "duration_ms",
                    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                ),
        );
        Ok(summary)
    }
}

fn initial_strategy(character: &CharacterSheet, working: &WorkingMemory) -> StrategicIntent {
    if let Some(intent) = &working.strategic_intent {
        let mut restored = intent.clone();
        restored.revision = restored.revision.max(1);
        return restored;
    }
    let objective = working
        .goal
        .as_ref()
        .filter(|goal| !goal.aim.trim().is_empty())
        .or(character
            .initial_goal
            .as_ref()
            .filter(|goal| !goal.aim.trim().is_empty()))
        .map_or_else(
            || "Live according to the character's identity.".to_owned(),
            |goal| goal.aim.clone(),
        );
    let subgoals = working
        .plan
        .iter()
        .filter(|step| matches!(step.status, WorkStatus::Next | WorkStatus::Doing))
        .map(|step| step.what.clone())
        .chain(
            working
                .todo
                .iter()
                .filter(|item| matches!(item.status, WorkStatus::Next | WorkStatus::Doing))
                .map(|item| item.what.clone()),
        )
        .collect();
    StrategicIntent {
        revision: 1,
        objective,
        subgoals,
        ..StrategicIntent::default()
    }
}

fn combine_usage(left: &ModelUsageTotals, right: &ModelUsageTotals) -> ModelUsageTotals {
    ModelUsageTotals {
        calls: left.calls.saturating_add(right.calls),
        input_tokens: left.input_tokens.saturating_add(right.input_tokens),
        output_tokens: left.output_tokens.saturating_add(right.output_tokens),
        total_tokens: left.total_tokens.saturating_add(right.total_tokens),
        cached_input_tokens: left
            .cached_input_tokens
            .saturating_add(right.cached_input_tokens),
        cache_creation_input_tokens: left
            .cache_creation_input_tokens
            .saturating_add(right.cache_creation_input_tokens),
        tool_use_prompt_tokens: left
            .tool_use_prompt_tokens
            .saturating_add(right.tool_use_prompt_tokens),
        reasoning_tokens: left.reasoning_tokens.saturating_add(right.reasoning_tokens),
        exact_cost_known_calls: left
            .exact_cost_known_calls
            .saturating_add(right.exact_cost_known_calls),
        exact_cost_usd: left.exact_cost_usd + right.exact_cost_usd,
    }
}

fn run_summary_event(summary: &RuntimeRunSummary) -> AnalyticsEvent {
    AnalyticsEvent::new("runtime.run_summary", EventLevel::Info)
        .character(&summary.character_id)
        .correlation(summary.runtime_id)
        .attribute("runtime_id", summary.runtime_id.to_string())
        .attribute("shutdown_reason", summary.shutdown_reason.clone())
        .attribute("connected_duration_ms", summary.connected_duration_ms)
        .attribute("model_calls", summary.total_usage.calls)
        .attribute("input_tokens", summary.total_usage.input_tokens)
        .attribute("output_tokens", summary.total_usage.output_tokens)
        .attribute("total_tokens", summary.total_usage.total_tokens)
        .attribute(
            "cached_input_tokens",
            summary.total_usage.cached_input_tokens,
        )
        .attribute("reasoning_tokens", summary.total_usage.reasoning_tokens)
        .attribute(
            "exact_cost_known_calls",
            summary.total_usage.exact_cost_known_calls,
        )
        .attribute("openrouter_cost_usd", summary.total_usage.exact_cost_usd)
        .attribute("tactician_calls", summary.tactician_usage.calls)
        .attribute("tactician_cost_usd", summary.tactician_usage.exact_cost_usd)
        .attribute("strategist_calls", summary.strategist_usage.calls)
        .attribute(
            "strategist_cost_usd",
            summary.strategist_usage.exact_cost_usd,
        )
        .attribute(
            "projected_cost_per_24_connected_hours_usd",
            summary.projected_cost_per_24_connected_hours_usd,
        )
        .attribute("actor_failures", summary.telemetry.actor_failures)
        .attribute("packets_accepted", summary.telemetry.packets_accepted)
        .attribute("packets_completed", summary.telemetry.packets_completed)
        .attribute("packets_failed", summary.telemetry.packets_failed)
        .attribute("actions_started", summary.telemetry.actions_started)
        .attribute("actions_succeeded", summary.telemetry.actions_succeeded)
        .attribute("actions_failed", summary.telemetry.actions_failed)
        .attribute(
            "action_success_rate_known",
            summary.action_success_rate.is_some(),
        )
        .attribute(
            "action_success_rate",
            summary.action_success_rate.unwrap_or_default(),
        )
        .attribute(
            "packet_completion_rate_known",
            summary.packet_completion_rate.is_some(),
        )
        .attribute(
            "packet_completion_rate",
            summary.packet_completion_rate.unwrap_or_default(),
        )
        .attribute("movement_arrivals", summary.telemetry.movement_arrivals)
        .attribute("movement_stalls", summary.telemetry.movement_stalls)
        .attribute("movement_stops", summary.telemetry.movement_stops)
        .attribute(
            "movement_stop_failures",
            summary.telemetry.movement_stop_failures,
        )
        .attribute(
            "tactical_inferences_started",
            summary.telemetry.tactical_inferences_started,
        )
        .attribute(
            "tactical_inferences_completed",
            summary.telemetry.tactical_inferences_completed,
        )
        .attribute(
            "tactical_inferences_failed",
            summary.telemetry.tactical_inferences_failed,
        )
        .attribute("body_connected", summary.body.connected)
}

impl RuntimeRunSummary {
    fn with_rates(mut self) -> Self {
        self.action_success_rate = success_rate(
            self.telemetry.actions_succeeded,
            self.telemetry.actions_failed,
        );
        self.packet_completion_rate = success_rate(
            self.telemetry.packets_completed,
            self.telemetry
                .packets_failed
                .saturating_add(self.telemetry.packets_cancelled)
                .saturating_add(self.telemetry.packets_superseded),
        );
        self
    }
}

fn success_rate(succeeded: u64, unsuccessful: u64) -> Option<f64> {
    let total = succeeded.saturating_add(unsuccessful);
    if total == 0 {
        return None;
    }
    Some(num_traits::ToPrimitive::to_f64(&succeeded)? / num_traits::ToPrimitive::to_f64(&total)?)
}

fn build_tactician_brain(
    config: &HarnessConfig,
    character: &CharacterSheet,
    analytics: &Arc<dyn AnalyticsSink>,
    background_tasks: Arc<ModelBackgroundTasks>,
    usage_ledger: Arc<ModelUsageLedger>,
) -> anyhow::Result<Arc<dyn crate::brain::Brain<TacticalInput, TacticalProposal>>> {
    if !config.runtime.tactical_rollout_mode.allows_inference() {
        return Ok(Arc::new(DisabledBrain));
    }
    let brain = OpenRouterJsonBrain::<TacticalInput, TacticalProposal>::new_observed(
        config.openrouter_api_key()?,
        &character.tactician_model,
        format!(
            "{TACTICIAN_V12}\n\nThe runtime supplies all identity, packet, and revision fields."
        ),
        config.models.tactician_temperature,
        config.models.tactician_max_output_tokens,
        ModelCallObservability::new(TACTICIAN_V12_VERSION, analytics.clone())
            .with_role("tactician")
            .with_usage_ledger(usage_ledger)
            .with_background_tasks(background_tasks),
    )?
    .with_reasoning(config.models.tactician_reasoning)?
    .with_local_input_capture(config.models.local_input_capture.clone())
    .with_prompt_caching(config.models.prompt_caching_enabled)
    .with_session_id(format!("{}-tactician", character.id))
    .with_request_timeout(config.models.tactician_request_timeout);
    record_brain_configuration(
        analytics,
        character,
        "tactician",
        &character.tactician_model,
        TACTICIAN_V12_VERSION,
        config.models.tactician_max_output_tokens,
        config.models.tactician_request_timeout,
        config.models.tactician_reasoning,
        config.models.prompt_caching_enabled,
    );
    Ok(Arc::new(brain))
}

fn build_strategist_brain(
    config: &HarnessConfig,
    character: &CharacterSheet,
    analytics: &Arc<dyn AnalyticsSink>,
    background_tasks: Arc<ModelBackgroundTasks>,
    usage_ledger: Arc<ModelUsageLedger>,
    conversation_memory: Option<Arc<dyn rig_core::memory::ConversationMemory>>,
) -> anyhow::Result<Option<Arc<dyn crate::brain::Brain<StrategicInput, StrategicProposal>>>> {
    if !config.models.strategist_enabled {
        return Ok(None);
    }
    // The persistent ThinkTool-enabled loop is opt-in during rollout. This
    // keeps the established proposal path available while we compare traces
    // and costs under live traffic.
    if std::env::var("NPC_STRATEGIST_AGENTIC_LOOP_ENABLED")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
    {
        let agentic_timeout = std::env::var("NPC_STRATEGIST_AGENTIC_TIMEOUT_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map_or(Duration::from_secs(120), Duration::from_millis);
        let session = RigStrategicAgent::new(
            config.openrouter_api_key()?,
            &character.strategist_model,
            include_str!("../brain/prompts/strategist_agentic_v1.txt"),
            conversation_memory.clone(),
            format!("{}-strategist-agentic-v1", character.id),
            8,
        )?
        .with_checkpoint_timeout(agentic_timeout)
        .with_observability(
            character.id.clone(),
            analytics.clone(),
            usage_ledger,
            "strategist/agentic-v1",
        );
        return Ok(Some(Arc::new(RigStrategicProposalBrain::new(session))));
    }
    let mut brain = OpenRouterJsonBrain::<StrategicInput, StrategicProposal>::new_observed(
        config.openrouter_api_key()?,
        &character.strategist_model,
        STRATEGIST_V6,
        config.models.strategist_temperature,
        config.models.strategist_max_output_tokens,
        ModelCallObservability::new(STRATEGIST_V6_VERSION, analytics.clone())
            .with_role("strategist")
            .with_usage_ledger(usage_ledger)
            .with_background_tasks(background_tasks),
    )?
    .with_reasoning(config.models.strategist_reasoning)?
    .with_local_input_capture(config.models.local_input_capture.clone())
    .with_prompt_caching(config.models.prompt_caching_enabled)
    .with_session_id(format!("{}-strategist", character.id))
    .with_request_timeout(config.models.strategist_request_timeout);
    record_brain_configuration(
        analytics,
        character,
        "strategist",
        &character.strategist_model,
        STRATEGIST_V6_VERSION,
        config.models.strategist_max_output_tokens,
        config.models.strategist_request_timeout,
        config.models.strategist_reasoning,
        config.models.prompt_caching_enabled,
    );
    if let Some(memory) = conversation_memory {
        // Retain older prompt transcripts for audit without loading stale
        // control output into the current strategic contract.
        brain = brain.with_conversation_memory(memory, "strategist-v6");
    }
    Ok(Some(Arc::new(brain)))
}

#[allow(
    clippy::too_many_arguments,
    reason = "a single event makes the complete per-brain model policy auditable at startup"
)]
fn record_brain_configuration(
    analytics: &Arc<dyn AnalyticsSink>,
    character: &CharacterSheet,
    cognitive_role: &'static str,
    requested_model: &str,
    prompt_version: &str,
    max_output_tokens: u64,
    request_timeout: Duration,
    reasoning: ModelReasoningConfig,
    prompt_caching_enabled: bool,
) {
    analytics.record(
        AnalyticsEvent::new("model.brain_configured", EventLevel::Info)
            .character(&character.id)
            .attribute("provider", "openrouter")
            .attribute("cognitive_role", cognitive_role)
            .attribute("requested_model", requested_model)
            .attribute("prompt_version", prompt_version)
            .attribute("requested_max_output_tokens", max_output_tokens)
            .attribute(
                "request_timeout_ms",
                u64::try_from(request_timeout.as_millis()).unwrap_or(u64::MAX),
            )
            .attribute("reasoning_requested_enabled", reasoning.enabled)
            .attribute("reasoning_requested_effort", reasoning.effort.as_str())
            .attribute("reasoning_response_excluded", reasoning.exclude)
            .attribute("reasoning_content_recorded", false)
            .attribute("cache_control_enabled", prompt_caching_enabled)
            .attribute("cache_control_supported_by_rig", true)
            .attribute("session_stickiness_enabled", true)
            .attribute("session_id", format!("{}-{cognitive_role}", character.id)),
    );
}

fn start_session_monitor(
    mut events: tokio::sync::broadcast::Receiver<SessionEvent>,
    supervisor: ActorRef<PlayerSupervisorMsg>,
    analytics: Arc<dyn AnalyticsSink>,
    character_id: String,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut latest_generation = 0_u64;
        loop {
            match events.recv().await {
                Ok(
                    SessionEvent::Connected { generation, .. }
                    | SessionEvent::Reconnected { generation, .. },
                ) => {
                    latest_generation = generation;
                }
                Ok(SessionEvent::DecisionsInvalidated { generation, reason }) => {
                    latest_generation = generation;
                    if supervisor
                        .send_message(PlayerSupervisorMsg::SessionInvalidated {
                            generation,
                            reason,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(SessionEvent::Disconnected { generation }) => {
                    latest_generation = generation;
                    if supervisor
                        .send_message(PlayerSupervisorMsg::SessionInvalidated {
                            generation,
                            reason: "mcp_session_disconnected".to_owned(),
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    analytics.record(
                        AnalyticsEvent::new("runtime.session_event_lagged", EventLevel::Warn)
                            .character(&character_id)
                            .attribute("skipped", skipped)
                            .attribute("latest_generation", latest_generation),
                    );
                    let _ = supervisor.send_message(PlayerSupervisorMsg::SessionInvalidated {
                        generation: latest_generation,
                        reason: "session_event_lagged".to_owned(),
                    });
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex};

    use async_trait::async_trait;

    use crate::{
        brain::tactical_frame::TacticalFrame,
        execution::{
            gateway::{
                BodyCommand, BodyCommandResult, BodyGateway, BodyGatewayError, ExecutionContext,
            },
            packet::{TacticalAction, TacticalIntent, TacticalMode, TacticalStyle},
        },
        memory::working::{Goal, PlanStep, TodoItem},
        observability::RecordingAnalyticsSink,
        runtime::control_gate::{ControlledPacketError, ControlledPacketRequest},
    };

    use super::*;

    fn config() -> HarnessConfig {
        let values = HashMap::from([
            ("ARENA_API_KEY", "test-arena-key"),
            ("OPENROUTER_API_KEY", "test-router-key"),
            (
                "NPC_CHARACTER_SHEET_PATH",
                concat!(env!("CARGO_MANIFEST_DIR"), "/characters/guy.json"),
            ),
        ]);
        let mut config = HarnessConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
            .expect("test configuration");
        config.memory_path = ":memory:".into();
        config
    }

    #[test]
    fn persisted_working_memory_seeds_the_initial_strategy() {
        let config = config();
        let character = config.character_sheet().expect("Guy sheet");
        let working = WorkingMemory {
            goal: Some(Goal {
                aim: "Find the source of the lighthouse signal.".to_owned(),
                done: None,
                why: Some("The signal may identify the missing sailor.".to_owned()),
            }),
            plan: vec![
                PlanStep {
                    step_id: Some(uuid::Uuid::new_v4()),
                    what: "Ask the harbor keeper about the signal.".to_owned(),
                    status: WorkStatus::Doing,
                    note: None,
                    tries: 1,
                    done_when: None,
                    evidence: Vec::new(),
                    reevaluate_when: Vec::new(),
                },
                PlanStep {
                    step_id: Some(uuid::Uuid::new_v4()),
                    what: "Buy supplies.".to_owned(),
                    status: WorkStatus::Done,
                    note: None,
                    tries: 1,
                    done_when: None,
                    evidence: Vec::new(),
                    reevaluate_when: Vec::new(),
                },
            ],
            todo: vec![
                TodoItem {
                    what: "Return the borrowed compass.".to_owned(),
                    status: WorkStatus::Next,
                    note: None,
                    asked_by: Some("the cartographer".to_owned()),
                },
                TodoItem {
                    what: "Repair the old cloak.".to_owned(),
                    status: WorkStatus::Done,
                    note: None,
                    asked_by: None,
                },
            ],
            notes: Vec::new(),
            plan_revision: 0,
            progress_summary: String::new(),
            reevaluate_when: Vec::new(),
            blocked_reason: None,
            goal_complete: false,
            strategic_intent: None,
        };

        let strategy = initial_strategy(&character, &working);

        assert_eq!(strategy.objective, working.goal.expect("goal").aim);
        assert_eq!(
            strategy.subgoals,
            [
                "Ask the harbor keeper about the signal.",
                "Return the borrowed compass."
            ]
        );
        assert_eq!(strategy.revision, 1);
    }

    #[test]
    fn blocked_legacy_work_does_not_reenter_active_strategy() {
        let config = config();
        let character = config.character_sheet().expect("Guy sheet");
        let working = WorkingMemory {
            plan: vec![PlanStep {
                step_id: Some(uuid::Uuid::new_v4()),
                what: "Walk back to the obsolete inn doorway.".to_owned(),
                status: WorkStatus::Blocked,
                note: Some("This route failed repeatedly.".to_owned()),
                tries: 6,
                done_when: None,
                evidence: Vec::new(),
                reevaluate_when: Vec::new(),
            }],
            todo: vec![TodoItem {
                what: "Wait for an unavailable person.".to_owned(),
                status: WorkStatus::Blocked,
                note: None,
                asked_by: None,
            }],
            ..WorkingMemory::default()
        };

        let strategy = initial_strategy(&character, &working);

        assert!(strategy.subgoals.is_empty());
    }

    #[test]
    fn accepted_durable_strategy_wins_over_legacy_goal_and_plan() {
        let config = config();
        let character = config.character_sheet().expect("Guy sheet");
        let accepted = StrategicIntent {
            revision: 14,
            objective: "Explore the bot forest and learn what lives there.".to_owned(),
            navigation_goal: Some(crate::brain::strategic_intent::NavigationGoal {
                scene: "bot-forest".to_owned(),
                destination: None,
                reason: "Find grounded opportunities.".to_owned(),
            }),
            ..StrategicIntent::default()
        };
        let working = WorkingMemory {
            goal: Some(Goal {
                aim: "Old broad goal".to_owned(),
                done: None,
                why: None,
            }),
            strategic_intent: Some(accepted.clone()),
            ..WorkingMemory::default()
        };

        assert_eq!(initial_strategy(&character, &working), accepted);
    }

    #[test]
    fn character_sheet_goal_is_used_when_durable_memory_has_no_goal() {
        let config = config();
        let character = config.character_sheet().expect("Guy sheet");
        let expected = character
            .initial_goal
            .as_ref()
            .expect("Guy has an initial goal")
            .aim
            .clone();

        let strategy = initial_strategy(&character, &WorkingMemory::default());

        assert_eq!(strategy.objective, expected);
        assert!(strategy.subgoals.is_empty());
    }

    #[test]
    fn blank_migrated_goal_does_not_create_an_empty_strategy() {
        let config = config();
        let mut character = config.character_sheet().expect("Guy sheet");
        character.initial_goal = None;
        let working = WorkingMemory {
            goal: Some(Goal {
                aim: "  ".to_owned(),
                done: None,
                why: None,
            }),
            ..WorkingMemory::default()
        };

        let strategy = initial_strategy(&character, &working);

        assert_eq!(
            strategy.objective,
            "Live according to the character's identity."
        );
    }

    #[derive(Default)]
    struct RecordingBodyGateway {
        calls: Mutex<Vec<(BodyCommand, ExecutionContext)>>,
    }

    #[async_trait]
    impl BodyGateway for RecordingBodyGateway {
        async fn execute(
            &self,
            command: BodyCommand,
            context: ExecutionContext,
        ) -> Result<BodyCommandResult, BodyGatewayError> {
            self.calls.lock().expect("calls").push((command, context));
            Ok(BodyCommandResult {
                accepted: Some(true),
                ..BodyCommandResult::default()
            })
        }
    }

    #[tokio::test]
    async fn starts_every_actor_and_stops_cleanly() {
        let config = config();
        let character = config.character_sheet().expect("Guy sheet");
        let runtime = PlayerRuntime::start(config, character)
            .await
            .expect("runtime starts");

        let status = runtime.status().await.expect("runtime health");
        for actor in [
            crate::runtime::messages::ActorKind::Body,
            crate::runtime::messages::ActorKind::Perception,
            crate::runtime::messages::ActorKind::Tactician,
            crate::runtime::messages::ActorKind::Strategist,
            crate::runtime::messages::ActorKind::Memory,
            crate::runtime::messages::ActorKind::Telemetry,
        ] {
            assert!(status.is_running(actor), "{actor:?} should be running");
        }

        let summary = runtime
            .shutdown_with_reason("unit_test_complete")
            .await
            .expect("clean shutdown");
        assert_eq!(summary.character_id, "guy");
        assert_eq!(summary.shutdown_reason, "unit_test_complete");
        assert_eq!(summary.total_usage.calls, 0);
        assert!(summary.total_usage.exact_cost_usd.abs() < f64::EPSILON);
        assert!(summary.telemetry.events_recorded >= 6);
        assert!(!summary.body.connected);
    }

    #[tokio::test]
    async fn accounted_model_response_wakes_the_runtime_cost_stop_without_a_new_frame() {
        let values = HashMap::from([
            ("ARENA_API_KEY", "test-arena-key"),
            ("NPC_RUN_MAX_OPENROUTER_COST_USD", "0.01"),
            (
                "NPC_CHARACTER_SHEET_PATH",
                concat!(env!("CARGO_MANIFEST_DIR"), "/characters/guy.json"),
            ),
        ]);
        let mut config = HarnessConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
            .expect("cost-bounded configuration");
        config.memory_path = ":memory:".into();
        let character = config.character_sheet().expect("Guy sheet");
        let runtime = PlayerRuntime::start(config, character)
            .await
            .expect("runtime starts");

        {
            let safety = runtime.wait_for_safety_stop();
            tokio::pin!(safety);
            assert!(
                tokio::time::timeout(Duration::from_millis(20), &mut safety)
                    .await
                    .is_err()
            );
            runtime.tactician_usage.record_test_usage("guy", Some(0.01));
            assert_eq!(
                tokio::time::timeout(Duration::from_millis(100), &mut safety)
                    .await
                    .expect("ledger update wakes safety monitor"),
                RuntimeSafetyStop::ModelCostLimitExceeded
            );
        }

        runtime.shutdown().await.expect("clean shutdown");
    }

    #[tokio::test]
    async fn supervisor_observes_and_restarts_a_failed_tactician() {
        let config = config();
        let character = config.character_sheet().expect("Guy sheet");
        let runtime = PlayerRuntime::start(config, character)
            .await
            .expect("runtime starts");

        runtime
            .supervisor
            .send_message(PlayerSupervisorMsg::FailTacticianForTest)
            .expect("inject tactician failure");

        let mut observed = false;
        for _ in 0..50 {
            let status = runtime.status().await.expect("runtime health");
            if status.failures_observed == 1
                && status.is_running(crate::runtime::messages::ActorKind::Tactician)
            {
                observed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(observed, "supervisor should restart the failed tactician");

        runtime.shutdown().await.expect("clean shutdown");
    }

    fn controlled_request(character: &CharacterSheet) -> ControlledPacketRequest {
        ControlledPacketRequest {
            expected_character_id: character.id.clone(),
            expected_player_name: character.player_name.clone(),
            expected_scene: "diagnostic-arena".to_owned(),
            proposal: TacticalProposal {
                intent: TacticalIntent::Stop,
                actions: vec![TacticalAction::Stop],
                valid_for_ms: 500,
                abort_if: Vec::new(),
                rationale: Some("controlled gate test".to_owned()),
            },
        }
    }

    #[tokio::test]
    async fn validation_is_read_only_and_controlled_submission_consumes_budget_once() {
        let mut config = config();
        config.runtime.tactical_rollout_mode =
            crate::runtime::tactical_schedule::TacticalRolloutMode::Controlled;
        config.runtime.allow_live_mutation = true;
        config.runtime.live_action_budget = crate::config::LiveActionBudget::Limited(1);
        config.runtime.live_allowed_scene = Some("diagnostic-arena".to_owned());
        let character = config.character_sheet().expect("Guy sheet");
        config.runtime.live_expected_character_id = Some(character.id.clone());
        config.runtime.live_expected_player_name = Some(character.player_name.clone());
        let request = controlled_request(&character);
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let runtime = PlayerRuntime::start_with_analytics(config, character, analytics.clone())
            .await
            .expect("runtime starts");
        let mut frame = TacticalFrame::empty(runtime.blackboard.strategy().as_ref().clone());
        frame.revision = 1;
        frame.perception_revision = 1;
        frame.self_state.alive = Some(true);
        frame.self_state.scene = Some("diagnostic-arena".to_owned());
        assert!(runtime.blackboard.publish_frame(Arc::new(frame)));

        let validation = runtime
            .validate_tactical_packet(request.clone())
            .await
            .expect("BodyActor validation accepts");
        assert!(!validation.released);
        assert_eq!(validation.remaining_live_action_budget, Some(1));

        let submission = runtime
            .submit_controlled_packet(request.clone())
            .await
            .expect("single controlled release");
        assert!(submission.released);
        assert_eq!(submission.remaining_live_action_budget, Some(0));
        assert!(matches!(
            runtime.submit_controlled_packet(request).await,
            Err(ControlledPacketError::GateDisabled)
        ));
        let decisions = analytics
            .events()
            .into_iter()
            .filter(|event| event.name == "runtime.controlled_packet_decided")
            .collect::<Vec<_>>();
        assert_eq!(decisions.len(), 3);
        assert_eq!(
            decisions[1].attributes.get("reason_code"),
            Some(&serde_json::Value::String("released".to_owned()))
        );
        assert_eq!(
            decisions[2].attributes.get("reason_code"),
            Some(&serde_json::Value::String("gate_disabled".to_owned()))
        );

        runtime.shutdown().await.expect("clean shutdown");
    }

    #[tokio::test]
    async fn full_model_packets_use_exact_assertions_and_the_shared_action_budget() {
        let mut config = config();
        config.runtime.tactical_rollout_mode =
            crate::runtime::tactical_schedule::TacticalRolloutMode::Full;
        config.runtime.allow_live_mutation = true;
        config.runtime.live_action_budget = crate::config::LiveActionBudget::Limited(1);
        config.runtime.live_allowed_scene = Some("diagnostic-arena".to_owned());
        let character = config.character_sheet().expect("Guy sheet");
        config.runtime.live_expected_character_id = Some(character.id.clone());
        config.runtime.live_expected_player_name = Some(character.player_name.clone());
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let runtime = PlayerRuntime::start_with_analytics(config, character, analytics.clone())
            .await
            .expect("runtime starts");
        let mut frame = TacticalFrame::empty(runtime.blackboard.strategy().as_ref().clone());
        frame.revision = 1;
        frame.perception_revision = 1;
        frame.self_state.alive = Some(true);
        frame.self_state.scene = Some("diagnostic-arena".to_owned());
        assert!(runtime.blackboard.publish_frame(Arc::new(frame.clone())));
        let proposal = TacticalProposal {
            intent: TacticalIntent::Stop,
            actions: vec![TacticalAction::Stop],
            valid_for_ms: 500,
            abort_if: Vec::new(),
            rationale: None,
        };
        let packet = crate::execution::packet::ActionPacket::from_proposal(
            uuid::Uuid::new_v4(),
            frame.revision,
            frame.strategic_intent.revision,
            frame.self_state.scene.clone(),
            proposal,
        );

        runtime
            .supervisor
            .send_message(PlayerSupervisorMsg::SubmitModelPacket(packet.clone()))
            .expect("first full packet reaches supervisor");
        runtime
            .supervisor
            .send_message(PlayerSupervisorMsg::SubmitModelPacket(packet))
            .expect("second full packet reaches supervisor");
        for _ in 0..100 {
            if analytics
                .events()
                .iter()
                .filter(|event| event.name == "runtime.model_packet_decided")
                .count()
                == 2
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let decisions = analytics
            .events()
            .into_iter()
            .filter(|event| event.name == "runtime.model_packet_decided")
            .collect::<Vec<_>>();
        assert_eq!(decisions.len(), 2);
        assert_eq!(decisions[0].attributes["released"], true);
        assert_eq!(decisions[0].attributes["remaining_live_action_budget"], 0);
        assert_eq!(decisions[1].attributes["released"], false);
        assert_eq!(decisions[1].attributes["reason_code"], "gate_disabled");

        runtime.shutdown().await.expect("clean shutdown");
    }

    #[tokio::test]
    async fn full_live_runtime_reports_blind_combat_and_requests_a_safety_stop() {
        let mut config = config();
        config.runtime.tactical_rollout_mode =
            crate::runtime::tactical_schedule::TacticalRolloutMode::Full;
        config.runtime.allow_live_mutation = true;
        let character = config.character_sheet().expect("Guy sheet");
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let runtime = PlayerRuntime::start_with_analytics(config, character, analytics.clone())
            .await
            .expect("runtime starts");
        let mut frame = TacticalFrame::empty(runtime.blackboard.strategy().as_ref().clone());
        frame.revision = 1;
        frame.perception_revision = 1;
        frame.combat.active = Some(true);
        frame.self_state.health = None;
        frame.self_state.max_health = None;
        assert!(runtime.blackboard.publish_frame(Arc::new(frame)));

        let stop = tokio::time::timeout(Duration::from_millis(100), runtime.wait_for_safety_stop())
            .await
            .expect("safety stop is prompt");
        assert_eq!(stop, RuntimeSafetyStop::CombatHealthUnknown);
        let event = analytics
            .events()
            .into_iter()
            .find(|event| event.name == "runtime.safety_stop_triggered")
            .expect("safety event");
        assert_eq!(event.attributes["reason_code"], "combat_health_unknown");
        assert_eq!(event.attributes["health_known"], false);
        assert_eq!(event.attributes["max_health_known"], false);

        runtime.shutdown().await.expect("clean shutdown");
    }

    #[tokio::test]
    async fn safety_stop_routes_flee_through_supervisor_and_body() {
        let mut config = config();
        config.runtime.tactical_rollout_mode =
            crate::runtime::tactical_schedule::TacticalRolloutMode::Full;
        config.runtime.allow_live_mutation = true;
        let character = config.character_sheet().expect("Guy sheet");
        let gateway = Arc::new(RecordingBodyGateway::default());
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let runtime = PlayerRuntime::start_with_dependencies(
            config,
            character,
            gateway.clone(),
            7,
            true,
            None,
            None,
            analytics.clone(),
        )
        .await
        .expect("runtime starts");
        let mut frame = TacticalFrame::empty(runtime.blackboard.strategy().as_ref().clone());
        frame.revision = 1;
        frame.perception_revision = 1;
        frame.combat.active = Some(true);
        assert!(runtime.blackboard.publish_frame(Arc::new(frame)));

        let stop = runtime.wait_for_safety_stop().await;
        let result = runtime
            .activate_safety_fallback(stop)
            .await
            .expect("fallback succeeds");

        assert_eq!(
            result.status,
            crate::execution::outcome::OutcomeStatus::Succeeded
        );
        {
            let calls = gateway.calls.lock().expect("calls");
            assert_eq!(calls.len(), 1);
            assert_eq!(
                calls[0].0,
                BodyCommand::SetTactics {
                    style: TacticalStyle::Flee,
                    mode: TacticalMode::SemiAuto,
                }
            );
            assert_eq!(calls[0].1.session_generation, 7);
        }
        let events = analytics.events();
        assert!(
            events
                .iter()
                .any(|event| event.name == "runtime.safety_fallback_started")
        );
        assert!(
            events
                .iter()
                .any(|event| event.name == "runtime.safety_fallback_completed")
        );

        runtime.shutdown().await.expect("clean shutdown");
    }
}
