use std::sync::Arc;

use agent_arena_npc_harness::{
    HarnessConfig, PlayerRuntime,
    mcp::{HttpMcpTransport, session::ArenaSession},
    observability::{self, AnalyticsEvent, EventLevel},
    runtime::perception_recovery::ReconnectingPerceptionSource,
};

#[tokio::main]
#[allow(
    clippy::too_many_lines,
    reason = "the executable keeps session startup, supervised runtime, safety stop, accounting drain, and disconnect in one auditable lifecycle"
)]
async fn main() -> anyhow::Result<()> {
    observability::init_tracing();

    let config = HarnessConfig::from_env()?;
    let character = config.character_sheet()?;
    let character_id = character.id.clone();
    let recovery_character = Arc::new(character.clone());
    let run_duration = config.runtime.run_duration;
    let analytics = observability::tracing_sink();
    if config.runtime.tactical_rollout_mode.allows_inference() || config.models.strategist_enabled {
        // Fail before creating a game session when the selected rollout needs a model.
        config.openrouter_api_key()?;
    }
    let transport = Arc::new(HttpMcpTransport::new(
        &config.arena.mcp_url,
        &config.arena.api_key,
        config.arena.request_timeout,
        analytics.clone(),
    )?);
    let session = Arc::new(ArenaSession::new(transport, analytics.clone()));
    let session_events = session.subscribe();
    let connected = session.connect(&character).await?;
    let agent_id = connected.agent.id.clone();
    let generation = connected.generation;
    let perception_source = Arc::new(ReconnectingPerceptionSource::new(
        connected.gateway.clone(),
        session.clone(),
        recovery_character,
        config.arena.reconnect_max_attempts,
        config.arena.reconnect_initial_backoff,
        analytics.clone(),
    ));
    let runtime = PlayerRuntime::start_connected_with_session_events_and_source(
        config,
        character,
        connected.gateway,
        perception_source,
        generation,
        session_events,
        analytics.clone(),
    )
    .await?;

    tracing::info!(
        agent_id,
        session_generation = generation,
        "Rust player runtime started with a character-bound BodyActor gateway"
    );
    let (shutdown_reason, safety_stop) = if let Some(duration) = run_duration {
        tokio::select! {
            () = tokio::time::sleep(duration) => ("configured_duration_elapsed", None),
            interrupt = tokio::signal::ctrl_c() => {
                interrupt?;
                ("interrupt", None)
            },
            safety = runtime.wait_for_safety_stop() => {
                tracing::error!(reason_code = safety.reason_code(), "runtime safety stop requested");
                ("safety_stop", Some(safety))
            },
        }
    } else {
        tokio::select! {
            interrupt = tokio::signal::ctrl_c() => {
                interrupt?;
                ("interrupt", None)
            },
            safety = runtime.wait_for_safety_stop() => {
                tracing::error!(reason_code = safety.reason_code(), "runtime safety stop requested");
                ("safety_stop", Some(safety))
            },
        }
    };
    analytics.record(
        AnalyticsEvent::new("runtime.shutdown_requested", EventLevel::Info)
            .character(&character_id)
            .attribute("reason", shutdown_reason),
    );
    if let Some(stop) = safety_stop
        && let Err(error) = runtime.activate_safety_fallback(stop).await
    {
        tracing::error!(
            reason_code = stop.reason_code(),
            error_class = "safety_fallback_failed",
            %error,
            "failed to activate backend flee mode before disconnect"
        );
        analytics.record(
            AnalyticsEvent::new("runtime.safety_fallback_failed", EventLevel::Error)
                .character(&character_id)
                .attribute("reason_code", stop.reason_code())
                .attribute("error_class", "body_or_mcp_failure"),
        );
    }
    let summary = runtime.shutdown_with_reason(shutdown_reason).await?;
    tracing::info!(
        runtime_id = %summary.runtime_id,
        connected_duration_ms = summary.connected_duration_ms,
        model_calls = summary.total_usage.calls,
        exact_cost_known_calls = summary.total_usage.exact_cost_known_calls,
        openrouter_cost_usd = summary.total_usage.exact_cost_usd,
        projected_cost_per_24_connected_hours_usd = summary.projected_cost_per_24_connected_hours_usd,
        actions_succeeded = summary.telemetry.actions_succeeded,
        actions_failed = summary.telemetry.actions_failed,
        "terminal runtime summary"
    );
    session.disconnect().await?;
    Ok(())
}
