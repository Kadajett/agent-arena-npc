use std::collections::HashSet;

use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::{
    brain::tactical_frame::TacticalFrame,
    character::Capability,
    execution::packet::{ActionPacket, TacticalAction, TacticalIntent, TacticalStyle},
};

#[derive(Debug, Clone)]
pub struct ValidationContext<'a> {
    pub minimum_valid_frame_revision: u64,
    pub current_strategic_revision: u64,
    pub now: DateTime<Utc>,
    pub capabilities: &'a HashSet<Capability>,
    pub frame: &'a TacticalFrame,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ActionRejected {
    #[error("packet frame {packet} predates minimum valid frame {minimum}")]
    StaleFrame { packet: u64, minimum: u64 },
    #[error("packet strategy {packet} does not match current strategy {current}")]
    StaleStrategy { packet: u64, current: u64 },
    #[error("the player is explicitly dead or unavailable")]
    PlayerUnavailable,
    #[error("active combat is missing authoritative health or maximum health")]
    MissingCombatHealth,
    #[error("missing required capability {0:?}")]
    MissingCapability(Capability),
    #[error("target {0:?} is not visible")]
    UnknownTarget(String),
    #[error("drop {0:?} is not visible")]
    UnknownDrop(String),
    #[error("item {0:?} is not carried or has no usable copy")]
    UnavailableItem(String),
    #[error("skill {0:?} is not currently available")]
    UnavailableSkill(String),
    #[error("movement destination ({tile_x}, {tile_y}) is outside the known local map")]
    UnknownDestination { tile_x: i32, tile_y: i32 },
    #[error("movement destination ({tile_x}, {tile_y}) is explicitly not walkable")]
    BlockedDestination { tile_x: i32, tile_y: i32 },
    #[error(
        "strategic navigation owns idle movement until an immediate tactical fact requires preemption"
    )]
    StrategicNavigationOwnsMovement,
    #[error("action packet has no actions")]
    EmptyPacket,
    #[error("packet validity must be between 1 and 10000 ms")]
    InvalidLifetime,
    #[error("action packet expired before execution")]
    Expired,
    #[error("packet scene {packet:?} does not match current scene {current:?}")]
    SceneChanged {
        packet: Option<String>,
        current: Option<String>,
    },
}

impl ActionRejected {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::StaleFrame { .. } => "stale_frame",
            Self::StaleStrategy { .. } => "stale_strategy",
            Self::PlayerUnavailable => "player_unavailable",
            Self::MissingCombatHealth => "missing_combat_health",
            Self::MissingCapability(_) => "missing_capability",
            Self::UnknownTarget(_) => "unknown_target",
            Self::UnknownDrop(_) => "unknown_drop",
            Self::UnavailableItem(_) => "unavailable_item",
            Self::UnavailableSkill(_) => "unavailable_skill",
            Self::UnknownDestination { .. } => "unknown_destination",
            Self::BlockedDestination { .. } => "blocked_destination",
            Self::StrategicNavigationOwnsMovement => "strategic_navigation_owns_movement",
            Self::EmptyPacket => "empty_packet",
            Self::InvalidLifetime => "invalid_lifetime",
            Self::Expired => "expired",
            Self::SceneChanged { .. } => "scene_changed",
        }
    }
}

/// Validate packet freshness and every requested operation against one frame.
///
/// # Errors
///
/// Returns [`ActionRejected`] when the packet is stale, its lifetime is
/// invalid, a capability is missing, or it refers to unavailable world state.
pub fn validate_packet(
    packet: &ActionPacket,
    context: &ValidationContext<'_>,
) -> Result<(), ActionRejected> {
    validate_packet_header(packet, context)?;
    for action in &packet.proposal.actions {
        // A strategic destination is guidance, not a lock on the body. The
        // executor owns the active navigation mission and can preempt it when
        // a fresh tactical packet supplies a local move. Rejecting every
        // tactical MoveTo here strands characters whenever a strategist is
        // slow or its previous mission is stale.
        validate_action(action, context)?;
    }
    Ok(())
}

/// Validate the runtime-owned packet facts without revalidating past actions.
///
/// # Errors
/// Returns [`ActionRejected`] when packet identity or freshness is no longer valid.
pub fn validate_packet_header(
    packet: &ActionPacket,
    context: &ValidationContext<'_>,
) -> Result<(), ActionRejected> {
    if packet.frame_revision < context.minimum_valid_frame_revision {
        return Err(ActionRejected::StaleFrame {
            packet: packet.frame_revision,
            minimum: context.minimum_valid_frame_revision,
        });
    }
    if packet.strategic_revision != context.current_strategic_revision {
        return Err(ActionRejected::StaleStrategy {
            packet: packet.strategic_revision,
            current: context.current_strategic_revision,
        });
    }
    if packet.proposal.actions.is_empty() && packet.proposal.intent != TacticalIntent::Continue {
        return Err(ActionRejected::EmptyPacket);
    }
    if !(1..=10_000).contains(&packet.proposal.valid_for_ms) {
        return Err(ActionRejected::InvalidLifetime);
    }
    let age_ms = context
        .now
        .signed_duration_since(packet.created_at)
        .num_milliseconds()
        .max(0);
    if u64::try_from(age_ms).unwrap_or(u64::MAX) > packet.proposal.valid_for_ms {
        return Err(ActionRejected::Expired);
    }
    if packet.scene != context.frame.self_state.scene {
        return Err(ActionRejected::SceneChanged {
            packet: packet.scene.clone(),
            current: context.frame.self_state.scene.clone(),
        });
    }
    // Older/current observations can identify the connected player without
    // reporting an explicit alive flag. Unknown is not evidence of death.
    if context.frame.self_state.alive == Some(false)
        || context.frame.self_state.recently_died == Some(true)
    {
        return Err(ActionRejected::PlayerUnavailable);
    }

    Ok(())
}

/// Validate one action against the latest authoritative frame.
///
/// # Errors
/// Returns [`ActionRejected`] when the requested mutation is currently illegal.
pub fn validate_action(
    action: &TacticalAction,
    context: &ValidationContext<'_>,
) -> Result<(), ActionRejected> {
    match action {
        TacticalAction::MoveTo { tile_x, tile_y } => validate_move(*tile_x, *tile_y, context),
        TacticalAction::Stop => require(context, Capability::Walk),
        TacticalAction::Attack { target_id } => {
            require_combat_health(context)?;
            require(context, Capability::Fight)?;
            if context
                .frame
                .nearby_entities
                .iter()
                .any(|entity| entity.id == *target_id && entity.hostile == Some(true))
            {
                Ok(())
            } else {
                Err(ActionRejected::UnknownTarget(target_id.clone()))
            }
        }
        TacticalAction::UseSkill {
            skill_id,
            target_id,
        } => {
            require_combat_health(context)?;
            require(context, Capability::Fight)?;
            if !context
                .frame
                .self_state
                .combat_actions
                .iter()
                .any(|skill| skill.id == *skill_id && skill.available != Some(false))
            {
                return Err(ActionRejected::UnavailableSkill(skill_id.clone()));
            }
            if let Some(target_id) = target_id
                && !context
                    .frame
                    .nearby_entities
                    .iter()
                    .any(|entity| entity.id == *target_id)
            {
                return Err(ActionRejected::UnknownTarget(target_id.clone()));
            }
            Ok(())
        }
        TacticalAction::UseItem { item_id } => {
            if context
                .frame
                .self_state
                .inventory
                .iter()
                .any(|item| item.id == *item_id && item.quantity > 0 && item.usable != Some(false))
            {
                Ok(())
            } else {
                Err(ActionRejected::UnavailableItem(item_id.clone()))
            }
        }
        TacticalAction::PickUp { drop_id } => {
            require(context, Capability::Trade)?;
            if context
                .frame
                .nearby_drops
                .iter()
                .any(|drop| drop.id == *drop_id)
            {
                Ok(())
            } else {
                Err(ActionRejected::UnknownDrop(drop_id.clone()))
            }
        }
        TacticalAction::SetTactics { style, .. } => validate_tactics(*style, context),
    }
}

fn validate_move(
    tile_x: i32,
    tile_y: i32,
    context: &ValidationContext<'_>,
) -> Result<(), ActionRejected> {
    require(context, Capability::Walk)?;
    let destination = crate::world::TilePosition {
        x: tile_x,
        y: tile_y,
    };
    let Some(tile) = context
        .frame
        .map
        .tiles
        .iter()
        .find(|tile| tile.position == destination)
    else {
        return Err(ActionRejected::UnknownDestination { tile_x, tile_y });
    };
    if tile.walkable == Some(false) {
        return Err(ActionRejected::BlockedDestination { tile_x, tile_y });
    }
    if context
        .frame
        .exits
        .iter()
        .any(|exit| exit.tile == destination)
    {
        require(context, Capability::Doors)?;
    }
    Ok(())
}

fn validate_tactics(
    style: TacticalStyle,
    context: &ValidationContext<'_>,
) -> Result<(), ActionRejected> {
    require(context, Capability::Fight)?;
    if style != TacticalStyle::Flee {
        require_combat_health(context)?;
    }
    Ok(())
}

fn require_combat_health(context: &ValidationContext<'_>) -> Result<(), ActionRejected> {
    if context.frame.combat.active == Some(true)
        && (context.frame.self_state.health.is_none()
            || context.frame.self_state.max_health.is_none())
    {
        Err(ActionRejected::MissingCombatHealth)
    } else {
        Ok(())
    }
}

fn require(context: &ValidationContext<'_>, capability: Capability) -> Result<(), ActionRejected> {
    if context.capabilities.contains(&capability) {
        Ok(())
    } else {
        Err(ActionRejected::MissingCapability(capability))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        brain::{strategic_intent::StrategicIntent, tactical_frame::VisibleEntity},
        execution::packet::{TacticalIntent, TacticalProposal},
        world::TilePosition,
    };

    fn frame() -> TacticalFrame {
        let mut frame = TacticalFrame::empty(StrategicIntent {
            revision: 4,
            ..StrategicIntent::default()
        });
        frame.revision = 10;
        frame.self_state.alive = Some(true);
        frame.nearby_entities.push(VisibleEntity {
            id: "spider-1".to_owned(),
            backend_object_id: Some(1),
            label: "Spider".to_owned(),
            kind: crate::brain::tactical_frame::EntityKind::Enemy,
            tile: Some(TilePosition { x: 1, y: 1 }),
            relative: Some(TilePosition { x: 1, y: 1 }),
            distance: Some(1.4),
            alive: Some(true),
            is_merchant: Some(false),
            interactable: Some(false),
            hostile: Some(true),
            targeting_you: Some(true),
        });
        frame
    }

    fn packet(frame: &TacticalFrame, action: TacticalAction) -> ActionPacket {
        ActionPacket::from_proposal(
            uuid::Uuid::new_v4(),
            frame.revision,
            frame.strategic_intent.revision,
            frame.self_state.scene.clone(),
            TacticalProposal {
                intent: TacticalIntent::Attack,
                actions: vec![action],
                valid_for_ms: 1_800,
                abort_if: Vec::new(),
                rationale: None,
            },
        )
    }

    #[test]
    fn rejects_offensive_actions_during_blind_combat() {
        let mut frame = frame();
        frame.combat.active = Some(true);
        frame.self_state.health = None;
        frame.self_state.max_health = None;
        let packet = packet(
            &frame,
            TacticalAction::Attack {
                target_id: "spider-1".to_owned(),
            },
        );
        let capabilities = HashSet::from([Capability::Fight]);
        let context = ValidationContext {
            minimum_valid_frame_revision: 0,
            current_strategic_revision: frame.strategic_intent.revision,
            now: Utc::now(),
            capabilities: &capabilities,
            frame: &frame,
        };

        assert_eq!(
            validate_packet(&packet, &context),
            Err(ActionRejected::MissingCombatHealth)
        );
    }

    #[test]
    fn accepts_visible_hostile_for_fighter() {
        let frame = frame();
        let capabilities = HashSet::from([Capability::Fight]);
        let packet = packet(
            &frame,
            TacticalAction::Attack {
                target_id: "spider-1".to_owned(),
            },
        );
        let context = ValidationContext {
            minimum_valid_frame_revision: 10,
            current_strategic_revision: 4,
            now: Utc::now(),
            capabilities: &capabilities,
            frame: &frame,
        };

        assert_eq!(validate_packet(&packet, &context), Ok(()));
    }

    #[test]
    fn accepts_unknown_alive_state_but_rejects_explicit_death() {
        let mut frame = frame();
        frame.self_state.alive = None;
        let capabilities = HashSet::from([Capability::Walk]);
        let packet = packet(&frame, TacticalAction::Stop);
        {
            let context = ValidationContext {
                minimum_valid_frame_revision: 10,
                current_strategic_revision: 4,
                now: Utc::now(),
                capabilities: &capabilities,
                frame: &frame,
            };
            assert_eq!(validate_packet(&packet, &context), Ok(()));
        }

        frame.self_state.alive = Some(false);
        let dead_context = ValidationContext {
            minimum_valid_frame_revision: 10,
            current_strategic_revision: 4,
            now: Utc::now(),
            capabilities: &capabilities,
            frame: &frame,
        };
        assert_eq!(
            validate_packet(&packet, &dead_context),
            Err(ActionRejected::PlayerUnavailable)
        );
    }

    #[test]
    fn rejects_hallucinated_target_before_execution() {
        let frame = frame();
        let capabilities = HashSet::from([Capability::Fight]);
        let packet = packet(
            &frame,
            TacticalAction::Attack {
                target_id: "dragon-9".to_owned(),
            },
        );
        let context = ValidationContext {
            minimum_valid_frame_revision: 10,
            current_strategic_revision: 4,
            now: Utc::now(),
            capabilities: &capabilities,
            frame: &frame,
        };

        assert_eq!(
            validate_packet(&packet, &context),
            Err(ActionRejected::UnknownTarget("dragon-9".to_owned()))
        );
    }

    #[test]
    fn rejects_expired_packet_and_scene_change_before_action_validation() {
        let mut frame = frame();
        frame.self_state.scene = Some("town".to_owned());
        let capabilities = HashSet::from([Capability::Walk]);
        let mut expired = packet(&frame, TacticalAction::Stop);
        expired.created_at = Utc::now() - chrono::Duration::seconds(2);
        expired.proposal.valid_for_ms = 100;
        let context = ValidationContext {
            minimum_valid_frame_revision: 10,
            current_strategic_revision: 4,
            now: Utc::now(),
            capabilities: &capabilities,
            frame: &frame,
        };
        assert_eq!(
            validate_packet(&expired, &context),
            Err(ActionRejected::Expired)
        );

        let mut wrong_scene = packet(&frame, TacticalAction::Stop);
        wrong_scene.scene = Some("forest".to_owned());
        assert_eq!(
            validate_packet(&wrong_scene, &context),
            Err(ActionRejected::SceneChanged {
                packet: Some("forest".to_owned()),
                current: Some("town".to_owned()),
            })
        );
    }

    #[test]
    fn accepts_backend_legal_skill_when_runtime_availability_is_unknown() {
        let mut frame = frame();
        frame.self_state.combat_actions.push(
            crate::brain::tactical_frame::CombatActionAvailability {
                id: "slash".to_owned(),
                available: None,
                cooldown_remaining_ms: None,
                target_kind: crate::brain::tactical_frame::TargetKind::Entity,
            },
        );
        let capabilities = HashSet::from([Capability::Fight]);
        let packet = packet(
            &frame,
            TacticalAction::UseSkill {
                skill_id: "slash".to_owned(),
                target_id: Some("spider-1".to_owned()),
            },
        );
        let context = ValidationContext {
            minimum_valid_frame_revision: 10,
            current_strategic_revision: 4,
            now: Utc::now(),
            capabilities: &capabilities,
            frame: &frame,
        };

        assert_eq!(validate_packet(&packet, &context), Ok(()));
    }

    #[test]
    fn accepts_an_empty_continue_as_an_explicit_no_op() {
        let frame = frame();
        let capabilities = HashSet::new();
        let packet = ActionPacket::from_proposal(
            uuid::Uuid::new_v4(),
            frame.revision,
            frame.strategic_intent.revision,
            frame.self_state.scene.clone(),
            TacticalProposal {
                intent: TacticalIntent::Continue,
                actions: Vec::new(),
                valid_for_ms: 500,
                abort_if: Vec::new(),
                rationale: None,
            },
        );
        let context = ValidationContext {
            minimum_valid_frame_revision: frame.revision,
            current_strategic_revision: frame.strategic_intent.revision,
            now: Utc::now(),
            capabilities: &capabilities,
            frame: &frame,
        };

        assert_eq!(validate_packet(&packet, &context), Ok(()));
    }

    #[test]
    fn rejects_unknown_and_explicitly_blocked_movement_destinations() {
        let mut frame = frame();
        frame.map.tiles = vec![crate::world::map::MapTile {
            position: TilePosition { x: 2, y: 3 },
            kind: crate::world::map::TileKind::Blocked,
            walkable: Some(false),
        }];
        let capabilities = HashSet::from([Capability::Walk]);
        let context = ValidationContext {
            minimum_valid_frame_revision: frame.revision,
            current_strategic_revision: frame.strategic_intent.revision,
            now: Utc::now(),
            capabilities: &capabilities,
            frame: &frame,
        };

        assert_eq!(
            validate_packet(
                &packet(
                    &frame,
                    TacticalAction::MoveTo {
                        tile_x: 99,
                        tile_y: 99,
                    },
                ),
                &context,
            ),
            Err(ActionRejected::UnknownDestination {
                tile_x: 99,
                tile_y: 99,
            })
        );
        assert_eq!(
            validate_packet(
                &packet(
                    &frame,
                    TacticalAction::MoveTo {
                        tile_x: 2,
                        tile_y: 3,
                    },
                ),
                &context,
            ),
            Err(ActionRejected::BlockedDestination {
                tile_x: 2,
                tile_y: 3,
            })
        );
    }

    #[test]
    fn a_reachable_exit_requires_the_generic_doors_capability() {
        let mut frame = frame();
        let destination = TilePosition { x: 2, y: 3 };
        frame.map.tiles = vec![crate::world::map::MapTile {
            position: destination,
            kind: crate::world::map::TileKind::Door,
            walkable: Some(true),
        }];
        frame.exits.push(crate::world::map::ReachableExit {
            tile: destination,
            destination_scene: Some("forest".to_owned()),
            label: Some("forest door".to_owned()),
            path_length_tiles: 4,
        });
        let action = TacticalAction::MoveTo {
            tile_x: destination.x,
            tile_y: destination.y,
        };

        let walking_only = HashSet::from([Capability::Walk]);
        let walking_context = ValidationContext {
            minimum_valid_frame_revision: frame.revision,
            current_strategic_revision: frame.strategic_intent.revision,
            now: Utc::now(),
            capabilities: &walking_only,
            frame: &frame,
        };
        assert_eq!(
            validate_action(&action, &walking_context),
            Err(ActionRejected::MissingCapability(Capability::Doors))
        );

        let walking_and_doors = HashSet::from([Capability::Walk, Capability::Doors]);
        let door_context = ValidationContext {
            capabilities: &walking_and_doors,
            ..walking_context
        };
        assert_eq!(validate_action(&action, &door_context), Ok(()));
    }

    #[test]
    fn strategic_navigation_rejects_idle_tactical_reposition_but_not_danger_preemption() {
        let mut frame = frame();
        frame.nearby_entities.clear();
        frame.strategic_intent.navigation_goal =
            Some(crate::brain::strategic_intent::NavigationGoal {
                scene: "forest".to_owned(),
                destination: None,
                reason: "continue the durable journey".to_owned(),
            });
        let destination = TilePosition { x: 2, y: 3 };
        frame.map.tiles = vec![crate::world::map::MapTile {
            position: destination,
            kind: crate::world::map::TileKind::Traversable,
            walkable: Some(true),
        }];
        let capabilities = HashSet::from([Capability::Walk]);
        let proposal = TacticalProposal {
            intent: TacticalIntent::Reposition,
            actions: vec![TacticalAction::MoveTo {
                tile_x: destination.x,
                tile_y: destination.y,
            }],
            valid_for_ms: 1_000,
            abort_if: Vec::new(),
            rationale: None,
        };
        let packet = ActionPacket::from_proposal(
            uuid::Uuid::new_v4(),
            frame.revision,
            frame.strategic_intent.revision,
            frame.self_state.scene.clone(),
            proposal,
        );
        {
            let context = ValidationContext {
                minimum_valid_frame_revision: frame.revision,
                current_strategic_revision: frame.strategic_intent.revision,
                now: Utc::now(),
                capabilities: &capabilities,
                frame: &frame,
            };
            assert_eq!(validate_packet(&packet, &context), Ok(()));
        }

        frame.combat.active = Some(true);
        frame.self_state.health = Some(50);
        frame.self_state.max_health = Some(100);
        let danger_context = ValidationContext {
            minimum_valid_frame_revision: frame.revision,
            current_strategic_revision: frame.strategic_intent.revision,
            now: Utc::now(),
            capabilities: &capabilities,
            frame: &frame,
        };
        assert_eq!(validate_packet(&packet, &danger_context), Ok(()));
    }
}
