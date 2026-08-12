use std::{sync::Arc, time::Duration, time::Instant};

use agent_arena_npc_harness::{
    brain::{
        Brain, BrainCallContext,
        models::{
            ModelBackgroundTasks, ModelCallObservability, ModelUsageLedger, OpenRouterJsonBrain,
        },
        prompts::{TACTICIAN_V10, TACTICIAN_V10_VERSION},
        strategic_intent::{Priority, StrategicIntent},
        tactical_frame::{
            CarriedItem, CombatActionAvailability, EntityKind, TargetKind, VisibleEntity,
        },
        tactical_input::TacticalInput,
        tactical_output::TacticalProposal,
    },
    observability,
    world::{
        PixelPosition, Position, TilePosition,
        combat::{CombatEpisodeSnapshot, CombatSnapshot},
        map::LocalMap,
    },
};
use anyhow::Context;

const DEFAULT_MODELS: &[&str] = &[
    "google/gemini-3.1-flash-lite",
    "openai/gpt-oss-safeguard-20b",
];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    observability::init_tracing();
    let api_key = std::env::var("OPENROUTER_API_KEY")
        .context("OPENROUTER_API_KEY is required for the tactical probe")?;
    let configured = std::env::args().skip(1).collect::<Vec<_>>();
    let models = if configured.is_empty() {
        DEFAULT_MODELS.iter().map(ToString::to_string).collect()
    } else {
        configured
    };
    let runs_per_model = std::env::var("NPC_TACTICAL_PROBE_RUNS")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(1);
    let scenario = ProbeScenario::from_env()?;
    let frame = scenario.frame();
    let input = TacticalInput::from(&frame);
    let analytics = observability::tracing_sink();
    let background_tasks = Arc::new(ModelBackgroundTasks::default());
    let mut failures = 0_u32;

    for model in models {
        let usage_ledger = Arc::new(ModelUsageLedger::default());
        let brain = OpenRouterJsonBrain::<_, TacticalProposal>::new_observed(
            &api_key,
            &model,
            probe_prompt(),
            0.1,
            150,
            ModelCallObservability::new(TACTICIAN_V10_VERSION, analytics.clone())
                .with_role("tactician")
                .with_usage_ledger(usage_ledger.clone())
                .with_background_tasks(background_tasks.clone()),
        )?
        .with_request_timeout(probe_timeout());
        let mut quality_passes = 0_u32;
        let mut completed = 0_u32;
        for trial in 1..=runs_per_model {
            let started = Instant::now();
            let context = BrainCallContext {
                decision_id: uuid::Uuid::new_v4(),
                character_id: Some("cassian".to_owned()),
                frame_revision: Some(frame.revision),
                strategic_revision: Some(frame.strategic_intent.revision),
            };
            match brain.decide_with_context(&input, &context).await {
                Ok(proposal) => {
                    completed = completed.saturating_add(1);
                    let verdict = scenario.verdict(&frame, &proposal);
                    quality_passes = quality_passes.saturating_add(u32::from(verdict.passed));
                    tracing::info!(
                        model,
                        scenario = scenario.as_str(),
                        trial,
                        latency_ms = u64::try_from(started.elapsed().as_millis())
                            .unwrap_or(u64::MAX),
                        intent = ?proposal.intent,
                        action_kinds = action_kinds(&proposal),
                        action_count = proposal.actions.len(),
                        valid_for_ms = proposal.valid_for_ms,
                        quality_passed = verdict.passed,
                        verdict = verdict.reason,
                        "tactical model probe completed"
                    );
                }
                Err(error) => {
                    failures = failures.saturating_add(1);
                    tracing::warn!(model, trial, error = %error, "tactical model probe failed");
                }
            }
        }
        tracing::info!(
            model,
            scenario = scenario.as_str(),
            runs_requested = runs_per_model,
            calls_completed = completed,
            provider_failures = runs_per_model.saturating_sub(completed),
            quality_passes,
            quality_failures = completed.saturating_sub(quality_passes),
            "tactical model probe summary"
        );
        log_usage_summary(&model, scenario, &usage_ledger.totals_for("cassian"));
    }
    if failures > 0 {
        anyhow::bail!("{failures} tactical model probe(s) failed");
    }
    let drained = background_tasks.drain(Duration::from_secs(45)).await;
    tracing::info!(
        accounting_tasks_completed = drained.completed,
        accounting_tasks_failed = drained.failed,
        accounting_tasks_aborted = drained.aborted,
        "tactical probe accounting tasks reached a terminal state"
    );
    if drained.failed > 0 || drained.aborted > 0 {
        anyhow::bail!(
            "provider accounting did not finish cleanly: failed={}, aborted={}",
            drained.failed,
            drained.aborted
        );
    }
    Ok(())
}

fn probe_timeout() -> Duration {
    std::env::var("NPC_TACTICIAN_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map_or_else(|| Duration::from_secs(5), Duration::from_millis)
}

fn log_usage_summary(
    model: &str,
    scenario: ProbeScenario,
    usage: &agent_arena_npc_harness::brain::models::ModelUsageTotals,
) {
    let average_exact_cost_usd = if usage.exact_cost_known_calls == 0 {
        0.0
    } else {
        usage.exact_cost_usd
            / num_traits::ToPrimitive::to_f64(&usage.exact_cost_known_calls)
                .unwrap_or(f64::INFINITY)
    };
    tracing::info!(
        model,
        scenario = scenario.as_str(),
        calls = usage.calls,
        input_tokens = usage.input_tokens,
        output_tokens = usage.output_tokens,
        total_tokens = usage.total_tokens,
        cached_input_tokens = usage.cached_input_tokens,
        reasoning_tokens = usage.reasoning_tokens,
        exact_cost_known_calls = usage.exact_cost_known_calls,
        exact_cost_usd = usage.exact_cost_usd,
        average_exact_cost_usd,
        "tactical model probe usage summary"
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProbeVerdict {
    passed: bool,
    reason: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeScenario {
    SurroundedLowHealth,
    CriticalNoHeal,
    HealthySingleEnemy,
    SafeIdle,
    ExploreIdle,
}

impl ProbeScenario {
    fn from_env() -> anyhow::Result<Self> {
        match std::env::var("NPC_TACTICAL_PROBE_SCENARIO")
            .unwrap_or_else(|_| "surrounded_low_health".to_owned())
            .as_str()
        {
            "surrounded_low_health" => Ok(Self::SurroundedLowHealth),
            "critical_no_heal" => Ok(Self::CriticalNoHeal),
            "healthy_single_enemy" => Ok(Self::HealthySingleEnemy),
            "safe_idle" => Ok(Self::SafeIdle),
            "explore_idle" => Ok(Self::ExploreIdle),
            value => anyhow::bail!("unknown tactical probe scenario: {value}"),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::SurroundedLowHealth => "surrounded_low_health",
            Self::CriticalNoHeal => "critical_no_heal",
            Self::HealthySingleEnemy => "healthy_single_enemy",
            Self::SafeIdle => "safe_idle",
            Self::ExploreIdle => "explore_idle",
        }
    }

    fn frame(self) -> agent_arena_npc_harness::brain::tactical_frame::TacticalFrame {
        let mut frame = surrounded_low_health_frame();
        match self {
            Self::SurroundedLowHealth => {}
            Self::CriticalNoHeal => {
                frame.self_state.health = Some(15);
                frame.self_state.inventory.clear();
                frame.combat.damage_received_last_five_seconds = Some(28);
                if let Some(episode) = frame.combat.episode.as_mut() {
                    episode.current_health = 15;
                    episode.damage_received = 85;
                }
            }
            Self::HealthySingleEnemy => {
                frame.self_state.health = Some(92);
                frame.combat.current_hostiles = 1;
                frame.combat.damage_received_last_five_seconds = Some(2);
                frame.combat.damage_dealt_last_five_seconds = Some(18);
                if let Some(episode) = frame.combat.episode.as_mut() {
                    episode.duration_ms = 5_000;
                    episode.kills = 0;
                    episode.hostile_spawns = 1;
                    episode.current_hostiles = 1;
                    episode.damage_dealt = 18;
                    episode.damage_received = 8;
                    episode.current_health = 92;
                    episode.respawn_after_kill_pairs = 0;
                }
                frame.nearby_entities.truncate(1);
                "###############\n#.............#\n#.............#\n#.............#\n#........S....#\n#......@......#\n#.............#\n#.............#\n#.............#\n###############"
                    .clone_into(&mut frame.map.ascii);
            }
            Self::SafeIdle => {
                "Wait here safely while the strategist reflects."
                    .clone_into(&mut frame.strategic_intent.objective);
                frame.strategic_intent.subgoals.clear();
                frame.strategic_intent.constraints =
                    vec!["Do not move until strategic intent changes.".to_owned()];
                frame.self_state.health = Some(100);
                frame.combat = CombatSnapshot::default();
                frame.nearby_entities.clear();
                "###############\n#.............#\n#.............#\n#.............#\n#.............#\n#......@......#\n#.............#\n#.............#\n#.............#\n###############"
                    .clone_into(&mut frame.map.ascii);
            }
            Self::ExploreIdle => {
                use agent_arena_npc_harness::world::map::{
                    CardinalDirection, ReachableExit, ReachableWaypoint,
                };

                "Leave this inn and explore for worthy rivals and an audience."
                    .clone_into(&mut frame.strategic_intent.objective);
                frame.strategic_intent.subgoals =
                    vec!["Make local progress toward leaving the inn.".to_owned()];
                frame.strategic_intent.constraints.clear();
                frame.self_state.health = Some(100);
                frame.combat = CombatSnapshot::default();
                frame.nearby_entities.clear();
                frame.exits = vec![ReachableExit {
                    tile: TilePosition { x: 71, y: 28 },
                    destination_scene: Some("reldens-town".to_owned()),
                    label: Some("front door".to_owned()),
                    path_length_tiles: 4,
                }];
                frame.local_waypoints = vec![
                    ReachableWaypoint {
                        tile: TilePosition { x: 67, y: 32 },
                        direction: CardinalDirection::West,
                        path_length_tiles: 4,
                    },
                    ReachableWaypoint {
                        tile: TilePosition { x: 75, y: 32 },
                        direction: CardinalDirection::East,
                        path_length_tiles: 4,
                    },
                ];
                "###############\n#......D......#\n#.............#\n#.............#\n#.............#\n#......@......#\n#.............#\n#.............#\n#.............#\n###############"
                    .clone_into(&mut frame.map.ascii);
            }
        }
        populate_map_tiles(&mut frame.map);
        frame
    }

    fn verdict(
        self,
        frame: &agent_arena_npc_harness::brain::tactical_frame::TacticalFrame,
        proposal: &TacticalProposal,
    ) -> ProbeVerdict {
        use std::collections::HashSet;

        use agent_arena_npc_harness::{
            character::Capability,
            execution::{
                packet::ActionPacket,
                validator::{ValidationContext, validate_packet},
            },
        };

        if proposal.validate_semantics().is_err() {
            return ProbeVerdict {
                passed: false,
                reason: "invalid_proposal_semantics",
            };
        }
        let packet = ActionPacket::from_proposal(
            uuid::Uuid::new_v4(),
            frame.revision,
            frame.strategic_intent.revision,
            frame.self_state.scene.clone(),
            proposal.clone(),
        );
        let capabilities = HashSet::from([
            Capability::Walk,
            Capability::Doors,
            Capability::Fight,
            Capability::Trade,
        ]);
        if let Err(error) = validate_packet(
            &packet,
            &ValidationContext {
                minimum_valid_frame_revision: frame.revision,
                current_strategic_revision: frame.strategic_intent.revision,
                now: chrono::Utc::now(),
                capabilities: &capabilities,
                frame,
            },
        ) {
            return ProbeVerdict {
                passed: false,
                reason: error.reason_code(),
            };
        }
        match self {
            Self::SurroundedLowHealth => low_health_verdict(proposal),
            Self::CriticalNoHeal => critical_no_heal_verdict(proposal),
            Self::HealthySingleEnemy => healthy_combat_verdict(proposal),
            Self::SafeIdle => safe_idle_verdict(proposal),
            Self::ExploreIdle => explore_idle_verdict(frame, proposal),
        }
    }
}

fn populate_map_tiles(map: &mut LocalMap) {
    use agent_arena_npc_harness::world::map::{MapTile, TileKind};

    let origin_tile_x = map.origin_tile_x;
    let origin_tile_y = map.origin_tile_y;
    map.tiles = map
        .ascii
        .lines()
        .enumerate()
        .flat_map(|(row, line)| {
            line.chars()
                .enumerate()
                .filter_map(move |(column, symbol)| {
                    let x = origin_tile_x.checked_add(i32::try_from(column).ok()?)?;
                    let y = origin_tile_y.checked_add(i32::try_from(row).ok()?)?;
                    Some(MapTile {
                        position: TilePosition { x, y },
                        kind: match symbol {
                            '#' => TileKind::Blocked,
                            _ => TileKind::Traversable,
                        },
                        walkable: Some(symbol != '#'),
                    })
                })
        })
        .collect();
}

fn low_health_verdict(proposal: &TacticalProposal) -> ProbeVerdict {
    use agent_arena_npc_harness::execution::packet::{TacticalAction, TacticalStyle};

    if proposal.actions.iter().any(|action| {
        matches!(
            action,
            TacticalAction::SetTactics {
                style: TacticalStyle::Flee,
                ..
            }
        )
    }) {
        ProbeVerdict {
            passed: true,
            reason: "sets_flee_tactics",
        }
    } else if proposal
        .actions
        .iter()
        .any(|action| matches!(action, TacticalAction::UseItem { item_id } if item_id == "minor-healing-potion"))
    {
        ProbeVerdict {
            passed: true,
            reason: "uses_survival_item",
        }
    } else if proposal
        .actions
        .iter()
        .all(|action| matches!(action, TacticalAction::Stop))
        && !proposal.actions.is_empty()
    {
        ProbeVerdict {
            passed: false,
            reason: "stop_does_not_disengage_combat",
        }
    } else {
        ProbeVerdict {
            passed: false,
            reason: "no_survival_action",
        }
    }
}

fn critical_no_heal_verdict(proposal: &TacticalProposal) -> ProbeVerdict {
    use agent_arena_npc_harness::execution::packet::{TacticalAction, TacticalStyle};

    if proposal.actions.iter().any(|action| {
        matches!(
            action,
            TacticalAction::SetTactics {
                style: TacticalStyle::Flee,
                ..
            }
        )
    }) {
        ProbeVerdict {
            passed: true,
            reason: "sets_flee_tactics",
        }
    } else {
        ProbeVerdict {
            passed: false,
            reason: "critical_health_without_flee",
        }
    }
}

fn healthy_combat_verdict(proposal: &TacticalProposal) -> ProbeVerdict {
    use agent_arena_npc_harness::execution::packet::{TacticalAction, TacticalIntent};

    let appropriate = matches!(proposal.intent, TacticalIntent::Continue)
        || proposal.actions.iter().any(|action| {
            matches!(
                action,
                TacticalAction::Attack { target_id } if target_id == "spider-92"
            ) || matches!(
                action,
                TacticalAction::UseSkill { skill_id, target_id }
                    if skill_id == "slash" && target_id.as_deref() == Some("spider-92")
            )
        });
    let wastes_heal = proposal
        .actions
        .iter()
        .any(|action| matches!(action, TacticalAction::UseItem { .. }));
    if appropriate && !wastes_heal {
        ProbeVerdict {
            passed: true,
            reason: "continues_healthy_combat",
        }
    } else {
        ProbeVerdict {
            passed: false,
            reason: "unnecessary_retreat_or_heal",
        }
    }
}

fn safe_idle_verdict(proposal: &TacticalProposal) -> ProbeVerdict {
    if proposal.intent == agent_arena_npc_harness::execution::packet::TacticalIntent::Continue
        && proposal.actions.is_empty()
    {
        ProbeVerdict {
            passed: true,
            reason: "remains_idle",
        }
    } else {
        ProbeVerdict {
            passed: false,
            reason: "invented_idle_action",
        }
    }
}

fn explore_idle_verdict(
    frame: &agent_arena_npc_harness::brain::tactical_frame::TacticalFrame,
    proposal: &TacticalProposal,
) -> ProbeVerdict {
    use agent_arena_npc_harness::execution::packet::TacticalAction;

    let offered = frame
        .exits
        .iter()
        .map(|exit| exit.tile)
        .chain(frame.local_waypoints.iter().map(|waypoint| waypoint.tile));
    let moves_to_offered_tile = proposal.actions.iter().any(|action| {
        let TacticalAction::MoveTo { tile_x, tile_y } = action else {
            return false;
        };
        offered
            .clone()
            .any(|tile| tile.x == *tile_x && tile.y == *tile_y)
    });
    if moves_to_offered_tile {
        ProbeVerdict {
            passed: true,
            reason: "moves_toward_exploration",
        }
    } else {
        ProbeVerdict {
            passed: false,
            reason: "does_not_make_local_progress",
        }
    }
}

fn action_kinds(proposal: &TacticalProposal) -> String {
    use agent_arena_npc_harness::execution::packet::TacticalAction;

    proposal
        .actions
        .iter()
        .map(|action| match action {
            TacticalAction::MoveTo { .. } => "move_to",
            TacticalAction::Attack { .. } => "attack",
            TacticalAction::UseSkill { .. } => "use_skill",
            TacticalAction::UseItem { .. } => "use_item",
            TacticalAction::PickUp { .. } => "pick_up",
            TacticalAction::SetTactics { .. } => "set_tactics",
            TacticalAction::Stop => "stop",
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn probe_prompt() -> String {
    format!(
        "{TACTICIAN_V10}\n\nReturn only a TacticalProposal. The runtime supplies packet ids and revisions."
    )
}

fn surrounded_low_health_frame() -> agent_arena_npc_harness::brain::tactical_frame::TacticalFrame {
    let strategy = StrategicIntent {
        revision: 42,
        objective: "Collect spider silk and return to town alive.".to_owned(),
        subgoals: vec!["Collect at least eight silk.".to_owned()],
        priorities: vec![Priority::Survival, Priority::Loot, Priority::Kills],
        constraints: vec![
            "Keep one healing potion in reserve.".to_owned(),
            "Do not chase enemies far from the road.".to_owned(),
        ],
        risk_tolerance: 0.35,
        ..StrategicIntent::default()
    };
    let mut frame = agent_arena_npc_harness::brain::tactical_frame::TacticalFrame::empty(strategy);
    frame.revision = 1_842;
    frame.perception_revision = 1_842;
    frame.self_state.scene = Some("spider-nest".to_owned());
    frame.self_state.position = Some(Position {
        pixel: PixelPosition {
            x: 2_288.0,
            y: 1_040.0,
        },
        tile: TilePosition { x: 71, y: 32 },
    });
    frame.self_state.health = Some(38);
    frame.self_state.max_health = Some(100);
    frame.self_state.alive = Some(true);
    frame.self_state.combat_actions = vec![CombatActionAvailability {
        id: "slash".to_owned(),
        available: Some(true),
        cooldown_remaining_ms: Some(0),
        target_kind: TargetKind::Entity,
    }];
    frame.self_state.inventory = vec![CarriedItem {
        id: "minor-healing-potion".to_owned(),
        label: "Minor Healing Potion".to_owned(),
        quantity: 2,
        usable: Some(true),
        equipment: Some(false),
        equipped: Some(false),
    }];
    frame.combat = CombatSnapshot {
        active: Some(true),
        style: Some("duck_and_weave".to_owned()),
        style_is_own_choice: Some(true),
        mode: Some("semi_auto".to_owned()),
        current_target_id: Some("spider-92".to_owned()),
        current_hostiles: 3,
        aggressors: Vec::new(),
        enemy_health: Vec::new(),
        damage_dealt: Vec::new(),
        damage_received_last_five_seconds: Some(21),
        damage_dealt_last_five_seconds: Some(44),
        episode: Some(CombatEpisodeSnapshot {
            duration_ms: 18_400,
            kills: 5,
            hostile_spawns: 5,
            current_hostiles: 3,
            damage_dealt: 96,
            damage_received: 62,
            starting_health: 100,
            current_health: 38,
            respawn_after_kill_pairs: 3,
        }),
    };
    frame.nearby_entities = vec![
        hostile("spider-92", -3, -3, 4.2, true),
        hostile("spider-93", 2, -1, 2.2, true),
        hostile("spider-94", 3, 2, 3.6, false),
    ];
    frame.map = LocalMap {
        revision: 89,
        origin_tile_x: 64,
        origin_tile_y: 27,
        width: 15,
        height: 10,
        tiles: Vec::new(),
        doors: Vec::new(),
        ascii: "###############\n#.....#.......#\n#..S..........#\n#.....##......#\n#.......S.....#\n#......@......#\n#.............#\n#....S........#\n#.............#\n###############".to_owned(),
    };
    frame
}

fn hostile(
    id: &str,
    relative_x: i32,
    relative_y: i32,
    distance: f32,
    targeting_you: bool,
) -> VisibleEntity {
    VisibleEntity {
        id: id.to_owned(),
        backend_object_id: None,
        label: "Spider".to_owned(),
        kind: EntityKind::Enemy,
        tile: Some(TilePosition {
            x: 71 + relative_x,
            y: 32 + relative_y,
        }),
        relative: Some(TilePosition {
            x: relative_x,
            y: relative_y,
        }),
        distance: Some(distance),
        alive: Some(true),
        is_merchant: Some(false),
        interactable: Some(false),
        hostile: Some(true),
        targeting_you: Some(targeting_you),
    }
}

#[cfg(test)]
mod tests {
    use agent_arena_npc_harness::execution::packet::{
        TacticalAction, TacticalIntent, TacticalMode, TacticalProposal, TacticalStyle,
    };

    use super::{ProbeScenario, low_health_verdict};

    fn proposal(intent: TacticalIntent, action: TacticalAction) -> TacticalProposal {
        TacticalProposal {
            intent,
            actions: vec![action],
            valid_for_ms: 500,
            abort_if: Vec::new(),
            rationale: None,
        }
    }

    #[test]
    fn low_health_probe_requires_an_action_that_can_improve_survival() {
        assert!(
            low_health_verdict(&proposal(
                TacticalIntent::UseItem,
                TacticalAction::UseItem {
                    item_id: "minor-healing-potion".to_owned(),
                },
            ))
            .passed
        );
        assert!(
            low_health_verdict(&proposal(
                TacticalIntent::Disengage,
                TacticalAction::SetTactics {
                    style: TacticalStyle::Flee,
                    mode: TacticalMode::SemiAuto,
                },
            ))
            .passed
        );
        assert!(
            !low_health_verdict(&proposal(
                TacticalIntent::Disengage,
                TacticalAction::MoveTo {
                    tile_x: 70,
                    tile_y: 32,
                },
            ))
            .passed
        );
        assert!(!low_health_verdict(&proposal(TacticalIntent::Stop, TacticalAction::Stop)).passed);
        assert!(
            !low_health_verdict(&proposal(
                TacticalIntent::Attack,
                TacticalAction::Attack {
                    target_id: "spider-92".to_owned(),
                },
            ))
            .passed
        );
    }

    #[test]
    fn critical_no_heal_requires_backend_flee_mode() {
        assert!(
            ProbeScenario::CriticalNoHeal
                .verdict(
                    &ProbeScenario::CriticalNoHeal.frame(),
                    &proposal(
                        TacticalIntent::Disengage,
                        TacticalAction::SetTactics {
                            style: TacticalStyle::Flee,
                            mode: TacticalMode::SemiAuto,
                        },
                    )
                )
                .passed
        );
        assert!(
            !ProbeScenario::CriticalNoHeal
                .verdict(
                    &ProbeScenario::CriticalNoHeal.frame(),
                    &proposal(TacticalIntent::Stop, TacticalAction::Stop),
                )
                .passed
        );
    }

    #[test]
    fn scenario_rejects_a_good_intent_with_an_unknown_move() {
        let mut proposal = proposal(
            TacticalIntent::Disengage,
            TacticalAction::SetTactics {
                style: TacticalStyle::Flee,
                mode: TacticalMode::SemiAuto,
            },
        );
        proposal.actions.push(TacticalAction::MoveTo {
            tile_x: 99_999,
            tile_y: 99_999,
        });
        let verdict = ProbeScenario::CriticalNoHeal
            .verdict(&ProbeScenario::CriticalNoHeal.frame(), &proposal);

        assert!(!verdict.passed);
        assert_eq!(verdict.reason, "unknown_destination");
    }
}
