use std::sync::Arc;

use agent_arena_npc_harness::{
    brain::openrouter_accounting::{
        OpenRouterAccountingClient, record_generation, record_price_snapshot,
    },
    observability::{self, AnalyticsSink},
};
use anyhow::Context;
use uuid::Uuid;

const DEFAULT_MODELS: [&str; 2] = [
    "google/gemini-3.1-flash-lite",
    "openai/gpt-oss-safeguard-20b",
];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    observability::init_tracing();
    let api_key = std::env::var("OPENROUTER_API_KEY")
        .context("OPENROUTER_API_KEY is required for accounting checks")?;
    let supplied = std::env::args().skip(1).collect::<Vec<_>>();
    let client = OpenRouterAccountingClient::new(api_key);
    let analytics: Arc<dyn AnalyticsSink> = observability::tracing_sink();
    if supplied.first().map(String::as_str) == Some("--generation") {
        let generation_id = supplied
            .get(1)
            .context("--generation requires one OpenRouter generation ID")?;
        let started = std::time::Instant::now();
        let result = client.generation(generation_id).await;
        record_generation(
            &analytics,
            Some("accounting-probe"),
            Uuid::new_v4(),
            generation_id,
            1,
            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            &result,
        );
        result.context("could not fetch the finalized generation record")?;
        tracing::info!("OpenRouter generation accounting check completed");
        return Ok(());
    }
    let models = {
        if supplied.is_empty() {
            DEFAULT_MODELS.iter().map(ToString::to_string).collect()
        } else {
            supplied
        }
    };
    for model in models {
        let correlation_id = Uuid::new_v4();
        let result = client.model_endpoints(&model).await;
        record_price_snapshot(
            &analytics,
            Some("accounting-probe"),
            correlation_id,
            &model,
            &result,
        );
        let endpoints = result
            .with_context(|| format!("could not fetch current provider prices for {model}"))?
            .endpoints
            .len();
        tracing::info!(model, endpoints, "OpenRouter price snapshot completed");
    }
    Ok(())
}
