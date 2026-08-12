use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use agent_arena_npc_harness::{
    HarnessConfig,
    brain::strategic_intent::StrategicIntent,
    mcp::{
        HttpMcpTransport,
        session::ArenaSession,
        tools,
        types::{
            CombatTarget, HistoryQuery, InventoryResult, MelodyInstrument, MoveDirection,
            SurveyResult,
        },
    },
    observability::{self, AnalyticsEvent, AnalyticsSink, EventLevel},
    world::{
        PixelPosition, TilePosition,
        dialogue::{DialogueChannel, normalize_dialogue},
        map::TileKind,
        perception::{PerceptionEngine, PerceptionInput, PerceptionSummary, PerceptionUpdate},
    },
};
use anyhow::{Context, bail};
use chrono::Utc;
use num_traits::ToPrimitive;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    observability::init_tracing();
    let config = HarnessConfig::from_env()?;
    let character = config.character_sheet()?;
    let analytics = observability::tracing_sink();
    let transport = Arc::new(HttpMcpTransport::new(
        &config.arena.mcp_url,
        &config.arena.api_key,
        config.arena.request_timeout,
        analytics.clone(),
    )?);
    let session = ArenaSession::new(transport, analytics.clone());
    let connected = session.connect(&character).await?;
    tracing::info!(
        player_name = %connected.agent.player_name,
        class_path = connected.agent.class_path.as_deref().unwrap_or("unknown"),
        "MCP smoke character connected"
    );
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let result = run_command(
        &connected.gateway,
        &session,
        &arguments,
        &character.id,
        &analytics,
    )
    .await;
    let disconnect_result = session.disconnect().await;
    result?;
    disconnect_result?;
    Ok(())
}

#[allow(
    clippy::too_many_lines,
    reason = "the explicit smoke-command dispatcher keeps mutation commands visible in one audit point"
)]
async fn run_command(
    gateway: &agent_arena_npc_harness::mcp::ArenaGateway,
    session: &ArenaSession,
    arguments: &[String],
    character_id: &str,
    analytics: &Arc<dyn AnalyticsSink>,
) -> anyhow::Result<()> {
    let command = arguments.first().map_or("read-only", String::as_str);
    match command {
        "tool-inventory" => {
            let advertised = session.list_tool_names().await?;
            let advertised_set = advertised
                .iter()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<_>>();
            let expected_set = tools::EXPECTED_PRODUCTION_TOOLS
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>();
            let missing = expected_set
                .difference(&advertised_set)
                .copied()
                .collect::<Vec<_>>();
            let unexpected = advertised_set
                .difference(&expected_set)
                .copied()
                .collect::<Vec<_>>();
            analytics.record(
                AnalyticsEvent::new("diagnostic.production_tool_inventory", EventLevel::Info)
                    .character(character_id)
                    .attribute("advertised_tool_count", as_u64(advertised.len()))
                    .attribute("expected_tool_count", as_u64(expected_set.len()))
                    .attribute("missing_tool_count", as_u64(missing.len()))
                    .attribute("unexpected_tool_count", as_u64(unexpected.len()))
                    .attribute("missing_tools", missing.clone())
                    .attribute("unexpected_tools", unexpected.clone()),
            );
            if !missing.is_empty() || !unexpected.is_empty() {
                bail!(
                    "production MCP tool inventory drifted: missing={missing:?}, unexpected={unexpected:?}"
                );
            }
            tracing::info!(
                advertised_tool_count = advertised.len(),
                "production MCP tool inventory matches the typed harness surface"
            );
        }
        "read-only" => {
            let observation = gateway.observe().await?;
            let map = gateway.render_map(16).await?;
            let inventory: InventoryResult = gateway.inventory().await?;
            let self_state = observation
                .own_player
                .as_ref()
                .and_then(|player| player.state.as_ref());
            tracing::info!(
                scene = self_state
                    .and_then(|state| state.scene.as_deref())
                    .or(observation.scene_name.as_deref())
                    .unwrap_or("unknown"),
                class_path = self_state
                    .and_then(|state| state.class_path.as_deref())
                    .unwrap_or("unknown"),
                health = self_state
                    .and_then(|state| state.health)
                    .unwrap_or_default(),
                visible_objects = observation.objects.len(),
                visible_players = observation.players.len(),
                inventory_items = inventory.carrying.len(),
                map_available = map.grid_available.unwrap_or(false),
                doors = map.doors.len(),
                "read-only MCP smoke test passed"
            );
        }
        "history" => {
            let limit = arguments
                .get(1)
                .map(|value| {
                    value
                        .parse::<u16>()
                        .context("history limit must be an integer")
                })
                .transpose()?
                .unwrap_or(20);
            let page = gateway
                .history(&HistoryQuery {
                    limit: Some(limit),
                    ..HistoryQuery::default()
                })
                .await?;
            tracing::info!(
                event_count = page.events.len(),
                cursor = page.cursor,
                oldest = page.oldest,
                has_more = page.has_more,
                summarized_event_count = page.summary.event_count,
                scene_count = page.summary.scenes.len(),
                movement_commands = page.summary.movement.commands,
                damage_dealt = page.summary.combat.damage_dealt,
                damage_taken = page.summary.combat.damage_taken,
                "durable history MCP smoke test passed without logging event payloads"
            );
        }
        "history-roundtrip" => {
            run_history_roundtrip(gateway, character_id, analytics).await?;
        }
        "say" => {
            let message = arguments.get(1).context("say requires one message")?;
            gateway.say(message).await?;
            tracing::info!("speech MCP smoke test passed");
        }
        "say-global" => {
            let message = arguments
                .get(1)
                .context("say-global requires one message")?;
            let result = gateway.say_global(message).await?;
            tracing::info!(
                accepted = result.accepted.unwrap_or(false),
                channel = result.channel.as_deref().unwrap_or("unknown"),
                "global speech MCP smoke test passed"
            );
        }
        "say-private" => {
            let recipient = arguments
                .get(1)
                .context("say-private requires a player name and one message")?;
            let message = arguments
                .get(2)
                .context("say-private requires a player name and one message")?;
            let result = gateway.say_private(recipient, message).await?;
            tracing::info!(
                accepted = result.accepted.unwrap_or(false),
                channel = result.channel.as_deref().unwrap_or("unknown"),
                recipient_confirmed = result.to_player.as_deref() == Some(recipient.as_str()),
                "private speech MCP smoke test passed"
            );
        }
        "chat-read" => {
            let observation = gateway.observe().await?;
            let dialogue = normalize_dialogue(&observation);
            let counts = chat_counts(&dialogue.lines);
            record_chat_read(
                character_id,
                analytics,
                &counts,
                dialogue.lines.len(),
                dialogue.filtered_count,
                false,
            );
            tracing::info!(
                line_count = dialogue.lines.len(),
                scene_count = counts.scene,
                global_count = counts.global,
                private_count = counts.private,
                team_count = counts.team,
                unknown_count = counts.unknown,
                "production chat read completed without logging message content"
            );
        }
        "chat-roundtrip" => {
            let initial = gateway.observe().await?;
            let player_name = initial
                .own_player
                .as_ref()
                .and_then(|player| {
                    player
                        .player_name
                        .as_deref()
                        .or(player.name.as_deref())
                        .or(player.label.as_deref())
                })
                .context("the observation did not include the bound player name")?;
            gateway
                .say("Cassian checks the nearby acoustics once more.")
                .await?;
            gateway
                .say_global("Cassian confirms that the world stage can hear him.")
                .await?;
            gateway
                .say_private(player_name, "Private rehearsal note received.")
                .await?;
            tokio::time::sleep(Duration::from_millis(750)).await;
            let observation = gateway.observe().await?;
            let dialogue = normalize_dialogue(&observation);
            let counts = chat_counts(&dialogue.lines);
            record_chat_read(
                character_id,
                analytics,
                &counts,
                dialogue.lines.len(),
                dialogue.filtered_count,
                true,
            );
            if counts.scene == 0 || counts.global == 0 || counts.private == 0 {
                bail!(
                    "chat round trip did not observe every written channel: scene={}, global={}, private={}",
                    counts.scene,
                    counts.global,
                    counts.private
                );
            }
            tracing::info!(
                scene_count = counts.scene,
                global_count = counts.global,
                private_count = counts.private,
                team_count = counts.team,
                "production chat write/read round trip passed"
            );
        }
        "play-melody" => {
            let instrument = parse_instrument(arguments.get(1))?;
            let times = arguments
                .get(2)
                .context("play-melody requires an instrument, repeat count, and melody")?
                .parse::<u8>()
                .context("play-melody repeat count must be an integer from 1 through 4")?;
            let melody = arguments
                .get(3)
                .context("play-melody requires an instrument, repeat count, and melody")?;
            let result = gateway.play_melody(melody, times, instrument).await?;
            tokio::time::sleep(Duration::from_millis(500)).await;
            let observation = gateway.observe().await?;
            let heard_melody_count = normalize_dialogue(&observation)
                .lines
                .iter()
                .filter(|line| {
                    line.kind == agent_arena_npc_harness::world::dialogue::DialogueKind::Melody
                })
                .count();
            analytics.record(
                AnalyticsEvent::new("diagnostic.production_melody_read", EventLevel::Info)
                    .character(character_id)
                    .attribute("heard_melody_count", as_u64(heard_melody_count))
                    .attribute("instrument", instrument.as_str()),
            );
            if heard_melody_count == 0 {
                bail!("the melody was accepted but did not return through perception");
            }
            tracing::info!(
                accepted = result.accepted.unwrap_or(false),
                played = result.played.unwrap_or(false),
                instrument = instrument.as_str(),
                times,
                note_count = melody.split_whitespace().count(),
                heard_melody_count,
                "production melody performance passed"
            );
        }
        "move-to" => {
            let x = parse_coordinate(arguments.get(1), "x")?;
            let y = parse_coordinate(arguments.get(2), "y")?;
            gateway.move_to(PixelPosition { x, y }).await?;
            tracing::info!(x, y, "movement MCP smoke test passed");
        }
        "move" => {
            let direction = parse_direction(arguments.get(1))?;
            gateway.move_direction(direction).await?;
            tracing::info!(?direction, "directional movement MCP smoke test passed");
        }
        "live-walk" => {
            run_live_walk(gateway).await?;
        }
        "live-perception" => {
            let cycle_limit = arguments
                .get(1)
                .map(|value| {
                    value
                        .parse::<u32>()
                        .context("live-perception cycle limit must be a positive integer")
                })
                .transpose()?
                .filter(|value| *value > 0);
            run_live_perception(gateway, character_id, analytics, cycle_limit).await?;
        }
        "map-shape" => {
            let map = gateway.render_map(16).await?;
            record_map_shape(&map, character_id, analytics);
            tracing::info!("map shape diagnostic completed");
        }
        "travel-bots-forest" => {
            travel_to_bots_forest(gateway, character_id, analytics).await?;
        }
        "attack-object" => {
            let object_index = arguments
                .get(1)
                .context("attack-object requires one observed object index")?;
            gateway
                .basic_attack(&CombatTarget::Object {
                    object_index: object_index.clone(),
                })
                .await?;
            tracing::info!("combat MCP smoke test passed");
        }
        _ => bail!(
            "unknown smoke command; use read-only, history [limit], history-roundtrip, say <message>, say-global <message>, say-private <player-name> <message>, chat-read, chat-roundtrip, play-melody <lute|flute|horn|bell> <times> <melody>, move <up|down|left|right>, live-walk, live-perception, map-shape, travel-bots-forest, move-to <x> <y>, or attack-object <object-index>"
        ),
    }
    Ok(())
}

async fn run_history_roundtrip(
    gateway: &agent_arena_npc_harness::mcp::ArenaGateway,
    character_id: &str,
    analytics: &Arc<dyn AnalyticsSink>,
) -> anyhow::Result<()> {
    let since = Utc::now().to_rfc3339();
    let mut engine = PerceptionEngine::default();
    let before = capture_perception(gateway, &mut engine, character_id, analytics, 1).await?;
    let before_position = before
        .frame
        .self_state
        .position
        .context("history round trip requires an authoritative starting position")?;
    let target = find_reachable_navigation_target(gateway, &before, true)
        .await?
        .context("history round trip found no reachable production target")?;
    let move_result = gateway.move_to(tile_center(target)).await?;
    let after = capture_perception(gateway, &mut engine, character_id, analytics, 2).await?;
    let after_position = after
        .frame
        .self_state
        .position
        .context("history round trip requires an authoritative ending position")?;
    if before_position == after_position || after_position.tile != target {
        bail!(
            "production navigation did not reach the history round-trip target: before={:?}, after={:?}, target={target:?}",
            before_position.tile,
            after_position.tile
        );
    }

    let mut captured = None;
    for _ in 0..10 {
        let page = gateway
            .history(&HistoryQuery {
                since: Some(since.clone()),
                limit: Some(100),
                ..HistoryQuery::default()
            })
            .await?;
        captured = page.events.into_iter().find(|event| {
            event.event_type == "movement" && event.tool.as_deref() == Some(tools::MOVE_TO)
        });
        if captured.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    let event = captured.context("production history did not return the completed movement")?;
    let scene_matches = event.scene == after.frame.self_state.scene;
    analytics.record(
        AnalyticsEvent::new("diagnostic.production_history_roundtrip", EventLevel::Info)
            .character(character_id)
            .attribute("movement_event_id", event.id)
            .attribute("decision_id_present", event.decision_id.is_some())
            .attribute("scene_present", event.scene.is_some())
            .attribute("scene_matches", scene_matches)
            .attribute("position_changed", true)
            .attribute("reached_target", true)
            .attribute("backend_arrived", move_result.arrived.unwrap_or(false)),
    );
    if event.decision_id.is_none() || !scene_matches {
        bail!(
            "production history movement lacked required lineage: decision_id={}, scene_matches={scene_matches}",
            event.decision_id.is_some()
        );
    }
    tracing::info!(
        movement_event_id = event.id,
        decision_id_present = true,
        scene_matches,
        before_tile_x = before_position.tile.x,
        before_tile_y = before_position.tile.y,
        after_tile_x = after_position.tile.x,
        after_tile_y = after_position.tile.y,
        "production movement is durably queryable through arena_history"
    );
    Ok(())
}

async fn travel_to_bots_forest(
    gateway: &agent_arena_npc_harness::mcp::ArenaGateway,
    character_id: &str,
    analytics: &Arc<dyn AnalyticsSink>,
) -> anyhow::Result<()> {
    const DESTINATION: &str = "reldens-bots-forest";
    for route_step in 1_u64..=2 {
        let observation = gateway.observe().await?;
        let scene = observation
            .own_player
            .as_ref()
            .and_then(|player| player.state.as_ref())
            .and_then(|state| state.scene.as_deref())
            .or(observation.scene_name.as_deref())
            .context("route observation did not identify the current scene")?;
        if scene == DESTINATION {
            break;
        }
        let next_scene = match scene {
            "reldens-town" => "reldens-bots",
            "reldens-bots" => DESTINATION,
            other => bail!("bot-forest route does not start from scene {other:?}"),
        };
        let survey = gateway.survey(None).await?;
        let way = survey
            .ways_out
            .iter()
            .find(|way| way.leads_to.as_deref() == Some(next_scene))
            .with_context(|| format!("scene {scene:?} did not report a door to {next_scene:?}"))?;
        if way.locked == Some(true) {
            bail!("the route door from {scene:?} to {next_scene:?} is locked");
        }
        let enter_at = way.enter_at.context("route door had no enter-at tile")?;
        let mut entered = false;
        for attempt in 1_u64..=3 {
            let started = Instant::now();
            let result = gateway
                .enter_door_tile(TilePosition {
                    x: enter_at.column,
                    y: enter_at.row,
                })
                .await?;
            let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            analytics.record(
                AnalyticsEvent::new("diagnostic.production_route_step", EventLevel::Info)
                    .character(character_id)
                    .attribute("route_step", route_step)
                    .attribute("attempt", attempt)
                    .attribute("from_scene", scene)
                    .attribute("expected_scene", next_scene)
                    .attribute("reported_scene", result.scene.clone().unwrap_or_default())
                    .attribute("entered", result.entered.unwrap_or(false))
                    .attribute("duration_ms", duration_ms)
                    .attribute(
                        "reason_code",
                        result.reason.clone().unwrap_or_else(|| "none".to_owned()),
                    ),
            );
            if result.entered == Some(true) && result.scene.as_deref() == Some(next_scene) {
                entered = true;
                break;
            }
            if result.reason.as_deref() != Some("DOOR_TOO_FAR") {
                bail!(
                    "door from {scene:?} to {next_scene:?} failed with {:?}",
                    result.reason
                );
            }
        }
        if !entered {
            bail!("door from {scene:?} to {next_scene:?} exceeded three bounded attempts");
        }
    }

    let destination = gateway.survey(None).await?;
    if destination.scene_name.as_deref() != Some(DESTINATION) {
        bail!(
            "route ended in {:?}, expected {DESTINATION:?}",
            destination.scene_name
        );
    }
    let counts = destination.counts.unwrap_or_default();
    analytics.record(
        AnalyticsEvent::new("diagnostic.production_route_completed", EventLevel::Info)
            .character(character_id)
            .attribute("scene", DESTINATION)
            .attribute("enemy_count", counts.enemies.unwrap_or(0))
            .attribute("way_out_count", counts.ways_out.unwrap_or(0)),
    );
    tracing::info!(
        scene = DESTINATION,
        enemy_count = counts.enemies.unwrap_or(0),
        "Cassian reached the production bot forest"
    );
    Ok(())
}

#[derive(Debug, Default)]
struct ChatCounts {
    scene: usize,
    global: usize,
    private: usize,
    team: usize,
    unknown: usize,
}

fn chat_counts(lines: &[agent_arena_npc_harness::world::dialogue::DialogueLine]) -> ChatCounts {
    let mut counts = ChatCounts::default();
    for line in lines {
        match line.channel {
            DialogueChannel::Scene => counts.scene += 1,
            DialogueChannel::Global => counts.global += 1,
            DialogueChannel::Private => counts.private += 1,
            DialogueChannel::Team => counts.team += 1,
            DialogueChannel::Unknown => counts.unknown += 1,
        }
    }
    counts
}

fn record_chat_read(
    character_id: &str,
    analytics: &Arc<dyn AnalyticsSink>,
    counts: &ChatCounts,
    line_count: usize,
    filtered_count: usize,
    roundtrip: bool,
) {
    analytics.record(
        AnalyticsEvent::new("diagnostic.production_chat_read", EventLevel::Info)
            .character(character_id)
            .attribute("roundtrip", roundtrip)
            .attribute("line_count", as_u64(line_count))
            .attribute("scene_count", as_u64(counts.scene))
            .attribute("global_count", as_u64(counts.global))
            .attribute("private_count", as_u64(counts.private))
            .attribute("team_count", as_u64(counts.team))
            .attribute("unknown_count", as_u64(counts.unknown))
            .attribute("filtered_count", as_u64(filtered_count)),
    );
}

fn record_map_shape(
    map: &agent_arena_npc_harness::mcp::types::MapObservation,
    character_id: &str,
    analytics: &Arc<dyn AnalyticsSink>,
) {
    let scene_width = map
        .scene_size
        .as_ref()
        .and_then(|scene| scene.width_tiles)
        .unwrap_or_default();
    let scene_height = map
        .scene_size
        .as_ref()
        .and_then(|scene| scene.height_tiles)
        .unwrap_or_default();
    for (line_index, line) in map.map.as_deref().unwrap_or_default().lines().enumerate() {
        let mut wall_count = 0_u64;
        let mut floor_count = 0_u64;
        let mut self_count = 0_u64;
        let mut player_count = 0_u64;
        let mut npc_count = 0_u64;
        let mut enemy_count = 0_u64;
        let mut door_count = 0_u64;
        let mut locked_door_count = 0_u64;
        let mut space_count = 0_u64;
        let mut other_count = 0_u64;
        for character in line.chars() {
            match character {
                '#' => wall_count += 1,
                '.' => floor_count += 1,
                '@' => self_count += 1,
                'P' => player_count += 1,
                'N' => npc_count += 1,
                'E' => enemy_count += 1,
                'D' => door_count += 1,
                'L' => locked_door_count += 1,
                ' ' => space_count += 1,
                _ => other_count += 1,
            }
        }
        analytics.record(
            AnalyticsEvent::new("diagnostic.map_line_shape", EventLevel::Debug)
                .character(character_id)
                .attribute("line_index", u64::try_from(line_index).unwrap_or(u64::MAX))
                .attribute(
                    "character_count",
                    u64::try_from(line.chars().count()).unwrap_or(u64::MAX),
                )
                .attribute("wall_count", wall_count)
                .attribute("floor_count", floor_count)
                .attribute("self_count", self_count)
                .attribute("player_count", player_count)
                .attribute("npc_count", npc_count)
                .attribute("enemy_count", enemy_count)
                .attribute("door_count", door_count)
                .attribute("locked_door_count", locked_door_count)
                .attribute("space_count", space_count)
                .attribute("other_count", other_count)
                .attribute("scene_width", u64::from(scene_width))
                .attribute("scene_height", u64::from(scene_height)),
        );
    }
}

async fn run_live_perception(
    gateway: &agent_arena_npc_harness::mcp::ArenaGateway,
    character_id: &str,
    analytics: &Arc<dyn AnalyticsSink>,
    cycle_limit: Option<u32>,
) -> anyhow::Result<()> {
    let mut prefer_west = true;
    let mut completed_cycles = 0_u32;
    let mut engine = PerceptionEngine::default();
    let survey = gateway.survey(None).await?;
    record_survey_diagnostic(&survey, character_id, analytics);
    let mut current = capture_perception(gateway, &mut engine, character_id, analytics, 1).await?;
    let (shutdown_sender, mut shutdown_receiver) = tokio::sync::watch::channel(false);
    let shutdown_listener = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            let _ = shutdown_sender.send(true);
        }
    });
    tracing::info!("live perception walk started; press Ctrl-C to stop and disconnect");
    loop {
        tokio::select! {
            changed = shutdown_receiver.changed() => {
                changed?;
                break;
            }
            () = tokio::time::sleep(Duration::from_secs(3)) => {
                let before = current.frame.self_state.position;
                let target = find_reachable_navigation_target(gateway, &current, prefer_west).await?;
                prefer_west = !prefer_west;
                let Some(target) = target else {
                    tracing::warn!("no safe reachable production navigation target was found");
                    continue;
                };
                let movement = gateway.move_to(tile_center(target)).await;
                current = capture_perception(
                    gateway,
                    &mut engine,
                    character_id,
                    analytics,
                    u64::from(completed_cycles).saturating_add(2),
                ).await?;
                record_navigation_result(
                    character_id,
                    analytics,
                    Some(target),
                    before,
                    current.frame.self_state.position,
                    movement.as_ref().ok(),
                );
                if movement.is_err() {
                    tracing::warn!(?target, error_class = "tool_or_transport", "live test movement failed");
                }
                completed_cycles = completed_cycles.saturating_add(1);
                if cycle_limit.is_some_and(|limit| completed_cycles >= limit) {
                    tracing::info!(completed_cycles, "live perception walk reached its cycle limit");
                    break;
                }
            }
        }
    }
    shutdown_listener.abort();
    tracing::info!("live perception walk stopping");
    Ok(())
}

fn record_survey_diagnostic(
    survey: &SurveyResult,
    character_id: &str,
    analytics: &Arc<dyn AnalyticsSink>,
) {
    let counts = survey.counts.as_ref();
    let event = AnalyticsEvent::new("diagnostic.production_survey", EventLevel::Info)
        .character(character_id)
        .attribute("scene_known", survey.scene_name.is_some())
        .attribute("grid_available", survey.grid_available.unwrap_or(false))
        .attribute(
            "enemy_count",
            counts.and_then(|counts| counts.enemies).unwrap_or(0),
        )
        .attribute(
            "people_count",
            counts.and_then(|counts| counts.people).unwrap_or(0),
        )
        .attribute(
            "readable_count",
            counts.and_then(|counts| counts.readables).unwrap_or(0),
        )
        .attribute(
            "drop_count",
            counts.and_then(|counts| counts.drops).unwrap_or(0),
        )
        .attribute(
            "other_player_count",
            counts.and_then(|counts| counts.other_players).unwrap_or(0),
        )
        .attribute(
            "way_out_count",
            counts.and_then(|counts| counts.ways_out).unwrap_or(0),
        )
        .attribute("structured_way_out_count", as_u64(survey.ways_out.len()))
        .attribute(
            "locked_way_out_count",
            as_u64(
                survey
                    .ways_out
                    .iter()
                    .filter(|way| way.locked == Some(true))
                    .count(),
            ),
        )
        .attribute(
            "contains_locked_door_marker",
            survey.survey.contains("<LOCKED DOOR>"),
        );
    analytics.record(event);
}

async fn capture_perception(
    gateway: &agent_arena_npc_harness::mcp::ArenaGateway,
    engine: &mut PerceptionEngine,
    character_id: &str,
    analytics: &Arc<dyn AnalyticsSink>,
    observation_cycle_sequence: u64,
) -> anyhow::Result<PerceptionUpdate> {
    let observation = gateway.observe().await?;
    let map = gateway.render_map(16).await?;
    let inventory = gateway.inventory().await?;
    let source_scene_width = map.scene_size.as_ref().and_then(|scene| scene.width_tiles);
    let source_scene_height = map.scene_size.as_ref().and_then(|scene| scene.height_tiles);
    let update = engine.update(PerceptionInput {
        observation_cycle_id: uuid::Uuid::new_v4(),
        observation_cycle_sequence,
        observation,
        map,
        inventory: Some(inventory),
        strategic_intent: StrategicIntent::default(),
        observed_at: Utc::now(),
    })?;
    record_perception_diagnostic(
        &update,
        source_scene_width,
        source_scene_height,
        character_id,
        analytics,
    );
    log_perception_summary(&update, source_scene_width, source_scene_height);
    Ok(update)
}

fn record_navigation_result(
    character_id: &str,
    analytics: &Arc<dyn AnalyticsSink>,
    target: Option<TilePosition>,
    before: Option<agent_arena_npc_harness::world::Position>,
    after: Option<agent_arena_npc_harness::world::Position>,
    result: Option<&agent_arena_npc_harness::mcp::types::MoveResult>,
) {
    let progressed = before
        .zip(after)
        .is_some_and(|(before, after)| before != after);
    let changed_tile = before
        .zip(after)
        .is_some_and(|(before, after)| before.tile != after.tile);
    let reached_target = target
        .zip(after)
        .is_some_and(|(target, after)| target == after.tile);
    let event = AnalyticsEvent::new("diagnostic.production_navigation", EventLevel::Info)
        .character(character_id)
        .attribute("progressed", progressed)
        .attribute("changed_tile", changed_tile)
        .attribute("reached_target", reached_target)
        .attribute("position_before_known", before.is_some())
        .attribute("position_after_known", after.is_some())
        .attribute("backend_result_known", result.is_some())
        .attribute(
            "backend_arrived_known",
            result.and_then(|result| result.arrived).is_some(),
        )
        .attribute(
            "backend_arrived",
            result.and_then(|result| result.arrived).unwrap_or(false),
        )
        .attribute(
            "backend_came_to_rest_known",
            result.and_then(|result| result.came_to_rest).is_some(),
        )
        .attribute(
            "backend_came_to_rest",
            result
                .and_then(|result| result.came_to_rest)
                .unwrap_or(false),
        );
    let event = target.map_or(event.clone(), |target| {
        event
            .attribute("target_tile_x", i64::from(target.x))
            .attribute("target_tile_y", i64::from(target.y))
    });
    let event = add_position_attributes(event, "before", before);
    analytics.record(add_position_attributes(event, "after", after));
    tracing::info!(
        ?target,
        progressed,
        changed_tile,
        reached_target,
        before_tile_x = before.map(|position| position.tile.x),
        before_tile_y = before.map(|position| position.tile.y),
        after_tile_x = after.map(|position| position.tile.x),
        after_tile_y = after.map(|position| position.tile.y),
        "production navigation result"
    );
}

async fn find_reachable_navigation_target(
    gateway: &agent_arena_npc_harness::mcp::ArenaGateway,
    current: &PerceptionUpdate,
    prefer_west: bool,
) -> anyhow::Result<Option<TilePosition>> {
    let Some(own) = current.frame.self_state.position else {
        return Ok(None);
    };
    let occupied = current
        .frame
        .nearby_entities
        .iter()
        .filter_map(|entity| entity.tile)
        .chain(
            current
                .frame
                .nearby_drops
                .iter()
                .filter_map(|drop| drop.tile),
        )
        .collect::<std::collections::BTreeSet<_>>();
    let mut candidates = current
        .frame
        .map
        .tiles
        .iter()
        .filter(|tile| tile.kind == TileKind::Traversable && tile.walkable == Some(true))
        .map(|tile| tile.position)
        .filter(|tile| tile.x >= 0 && tile.y >= 0 && !occupied.contains(tile))
        .filter(|tile| {
            let distance = tile.x.abs_diff(own.tile.x) + tile.y.abs_diff(own.tile.y);
            (4..=10).contains(&distance)
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|tile| {
        let wrong_direction =
            (prefer_west && tile.x >= own.tile.x) || (!prefer_west && tile.x <= own.tile.x);
        let distance = tile.x.abs_diff(own.tile.x) + tile.y.abs_diff(own.tile.y);
        (
            wrong_direction,
            distance.abs_diff(6),
            tile.y.abs_diff(own.tile.y),
            tile.y,
            tile.x,
        )
    });
    for candidate in candidates.into_iter().take(24) {
        let path = gateway.check_path(tile_center(candidate)).await?;
        if path.reachable == Some(true) {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

fn tile_center(tile: TilePosition) -> PixelPosition {
    PixelPosition {
        x: tile.x.to_f32().unwrap_or(f32::MAX).mul_add(32.0, 16.0),
        y: tile.y.to_f32().unwrap_or(f32::MAX).mul_add(32.0, 16.0),
    }
}

fn add_position_attributes(
    event: AnalyticsEvent,
    prefix: &str,
    position: Option<agent_arena_npc_harness::world::Position>,
) -> AnalyticsEvent {
    let Some(position) = position else {
        return event;
    };
    event
        .attribute(format!("{prefix}_pixel_x"), f64::from(position.pixel.x))
        .attribute(format!("{prefix}_pixel_y"), f64::from(position.pixel.y))
        .attribute(format!("{prefix}_tile_x"), i64::from(position.tile.x))
        .attribute(format!("{prefix}_tile_y"), i64::from(position.tile.y))
}

fn record_perception_diagnostic(
    update: &PerceptionUpdate,
    source_scene_width: Option<u32>,
    source_scene_height: Option<u32>,
    character_id: &str,
    analytics: &Arc<dyn AnalyticsSink>,
) {
    let event = AnalyticsEvent::new("diagnostic.perception_frame_normalized", EventLevel::Info)
        .character(character_id)
        .correlation(update.observation_cycle_id)
        .attribute(
            "observation_cycle_id",
            update.observation_cycle_id.to_string(),
        )
        .attribute(
            "observation_cycle_sequence",
            update.observation_cycle_sequence,
        )
        .attribute("frame_revision", update.frame.revision)
        .attribute("perception_revision", update.frame.perception_revision)
        .attribute("inventory_revision", update.frame.inventory_revision)
        .attribute("map_revision", update.frame.map.revision)
        .attribute(
            "map_width",
            u64::try_from(update.frame.map.width).unwrap_or(u64::MAX),
        )
        .attribute(
            "map_height",
            u64::try_from(update.frame.map.height).unwrap_or(u64::MAX),
        )
        .attribute("map_origin_x", i64::from(update.frame.map.origin_tile_x))
        .attribute("map_origin_y", i64::from(update.frame.map.origin_tile_y))
        .attribute(
            "source_scene_width",
            u64::from(source_scene_width.unwrap_or_default()),
        )
        .attribute(
            "source_scene_height",
            u64::from(source_scene_height.unwrap_or_default()),
        )
        .attribute(
            "self_tile_x",
            i64::from(
                update
                    .frame
                    .self_state
                    .position
                    .map_or(0, |position| position.tile.x),
            ),
        )
        .attribute(
            "self_tile_y",
            i64::from(
                update
                    .frame
                    .self_state
                    .position
                    .map_or(0, |position| position.tile.y),
            ),
        )
        .attribute("health_known", update.frame.self_state.health.is_some())
        .attribute("position_known", update.frame.self_state.position.is_some())
        .attribute("combat_known", update.frame.combat.active.is_some());
    analytics.record(add_perception_summary_attributes(event, &update.summary));
}

fn add_perception_summary_attributes(
    event: AnalyticsEvent,
    summary: &PerceptionSummary,
) -> AnalyticsEvent {
    event
        .attribute("material_change", summary.material_change)
        .attribute("derived_event_count", as_u64(summary.derived_event_count))
        .attribute("backend_event_count", as_u64(summary.backend_event_count))
        .attribute("visible_entity_count", as_u64(summary.visible_entity_count))
        .attribute(
            "visible_hostile_count",
            as_u64(summary.visible_hostile_count),
        )
        .attribute("visible_player_count", as_u64(summary.visible_player_count))
        .attribute("visible_npc_count", as_u64(summary.visible_npc_count))
        .attribute(
            "visible_merchant_count",
            as_u64(summary.visible_merchant_count),
        )
        .attribute("visible_enemy_count", as_u64(summary.visible_enemy_count))
        .attribute(
            "visible_unknown_count",
            as_u64(summary.visible_unknown_count),
        )
        .attribute("drop_count", as_u64(summary.drop_count))
        .attribute(
            "positioned_drop_count",
            as_u64(summary.positioned_drop_count),
        )
        .attribute(
            "unpositioned_drop_count",
            as_u64(summary.unpositioned_drop_count),
        )
        .attribute("carried_item_count", as_u64(summary.carried_item_count))
        .attribute("carried_item_units", summary.carried_item_units)
        .attribute("door_count", as_u64(summary.door_count))
        .attribute("locked_door_count", as_u64(summary.locked_door_count))
        .attribute(
            "unknown_lock_door_count",
            as_u64(summary.unknown_lock_door_count),
        )
        .attribute(
            "reported_total_object_count_known",
            summary.reported_total_object_count.is_some(),
        )
        .attribute(
            "reported_total_object_count",
            u64::from(summary.reported_total_object_count.unwrap_or(0)),
        )
        .attribute(
            "object_list_truncated_known",
            summary.object_list_truncated.is_some(),
        )
        .attribute(
            "object_list_truncated",
            summary.object_list_truncated.unwrap_or(false),
        )
        .attribute("new_dialogue_count", as_u64(summary.new_dialogue_count))
        .attribute("new_scene_chat_count", as_u64(summary.new_scene_chat_count))
        .attribute(
            "new_global_chat_count",
            as_u64(summary.new_global_chat_count),
        )
        .attribute(
            "new_private_chat_count",
            as_u64(summary.new_private_chat_count),
        )
        .attribute("new_team_chat_count", as_u64(summary.new_team_chat_count))
        .attribute(
            "new_unknown_chat_count",
            as_u64(summary.new_unknown_chat_count),
        )
        .attribute("new_melody_count", as_u64(summary.new_melody_count))
        .attribute("filtered_chat_count", as_u64(summary.filtered_chat_count))
        .attribute("reachable_exit_count", as_u64(summary.reachable_exit_count))
        .attribute(
            "nearest_exit_path_length_known",
            summary.nearest_exit_path_length.is_some(),
        )
        .attribute(
            "nearest_exit_path_length",
            u64::from(summary.nearest_exit_path_length.unwrap_or(0)),
        )
        .attribute("map_tile_count", as_u64(summary.map_tile_count))
}

fn as_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn log_perception_summary(
    update: &PerceptionUpdate,
    source_scene_width: Option<u32>,
    source_scene_height: Option<u32>,
) {
    tracing::info!(
        observation_cycle_id = %update.observation_cycle_id,
        observation_cycle_sequence = update.observation_cycle_sequence,
        frame_revision = update.frame.revision,
        perception_revision = update.frame.perception_revision,
        inventory_revision = update.frame.inventory_revision,
        map_revision = update.frame.map.revision,
        map_width = update.frame.map.width,
        map_height = update.frame.map.height,
        map_origin_x = update.frame.map.origin_tile_x,
        map_origin_y = update.frame.map.origin_tile_y,
        source_scene_width,
        source_scene_height,
        self_tile_x = update
            .frame
            .self_state
            .position
            .map(|position| position.tile.x),
        self_tile_y = update
            .frame
            .self_state
            .position
            .map(|position| position.tile.y),
        material_change = update.summary.material_change,
        derived_event_count = update.summary.derived_event_count,
        backend_event_count = update.summary.backend_event_count,
        visible_entity_count = update.summary.visible_entity_count,
        visible_hostile_count = update.summary.visible_hostile_count,
        visible_player_count = update.summary.visible_player_count,
        visible_npc_count = update.summary.visible_npc_count,
        visible_merchant_count = update.summary.visible_merchant_count,
        visible_enemy_count = update.summary.visible_enemy_count,
        visible_unknown_count = update.summary.visible_unknown_count,
        drop_count = update.summary.drop_count,
        positioned_drop_count = update.summary.positioned_drop_count,
        unpositioned_drop_count = update.summary.unpositioned_drop_count,
        carried_item_count = update.summary.carried_item_count,
        carried_item_units = update.summary.carried_item_units,
        door_count = update.summary.door_count,
        locked_door_count = update.summary.locked_door_count,
        unknown_lock_door_count = update.summary.unknown_lock_door_count,
        reported_total_object_count = update.summary.reported_total_object_count,
        object_list_truncated = update.summary.object_list_truncated,
        new_dialogue_count = update.summary.new_dialogue_count,
        new_scene_chat_count = update.summary.new_scene_chat_count,
        new_global_chat_count = update.summary.new_global_chat_count,
        new_private_chat_count = update.summary.new_private_chat_count,
        new_team_chat_count = update.summary.new_team_chat_count,
        new_unknown_chat_count = update.summary.new_unknown_chat_count,
        new_melody_count = update.summary.new_melody_count,
        filtered_chat_count = update.summary.filtered_chat_count,
        reachable_exit_count = update.summary.reachable_exit_count,
        nearest_exit_path_length = update.summary.nearest_exit_path_length,
        map_tile_count = update.summary.map_tile_count,
        health_known = update.frame.self_state.health.is_some(),
        position_known = update.frame.self_state.position.is_some(),
        combat_known = update.frame.combat.active.is_some(),
        "live perception frame normalized"
    );
}

async fn run_live_walk(gateway: &agent_arena_npc_harness::mcp::ArenaGateway) -> anyhow::Result<()> {
    let directions = [MoveDirection::Left, MoveDirection::Right];
    let mut next_direction = 0_usize;
    tracing::info!("live test walk started; press Ctrl-C to stop and disconnect");
    loop {
        tokio::select! {
            signal = tokio::signal::ctrl_c() => {
                signal?;
                tracing::info!("live test walk stopping");
                return Ok(());
            }
            () = tokio::time::sleep(Duration::from_secs(3)) => {
                let direction = directions[next_direction];
                next_direction = (next_direction + 1) % directions.len();
                if gateway.move_direction(direction).await.is_ok() {
                    tracing::info!(?direction, "live test movement completed");
                } else {
                    tracing::warn!(?direction, error_class = "tool_or_transport", "live test movement failed");
                }
            }
        }
    }
}

fn parse_direction(value: Option<&String>) -> anyhow::Result<MoveDirection> {
    match value.map(String::as_str) {
        Some("up" | "north") => Ok(MoveDirection::Up),
        Some("down" | "south") => Ok(MoveDirection::Down),
        Some("left" | "west") => Ok(MoveDirection::Left),
        Some("right" | "east") => Ok(MoveDirection::Right),
        _ => bail!("move requires up, down, left, or right"),
    }
}

fn parse_instrument(value: Option<&String>) -> anyhow::Result<MelodyInstrument> {
    match value.map(String::as_str) {
        Some("lute") => Ok(MelodyInstrument::Lute),
        Some("flute") => Ok(MelodyInstrument::Flute),
        Some("horn") => Ok(MelodyInstrument::Horn),
        Some("bell") => Ok(MelodyInstrument::Bell),
        _ => bail!("play-melody requires lute, flute, horn, or bell"),
    }
}

fn parse_coordinate(value: Option<&String>, name: &str) -> anyhow::Result<f32> {
    value
        .with_context(|| format!("move-to requires coordinate {name}"))?
        .parse::<f32>()
        .with_context(|| format!("coordinate {name} must be a number"))
}
