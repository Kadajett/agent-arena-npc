use std::{env, sync::Arc, time::Duration};

use agent_arena_npc_harness::{
    ControlledPacketRequest, HarnessConfig, PlayerRuntime,
    brain::tactical_frame::{EntityKind, TacticalFrame},
    execution::packet::{
        AbortCondition, TacticalAction, TacticalIntent, TacticalMode, TacticalProposal,
        TacticalStyle,
    },
    mcp::{HttpMcpTransport, session::ArenaSession},
    observability::{self, AnalyticsEvent, EventLevel},
    runtime::tactical_schedule::TacticalRolloutMode,
};
use anyhow::Context;

#[tokio::main]
#[allow(
    clippy::too_many_lines,
    reason = "one linear diagnostic keeps every live-mutation assertion visible before connection and release"
)]
async fn main() -> anyhow::Result<()> {
    observability::init_tracing();

    let config = HarnessConfig::from_env()?;
    anyhow::ensure!(
        config.runtime.tactical_rollout_mode == TacticalRolloutMode::Controlled,
        "NPC_TACTICAL_ROLLOUT_MODE must be controlled"
    );
    anyhow::ensure!(
        config.runtime.allow_live_mutation,
        "NPC_ALLOW_LIVE_MUTATION must be true"
    );
    anyhow::ensure!(
        config.runtime.live_action_budget.allows_any(),
        "NPC_LIVE_ACTION_BUDGET must be positive or unlimited"
    );
    config.openrouter_api_key()?;
    let configured_proposal = env::var("NPC_CONTROLLED_PACKET_JSON")
        .ok()
        .map(|value| serde_json::from_str(&value))
        .transpose()
        .context("NPC_CONTROLLED_PACKET_JSON is not a valid TacticalProposal")?;
    let packet_mode = env::var("NPC_CONTROLLED_PACKET_MODE").ok();
    let post_action_observation = env::var("NPC_CONTROLLED_POST_ACTION_SECONDS")
        .ok()
        .map(|value| value.parse::<u64>())
        .transpose()
        .context("NPC_CONTROLLED_POST_ACTION_SECONDS must be a nonnegative integer")?
        .map(Duration::from_secs)
        .unwrap_or_default();
    let diagnostic_max_valid_for_ms =
        u64::try_from(config.runtime.live_packet_max_age.as_millis()).unwrap_or(u64::MAX);
    anyhow::ensure!(
        configured_proposal.is_some() ^ packet_mode.is_some(),
        "set exactly one of NPC_CONTROLLED_PACKET_JSON or NPC_CONTROLLED_PACKET_MODE"
    );
    let expected_character_id = config
        .runtime
        .live_expected_character_id
        .clone()
        .context("NPC_LIVE_EXPECTED_CHARACTER_ID is required")?;
    let expected_player_name = config
        .runtime
        .live_expected_player_name
        .clone()
        .context("NPC_LIVE_EXPECTED_PLAYER_NAME is required")?;
    let expected_scene = config
        .runtime
        .live_allowed_scene
        .clone()
        .context("NPC_LIVE_ALLOWED_SCENE is required")?;
    let character = config.character_sheet()?;
    let analytics = observability::tracing_sink();
    let transport = Arc::new(HttpMcpTransport::new(
        &config.arena.mcp_url,
        &config.arena.api_key,
        config.arena.request_timeout,
        analytics.clone(),
    )?);
    let session = ArenaSession::new(transport, analytics.clone());
    let session_events = session.subscribe();
    let connected = session.connect(&character).await?;
    let runtime = PlayerRuntime::start_connected_with_session_events(
        config.clone(),
        character,
        connected.gateway,
        connected.generation,
        session_events,
        analytics.clone(),
    )
    .await;
    let runtime = match runtime {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = session.disconnect().await;
            return Err(error);
        }
    };

    let observed = tokio::time::timeout(config.arena.request_timeout, async {
        loop {
            let frame = runtime.tactical_frame();
            if frame.self_state.scene.as_deref() == Some(expected_scene.as_str())
                && frame.self_state.alive != Some(false)
            {
                break frame;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    let result = match observed {
        Ok(frame) => {
            let proposal = match configured_proposal {
                Some(proposal) => proposal,
                None => proposal_for_mode(
                    packet_mode.as_deref(),
                    &expected_character_id,
                    &frame,
                    &analytics,
                    diagnostic_max_valid_for_ms,
                )?,
            };
            let receipt = runtime
                .submit_controlled_packet(ControlledPacketRequest {
                    expected_character_id: expected_character_id.clone(),
                    expected_player_name,
                    expected_scene,
                    proposal,
                })
                .await
                .map_err(anyhow::Error::from)?;
            tokio::time::timeout(config.arena.request_timeout, async {
                loop {
                    let status = runtime.body_status().await?;
                    if status.last_terminal_packet_id == Some(receipt.packet_id) {
                        anyhow::ensure!(
                            status.last_terminal_status
                                == Some(agent_arena_npc_harness::execution::outcome::PacketTerminalStatus::Completed),
                            "controlled packet terminated with {:?}",
                            status.last_terminal_status
                        );
                        break Ok::<(), anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            })
            .await
            .context("timed out waiting for the controlled packet to terminate")??;
            if !post_action_observation.is_zero() {
                analytics.record(
                    AnalyticsEvent::new(
                        "diagnostic.post_action_observation_started",
                        EventLevel::Info,
                    )
                    .character(&expected_character_id)
                    .correlation(receipt.decision_id)
                    .attribute("packet_id", receipt.packet_id.to_string())
                    .attribute(
                        "duration_ms",
                        u64::try_from(post_action_observation.as_millis()).unwrap_or(u64::MAX),
                    ),
                );
                tokio::time::sleep(post_action_observation).await;
                let final_frame = runtime.tactical_frame();
                analytics.record(
                    AnalyticsEvent::new(
                        "diagnostic.post_action_observation_completed",
                        EventLevel::Info,
                    )
                    .character(&expected_character_id)
                    .correlation(receipt.decision_id)
                    .attribute("packet_id", receipt.packet_id.to_string())
                    .attribute("frame_revision", final_frame.revision)
                    .attribute("perception_revision", final_frame.perception_revision)
                    .attribute("combat_active_known", final_frame.combat.active.is_some())
                    .attribute("combat_active", final_frame.combat.active.unwrap_or(false))
                    .attribute(
                        "hostiles_targeting_self_count",
                        u64::try_from(
                            final_frame
                                .nearby_entities
                                .iter()
                                .filter(|entity| entity.targeting_you == Some(true))
                                .count(),
                        )
                        .unwrap_or(u64::MAX),
                    )
                    .attribute("health_known", final_frame.self_state.health.is_some())
                    .attribute(
                        "health",
                        i64::from(final_frame.self_state.health.unwrap_or(0)),
                    )
                    .attribute(
                        "damage_received_last_five_seconds_known",
                        final_frame
                            .combat
                            .damage_received_last_five_seconds
                            .is_some(),
                    )
                    .attribute(
                        "damage_received_last_five_seconds",
                        final_frame
                            .combat
                            .damage_received_last_five_seconds
                            .unwrap_or(0),
                    )
                    .attribute(
                        "damage_dealt_last_five_seconds_known",
                        final_frame.combat.damage_dealt_last_five_seconds.is_some(),
                    )
                    .attribute(
                        "damage_dealt_last_five_seconds",
                        final_frame
                            .combat
                            .damage_dealt_last_five_seconds
                            .unwrap_or(0),
                    )
                    .attribute(
                        "drop_count",
                        u64::try_from(final_frame.nearby_drops.len()).unwrap_or(u64::MAX),
                    ),
                );
            }
            Ok(receipt)
        }
        Err(error) => Err(anyhow::Error::from(error).context(
            "timed out waiting for the asserted live scene and a non-dead player observation",
        )),
    };

    runtime.shutdown().await?;
    session.disconnect().await?;
    let receipt = result?;
    tracing::info!(
        decision_id = %receipt.decision_id,
        packet_id = %receipt.packet_id,
        frame_revision = receipt.frame_revision,
        strategic_revision = receipt.strategic_revision,
        action_count = receipt.action_count,
        remaining_live_action_budget = receipt.remaining_live_action_budget.unwrap_or(0),
        live_action_budget_unlimited = receipt.live_action_budget_unlimited,
        "controlled diagnostic packet submitted"
    );
    Ok(())
}

#[allow(
    clippy::too_many_lines,
    reason = "the explicit diagnostic keeps target selection, reduced telemetry, and the one test packet together"
)]
fn proposal_for_mode(
    mode: Option<&str>,
    character_id: &str,
    frame: &TacticalFrame,
    analytics: &std::sync::Arc<dyn agent_arena_npc_harness::observability::AnalyticsSink>,
    max_valid_for_ms: u64,
) -> anyhow::Result<TacticalProposal> {
    anyhow::ensure!(
        matches!(
            mode,
            Some(
                "attack_nearest_hostile"
                    | "approach_nearest_hostile"
                    | "step_to_nearest_walkable"
                    | "flee"
            )
        ),
        "NPC_CONTROLLED_PACKET_MODE must be attack_nearest_hostile, approach_nearest_hostile, step_to_nearest_walkable, or flee"
    );
    if mode == Some("flee") {
        analytics.record(
            AnalyticsEvent::new(
                "diagnostic.controlled_safety_mode_selected",
                EventLevel::Warn,
            )
            .character(character_id)
            .attribute("selection", "set_tactics_flee_semi_auto")
            .attribute("frame_revision", frame.revision)
            .attribute("strategic_revision", frame.strategic_intent.revision)
            .attribute("combat_active_known", frame.combat.active.is_some())
            .attribute("combat_active", frame.combat.active.unwrap_or(false)),
        );
        return Ok(TacticalProposal {
            intent: TacticalIntent::Disengage,
            actions: vec![TacticalAction::SetTactics {
                style: TacticalStyle::Flee,
                mode: TacticalMode::SemiAuto,
            }],
            valid_for_ms: bounded_validity(1_000, max_valid_for_ms)?,
            abort_if: vec![AbortCondition::SceneChanged, AbortCondition::PlayerDied],
            rationale: None,
        });
    }
    if mode == Some("step_to_nearest_walkable") {
        let self_tile = frame
            .self_state
            .position
            .map(|position| position.tile)
            .context("the controlled step requires an authoritative self tile")?;
        let destination = frame
            .map
            .tiles
            .iter()
            .filter(|tile| tile.walkable == Some(true) && tile.position != self_tile)
            .min_by_key(|tile| squared_tile_distance(tile.position, self_tile))
            .map(|tile| tile.position)
            .context("no traversable local destination is available")?;
        analytics.record(
            AnalyticsEvent::new("diagnostic.controlled_step_selected", EventLevel::Info)
                .character(character_id)
                .attribute("frame_revision", frame.revision)
                .attribute("strategic_revision", frame.strategic_intent.revision)
                .attribute("origin_tile_x", i64::from(self_tile.x))
                .attribute("origin_tile_y", i64::from(self_tile.y))
                .attribute("destination_tile_x", i64::from(destination.x))
                .attribute("destination_tile_y", i64::from(destination.y)),
        );
        return Ok(TacticalProposal {
            intent: TacticalIntent::Reposition,
            actions: vec![TacticalAction::MoveTo {
                tile_x: destination.x,
                tile_y: destination.y,
            }],
            valid_for_ms: bounded_validity(5_000, max_valid_for_ms)?,
            abort_if: vec![AbortCondition::SceneChanged, AbortCondition::PlayerDied],
            rationale: None,
        });
    }
    let candidate_count = frame
        .nearby_entities
        .iter()
        .filter(|entity| {
            entity.kind == EntityKind::Enemy
                && entity.hostile == Some(true)
                && entity.alive != Some(false)
        })
        .count();
    let target = frame
        .nearby_entities
        .iter()
        .filter(|entity| {
            entity.kind == EntityKind::Enemy
                && entity.hostile == Some(true)
                && entity.alive != Some(false)
        })
        .min_by(|left, right| match (left.distance, right.distance) {
            (Some(left), Some(right)) => left.total_cmp(&right),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => left.id.cmp(&right.id),
        })
        .context("no visible live hostile is available for the controlled attack")?;
    if mode == Some("approach_nearest_hostile") {
        let self_tile = frame
            .self_state
            .position
            .map(|position| position.tile)
            .context("the controlled approach requires an authoritative self tile")?;
        let target_tile = target
            .tile
            .context("the nearest hostile has no authoritative tile")?;
        let destination = frame
            .map
            .tiles
            .iter()
            .filter(|tile| tile.walkable == Some(true))
            .filter(|tile| {
                (tile.position.x - self_tile.x)
                    .abs()
                    .max((tile.position.y - self_tile.y).abs())
                    <= 8
            })
            .min_by_key(|tile| squared_tile_distance(tile.position, target_tile))
            .map(|tile| tile.position)
            .filter(|destination| *destination != self_tile)
            .context("no closer traversable local tile is available")?;
        analytics.record(
            AnalyticsEvent::new("diagnostic.controlled_target_selected", EventLevel::Info)
                .character(character_id)
                .attribute("selection", "approach_nearest_visible_hostile")
                .attribute("frame_revision", frame.revision)
                .attribute("strategic_revision", frame.strategic_intent.revision)
                .attribute(
                    "candidate_count",
                    u64::try_from(candidate_count).unwrap_or(u64::MAX),
                )
                .attribute("distance_known", target.distance.is_some())
                .attribute("distance_tiles", f64::from(target.distance.unwrap_or(0.0)))
                .attribute("destination_tile_x", i64::from(destination.x))
                .attribute("destination_tile_y", i64::from(destination.y)),
        );
        return Ok(TacticalProposal {
            intent: TacticalIntent::Reposition,
            actions: vec![TacticalAction::MoveTo {
                tile_x: destination.x,
                tile_y: destination.y,
            }],
            valid_for_ms: bounded_validity(5_000, max_valid_for_ms)?,
            abort_if: vec![AbortCondition::SceneChanged, AbortCondition::PlayerDied],
            rationale: None,
        });
    }
    analytics.record(
        AnalyticsEvent::new("diagnostic.controlled_target_selected", EventLevel::Info)
            .character(character_id)
            .attribute("selection", "nearest_visible_hostile")
            .attribute("frame_revision", frame.revision)
            .attribute("strategic_revision", frame.strategic_intent.revision)
            .attribute(
                "candidate_count",
                u64::try_from(candidate_count).unwrap_or(u64::MAX),
            )
            .attribute("distance_known", target.distance.is_some())
            .attribute("distance_tiles", f64::from(target.distance.unwrap_or(0.0))),
    );
    Ok(TacticalProposal {
        intent: TacticalIntent::Attack,
        actions: vec![TacticalAction::Attack {
            target_id: target.id.clone(),
        }],
        valid_for_ms: bounded_validity(1_500, max_valid_for_ms)?,
        abort_if: vec![
            AbortCondition::TargetUnavailable,
            AbortCondition::SceneChanged,
            AbortCondition::PlayerDied,
        ],
        rationale: None,
    })
}

fn bounded_validity(desired_ms: u64, maximum_ms: u64) -> anyhow::Result<u64> {
    let valid_for_ms = desired_ms.min(maximum_ms);
    anyhow::ensure!(
        valid_for_ms >= 100,
        "NPC_LIVE_PACKET_MAX_AGE_MS must be at least 100 for a generated diagnostic packet"
    );
    Ok(valid_for_ms)
}

fn squared_tile_distance(
    left: agent_arena_npc_harness::world::TilePosition,
    right: agent_arena_npc_harness::world::TilePosition,
) -> i64 {
    let dx = i64::from(left.x) - i64::from(right.x);
    let dy = i64::from(left.y) - i64::from(right.y);
    dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy))
}

#[cfg(test)]
mod tests {
    use super::bounded_validity;

    #[test]
    fn generated_packet_lifetime_obeys_the_runtime_gate() {
        assert_eq!(bounded_validity(5_000, 1_000).expect("valid limit"), 1_000);
    }

    #[test]
    fn generated_packet_rejects_an_impossibly_short_runtime_gate() {
        assert!(bounded_validity(1_000, 99).is_err());
    }
}
