//! Typed checkpoint protocol for the persistent strategist loop.
//!
//! This protocol is deliberately separate from MCP and from provider message
//! history. The actor owns revisions and routing; the Rig session owns bounded
//! conversation state.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, time::Duration};

use async_trait::async_trait;
use rig_agent::{
    Agent,
    client::AgentClientExt,
    completion::Prompt,
    core::{providers::openrouter, tool::builtin::ThinkTool},
};

use super::{
    Brain, BrainCallContext, strategic_input::StrategicWorldSnapshot,
    strategic_intent::StrategicIntent,
};
use crate::memory::{recall::StrategicRecall, working::WorkingMemory};
use crate::{
    brain::models::ModelUsageLedger,
    observability::{AnalyticsEvent, AnalyticsSink, EventLevel},
};
use std::sync::Arc;

pub const STRATEGIC_CHECKPOINT_PROTOCOL_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StrategicCheckpointEnvelope {
    pub protocol_version: u32,
    pub checkpoint_id: String,
    pub character_id: String,
    pub inbox: Vec<StrategicInboxFact>,
    pub current_intent: StrategicIntent,
    pub working_memory: WorkingMemory,
    pub world_snapshot: StrategicWorldSnapshot,
    pub recalled_memory: StrategicRecall,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StrategicInboxFact {
    pub kind: String,
    pub summary: String,
    pub speaker: Option<String>,
    pub channel: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StrategicCheckpointOutput {
    pub protocol_version: u32,
    pub checkpoint_id: String,
    pub continue_thinking: bool,
    pub events: Vec<StrategicEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum StrategicEvent {
    ThoughtCheckpoint {
        summary: String,
    },
    NavigationGoal {
        scene: String,
        destination: Option<StrategicTile>,
        reason: String,
    },
    Speech {
        message: String,
        channel: Option<String>,
        recipient: Option<String>,
    },
    Interact {
        target_id: String,
    },
    QueueDuel,
    PlanUpdate {
        summary: String,
    },
    GoalComplete {
        evidence: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StrategicTile {
    pub tile_x: i32,
    pub tile_y: i32,
}

/// Runtime-side mailbox for one persistent strategist session.
///
/// It implements latest-value semantics without cancelling the Rig agent: new
/// facts are coalesced while a checkpoint is in flight and become the input to
/// the next checkpoint. This keeps actor handlers responsive and prevents an
/// old world snapshot from building an unbounded model-call queue.
#[derive(Debug, Default)]
pub struct StrategicAgenticLoop {
    inbox: VecDeque<StrategicInboxFact>,
    checkpoint_sequence: u64,
    in_flight: Option<String>,
    continuation_requested: bool,
}

impl StrategicAgenticLoop {
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inbox: VecDeque::with_capacity(capacity),
            ..Self::default()
        }
    }

    /// Add a fact for the next stopping point. Repeated world snapshots are
    /// coalesced; dialogue and failures are retained in order.
    pub fn push(&mut self, fact: StrategicInboxFact, capacity: usize) {
        if fact.kind == "world" {
            self.inbox.retain(|existing| existing.kind != "world");
        }
        self.inbox.push_back(fact);
        while self.inbox.len() > capacity.max(1) {
            let _ = self.inbox.pop_front();
        }
    }

    /// Begin a checkpoint if no request is currently running.
    pub fn begin(&mut self) -> Option<(String, Vec<StrategicInboxFact>)> {
        if self.in_flight.is_some() || (self.inbox.is_empty() && !self.continuation_requested) {
            return None;
        }
        self.checkpoint_sequence = self.checkpoint_sequence.saturating_add(1);
        let id = format!("strategic-checkpoint-{}", self.checkpoint_sequence);
        self.in_flight = Some(id.clone());
        self.continuation_requested = false;
        let facts = self.inbox.drain(..).collect();
        Some((id, facts))
    }

    /// Accept a validated result and report whether another checkpoint is due.
    pub fn finish(&mut self, output: &StrategicCheckpointOutput) -> anyhow::Result<bool> {
        let Some(expected) = self.in_flight.take() else {
            anyhow::bail!("strategic checkpoint completed without an in-flight request");
        };
        if expected != output.checkpoint_id {
            anyhow::bail!("strategic checkpoint completion does not match request");
        }
        self.continuation_requested = output.continue_thinking;
        Ok(self.continuation_requested || !self.inbox.is_empty())
    }

    #[must_use]
    pub fn continuation_requested(&self) -> bool {
        self.continuation_requested
    }

    #[must_use]
    pub fn in_flight(&self) -> bool {
        self.in_flight.is_some()
    }
}

/// A persistent Rig agent session for strategic cognition.
///
/// The session owns the conversation memory and the bounded Rig tool loop. The
/// runtime still owns all game mutations: the only built-in tool installed here
/// is `ThinkTool`, and the final response is a strict checkpoint envelope.
#[derive(Clone)]
pub struct RigStrategicAgent {
    agent: Agent<openrouter::CompletionModel>,
    checkpoint_timeout: Duration,
    character_id: Option<String>,
    analytics: Option<Arc<dyn AnalyticsSink>>,
    usage_ledger: Option<Arc<ModelUsageLedger>>,
    model_id: String,
    prompt_version: String,
}

impl RigStrategicAgent {
    /// Construct a strategist with a persistent conversation and bounded turns.
    ///
    /// # Errors
    /// Returns an error when the OpenRouter client cannot be constructed.
    pub fn new(
        api_key: &str,
        model_id: &str,
        preamble: impl Into<String>,
        conversation_memory: Option<std::sync::Arc<dyn rig_core::memory::ConversationMemory>>,
        conversation_id: impl Into<String>,
        max_turns: usize,
    ) -> anyhow::Result<Self> {
        let client = openrouter::Client::new(api_key)?;
        let mut builder = client
            .agent(model_id)
            .name("strategist")
            .description("Persistent long-horizon planner for one Agent Arena character.")
            .preamble(&preamble.into())
            .tool(ThinkTool)
            .output_schema::<StrategicCheckpointOutput>()
            .default_max_turns(max_turns)
            .conversation(conversation_id);
        if let Some(memory) = conversation_memory {
            builder = builder.memory(memory);
        }
        Ok(Self {
            agent: builder.record_content_telemetry(false).build(),
            checkpoint_timeout: Duration::from_secs(120),
            character_id: None,
            analytics: None,
            usage_ledger: None,
            model_id: model_id.to_owned(),
            prompt_version: "strategist/agentic-v1".to_owned(),
        })
    }

    #[must_use]
    pub fn with_observability(
        mut self,
        character_id: impl Into<String>,
        analytics: Arc<dyn AnalyticsSink>,
        usage_ledger: Arc<ModelUsageLedger>,
        prompt_version: impl Into<String>,
    ) -> Self {
        self.character_id = Some(character_id.into());
        self.analytics = Some(analytics);
        self.usage_ledger = Some(usage_ledger);
        self.prompt_version = prompt_version.into();
        self
    }

    /// Set the maximum wall-clock time for one complete Rig agent run.
    #[must_use]
    pub const fn with_checkpoint_timeout(mut self, timeout: Duration) -> Self {
        self.checkpoint_timeout = timeout;
        self
    }

    /// Run one checkpoint. Rig may perform several internal think/tool turns;
    /// callers should feed new runtime messages into the next checkpoint rather
    /// than starting a second concurrent session.
    pub async fn checkpoint(
        &self,
        input: &StrategicCheckpointEnvelope,
    ) -> anyhow::Result<StrategicCheckpointOutput> {
        let prompt = serde_json::to_string(input)?;
        let response = tokio::time::timeout(
            self.checkpoint_timeout,
            self.agent.prompt(prompt).extended_details(),
        )
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "strategist Rig checkpoint exceeded {}ms",
                self.checkpoint_timeout.as_millis()
            )
        })??;
        let decision_id = uuid::Uuid::new_v4();
        if let (Some(character_id), Some(ledger), Some(analytics)) =
            (&self.character_id, &self.usage_ledger, &self.analytics)
        {
            let totals = ledger.record_rig_agent_usage(
                character_id,
                &self.model_id,
                "strategist",
                &self.prompt_version,
                decision_id,
                &response.completion_calls,
            );
            analytics.record(
                AnalyticsEvent::new("model.rig_agent_usage", EventLevel::Info)
                    .character(character_id)
                    .correlation(decision_id)
                    .attribute("provider", "openrouter")
                    .attribute("requested_model", self.model_id.clone())
                    .attribute("cognitive_role", "strategist")
                    .attribute("prompt_version", self.prompt_version.clone())
                    .attribute("agent_run", true)
                    .attribute("completion_calls", response.completion_calls.len())
                    .attribute("input_tokens", response.usage.input_tokens)
                    .attribute("output_tokens", response.usage.output_tokens)
                    .attribute("total_tokens", response.usage.total_tokens)
                    .attribute("cached_input_tokens", response.usage.cached_input_tokens)
                    .attribute("reasoning_tokens", response.usage.reasoning_tokens)
                    .attribute("exact_cost_known", totals.exact_cost_known_calls > 0)
                    .attribute("cumulative_input_tokens", totals.input_tokens)
                    .attribute("cumulative_output_tokens", totals.output_tokens)
                    .attribute("cumulative_total_tokens", totals.total_tokens)
                    .attribute("cumulative_cached_input_tokens", totals.cached_input_tokens),
            );
        }
        let output = serde_json::from_str::<StrategicCheckpointOutput>(&response.output)
            .map_err(|error| anyhow::anyhow!("strategist checkpoint JSON rejected: {error}"))?;
        if output.protocol_version != STRATEGIC_CHECKPOINT_PROTOCOL_VERSION {
            anyhow::bail!(
                "strategist checkpoint protocol {} does not match {}",
                output.protocol_version,
                STRATEGIC_CHECKPOINT_PROTOCOL_VERSION
            );
        }
        if output.checkpoint_id != input.checkpoint_id {
            anyhow::bail!("strategist checkpoint id does not match request");
        }
        if let (Some(character_id), Some(analytics)) = (&self.character_id, &self.analytics) {
            let mut thought_count = 0_u64;
            let mut navigation_count = 0_u64;
            let mut speech_count = 0_u64;
            let mut interaction_count = 0_u64;
            let mut plan_count = 0_u64;
            let mut completion_count = 0_u64;
            for event in &output.events {
                match event {
                    StrategicEvent::ThoughtCheckpoint { .. } => thought_count += 1,
                    StrategicEvent::NavigationGoal { .. } => navigation_count += 1,
                    StrategicEvent::Speech { .. } => speech_count += 1,
                    StrategicEvent::Interact { .. } => interaction_count += 1,
                    StrategicEvent::QueueDuel => interaction_count += 1,
                    StrategicEvent::PlanUpdate { .. } => plan_count += 1,
                    StrategicEvent::GoalComplete { .. } => completion_count += 1,
                }
            }
            analytics.record(
                AnalyticsEvent::new("strategic.agentic_output", EventLevel::Info)
                    .character(character_id)
                    .attribute("checkpoint_id", input.checkpoint_id.clone())
                    .attribute("event_count", output.events.len())
                    .attribute("thought_count", thought_count)
                    .attribute("navigation_count", navigation_count)
                    .attribute("speech_count", speech_count)
                    .attribute("interaction_count", interaction_count)
                    .attribute("plan_count", plan_count)
                    .attribute("goal_completion_count", completion_count)
                    .attribute("continue_thinking", output.continue_thinking)
                    .attribute(
                        "actionable_event_count",
                        navigation_count + speech_count + interaction_count,
                    ),
            );
        }
        Ok(output)
    }
}

/// Adapter for code that still uses the one-proposal brain seam.
///
/// This is intentionally a protocol adapter, not a second game executor. It
/// lets the persistent Rig session be introduced behind the existing actor
/// boundary while the actor migration is completed.
#[async_trait]
impl Brain<StrategicCheckpointEnvelope, StrategicCheckpointOutput> for RigStrategicAgent {
    async fn decide(
        &self,
        input: &StrategicCheckpointEnvelope,
    ) -> anyhow::Result<StrategicCheckpointOutput> {
        self.checkpoint(input).await
    }

    async fn decide_with_context(
        &self,
        input: &StrategicCheckpointEnvelope,
        _context: &BrainCallContext,
    ) -> anyhow::Result<StrategicCheckpointOutput> {
        self.checkpoint(input).await
    }
}

/// Compatibility adapter while the StrategistActor migrates to checkpoint
/// events directly. It preserves the existing proposal seam but obtains its
/// decision from the persistent ThinkTool-enabled Rig session.
#[derive(Clone)]
pub struct RigStrategicProposalBrain {
    session: RigStrategicAgent,
}

impl RigStrategicProposalBrain {
    #[must_use]
    pub const fn new(session: RigStrategicAgent) -> Self {
        Self { session }
    }
}

#[async_trait]
impl Brain<super::strategic_input::StrategicInput, super::strategic_output::StrategicProposal>
    for RigStrategicProposalBrain
{
    async fn decide(
        &self,
        input: &super::strategic_input::StrategicInput,
    ) -> anyhow::Result<super::strategic_output::StrategicProposal> {
        let checkpoint_id = format!("compat-{}", uuid::Uuid::new_v4());
        let envelope = StrategicCheckpointEnvelope {
            protocol_version: STRATEGIC_CHECKPOINT_PROTOCOL_VERSION,
            checkpoint_id,
            character_id: input.character_id.clone(),
            inbox: input
                .moments
                .iter()
                .map(|moment| StrategicInboxFact {
                    kind: format!("{:?}", moment.kind).to_lowercase(),
                    summary: moment.summary.clone(),
                    speaker: moment.speaker.clone(),
                    channel: moment.dialogue_channel.clone(),
                })
                .collect(),
            current_intent: input.current_intent.clone(),
            working_memory: input.memory.working.clone(),
            world_snapshot: input.world.clone(),
            recalled_memory: input.memory.clone(),
        };
        let output = self.session.checkpoint(&envelope).await?;
        let mut proposal = super::strategic_output::StrategicProposal::from(&input.current_intent);
        proposal.continue_thinking = output.continue_thinking;
        for event in output.events {
            match event {
                StrategicEvent::NavigationGoal {
                    scene,
                    destination,
                    reason,
                } => {
                    let goal = super::strategic_intent::NavigationGoal {
                        scene,
                        destination: destination.map(|tile| {
                            super::strategic_intent::NamedDestination {
                                name: "strategic checkpoint destination".to_owned(),
                                tile: Some(crate::world::TilePosition {
                                    x: tile.tile_x,
                                    y: tile.tile_y,
                                }),
                            }
                        }),
                        reason,
                    };
                    if proposal.navigation_goal.is_none() {
                        proposal.navigation_goal = Some(goal.clone());
                    }
                    if proposal.navigation_queue.len() < 5 {
                        proposal.navigation_queue.push(goal);
                    }
                }
                StrategicEvent::Speech {
                    message, channel, ..
                } => {
                    proposal.speech = Some(message);
                    proposal.speech_channel = channel;
                }
                StrategicEvent::Interact { target_id } => {
                    proposal.interaction_target_id = Some(target_id);
                }
                StrategicEvent::QueueDuel => {
                    proposal
                        .actions
                        .push(super::strategic_output::StrategicAction::QueueDuel);
                }
                StrategicEvent::GoalComplete { evidence } => {
                    proposal.goal_completion_claimed = true;
                    proposal.progress_summary = evidence;
                }
                StrategicEvent::PlanUpdate { summary }
                | StrategicEvent::ThoughtCheckpoint { summary } => {
                    proposal.progress_summary = summary;
                }
            }
        }
        proposal.validate_semantics()?;
        Ok(proposal)
    }

    async fn decide_with_context(
        &self,
        input: &super::strategic_input::StrategicInput,
        _context: &BrainCallContext,
    ) -> anyhow::Result<super::strategic_output::StrategicProposal> {
        self.decide(input).await
    }
}

impl StrategicCheckpointEnvelope {
    #[must_use]
    pub const fn protocol_version() -> u32 {
        STRATEGIC_CHECKPOINT_PROTOCOL_VERSION
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_output_is_strictly_tagged_json() {
        let output = StrategicCheckpointOutput {
            protocol_version: STRATEGIC_CHECKPOINT_PROTOCOL_VERSION,
            checkpoint_id: "checkpoint-1".to_owned(),
            continue_thinking: true,
            events: vec![StrategicEvent::NavigationGoal {
                scene: "reldens-town".to_owned(),
                destination: Some(StrategicTile {
                    tile_x: 3,
                    tile_y: 4,
                }),
                reason: "leave the inn".to_owned(),
            }],
        };
        let value = serde_json::to_value(output).expect("serialize checkpoint");
        assert_eq!(value["events"][0]["type"], "navigation_goal");
        assert_eq!(value["events"][0]["destination"]["tile_x"], 3);
        assert!(value.get("agent_id").is_none());
    }

    #[test]
    fn unknown_event_types_fail_instead_of_becoming_prose() {
        let result = serde_json::from_str::<StrategicCheckpointOutput>(
            r#"{"protocol_version":2,"checkpoint_id":"x","continue_thinking":false,"events":[{"type":"mcp_call","tool":"arena_move"}]}"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn loop_coalesces_world_facts_and_keeps_dialogue() {
        let mut loop_state = StrategicAgenticLoop::with_capacity(4);
        loop_state.push(
            StrategicInboxFact {
                kind: "world".into(),
                summary: "first".into(),
                speaker: None,
                channel: None,
            },
            4,
        );
        loop_state.push(
            StrategicInboxFact {
                kind: "dialogue".into(),
                summary: "hello".into(),
                speaker: Some("Ash".into()),
                channel: Some("global".into()),
            },
            4,
        );
        loop_state.push(
            StrategicInboxFact {
                kind: "world".into(),
                summary: "latest".into(),
                speaker: None,
                channel: None,
            },
            4,
        );
        let (id, facts) = loop_state.begin().expect("checkpoint");
        assert_eq!(id, "strategic-checkpoint-1");
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].summary, "hello");
        assert_eq!(facts[1].summary, "latest");
    }

    #[test]
    fn loop_requests_next_checkpoint_without_new_message() {
        let mut loop_state = StrategicAgenticLoop::default();
        loop_state.push(
            StrategicInboxFact {
                kind: "reflection".into(),
                summary: "reconsider the plan".into(),
                speaker: None,
                channel: None,
            },
            8,
        );
        let (id, _) = loop_state.begin().expect("checkpoint");
        let output = StrategicCheckpointOutput {
            protocol_version: STRATEGIC_CHECKPOINT_PROTOCOL_VERSION,
            checkpoint_id: id,
            continue_thinking: true,
            events: vec![],
        };
        assert!(loop_state.finish(&output).expect("finish"));
        assert!(loop_state.continuation_requested());
    }
}
