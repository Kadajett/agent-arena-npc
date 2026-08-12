use crate::{
    brain::{models::ModelUsageTotals, tactical_frame::TacticalFrame},
    config::RuntimeConfig,
    runtime::tactical_schedule::TacticalRolloutMode,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeSafetyStop {
    CombatHealthUnknown,
    ModelCostLimitExceeded,
    ModelCostUnknown,
    PerceptionUpdatesUnavailable,
}

#[must_use]
pub fn evaluate_model_cost_safety(
    limit_usd: Option<f64>,
    totals: &ModelUsageTotals,
) -> Option<RuntimeSafetyStop> {
    let limit = limit_usd?;
    if totals.calls > totals.exact_cost_known_calls {
        Some(RuntimeSafetyStop::ModelCostUnknown)
    } else if totals.exact_cost_usd >= limit {
        Some(RuntimeSafetyStop::ModelCostLimitExceeded)
    } else {
        None
    }
}

impl RuntimeSafetyStop {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::CombatHealthUnknown => "combat_health_unknown",
            Self::ModelCostLimitExceeded => "model_cost_limit_exceeded",
            Self::ModelCostUnknown => "model_cost_unknown",
            Self::PerceptionUpdatesUnavailable => "perception_updates_unavailable",
        }
    }
}

#[must_use]
pub fn evaluate_runtime_safety(
    config: &RuntimeConfig,
    frame: &TacticalFrame,
) -> Option<RuntimeSafetyStop> {
    if config.tactical_rollout_mode == TacticalRolloutMode::Full
        && config.allow_live_mutation
        && matches!(frame.combat.active, Some(true))
        && (frame.self_state.health.is_none() || frame.self_state.max_health.is_none())
    {
        Some(RuntimeSafetyStop::CombatHealthUnknown)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::{HarnessConfig, brain::strategic_intent::StrategicIntent};

    fn full_live_config() -> RuntimeConfig {
        let values = HashMap::from([
            ("ARENA_API_KEY", "arena"),
            ("NPC_TACTICAL_ROLLOUT_MODE", "full"),
            ("NPC_ALLOW_LIVE_MUTATION", "true"),
        ]);
        HarnessConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
            .expect("full live config")
            .runtime
    }

    #[test]
    fn full_live_rollout_stops_when_combat_health_is_unknown() {
        let config = full_live_config();
        let mut frame = TacticalFrame::empty(StrategicIntent::default());
        frame.combat.active = Some(true);
        frame.self_state.health = None;
        frame.self_state.max_health = None;

        assert_eq!(
            evaluate_runtime_safety(&config, &frame),
            Some(RuntimeSafetyStop::CombatHealthUnknown)
        );
    }

    #[test]
    fn shadow_observation_does_not_stop_on_missing_combat_health() {
        let mut config = full_live_config();
        config.tactical_rollout_mode = TacticalRolloutMode::Shadow;
        let mut frame = TacticalFrame::empty(StrategicIntent::default());
        frame.combat.active = Some(true);

        assert_eq!(evaluate_runtime_safety(&config, &frame), None);
    }

    #[test]
    fn full_live_rollout_continues_with_authoritative_combat_health() {
        let config = full_live_config();
        let mut frame = TacticalFrame::empty(StrategicIntent::default());
        frame.combat.active = Some(true);
        frame.self_state.health = Some(35);
        frame.self_state.max_health = Some(100);

        assert_eq!(evaluate_runtime_safety(&config, &frame), None);
    }

    #[test]
    fn configured_cost_ceiling_stops_at_limit() {
        let totals = ModelUsageTotals {
            calls: 2,
            exact_cost_known_calls: 2,
            exact_cost_usd: 0.01,
            ..ModelUsageTotals::default()
        };
        assert_eq!(
            evaluate_model_cost_safety(Some(0.01), &totals),
            Some(RuntimeSafetyStop::ModelCostLimitExceeded)
        );
    }

    #[test]
    fn configured_cost_ceiling_fails_closed_for_unknown_charge() {
        let totals = ModelUsageTotals {
            calls: 2,
            exact_cost_known_calls: 1,
            ..ModelUsageTotals::default()
        };
        assert_eq!(
            evaluate_model_cost_safety(Some(1.0), &totals),
            Some(RuntimeSafetyStop::ModelCostUnknown)
        );
    }

    #[test]
    fn absent_cost_ceiling_does_not_stop_unknown_charge() {
        let totals = ModelUsageTotals {
            calls: 1,
            ..ModelUsageTotals::default()
        };
        assert_eq!(evaluate_model_cost_safety(None, &totals), None);
    }
}
