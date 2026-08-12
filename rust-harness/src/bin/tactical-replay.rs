use std::{
    env,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use agent_arena_npc_harness::{
    brain::{
        Brain, BrainCallContext,
        models::{
            ModelBackgroundTasks, ModelCallObservability, ModelUsageLedger, ModelUsageTotals,
            OpenRouterJsonBrain,
        },
        prompts::{TACTICIAN_V10, TACTICIAN_V10_VERSION},
        tactical_input::TacticalInput,
    },
    execution::packet::TacticalProposal,
    observability::{AnalyticsSink, RecordingAnalyticsSink},
    replay::{
        ReplayEvaluation, TACTICAL_REPLAY_SCHEMA_VERSION, TacticalReplayFixture, evaluate_proposal,
    },
};
use anyhow::Context;
use serde::Serialize;

#[derive(Debug)]
struct Arguments {
    fixture: PathBuf,
    models: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ReplayRunRecord {
    fixture_schema_version: u32,
    case_id: String,
    model: String,
    prompt_version: String,
    latency_ms: u64,
    provider_succeeded: bool,
    error_class: Option<String>,
    proposal: Option<TacticalProposal>,
    evaluation: Option<ReplayEvaluation>,
    usage: ModelUsageTotals,
    analytics_event_count: usize,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let arguments = parse_arguments(env::args().skip(1))?;
    let fixture_text = std::fs::read_to_string(&arguments.fixture)
        .with_context(|| format!("failed to read {}", arguments.fixture.display()))?;
    let fixture: TacticalReplayFixture = serde_json::from_str(&fixture_text)
        .with_context(|| format!("invalid replay fixture {}", arguments.fixture.display()))?;
    anyhow::ensure!(
        fixture.schema_version == TACTICAL_REPLAY_SCHEMA_VERSION,
        "fixture schema version {} is not supported; expected {}",
        fixture.schema_version,
        TACTICAL_REPLAY_SCHEMA_VERSION
    );

    for model in arguments.models {
        let record = run_one(&fixture, &model).await;
        println!("{}", serde_json::to_string(&record)?);
    }
    Ok(())
}

async fn run_one(fixture: &TacticalReplayFixture, model: &str) -> ReplayRunRecord {
    if model == "scripted" {
        return ReplayRunRecord {
            fixture_schema_version: fixture.schema_version,
            case_id: fixture.case_id.clone(),
            model: model.to_owned(),
            prompt_version: "scripted/v1".to_owned(),
            latency_ms: 0,
            provider_succeeded: true,
            error_class: None,
            proposal: Some(fixture.scripted_proposal.clone()),
            evaluation: Some(evaluate_proposal(fixture, &fixture.scripted_proposal)),
            usage: ModelUsageTotals::default(),
            analytics_event_count: 0,
        };
    }

    let sink = Arc::new(RecordingAnalyticsSink::default());
    let analytics: Arc<dyn AnalyticsSink> = sink.clone();
    let ledger = Arc::new(ModelUsageLedger::default());
    let background_tasks = Arc::new(ModelBackgroundTasks::default());
    let started = Instant::now();
    let result = async {
        let key = env::var("OPENROUTER_API_KEY")
            .context("OPENROUTER_API_KEY is required for a provider replay")?;
        let brain = OpenRouterJsonBrain::<TacticalInput, TacticalProposal>::new_observed(
            &key,
            model,
            format!(
                "{TACTICIAN_V10}\n\nThe runtime supplies all identity, packet, and revision fields."
            ),
            0.1,
            150,
            ModelCallObservability::new(TACTICIAN_V10_VERSION, analytics)
                .with_role("tactician_replay")
                .with_usage_ledger(ledger.clone())
                .with_background_tasks(background_tasks.clone()),
        )?
        .with_request_timeout(replay_timeout());
        brain
            .decide_with_context(
                &TacticalInput::from(&fixture.frame),
                &BrainCallContext {
                    decision_id: uuid::Uuid::new_v4(),
                    character_id: Some(format!("replay: {}", fixture.case_id)),
                    frame_revision: Some(fixture.frame.revision),
                    strategic_revision: Some(fixture.frame.strategic_intent.revision),
                },
            )
            .await
    }
    .await;
    let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let _ = background_tasks.drain(Duration::from_secs(35)).await;
    let usage = ledger.totals_for(&format!("replay: {}", fixture.case_id));
    let events = sink.events();
    let classified_error = events.iter().rev().find_map(|event| {
        (event.name == "model.call_failed")
            .then(|| event.attributes.get("error_class"))
            .flatten()
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
    });
    let analytics_event_count = events.len();
    match result {
        Ok(proposal) => ReplayRunRecord {
            fixture_schema_version: fixture.schema_version,
            case_id: fixture.case_id.clone(),
            model: model.to_owned(),
            prompt_version: TACTICIAN_V10_VERSION.to_owned(),
            latency_ms,
            provider_succeeded: true,
            error_class: None,
            evaluation: Some(evaluate_proposal(fixture, &proposal)),
            proposal: Some(proposal),
            usage,
            analytics_event_count,
        },
        Err(_) => ReplayRunRecord {
            fixture_schema_version: fixture.schema_version,
            case_id: fixture.case_id.clone(),
            model: model.to_owned(),
            prompt_version: TACTICIAN_V10_VERSION.to_owned(),
            latency_ms,
            provider_succeeded: false,
            error_class: Some(classified_error.unwrap_or_else(|| "model_or_parse".to_owned())),
            proposal: None,
            evaluation: None,
            usage,
            analytics_event_count,
        },
    }
}

fn replay_timeout() -> Duration {
    env::var("NPC_TACTICIAN_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map_or_else(|| Duration::from_secs(5), Duration::from_millis)
}

fn parse_arguments(arguments: impl Iterator<Item = String>) -> anyhow::Result<Arguments> {
    let mut fixture = None;
    let mut models = Vec::new();
    let mut arguments = arguments.peekable();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--fixture" => {
                fixture = Some(PathBuf::from(
                    arguments.next().context("--fixture requires a path")?,
                ));
            }
            "--model" => models.push(arguments.next().context("--model requires an id")?),
            _ => anyhow::bail!("unknown tactical-replay argument {argument:?}"),
        }
    }
    anyhow::ensure!(!models.is_empty(), "at least one --model is required");
    Ok(Arguments {
        fixture: fixture.context("--fixture is required")?,
        models,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_one_fixture_and_multiple_models() {
        let arguments = parse_arguments(
            [
                "--fixture",
                "fixtures/combat/case.json",
                "--model",
                "scripted",
                "--model",
                "provider/model",
            ]
            .into_iter()
            .map(ToOwned::to_owned),
        )
        .expect("valid arguments");
        assert_eq!(arguments.models, ["scripted", "provider/model"]);
        assert_eq!(
            arguments.fixture,
            PathBuf::from("fixtures/combat/case.json")
        );
    }

    #[test]
    fn rejects_a_model_without_a_fixture() {
        let result = parse_arguments(["--model", "scripted"].into_iter().map(ToOwned::to_owned));
        assert!(result.is_err());
    }
}
