use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use chrono::{DateTime, Utc};

use super::events::{GameEvent, GameEventKind};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CombatSnapshot {
    pub active: Option<bool>,
    pub style: Option<String>,
    pub style_is_own_choice: Option<bool>,
    pub mode: Option<String>,
    pub current_target_id: Option<String>,
    pub current_hostiles: usize,
    pub aggressors: Vec<CombatAggressor>,
    pub enemy_health: Vec<EnemyHealth>,
    pub damage_dealt: Vec<DamageDealt>,
    pub damage_received_last_five_seconds: Option<i64>,
    pub damage_dealt_last_five_seconds: Option<i64>,
    pub episode: Option<CombatEpisodeSnapshot>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CombatAggressor {
    pub id: String,
    pub label: Option<String>,
    pub damage_dealt_to_you: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EnemyHealth {
    pub id: String,
    pub label: Option<String>,
    pub health: Option<i64>,
    pub max_health: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DamageDealt {
    pub target_id: String,
    pub label: Option<String>,
    pub amount: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CombatEpisodeSnapshot {
    pub duration_ms: u64,
    pub kills: u32,
    pub hostile_spawns: u32,
    pub current_hostiles: usize,
    pub damage_dealt: i64,
    pub damage_received: i64,
    pub starting_health: i32,
    pub current_health: i32,
    pub respawn_after_kill_pairs: u32,
}

#[derive(Debug, Default)]
pub struct CombatEpisodeReducer {
    active: Option<ActiveCombatEpisode>,
}

#[derive(Debug)]
struct ActiveCombatEpisode {
    started_at: DateTime<Utc>,
    kills: u32,
    hostile_spawns: u32,
    damage_dealt: i64,
    damage_received: i64,
    starting_health: i32,
    current_health: i32,
    respawn_after_kill_pairs: u32,
    recent_kills: HashMap<String, DateTime<Utc>>,
}

impl CombatEpisodeReducer {
    /// Apply only the newly accepted events from one perception update.
    ///
    /// The reducer compresses facts. It does not decide whether combat should
    /// continue or whether the character should flee.
    pub fn update(
        &mut self,
        combat_active: Option<bool>,
        health: Option<i32>,
        current_hostiles: usize,
        new_events: &[GameEvent],
        observed_at: DateTime<Utc>,
    ) -> Option<CombatEpisodeSnapshot> {
        if self.active.is_none() && combat_active == Some(true) {
            let starting_health = health.unwrap_or_default();
            self.active = Some(ActiveCombatEpisode {
                started_at: observed_at,
                kills: 0,
                hostile_spawns: 0,
                damage_dealt: 0,
                damage_received: 0,
                starting_health,
                current_health: starting_health,
                respawn_after_kill_pairs: 0,
                recent_kills: HashMap::new(),
            });
        }

        let episode = self.active.as_mut()?;
        if let Some(health) = health {
            episode.current_health = health;
        }
        for event in new_events {
            match event.kind {
                GameEventKind::DamageTaken => {
                    episode.damage_received = episode
                        .damage_received
                        .saturating_add(event.amount.unwrap_or_default().max(0));
                }
                GameEventKind::DamageDealt => {
                    episode.damage_dealt = episode
                        .damage_dealt
                        .saturating_add(event.amount.unwrap_or_default().max(0));
                }
                GameEventKind::TargetKilled => {
                    episode.kills = episode.kills.saturating_add(1);
                    if let Some(entity_id) = &event.entity_id {
                        episode
                            .recent_kills
                            .insert(entity_id.clone(), event.observed_at);
                    }
                }
                GameEventKind::EnemySpawned => {
                    episode.hostile_spawns = episode.hostile_spawns.saturating_add(1);
                    if let Some(entity_id) = &event.entity_id
                        && episode
                            .recent_kills
                            .remove(entity_id)
                            .is_some_and(|killed_at| {
                                let delay = event.observed_at.signed_duration_since(killed_at);
                                delay >= chrono::Duration::zero()
                                    && delay <= chrono::Duration::seconds(2)
                            })
                    {
                        episode.respawn_after_kill_pairs =
                            episode.respawn_after_kill_pairs.saturating_add(1);
                    }
                }
                _ => {}
            }
        }
        episode.recent_kills.retain(|_, killed_at| {
            let age = observed_at.signed_duration_since(*killed_at);
            age >= chrono::Duration::zero() && age <= chrono::Duration::seconds(2)
        });

        let snapshot = snapshot(episode, current_hostiles, observed_at);
        if combat_active == Some(false) {
            self.active = None;
        }
        Some(snapshot)
    }
}

fn snapshot(
    episode: &ActiveCombatEpisode,
    current_hostiles: usize,
    observed_at: DateTime<Utc>,
) -> CombatEpisodeSnapshot {
    CombatEpisodeSnapshot {
        duration_ms: u64::try_from(
            observed_at
                .signed_duration_since(episode.started_at)
                .num_milliseconds()
                .max(0),
        )
        .unwrap_or(u64::MAX),
        kills: episode.kills,
        hostile_spawns: episode.hostile_spawns,
        current_hostiles,
        damage_dealt: episode.damage_dealt,
        damage_received: episode.damage_received,
        starting_health: episode.starting_health,
        current_health: episode.current_health,
        respawn_after_kill_pairs: episode.respawn_after_kill_pairs,
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::world::events::GameEventOrigin;

    fn at(second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 11, 12, 0, second)
            .single()
            .expect("time")
    }

    fn event(
        kind: GameEventKind,
        entity_id: Option<&str>,
        amount: Option<i64>,
        second: u32,
    ) -> GameEvent {
        GameEvent {
            sequence: Some(u64::from(second)),
            observed_at: at(second),
            origin: GameEventOrigin::Backend,
            kind,
            entity_id: entity_id.map(ToOwned::to_owned),
            amount,
            tile: None,
            detail: None,
        }
    }

    #[test]
    fn reduces_damage_kills_and_reused_enemy_respawns_without_advice() {
        let mut reducer = CombatEpisodeReducer::default();
        let opened = reducer
            .update(
                Some(true),
                Some(90),
                1,
                &[
                    event(GameEventKind::DamageTaken, Some("spider-1"), Some(10), 1),
                    event(GameEventKind::DamageDealt, Some("spider-1"), Some(20), 1),
                    event(GameEventKind::TargetKilled, Some("spider-1"), None, 2),
                ],
                at(2),
            )
            .expect("episode");
        assert_eq!(opened.kills, 1);
        assert_eq!(opened.damage_received, 10);
        assert_eq!(opened.damage_dealt, 20);

        let respawned = reducer
            .update(
                Some(true),
                Some(90),
                1,
                &[event(
                    GameEventKind::EnemySpawned,
                    Some("spider-1"),
                    None,
                    3,
                )],
                at(3),
            )
            .expect("episode");
        assert_eq!(respawned.hostile_spawns, 1);
        assert_eq!(respawned.respawn_after_kill_pairs, 1);
    }
}
