use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    brain::{strategic_intent::StrategicIntent, tactical_frame::TacticalFrame},
    execution::{
        movement::{MovementState, PathPreflightStatus},
        packet::{AbortCondition, ActionPacket, TacticalAction},
    },
    world::TilePosition,
};

/// Execution observations which do not belong to a tactical frame.
///
/// `None` means that the executor has no authoritative fact. Unknown state
/// never becomes evidence that a path or movement was invalidated.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExecutionValidityFacts {
    pub path_preflight: Option<PathPreflightStatus>,
    pub movement_state: Option<MovementState>,
}

/// Inputs captured when a packet was accepted and when it is reconsidered.
///
/// The health threshold is runtime-owned. Without an explicit threshold, this
/// module will not label any health value as critical.
#[derive(Debug, Clone, Copy)]
pub struct MaterialComparison<'a> {
    pub packet: &'a ActionPacket,
    pub accepted_frame: &'a TacticalFrame,
    pub accepted_intent: &'a StrategicIntent,
    pub current_frame: &'a TacticalFrame,
    pub current_intent: &'a StrategicIntent,
    pub health_critical_at_or_below: Option<i32>,
    pub execution: ExecutionValidityFacts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PathInvalidationEvidence {
    DestinationExplicitlyBlocked,
    PathPreflightUnreachable,
}

/// A material fact observed after a tactical packet was accepted.
///
/// These variants describe state transitions and execution integrity only.
/// They do not recommend a tactical response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MaterialInvalidationFact {
    TargetUnavailable {
        target_id: String,
    },
    TargetDead {
        target_id: String,
    },
    SceneChanged {
        previous_scene: String,
        current_scene: String,
    },
    PlayerDied,
    RequiredItemUnavailable {
        item_id: String,
    },
    RequiredDropUnavailable {
        drop_id: String,
    },
    RequiredSkillUnavailable {
        skill_id: String,
    },
    PathInvalidated {
        destination: Option<TilePosition>,
        evidence: PathInvalidationEvidence,
    },
    MovementInvalidated {
        state: MovementState,
    },
    HealthBecameCritical {
        previous_health: i32,
        current_health: i32,
        threshold: i32,
    },
    NewHostile {
        entity_id: Option<String>,
        previous_count: usize,
        current_count: usize,
    },
    StrategicHardConstraintsChanged {
        constraints_changed: bool,
        avoid_changed: bool,
        previous_constraint_count: usize,
        current_constraint_count: usize,
        previous_avoid_count: usize,
        current_avoid_count: usize,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InvalidationReport {
    pub facts: Vec<MaterialInvalidationFact>,
    pub triggered_abort_conditions: Vec<AbortCondition>,
}

impl InvalidationReport {
    #[must_use]
    pub fn has_material_invalidation(&self) -> bool {
        !self.facts.is_empty()
    }

    #[must_use]
    pub fn abort_condition_triggered(&self, condition: AbortCondition) -> bool {
        self.triggered_abort_conditions.contains(&condition)
    }
}

/// Compare accepted packet assumptions with the latest factual state.
///
/// Revisions, timestamps, recent chat, recent actions, and unrelated tactical
/// frame changes are deliberately ignored. A revision change alone is never a
/// material invalidation.
#[must_use]
pub fn compare_material_state(comparison: &MaterialComparison<'_>) -> InvalidationReport {
    let mut facts = Vec::new();

    record_scene_change(comparison, &mut facts);
    record_player_death(comparison, &mut facts);
    record_required_target_changes(comparison, &mut facts);
    record_required_resource_changes(comparison, &mut facts);
    record_path_and_movement_changes(comparison, &mut facts);
    record_health_change(comparison, &mut facts);
    record_new_hostiles(comparison, &mut facts);
    record_strategic_hard_constraint_change(comparison, &mut facts);

    let triggered_abort_conditions = comparison
        .packet
        .proposal
        .abort_if
        .iter()
        .copied()
        .filter(|condition| abort_condition_matches(*condition, &facts))
        .fold(Vec::new(), |mut unique, condition| {
            if !unique.contains(&condition) {
                unique.push(condition);
            }
            unique
        });

    InvalidationReport {
        facts,
        triggered_abort_conditions,
    }
}

fn record_scene_change(
    comparison: &MaterialComparison<'_>,
    facts: &mut Vec<MaterialInvalidationFact>,
) {
    if let (Some(previous_scene), Some(current_scene)) = (
        comparison.accepted_frame.self_state.scene.as_ref(),
        comparison.current_frame.self_state.scene.as_ref(),
    ) && previous_scene != current_scene
    {
        facts.push(MaterialInvalidationFact::SceneChanged {
            previous_scene: previous_scene.clone(),
            current_scene: current_scene.clone(),
        });
    }
}

fn record_player_death(
    comparison: &MaterialComparison<'_>,
    facts: &mut Vec<MaterialInvalidationFact>,
) {
    let was_dead = comparison.accepted_frame.self_state.alive == Some(false)
        || comparison.accepted_frame.self_state.recently_died == Some(true);
    let is_dead = comparison.current_frame.self_state.alive == Some(false)
        || comparison.current_frame.self_state.recently_died == Some(true);
    if !was_dead && is_dead {
        facts.push(MaterialInvalidationFact::PlayerDied);
    }
}

fn record_required_target_changes(
    comparison: &MaterialComparison<'_>,
    facts: &mut Vec<MaterialInvalidationFact>,
) {
    for target_id in required_targets(comparison.packet) {
        if !entity_is_present(comparison.accepted_frame, &target_id) {
            continue;
        }
        if target_is_dead(comparison.current_frame, &target_id) {
            facts.push(MaterialInvalidationFact::TargetDead { target_id });
        } else if !entity_is_present(comparison.current_frame, &target_id) {
            facts.push(MaterialInvalidationFact::TargetUnavailable { target_id });
        }
    }
}

fn record_required_resource_changes(
    comparison: &MaterialComparison<'_>,
    facts: &mut Vec<MaterialInvalidationFact>,
) {
    for item_id in required_items(comparison.packet) {
        if item_is_available(comparison.accepted_frame, &item_id)
            && !item_is_available(comparison.current_frame, &item_id)
        {
            facts.push(MaterialInvalidationFact::RequiredItemUnavailable { item_id });
        }
    }
    for drop_id in required_drops(comparison.packet) {
        if drop_is_available(comparison.accepted_frame, &drop_id)
            && !drop_is_available(comparison.current_frame, &drop_id)
        {
            facts.push(MaterialInvalidationFact::RequiredDropUnavailable { drop_id });
        }
    }
    for skill_id in required_skills(comparison.packet) {
        if skill_is_available(comparison.accepted_frame, &skill_id)
            && !skill_is_available(comparison.current_frame, &skill_id)
        {
            facts.push(MaterialInvalidationFact::RequiredSkillUnavailable { skill_id });
        }
    }
}

fn record_path_and_movement_changes(
    comparison: &MaterialComparison<'_>,
    facts: &mut Vec<MaterialInvalidationFact>,
) {
    let destinations = required_destinations(comparison.packet);
    for destination in &destinations {
        let explicitly_blocked = comparison
            .current_frame
            .map
            .tiles
            .iter()
            .find(|tile| tile.position == *destination)
            .is_some_and(|tile| tile.walkable == Some(false));
        if explicitly_blocked {
            facts.push(MaterialInvalidationFact::PathInvalidated {
                destination: Some(*destination),
                evidence: PathInvalidationEvidence::DestinationExplicitlyBlocked,
            });
        }
    }

    if !destinations.is_empty()
        && comparison.execution.path_preflight == Some(PathPreflightStatus::Unreachable)
    {
        facts.push(MaterialInvalidationFact::PathInvalidated {
            destination: destinations.first().copied(),
            evidence: PathInvalidationEvidence::PathPreflightUnreachable,
        });
    }

    if !destinations.is_empty()
        && let Some(
            state @ (MovementState::Stalled
            | MovementState::Blocked
            | MovementState::Cancelled
            | MovementState::Interrupted),
        ) = comparison.execution.movement_state
    {
        facts.push(MaterialInvalidationFact::MovementInvalidated { state });
    }
}

fn record_health_change(
    comparison: &MaterialComparison<'_>,
    facts: &mut Vec<MaterialInvalidationFact>,
) {
    let Some(threshold) = comparison.health_critical_at_or_below else {
        return;
    };
    if let (Some(previous_health), Some(current_health)) = (
        comparison.accepted_frame.self_state.health,
        comparison.current_frame.self_state.health,
    ) && previous_health > threshold
        && current_health <= threshold
    {
        facts.push(MaterialInvalidationFact::HealthBecameCritical {
            previous_health,
            current_health,
            threshold,
        });
    }
}

fn record_new_hostiles(
    comparison: &MaterialComparison<'_>,
    facts: &mut Vec<MaterialInvalidationFact>,
) {
    let previous = visible_hostiles(comparison.accepted_frame);
    let current = visible_hostiles(comparison.current_frame);
    let previous_count = effective_hostile_count(comparison.accepted_frame, previous.len());
    let current_count = effective_hostile_count(comparison.current_frame, current.len());

    let mut found_identified_hostile = false;
    for entity_id in current.difference(&previous) {
        found_identified_hostile = true;
        facts.push(MaterialInvalidationFact::NewHostile {
            entity_id: Some((*entity_id).clone()),
            previous_count,
            current_count,
        });
    }
    if !found_identified_hostile && current_count > previous_count {
        facts.push(MaterialInvalidationFact::NewHostile {
            entity_id: None,
            previous_count,
            current_count,
        });
    }
}

fn record_strategic_hard_constraint_change(
    comparison: &MaterialComparison<'_>,
    facts: &mut Vec<MaterialInvalidationFact>,
) {
    let previous_constraints = normalized_set(&comparison.accepted_intent.constraints);
    let current_constraints = normalized_set(&comparison.current_intent.constraints);
    let previous_avoid = normalized_set(&comparison.accepted_intent.avoid);
    let current_avoid = normalized_set(&comparison.current_intent.avoid);
    let constraints_changed = previous_constraints != current_constraints;
    let avoid_changed = previous_avoid != current_avoid;
    if constraints_changed || avoid_changed {
        facts.push(MaterialInvalidationFact::StrategicHardConstraintsChanged {
            constraints_changed,
            avoid_changed,
            previous_constraint_count: previous_constraints.len(),
            current_constraint_count: current_constraints.len(),
            previous_avoid_count: previous_avoid.len(),
            current_avoid_count: current_avoid.len(),
        });
    }
}

fn abort_condition_matches(condition: AbortCondition, facts: &[MaterialInvalidationFact]) -> bool {
    facts.iter().any(|fact| {
        matches!(
            (condition, fact),
            (
                AbortCondition::HealthCritical,
                MaterialInvalidationFact::HealthBecameCritical { .. }
            ) | (
                AbortCondition::PathBlocked,
                MaterialInvalidationFact::PathInvalidated { .. }
                    | MaterialInvalidationFact::MovementInvalidated { .. }
            ) | (
                AbortCondition::NewHostile,
                MaterialInvalidationFact::NewHostile { .. }
            ) | (
                AbortCondition::StrategicIntentChanged,
                MaterialInvalidationFact::StrategicHardConstraintsChanged { .. }
            ) | (
                AbortCondition::TargetUnavailable,
                MaterialInvalidationFact::TargetUnavailable { .. }
                    | MaterialInvalidationFact::TargetDead { .. }
            ) | (
                AbortCondition::SceneChanged,
                MaterialInvalidationFact::SceneChanged { .. }
            ) | (
                AbortCondition::PlayerDied,
                MaterialInvalidationFact::PlayerDied
            )
        )
    })
}

fn required_targets(packet: &ActionPacket) -> BTreeSet<String> {
    packet
        .proposal
        .actions
        .iter()
        .filter_map(|action| match action {
            TacticalAction::Attack { target_id }
            | TacticalAction::UseSkill {
                target_id: Some(target_id),
                ..
            } => Some(target_id.clone()),
            _ => None,
        })
        .collect()
}

fn required_items(packet: &ActionPacket) -> BTreeSet<String> {
    packet
        .proposal
        .actions
        .iter()
        .filter_map(|action| match action {
            TacticalAction::UseItem { item_id } => Some(item_id.clone()),
            _ => None,
        })
        .collect()
}

fn required_drops(packet: &ActionPacket) -> BTreeSet<String> {
    packet
        .proposal
        .actions
        .iter()
        .filter_map(|action| match action {
            TacticalAction::PickUp { drop_id } => Some(drop_id.clone()),
            _ => None,
        })
        .collect()
}

fn required_skills(packet: &ActionPacket) -> BTreeSet<String> {
    packet
        .proposal
        .actions
        .iter()
        .filter_map(|action| match action {
            TacticalAction::UseSkill { skill_id, .. } => Some(skill_id.clone()),
            _ => None,
        })
        .collect()
}

fn required_destinations(packet: &ActionPacket) -> Vec<TilePosition> {
    packet
        .proposal
        .actions
        .iter()
        .filter_map(TacticalAction::destination)
        .collect()
}

fn entity_is_present(frame: &TacticalFrame, target_id: &str) -> bool {
    frame
        .nearby_entities
        .iter()
        .any(|entity| entity.id == target_id)
}

fn target_is_dead(frame: &TacticalFrame, target_id: &str) -> bool {
    frame
        .nearby_entities
        .iter()
        .any(|entity| entity.id == target_id && entity.alive == Some(false))
        || frame
            .combat
            .enemy_health
            .iter()
            .any(|enemy| enemy.id == target_id && enemy.health.is_some_and(|health| health <= 0))
}

fn item_is_available(frame: &TacticalFrame, item_id: &str) -> bool {
    frame
        .self_state
        .inventory
        .iter()
        .any(|item| item.id == item_id && item.quantity > 0 && item.usable != Some(false))
}

fn drop_is_available(frame: &TacticalFrame, drop_id: &str) -> bool {
    frame.nearby_drops.iter().any(|drop| drop.id == drop_id)
}

fn skill_is_available(frame: &TacticalFrame, skill_id: &str) -> bool {
    frame
        .self_state
        .combat_actions
        .iter()
        .any(|skill| skill.id == skill_id && skill.available != Some(false))
}

fn visible_hostiles(frame: &TacticalFrame) -> BTreeSet<String> {
    frame
        .nearby_entities
        .iter()
        .filter(|entity| entity.hostile == Some(true) && entity.alive != Some(false))
        .map(|entity| entity.id.clone())
        .collect()
}

fn effective_hostile_count(frame: &TacticalFrame, visible_count: usize) -> usize {
    frame.combat.current_hostiles.max(visible_count)
}

fn normalized_set(values: &[String]) -> BTreeSet<String> {
    values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use uuid::Uuid;

    use super::*;
    use crate::{
        brain::{
            strategic_intent::StrategicIntent,
            tactical_frame::{
                CarriedItem, CombatActionAvailability, Drop, EntityKind, TargetKind, VisibleEntity,
            },
        },
        execution::packet::{TacticalIntent, TacticalProposal},
        world::map::{MapTile, TileKind},
    };

    fn hostile(id: &str, alive: Option<bool>) -> VisibleEntity {
        VisibleEntity {
            id: id.to_owned(),
            backend_object_id: None,
            label: "enemy".to_owned(),
            kind: EntityKind::Enemy,
            tile: None,
            relative: None,
            distance: None,
            alive,
            is_merchant: None,
            interactable: Some(false),
            hostile: Some(true),
            targeting_you: None,
        }
    }

    fn frame(intent: StrategicIntent) -> TacticalFrame {
        let mut frame = TacticalFrame::empty(intent);
        frame.revision = 10;
        frame.perception_revision = 10;
        frame.self_state.scene = Some("town".to_owned());
        frame.self_state.alive = Some(true);
        frame.self_state.health = Some(100);
        frame.self_state.max_health = Some(100);
        frame.nearby_entities.push(hostile("spider-1", Some(true)));
        frame.combat.current_hostiles = 1;
        frame.self_state.inventory.push(CarriedItem {
            id: "potion".to_owned(),
            label: "Potion".to_owned(),
            quantity: 1,
            usable: Some(true),
            equipment: Some(false),
            equipped: Some(false),
        });
        frame
            .self_state
            .combat_actions
            .push(CombatActionAvailability {
                id: "slash".to_owned(),
                available: Some(true),
                cooldown_remaining_ms: Some(0),
                target_kind: TargetKind::Entity,
            });
        frame.nearby_drops.push(Drop {
            id: "silk-drop".to_owned(),
            item_id: Some("silk".to_owned()),
            label: Some("Silk".to_owned()),
            tile: None,
            relative: None,
            distance: None,
        });
        frame
    }

    fn packet(frame: &TacticalFrame, actions: Vec<TacticalAction>) -> ActionPacket {
        ActionPacket::from_proposal(
            Uuid::new_v4(),
            frame.revision,
            frame.strategic_intent.revision,
            frame.self_state.scene.clone(),
            TacticalProposal {
                intent: TacticalIntent::Continue,
                actions,
                valid_for_ms: 1_800,
                abort_if: vec![
                    AbortCondition::HealthCritical,
                    AbortCondition::PathBlocked,
                    AbortCondition::NewHostile,
                    AbortCondition::StrategicIntentChanged,
                    AbortCondition::TargetUnavailable,
                    AbortCondition::SceneChanged,
                    AbortCondition::PlayerDied,
                ],
                rationale: None,
            },
        )
    }

    fn compare<'a>(
        packet: &'a ActionPacket,
        accepted: &'a TacticalFrame,
        current: &'a TacticalFrame,
        accepted_intent: &'a StrategicIntent,
        current_intent: &'a StrategicIntent,
    ) -> MaterialComparison<'a> {
        MaterialComparison {
            packet,
            accepted_frame: accepted,
            accepted_intent,
            current_frame: current,
            current_intent,
            health_critical_at_or_below: None,
            execution: ExecutionValidityFacts::default(),
        }
    }

    #[test]
    fn unrelated_revision_and_non_constraint_strategy_changes_do_not_invalidate() {
        let accepted_intent = StrategicIntent::default();
        let mut current_intent = accepted_intent.clone();
        current_intent.revision = 99;
        current_intent.objective = "A different objective".to_owned();
        current_intent.risk_tolerance = 0.9;
        let accepted = frame(accepted_intent.clone());
        let mut current = accepted.clone();
        current.revision = 99;
        current.perception_revision = 500;
        current.generated_at = Utc::now();
        current.strategic_intent = current_intent.clone();
        let current_packet = packet(&accepted, vec![TacticalAction::Stop]);

        let report = compare_material_state(&compare(
            &current_packet,
            &accepted,
            &current,
            &accepted_intent,
            &current_intent,
        ));

        assert_eq!(report, InvalidationReport::default());
    }

    #[test]
    fn distinguishes_required_target_death_from_disappearance() {
        let intent = StrategicIntent::default();
        let accepted = frame(intent.clone());
        let packet = packet(
            &accepted,
            vec![TacticalAction::Attack {
                target_id: "spider-1".to_owned(),
            }],
        );

        let mut dead = accepted.clone();
        dead.nearby_entities[0].alive = Some(false);
        let dead_report =
            compare_material_state(&compare(&packet, &accepted, &dead, &intent, &intent));
        assert!(
            dead_report
                .facts
                .contains(&MaterialInvalidationFact::TargetDead {
                    target_id: "spider-1".to_owned(),
                })
        );
        assert!(dead_report.abort_condition_triggered(AbortCondition::TargetUnavailable));

        let mut missing = accepted.clone();
        missing.nearby_entities.clear();
        missing.combat.current_hostiles = 0;
        let missing_report =
            compare_material_state(&compare(&packet, &accepted, &missing, &intent, &intent));
        assert!(
            missing_report
                .facts
                .contains(&MaterialInvalidationFact::TargetUnavailable {
                    target_id: "spider-1".to_owned(),
                })
        );
    }

    #[test]
    fn reports_only_required_item_drop_and_skill_losses() {
        let intent = StrategicIntent::default();
        let accepted = frame(intent.clone());
        let packet = packet(
            &accepted,
            vec![
                TacticalAction::UseItem {
                    item_id: "potion".to_owned(),
                },
                TacticalAction::PickUp {
                    drop_id: "silk-drop".to_owned(),
                },
                TacticalAction::UseSkill {
                    skill_id: "slash".to_owned(),
                    target_id: None,
                },
            ],
        );
        let mut current = accepted.clone();
        current.self_state.inventory[0].quantity = 0;
        current.nearby_drops.clear();
        current.self_state.combat_actions[0].available = Some(false);

        let report =
            compare_material_state(&compare(&packet, &accepted, &current, &intent, &intent));

        assert!(
            report
                .facts
                .contains(&MaterialInvalidationFact::RequiredItemUnavailable {
                    item_id: "potion".to_owned(),
                })
        );
        assert!(
            report
                .facts
                .contains(&MaterialInvalidationFact::RequiredDropUnavailable {
                    drop_id: "silk-drop".to_owned(),
                })
        );
        assert!(
            report
                .facts
                .contains(&MaterialInvalidationFact::RequiredSkillUnavailable {
                    skill_id: "slash".to_owned(),
                })
        );
        assert!(report.triggered_abort_conditions.is_empty());
    }

    #[test]
    fn reports_scene_change_and_new_player_death_but_not_unknown_scene() {
        let intent = StrategicIntent::default();
        let accepted = frame(intent.clone());
        let scene_packet = packet(&accepted, vec![TacticalAction::Stop]);
        let mut current = accepted.clone();
        current.self_state.scene = Some("forest".to_owned());
        current.self_state.alive = Some(false);

        let report = compare_material_state(&compare(
            &scene_packet,
            &accepted,
            &current,
            &intent,
            &intent,
        ));
        assert!(
            report
                .facts
                .contains(&MaterialInvalidationFact::SceneChanged {
                    previous_scene: "town".to_owned(),
                    current_scene: "forest".to_owned(),
                })
        );
        assert!(report.facts.contains(&MaterialInvalidationFact::PlayerDied));

        current.self_state.scene = None;
        let unknown_report = compare_material_state(&compare(
            &scene_packet,
            &accepted,
            &current,
            &intent,
            &intent,
        ));
        assert!(
            !unknown_report
                .facts
                .iter()
                .any(|fact| matches!(fact, MaterialInvalidationFact::SceneChanged { .. }))
        );
    }

    #[test]
    fn health_requires_an_explicit_threshold_and_a_crossing() {
        let intent = StrategicIntent::default();
        let mut accepted = frame(intent.clone());
        accepted.self_state.health = Some(60);
        let health_packet = packet(&accepted, vec![TacticalAction::Stop]);
        let mut current = accepted.clone();
        current.self_state.health = Some(20);

        let without_threshold = compare_material_state(&compare(
            &health_packet,
            &accepted,
            &current,
            &intent,
            &intent,
        ));
        assert!(
            !without_threshold
                .facts
                .iter()
                .any(|fact| matches!(fact, MaterialInvalidationFact::HealthBecameCritical { .. }))
        );

        let mut comparison = compare(&health_packet, &accepted, &current, &intent, &intent);
        comparison.health_critical_at_or_below = Some(25);
        let crossed = compare_material_state(&comparison);
        assert!(
            crossed
                .facts
                .contains(&MaterialInvalidationFact::HealthBecameCritical {
                    previous_health: 60,
                    current_health: 20,
                    threshold: 25,
                })
        );
        assert!(crossed.abort_condition_triggered(AbortCondition::HealthCritical));

        let mut already_critical = accepted.clone();
        already_critical.self_state.health = Some(25);
        let mut still_critical = current.clone();
        still_critical.self_state.health = Some(10);
        let already_critical_packet = packet(&already_critical, vec![TacticalAction::Stop]);
        let mut no_crossing = compare(
            &already_critical_packet,
            &already_critical,
            &still_critical,
            &intent,
            &intent,
        );
        no_crossing.health_critical_at_or_below = Some(25);
        assert!(
            !compare_material_state(&no_crossing)
                .facts
                .iter()
                .any(|fact| matches!(fact, MaterialInvalidationFact::HealthBecameCritical { .. }))
        );
    }

    #[test]
    fn new_hostile_and_hard_constraint_changes_trigger_only_their_conditions() {
        let accepted_intent = StrategicIntent {
            constraints: vec!["Keep one potion".to_owned()],
            avoid: vec!["lava".to_owned()],
            ..StrategicIntent::default()
        };
        let mut current_intent = accepted_intent.clone();
        current_intent.constraints = vec![
            "Do not enter caves".to_owned(),
            "Keep one potion".to_owned(),
        ];
        current_intent.avoid = vec!["lava".to_owned()];
        let accepted = frame(accepted_intent.clone());
        let packet = packet(&accepted, vec![TacticalAction::Stop]);
        let mut current = accepted.clone();
        current
            .nearby_entities
            .push(hostile("spider-2", Some(true)));
        current.combat.current_hostiles = 2;

        let report = compare_material_state(&compare(
            &packet,
            &accepted,
            &current,
            &accepted_intent,
            &current_intent,
        ));

        assert!(report.facts.iter().any(|fact| matches!(
            fact,
            MaterialInvalidationFact::NewHostile {
                entity_id: Some(id),
                ..
            } if id == "spider-2"
        )));
        assert!(report.facts.iter().any(|fact| matches!(
            fact,
            MaterialInvalidationFact::StrategicHardConstraintsChanged {
                constraints_changed: true,
                avoid_changed: false,
                ..
            }
        )));
        assert!(report.abort_condition_triggered(AbortCondition::NewHostile));
        assert!(report.abort_condition_triggered(AbortCondition::StrategicIntentChanged));
        assert!(!report.abort_condition_triggered(AbortCondition::TargetUnavailable));
    }

    #[test]
    fn path_and_movement_require_explicit_factual_evidence() {
        let intent = StrategicIntent::default();
        let accepted = frame(intent.clone());
        let packet = packet(
            &accepted,
            vec![TacticalAction::MoveTo {
                tile_x: 4,
                tile_y: 8,
            }],
        );
        let mut current = accepted.clone();
        current.map.tiles.push(MapTile {
            position: TilePosition { x: 4, y: 8 },
            kind: TileKind::Blocked,
            walkable: Some(false),
        });
        let mut comparison = compare(&packet, &accepted, &current, &intent, &intent);
        comparison.execution = ExecutionValidityFacts {
            path_preflight: Some(PathPreflightStatus::Unreachable),
            movement_state: Some(MovementState::Stalled),
        };

        let report = compare_material_state(&comparison);

        assert!(
            report
                .facts
                .contains(&MaterialInvalidationFact::PathInvalidated {
                    destination: Some(TilePosition { x: 4, y: 8 }),
                    evidence: PathInvalidationEvidence::DestinationExplicitlyBlocked,
                })
        );
        assert!(
            report
                .facts
                .contains(&MaterialInvalidationFact::PathInvalidated {
                    destination: Some(TilePosition { x: 4, y: 8 }),
                    evidence: PathInvalidationEvidence::PathPreflightUnreachable,
                })
        );
        assert!(
            report
                .facts
                .contains(&MaterialInvalidationFact::MovementInvalidated {
                    state: MovementState::Stalled,
                })
        );
        assert!(report.abort_condition_triggered(AbortCondition::PathBlocked));
    }

    #[test]
    fn reordered_constraints_and_unknown_execution_state_are_not_material() {
        let accepted_intent = StrategicIntent {
            constraints: vec!["alpha".to_owned(), "beta".to_owned()],
            ..StrategicIntent::default()
        };
        let mut current_intent = accepted_intent.clone();
        current_intent.constraints.reverse();
        let accepted = frame(accepted_intent.clone());
        let current = accepted.clone();
        let packet = packet(&accepted, vec![TacticalAction::Stop]);
        let mut comparison = compare(
            &packet,
            &accepted,
            &current,
            &accepted_intent,
            &current_intent,
        );
        comparison.execution = ExecutionValidityFacts {
            path_preflight: Some(PathPreflightStatus::Unknown),
            movement_state: Some(MovementState::Moving),
        };

        assert_eq!(
            compare_material_state(&comparison),
            InvalidationReport::default()
        );
    }
}
