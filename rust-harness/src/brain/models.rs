use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    future::Future,
    io::Write,
    marker::PhantomData,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

use async_trait::async_trait;
use num_traits::ToPrimitive;
use rig_core::{
    client::CompletionClient,
    completion::{AssistantContent, CompletionError, CompletionModel, Message},
    memory::ConversationMemory,
    providers::openrouter,
};
use schemars::JsonSchema;
use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

use super::{
    Brain, BrainCallContext,
    openrouter_accounting::{OpenRouterAccountingClient, record_generation, record_price_snapshot},
};
use crate::config::{ModelReasoningConfig, ReasoningEffort};
use crate::observability::{AnalyticsEvent, AnalyticsSink, EventLevel, tracing_sink};

#[derive(Clone)]
pub struct ModelCallObservability {
    pub prompt_version: String,
    pub cognitive_role: String,
    pub analytics: Arc<dyn AnalyticsSink>,
    pub usage_ledger: Arc<ModelUsageLedger>,
    pub background_tasks: Arc<ModelBackgroundTasks>,
}

impl ModelCallObservability {
    #[must_use]
    pub fn new(prompt_version: impl Into<String>, analytics: Arc<dyn AnalyticsSink>) -> Self {
        Self {
            prompt_version: prompt_version.into(),
            cognitive_role: "unspecified".to_owned(),
            analytics,
            usage_ledger: Arc::new(ModelUsageLedger::default()),
            background_tasks: Arc::new(ModelBackgroundTasks::default()),
        }
    }

    #[must_use]
    pub fn with_role(mut self, cognitive_role: impl Into<String>) -> Self {
        self.cognitive_role = cognitive_role.into();
        self
    }

    #[must_use]
    pub fn with_usage_ledger(mut self, usage_ledger: Arc<ModelUsageLedger>) -> Self {
        self.usage_ledger = usage_ledger;
        self
    }

    #[must_use]
    pub fn with_background_tasks(mut self, tasks: Arc<ModelBackgroundTasks>) -> Self {
        self.background_tasks = tasks;
        self
    }
}

/// Tracks provider-accounting work that must finish or terminate explicitly.
#[derive(Default)]
pub struct ModelBackgroundTasks {
    handles: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    active_model_calls: AtomicUsize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModelBackgroundDrain {
    pub completed: usize,
    pub failed: usize,
    pub aborted: usize,
    pub active_model_calls_remaining: usize,
}

struct ActiveModelCall {
    tasks: Arc<ModelBackgroundTasks>,
}

impl Drop for ActiveModelCall {
    fn drop(&mut self) {
        self.tasks.active_model_calls.fetch_sub(1, Ordering::AcqRel);
    }
}

struct ModelCallTerminalGuard {
    analytics: Arc<dyn AnalyticsSink>,
    context: BrainCallContext,
    requested_model: String,
    cognitive_role: String,
    prompt_version: String,
    reasoning: ModelReasoningConfig,
    reasoning_support_known: bool,
    started_at: Instant,
    terminal_recorded: bool,
}

impl ModelCallTerminalGuard {
    fn new(
        analytics: Arc<dyn AnalyticsSink>,
        context: BrainCallContext,
        metadata: ModelEventMetadata<'_>,
        started_at: Instant,
    ) -> Self {
        Self {
            analytics,
            context,
            requested_model: metadata.requested_model.to_owned(),
            cognitive_role: metadata.cognitive_role.to_owned(),
            prompt_version: metadata.prompt_version.to_owned(),
            reasoning: metadata.reasoning,
            reasoning_support_known: metadata.reasoning_support_known,
            started_at,
            terminal_recorded: false,
        }
    }

    const fn mark_terminal_recorded(&mut self) {
        self.terminal_recorded = true;
    }
}

impl Drop for ModelCallTerminalGuard {
    fn drop(&mut self) {
        if self.terminal_recorded {
            return;
        }
        self.analytics.record(reasoning_event_attributes(
            model_event("model.call_failed", EventLevel::Warn, &self.context)
                .attribute("provider", "openrouter")
                .attribute("requested_model", self.requested_model.clone())
                .attribute("cognitive_role", self.cognitive_role.clone())
                .attribute("prompt_version", self.prompt_version.clone())
                .attribute("latency_ms", elapsed_ms(self.started_at))
                .attribute("error_class", "cancelled")
                .attribute("response_received", false)
                .attribute("usage_accounted", false),
            self.reasoning,
            self.reasoning_support_known,
        ));
    }
}

impl ModelBackgroundTasks {
    fn begin_model_call(self: &Arc<Self>) -> ActiveModelCall {
        self.active_model_calls.fetch_add(1, Ordering::AcqRel);
        ActiveModelCall {
            tasks: self.clone(),
        }
    }

    fn spawn(&self, future: impl Future<Output = ()> + Send + 'static) {
        let mut handles = self
            .handles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        handles.push(tokio::spawn(future));
    }

    /// Wait for active calls and every accounting task they register.
    ///
    /// Tasks that exceed the shared deadline are explicitly aborted. A model
    /// call that exceeds the deadline is reported in the returned facts, even
    /// though its detached inference task cannot be aborted through this
    /// accounting registry.
    pub async fn drain(&self, timeout: Duration) -> ModelBackgroundDrain {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut result = ModelBackgroundDrain::default();
        loop {
            let handles = {
                let mut guard = self
                    .handles
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                std::mem::take(&mut *guard)
            };
            for mut handle in handles {
                match tokio::time::timeout_at(deadline, &mut handle).await {
                    Ok(Ok(())) => result.completed += 1,
                    Ok(Err(_)) => result.failed += 1,
                    Err(_) => {
                        handle.abort();
                        result.aborted += 1;
                    }
                }
            }

            let active = self.active_model_calls.load(Ordering::Acquire);
            let no_registered_tasks = self
                .handles
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty();
            if active == 0 && no_registered_tasks {
                tokio::task::yield_now().await;
                let still_quiescent = self.active_model_calls.load(Ordering::Acquire) == 0
                    && self
                        .handles
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .is_empty();
                if still_quiescent {
                    break;
                }
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                break;
            }
            tokio::time::sleep(std::cmp::min(
                Duration::from_millis(10),
                deadline.saturating_duration_since(now),
            ))
            .await;
        }
        let late_handles = {
            let mut guard = self
                .handles
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut *guard)
        };
        for handle in late_handles {
            handle.abort();
            result.aborted += 1;
        }
        result.active_model_calls_remaining = self.active_model_calls.load(Ordering::Acquire);
        result
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ModelUsageTotals {
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub tool_use_prompt_tokens: u64,
    pub reasoning_tokens: u64,
    pub exact_cost_known_calls: u64,
    pub exact_cost_usd: f64,
}

/// Process-local running totals, keyed by stable character id.
///
/// The append-only completion events remain the durable source for cross-process totals.
#[derive(Debug)]
pub struct ModelUsageLedger {
    totals: Mutex<HashMap<String, ModelUsageTotals>>,
    revision: AtomicU64,
    revisions: tokio::sync::watch::Sender<u64>,
}

impl Default for ModelUsageLedger {
    fn default() -> Self {
        let (revisions, _) = tokio::sync::watch::channel(0);
        Self {
            totals: Mutex::new(HashMap::new()),
            revision: AtomicU64::new(0),
            revisions,
        }
    }
}

impl ModelUsageLedger {
    fn add(&self, character_id: Option<&str>, metrics: &ModelCallMetrics) -> ModelUsageTotals {
        let key = character_id.unwrap_or("unbound").to_owned();
        let mut guard = self
            .totals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let totals = guard.entry(key).or_default();
        totals.calls = totals.calls.saturating_add(1);
        totals.input_tokens = totals.input_tokens.saturating_add(metrics.input_tokens);
        totals.output_tokens = totals.output_tokens.saturating_add(metrics.output_tokens);
        totals.total_tokens = totals.total_tokens.saturating_add(metrics.total_tokens);
        totals.cached_input_tokens = totals
            .cached_input_tokens
            .saturating_add(metrics.cached_input_tokens);
        totals.cache_creation_input_tokens = totals
            .cache_creation_input_tokens
            .saturating_add(metrics.cache_creation_input_tokens);
        totals.tool_use_prompt_tokens = totals
            .tool_use_prompt_tokens
            .saturating_add(metrics.tool_use_prompt_tokens);
        totals.reasoning_tokens = totals
            .reasoning_tokens
            .saturating_add(metrics.reasoning_tokens);
        if let Some(cost) = metrics.exact_cost_usd {
            totals.exact_cost_known_calls = totals.exact_cost_known_calls.saturating_add(1);
            totals.exact_cost_usd += cost;
        }
        let result = totals.clone();
        drop(guard);
        let revision = self
            .revision
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        self.revisions.send_replace(revision);
        result
    }

    /// Account usage returned by Rig's multi-turn Agent runner. Rig exposes
    /// per-completion token usage but not the provider generation identifier or
    /// billed cost at this layer, so those fields remain explicitly unknown.
    pub(crate) fn record_rig_agent_usage(
        &self,
        character_id: &str,
        model_id: &str,
        _role: &str,
        _prompt_version: &str,
        decision_id: uuid::Uuid,
        calls: &[rig_agent::agent::CompletionCall],
    ) -> ModelUsageTotals {
        let mut totals = ModelUsageTotals::default();
        for call in calls {
            let usage = call.usage;
            totals = self.add(
                Some(character_id),
                &ModelCallMetrics {
                    generation_id: format!("rig-agent-{decision_id}-{}", call.call_index),
                    actual_model: model_id.to_owned(),
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    total_tokens: usage.total_tokens,
                    cached_input_tokens: usage.cached_input_tokens,
                    cache_creation_input_tokens: usage.cache_creation_input_tokens,
                    tool_use_prompt_tokens: usage.tool_use_prompt_tokens,
                    reasoning_tokens: usage.reasoning_tokens,
                    exact_cost_usd: None,
                    requested_max_output_tokens: 0,
                    finish_reason: None,
                    native_finish_reason: None,
                },
            );
        }
        totals
    }

    #[must_use]
    pub fn totals_for(&self, character_id: &str) -> ModelUsageTotals {
        self.totals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(character_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Subscribe to newly accounted provider responses.
    #[must_use]
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.revisions.subscribe()
    }

    fn reconcile_native_usage(
        &self,
        character_id: Option<&str>,
        rig_reasoning_tokens: u64,
        rig_cached_input_tokens: u64,
        native_reasoning_tokens: Option<u64>,
        native_cached_input_tokens: Option<u64>,
    ) -> NativeUsageReconciliation {
        let reasoning_delta = native_reasoning_tokens
            .unwrap_or_default()
            .saturating_sub(rig_reasoning_tokens);
        let cached_delta = native_cached_input_tokens
            .unwrap_or_default()
            .saturating_sub(rig_cached_input_tokens);
        let key = character_id.unwrap_or("unbound").to_owned();
        let mut guard = self
            .totals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let totals = guard.entry(key).or_default();
        totals.reasoning_tokens = totals.reasoning_tokens.saturating_add(reasoning_delta);
        totals.cached_input_tokens = totals.cached_input_tokens.saturating_add(cached_delta);
        let updated_totals = totals.clone();
        drop(guard);
        if reasoning_delta > 0 || cached_delta > 0 {
            let revision = self
                .revision
                .fetch_add(1, Ordering::AcqRel)
                .saturating_add(1);
            self.revisions.send_replace(revision);
        }
        NativeUsageReconciliation {
            reasoning_delta,
            cached_delta,
            updated_totals,
        }
    }

    #[cfg(test)]
    pub(crate) fn record_test_usage(&self, character_id: &str, exact_cost_usd: Option<f64>) {
        self.add(
            Some(character_id),
            &ModelCallMetrics {
                generation_id: "test-generation".to_owned(),
                actual_model: "test-model".to_owned(),
                input_tokens: 10,
                output_tokens: 2,
                total_tokens: 12,
                cached_input_tokens: 0,
                cache_creation_input_tokens: 0,
                tool_use_prompt_tokens: 0,
                reasoning_tokens: 0,
                exact_cost_usd,
                requested_max_output_tokens: 10,
                finish_reason: Some("stop".to_owned()),
                native_finish_reason: Some("stop".to_owned()),
            },
        );
    }
}

struct NativeUsageReconciliation {
    reasoning_delta: u64,
    cached_delta: u64,
    updated_totals: ModelUsageTotals,
}

pub struct OpenRouterJsonBrain<I, O> {
    model: openrouter::CompletionModel,
    preamble: String,
    temperature: f64,
    max_output_tokens: u64,
    request_timeout: Duration,
    model_id: String,
    prompt_version: String,
    cognitive_role: String,
    analytics: Arc<dyn AnalyticsSink>,
    accounting: OpenRouterAccountingClient,
    price_fetch_started: Arc<AtomicBool>,
    usage_ledger: Arc<ModelUsageLedger>,
    background_tasks: Arc<ModelBackgroundTasks>,
    conversation_memory: Option<Arc<dyn ConversationMemory>>,
    conversation_id: Option<String>,
    reasoning: ModelReasoningConfig,
    reasoning_support_known: bool,
    local_input_capture: Option<PathBuf>,
    prompt_caching_enabled: bool,
    session_id: Option<String>,
    marker: PhantomData<fn(&I) -> O>,
}

impl<I, O> Clone for OpenRouterJsonBrain<I, O> {
    fn clone(&self) -> Self {
        Self {
            model: self.model.clone(),
            preamble: self.preamble.clone(),
            temperature: self.temperature,
            max_output_tokens: self.max_output_tokens,
            request_timeout: self.request_timeout,
            model_id: self.model_id.clone(),
            prompt_version: self.prompt_version.clone(),
            cognitive_role: self.cognitive_role.clone(),
            analytics: self.analytics.clone(),
            accounting: self.accounting.clone(),
            price_fetch_started: self.price_fetch_started.clone(),
            usage_ledger: self.usage_ledger.clone(),
            background_tasks: self.background_tasks.clone(),
            conversation_memory: self.conversation_memory.clone(),
            conversation_id: self.conversation_id.clone(),
            reasoning: self.reasoning,
            reasoning_support_known: self.reasoning_support_known,
            local_input_capture: self.local_input_capture.clone(),
            prompt_caching_enabled: self.prompt_caching_enabled,
            session_id: self.session_id.clone(),
            marker: PhantomData,
        }
    }
}

impl<I, O> OpenRouterJsonBrain<I, O> {
    /// Build a stateless structured-output brain backed by `OpenRouter`.
    ///
    /// # Errors
    ///
    /// Returns an error if Rig cannot construct the provider client.
    pub fn new(
        api_key: &str,
        model_id: &str,
        preamble: impl Into<String>,
        temperature: f64,
        max_output_tokens: u64,
    ) -> anyhow::Result<Self> {
        Self::new_observed(
            api_key,
            model_id,
            preamble,
            temperature,
            max_output_tokens,
            ModelCallObservability::new("unspecified", tracing_sink()),
        )
    }

    /// Build a stateless brain with an explicit prompt version and event sink.
    ///
    /// # Errors
    ///
    /// Returns an error if Rig cannot construct the provider client.
    pub fn new_observed(
        api_key: &str,
        model_id: &str,
        preamble: impl Into<String>,
        temperature: f64,
        max_output_tokens: u64,
        observability: ModelCallObservability,
    ) -> anyhow::Result<Self> {
        let client = openrouter::Client::new(api_key)?;
        Ok(Self {
            model: client.completion_model(model_id),
            preamble: preamble.into(),
            temperature,
            max_output_tokens,
            request_timeout: Duration::from_secs(30),
            model_id: model_id.to_owned(),
            prompt_version: observability.prompt_version,
            cognitive_role: observability.cognitive_role,
            analytics: observability.analytics,
            accounting: OpenRouterAccountingClient::new(api_key),
            price_fetch_started: Arc::new(AtomicBool::new(false)),
            usage_ledger: observability.usage_ledger,
            background_tasks: observability.background_tasks,
            conversation_memory: None,
            conversation_id: None,
            reasoning: ModelReasoningConfig {
                enabled: false,
                effort: ReasoningEffort::Minimal,
                exclude: true,
            },
            reasoning_support_known: false,
            local_input_capture: None,
            prompt_caching_enabled: false,
            session_id: None,
            marker: PhantomData,
        })
    }

    #[must_use]
    pub const fn with_request_timeout(mut self, request_timeout: Duration) -> Self {
        self.request_timeout = request_timeout;
        self
    }

    /// Configure `OpenRouter` reasoning through Rig's completion request builder.
    ///
    /// Known live models are checked against their advertised effort levels.
    /// Unknown models remain usable, but `OpenRouter` is instructed to require
    /// provider support so the setting cannot be silently dropped.
    ///
    /// # Errors
    ///
    /// Returns an error when a known mandatory-reasoning model is disabled or
    /// a known model is configured with an unsupported effort.
    pub fn with_reasoning(mut self, reasoning: ModelReasoningConfig) -> anyhow::Result<Self> {
        self.reasoning_support_known = validate_reasoning_for_model(&self.model_id, reasoning)?;
        self.reasoning = reasoning;
        Ok(self)
    }

    /// Enable sensitive logical-request capture for local diagnostics.
    ///
    /// Captures are never emitted through analytics. Each file is created with
    /// mode 0600 inside a directory created with mode 0700 on Unix.
    #[must_use]
    pub fn with_local_input_capture(mut self, directory: Option<PathBuf>) -> Self {
        self.local_input_capture = directory;
        self
    }

    /// Enable Rig's `OpenRouter` cache-control marker on the stable system prefix.
    #[must_use]
    pub fn with_prompt_caching(mut self, enabled: bool) -> Self {
        if enabled {
            self.model = self.model.with_prompt_caching();
        }
        self.prompt_caching_enabled = enabled;
        self
    }

    /// Attach a stable, non-secret `OpenRouter` routing session identifier.
    #[must_use]
    pub fn with_session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Attach Rig's provider-independent conversation-memory boundary.
    ///
    /// The supplied adapter controls persistence and history shaping. The
    /// current input and a successfully parsed assistant response are appended
    /// as one turn, matching Rig Agent memory semantics while retaining this
    /// adapter's exact `OpenRouter` generation accounting.
    #[must_use]
    pub fn with_conversation_memory(
        mut self,
        memory: Arc<dyn ConversationMemory>,
        conversation_id: impl Into<String>,
    ) -> Self {
        self.conversation_memory = Some(memory);
        self.conversation_id = Some(conversation_id.into());
        self
    }

    async fn request_model(
        &self,
        prompt: String,
        history: Vec<Message>,
        context: &BrainCallContext,
    ) -> anyhow::Result<ReceivedModelResponse>
    where
        O: JsonSchema,
    {
        let output_schema = compatible_output_schema::<O>();
        let additional_params =
            reasoning_additional_params(self.reasoning, self.session_id.as_deref());
        self.record_logical_input(
            context,
            &prompt,
            &history,
            &output_schema,
            &additional_params,
        )?;
        let request = self
            .model
            .completion_request(prompt)
            .messages(history)
            .preamble(self.preamble.clone())
            .temperature(self.temperature)
            .max_tokens(self.max_output_tokens)
            .additional_params(additional_params)
            .output_schema(output_schema)
            .record_content_telemetry(false)
            .build();
        let response = self.model.completion(request).await?;
        let raw_usage = response.raw_response.usage.as_ref();
        let finish_reason = response
            .raw_response
            .choices
            .first()
            .and_then(|choice| choice.finish_reason.clone());
        let native_finish_reason = response
            .raw_response
            .choices
            .first()
            .and_then(|choice| choice.native_finish_reason.clone());
        let metrics = ModelCallMetrics {
            generation_id: response.raw_response.id.clone(),
            actual_model: response.raw_response.model.clone(),
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
            total_tokens: response.usage.total_tokens,
            cached_input_tokens: response.usage.cached_input_tokens,
            cache_creation_input_tokens: response.usage.cache_creation_input_tokens,
            tool_use_prompt_tokens: response.usage.tool_use_prompt_tokens,
            reasoning_tokens: response.usage.reasoning_tokens,
            exact_cost_usd: raw_usage.map(|usage| usage.cost),
            requested_max_output_tokens: self.max_output_tokens,
            finish_reason,
            native_finish_reason,
        };
        let output_text = response.choice.iter().find_map(|content| match content {
            AssistantContent::Text(text) => Some(text.text.trim().to_owned()),
            _ => None,
        });
        Ok(ReceivedModelResponse {
            output_text,
            metrics,
        })
    }

    fn record_logical_input(
        &self,
        context: &BrainCallContext,
        prompt: &str,
        history: &[Message],
        output_schema: &schemars::Schema,
        additional_params: &serde_json::Value,
    ) -> anyhow::Result<()> {
        let mut input = LogicalModelInput {
            format_version: 1,
            captured_at_unix_ms: 0,
            cognitive_role: &self.cognitive_role,
            requested_model: &self.model_id,
            prompt_version: &self.prompt_version,
            decision_id: context.decision_id,
            character_id: context.character_id.as_deref(),
            frame_revision: context.frame_revision,
            strategic_revision: context.strategic_revision,
            preamble: &self.preamble,
            bounded_history: history,
            current_typed_input_json: prompt,
            output_schema,
            additional_provider_params: additional_params,
            temperature: self.temperature,
            max_output_tokens: self.max_output_tokens,
        };
        let fingerprint = UuidFingerprint::of(&serde_json::to_vec(&input)?);
        input.captured_at_unix_ms = unix_time_ms();
        let bytes = serde_json::to_vec_pretty(&input)?;
        let local_input_capture_succeeded = if let Some(directory) = &self.local_input_capture {
            if write_private_capture(
                directory,
                &format!(
                    "{}-{}-{}.json",
                    self.cognitive_role, context.decision_id, fingerprint
                ),
                &bytes,
            )
            .is_err()
            {
                self.analytics.record(
                    model_event("model.input_capture_failed", EventLevel::Warn, context)
                        .attribute("provider", "openrouter")
                        .attribute("requested_model", self.model_id.clone())
                        .attribute("cognitive_role", self.cognitive_role.clone())
                        .attribute("prompt_version", self.prompt_version.clone())
                        .attribute("request_fingerprint", fingerprint.to_string())
                        .attribute("error_class", "local_file_write"),
                );
                anyhow::bail!("local model input capture failed");
            }
            true
        } else {
            false
        };
        self.analytics.record(
            model_event("model.input_assembled", EventLevel::Debug, context)
                .attribute("provider", "openrouter")
                .attribute("requested_model", self.model_id.clone())
                .attribute("cognitive_role", self.cognitive_role.clone())
                .attribute("prompt_version", self.prompt_version.clone())
                .attribute("request_fingerprint", fingerprint.to_string())
                .attribute("logical_request_bytes", bytes.len())
                .attribute("logical_request_tokens_estimated", bytes.len().div_ceil(4))
                .attribute(
                    "bounded_history_message_count",
                    u64::try_from(history.len()).unwrap_or(u64::MAX),
                )
                .attribute("current_input_bytes", prompt.len())
                .attribute("preamble_bytes", self.preamble.len())
                .attribute(
                    "stable_prefix_tokens_estimated",
                    self.preamble.len().div_ceil(4),
                )
                .attribute(
                    "cache_prefix_meets_4096_token_threshold",
                    self.preamble.len().div_ceil(4) >= 4_096,
                )
                .attribute("cache_control_enabled", self.prompt_caching_enabled)
                .attribute("cache_control_supported_by_rig", true)
                .attribute("session_stickiness_enabled", self.session_id.is_some())
                .attribute("session_id", self.session_id.as_deref().unwrap_or(""))
                .attribute(
                    "output_schema_bytes",
                    serde_json::to_vec(output_schema).map_or(0, |value| value.len()),
                )
                .attribute(
                    "local_input_capture_enabled",
                    self.local_input_capture.is_some(),
                )
                .attribute(
                    "local_input_capture_succeeded",
                    local_input_capture_succeeded,
                ),
        );
        Ok(())
    }
}

#[derive(Serialize)]
struct LogicalModelInput<'a> {
    format_version: u8,
    captured_at_unix_ms: u64,
    cognitive_role: &'a str,
    requested_model: &'a str,
    prompt_version: &'a str,
    decision_id: uuid::Uuid,
    character_id: Option<&'a str>,
    frame_revision: Option<u64>,
    strategic_revision: Option<u64>,
    preamble: &'a str,
    bounded_history: &'a [Message],
    current_typed_input_json: &'a str,
    output_schema: &'a schemars::Schema,
    additional_provider_params: &'a serde_json::Value,
    temperature: f64,
    max_output_tokens: u64,
}

struct UuidFingerprint(uuid::Uuid);

impl UuidFingerprint {
    fn of(bytes: &[u8]) -> Self {
        Self(uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, bytes))
    }
}

impl std::fmt::Display for UuidFingerprint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, formatter)
    }
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

fn write_private_capture(directory: &Path, file_name: &str, bytes: &[u8]) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700).create(directory)?;
        let path = directory.join(file_name);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(bytes)?;
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(directory)?;
        let path = directory.join(file_name);
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(bytes)?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct KnownReasoningSupport {
    mandatory: bool,
    efforts: &'static [ReasoningEffort],
}

const GPT_OSS_EFFORTS: &[ReasoningEffort] = &[
    ReasoningEffort::Low,
    ReasoningEffort::Medium,
    ReasoningEffort::High,
];
const NEMOTRON_SUPER_EFFORTS: &[ReasoningEffort] = &[ReasoningEffort::Low, ReasoningEffort::Medium];
const GEMINI_FLASH_LITE_EFFORTS: &[ReasoningEffort] = &[
    ReasoningEffort::Minimal,
    ReasoningEffort::Low,
    ReasoningEffort::Medium,
    ReasoningEffort::High,
];

fn known_reasoning_support(model_id: &str) -> Option<KnownReasoningSupport> {
    let base_model = model_id.split(':').next().unwrap_or(model_id);
    match base_model {
        "openai/gpt-oss-120b" | "openai/gpt-oss-20b" => Some(KnownReasoningSupport {
            mandatory: true,
            efforts: GPT_OSS_EFFORTS,
        }),
        "openai/gpt-oss-safeguard-20b" => Some(KnownReasoningSupport {
            mandatory: true,
            // OpenRouter marks reasoning mandatory but currently does not
            // advertise selectable effort levels for this model.
            efforts: &[],
        }),
        "nvidia/nemotron-3-super-120b-a12b" => Some(KnownReasoningSupport {
            mandatory: false,
            efforts: NEMOTRON_SUPER_EFFORTS,
        }),
        "google/gemini-3.1-flash-lite" | "google/gemini-3.1-flash-lite-20260507" => {
            Some(KnownReasoningSupport {
                mandatory: false,
                efforts: GEMINI_FLASH_LITE_EFFORTS,
            })
        }
        _ => None,
    }
}

fn validate_reasoning_for_model(
    model_id: &str,
    reasoning: ModelReasoningConfig,
) -> anyhow::Result<bool> {
    let Some(support) = known_reasoning_support(model_id) else {
        return Ok(false);
    };
    if support.mandatory && !reasoning.enabled {
        anyhow::bail!(
            "model {model_id} requires reasoning; enable it with the role-specific reasoning configuration"
        );
    }
    if reasoning.enabled
        && !support.efforts.is_empty()
        && !support.efforts.contains(&reasoning.effort)
    {
        let allowed = support
            .efforts
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        anyhow::bail!(
            "model {model_id} does not support reasoning effort {}; supported efforts: {allowed}",
            reasoning.effort
        );
    }
    if reasoning.enabled && support.efforts.is_empty() {
        anyhow::bail!(
            "model {model_id} requires reasoning but does not advertise selectable effort levels; explicit effort configuration is unsupported"
        );
    }
    Ok(true)
}

fn reasoning_additional_params(
    reasoning: ModelReasoningConfig,
    session_id: Option<&str>,
) -> serde_json::Value {
    let reasoning = if reasoning.enabled {
        serde_json::json!({
            "effort": reasoning.effort.as_str(),
            "exclude": reasoning.exclude,
        })
    } else {
        serde_json::json!({
            "enabled": false,
            "exclude": reasoning.exclude,
        })
    };
    let mut params = serde_json::json!({
        "reasoning": reasoning,
        // Rig exposes OpenRouter's provider policy in the same additional
        // parameter seam. This makes unsupported provider parameters fail
        // instead of being silently omitted by a routed endpoint.
        "provider": {
            "require_parameters": true,
        },
    });
    if let Some(session_id) = session_id {
        params["session_id"] = session_id.into();
    }
    params
}

fn compatible_output_schema<O: JsonSchema>() -> schemars::Schema {
    let mut schema = schemars::schema_for!(O);
    if let Some(object) = schema.as_object_mut() {
        sanitize_schema_object(object);
    }
    schema
}

fn sanitize_schema_object(object: &mut serde_json::Map<String, serde_json::Value>) {
    let format_allowed = match object.get("type") {
        Some(serde_json::Value::String(kind)) => kind == "string",
        Some(serde_json::Value::Array(kinds)) => kinds.iter().any(|kind| kind == "string"),
        _ => false,
    };
    if !format_allowed {
        object.remove("format");
    }
    for value in object.values_mut() {
        sanitize_schema_value(value);
    }
}

fn sanitize_schema_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => sanitize_schema_object(object),
        serde_json::Value::Array(values) => {
            for value in values {
                sanitize_schema_value(value);
            }
        }
        _ => {}
    }
}

struct ReceivedModelResponse {
    output_text: Option<String>,
    metrics: ModelCallMetrics,
}

#[derive(Debug, Error)]
enum ModelOutputError {
    #[error("model returned no text content")]
    MissingText,
    #[error("model output was not valid structured JSON: {source}")]
    InvalidJson {
        #[source]
        source: serde_json::Error,
    },
}

impl ModelOutputError {
    const fn class(&self) -> &'static str {
        match self {
            Self::MissingText => "missing_text",
            Self::InvalidJson { .. } => "invalid_json",
        }
    }

    fn json_diagnostics(&self) -> Option<(&'static str, usize, usize)> {
        match self {
            Self::MissingText => None,
            Self::InvalidJson { source } => Some((
                match source.classify() {
                    serde_json::error::Category::Io => "io",
                    serde_json::error::Category::Syntax => "syntax",
                    serde_json::error::Category::Data => "data",
                    serde_json::error::Category::Eof => "eof",
                },
                source.line(),
                source.column(),
            )),
        }
    }

    fn schema_error_kind(&self) -> &'static str {
        let Self::InvalidJson { source } = self else {
            return "missing_text";
        };
        let message = source.to_string();
        if message.contains("missing field") {
            "missing_field"
        } else if message.contains("unknown variant") {
            "unknown_variant"
        } else if message.contains("invalid type") {
            "invalid_type"
        } else if message.contains("unknown field") {
            "unknown_field"
        } else if source.classify() == serde_json::error::Category::Data {
            "schema_data"
        } else {
            "json_syntax"
        }
    }
}

#[derive(Debug, Error)]
#[error("model request exceeded its {timeout_ms} ms deadline")]
struct ModelRequestTimeout {
    timeout_ms: u64,
}

async fn time_bound_model_request<F, T>(
    timeout: Duration,
    future: F,
) -> Result<T, ModelRequestTimeout>
where
    F: Future<Output = T>,
{
    tokio::time::timeout(timeout, future)
        .await
        .map_err(|_| ModelRequestTimeout {
            timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
        })
}

fn parse_model_output<O: DeserializeOwned>(text: Option<&str>) -> Result<O, ModelOutputError> {
    let text = text.ok_or(ModelOutputError::MissingText)?;
    let json = text.trim();
    serde_json::from_str(json).map_err(|source| ModelOutputError::InvalidJson { source })
}

fn safe_output_shape(output_text: Option<&str>) -> Vec<(&'static str, serde_json::Value)> {
    let Some(text) = output_text else {
        return Vec::new();
    };
    let json_text = text.trim();
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json_text) else {
        return vec![("untyped_json_valid", false.into())];
    };
    let mut facts = vec![
        ("untyped_json_valid", true.into()),
        ("root_json_kind", json_kind(&value).into()),
    ];
    let Some(object) = value.as_object() else {
        return facts;
    };
    facts.push((
        "root_field_count",
        u64::try_from(object.len()).unwrap_or(u64::MAX).into(),
    ));
    for (field, key) in [
        ("intent", "intent_json_kind"),
        ("actions", "actions_json_kind"),
        ("valid_for_ms", "valid_for_ms_json_kind"),
        ("abort_if", "abort_if_json_kind"),
        ("rationale", "rationale_json_kind"),
        ("objective", "objective_json_kind"),
        ("subgoals", "subgoals_json_kind"),
        ("priorities", "priorities_json_kind"),
        ("constraints", "constraints_json_kind"),
        ("risk_tolerance", "risk_tolerance_json_kind"),
        ("preferred_targets", "preferred_targets_json_kind"),
        ("avoid", "avoid_json_kind"),
        ("navigation_goal", "navigation_goal_json_kind"),
        ("speech", "speech_json_kind"),
        ("expires_at", "expires_at_json_kind"),
    ] {
        facts.push((key, object.get(field).map_or("missing", json_kind).into()));
    }
    if let Some(intent) = object.get("intent").and_then(serde_json::Value::as_str) {
        facts.push(("intent_value", truncate_dimension(intent).into()));
    }
    if let Some(actions) = object.get("actions").and_then(serde_json::Value::as_array) {
        facts.push((
            "actions_count",
            u64::try_from(actions.len()).unwrap_or(u64::MAX).into(),
        ));
        let action_types = actions
            .iter()
            .filter_map(|action| action.get("type").and_then(serde_json::Value::as_str))
            .take(5)
            .map(truncate_dimension)
            .collect::<Vec<_>>()
            .join(",");
        facts.push(("action_type_values", action_types.into()));
    }
    facts
}

const fn json_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

fn truncate_dimension(value: &str) -> String {
    value.chars().take(40).collect()
}

#[derive(Clone, Copy)]
struct ModelEventMetadata<'a> {
    requested_model: &'a str,
    cognitive_role: &'a str,
    prompt_version: &'a str,
    reasoning: ModelReasoningConfig,
    reasoning_support_known: bool,
}

struct ModelCallMetrics {
    generation_id: String,
    actual_model: String,
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    cached_input_tokens: u64,
    cache_creation_input_tokens: u64,
    tool_use_prompt_tokens: u64,
    reasoning_tokens: u64,
    exact_cost_usd: Option<f64>,
    requested_max_output_tokens: u64,
    finish_reason: Option<String>,
    native_finish_reason: Option<String>,
}

#[async_trait]
impl<I, O> Brain<I, O> for OpenRouterJsonBrain<I, O>
where
    I: Serialize + Sync,
    O: DeserializeOwned + JsonSchema + Send + Sync + 'static,
{
    async fn decide(&self, input: &I) -> anyhow::Result<O> {
        self.decide_observed(input, &BrainCallContext::standalone())
            .await
    }

    async fn decide_with_context(
        &self,
        input: &I,
        context: &BrainCallContext,
    ) -> anyhow::Result<O> {
        self.decide_observed(input, context).await
    }
}

impl<I, O> OpenRouterJsonBrain<I, O>
where
    I: Serialize + Sync,
    O: DeserializeOwned + JsonSchema + Send + Sync + 'static,
{
    #[allow(
        clippy::too_many_lines,
        reason = "one linear causal path keeps model request, exact accounting, memory, parsing, and terminal telemetry ordered"
    )]
    async fn decide_observed(&self, input: &I, context: &BrainCallContext) -> anyhow::Result<O> {
        let _active_model_call = self.background_tasks.begin_model_call();
        let started = Instant::now();
        let metadata = ModelEventMetadata {
            requested_model: &self.model_id,
            cognitive_role: &self.cognitive_role,
            prompt_version: &self.prompt_version,
            reasoning: self.reasoning,
            reasoning_support_known: self.reasoning_support_known,
        };
        self.start_price_snapshot(context);
        let prompt = prepare_model_prompt(&self.analytics, input, context, metadata, started)?;
        let mut terminal_guard =
            ModelCallTerminalGuard::new(self.analytics.clone(), context.clone(), metadata, started);
        let history = match self.load_conversation_history().await {
            Ok(history) => history,
            Err(error) => {
                record_memory_failure(
                    &self.analytics,
                    context,
                    metadata,
                    elapsed_ms(started),
                    "load",
                    false,
                    false,
                );
                terminal_guard.mark_terminal_recorded();
                return Err(error);
            }
        };
        match time_bound_model_request(
            self.request_timeout,
            self.request_model(prompt.clone(), history, context),
        )
        .await
        {
            Ok(Ok(response)) => {
                let response_received_at = Instant::now();
                let metrics = response.metrics;
                let totals = account_received_response(
                    &self.analytics,
                    &self.usage_ledger,
                    context,
                    metadata,
                    &metrics,
                    elapsed_ms(started),
                );
                record_usage_anomaly(&self.analytics, context, metadata, &metrics);
                self.start_generation_audit(context, metadata, &metrics);

                match parse_model_output(response.output_text.as_deref()) {
                    Ok(output) => {
                        if let Err(error) = self
                            .append_conversation_turn(prompt, response.output_text.as_deref())
                            .await
                        {
                            record_memory_failure(
                                &self.analytics,
                                context,
                                metadata,
                                elapsed_ms(started),
                                "append",
                                true,
                                true,
                            );
                            terminal_guard.mark_terminal_recorded();
                            return Err(error);
                        }
                        record_output_parse_success(
                            &self.analytics,
                            &ModelParseTelemetry {
                                context,
                                metadata,
                                metrics: &metrics,
                                parse_latency_ms: elapsed_ms(response_received_at),
                                call_latency_ms: elapsed_ms(started),
                            },
                            &totals,
                        );
                        terminal_guard.mark_terminal_recorded();
                        Ok(output)
                    }
                    Err(error) => {
                        record_output_parse_failure(
                            &self.analytics,
                            &ModelParseTelemetry {
                                context,
                                metadata,
                                metrics: &metrics,
                                parse_latency_ms: elapsed_ms(response_received_at),
                                call_latency_ms: elapsed_ms(started),
                            },
                            &error,
                            response.output_text.as_deref(),
                        );
                        terminal_guard.mark_terminal_recorded();
                        Err(error.into())
                    }
                }
            }
            Ok(Err(error)) => {
                let failure = classify_request_failure(&error);
                record_request_failure(
                    &self.analytics,
                    context,
                    metadata,
                    elapsed_ms(started),
                    &failure,
                );
                terminal_guard.mark_terminal_recorded();
                Err(error)
            }
            Err(error) => {
                record_request_failure(
                    &self.analytics,
                    context,
                    metadata,
                    elapsed_ms(started),
                    &RequestFailureFacts::timeout(error.timeout_ms),
                );
                terminal_guard.mark_terminal_recorded();
                Err(error.into())
            }
        }
    }

    async fn load_conversation_history(&self) -> anyhow::Result<Vec<Message>> {
        let (Some(memory), Some(conversation_id)) =
            (&self.conversation_memory, &self.conversation_id)
        else {
            return Ok(Vec::new());
        };
        memory
            .load(conversation_id)
            .await
            .map_err(anyhow::Error::from)
    }

    async fn append_conversation_turn(
        &self,
        prompt: String,
        output_text: Option<&str>,
    ) -> anyhow::Result<()> {
        let (Some(memory), Some(conversation_id), Some(output_text)) = (
            &self.conversation_memory,
            &self.conversation_id,
            output_text,
        ) else {
            return Ok(());
        };
        memory
            .append(
                conversation_id,
                vec![Message::user(prompt), Message::assistant(output_text)],
            )
            .await
            .map_err(anyhow::Error::from)
    }

    fn start_price_snapshot(&self, context: &BrainCallContext) {
        if self
            .price_fetch_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let accounting = self.accounting.clone();
        let analytics = self.analytics.clone();
        let model_id = self.model_id.clone();
        let character_id = context.character_id.clone();
        let correlation_id = context.decision_id;
        self.background_tasks.spawn(async move {
            let result = accounting.model_endpoints(&model_id).await;
            record_price_snapshot(
                &analytics,
                character_id.as_deref(),
                correlation_id,
                &model_id,
                &result,
            );
        });
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the asynchronous provider audit and one-time native usage reconciliation share one causal record"
    )]
    fn start_generation_audit(
        &self,
        context: &BrainCallContext,
        metadata: ModelEventMetadata<'_>,
        metrics: &ModelCallMetrics,
    ) {
        let accounting = self.accounting.clone();
        let analytics = self.analytics.clone();
        let character_id = context.character_id.clone();
        let correlation_id = context.decision_id;
        let context = context.clone();
        let requested_model = metadata.requested_model.to_owned();
        let cognitive_role = metadata.cognitive_role.to_owned();
        let prompt_version = metadata.prompt_version.to_owned();
        let reasoning = metadata.reasoning;
        let reasoning_support_known = metadata.reasoning_support_known;
        let generation_id = metrics.generation_id.clone();
        let rig_reasoning_tokens = metrics.reasoning_tokens;
        let rig_cached_input_tokens = metrics.cached_input_tokens;
        let usage_ledger = self.usage_ledger.clone();
        self.background_tasks.spawn(async move {
            const RETRY_DELAYS_MS: [u64; 6] = [250, 750, 2_000, 5_000, 10_000, 30_000];
            let started = Instant::now();
            let mut attempt = 0_u8;
            let result = loop {
                attempt = attempt.saturating_add(1);
                match accounting.generation(&generation_id).await {
                    Ok(record) => break Ok(record),
                    Err(error) if usize::from(attempt) <= RETRY_DELAYS_MS.len() => {
                        let delay = RETRY_DELAYS_MS[usize::from(attempt) - 1];
                        tokio::time::sleep(Duration::from_millis(delay)).await;
                        drop(error);
                    }
                    Err(error) => break Err(error),
                }
            };
            record_generation(
                &analytics,
                character_id.as_deref(),
                correlation_id,
                &generation_id,
                attempt,
                elapsed_ms(started),
                &result,
            );
            if let Ok(record) = &result {
                let reconciliation = usage_ledger.reconcile_native_usage(
                    character_id.as_deref(),
                    rig_reasoning_tokens,
                    rig_cached_input_tokens,
                    record.native_tokens_reasoning,
                    record.native_tokens_cached,
                );
                let metadata = ModelEventMetadata {
                    requested_model: &requested_model,
                    cognitive_role: &cognitive_role,
                    prompt_version: &prompt_version,
                    reasoning,
                    reasoning_support_known,
                };
                analytics.record(
                    model_event_with_metadata(
                        "model.native_usage_reconciled",
                        EventLevel::Info,
                        &context,
                        metadata,
                    )
                    .attribute("provider", "openrouter")
                    .attribute("generation_id", generation_id.clone())
                    .attribute(
                        "actual_provider",
                        record.provider_name.as_deref().unwrap_or(""),
                    )
                    .attribute("actual_model", record.model.as_deref().unwrap_or(""))
                    .attribute("rig_reasoning_tokens", rig_reasoning_tokens)
                    .attribute(
                        "native_reasoning_tokens_known",
                        record.native_tokens_reasoning.is_some(),
                    )
                    .attribute(
                        "native_reasoning_tokens",
                        record.native_tokens_reasoning.unwrap_or_default(),
                    )
                    .attribute(
                        "reasoning_tokens_reconciled_delta",
                        reconciliation.reasoning_delta,
                    )
                    .attribute("rig_cached_input_tokens", rig_cached_input_tokens)
                    .attribute(
                        "native_cached_input_tokens_known",
                        record.native_tokens_cached.is_some(),
                    )
                    .attribute(
                        "native_cached_input_tokens",
                        record.native_tokens_cached.unwrap_or_default(),
                    )
                    .attribute(
                        "cached_input_tokens_reconciled_delta",
                        reconciliation.cached_delta,
                    )
                    .attribute(
                        "agent_reasoning_tokens_total",
                        reconciliation.updated_totals.reasoning_tokens,
                    )
                    .attribute(
                        "agent_cached_input_tokens_total",
                        reconciliation.updated_totals.cached_input_tokens,
                    ),
                );
            }
        });
    }
}

fn record_memory_failure(
    analytics: &Arc<dyn AnalyticsSink>,
    context: &BrainCallContext,
    metadata: ModelEventMetadata<'_>,
    latency_ms: u64,
    operation: &'static str,
    response_received: bool,
    usage_accounted: bool,
) {
    analytics.record(
        model_event_with_metadata("model.call_failed", EventLevel::Warn, context, metadata)
            .attribute("provider", "openrouter")
            .attribute("requested_model", metadata.requested_model)
            .attribute("cognitive_role", metadata.cognitive_role)
            .attribute("prompt_version", metadata.prompt_version)
            .attribute("latency_ms", latency_ms)
            .attribute("error_class", "conversation_memory")
            .attribute("memory_operation", operation)
            .attribute("response_received", response_received)
            .attribute("usage_accounted", usage_accounted),
    );
}

fn prepare_model_prompt<I: Serialize>(
    analytics: &Arc<dyn AnalyticsSink>,
    input: &I,
    context: &BrainCallContext,
    metadata: ModelEventMetadata<'_>,
    started: Instant,
) -> anyhow::Result<String> {
    let prompt = serde_json::to_string(input).inspect_err(|_error| {
        analytics.record(
            model_event_with_metadata("model.call_failed", EventLevel::Warn, context, metadata)
                .attribute("provider", "openrouter")
                .attribute("requested_model", metadata.requested_model)
                .attribute("cognitive_role", metadata.cognitive_role)
                .attribute("prompt_version", metadata.prompt_version)
                .attribute("latency_ms", elapsed_ms(started))
                .attribute("error_class", "input_serialization")
                .attribute("response_received", false)
                .attribute("usage_accounted", false),
        );
    })?;
    analytics.record(
        model_event_with_metadata("model.call_started", EventLevel::Info, context, metadata)
            .attribute("provider", "openrouter")
            .attribute("requested_model", metadata.requested_model)
            .attribute("cognitive_role", metadata.cognitive_role)
            .attribute("prompt_version", metadata.prompt_version)
            .attribute("input_serialized_bytes", prompt.len()),
    );
    Ok(prompt)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RequestFailureFacts {
    error_class: &'static str,
    timeout_ms: Option<u64>,
    http_status: Option<u16>,
    provider_error_code: Option<String>,
    rate_limited: bool,
    quota_exhausted: bool,
}

impl RequestFailureFacts {
    const fn timeout(timeout_ms: u64) -> Self {
        Self {
            error_class: "timeout",
            timeout_ms: Some(timeout_ms),
            http_status: None,
            provider_error_code: None,
            rate_limited: false,
            quota_exhausted: false,
        }
    }
}

fn classify_request_failure(error: &anyhow::Error) -> RequestFailureFacts {
    let Some(completion) = error.downcast_ref::<CompletionError>() else {
        return RequestFailureFacts {
            error_class: "request_or_provider",
            timeout_ms: None,
            http_status: None,
            provider_error_code: None,
            rate_limited: false,
            quota_exhausted: false,
        };
    };
    let status = completion
        .provider_response_status()
        .map(|status| status.as_u16());
    let provider_json = completion.provider_response_json().ok().flatten();
    let provider_error_code = provider_json
        .as_ref()
        .and_then(|json| json.pointer("/error/code"))
        .map(|code| match code {
            serde_json::Value::String(value) => value.clone(),
            value => value.to_string(),
        });
    let provider_message = provider_json
        .as_ref()
        .and_then(|json| json.pointer("/error/message"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    let rate_limited = status == Some(429);
    let quota_exhausted = provider_message.contains("limit exceeded")
        || provider_message.contains("quota exceeded")
        || provider_message.contains("insufficient credits");
    let error_class = match status {
        Some(400) => "provider_bad_request",
        Some(401) => "provider_authentication",
        Some(403) if quota_exhausted => "provider_quota_exhausted",
        Some(403) => "provider_forbidden",
        Some(404) => "provider_model_unavailable",
        Some(429) => "provider_rate_limited",
        Some(500..=599) => "provider_unavailable",
        Some(_) => "provider_http_error",
        None => "provider_transport_or_response",
    };
    RequestFailureFacts {
        error_class,
        timeout_ms: None,
        http_status: status,
        provider_error_code,
        rate_limited,
        quota_exhausted,
    }
}

fn record_request_failure(
    analytics: &Arc<dyn AnalyticsSink>,
    context: &BrainCallContext,
    metadata: ModelEventMetadata<'_>,
    latency_ms: u64,
    failure: &RequestFailureFacts,
) {
    let mut event =
        model_event_with_metadata("model.call_failed", EventLevel::Warn, context, metadata)
            .attribute("provider", "openrouter")
            .attribute("requested_model", metadata.requested_model)
            .attribute("cognitive_role", metadata.cognitive_role)
            .attribute("prompt_version", metadata.prompt_version)
            .attribute("latency_ms", latency_ms)
            .attribute("error_class", failure.error_class)
            .attribute("response_received", false)
            .attribute("usage_accounted", false)
            .attribute("timeout_configured", failure.timeout_ms.is_some())
            .attribute("http_status_known", failure.http_status.is_some())
            .attribute("http_status", failure.http_status.unwrap_or(0))
            .attribute(
                "provider_error_code_known",
                failure.provider_error_code.is_some(),
            )
            .attribute(
                "provider_error_code",
                failure.provider_error_code.as_deref().unwrap_or(""),
            )
            .attribute("rate_limited", failure.rate_limited)
            .attribute("quota_exhausted", failure.quota_exhausted);
    if let Some(timeout_ms) = failure.timeout_ms {
        event = event.attribute("timeout_ms", timeout_ms);
    }
    analytics.record(event);
}

fn record_usage_anomaly(
    analytics: &Arc<dyn AnalyticsSink>,
    context: &BrainCallContext,
    metadata: ModelEventMetadata<'_>,
    metrics: &ModelCallMetrics,
) {
    let reported_output_exceeds_requested_max =
        metrics.output_tokens > metrics.requested_max_output_tokens;
    let completion_budget_exhausted = completion_budget_exhausted(metrics);
    if !reported_output_exceeds_requested_max && !completion_budget_exhausted {
        return;
    }
    analytics.record(
        model_event_with_metadata("model.usage_anomaly", EventLevel::Warn, context, metadata)
            .attribute("provider", "openrouter")
            .attribute("requested_model", metadata.requested_model)
            .attribute("actual_model", metrics.actual_model.clone())
            .attribute("generation_id", metrics.generation_id.clone())
            .attribute("cognitive_role", metadata.cognitive_role)
            .attribute("prompt_version", metadata.prompt_version)
            .attribute(
                "anomaly_class",
                if completion_budget_exhausted {
                    "completion_budget_exhausted"
                } else {
                    "reported_output_exceeds_requested_max"
                },
            )
            .attribute(
                "requested_max_output_tokens",
                metrics.requested_max_output_tokens,
            )
            .attribute("reported_output_tokens", metrics.output_tokens)
            .attribute("completion_budget_exhausted", completion_budget_exhausted)
            .attribute(
                "finish_reason",
                metrics.finish_reason.as_deref().unwrap_or(""),
            )
            .attribute(
                "native_finish_reason",
                metrics.native_finish_reason.as_deref().unwrap_or(""),
            ),
    );
}

struct ModelParseTelemetry<'a> {
    context: &'a BrainCallContext,
    metadata: ModelEventMetadata<'a>,
    metrics: &'a ModelCallMetrics,
    parse_latency_ms: u64,
    call_latency_ms: u64,
}

fn record_output_parse_success(
    analytics: &Arc<dyn AnalyticsSink>,
    telemetry: &ModelParseTelemetry<'_>,
    totals: &ModelUsageTotals,
) {
    analytics.record(output_parse_event(
        "model.output_parse_completed",
        EventLevel::Debug,
        telemetry.context,
        telemetry.metadata,
        telemetry.metrics,
        telemetry.parse_latency_ms,
        None,
    ));
    analytics.record(response_accounting_event(
        "model.call_completed",
        EventLevel::Info,
        telemetry.context,
        telemetry.metadata,
        telemetry.metrics,
        totals,
        telemetry.call_latency_ms,
    ));
}

fn record_output_parse_failure(
    analytics: &Arc<dyn AnalyticsSink>,
    telemetry: &ModelParseTelemetry<'_>,
    error: &ModelOutputError,
    output_text: Option<&str>,
) {
    let mut event = output_parse_event(
        "model.output_parse_failed",
        EventLevel::Warn,
        telemetry.context,
        telemetry.metadata,
        telemetry.metrics,
        telemetry.parse_latency_ms,
        Some(error),
    )
    .attribute("schema_error_kind", error.schema_error_kind());
    for (key, value) in safe_output_shape(output_text) {
        event = event.attribute(key, value);
    }
    analytics.record(event);
    analytics.record(
        model_event_with_metadata(
            "model.call_failed",
            EventLevel::Warn,
            telemetry.context,
            telemetry.metadata,
        )
        .attribute("provider", "openrouter")
        .attribute("requested_model", telemetry.metadata.requested_model)
        .attribute("actual_model", telemetry.metrics.actual_model.clone())
        .attribute("generation_id", telemetry.metrics.generation_id.clone())
        .attribute("cognitive_role", telemetry.metadata.cognitive_role)
        .attribute("prompt_version", telemetry.metadata.prompt_version)
        .attribute("latency_ms", telemetry.call_latency_ms)
        .attribute("error_class", "output_parse")
        .attribute("response_received", true)
        .attribute("usage_accounted", true),
    );
}

fn account_received_response(
    analytics: &Arc<dyn AnalyticsSink>,
    usage_ledger: &ModelUsageLedger,
    context: &BrainCallContext,
    metadata: ModelEventMetadata<'_>,
    metrics: &ModelCallMetrics,
    latency_ms: u64,
) -> ModelUsageTotals {
    let totals = usage_ledger.add(context.character_id.as_deref(), metrics);
    analytics.record(response_accounting_event(
        "model.response_received",
        EventLevel::Info,
        context,
        metadata,
        metrics,
        &totals,
        latency_ms,
    ));
    totals
}

fn response_accounting_event(
    name: &'static str,
    level: EventLevel,
    context: &BrainCallContext,
    metadata: ModelEventMetadata<'_>,
    metrics: &ModelCallMetrics,
    totals: &ModelUsageTotals,
    latency_ms: u64,
) -> AnalyticsEvent {
    let mut event = model_event_with_metadata(name, level, context, metadata)
        .attribute("provider", "openrouter")
        .attribute("requested_model", metadata.requested_model)
        .attribute("actual_model", metrics.actual_model.clone())
        .attribute("generation_id", metrics.generation_id.clone())
        .attribute("cognitive_role", metadata.cognitive_role)
        .attribute("prompt_version", metadata.prompt_version)
        .attribute("latency_ms", latency_ms)
        .attribute("input_tokens", metrics.input_tokens)
        .attribute("output_tokens", metrics.output_tokens)
        .attribute("total_tokens", metrics.total_tokens)
        .attribute("cached_input_tokens", metrics.cached_input_tokens)
        .attribute(
            "cache_creation_input_tokens",
            metrics.cache_creation_input_tokens,
        )
        .attribute("tool_use_prompt_tokens", metrics.tool_use_prompt_tokens)
        .attribute("reasoning_tokens", metrics.reasoning_tokens)
        .attribute("reasoning_tokens_source", "rig_normalized_usage")
        .attribute(
            "requested_max_output_tokens",
            metrics.requested_max_output_tokens,
        )
        .attribute(
            "reported_output_exceeds_requested_max",
            metrics.output_tokens > metrics.requested_max_output_tokens,
        )
        .attribute("finish_reason_known", metrics.finish_reason.is_some())
        .attribute(
            "finish_reason",
            metrics.finish_reason.as_deref().unwrap_or(""),
        )
        .attribute(
            "native_finish_reason_known",
            metrics.native_finish_reason.is_some(),
        )
        .attribute(
            "native_finish_reason",
            metrics.native_finish_reason.as_deref().unwrap_or(""),
        )
        .attribute(
            "completion_budget_exhausted",
            completion_budget_exhausted(metrics),
        )
        .attribute(
            "openrouter_cost_usd_known",
            metrics.exact_cost_usd.is_some(),
        )
        .attribute(
            "openrouter_cost_usd",
            metrics.exact_cost_usd.unwrap_or_default(),
        )
        .attribute(
            "openrouter_cost_usd_exact",
            metrics
                .exact_cost_usd
                .map_or_else(String::new, |cost| cost.to_string()),
        )
        .attribute("agent_model_calls_total", totals.calls)
        .attribute("agent_input_tokens_total", totals.input_tokens)
        .attribute("agent_output_tokens_total", totals.output_tokens)
        .attribute("agent_tokens_total", totals.total_tokens)
        .attribute(
            "agent_cached_input_tokens_total",
            totals.cached_input_tokens,
        )
        .attribute(
            "agent_cache_creation_input_tokens_total",
            totals.cache_creation_input_tokens,
        )
        .attribute(
            "agent_tool_use_prompt_tokens_total",
            totals.tool_use_prompt_tokens,
        )
        .attribute("agent_reasoning_tokens_total", totals.reasoning_tokens)
        .attribute(
            "agent_exact_cost_known_calls_total",
            totals.exact_cost_known_calls,
        )
        .attribute("agent_openrouter_cost_usd_total", totals.exact_cost_usd);
    if let Some(cost) = metrics.exact_cost_usd
        && let Some(total_tokens) = metrics.total_tokens.to_f64()
        && total_tokens > 0.0
    {
        event = event.attribute(
            "effective_openrouter_usd_per_total_token",
            cost / total_tokens,
        );
    }
    event
}

fn output_parse_event(
    name: &'static str,
    level: EventLevel,
    context: &BrainCallContext,
    metadata: ModelEventMetadata<'_>,
    metrics: &ModelCallMetrics,
    parse_latency_ms: u64,
    error: Option<&ModelOutputError>,
) -> AnalyticsEvent {
    let mut event = model_event_with_metadata(name, level, context, metadata)
        .attribute("provider", "openrouter")
        .attribute("requested_model", metadata.requested_model)
        .attribute("actual_model", metrics.actual_model.clone())
        .attribute("generation_id", metrics.generation_id.clone())
        .attribute("cognitive_role", metadata.cognitive_role)
        .attribute("prompt_version", metadata.prompt_version)
        .attribute("finish_reason_known", metrics.finish_reason.is_some())
        .attribute(
            "finish_reason",
            metrics.finish_reason.as_deref().unwrap_or(""),
        )
        .attribute(
            "native_finish_reason",
            metrics.native_finish_reason.as_deref().unwrap_or(""),
        )
        .attribute(
            "completion_budget_exhausted",
            completion_budget_exhausted(metrics),
        )
        .attribute("parse_latency_ms", parse_latency_ms)
        .attribute("parse_succeeded", error.is_none());
    if let Some(error) = error {
        event = event.attribute("error_class", error.class());
        if let Some((category, line, column)) = error.json_diagnostics() {
            event = event
                .attribute("json_error_category", category)
                .attribute("json_error_line", u64::try_from(line).unwrap_or(u64::MAX))
                .attribute(
                    "json_error_column",
                    u64::try_from(column).unwrap_or(u64::MAX),
                );
        }
    }
    event
}

fn completion_budget_exhausted(metrics: &ModelCallMetrics) -> bool {
    [
        metrics.finish_reason.as_deref(),
        metrics.native_finish_reason.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|reason| {
        matches!(
            reason.to_ascii_lowercase().as_str(),
            "length" | "max_tokens" | "max_output_tokens"
        )
    })
}

fn model_event(
    name: &'static str,
    level: EventLevel,
    context: &BrainCallContext,
) -> AnalyticsEvent {
    let event = AnalyticsEvent::new(name, level)
        .correlation(context.decision_id)
        .attribute("decision_id", context.decision_id.to_string())
        .attribute("frame_revision_known", context.frame_revision.is_some())
        .attribute("frame_revision", context.frame_revision.unwrap_or(0))
        .attribute(
            "strategic_revision_known",
            context.strategic_revision.is_some(),
        )
        .attribute(
            "strategic_revision",
            context.strategic_revision.unwrap_or(0),
        );
    context
        .character_id
        .as_deref()
        .map_or(event.clone(), |character_id| event.character(character_id))
}

fn model_event_with_metadata(
    name: &'static str,
    level: EventLevel,
    context: &BrainCallContext,
    metadata: ModelEventMetadata<'_>,
) -> AnalyticsEvent {
    reasoning_event_attributes(
        model_event(name, level, context),
        metadata.reasoning,
        metadata.reasoning_support_known,
    )
}

fn reasoning_event_attributes(
    event: AnalyticsEvent,
    reasoning: ModelReasoningConfig,
    support_known: bool,
) -> AnalyticsEvent {
    event
        .attribute("reasoning_requested_enabled", reasoning.enabled)
        .attribute("reasoning_requested_effort", reasoning.effort.as_str())
        .attribute("reasoning_response_excluded", reasoning.exclude)
        .attribute("reasoning_content_recorded", false)
        .attribute("reasoning_model_support_known", support_known)
        .attribute("reasoning_provider_parameters_required", true)
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DisabledBrain;

#[async_trait]
impl<I, O> Brain<I, O> for DisabledBrain
where
    I: Sync,
    O: Send,
{
    async fn decide(&self, _input: &I) -> anyhow::Result<O> {
        anyhow::bail!("brain is disabled during the Phase 1 runtime skeleton")
    }
}

#[cfg(test)]
mod tests {
    use rig_core::memory::{ConversationMemory, InMemoryConversationMemory};

    use super::*;
    use crate::observability::RecordingAnalyticsSink;

    #[test]
    fn model_event_inherits_the_decision_causal_context() {
        let decision_id = uuid::Uuid::new_v4();
        let event = model_event(
            "model.call_started",
            EventLevel::Debug,
            &BrainCallContext {
                decision_id,
                character_id: Some("cassian".to_owned()),
                frame_revision: Some(88),
                strategic_revision: Some(13),
            },
        );

        assert_eq!(event.character_id.as_deref(), Some("cassian"));
        assert_eq!(event.correlation_id, Some(decision_id));
        assert_eq!(event.attributes["decision_id"], decision_id.to_string());
        assert_eq!(event.attributes["frame_revision"], 88);
        assert_eq!(event.attributes["strategic_revision"], 13);
    }

    #[tokio::test]
    async fn openrouter_brain_uses_rig_conversation_memory_for_successful_turns() {
        let memory = Arc::new(InMemoryConversationMemory::new());
        let brain = OpenRouterJsonBrain::<serde_json::Value, serde_json::Value>::new_observed(
            "test-key",
            "test/model",
            "test prompt",
            0.1,
            100,
            ModelCallObservability::new("test/v1", Arc::new(RecordingAnalyticsSink::default())),
        )
        .expect("construct provider adapter")
        .with_conversation_memory(memory.clone(), "strategist");

        assert!(
            brain
                .load_conversation_history()
                .await
                .expect("empty load")
                .is_empty()
        );
        brain
            .append_conversation_turn(
                r#"{"moment":"met Orin"}"#.to_owned(),
                Some(r#"{"objective":"help Orin"}"#),
            )
            .await
            .expect("append successful turn");

        let stored = memory.load("strategist").await.expect("load stored turn");
        assert_eq!(stored.len(), 2);
        assert_eq!(
            brain.load_conversation_history().await.expect("brain load"),
            stored
        );
    }

    #[test]
    fn cancelled_model_call_records_one_terminal_failure() {
        let sink = Arc::new(RecordingAnalyticsSink::default());
        let decision_id = uuid::Uuid::new_v4();
        let context = BrainCallContext {
            decision_id,
            character_id: Some("cassian".to_owned()),
            frame_revision: Some(4),
            strategic_revision: Some(2),
        };
        {
            let _guard = ModelCallTerminalGuard::new(
                sink.clone(),
                context,
                ModelEventMetadata {
                    requested_model: "test/model",
                    cognitive_role: "tactician",
                    prompt_version: "tactician/v2",
                    reasoning: ModelReasoningConfig {
                        enabled: false,
                        effort: ReasoningEffort::Minimal,
                        exclude: true,
                    },
                    reasoning_support_known: false,
                },
                Instant::now(),
            );
        }

        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "model.call_failed");
        assert_eq!(events[0].correlation_id, Some(decision_id));
        assert_eq!(events[0].attributes["error_class"], "cancelled");
        assert_eq!(events[0].attributes["response_received"], false);
        assert_eq!(events[0].attributes["usage_accounted"], false);
    }

    #[test]
    fn accepts_one_json_document_and_rejects_wrappers() {
        let plain: serde_json::Value =
            parse_model_output(Some(r#"{"intent":"stop"}"#)).expect("plain JSON");
        assert_eq!(plain["intent"], "stop");
        assert!(
            parse_model_output::<serde_json::Value>(Some("```json\n{\"intent\":\"stop\"}\n```"))
                .is_err()
        );

        let encoded_object = r#""{\"intent\":\"stop\"}""#;
        assert!(
            parse_model_output::<crate::execution::packet::TacticalProposal>(Some(encoded_object))
                .is_err()
        );
    }

    #[test]
    fn provider_schema_removes_non_string_formats_but_preserves_constraints() {
        let schema = compatible_output_schema::<crate::execution::packet::TacticalProposal>();
        let value = schema.as_value();
        let valid_for_ms = &value["properties"]["valid_for_ms"];

        assert_eq!(valid_for_ms["type"], "integer");
        assert!(valid_for_ms.get("format").is_none());
        assert_eq!(valid_for_ms["minimum"], 100);
        assert_eq!(valid_for_ms["maximum"], 5_000);
        assert_eq!(value["additionalProperties"], false);
    }

    #[test]
    fn reasoning_parameters_use_rig_additional_parameter_shape_without_content_capture() {
        let params = reasoning_additional_params(
            ModelReasoningConfig {
                enabled: true,
                effort: ReasoningEffort::Medium,
                exclude: true,
            },
            Some("guy-strategist"),
        );

        assert_eq!(params["reasoning"]["effort"], "medium");
        assert_eq!(params["reasoning"]["exclude"], true);
        assert!(params["reasoning"].get("enabled").is_none());
        assert_eq!(params["provider"]["require_parameters"], true);
        assert_eq!(params["session_id"], "guy-strategist");
        assert!(!params.to_string().contains("reasoning_details"));
    }

    #[test]
    fn disabled_reasoning_is_explicit_for_latency_oriented_brains() {
        let params = reasoning_additional_params(
            ModelReasoningConfig {
                enabled: false,
                effort: ReasoningEffort::Minimal,
                exclude: true,
            },
            None,
        );

        assert_eq!(params["reasoning"]["enabled"], false);
        assert_eq!(params["reasoning"]["exclude"], true);
        assert!(params["reasoning"].get("effort").is_none());
    }

    #[test]
    fn known_model_reasoning_capabilities_fail_instead_of_mapping_silently() {
        let medium = ModelReasoningConfig {
            enabled: true,
            effort: ReasoningEffort::Medium,
            exclude: true,
        };
        assert!(
            validate_reasoning_for_model("openai/gpt-oss-120b", medium)
                .expect("GPT-OSS medium is supported")
        );
        assert!(
            validate_reasoning_for_model(
                "nvidia/nemotron-3-super-120b-a12b",
                ModelReasoningConfig {
                    effort: ReasoningEffort::High,
                    ..medium
                }
            )
            .is_err()
        );
        assert!(
            validate_reasoning_for_model(
                "openai/gpt-oss-120b",
                ModelReasoningConfig {
                    enabled: false,
                    ..medium
                }
            )
            .is_err()
        );
        assert!(
            !validate_reasoning_for_model("future/provider-model", medium)
                .expect("unknown models defer to required provider-parameter validation")
        );
    }

    #[test]
    fn finish_reason_exposes_shared_completion_budget_exhaustion() {
        let metrics = ModelCallMetrics {
            generation_id: "generation-truncated".to_owned(),
            actual_model: "openai/gpt-oss-120b".to_owned(),
            input_tokens: 100,
            output_tokens: 4_000,
            total_tokens: 4_100,
            cached_input_tokens: 0,
            cache_creation_input_tokens: 0,
            tool_use_prompt_tokens: 0,
            reasoning_tokens: 0,
            exact_cost_usd: Some(0.001),
            requested_max_output_tokens: 4_000,
            finish_reason: Some("length".to_owned()),
            native_finish_reason: Some("MAX_TOKENS".to_owned()),
        };

        assert!(completion_budget_exhausted(&metrics));
    }

    #[test]
    fn finalized_native_reasoning_and_cache_usage_reconcile_into_agent_totals() {
        let ledger = ModelUsageLedger::default();
        ledger.add(
            Some("guy"),
            &ModelCallMetrics {
                generation_id: "generation-native".to_owned(),
                actual_model: "openai/gpt-oss-120b".to_owned(),
                input_tokens: 100,
                output_tokens: 50,
                total_tokens: 150,
                cached_input_tokens: 10,
                cache_creation_input_tokens: 0,
                tool_use_prompt_tokens: 0,
                reasoning_tokens: 0,
                exact_cost_usd: Some(0.001),
                requested_max_output_tokens: 4_000,
                finish_reason: Some("stop".to_owned()),
                native_finish_reason: Some("STOP".to_owned()),
            },
        );

        let reconciliation = ledger.reconcile_native_usage(Some("guy"), 0, 10, Some(700), Some(80));

        assert_eq!(reconciliation.reasoning_delta, 700);
        assert_eq!(reconciliation.cached_delta, 70);
        assert_eq!(reconciliation.updated_totals.reasoning_tokens, 700);
        assert_eq!(reconciliation.updated_totals.cached_input_tokens, 80);
        assert_eq!(reconciliation.updated_totals.calls, 1);
    }

    #[cfg(unix)]
    #[test]
    fn local_logical_input_capture_is_private_and_contains_no_credentials() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().expect("temporary capture parent");
        let directory = root.path().join("captures");
        write_private_capture(&directory, "request.json", br#"{"model":"test"}"#)
            .expect("private capture");

        let directory_mode = fs::metadata(&directory)
            .expect("directory metadata")
            .permissions()
            .mode()
            & 0o777;
        let capture = directory.join("request.json");
        let file_mode = fs::metadata(&capture)
            .expect("capture metadata")
            .permissions()
            .mode()
            & 0o777;
        let content = fs::read_to_string(capture).expect("read capture");

        assert_eq!(directory_mode, 0o700);
        assert_eq!(file_mode, 0o600);
        assert_eq!(content, r#"{"model":"test"}"#);
        assert!(!content.contains("api_key"));
        assert!(!content.contains("Authorization"));
    }

    #[test]
    fn accounts_a_billable_response_before_reporting_parse_failure() {
        let sink = Arc::new(RecordingAnalyticsSink::default());
        let analytics: Arc<dyn AnalyticsSink> = sink.clone();
        let ledger = ModelUsageLedger::default();
        let mut usage_revisions = ledger.subscribe();
        let decision_id = uuid::Uuid::new_v4();
        let context = BrainCallContext {
            decision_id,
            character_id: Some("cassian".to_owned()),
            frame_revision: Some(91),
            strategic_revision: Some(12),
        };
        let metadata = ModelEventMetadata {
            requested_model: "test/requested-model",
            cognitive_role: "tactician",
            prompt_version: "tactician/v1",
            reasoning: ModelReasoningConfig {
                enabled: false,
                effort: ReasoningEffort::Minimal,
                exclude: true,
            },
            reasoning_support_known: false,
        };
        let metrics = ModelCallMetrics {
            generation_id: "generation-billed-1".to_owned(),
            actual_model: "test/actual-model".to_owned(),
            input_tokens: 101,
            output_tokens: 17,
            total_tokens: 118,
            cached_input_tokens: 64,
            cache_creation_input_tokens: 3,
            tool_use_prompt_tokens: 0,
            reasoning_tokens: 9,
            exact_cost_usd: Some(0.000_012_34),
            requested_max_output_tokens: 150,
            finish_reason: Some("stop".to_owned()),
            native_finish_reason: Some("STOP".to_owned()),
        };

        let totals =
            account_received_response(&analytics, &ledger, &context, metadata, &metrics, 42);
        let private_raw_output = "not-json PRIVATE_MODEL_OUTPUT_MUST_NOT_LEAK";
        let parse_error = parse_model_output::<serde_json::Value>(Some(private_raw_output))
            .expect_err("invalid output must fail");
        record_output_parse_failure(
            &analytics,
            &ModelParseTelemetry {
                context: &context,
                metadata,
                metrics: &metrics,
                parse_latency_ms: 1,
                call_latency_ms: 43,
            },
            &parse_error,
            Some(private_raw_output),
        );

        assert_eq!(totals.calls, 1);
        assert_eq!(totals.total_tokens, 118);
        assert_eq!(totals.cached_input_tokens, 64);
        assert_eq!(totals.reasoning_tokens, 9);
        assert_eq!(totals.exact_cost_known_calls, 1);
        assert!((totals.exact_cost_usd - 0.000_012_34).abs() < f64::EPSILON);
        assert!(usage_revisions.has_changed().expect("ledger remains open"));
        assert_eq!(*usage_revisions.borrow_and_update(), 1);

        let events = sink.events();
        assert_eq!(events.len(), 3);
        let received = &events[0];
        assert_eq!(received.name, "model.response_received");
        assert_eq!(received.character_id.as_deref(), Some("cassian"));
        assert_eq!(received.correlation_id, Some(decision_id));
        assert_eq!(received.attributes["generation_id"], "generation-billed-1");
        assert_eq!(received.attributes["input_tokens"], 101);
        assert_eq!(received.attributes["cached_input_tokens"], 64);
        assert_eq!(received.attributes["reasoning_tokens"], 9);
        assert_eq!(received.attributes["requested_max_output_tokens"], 150);
        assert_eq!(
            received.attributes["reported_output_exceeds_requested_max"],
            false
        );
        assert_eq!(received.attributes["openrouter_cost_usd_known"], true);
        assert_eq!(
            received.attributes["openrouter_cost_usd_exact"],
            "0.00001234"
        );

        let failed = &events[1];
        assert_eq!(failed.name, "model.output_parse_failed");
        assert_eq!(failed.correlation_id, Some(decision_id));
        assert_eq!(failed.attributes["generation_id"], "generation-billed-1");
        assert_eq!(failed.attributes["error_class"], "invalid_json");

        let terminal = &events[2];
        assert_eq!(terminal.name, "model.call_failed");
        assert_eq!(terminal.correlation_id, Some(decision_id));
        assert_eq!(terminal.attributes["generation_id"], "generation-billed-1");
        assert_eq!(terminal.attributes["error_class"], "output_parse");
        assert_eq!(terminal.attributes["response_received"], true);
        assert_eq!(terminal.attributes["usage_accounted"], true);

        let serialized = serde_json::to_string(&events).expect("events serialize");
        assert!(!serialized.contains(private_raw_output));
        assert!(!serialized.contains("PRIVATE_MODEL_OUTPUT_MUST_NOT_LEAK"));
    }

    #[test]
    fn missing_text_has_a_stable_private_parse_failure() {
        let error = parse_model_output::<serde_json::Value>(None)
            .expect_err("missing text must be rejected");

        assert_eq!(error.class(), "missing_text");
        assert!(error.json_diagnostics().is_none());
    }

    #[tokio::test]
    async fn drains_or_explicitly_aborts_every_background_task() {
        let tasks = ModelBackgroundTasks::default();
        tasks.spawn(async {});
        tasks.spawn(async {
            tokio::time::sleep(Duration::from_mins(1)).await;
        });

        let drained = tasks.drain(Duration::from_millis(10)).await;

        assert_eq!(drained.completed, 1);
        assert_eq!(drained.failed, 0);
        assert_eq!(drained.aborted, 1);
        assert_eq!(drained.active_model_calls_remaining, 0);
    }

    #[tokio::test]
    async fn model_request_deadline_terminates_a_hung_provider_future() {
        let started = Instant::now();
        let error = time_bound_model_request(Duration::from_millis(10), async {
            std::future::pending::<()>().await;
        })
        .await
        .expect_err("hung provider call must time out");

        assert_eq!(error.timeout_ms, 10);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn timeout_failure_has_stable_private_telemetry() {
        let sink = Arc::new(RecordingAnalyticsSink::default());
        let analytics: Arc<dyn AnalyticsSink> = sink.clone();
        let context = BrainCallContext {
            decision_id: uuid::Uuid::new_v4(),
            character_id: Some("cassian".to_owned()),
            frame_revision: Some(1842),
            strategic_revision: Some(42),
        };
        record_request_failure(
            &analytics,
            &context,
            ModelEventMetadata {
                requested_model: "test/hung-model",
                cognitive_role: "tactician",
                prompt_version: "tactician/v3",
                reasoning: ModelReasoningConfig {
                    enabled: false,
                    effort: ReasoningEffort::Minimal,
                    exclude: true,
                },
                reasoning_support_known: false,
            },
            5_001,
            &RequestFailureFacts::timeout(5_000),
        );

        let event = &sink.events()[0];
        assert_eq!(event.name, "model.call_failed");
        assert_eq!(event.attributes["error_class"], "timeout");
        assert_eq!(event.attributes["timeout_configured"], true);
        assert_eq!(event.attributes["timeout_ms"], 5_000);
        assert_eq!(event.attributes["response_received"], false);
        assert_eq!(event.attributes["usage_accounted"], false);
        assert_eq!(event.attributes["http_status_known"], false);
        assert_eq!(event.attributes["rate_limited"], false);
        assert_eq!(event.attributes["quota_exhausted"], false);
    }

    #[test]
    fn provider_quota_failure_is_classified_without_recording_its_message() {
        let error = CompletionError::from_http_response(
            reqwest::StatusCode::FORBIDDEN,
            r#"{"error":{"message":"Key limit exceeded (total limit)","code":403}}"#,
        );
        let facts = classify_request_failure(&anyhow::Error::new(error));

        assert_eq!(facts.error_class, "provider_quota_exhausted");
        assert_eq!(facts.http_status, Some(403));
        assert_eq!(facts.provider_error_code.as_deref(), Some("403"));
        assert!(!facts.rate_limited);
        assert!(facts.quota_exhausted);
    }

    #[tokio::test]
    async fn drain_waits_for_accounting_registered_by_an_active_model_call() {
        let tasks = Arc::new(ModelBackgroundTasks::default());
        let active_call = tasks.begin_model_call();
        let producer_tasks = tasks.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            producer_tasks.spawn(async {});
            drop(active_call);
        });

        let drained = tasks.drain(Duration::from_millis(100)).await;

        assert_eq!(drained.completed, 1);
        assert_eq!(drained.failed, 0);
        assert_eq!(drained.aborted, 0);
        assert_eq!(drained.active_model_calls_remaining, 0);
    }
}
