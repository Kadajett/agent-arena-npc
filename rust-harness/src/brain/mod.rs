pub mod agent;
pub mod models;
pub mod openrouter_accounting;
pub mod prompts;
pub mod strategic_agentic;
pub mod strategic_input;
pub mod strategic_intent;
pub mod strategic_output;
pub mod tactical_frame;
pub mod tactical_input;
pub mod tactical_output;

use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrainCallContext {
    pub decision_id: uuid::Uuid,
    pub character_id: Option<String>,
    pub frame_revision: Option<u64>,
    pub strategic_revision: Option<u64>,
}

impl BrainCallContext {
    #[must_use]
    pub fn standalone() -> Self {
        Self {
            decision_id: uuid::Uuid::new_v4(),
            character_id: None,
            frame_revision: None,
            strategic_revision: None,
        }
    }
}

#[async_trait]
pub trait Brain<I: Sync, O>: Send + Sync {
    async fn decide(&self, input: &I) -> anyhow::Result<O>;

    async fn decide_with_context(
        &self,
        input: &I,
        _context: &BrainCallContext,
    ) -> anyhow::Result<O> {
        self.decide(input).await
    }
}
