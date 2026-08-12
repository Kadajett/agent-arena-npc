use std::collections::{BTreeMap, BTreeSet, VecDeque};

use chrono::{DateTime, Duration, Utc};
use num_traits::ToPrimitive;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    brain::{
        strategic_intent::StrategicIntent,
        tactical_frame::{
            CarriedItem, CombatActionAvailability, Drop, EntityKind, SelfState, TacticalFrame,
            TargetKind, VisibilityCensus, VisibleEntity,
        },
    },
    execution::outcome::ActionOutcome,
    mcp::{
        observation::{
            Observation, ObservedBattleEvent, ObservedCombatAction, ObservedItem, ObservedPlayer,
        },
        types::{InventoryResult, MapObservation},
    },
    world::{
        PixelPosition, Position, TilePosition,
        combat::{CombatAggressor, CombatEpisodeReducer, CombatSnapshot, DamageDealt, EnemyHealth},
        dialogue::{
            DialogueChannel, DialogueKind, DialogueLine, new_dialogue_lines, normalize_dialogue,
        },
        events::{GameEvent, GameEventKind, GameEventOrigin},
        map::{
            CardinalDirection, Doorway, LocalMap, MapTile, ReachableExit, ReachableWaypoint,
            TileKind,
        },
    },
};

pub const TILE_SIZE_PIXELS: f32 = 32.0;
const MAX_RECENT_EVENTS: usize = 256;
const MAX_RECENT_ACTIONS: usize = 64;
const EVENT_WINDOW_SECONDS: i64 = 30;

#[derive(Debug, Clone)]
pub struct PerceptionInput {
    pub observation_cycle_id: Uuid,
    pub observation_cycle_sequence: u64,
    pub observation: Observation,
    pub map: MapObservation,
    pub inventory: Option<InventoryResult>,
    pub strategic_intent: StrategicIntent,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PerceptionSummary {
    pub scene: Option<String>,
    pub position_tile: Option<TilePosition>,
    pub alive: Option<bool>,
    pub recently_died: Option<bool>,
    pub material_change: bool,
    pub derived_event_count: usize,
    pub backend_event_count: usize,
    pub visible_entity_count: usize,
    pub visible_hostile_count: usize,
    pub hostiles_targeting_self_count: usize,
    pub nearest_hostile_distance_mill_tiles: Option<u32>,
    pub visible_player_count: usize,
    pub visible_npc_count: usize,
    pub visible_merchant_count: usize,
    pub visible_enemy_count: usize,
    pub visible_unknown_count: usize,
    pub drop_count: usize,
    pub positioned_drop_count: usize,
    pub unpositioned_drop_count: usize,
    pub carried_item_count: usize,
    pub carried_item_units: u64,
    pub door_count: usize,
    pub locked_door_count: usize,
    pub unknown_lock_door_count: usize,
    pub reported_total_object_count: Option<u32>,
    pub object_list_truncated: Option<bool>,
    pub new_dialogue_count: usize,
    pub new_scene_chat_count: usize,
    pub new_global_chat_count: usize,
    pub new_private_chat_count: usize,
    pub new_team_chat_count: usize,
    pub new_unknown_chat_count: usize,
    pub new_melody_count: usize,
    pub filtered_chat_count: usize,
    pub reachable_exit_count: usize,
    pub nearest_exit_path_length: Option<u32>,
    pub local_waypoint_count: usize,
    pub farthest_waypoint_path_length: Option<u32>,
    pub map_tile_count: usize,
    pub health: Option<i32>,
    pub max_health: Option<i32>,
    pub combat_active: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct PerceptionUpdate {
    pub observation_cycle_id: Uuid,
    pub observation_cycle_sequence: u64,
    pub frame: TacticalFrame,
    pub new_dialogue: Vec<DialogueLine>,
    pub summary: PerceptionSummary,
}

#[derive(Debug, Error, PartialEq)]
pub enum PerceptionError {
    #[error("{field} coordinate must be finite, got {value}")]
    InvalidCoordinate { field: &'static str, value: f32 },
    #[error("map dimensions exceed the supported coordinate range")]
    MapTooLarge,
}

#[derive(Default)]
pub struct PerceptionEngine {
    world_revision: u64,
    perception_revision: u64,
    map_revision: u64,
    inventory_revision: u64,
    derived_sequence: u64,
    previous_frame: Option<TacticalFrame>,
    previous_dialogue_window: Vec<DialogueLine>,
    seen_battle_events: VecDeque<BattleEventKey>,
    recent_events: VecDeque<GameEvent>,
    recent_actions: VecDeque<ActionOutcome>,
    combat_episode: CombatEpisodeReducer,
}

impl PerceptionEngine {
    /// Normalize one accepted observation and update the bounded fact windows.
    ///
    /// # Errors
    ///
    /// Returns [`PerceptionError`] when the backend supplies an invalid
    /// coordinate or an unsupported map size.
    #[allow(
        clippy::too_many_lines,
        reason = "one ordered reducer keeps revision and derived-event updates atomic"
    )]
    pub fn update(&mut self, input: PerceptionInput) -> Result<PerceptionUpdate, PerceptionError> {
        let observation_cycle_id = input.observation_cycle_id;
        let observation_cycle_sequence = input.observation_cycle_sequence;
        self.perception_revision = self.perception_revision.saturating_add(1);
        let dialogue_window = normalize_dialogue(&input.observation);
        let new_dialogue =
            new_dialogue_lines(&self.previous_dialogue_window, &dialogue_window.lines);
        let inventory = input
            .inventory
            .map_or(input.observation.carrying.clone(), |result| result.carrying);
        let self_state = normalize_self(&input.observation, &inventory)?;
        let nearby_entities = normalize_entities(&input.observation, self_state.position)?;
        let nearby_drops = normalize_drops(&input.observation, self_state.position)?;
        let mut map = normalize_map(
            &input.map,
            self_state.position,
            &nearby_entities,
            &nearby_drops,
            self.map_revision,
        )?;
        if self
            .previous_frame
            .as_ref()
            .is_none_or(|previous| map_facts_differ(&previous.map, &map))
        {
            self.map_revision = self.map_revision.saturating_add(1);
        }
        map.revision = self.map_revision;

        let inventory_changed = self
            .previous_frame
            .as_ref()
            .is_none_or(|previous| previous.self_state.inventory != self_state.inventory);
        if inventory_changed {
            self.inventory_revision = self.inventory_revision.saturating_add(1);
        }

        let combat = normalize_combat(
            &self_state,
            &nearby_entities,
            &input.observation,
            input.observed_at,
        );
        let census = normalize_census(&input.observation, &nearby_entities);
        let exits = normalize_exits(&map, self_state.position);
        let local_waypoints = normalize_waypoints(&map, self_state.position);
        let mut candidate = TacticalFrame {
            revision: self.world_revision,
            perception_revision: self.perception_revision,
            inventory_revision: self.inventory_revision,
            generated_at: input.observed_at,
            self_state,
            combat,
            census,
            nearby_entities,
            nearby_drops,
            map,
            exits,
            local_waypoints,
            recent_events: Vec::new(),
            recent_actions: Vec::new(),
            strategic_intent: input.strategic_intent,
        };

        let material_change = self
            .previous_frame
            .as_ref()
            .is_none_or(|previous| material_facts_differ(previous, &candidate));
        if material_change {
            self.world_revision = self.world_revision.saturating_add(1);
        }
        candidate.revision = self.world_revision;

        let (derived_event_count, backend_event_count) =
            self.update_event_state(&input.observation, &mut candidate, input.observed_at);

        let summary = summarize_frame(
            &candidate,
            material_change,
            derived_event_count,
            backend_event_count,
            &new_dialogue,
            dialogue_window.filtered_count,
        );
        self.previous_dialogue_window = dialogue_window.lines;
        self.previous_frame = Some(candidate.clone());
        Ok(PerceptionUpdate {
            observation_cycle_id,
            observation_cycle_sequence,
            frame: candidate,
            new_dialogue,
            summary,
        })
    }

    pub fn record_backend_event(&mut self, mut event: GameEvent) {
        event.origin = GameEventOrigin::Backend;
        self.recent_events.push_back(event);
        while self.recent_events.len() > MAX_RECENT_EVENTS {
            self.recent_events.pop_front();
        }
    }

    pub fn record_action(&mut self, outcome: ActionOutcome) {
        if self.recent_actions.len() == MAX_RECENT_ACTIONS {
            self.recent_actions.pop_front();
        }
        self.recent_actions.push_back(outcome);
    }

    fn update_event_state(
        &mut self,
        observation: &Observation,
        candidate: &mut TacticalFrame,
        observed_at: DateTime<Utc>,
    ) -> (usize, usize) {
        let backend_events = self.new_battle_events(observation, observed_at);
        let backend_event_count = backend_events.len();
        let authoritative_kinds = backend_events
            .iter()
            .map(|event| event.kind)
            .collect::<BTreeSet<_>>();
        self.recent_events.extend(backend_events.iter().cloned());

        let mut derived_events =
            derive_events(self.previous_frame.as_ref(), candidate, observed_at);
        derived_events.retain(|event| !authoritative_kinds.contains(&event.kind));
        let derived_event_count = derived_events.len();
        for event in &mut derived_events {
            self.derived_sequence = self.derived_sequence.saturating_add(1);
            event.sequence = Some(self.derived_sequence);
        }
        self.recent_events.extend(derived_events.iter().cloned());

        let mut new_events = backend_events;
        new_events.extend(derived_events);
        let oldest_episode_event = observed_at - Duration::seconds(EVENT_WINDOW_SECONDS);
        new_events.retain(|event| {
            event.observed_at >= oldest_episode_event && event.observed_at <= observed_at
        });
        prune_events(&mut self.recent_events, observed_at);
        candidate.recent_events = self.recent_events.iter().cloned().collect();
        candidate.recent_actions = self.recent_actions.iter().cloned().collect();
        candidate.combat.episode = self.combat_episode.update(
            candidate.combat.active,
            candidate.self_state.health,
            candidate.combat.current_hostiles,
            &new_events,
            observed_at,
        );

        (derived_event_count, backend_event_count)
    }

    fn new_battle_events(
        &mut self,
        observation: &Observation,
        observed_at: DateTime<Utc>,
    ) -> Vec<GameEvent> {
        let Some(battle) = &observation.battle else {
            return Vec::new();
        };
        let mut events = Vec::new();
        for source in &battle.events {
            let key = BattleEventKey::from(source);
            if self.seen_battle_events.contains(&key) {
                continue;
            }
            self.seen_battle_events.push_back(key);
            if self.seen_battle_events.len() > MAX_RECENT_EVENTS {
                self.seen_battle_events.pop_front();
            }
            if let Some(event) = normalize_battle_event(source, observed_at) {
                events.push(event);
            }
        }
        events
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BattleEventKey {
    sequence: Option<u64>,
    at: Option<String>,
    event_type: Option<String>,
    actor_id: Option<String>,
    target_id: Option<String>,
    amount: Option<i64>,
    damage_you_dealt: Option<i64>,
}

impl From<&ObservedBattleEvent> for BattleEventKey {
    fn from(event: &ObservedBattleEvent) -> Self {
        Self {
            sequence: event.seq,
            at: event.at.clone(),
            event_type: event.event_type.clone(),
            actor_id: event.actor_id.clone(),
            target_id: event.target_id.clone(),
            amount: event.amount,
            damage_you_dealt: event.damage_you_dealt,
        }
    }
}

fn normalize_self(
    observation: &Observation,
    inventory: &[ObservedItem],
) -> Result<SelfState, PerceptionError> {
    let state = observation
        .own_player
        .as_ref()
        .and_then(|player| player.state.as_ref());
    let position = state
        .and_then(|state| state.x.zip(state.y))
        .map(|(x, y)| position_from_pixels(x, y))
        .transpose()?;
    let battle_health = observation
        .battle
        .as_ref()
        .and_then(|battle| battle.hp.as_deref())
        .and_then(parse_health_pair);
    let health = state
        .and_then(|state| state.health)
        .or_else(|| battle_health.map(|(health, _)| health));
    let alive = state
        .and_then(|state| state.alive)
        .or_else(|| health.map(|value| value > 0));
    Ok(SelfState {
        scene: state
            .and_then(|state| state.scene.clone())
            .or_else(|| observation.scene_name.clone()),
        position,
        health,
        max_health: state
            .and_then(|state| state.max_health)
            .or_else(|| battle_health.map(|(_, maximum)| maximum)),
        level: state
            .and_then(|state| state.level)
            .or_else(|| observation.class_path.as_ref().and_then(|path| path.level)),
        experience: state.and_then(|state| state.experience),
        class_path: observation
            .class_path
            .as_ref()
            .and_then(|path| path.key.clone().or_else(|| path.label.clone()))
            .or_else(|| state.and_then(|state| state.class_path.clone())),
        alive,
        recently_died: observation.recently_died,
        moving: state.and_then(|state| state.moving),
        combat_actions: observation.skills.as_ref().map_or_else(
            || {
                state.map_or_else(Vec::new, |state| {
                    state
                        .combat_actions
                        .iter()
                        .map(normalize_combat_action)
                        .collect()
                })
            },
            |skills| {
                skills
                    .iter()
                    .map(|skill| normalize_legal_skill(skill))
                    .collect()
            },
        ),
        inventory: inventory.iter().map(normalize_item).collect(),
    })
}

fn normalize_legal_skill(skill: &str) -> CombatActionAvailability {
    CombatActionAvailability {
        id: skill.to_owned(),
        // Presence in Observation.skills establishes legality only. The
        // current backend does not report live cooldown availability here.
        available: None,
        cooldown_remaining_ms: None,
        target_kind: TargetKind::Unknown,
    }
}

fn normalize_combat_action(action: &ObservedCombatAction) -> CombatActionAvailability {
    CombatActionAvailability {
        id: action.id.clone(),
        available: action.available,
        cooldown_remaining_ms: action.cooldown_remaining_ms,
        target_kind: match action.target_kind.as_deref() {
            Some("none") => TargetKind::None,
            Some("self" | "self_target") => TargetKind::SelfTarget,
            Some("enemy" | "entity" | "player" | "object") => TargetKind::Entity,
            Some("position" | "tile") => TargetKind::Position,
            _ => TargetKind::Unknown,
        },
    }
}

fn normalize_item(item: &ObservedItem) -> CarriedItem {
    CarriedItem {
        id: item.key.clone(),
        label: item.label.clone(),
        quantity: item.quantity,
        usable: item.usable,
        equipment: item.equipment,
        equipped: item.equipped,
    }
}

fn normalize_entities(
    observation: &Observation,
    own_position: Option<Position>,
) -> Result<Vec<VisibleEntity>, PerceptionError> {
    let aggressors = observation
        .battle
        .as_ref()
        .map(|battle| {
            battle
                .aggressors
                .iter()
                .map(|aggressor| aggressor.object_index.as_str())
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let mut entities = Vec::new();
    for object in &observation.objects {
        let tile = TilePosition {
            x: object.tile_x,
            y: object.tile_y,
        };
        let kind = normalize_entity_kind(&object.kind);
        let hostile = (kind == EntityKind::Enemy)
            .then_some(object.alive)
            .flatten();
        entities.push(VisibleEntity {
            id: object.object_index.clone(),
            backend_object_id: object.object_id,
            label: object.label.clone(),
            kind,
            tile: Some(tile),
            relative: own_position.map(|own| relative(tile, own.tile)),
            // Production currently reports object distance in pixels. The
            // tactical contract is explicitly in tiles, so derive it from the
            // authoritative tile coordinates. If self position is unknown,
            // tile distance is unknown too; do not relabel pixels as tiles.
            distance: own_position.map(|own| tile_distance(tile, own.tile)),
            alive: object.alive,
            is_merchant: object.is_merchant,
            interactable: object.interactable,
            hostile,
            targeting_you: (kind == EntityKind::Enemy)
                .then_some(aggressors.contains(object.object_index.as_str())),
        });
    }
    for player in &observation.players {
        if is_own_player(player, observation.own_player.as_ref()) {
            continue;
        }
        let position = player_position(player)?;
        let id = player
            .session_id
            .clone()
            .or_else(|| player.player_id.map(|value| format!("player:{value}")))
            .unwrap_or_else(|| format!("player-name:{}", player_label(player)));
        entities.push(VisibleEntity {
            id,
            backend_object_id: None,
            label: player_label(player),
            kind: EntityKind::Player,
            tile: position.map(|value| value.tile),
            relative: position
                .zip(own_position)
                .map(|(other, own)| relative(other.tile, own.tile)),
            distance: position
                .zip(own_position)
                .map(|(other, own)| tile_distance(other.tile, own.tile)),
            alive: player.state.as_ref().and_then(|state| state.alive),
            is_merchant: None,
            interactable: None,
            hostile: None,
            targeting_you: None,
        });
    }
    entities.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(entities)
}

fn normalize_census(
    observation: &Observation,
    nearby_entities: &[VisibleEntity],
) -> VisibilityCensus {
    VisibilityCensus {
        reported_total_players: observation.total_players,
        listed_other_players: nearby_entities
            .iter()
            .filter(|entity| entity.kind == EntityKind::Player)
            .count(),
        reported_total_objects: observation.total_objects,
        listed_objects: observation.objects.len(),
        object_list_truncated: observation.total_objects.map(|total| {
            usize::try_from(total).map_or(true, |total| total > observation.objects.len())
        }),
    }
}

fn normalize_drops(
    observation: &Observation,
    own_position: Option<Position>,
) -> Result<Vec<Drop>, PerceptionError> {
    let mut drops = observation
        .drops
        .iter()
        .map(|drop| {
            let tile = drop_tile(drop)?;
            Ok(Drop {
                id: drop.drop_id.clone(),
                item_id: drop.item_key.clone(),
                label: None,
                tile,
                relative: tile
                    .zip(own_position)
                    .map(|(drop, own)| relative(drop, own.tile)),
                // As with objects, production's raw distance is in pixels.
                // Use an exact tile-space calculation when both positions are
                // available. Otherwise leave distance unknown.
                distance: tile
                    .zip(own_position)
                    .map(|(drop, own)| tile_distance(drop, own.tile)),
            })
        })
        .collect::<Result<Vec<_>, PerceptionError>>()?;
    drops.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(drops)
}

fn normalize_combat(
    _self_state: &SelfState,
    entities: &[VisibleEntity],
    observation: &Observation,
    observed_at: DateTime<Utc>,
) -> CombatSnapshot {
    let state = observation
        .own_player
        .as_ref()
        .and_then(|player| player.state.as_ref());
    let battle = observation.battle.as_ref();
    CombatSnapshot {
        active: battle
            .and_then(|battle| battle.in_battle)
            .or_else(|| state.and_then(|state| state.combat_active)),
        style: battle.and_then(|battle| battle.style.clone()),
        style_is_own_choice: battle.and_then(|battle| battle.style_is_own_choice),
        mode: battle.and_then(|battle| battle.mode.clone()),
        current_target_id: state.and_then(|state| state.current_target_id.clone()),
        current_hostiles: entities
            .iter()
            .filter(|entity| entity.hostile == Some(true))
            .count(),
        aggressors: battle.map_or_else(Vec::new, |battle| {
            battle
                .aggressors
                .iter()
                .map(|aggressor| CombatAggressor {
                    id: aggressor.object_index.clone(),
                    label: aggressor.label.clone(),
                    damage_dealt_to_you: aggressor.damage_dealt_to_you,
                })
                .collect()
        }),
        enemy_health: battle.map_or_else(Vec::new, |battle| {
            battle
                .enemy_health
                .iter()
                .map(|enemy| EnemyHealth {
                    id: enemy.object_index.clone(),
                    label: enemy.label.clone(),
                    health: enemy.hp,
                    max_health: enemy.max,
                })
                .collect()
        }),
        damage_dealt: battle.map_or_else(Vec::new, |battle| {
            battle
                .damage_dealt
                .iter()
                .map(|damage| DamageDealt {
                    target_id: damage.object_index.clone(),
                    label: damage.label.clone(),
                    amount: damage.amount,
                })
                .collect()
        }),
        damage_received_last_five_seconds: battle
            .map(|battle| recent_battle_damage(&battle.events, "damage_taken", observed_at)),
        damage_dealt_last_five_seconds: battle
            .map(|battle| recent_battle_damage(&battle.events, "damage_dealt", observed_at)),
        episode: None,
    }
}

fn normalize_map(
    source: &MapObservation,
    own_position: Option<Position>,
    entities: &[VisibleEntity],
    drops: &[Drop],
    current_revision: u64,
) -> Result<LocalMap, PerceptionError> {
    let lines = extract_grid_lines(source);
    let width = lines.iter().map(Vec::len).max().unwrap_or(0);
    let height = lines.len();
    let width_i32 = i32::try_from(width).map_err(|_| PerceptionError::MapTooLarge)?;
    let height_i32 = i32::try_from(height).map_err(|_| PerceptionError::MapTooLarge)?;
    let source_self = lines.iter().enumerate().find_map(|(y, line)| {
        line.iter()
            .position(|character| *character == '@')
            .map(|x| (x, y))
    });
    let explicit_origin = source
        .origin
        .map(|origin| position_from_pixels(origin.x, origin.y))
        .transpose()?
        .map(|position| position.tile);
    let full_scene_origin = source.scene_size.as_ref().and_then(|scene| {
        (scene.width_tiles == u32::try_from(width).ok()
            && scene.height_tiles == u32::try_from(height).ok())
        .then_some(TilePosition { x: 0, y: 0 })
    });
    let origin = explicit_origin.or(full_scene_origin).or_else(|| {
        own_position.map(|own| {
            source_self.map_or(
                TilePosition {
                    x: own.tile.x - width_i32 / 2,
                    y: own.tile.y - height_i32 / 2,
                },
                |(x, y)| TilePosition {
                    x: own.tile.x - i32::try_from(x).unwrap_or(i32::MAX),
                    y: own.tile.y - i32::try_from(y).unwrap_or(i32::MAX),
                },
            )
        })
    });
    let origin = origin.unwrap_or(TilePosition { x: 0, y: 0 });
    let mut tiles = Vec::with_capacity(width.saturating_mul(height));
    for (local_y, characters) in lines.iter().enumerate() {
        let local_y = i32::try_from(local_y).map_err(|_| PerceptionError::MapTooLarge)?;
        for local_x in 0..width {
            let local_x_i32 = i32::try_from(local_x).map_err(|_| PerceptionError::MapTooLarge)?;
            let character = characters.get(local_x).copied().unwrap_or(' ');
            let (kind, walkable) = tile_facts(character);
            tiles.push(MapTile {
                position: TilePosition {
                    x: origin.x.saturating_add(local_x_i32),
                    y: origin.y.saturating_add(local_y),
                },
                kind,
                walkable,
            });
        }
    }
    let mut doors = Vec::new();
    for door in &source.doors {
        if let Some(position) = door_position(door)? {
            if let Some(tile) = tiles.iter_mut().find(|tile| tile.position == position) {
                tile.kind = if door.locked == Some(true) {
                    TileKind::LockedDoor
                } else {
                    TileKind::Door
                };
                tile.walkable = door.locked.map(|locked| !locked);
            }
            doors.push(Doorway {
                tile: position,
                destination_scene: door.leads_to.clone(),
                label: door.label.clone(),
                locked: door.locked,
                lock_known: door.lock_known,
                required_key: door.required_key.clone(),
            });
        }
    }
    doors.sort_by_key(|door| (door.tile.y, door.tile.x));
    let mut map = LocalMap {
        revision: current_revision,
        origin_tile_x: origin.x,
        origin_tile_y: origin.y,
        width,
        height,
        tiles,
        doors,
        ascii: String::new(),
    };
    map.ascii = render_ascii(
        &map,
        own_position.map(|position| position.tile),
        entities,
        drops,
    );
    Ok(map)
}

fn normalize_exits(map: &LocalMap, own_position: Option<Position>) -> Vec<ReachableExit> {
    let Some(own_position) = own_position else {
        return Vec::new();
    };
    let path_lengths = local_path_lengths(map, own_position.tile);
    let mut exits = map
        .doors
        .iter()
        .filter_map(|door| {
            let path_length_tiles = path_lengths.get(&door.tile).copied()?;
            Some(ReachableExit {
                tile: door.tile,
                destination_scene: door.destination_scene.clone(),
                label: door.label.clone(),
                path_length_tiles,
            })
        })
        .collect::<Vec<_>>();
    exits.sort_by_key(|exit| (exit.path_length_tiles, exit.tile.y, exit.tile.x));
    exits
}

fn normalize_waypoints(map: &LocalMap, own_position: Option<Position>) -> Vec<ReachableWaypoint> {
    let Some(own_position) = own_position else {
        return Vec::new();
    };
    let start = own_position.tile;
    let path_lengths = local_path_lengths(map, start);
    let mut best = BTreeMap::<CardinalDirection, (i32, u32, TilePosition)>::new();
    for (tile, path_length) in path_lengths {
        if tile == start {
            continue;
        }
        let delta_x = tile.x.saturating_sub(start.x);
        let delta_y = tile.y.saturating_sub(start.y);
        let (direction, projection) = if delta_x.abs() >= delta_y.abs() {
            if delta_x > 0 {
                (CardinalDirection::East, delta_x)
            } else {
                (CardinalDirection::West, delta_x.saturating_abs())
            }
        } else if delta_y > 0 {
            (CardinalDirection::South, delta_y)
        } else {
            (CardinalDirection::North, delta_y.saturating_abs())
        };
        let candidate = (projection, path_length, tile);
        if best
            .get(&direction)
            .is_none_or(|current| candidate > *current)
        {
            best.insert(direction, candidate);
        }
    }
    best.into_iter()
        .map(
            |(direction, (_, path_length_tiles, tile))| ReachableWaypoint {
                tile,
                direction,
                path_length_tiles,
            },
        )
        .collect()
}

pub(crate) fn local_path_lengths(
    map: &LocalMap,
    start: TilePosition,
) -> BTreeMap<TilePosition, u32> {
    let walkable = map
        .tiles
        .iter()
        .filter(|tile| tile.walkable == Some(true))
        .map(|tile| tile.position)
        .collect::<BTreeSet<_>>();
    if !walkable.contains(&start) {
        return BTreeMap::new();
    }
    let mut distances = BTreeMap::from([(start, 0_u32)]);
    let mut frontier = VecDeque::from([start]);
    while let Some(current) = frontier.pop_front() {
        let distance = distances[&current];
        for neighbor in cardinal_neighbors(current) {
            if walkable.contains(&neighbor) && !distances.contains_key(&neighbor) {
                distances.insert(neighbor, distance.saturating_add(1));
                frontier.push_back(neighbor);
            }
        }
    }
    distances
}

pub(crate) fn cardinal_neighbors(tile: TilePosition) -> [TilePosition; 4] {
    [
        TilePosition {
            x: tile.x.saturating_sub(1),
            y: tile.y,
        },
        TilePosition {
            x: tile.x.saturating_add(1),
            y: tile.y,
        },
        TilePosition {
            x: tile.x,
            y: tile.y.saturating_sub(1),
        },
        TilePosition {
            x: tile.x,
            y: tile.y.saturating_add(1),
        },
    ]
}

fn derive_events(
    previous: Option<&TacticalFrame>,
    current: &TacticalFrame,
    observed_at: DateTime<Utc>,
) -> Vec<GameEvent> {
    let mut events = Vec::new();
    let Some(previous) = previous else {
        if let Some(scene) = &current.self_state.scene {
            events.push(event(
                observed_at,
                GameEventKind::SceneEntered,
                None,
                None,
                Some(scene.clone()),
            ));
        }
        for enemy in current
            .nearby_entities
            .iter()
            .filter(|entity| entity.hostile == Some(true))
        {
            events.push(event(
                observed_at,
                GameEventKind::EnemySeen,
                Some(enemy.id.clone()),
                None,
                None,
            ));
        }
        for drop in &current.nearby_drops {
            events.push(event(
                observed_at,
                GameEventKind::LootDropped,
                Some(drop.id.clone()),
                None,
                drop.item_id.clone(),
            ));
        }
        if current.self_state.recently_died == Some(true) {
            events.push(event(
                observed_at,
                GameEventKind::PlayerDied,
                None,
                None,
                Some("reported_by_backend_after_respawn".to_owned()),
            ));
        }
        return events;
    };

    derive_scene_events(previous, current, observed_at, &mut events);
    derive_health_events(previous, current, observed_at, &mut events);
    derive_progression_events(previous, current, observed_at, &mut events);
    derive_life_events(previous, current, observed_at, &mut events);
    derive_movement_events(previous, current, observed_at, &mut events);
    derive_target_events(previous, current, observed_at, &mut events);
    derive_enemy_events(previous, current, observed_at, &mut events);
    derive_drop_events(previous, current, observed_at, &mut events);
    events
}

fn derive_scene_events(
    previous: &TacticalFrame,
    current: &TacticalFrame,
    now: DateTime<Utc>,
    events: &mut Vec<GameEvent>,
) {
    if previous.self_state.scene != current.self_state.scene {
        if let Some(scene) = &previous.self_state.scene {
            events.push(event(
                now,
                GameEventKind::SceneLeft,
                None,
                None,
                Some(scene.clone()),
            ));
        }
        if let Some(scene) = &current.self_state.scene {
            events.push(event(
                now,
                GameEventKind::SceneEntered,
                None,
                None,
                Some(scene.clone()),
            ));
        }
    }
}

fn derive_health_events(
    previous: &TacticalFrame,
    current: &TacticalFrame,
    now: DateTime<Utc>,
    events: &mut Vec<GameEvent>,
) {
    if let (Some(before), Some(after)) = (previous.self_state.health, current.self_state.health) {
        let difference = i64::from(after) - i64::from(before);
        if difference < 0 {
            events.push(event(
                now,
                GameEventKind::DamageTaken,
                None,
                Some(-difference),
                None,
            ));
        } else if difference > 0 {
            events.push(event(
                now,
                GameEventKind::Heal,
                None,
                Some(difference),
                None,
            ));
        }
    }
}

fn derive_progression_events(
    previous: &TacticalFrame,
    current: &TacticalFrame,
    now: DateTime<Utc>,
    events: &mut Vec<GameEvent>,
) {
    if previous.self_state.level != current.self_state.level
        && let Some(level) = current.self_state.level
    {
        events.push(event(
            now,
            GameEventKind::LevelChanged,
            None,
            Some(i64::from(level)),
            None,
        ));
    }
    if previous.self_state.experience != current.self_state.experience
        && let Some(experience) = current.self_state.experience
    {
        events.push(event(
            now,
            GameEventKind::ExperienceChanged,
            None,
            Some(experience),
            None,
        ));
    }
}

fn derive_life_events(
    previous: &TacticalFrame,
    current: &TacticalFrame,
    now: DateTime<Utc>,
    events: &mut Vec<GameEvent>,
) {
    let backend_reported_death = current.self_state.recently_died == Some(true)
        && previous.self_state.recently_died != Some(true);
    if backend_reported_death {
        events.push(event(
            now,
            GameEventKind::PlayerDied,
            None,
            None,
            Some("reported_by_backend_after_respawn".to_owned()),
        ));
    }
    match (previous.self_state.alive, current.self_state.alive) {
        (Some(true), Some(false)) if !backend_reported_death => {
            events.push(event(now, GameEventKind::PlayerDied, None, None, None));
        }
        (Some(false), Some(true)) => {
            events.push(event(now, GameEventKind::PlayerRespawned, None, None, None));
        }
        _ => {}
    }
}

fn derive_movement_events(
    previous: &TacticalFrame,
    current: &TacticalFrame,
    now: DateTime<Utc>,
    events: &mut Vec<GameEvent>,
) {
    match (previous.self_state.moving, current.self_state.moving) {
        (Some(false), Some(true)) => {
            events.push(event(now, GameEventKind::MovementStarted, None, None, None));
        }
        (Some(true), Some(false)) => {
            events.push(event(now, GameEventKind::MovementStopped, None, None, None));
        }
        _ => {}
    }
}

fn derive_target_events(
    previous: &TacticalFrame,
    current: &TacticalFrame,
    now: DateTime<Utc>,
    events: &mut Vec<GameEvent>,
) {
    if previous.combat.current_target_id != current.combat.current_target_id {
        events.push(event(
            now,
            GameEventKind::TargetChanged,
            current.combat.current_target_id.clone(),
            None,
            None,
        ));
    }
}

fn derive_enemy_events(
    previous: &TacticalFrame,
    current: &TacticalFrame,
    now: DateTime<Utc>,
    events: &mut Vec<GameEvent>,
) {
    let old = hostile_ids(previous);
    let new = hostile_ids(current);
    for id in new.difference(&old) {
        events.push(event(
            now,
            GameEventKind::EnemySpawned,
            Some(id.clone()),
            None,
            None,
        ));
    }
    for id in old.difference(&new) {
        events.push(event(
            now,
            GameEventKind::EnemyDespawned,
            Some(id.clone()),
            None,
            None,
        ));
    }
}

fn derive_drop_events(
    previous: &TacticalFrame,
    current: &TacticalFrame,
    now: DateTime<Utc>,
    events: &mut Vec<GameEvent>,
) {
    let old = previous
        .nearby_drops
        .iter()
        .map(|drop| (drop.id.clone(), drop.item_id.clone()))
        .collect::<BTreeMap<_, _>>();
    let new = current
        .nearby_drops
        .iter()
        .map(|drop| (drop.id.clone(), drop.item_id.clone()))
        .collect::<BTreeMap<_, _>>();
    for (id, item) in &new {
        if !old.contains_key(id) {
            events.push(event(
                now,
                GameEventKind::LootDropped,
                Some(id.clone()),
                None,
                item.clone(),
            ));
        }
    }
    let inventory_increases = inventory_increases(previous, current);
    for (id, item) in &old {
        if !new.contains_key(id)
            && item
                .as_ref()
                .is_some_and(|item_id| inventory_increases.contains(item_id))
        {
            events.push(event(
                now,
                GameEventKind::LootPickedUp,
                Some(id.clone()),
                None,
                item.clone(),
            ));
        }
    }
}

fn event(
    observed_at: DateTime<Utc>,
    kind: GameEventKind,
    entity_id: Option<String>,
    amount: Option<i64>,
    detail: Option<String>,
) -> GameEvent {
    GameEvent {
        sequence: None,
        observed_at,
        origin: GameEventOrigin::Derived,
        kind,
        entity_id,
        amount,
        tile: None,
        detail,
    }
}

fn hostile_ids(frame: &TacticalFrame) -> BTreeSet<String> {
    frame
        .nearby_entities
        .iter()
        .filter(|entity| entity.hostile == Some(true))
        .map(|entity| entity.id.clone())
        .collect()
}

fn inventory_increases(previous: &TacticalFrame, current: &TacticalFrame) -> BTreeSet<String> {
    let old = previous
        .self_state
        .inventory
        .iter()
        .map(|item| (item.id.as_str(), item.quantity))
        .collect::<BTreeMap<_, _>>();
    current
        .self_state
        .inventory
        .iter()
        .filter(|item| item.quantity > old.get(item.id.as_str()).copied().unwrap_or(0))
        .map(|item| item.id.clone())
        .collect()
}

fn material_facts_differ(previous: &TacticalFrame, current: &TacticalFrame) -> bool {
    previous.self_state != current.self_state
        || combat_facts_differ(&previous.combat, &current.combat)
        || previous.census != current.census
        || previous.nearby_entities != current.nearby_entities
        || previous.nearby_drops != current.nearby_drops
        || map_facts_differ(&previous.map, &current.map)
        || previous.exits != current.exits
}

fn combat_facts_differ(previous: &CombatSnapshot, current: &CombatSnapshot) -> bool {
    previous.active != current.active
        || previous.style != current.style
        || previous.style_is_own_choice != current.style_is_own_choice
        || previous.mode != current.mode
        || previous.current_target_id != current.current_target_id
        || previous.current_hostiles != current.current_hostiles
        || previous.aggressors != current.aggressors
        || previous.enemy_health != current.enemy_health
        || previous.damage_dealt != current.damage_dealt
        || previous.damage_received_last_five_seconds != current.damage_received_last_five_seconds
        || previous.damage_dealt_last_five_seconds != current.damage_dealt_last_five_seconds
}

fn map_facts_differ(previous: &LocalMap, current: &LocalMap) -> bool {
    previous.origin_tile_x != current.origin_tile_x
        || previous.origin_tile_y != current.origin_tile_y
        || previous.width != current.width
        || previous.height != current.height
        || previous.doors != current.doors
        || layout_tiles(previous) != layout_tiles(current)
}

fn layout_tiles(map: &LocalMap) -> BTreeMap<TilePosition, TileKind> {
    map.tiles
        .iter()
        .filter(|tile| {
            matches!(
                tile.kind,
                TileKind::Blocked | TileKind::Door | TileKind::LockedDoor
            )
        })
        .map(|tile| (tile.position, tile.kind))
        .collect()
}

fn extract_grid_lines(source: &MapObservation) -> Vec<Vec<char>> {
    let raw = source.map.as_deref().map_or_else(Vec::new, |map| {
        map.lines()
            .map(|line| line.chars().collect::<Vec<_>>())
            .collect::<Vec<_>>()
    });
    let Some(side) = source
        .requested_radius
        .and_then(|radius| radius.checked_mul(2))
        .and_then(|diameter| diameter.checked_add(1))
        .and_then(|side| usize::try_from(side).ok())
    else {
        return raw;
    };
    raw.windows(side)
        .find(|window| window.iter().all(|line| line.len() == side))
        .map_or(raw.clone(), <[Vec<char>]>::to_vec)
}

fn prune_events(events: &mut VecDeque<GameEvent>, now: DateTime<Utc>) {
    let oldest = now - Duration::seconds(EVENT_WINDOW_SECONDS);
    events.retain(|event| event.observed_at >= oldest);
    while events.len() > MAX_RECENT_EVENTS {
        events.pop_front();
    }
}

fn position_from_pixels(x: f32, y: f32) -> Result<Position, PerceptionError> {
    ensure_finite(x, "x")?;
    ensure_finite(y, "y")?;
    Ok(Position {
        pixel: PixelPosition { x, y },
        tile: TilePosition {
            x: floor_to_i32(x / TILE_SIZE_PIXELS),
            y: floor_to_i32(y / TILE_SIZE_PIXELS),
        },
    })
}

fn player_position(player: &ObservedPlayer) -> Result<Option<Position>, PerceptionError> {
    player
        .state
        .as_ref()
        .and_then(|state| state.x.zip(state.y))
        .map(|(x, y)| position_from_pixels(x, y))
        .transpose()
}

fn door_position(
    door: &crate::mcp::types::MapDoor,
) -> Result<Option<TilePosition>, PerceptionError> {
    if let (Some(x), Some(y)) = (door.tile_x, door.tile_y) {
        return Ok(Some(TilePosition { x, y }));
    }
    door.x
        .zip(door.y)
        .map(|(x, y)| position_from_pixels(x, y).map(|position| position.tile))
        .transpose()
}

fn drop_tile(
    drop: &crate::mcp::observation::ObservedDrop,
) -> Result<Option<TilePosition>, PerceptionError> {
    if let (Some(x), Some(y)) = (drop.tile_x, drop.tile_y) {
        return Ok(Some(TilePosition { x, y }));
    }
    drop.x
        .zip(drop.y)
        .map(|(x, y)| position_from_pixels(x, y).map(|position| position.tile))
        .transpose()
}

fn parse_health_pair(value: &str) -> Option<(i32, i32)> {
    let (health, maximum) = value.split_once('/')?;
    Some((health.trim().parse().ok()?, maximum.trim().parse().ok()?))
}

fn recent_battle_damage(
    events: &[ObservedBattleEvent],
    event_type: &str,
    observed_at: DateTime<Utc>,
) -> i64 {
    let oldest = observed_at - Duration::seconds(5);
    events
        .iter()
        .filter(|event| event.event_type.as_deref() == Some(event_type))
        .filter(|event| {
            event
                .at
                .as_deref()
                .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
                .is_some_and(|at| at >= oldest && at <= observed_at)
        })
        .filter_map(|event| event.amount)
        .filter(|amount| *amount > 0)
        .sum()
}

fn normalize_battle_event(
    source: &ObservedBattleEvent,
    observed_at: DateTime<Utc>,
) -> Option<GameEvent> {
    let event_type = source.event_type.as_deref()?;
    let (kind, entity_id) = match event_type {
        "damage_taken" => (GameEventKind::DamageTaken, source.actor_id.clone()),
        "damage_dealt" => (GameEventKind::DamageDealt, source.target_id.clone()),
        "you_died" => (GameEventKind::PlayerDied, None),
        "enemy_killed" => (GameEventKind::TargetKilled, source.target_id.clone()),
        "aggressor_died" => (GameEventKind::EnemyDespawned, source.target_id.clone()),
        _ => return None,
    };
    let event_time = source
        .at
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map_or(observed_at, |value| value.with_timezone(&Utc));
    Some(GameEvent {
        sequence: source.seq,
        observed_at: event_time,
        origin: GameEventOrigin::Backend,
        kind,
        entity_id,
        amount: source.amount.or(source.damage_you_dealt),
        tile: None,
        detail: source.label.as_ref().map_or_else(
            || Some(event_type.to_owned()),
            |label| Some(format!("{event_type}:{label}")),
        ),
    })
}

fn ensure_finite(value: f32, field: &'static str) -> Result<(), PerceptionError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(PerceptionError::InvalidCoordinate { field, value })
    }
}

fn floor_to_i32(value: f32) -> i32 {
    value.floor().to_i32().unwrap_or_else(|| {
        if value.is_sign_negative() {
            i32::MIN
        } else {
            i32::MAX
        }
    })
}

fn normalize_entity_kind(kind: &str) -> EntityKind {
    match kind.trim().to_ascii_lowercase().as_str() {
        "enemy" | "hostile" | "mob" | "monster" => EntityKind::Enemy,
        "npc" | "merchant" | "folk" => EntityKind::Npc,
        "player" => EntityKind::Player,
        _ => EntityKind::Unknown,
    }
}

fn player_label(player: &ObservedPlayer) -> String {
    player
        .player_name
        .clone()
        .or_else(|| player.name.clone())
        .or_else(|| player.label.clone())
        .unwrap_or_else(|| "Unknown player".to_owned())
}

fn is_own_player(player: &ObservedPlayer, own: Option<&ObservedPlayer>) -> bool {
    own.is_some_and(|own| {
        player.session_id.is_some() && player.session_id == own.session_id
            || player.player_id.is_some() && player.player_id == own.player_id
    })
}

fn relative(position: TilePosition, origin: TilePosition) -> TilePosition {
    TilePosition {
        x: position.x.saturating_sub(origin.x),
        y: position.y.saturating_sub(origin.y),
    }
}

fn tile_distance(left: TilePosition, right: TilePosition) -> f32 {
    let dx = f64::from(left.x) - f64::from(right.x);
    let dy = f64::from(left.y) - f64::from(right.y);
    dx.hypot(dy).to_f32().unwrap_or(f32::MAX)
}

fn tile_facts(character: char) -> (TileKind, Option<bool>) {
    match character {
        '#' => (TileKind::Blocked, Some(false)),
        '.' | '@' | 'E' | 'N' | 'P' | 'S' | '*' => (TileKind::Traversable, Some(true)),
        'D' | '+' => (TileKind::Door, None),
        'L' => (TileKind::LockedDoor, Some(false)),
        _ => (TileKind::Unknown, None),
    }
}

fn render_ascii(
    map: &LocalMap,
    own_tile: Option<TilePosition>,
    entities: &[VisibleEntity],
    drops: &[Drop],
) -> String {
    let hostiles = entities
        .iter()
        .filter(|entity| entity.hostile == Some(true))
        .filter_map(|entity| entity.tile)
        .collect::<BTreeSet<_>>();
    let drop_tiles = drops
        .iter()
        .filter_map(|drop| drop.tile)
        .collect::<BTreeSet<_>>();
    let tiles = map
        .tiles
        .iter()
        .map(|tile| (tile.position, tile))
        .collect::<BTreeMap<_, _>>();
    let mut lines = Vec::with_capacity(map.height);
    for local_y in 0..map.height {
        let mut line = String::with_capacity(map.width);
        for local_x in 0..map.width {
            let position = TilePosition {
                x: map
                    .origin_tile_x
                    .saturating_add(i32::try_from(local_x).unwrap_or(i32::MAX)),
                y: map
                    .origin_tile_y
                    .saturating_add(i32::try_from(local_y).unwrap_or(i32::MAX)),
            };
            let character = if own_tile == Some(position) {
                '@'
            } else if hostiles.contains(&position) {
                'S'
            } else if drop_tiles.contains(&position) {
                '*'
            } else {
                match tiles.get(&position).map(|tile| tile.kind) {
                    Some(TileKind::Traversable) => '.',
                    Some(TileKind::Blocked) => '#',
                    Some(TileKind::Door) => 'D',
                    Some(TileKind::LockedDoor) => 'L',
                    Some(TileKind::Unknown) | None => ' ',
                }
            };
            line.push(character);
        }
        lines.push(line);
    }
    lines.join("\n")
}

fn count_entity_kind(frame: &TacticalFrame, kind: EntityKind) -> usize {
    frame
        .nearby_entities
        .iter()
        .filter(|entity| entity.kind == kind)
        .count()
}

fn summarize_frame(
    frame: &TacticalFrame,
    material_change: bool,
    derived_event_count: usize,
    backend_event_count: usize,
    new_dialogue: &[DialogueLine],
    filtered_chat_count: usize,
) -> PerceptionSummary {
    PerceptionSummary {
        scene: frame.self_state.scene.clone(),
        position_tile: frame.self_state.position.map(|position| position.tile),
        alive: frame.self_state.alive,
        recently_died: frame.self_state.recently_died,
        material_change,
        derived_event_count,
        backend_event_count,
        visible_entity_count: frame.nearby_entities.len(),
        visible_hostile_count: frame
            .nearby_entities
            .iter()
            .filter(|entity| entity.hostile == Some(true))
            .count(),
        hostiles_targeting_self_count: frame
            .nearby_entities
            .iter()
            .filter(|entity| entity.hostile == Some(true) && entity.targeting_you == Some(true))
            .count(),
        nearest_hostile_distance_mill_tiles: nearest_hostile_distance_mill_tiles(frame),
        visible_player_count: count_entity_kind(frame, EntityKind::Player),
        visible_npc_count: count_entity_kind(frame, EntityKind::Npc),
        visible_merchant_count: frame
            .nearby_entities
            .iter()
            .filter(|entity| entity.is_merchant == Some(true))
            .count(),
        visible_enemy_count: count_entity_kind(frame, EntityKind::Enemy),
        visible_unknown_count: count_entity_kind(frame, EntityKind::Unknown),
        drop_count: frame.nearby_drops.len(),
        positioned_drop_count: frame
            .nearby_drops
            .iter()
            .filter(|drop| drop.tile.is_some())
            .count(),
        unpositioned_drop_count: frame
            .nearby_drops
            .iter()
            .filter(|drop| drop.tile.is_none())
            .count(),
        carried_item_count: frame.self_state.inventory.len(),
        carried_item_units: frame
            .self_state
            .inventory
            .iter()
            .map(|item| u64::from(item.quantity))
            .sum(),
        door_count: frame.map.doors.len(),
        locked_door_count: frame
            .map
            .doors
            .iter()
            .filter(|door| door.locked == Some(true))
            .count(),
        unknown_lock_door_count: frame
            .map
            .doors
            .iter()
            .filter(|door| door.locked.is_none())
            .count(),
        reported_total_object_count: frame.census.reported_total_objects,
        object_list_truncated: frame.census.object_list_truncated,
        new_dialogue_count: new_dialogue.len(),
        new_scene_chat_count: count_dialogue_channel(new_dialogue, DialogueChannel::Scene),
        new_global_chat_count: count_dialogue_channel(new_dialogue, DialogueChannel::Global),
        new_private_chat_count: count_dialogue_channel(new_dialogue, DialogueChannel::Private),
        new_team_chat_count: count_dialogue_channel(new_dialogue, DialogueChannel::Team),
        new_unknown_chat_count: count_dialogue_channel(new_dialogue, DialogueChannel::Unknown),
        new_melody_count: new_dialogue
            .iter()
            .filter(|line| line.kind == DialogueKind::Melody)
            .count(),
        filtered_chat_count,
        reachable_exit_count: frame.exits.len(),
        nearest_exit_path_length: frame.exits.iter().map(|exit| exit.path_length_tiles).min(),
        local_waypoint_count: frame.local_waypoints.len(),
        farthest_waypoint_path_length: frame
            .local_waypoints
            .iter()
            .map(|waypoint| waypoint.path_length_tiles)
            .max(),
        map_tile_count: frame.map.tiles.len(),
        health: frame.self_state.health,
        max_health: frame.self_state.max_health,
        combat_active: frame.combat.active,
    }
}

fn nearest_hostile_distance_mill_tiles(frame: &TacticalFrame) -> Option<u32> {
    frame
        .nearby_entities
        .iter()
        .filter(|entity| entity.hostile == Some(true))
        .filter_map(|entity| entity.distance)
        .filter(|distance| distance.is_finite() && *distance >= 0.0)
        .min_by(f32::total_cmp)
        .and_then(|distance| (distance * 1_000.0).round().to_u32())
}

fn count_dialogue_channel(lines: &[DialogueLine], channel: DialogueChannel) -> usize {
    lines.iter().filter(|line| line.channel == channel).count()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::mcp::{
        observation::{ObservedDrop, ObservedObject, ObservedPlayerState},
        types::MapDoor,
    };

    fn time(second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 11, 12, 0, second)
            .single()
            .expect("time")
    }

    fn input(second: u32) -> PerceptionInput {
        PerceptionInput {
            observation_cycle_id: Uuid::from_u128(u128::from(second) + 1),
            observation_cycle_sequence: u64::from(second) + 1,
            observation: Observation {
                own_player: Some(ObservedPlayer {
                    player_name: Some("Cassian Vey Unbound".to_owned()),
                    state: Some(ObservedPlayerState {
                        scene: Some("reldens-town".to_owned()),
                        x: Some(336.0),
                        y: Some(368.0),
                        health: Some(100),
                        max_health: Some(100),
                        level: Some(1),
                        experience: Some(0),
                        alive: Some(true),
                        moving: Some(false),
                        ..ObservedPlayerState::default()
                    }),
                    ..ObservedPlayer::default()
                }),
                scene_name: Some("reldens-town".to_owned()),
                ..Observation::default()
            },
            map: MapObservation {
                grid_available: Some(true),
                map: Some("#####\n#.@.#\n#####".to_owned()),
                ..MapObservation::default()
            },
            inventory: Some(InventoryResult::default()),
            strategic_intent: StrategicIntent::default(),
            observed_at: time(second),
        }
    }

    fn battle_fixture() -> crate::mcp::observation::ObservedBattle {
        serde_json::from_value(serde_json::json!({
            "inBattle": true,
            "style": "duck_and_weave",
            "styleIsOwnChoice": true,
            "mode": "semi_auto",
            "hp": "73/100",
            "aggressors": [{
                "label": "Spider",
                "objectIndex": "spider-92",
                "damageDealtToYou": 27
            }],
            "enemyHealth": [{
                "objectIndex": "spider-92",
                "label": "Spider",
                "hp": 9,
                "max": 20
            }],
            "damageDealt": [{
                "objectIndex": "spider-92",
                "label": "Spider",
                "amount": 31
            }],
            "events": [
                {
                    "seq": 1,
                    "at": "2026-08-11T12:00:03Z",
                    "event_type": "damage_taken",
                    "amount": 7
                },
                {
                    "seq": 2,
                    "at": "2026-08-11T12:00:04Z",
                    "event_type": "damage_dealt",
                    "amount": 11
                },
                {
                    "seq": 3,
                    "at": "2026-08-11T11:59:00Z",
                    "event_type": "damage_taken",
                    "amount": 99
                },
                {
                    "seq": 4,
                    "at": "2026-08-11T12:00:05Z",
                    "event_type": "enemy_killed",
                    "target_id": "spider-92",
                    "label": "Spider",
                    "damageYouDealt": 31
                }
            ]
        }))
        .expect("battle")
    }

    #[test]
    fn normalizes_position_map_and_unknown_backend_fields() {
        let update = PerceptionEngine::default()
            .update(input(0))
            .expect("normalized frame");

        let self_state = &update.frame.self_state;
        assert_eq!(
            self_state.position,
            Some(Position {
                pixel: PixelPosition { x: 336.0, y: 368.0 },
                tile: TilePosition { x: 10, y: 11 },
            })
        );
        assert_eq!(self_state.class_path, None);
        assert_eq!(update.frame.combat.active, None);
        assert_eq!(update.frame.map.origin_tile_x, 8);
        assert_eq!(update.frame.map.origin_tile_y, 10);
        assert_eq!(update.frame.map.ascii, "#####\n#.@.#\n#####");
        assert_eq!(update.frame.map.tiles.len(), 15);
        assert_eq!(
            update
                .frame
                .local_waypoints
                .iter()
                .map(|waypoint| (
                    waypoint.direction,
                    waypoint.tile,
                    waypoint.path_length_tiles
                ))
                .collect::<Vec<_>>(),
            vec![
                (CardinalDirection::East, TilePosition { x: 11, y: 11 }, 1),
                (CardinalDirection::West, TilePosition { x: 9, y: 11 }, 1),
            ]
        );
        assert_eq!(update.summary.local_waypoint_count, 2);
        assert_eq!(update.summary.farthest_waypoint_path_length, Some(1));
    }

    #[test]
    fn uses_top_level_world_class_and_legal_skills_without_inventing_availability() {
        let mut current = input(0);
        current.observation.class_path = Some(crate::mcp::observation::ObservedClassPath {
            key: None,
            label: Some("Swordsman".to_owned()),
            level: Some(4),
        });
        current.observation.skills = Some(vec!["attackShort".to_owned(), "slash".to_owned()]);
        let state = current
            .observation
            .own_player
            .as_mut()
            .and_then(|player| player.state.as_mut())
            .expect("state");
        state.level = None;
        state.class_path = Some("obsolete-local-copy".to_owned());
        state.combat_actions = vec![ObservedCombatAction {
            id: "obsolete-skill".to_owned(),
            available: Some(true),
            cooldown_remaining_ms: Some(0),
            target_kind: Some("enemy".to_owned()),
        }];

        let update = PerceptionEngine::default()
            .update(current)
            .expect("current production class contract");

        assert_eq!(
            update.frame.self_state.class_path.as_deref(),
            Some("Swordsman")
        );
        assert_eq!(update.frame.self_state.level, Some(4));
        assert_eq!(
            update
                .frame
                .self_state
                .combat_actions
                .iter()
                .map(|action| action.id.as_str())
                .collect::<Vec<_>>(),
            vec!["attackShort", "slash"]
        );
        assert!(update.frame.self_state.combat_actions.iter().all(|action| {
            action.available.is_none()
                && action.cooldown_remaining_ms.is_none()
                && action.target_kind == TargetKind::Unknown
        }));
    }

    #[test]
    fn an_authoritative_empty_skill_list_does_not_fall_back_to_legacy_actions() {
        let mut current = input(0);
        current.observation.skills = Some(Vec::new());
        current
            .observation
            .own_player
            .as_mut()
            .and_then(|player| player.state.as_mut())
            .expect("state")
            .combat_actions
            .push(ObservedCombatAction {
                id: "legacy-only".to_owned(),
                available: Some(true),
                cooldown_remaining_ms: Some(0),
                target_kind: Some("enemy".to_owned()),
            });

        let update = PerceptionEngine::default()
            .update(current)
            .expect("authoritative empty list");
        assert!(update.frame.self_state.combat_actions.is_empty());
    }

    #[test]
    fn exposes_live_enemies_equipment_usable_items_and_ground_drops() {
        let mut current = input(0);
        current.observation.objects.extend([
            ObservedObject {
                object_id: Some(92),
                object_index: "spider-live".to_owned(),
                label: "Spider".to_owned(),
                kind: "enemy".to_owned(),
                alive: Some(true),
                is_merchant: None,
                interactable: Some(false),
                distance_from_self: Some(2.0),
                tile_x: 11,
                tile_y: 11,
            },
            ObservedObject {
                object_id: Some(93),
                object_index: "spider-dead".to_owned(),
                label: "Spider".to_owned(),
                kind: "enemy".to_owned(),
                alive: Some(false),
                is_merchant: None,
                interactable: Some(false),
                distance_from_self: Some(3.0),
                tile_x: 12,
                tile_y: 11,
            },
        ]);
        current.observation.drops.push(ObservedDrop {
            drop_id: "drop-silk-1".to_owned(),
            item_key: Some("spider-silk".to_owned()),
            x: None,
            y: None,
            tile_x: Some(10),
            tile_y: Some(11),
            distance_from_self: Some(1.0),
        });
        current.inventory = Some(InventoryResult {
            carrying: vec![
                ObservedItem {
                    key: "minor-healing-potion".to_owned(),
                    label: "Minor Healing Potion".to_owned(),
                    description: Some("Restores health.".to_owned()),
                    quantity: 2,
                    usable: Some(true),
                    equipment: Some(false),
                    equipped: Some(false),
                },
                ObservedItem {
                    key: "iron-longsword".to_owned(),
                    label: "Iron Longsword".to_owned(),
                    description: Some("A balanced blade.".to_owned()),
                    quantity: 1,
                    usable: Some(false),
                    equipment: Some(true),
                    equipped: Some(true),
                },
            ],
        });

        let update = PerceptionEngine::default()
            .update(current)
            .expect("combat inventory facts");
        let living = update
            .frame
            .nearby_entities
            .iter()
            .find(|entity| entity.id == "spider-live")
            .expect("living enemy");
        let dead = update
            .frame
            .nearby_entities
            .iter()
            .find(|entity| entity.id == "spider-dead")
            .expect("dead enemy");
        assert_eq!(living.hostile, Some(true));
        assert_eq!(dead.hostile, Some(false));
        assert_eq!(update.frame.nearby_drops[0].id, "drop-silk-1");
        assert_eq!(
            update.frame.nearby_drops[0].item_id.as_deref(),
            Some("spider-silk")
        );
        let sword = update
            .frame
            .self_state
            .inventory
            .iter()
            .find(|item| item.id == "iron-longsword")
            .expect("sword");
        assert_eq!(sword.equipment, Some(true));
        assert_eq!(sword.equipped, Some(true));
        let potion = update
            .frame
            .self_state
            .inventory
            .iter()
            .find(|item| item.id == "minor-healing-potion")
            .expect("potion");
        assert_eq!(potion.usable, Some(true));
        assert_eq!(potion.quantity, 2);
    }

    #[test]
    fn normalizes_production_pixel_distances_into_tile_distances() {
        let mut current = input(0);
        current.observation.objects.push(ObservedObject {
            object_id: Some(92),
            object_index: "spider-distance".to_owned(),
            label: "Spider".to_owned(),
            kind: "enemy".to_owned(),
            alive: Some(true),
            is_merchant: None,
            interactable: Some(false),
            distance_from_self: Some(160.0),
            tile_x: 13,
            tile_y: 15,
        });
        current.observation.drops.push(ObservedDrop {
            drop_id: "drop-distance".to_owned(),
            item_key: Some("spider-silk".to_owned()),
            x: Some(528.0),
            y: Some(624.0),
            tile_x: Some(16),
            tile_y: Some(19),
            distance_from_self: Some(320.0),
        });

        let update = PerceptionEngine::default()
            .update(current)
            .expect("normalized frame");

        assert_eq!(update.frame.nearby_entities[0].distance, Some(5.0));
        assert_eq!(update.frame.nearby_drops[0].distance, Some(10.0));
    }

    #[test]
    fn anchors_a_complete_scene_map_at_zero() {
        let mut complete = input(0);
        complete.map.scene_size = Some(crate::mcp::types::SceneSize {
            width_tiles: Some(5),
            height_tiles: Some(3),
        });
        let update = PerceptionEngine::default()
            .update(complete)
            .expect("complete map");

        assert_eq!(update.frame.map.origin_tile_x, 0);
        assert_eq!(update.frame.map.origin_tile_y, 0);
    }

    #[test]
    fn extracts_the_requested_grid_from_presentation_text() {
        let mut presented = input(0);
        presented.map.requested_radius = Some(2);
        presented.map.map = Some(
            "Map for Cassian with a long presentation header\n     \n#####\n#.@.#\n#####\n     \nLegend: @ is you and # is blocked"
                .to_owned(),
        );
        let update = PerceptionEngine::default()
            .update(presented)
            .expect("presented map");

        assert_eq!(update.frame.map.width, 5);
        assert_eq!(update.frame.map.height, 5);
        assert_eq!(update.frame.map.tiles.len(), 25);
        assert_eq!(update.frame.map.ascii.lines().count(), 5);
    }

    #[test]
    fn exposes_positioned_drops_and_locked_doors_as_distinct_facts() {
        let mut current = input(0);
        current.observation.drops.push(ObservedDrop {
            drop_id: "drop-82".to_owned(),
            item_key: Some("spider-silk".to_owned()),
            x: Some(288.0),
            y: Some(352.0),
            tile_x: None,
            tile_y: None,
            distance_from_self: None,
        });
        current.map.doors.push(MapDoor {
            x: None,
            y: None,
            tile_x: Some(11),
            tile_y: Some(11),
            leads_to: Some("arena-depths".to_owned()),
            label: Some("sealed arch".to_owned()),
            locked: Some(true),
            lock_known: Some(true),
            required_key: Some("depths-key".to_owned()),
        });
        current.map.doors.push(MapDoor {
            x: None,
            y: None,
            tile_x: Some(9),
            tile_y: Some(11),
            leads_to: Some("arena-road".to_owned()),
            label: Some("open arch".to_owned()),
            locked: Some(false),
            lock_known: Some(true),
            required_key: None,
        });

        let update = PerceptionEngine::default()
            .update(current)
            .expect("normalized frame");

        assert_eq!(
            update.frame.nearby_drops[0].tile,
            Some(TilePosition { x: 9, y: 11 })
        );
        assert_eq!(
            update.frame.nearby_drops[0].relative,
            Some(TilePosition { x: -1, y: 0 })
        );
        assert_eq!(update.summary.positioned_drop_count, 1);
        assert_eq!(update.summary.unpositioned_drop_count, 0);
        assert!(update.frame.map.ascii.contains('*'));
        assert!(update.frame.map.ascii.contains('L'));
        let locked = update
            .frame
            .map
            .doors
            .iter()
            .find(|door| door.locked == Some(true))
            .expect("locked door");
        assert_eq!(locked.locked, Some(true));
        assert_eq!(locked.required_key.as_deref(), Some("depths-key"));
        assert_eq!(update.frame.exits.len(), 1);
        assert_eq!(update.frame.exits[0].tile, TilePosition { x: 9, y: 11 });
        assert_eq!(update.frame.exits[0].path_length_tiles, 1);
    }

    #[test]
    fn uses_authoritative_battle_facts_for_health_targeting_and_recent_damage() {
        let mut current = input(5);
        let state = current
            .observation
            .own_player
            .as_mut()
            .and_then(|player| player.state.as_mut())
            .expect("state");
        state.health = None;
        state.max_health = None;
        current.observation.battle = Some(battle_fixture());
        current.observation.objects.push(ObservedObject {
            object_id: Some(92),
            object_index: "spider-92".to_owned(),
            label: "Spider".to_owned(),
            kind: "enemy".to_owned(),
            alive: Some(true),
            is_merchant: None,
            interactable: None,
            distance_from_self: Some(2.0),
            tile_x: 11,
            tile_y: 11,
        });
        current.observation.total_objects = Some(100);

        let mut engine = PerceptionEngine::default();
        let repeated = current.clone();
        let update = engine.update(current).expect("frame");

        assert_eq!(update.frame.self_state.health, Some(73));
        assert_eq!(update.frame.self_state.max_health, Some(100));
        assert_eq!(update.frame.combat.active, Some(true));
        assert_eq!(update.frame.combat.style.as_deref(), Some("duck_and_weave"));
        assert_eq!(
            update.frame.combat.damage_received_last_five_seconds,
            Some(7)
        );
        assert_eq!(update.frame.combat.damage_dealt_last_five_seconds, Some(11));
        assert_eq!(update.frame.combat.aggressors[0].id, "spider-92");
        assert_eq!(update.frame.combat.enemy_health[0].health, Some(9));
        assert_eq!(update.frame.combat.damage_dealt[0].amount, Some(31));
        assert_eq!(update.frame.nearby_entities[0].targeting_you, Some(true));
        assert_eq!(update.frame.census.reported_total_objects, Some(100));
        assert_eq!(update.frame.census.listed_objects, 1);
        assert_eq!(update.frame.census.object_list_truncated, Some(true));
        assert_eq!(update.summary.backend_event_count, 4);
        assert_eq!(update.summary.derived_event_count, 2);
        assert!(update.frame.recent_events.iter().any(|event| {
            event.kind == GameEventKind::DamageTaken
                && event.origin == GameEventOrigin::Backend
                && event.amount == Some(7)
        }));
        assert!(update.frame.recent_events.iter().any(|event| {
            event.kind == GameEventKind::DamageDealt
                && event.origin == GameEventOrigin::Backend
                && event.amount == Some(11)
        }));
        assert!(update.frame.recent_events.iter().any(|event| {
            event.kind == GameEventKind::TargetKilled
                && event.entity_id.as_deref() == Some("spider-92")
                && event.amount == Some(31)
                && event.detail.as_deref() == Some("enemy_killed:Spider")
        }));
        let episode = update
            .frame
            .combat
            .episode
            .as_ref()
            .expect("combat episode");
        assert_eq!(episode.kills, 1);
        assert_eq!(episode.damage_received, 7);
        assert_eq!(episode.damage_dealt, 11);
        assert_eq!(episode.current_hostiles, 1);
        assert!(
            !update
                .frame
                .recent_events
                .iter()
                .any(|event| event.amount == Some(99))
        );

        let repeated = engine.update(repeated).expect("repeated frame");
        assert_eq!(repeated.summary.backend_event_count, 0);
        assert_eq!(repeated.summary.derived_event_count, 0);
    }

    #[test]
    fn increments_perception_for_every_snapshot_and_world_only_for_material_change() {
        let mut engine = PerceptionEngine::default();
        let first = engine.update(input(0)).expect("first");
        let second = engine.update(input(1)).expect("second");

        assert_eq!(first.frame.revision, 1);
        assert_eq!(second.frame.revision, 1);
        assert_eq!(first.frame.perception_revision, 1);
        assert_eq!(second.frame.perception_revision, 2);
        assert_eq!(first.observation_cycle_id, Uuid::from_u128(1));
        assert_eq!(first.observation_cycle_sequence, 1);
        assert_eq!(second.observation_cycle_id, Uuid::from_u128(2));
        assert_eq!(second.observation_cycle_sequence, 2);
        assert!(!second.summary.material_change);
    }

    #[test]
    fn derives_damage_enemy_spawn_loot_pickup_and_death_without_advice() {
        let mut engine = PerceptionEngine::default();
        let mut first = input(0);
        first.observation.objects.push(ObservedObject {
            object_id: Some(92),
            object_index: "spider-92".to_owned(),
            label: "Spider".to_owned(),
            kind: "enemy".to_owned(),
            alive: Some(true),
            is_merchant: None,
            interactable: None,
            distance_from_self: Some(4.2),
            tile_x: 8,
            tile_y: 8,
        });
        first.observation.drops.push(ObservedDrop {
            drop_id: "drop-1".to_owned(),
            item_key: Some("silk".to_owned()),
            x: None,
            y: None,
            tile_x: Some(10),
            tile_y: Some(11),
            distance_from_self: Some(1.0),
        });
        engine.update(first).expect("first");

        let mut second = input(1);
        let state = second
            .observation
            .own_player
            .as_mut()
            .and_then(|player| player.state.as_mut())
            .expect("state");
        state.health = Some(0);
        state.alive = Some(false);
        second.inventory = Some(InventoryResult {
            carrying: vec![ObservedItem {
                key: "silk".to_owned(),
                label: "Spider Silk".to_owned(),
                description: None,
                quantity: 1,
                usable: Some(false),
                equipment: Some(false),
                equipped: Some(false),
            }],
        });
        let update = engine.update(second).expect("second");
        let kinds = update
            .frame
            .recent_events
            .iter()
            .map(|event| event.kind)
            .collect::<Vec<_>>();

        assert!(kinds.contains(&GameEventKind::DamageTaken));
        assert!(kinds.contains(&GameEventKind::PlayerDied));
        assert!(kinds.contains(&GameEventKind::EnemyDespawned));
        assert!(kinds.contains(&GameEventKind::LootPickedUp));
        assert!(
            update
                .frame
                .recent_events
                .iter()
                .all(|event| event.origin == GameEventOrigin::Derived)
        );
    }

    #[test]
    fn rejects_non_finite_coordinates() {
        let mut invalid = input(0);
        invalid
            .observation
            .own_player
            .as_mut()
            .and_then(|player| player.state.as_mut())
            .expect("state")
            .x = Some(f32::NAN);

        assert!(matches!(
            PerceptionEngine::default().update(invalid),
            Err(PerceptionError::InvalidCoordinate { field: "x", .. })
        ));
    }

    #[test]
    fn bounds_event_and_action_windows() {
        let mut engine = PerceptionEngine::default();
        for index in 0..300 {
            engine.record_backend_event(GameEvent {
                sequence: Some(index),
                observed_at: time(0),
                origin: GameEventOrigin::Backend,
                kind: GameEventKind::EnemySeen,
                entity_id: Some(format!("enemy-{index}")),
                amount: None,
                tile: None,
                detail: None,
            });
        }
        let update = engine.update(input(1)).expect("update");

        assert!(update.frame.recent_events.len() <= MAX_RECENT_EVENTS);
    }
}
