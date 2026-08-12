use std::{
    marker::PhantomData,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use rig_agent::{AgentBuilder, completion::TypedPrompt};
use rig_core::{
    completion::CompletionModel,
    memory::ConversationMemory,
    wasm_compat::{WasmCompatSend, WasmCompatSync},
};
use schemars::JsonSchema;
use serde::{Serialize, de::DeserializeOwned};

use super::{Brain, BrainCallContext};
use crate::observability::{AnalyticsEvent, AnalyticsSink, EventLevel};

/// A Rig Agent adapter for a typed, conversation-aware brain.
///
/// This module hides Rig messages, conversation loading, history append, and
/// structured output behind the harness `Brain` interface. The Ractor actor
/// supplies typed input and never manages conversation history.
pub struct RigAgentBrain<M, I, O>
where
    M: CompletionModel,
{
    model: M,
    preamble: String,
    conversation_id: String,
    memory: Arc<dyn ConversationMemory>,
    temperature: f64,
    max_output_tokens: u64,
    max_turns: usize,
    request_timeout: Duration,
    analytics: Arc<dyn AnalyticsSink>,
    marker: PhantomData<fn(&I) -> O>,
}

impl<M, I, O> RigAgentBrain<M, I, O>
where
    M: CompletionModel,
{
    pub fn new(
        model: M,
        preamble: impl Into<String>,
        conversation_id: impl Into<String>,
        memory: Arc<dyn ConversationMemory>,
        analytics: Arc<dyn AnalyticsSink>,
    ) -> Self {
        Self {
            model,
            preamble: preamble.into(),
            conversation_id: conversation_id.into(),
            memory,
            temperature: 0.2,
            max_output_tokens: 800,
            max_turns: 1,
            request_timeout: Duration::from_secs(30),
            analytics,
            marker: PhantomData,
        }
    }

    #[must_use]
    pub const fn with_generation_limits(
        mut self,
        temperature: f64,
        max_output_tokens: u64,
        max_turns: usize,
    ) -> Self {
        self.temperature = temperature;
        self.max_output_tokens = max_output_tokens;
        self.max_turns = max_turns;
        self
    }

    #[must_use]
    pub const fn with_request_timeout(mut self, request_timeout: Duration) -> Self {
        self.request_timeout = request_timeout;
        self
    }
}

#[async_trait]
impl<M, I, O> Brain<I, O> for RigAgentBrain<M, I, O>
where
    M: CompletionModel + 'static,
    I: Serialize + Sync,
    O: DeserializeOwned + JsonSchema + Send + Sync + WasmCompatSend + WasmCompatSync + 'static,
{
    async fn decide(&self, input: &I) -> anyhow::Result<O> {
        self.decide_with_context(input, &BrainCallContext::standalone())
            .await
    }

    async fn decide_with_context(
        &self,
        input: &I,
        context: &BrainCallContext,
    ) -> anyhow::Result<O> {
        let started = Instant::now();
        self.analytics.record(with_context(
            AnalyticsEvent::new("rig.agent_run_started", EventLevel::Info)
                .correlation(context.decision_id)
                .attribute("max_turns", usize_to_u64(self.max_turns))
                .attribute("requested_max_output_tokens", self.max_output_tokens)
                .attribute("timeout_ms", duration_ms(self.request_timeout)),
            context,
        ));
        let prompt = match serde_json::to_string(input) {
            Ok(prompt) => prompt,
            Err(error) => {
                self.record_failure(context, started, "input_serialize");
                return Err(error.into());
            }
        };
        let agent = AgentBuilder::new(self.model.clone())
            .preamble(&self.preamble)
            .temperature(self.temperature)
            .max_tokens(self.max_output_tokens)
            .memory(self.memory.clone())
            .build();
        let request = agent
            .prompt_typed::<O>(prompt)
            .conversation(&self.conversation_id)
            .max_turns(self.max_turns)
            .extended_details();
        match tokio::time::timeout(self.request_timeout, request).await {
            Ok(Ok(response)) => {
                self.analytics.record(with_context(
                    AnalyticsEvent::new("rig.agent_run_completed", EventLevel::Info)
                        .correlation(context.decision_id)
                        .attribute("duration_ms", elapsed_ms(started))
                        .attribute("model_request_count", usize_to_u64(response.requests()))
                        .attribute("input_tokens", response.usage.input_tokens)
                        .attribute("output_tokens", response.usage.output_tokens)
                        .attribute("total_tokens", response.usage.total_tokens)
                        .attribute("cached_input_tokens", response.usage.cached_input_tokens)
                        .attribute(
                            "cache_creation_input_tokens",
                            response.usage.cache_creation_input_tokens,
                        )
                        .attribute(
                            "tool_use_prompt_tokens",
                            response.usage.tool_use_prompt_tokens,
                        )
                        .attribute("reasoning_tokens", response.usage.reasoning_tokens),
                    context,
                ));
                Ok(response.output)
            }
            Ok(Err(error)) => {
                self.record_failure(context, started, "agent_or_structured_output");
                Err(error.into())
            }
            Err(error) => {
                self.record_failure(context, started, "timeout");
                Err(error.into())
            }
        }
    }
}

impl<M, I, O> RigAgentBrain<M, I, O>
where
    M: CompletionModel,
{
    fn record_failure(
        &self,
        context: &BrainCallContext,
        started: Instant,
        error_class: &'static str,
    ) {
        self.analytics.record(with_context(
            AnalyticsEvent::new("rig.agent_run_failed", EventLevel::Warn)
                .correlation(context.decision_id)
                .attribute("duration_ms", elapsed_ms(started))
                .attribute("error_class", error_class),
            context,
        ));
    }
}

fn with_context(mut event: AnalyticsEvent, context: &BrainCallContext) -> AnalyticsEvent {
    if let Some(character_id) = &context.character_id {
        event = event.character(character_id);
    }
    event
        .attribute("frame_revision_known", context.frame_revision.is_some())
        .attribute("frame_revision", context.frame_revision.unwrap_or_default())
        .attribute(
            "strategic_revision_known",
            context.strategic_revision.is_some(),
        )
        .attribute(
            "strategic_revision",
            context.strategic_revision.unwrap_or_default(),
        )
}

fn elapsed_ms(started: Instant) -> u64 {
    duration_ms(started.elapsed())
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use rig_core::{
        memory::{ConversationMemory, InMemoryConversationMemory},
        test_utils::{MockCompletionModel, MockTurn},
    };
    use serde::Deserialize;

    use super::*;
    use crate::observability::RecordingAnalyticsSink;

    #[derive(Debug, Deserialize, JsonSchema, PartialEq, Eq)]
    struct Answer {
        value: String,
    }

    #[tokio::test]
    async fn rig_agent_loads_and_appends_conversation_automatically() {
        let model = MockCompletionModel::new([
            MockTurn::text(r#"{"value":"first"}"#),
            MockTurn::text(r#"{"value":"second"}"#),
        ]);
        let memory = Arc::new(InMemoryConversationMemory::new());
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let brain = RigAgentBrain::<_, String, Answer>::new(
            model.clone(),
            "Return JSON.",
            "strategist:cassian:main",
            memory.clone(),
            analytics.clone(),
        );

        let first = brain
            .decide(&"first prompt".to_owned())
            .await
            .expect("first");
        let second = brain
            .decide(&"second prompt".to_owned())
            .await
            .expect("second");

        assert_eq!(first.value, "first");
        assert_eq!(second.value, "second");
        assert_eq!(
            memory
                .load("strategist:cassian:main")
                .await
                .expect("load history")
                .len(),
            4
        );
        let requests = model.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].chat_history.len() > requests[0].chat_history.len());
        assert_eq!(
            analytics
                .events()
                .iter()
                .filter(|event| event.name == "rig.agent_run_completed")
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn explicit_rig_memory_failure_is_visible_and_content_safe() {
        let model = MockCompletionModel::text(r#"{"value":"unused"}"#);
        let memory = Arc::new(rig_core::test_utils::FailingMemory::new("unavailable"));
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let brain = RigAgentBrain::<_, String, Answer>::new(
            model,
            "Return JSON.",
            "strategist:cassian:main",
            memory,
            analytics.clone(),
        );
        let context = BrainCallContext {
            decision_id: uuid::Uuid::new_v4(),
            character_id: Some("cassian".to_owned()),
            frame_revision: None,
            strategic_revision: Some(7),
        };

        let result = brain
            .decide_with_context(&"private prompt".to_owned(), &context)
            .await;

        assert!(result.is_err());
        let events = analytics.events();
        assert!(events.iter().any(|event| {
            event.name == "rig.agent_run_failed"
                && event.correlation_id == Some(context.decision_id)
                && event.character_id.as_deref() == Some("cassian")
                && event.attributes["error_class"] == "agent_or_structured_output"
        }));
        let encoded = serde_json::to_string(&events).expect("encode events");
        assert!(!encoded.contains("private prompt"));
        assert!(!encoded.contains("unavailable"));
        assert!(!encoded.contains("strategist:cassian:main"));
    }
}
