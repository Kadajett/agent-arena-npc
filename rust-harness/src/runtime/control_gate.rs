use std::time::Duration;

use thiserror::Error;

use crate::{
    character::CharacterSheet,
    config::{LiveActionBudget, RuntimeConfig},
    execution::packet::{ActionPacket, TacticalProposal},
    execution::validator::ActionRejected,
    runtime::tactical_schedule::TacticalRolloutMode,
};

/// Exact assertions supplied for one diagnostic against one connected body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlledPacketRequest {
    pub expected_character_id: String,
    pub expected_player_name: String,
    pub expected_scene: String,
    pub proposal: TacticalProposal,
}

/// Causal receipt returned by validation and controlled submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlledPacketReceipt {
    pub packet_id: uuid::Uuid,
    pub decision_id: uuid::Uuid,
    pub frame_revision: u64,
    pub strategic_revision: u64,
    pub action_count: usize,
    pub remaining_live_action_budget: Option<u32>,
    pub live_action_budget_unlimited: bool,
    pub released: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ControlledPacketError {
    #[error("runtime character assertion failed: expected {expected:?}, connected {actual:?}")]
    CharacterMismatch { expected: String, actual: String },
    #[error("runtime player-name assertion failed: expected {expected:?}, connected {actual:?}")]
    PlayerNameMismatch { expected: String, actual: String },
    #[error("runtime scene assertion failed: expected {expected:?}, observed {actual:?}")]
    SceneMismatch {
        expected: String,
        actual: Option<String>,
    },
    #[error("controlled release requires NPC_TACTICAL_ROLLOUT_MODE=controlled")]
    RolloutNotControlled,
    #[error("automatic model release requires NPC_TACTICAL_ROLLOUT_MODE=full")]
    RolloutNotFull,
    #[error("controlled release requires NPC_ALLOW_LIVE_MUTATION=true")]
    MutationNotAuthorized,
    #[error("{name} must be configured for a controlled release")]
    MissingConfiguredAssertion { name: &'static str },
    #[error("configured {field} {configured:?} does not match asserted {asserted:?}")]
    ConfiguredAssertionMismatch {
        field: &'static str,
        configured: String,
        asserted: String,
    },
    #[error("packet contains {actual} actions; configured maximum is {maximum}")]
    TooManyActions { actual: usize, maximum: u32 },
    #[error("controlled release requires at least one action")]
    NoActions,
    #[error("controlled release gate is disabled because the live action budget is zero")]
    GateDisabled,
    #[error("packet lifetime {actual_ms} ms exceeds configured maximum {maximum_ms} ms")]
    LifetimeTooLong { actual_ms: u64, maximum_ms: u64 },
    #[error("live action budget exhausted: requested {requested}, remaining {remaining}")]
    BudgetExhausted { requested: u32, remaining: u32 },
    #[error("BodyActor rejected packet: {0}")]
    BodyRejected(ActionRejected),
    #[error("BodyActor validation mailbox unavailable")]
    BodyUnavailable,
    #[error("player supervisor mailbox unavailable")]
    SupervisorUnavailable,
}

impl ControlledPacketError {
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::CharacterMismatch { .. } => "character_mismatch",
            Self::PlayerNameMismatch { .. } => "player_name_mismatch",
            Self::SceneMismatch { .. } => "scene_mismatch",
            Self::RolloutNotControlled => "rollout_not_controlled",
            Self::RolloutNotFull => "rollout_not_full",
            Self::MutationNotAuthorized => "mutation_not_authorized",
            Self::MissingConfiguredAssertion { .. } => "configured_assertion_missing",
            Self::ConfiguredAssertionMismatch { .. } => "configured_assertion_mismatch",
            Self::TooManyActions { .. } => "too_many_actions",
            Self::NoActions => "no_actions",
            Self::GateDisabled => "gate_disabled",
            Self::LifetimeTooLong { .. } => "lifetime_too_long",
            Self::BudgetExhausted { .. } => "budget_exhausted",
            Self::BodyRejected(_) => "body_rejected",
            Self::BodyUnavailable => "body_unavailable",
            Self::SupervisorUnavailable => "supervisor_unavailable",
        }
    }

    #[must_use]
    pub const fn body_rejection_code(&self) -> Option<&'static str> {
        match self {
            Self::BodyRejected(reason) => Some(reason.reason_code()),
            _ => None,
        }
    }
}

/// One-way mutation authorization state, owned by a single player supervisor.
pub(super) struct LiveMutationGate {
    budget: LiveActionBudget,
}

impl LiveMutationGate {
    pub(super) const fn new(budget: LiveActionBudget) -> Self {
        Self { budget }
    }

    pub(super) const fn remaining_actions(&self) -> Option<u32> {
        match self.budget {
            LiveActionBudget::Limited(remaining) => Some(remaining),
            LiveActionBudget::Unlimited => None,
        }
    }

    pub(super) const fn is_unlimited(&self) -> bool {
        matches!(self.budget, LiveActionBudget::Unlimited)
    }

    /// Check immutable release policy before asking the body to validate.
    ///
    /// # Errors
    ///
    /// Returns [`ControlledPacketError`] unless rollout, mutation, configured
    /// assertions, packet limits, and remaining budget all authorize release.
    pub(super) fn authorize(
        &self,
        config: &RuntimeConfig,
        character: &CharacterSheet,
        observed_scene: Option<&str>,
        request: &ControlledPacketRequest,
    ) -> Result<(), ControlledPacketError> {
        assert_runtime(character, observed_scene, request)?;
        if config.tactical_rollout_mode != TacticalRolloutMode::Controlled {
            return Err(ControlledPacketError::RolloutNotControlled);
        }
        if !config.allow_live_mutation {
            return Err(ControlledPacketError::MutationNotAuthorized);
        }
        assert_configured(
            "NPC_LIVE_EXPECTED_CHARACTER_ID",
            config.live_expected_character_id.as_deref(),
            &request.expected_character_id,
        )?;
        assert_configured(
            "NPC_LIVE_EXPECTED_PLAYER_NAME",
            config.live_expected_player_name.as_deref(),
            &request.expected_player_name,
        )?;
        assert_scene_configured(
            config.live_allowed_scene.as_deref(),
            &request.expected_scene,
        )?;
        if self.budget == LiveActionBudget::Limited(0) {
            return Err(ControlledPacketError::GateDisabled);
        }
        if request.proposal.actions.is_empty() {
            return Err(ControlledPacketError::NoActions);
        }
        assert_packet_limits(config, &request.proposal)?;
        let requested = u32::try_from(request.proposal.actions.len()).unwrap_or(u32::MAX);
        if let LiveActionBudget::Limited(remaining) = self.budget
            && requested > remaining
        {
            return Err(ControlledPacketError::BudgetExhausted {
                requested,
                remaining,
            });
        }
        Ok(())
    }

    /// Authorize one model-created packet through the same production limits.
    ///
    /// # Errors
    ///
    /// Returns [`ControlledPacketError`] unless full rollout, mutation,
    /// configured identity and scene assertions, packet limits, and remaining
    /// budget all authorize release.
    pub(super) fn authorize_model_packet(
        &self,
        config: &RuntimeConfig,
        character: &CharacterSheet,
        observed_scene: Option<&str>,
        proposal: &TacticalProposal,
    ) -> Result<(), ControlledPacketError> {
        if config.tactical_rollout_mode != TacticalRolloutMode::Full {
            return Err(ControlledPacketError::RolloutNotFull);
        }
        if !config.allow_live_mutation {
            return Err(ControlledPacketError::MutationNotAuthorized);
        }
        assert_configured(
            "NPC_LIVE_EXPECTED_CHARACTER_ID",
            config.live_expected_character_id.as_deref(),
            &character.id,
        )?;
        assert_configured(
            "NPC_LIVE_EXPECTED_PLAYER_NAME",
            config.live_expected_player_name.as_deref(),
            &character.player_name,
        )?;
        let expected_scene = config.live_allowed_scene.as_deref().ok_or(
            ControlledPacketError::MissingConfiguredAssertion {
                name: "NPC_LIVE_ALLOWED_SCENE",
            },
        )?;
        if observed_scene.is_none()
            || (expected_scene != "*" && Some(expected_scene) != observed_scene)
        {
            return Err(ControlledPacketError::SceneMismatch {
                expected: expected_scene.to_owned(),
                actual: observed_scene.map(ToOwned::to_owned),
            });
        }
        if self.budget == LiveActionBudget::Limited(0) {
            return Err(ControlledPacketError::GateDisabled);
        }
        if proposal.actions.is_empty() {
            return Err(ControlledPacketError::NoActions);
        }
        assert_packet_limits(config, proposal)?;
        let requested = u32::try_from(proposal.actions.len()).unwrap_or(u32::MAX);
        if let LiveActionBudget::Limited(remaining) = self.budget
            && requested > remaining
        {
            return Err(ControlledPacketError::BudgetExhausted {
                requested,
                remaining,
            });
        }
        Ok(())
    }

    /// Consume budget only after the real body validator accepts the packet.
    ///
    /// # Errors
    ///
    /// Returns [`ControlledPacketError::BudgetExhausted`] if the requested
    /// action count exceeds the remaining process-local budget.
    pub(super) fn consume(
        &mut self,
        action_count: usize,
    ) -> Result<Option<u32>, ControlledPacketError> {
        let requested = u32::try_from(action_count).unwrap_or(u32::MAX);
        match self.budget {
            LiveActionBudget::Unlimited => Ok(None),
            LiveActionBudget::Limited(remaining) => {
                let remaining = remaining.checked_sub(requested).ok_or(
                    ControlledPacketError::BudgetExhausted {
                        requested,
                        remaining,
                    },
                )?;
                self.budget = LiveActionBudget::Limited(remaining);
                Ok(Some(remaining))
            }
        }
    }
}

/// Match caller-supplied identity and scene assertions to current runtime facts.
///
/// # Errors
///
/// Returns [`ControlledPacketError`] for any character, player-name, or scene
/// mismatch, including an unknown observed scene.
pub(super) fn assert_runtime(
    character: &CharacterSheet,
    observed_scene: Option<&str>,
    request: &ControlledPacketRequest,
) -> Result<(), ControlledPacketError> {
    if request.expected_character_id != character.id {
        return Err(ControlledPacketError::CharacterMismatch {
            expected: request.expected_character_id.clone(),
            actual: character.id.clone(),
        });
    }
    if request.expected_player_name != character.player_name {
        return Err(ControlledPacketError::PlayerNameMismatch {
            expected: request.expected_player_name.clone(),
            actual: character.player_name.clone(),
        });
    }
    if Some(request.expected_scene.as_str()) != observed_scene {
        return Err(ControlledPacketError::SceneMismatch {
            expected: request.expected_scene.clone(),
            actual: observed_scene.map(ToOwned::to_owned),
        });
    }
    Ok(())
}

/// Check per-packet action and lifetime ceilings without authorizing mutation.
///
/// # Errors
///
/// Returns [`ControlledPacketError`] when the proposal exceeds either ceiling.
pub(super) fn assert_packet_limits(
    config: &RuntimeConfig,
    proposal: &TacticalProposal,
) -> Result<(), ControlledPacketError> {
    let actual = proposal.actions.len();
    if actual > config.live_max_actions_per_packet as usize {
        return Err(ControlledPacketError::TooManyActions {
            actual,
            maximum: config.live_max_actions_per_packet,
        });
    }
    let maximum_ms = duration_ms(config.live_packet_max_age);
    if proposal.valid_for_ms > maximum_ms {
        return Err(ControlledPacketError::LifetimeTooLong {
            actual_ms: proposal.valid_for_ms,
            maximum_ms,
        });
    }
    Ok(())
}

pub(super) fn runtime_packet(
    frame_revision: u64,
    strategic_revision: u64,
    scene: Option<String>,
    proposal: TacticalProposal,
) -> ActionPacket {
    ActionPacket::from_proposal(
        uuid::Uuid::new_v4(),
        frame_revision,
        strategic_revision,
        scene,
        proposal,
    )
}

fn assert_configured(
    name: &'static str,
    configured: Option<&str>,
    asserted: &str,
) -> Result<(), ControlledPacketError> {
    let configured =
        configured.ok_or(ControlledPacketError::MissingConfiguredAssertion { name })?;
    if configured != asserted {
        return Err(ControlledPacketError::ConfiguredAssertionMismatch {
            field: name,
            configured: configured.to_owned(),
            asserted: asserted.to_owned(),
        });
    }
    Ok(())
}

fn assert_scene_configured(
    configured: Option<&str>,
    asserted: &str,
) -> Result<(), ControlledPacketError> {
    let configured = configured.ok_or(ControlledPacketError::MissingConfiguredAssertion {
        name: "NPC_LIVE_ALLOWED_SCENE",
    })?;
    if configured == "*" || configured == asserted {
        return Ok(());
    }
    Err(ControlledPacketError::ConfiguredAssertionMismatch {
        field: "NPC_LIVE_ALLOWED_SCENE",
        configured: configured.to_owned(),
        asserted: asserted.to_owned(),
    })
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::{
        config::HarnessConfig,
        execution::packet::{TacticalAction, TacticalIntent},
    };

    use super::*;

    fn fixture() -> (HarnessConfig, CharacterSheet, ControlledPacketRequest) {
        let values = HashMap::from([
            ("ARENA_API_KEY", "arena"),
            ("ARENA_PLAYER_NAME", "Ignored by CharacterSheet env lookup"),
            ("NPC_TACTICAL_ROLLOUT_MODE", "controlled"),
            ("NPC_ALLOW_LIVE_MUTATION", "true"),
            ("NPC_LIVE_ACTION_BUDGET", "1"),
            ("NPC_LIVE_EXPECTED_CHARACTER_ID", "guy"),
            ("NPC_LIVE_EXPECTED_PLAYER_NAME", "Guy"),
            ("NPC_LIVE_ALLOWED_SCENE", "arena"),
            (
                "NPC_CHARACTER_SHEET_PATH",
                concat!(env!("CARGO_MANIFEST_DIR"), "/characters/guy.json"),
            ),
        ]);
        let config = HarnessConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
            .expect("controlled config");
        let mut character = config.character_sheet().expect("character");
        character.player_name = "Guy".to_owned();
        let request = ControlledPacketRequest {
            expected_character_id: "guy".to_owned(),
            expected_player_name: "Guy".to_owned(),
            expected_scene: "arena".to_owned(),
            proposal: TacticalProposal {
                intent: TacticalIntent::Stop,
                actions: vec![TacticalAction::Stop],
                valid_for_ms: 500,
                abort_if: Vec::new(),
                rationale: Some("bounded diagnostic".to_owned()),
            },
        };
        (config, character, request)
    }

    #[test]
    fn default_configuration_denies_release_with_zero_budget() {
        let values = HashMap::from([
            ("ARENA_API_KEY", "arena"),
            (
                "NPC_CHARACTER_SHEET_PATH",
                concat!(env!("CARGO_MANIFEST_DIR"), "/characters/guy.json"),
            ),
        ]);
        let config = HarnessConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
            .expect("default config");
        let character = config.character_sheet().expect("character");
        let request = ControlledPacketRequest {
            expected_character_id: character.id.clone(),
            expected_player_name: character.player_name.clone(),
            expected_scene: "arena".to_owned(),
            proposal: TacticalProposal {
                intent: TacticalIntent::Stop,
                actions: vec![TacticalAction::Stop],
                valid_for_ms: 500,
                abort_if: Vec::new(),
                rationale: None,
            },
        };
        let gate = LiveMutationGate::new(config.runtime.live_action_budget);

        assert_eq!(
            gate.authorize(&config.runtime, &character, Some("arena"), &request),
            Err(ControlledPacketError::RolloutNotControlled)
        );
        assert_eq!(gate.remaining_actions(), Some(0));
    }

    #[test]
    fn zero_budget_independently_disables_an_otherwise_authorized_gate() {
        let (mut config, character, request) = fixture();
        config.runtime.live_action_budget = LiveActionBudget::Limited(0);
        let gate = LiveMutationGate::new(config.runtime.live_action_budget);

        assert_eq!(
            gate.authorize(&config.runtime, &character, Some("arena"), &request),
            Err(ControlledPacketError::GateDisabled)
        );
    }

    #[test]
    fn exact_assertions_and_budget_allow_one_action_once() {
        let (config, character, request) = fixture();
        let mut gate = LiveMutationGate::new(config.runtime.live_action_budget);

        gate.authorize(&config.runtime, &character, Some("arena"), &request)
            .expect("authorized");
        assert_eq!(gate.consume(1).expect("consumed"), Some(0));
        assert!(matches!(
            gate.authorize(&config.runtime, &character, Some("arena"), &request),
            Err(ControlledPacketError::GateDisabled)
        ));
    }

    #[test]
    fn rejects_scene_and_lifetime_mismatches_without_consuming_budget() {
        let (mut config, character, request) = fixture();
        let gate = LiveMutationGate::new(config.runtime.live_action_budget);
        assert!(matches!(
            gate.authorize(&config.runtime, &character, Some("town"), &request),
            Err(ControlledPacketError::SceneMismatch { .. })
        ));

        config.runtime.live_packet_max_age = Duration::from_millis(100);
        assert!(matches!(
            gate.authorize(&config.runtime, &character, Some("arena"), &request),
            Err(ControlledPacketError::LifetimeTooLong { .. })
        ));
        assert_eq!(gate.remaining_actions(), Some(1));
    }

    #[test]
    fn full_model_packet_requires_the_same_exact_assertions_and_budget() {
        let (mut config, character, request) = fixture();
        config.runtime.tactical_rollout_mode = TacticalRolloutMode::Full;
        let mut gate = LiveMutationGate::new(config.runtime.live_action_budget);

        gate.authorize_model_packet(
            &config.runtime,
            &character,
            Some("arena"),
            &request.proposal,
        )
        .expect("full packet is bounded and exactly asserted");
        assert_eq!(gate.consume(request.proposal.actions.len()), Ok(Some(0)));
        assert_eq!(
            gate.authorize_model_packet(
                &config.runtime,
                &character,
                Some("arena"),
                &request.proposal,
            ),
            Err(ControlledPacketError::GateDisabled)
        );
    }

    #[test]
    fn explicitly_unlimited_budget_does_not_exhaust() {
        let (mut config, character, request) = fixture();
        config.runtime.live_action_budget = LiveActionBudget::Unlimited;
        let mut gate = LiveMutationGate::new(config.runtime.live_action_budget);

        for _ in 0..10_000 {
            gate.authorize(&config.runtime, &character, Some("arena"), &request)
                .expect("persistent release remains authorized");
            assert_eq!(gate.consume(1), Ok(None));
        }
        assert!(gate.is_unlimited());
        assert_eq!(gate.remaining_actions(), None);
    }

    #[test]
    fn explicit_scene_wildcard_allows_known_scene_changes_for_persistent_players() {
        let (mut config, character, mut request) = fixture();
        config.runtime.live_allowed_scene = Some("*".to_owned());
        request.expected_scene = "bot-forest".to_owned();
        let gate = LiveMutationGate::new(config.runtime.live_action_budget);

        gate.authorize(&config.runtime, &character, Some("bot-forest"), &request)
            .expect("explicit wildcard accepts the current known scene");

        config.runtime.tactical_rollout_mode = TacticalRolloutMode::Full;
        gate.authorize_model_packet(
            &config.runtime,
            &character,
            Some("reldens-town"),
            &request.proposal,
        )
        .expect("full rollout can traverse scenes");
        assert!(matches!(
            gate.authorize_model_packet(&config.runtime, &character, None, &request.proposal,),
            Err(ControlledPacketError::SceneMismatch { .. })
        ));
    }
}
