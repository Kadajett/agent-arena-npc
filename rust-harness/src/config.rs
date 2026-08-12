use std::{
    env,
    fmt::{Display, Formatter},
    path::PathBuf,
    str::FromStr,
    time::Duration,
};

use thiserror::Error;

use crate::character::{CharacterError, CharacterSheet};
use crate::runtime::tactical_schedule::TacticalRolloutMode;

const DEFAULT_ARENA_MCP_URL: &str = "https://mcp.yougotserved.dev/mcp";
const DEFAULT_STRATEGIST_MODEL: &str = "openai/gpt-oss-120b";
const DEFAULT_TACTICIAN_MODEL: &str = "google/gemini-3.1-flash-lite";

#[derive(Clone)]
pub struct HarnessConfig {
    pub arena: ArenaConfig,
    pub models: ModelConfig,
    pub runtime: RuntimeConfig,
    pub character: String,
    pub character_sheet_path: Option<PathBuf>,
    pub memory_path: PathBuf,
    pub local_rag_enabled: bool,
    pub local_rag_minimum_score: f64,
}

#[derive(Clone)]
pub struct ArenaConfig {
    pub mcp_url: String,
    pub api_key: String,
    pub request_timeout: Duration,
    pub reconnect_max_attempts: u32,
    pub reconnect_initial_backoff: Duration,
}

#[derive(Clone)]
pub struct ModelConfig {
    pub openrouter_api_key: Option<String>,
    pub strategist_model: String,
    pub tactician_model: String,
    pub tactician_temperature: f64,
    pub tactician_max_output_tokens: u64,
    pub tactician_request_timeout: Duration,
    pub tactician_reasoning: ModelReasoningConfig,
    pub strategist_temperature: f64,
    pub strategist_max_output_tokens: u64,
    pub strategist_request_timeout: Duration,
    pub strategist_reasoning: ModelReasoningConfig,
    pub strategist_min_interval: Duration,
    pub strategist_enabled: bool,
    pub strategist_memory_max_tokens: usize,
    pub local_input_capture: Option<PathBuf>,
    pub prompt_caching_enabled: bool,
}

/// `OpenRouter`'s provider-independent reasoning effort levels used by this harness.
///
/// `max`, `xhigh`, and `none` are deliberately excluded for now. The models
/// used by the live harness expose the four levels below; a separate `enabled`
/// flag represents disabled reasoning without overloading effort selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningEffort {
    Minimal,
    Low,
    Medium,
    High,
}

impl ReasoningEffort {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    /// Smallest completion budget that leaves practical room for both hidden
    /// reasoning and this harness's structured answer.
    #[must_use]
    pub const fn minimum_completion_tokens(self) -> u64 {
        match self {
            Self::Minimal => 512,
            Self::Low => 1_000,
            Self::Medium => 2_000,
            Self::High => 4_000,
        }
    }
}

impl Display for ReasoningEffort {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ReasoningEffort {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "minimal" => Ok(Self::Minimal),
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            _ => Err("reasoning effort must be one of minimal, low, medium, or high"),
        }
    }
}

/// Per-brain `OpenRouter` reasoning request policy.
///
/// Reasoning content is excluded by default. The harness records settings and
/// token accounting, never private reasoning text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelReasoningConfig {
    pub enabled: bool,
    pub effort: ReasoningEffort,
    pub exclude: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeConfig {
    pub tactical_max_hz: f64,
    pub idle_tactical_hz: f64,
    pub perception_interval: Duration,
    pub perception_map_radius: u32,
    pub perception_inventory_every_cycles: u64,
    pub tactical_rollout_mode: TacticalRolloutMode,
    pub allow_live_mutation: bool,
    pub live_action_budget: LiveActionBudget,
    pub live_max_actions_per_packet: u32,
    pub live_packet_max_age: Duration,
    pub live_expected_character_id: Option<String>,
    pub live_expected_player_name: Option<String>,
    pub live_allowed_scene: Option<String>,
    pub run_duration: Option<Duration>,
    pub run_max_openrouter_cost_usd: Option<f64>,
}

/// Process-local authorization budget for world mutations.
///
/// The default remains `Limited(0)`, which disables mutation. Persistent
/// deployments must opt in with the literal `unlimited`; a misspelled value
/// fails configuration instead of accidentally enabling the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveActionBudget {
    Limited(u32),
    Unlimited,
}

impl LiveActionBudget {
    #[must_use]
    pub const fn allows_any(self) -> bool {
        matches!(self, Self::Unlimited | Self::Limited(1..))
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("required environment variable {0} is not set")]
    Missing(&'static str),
    #[error("{name} must be a finite number greater than zero, got {value:?}")]
    InvalidRate { name: &'static str, value: String },
    #[error("{name} must be a valid {kind}, got {value:?}")]
    InvalidValue {
        name: &'static str,
        kind: &'static str,
        value: String,
    },
    #[error(transparent)]
    Character(#[from] CharacterError),
}

impl HarnessConfig {
    /// Load and validate harness configuration from the process environment.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when a required secret is absent or a configured
    /// numeric value is invalid.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|name| env::var(name).ok())
    }

    /// Load configuration through an injected environment lookup.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] for missing required values or invalid numeric
    /// configuration.
    #[allow(
        clippy::too_many_lines,
        reason = "configuration validation stays centralized so unsafe rollout combinations fail together"
    )]
    pub fn from_lookup(
        mut lookup: impl FnMut(&str) -> Option<String>,
    ) -> Result<Self, ConfigError> {
        let arena_api_key = required(&mut lookup, "ARENA_API_KEY")?;
        let openrouter_api_key =
            lookup("OPENROUTER_API_KEY").filter(|value| !value.trim().is_empty());
        let tactical_max_hz = parse_rate(&mut lookup, "NPC_TACTICAL_MAX_HZ", 5.0)?;
        let idle_tactical_hz = parse_nonnegative_rate(&mut lookup, "NPC_IDLE_TACTICAL_HZ", 0.2)?;
        let tactician_temperature: f64 = parse_value(
            &mut lookup,
            "NPC_TACTICIAN_TEMPERATURE",
            "0.1",
            "floating-point number",
        )?;
        if !tactician_temperature.is_finite() || !(0.0..=2.0).contains(&tactician_temperature) {
            return Err(ConfigError::InvalidValue {
                name: "NPC_TACTICIAN_TEMPERATURE",
                kind: "finite floating-point number between 0 and 2",
                value: tactician_temperature.to_string(),
            });
        }
        let tactician_max_output_tokens: u64 = parse_value(
            &mut lookup,
            "NPC_TACTICIAN_MAX_OUTPUT_TOKENS",
            "150",
            "positive integer",
        )?;
        if tactician_max_output_tokens == 0 {
            return Err(ConfigError::InvalidValue {
                name: "NPC_TACTICIAN_MAX_OUTPUT_TOKENS",
                kind: "positive integer",
                value: tactician_max_output_tokens.to_string(),
            });
        }
        let tactician_timeout_ms =
            parse_positive_u64(&mut lookup, "NPC_TACTICIAN_TIMEOUT_MS", 5_000)?;
        let tactician_reasoning = parse_reasoning_config(
            &mut lookup,
            "NPC_TACTICIAN_REASONING_ENABLED",
            false,
            "NPC_TACTICIAN_REASONING_EFFORT",
            ReasoningEffort::Minimal,
            "NPC_TACTICIAN_REASONING_EXCLUDE",
        )?;
        let strategist_temperature: f64 = parse_value(
            &mut lookup,
            "NPC_STRATEGIST_TEMPERATURE",
            "0.4",
            "floating-point number",
        )?;
        if !strategist_temperature.is_finite() || !(0.0..=2.0).contains(&strategist_temperature) {
            return Err(ConfigError::InvalidValue {
                name: "NPC_STRATEGIST_TEMPERATURE",
                kind: "finite floating-point number between 0 and 2",
                value: strategist_temperature.to_string(),
            });
        }
        let strategist_max_output_tokens: u64 = parse_value(
            &mut lookup,
            "NPC_STRATEGIST_MAX_OUTPUT_TOKENS",
            "4000",
            "positive integer",
        )?;
        if strategist_max_output_tokens == 0 {
            return Err(ConfigError::InvalidValue {
                name: "NPC_STRATEGIST_MAX_OUTPUT_TOKENS",
                kind: "positive integer",
                value: strategist_max_output_tokens.to_string(),
            });
        }
        let strategist_timeout_ms =
            parse_positive_u64(&mut lookup, "NPC_STRATEGIST_TIMEOUT_MS", 60_000)?;
        let strategist_reasoning = parse_reasoning_config(
            &mut lookup,
            "NPC_STRATEGIST_REASONING_ENABLED",
            true,
            "NPC_STRATEGIST_REASONING_EFFORT",
            ReasoningEffort::Medium,
            "NPC_STRATEGIST_REASONING_EXCLUDE",
        )?;
        validate_reasoning_budget(
            "NPC_STRATEGIST_MAX_OUTPUT_TOKENS",
            strategist_max_output_tokens,
            strategist_reasoning,
        )?;
        validate_reasoning_budget(
            "NPC_TACTICIAN_MAX_OUTPUT_TOKENS",
            tactician_max_output_tokens,
            tactician_reasoning,
        )?;
        let strategist_min_interval_ms: u64 = parse_value(
            &mut lookup,
            "NPC_STRATEGIST_MIN_INTERVAL_MS",
            "30000",
            "non-negative integer",
        )?;
        let strategist_enabled: bool =
            parse_value(&mut lookup, "NPC_STRATEGIST_ENABLED", "false", "boolean")?;
        let strategist_memory_max_tokens = usize::try_from(parse_positive_u64(
            &mut lookup,
            "NPC_STRATEGIST_MEMORY_MAX_TOKENS",
            8_000,
        )?)
        .map_err(|_| ConfigError::InvalidValue {
            name: "NPC_STRATEGIST_MEMORY_MAX_TOKENS",
            kind: "positive integer supported by this platform",
            value: "out_of_range".to_owned(),
        })?;
        let local_input_capture_enabled: bool = parse_value(
            &mut lookup,
            "NPC_LOCAL_MODEL_INPUT_CAPTURE_ENABLED",
            "false",
            "boolean",
        )?;
        let local_input_capture = local_input_capture_enabled.then(|| {
            lookup("NPC_LOCAL_MODEL_INPUT_CAPTURE_PATH")
                .filter(|value| !value.trim().is_empty())
                .map_or_else(
                    || PathBuf::from("./var/model-input-captures"),
                    PathBuf::from,
                )
        });
        let prompt_caching_enabled: bool = parse_value(
            &mut lookup,
            "NPC_OPENROUTER_PROMPT_CACHING_ENABLED",
            "true",
            "boolean",
        )?;
        let local_rag_enabled: bool =
            parse_value(&mut lookup, "NPC_LOCAL_RAG_ENABLED", "true", "boolean")?;
        let local_rag_minimum_score: f64 = parse_value(
            &mut lookup,
            "NPC_LOCAL_RAG_MINIMUM_SCORE",
            "0.25",
            "floating-point number between 0 and 1",
        )?;
        if !local_rag_minimum_score.is_finite() || !(0.0..=1.0).contains(&local_rag_minimum_score) {
            return Err(ConfigError::InvalidValue {
                name: "NPC_LOCAL_RAG_MINIMUM_SCORE",
                kind: "finite floating-point number between 0 and 1",
                value: local_rag_minimum_score.to_string(),
            });
        }
        let request_timeout_ms = parse_positive_u64(&mut lookup, "ARENA_MCP_TIMEOUT_MS", 60_000)?;
        let reconnect_max_attempts =
            parse_positive_u32(&mut lookup, "ARENA_RECONNECT_MAX_ATTEMPTS", 5)?;
        let reconnect_initial_backoff_ms =
            parse_positive_u64(&mut lookup, "ARENA_RECONNECT_INITIAL_BACKOFF_MS", 250)?;
        let perception_interval_ms =
            parse_positive_u64(&mut lookup, "NPC_PERCEPTION_INTERVAL_MS", 500)?;
        let perception_map_radius =
            parse_positive_u32(&mut lookup, "NPC_PERCEPTION_MAP_RADIUS", 16)?;
        let perception_inventory_every_cycles =
            parse_positive_u64(&mut lookup, "NPC_PERCEPTION_INVENTORY_EVERY_CYCLES", 10)?;
        let tactical_rollout_mode: TacticalRolloutMode = parse_value(
            &mut lookup,
            "NPC_TACTICAL_ROLLOUT_MODE",
            "observe_only",
            "one of observe_only, shadow, controlled, or full",
        )?;
        let allow_live_mutation: bool =
            parse_value(&mut lookup, "NPC_ALLOW_LIVE_MUTATION", "false", "boolean")?;
        let live_action_budget = parse_live_action_budget(&mut lookup)?;
        let live_max_actions_per_packet =
            parse_positive_u32(&mut lookup, "NPC_LIVE_MAX_ACTIONS_PER_PACKET", 1)?;
        let live_packet_max_age_ms =
            parse_positive_u64(&mut lookup, "NPC_LIVE_PACKET_MAX_AGE_MS", 1_000)?;
        let live_expected_character_id =
            optional_nonempty(&mut lookup, "NPC_LIVE_EXPECTED_CHARACTER_ID");
        let live_expected_player_name =
            optional_nonempty(&mut lookup, "NPC_LIVE_EXPECTED_PLAYER_NAME");
        let live_allowed_scene = optional_nonempty(&mut lookup, "NPC_LIVE_ALLOWED_SCENE");
        let run_duration = optional_positive_u64(&mut lookup, "NPC_RUN_DURATION_SECONDS")?
            .map(Duration::from_secs);
        let run_max_openrouter_cost_usd =
            optional_positive_f64(&mut lookup, "NPC_RUN_MAX_OPENROUTER_COST_USD")?;
        if tactical_rollout_mode == TacticalRolloutMode::Full && !allow_live_mutation {
            return Err(ConfigError::InvalidValue {
                name: "NPC_TACTICAL_ROLLOUT_MODE",
                kind: "full only when NPC_ALLOW_LIVE_MUTATION=true",
                value: tactical_rollout_mode.to_string(),
            });
        }

        let character = lookup("NPC_CHARACTER")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "guy".to_owned())
            .to_lowercase();
        Ok(Self {
            arena: ArenaConfig {
                mcp_url: lookup("ARENA_MCP_URL")
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| DEFAULT_ARENA_MCP_URL.to_owned()),
                api_key: arena_api_key,
                request_timeout: Duration::from_millis(request_timeout_ms),
                reconnect_max_attempts,
                reconnect_initial_backoff: Duration::from_millis(reconnect_initial_backoff_ms),
            },
            models: ModelConfig {
                openrouter_api_key,
                strategist_model: lookup("NPC_STRATEGIST_MODEL")
                    .or_else(|| lookup("NPC_MODEL"))
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| DEFAULT_STRATEGIST_MODEL.to_owned()),
                tactician_model: lookup("NPC_TACTICIAN_MODEL")
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| DEFAULT_TACTICIAN_MODEL.to_owned()),
                tactician_temperature,
                tactician_max_output_tokens,
                tactician_request_timeout: Duration::from_millis(tactician_timeout_ms),
                tactician_reasoning,
                strategist_temperature,
                strategist_max_output_tokens,
                strategist_request_timeout: Duration::from_millis(strategist_timeout_ms),
                strategist_reasoning,
                strategist_min_interval: Duration::from_millis(strategist_min_interval_ms),
                strategist_enabled,
                strategist_memory_max_tokens,
                local_input_capture,
                prompt_caching_enabled,
            },
            runtime: RuntimeConfig {
                tactical_max_hz,
                idle_tactical_hz,
                perception_interval: Duration::from_millis(perception_interval_ms),
                perception_map_radius,
                perception_inventory_every_cycles,
                tactical_rollout_mode,
                allow_live_mutation,
                live_action_budget,
                live_max_actions_per_packet,
                live_packet_max_age: Duration::from_millis(live_packet_max_age_ms),
                live_expected_character_id,
                live_expected_player_name,
                live_allowed_scene,
                run_duration,
                run_max_openrouter_cost_usd,
            },
            character_sheet_path: optional_nonempty(&mut lookup, "NPC_CHARACTER_SHEET_PATH")
                .map(PathBuf::from),
            memory_path: lookup("NPC_MEMORY_PATH")
                .or_else(|| lookup("NPC_MEMORY_DIR"))
                .filter(|value| !value.trim().is_empty())
                .map_or_else(
                    || PathBuf::from(format!("./var/{character}.sqlite")),
                    PathBuf::from,
                ),
            local_rag_enabled,
            local_rag_minimum_score,
            character,
        })
    }

    /// Build the selected character sheet.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Character`] when the configured external sheet
    /// cannot be loaded. Returns [`ConfigError::Missing`] when no sheet path
    /// was configured.
    pub fn character_sheet(&self) -> Result<CharacterSheet, ConfigError> {
        let path = self
            .character_sheet_path
            .as_ref()
            .ok_or(ConfigError::Missing("NPC_CHARACTER_SHEET_PATH"))?;
        CharacterSheet::from_file(path, self).map_err(ConfigError::from)
    }

    /// Return the configured `OpenRouter` key when a model-enabled component starts.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Missing`] when no key is configured.
    pub fn openrouter_api_key(&self) -> Result<&str, ConfigError> {
        self.models
            .openrouter_api_key
            .as_deref()
            .ok_or(ConfigError::Missing("OPENROUTER_API_KEY"))
    }
}

fn parse_reasoning_config(
    lookup: &mut impl FnMut(&str) -> Option<String>,
    enabled_name: &'static str,
    enabled_default: bool,
    effort_name: &'static str,
    effort_default: ReasoningEffort,
    exclude_name: &'static str,
) -> Result<ModelReasoningConfig, ConfigError> {
    Ok(ModelReasoningConfig {
        enabled: parse_value(
            lookup,
            enabled_name,
            if enabled_default { "true" } else { "false" },
            "boolean",
        )?,
        effort: parse_value(
            lookup,
            effort_name,
            effort_default.as_str(),
            "one of minimal, low, medium, or high",
        )?,
        // Provider-returned reasoning is not needed by either typed brain. It
        // remains opt-in while content telemetry is always disabled in code.
        exclude: parse_value(lookup, exclude_name, "true", "boolean")?,
    })
}

fn validate_reasoning_budget(
    name: &'static str,
    max_output_tokens: u64,
    reasoning: ModelReasoningConfig,
) -> Result<(), ConfigError> {
    let minimum = reasoning.effort.minimum_completion_tokens();
    if reasoning.enabled && max_output_tokens < minimum {
        return Err(ConfigError::InvalidValue {
            name,
            kind: "completion-token budget large enough for configured reasoning effort and structured output",
            value: format!(
                "{max_output_tokens} (minimum {minimum} for {} reasoning)",
                reasoning.effort
            ),
        });
    }
    Ok(())
}

fn parse_positive_u64(
    lookup: &mut impl FnMut(&str) -> Option<String>,
    name: &'static str,
    default: u64,
) -> Result<u64, ConfigError> {
    let value: u64 = parse_value(lookup, name, &default.to_string(), "positive integer")?;
    if value == 0 {
        return Err(ConfigError::InvalidValue {
            name,
            kind: "positive integer",
            value: value.to_string(),
        });
    }
    Ok(value)
}

fn parse_positive_u32(
    lookup: &mut impl FnMut(&str) -> Option<String>,
    name: &'static str,
    default: u32,
) -> Result<u32, ConfigError> {
    let value: u32 = parse_value(lookup, name, &default.to_string(), "positive integer")?;
    if value == 0 {
        return Err(ConfigError::InvalidValue {
            name,
            kind: "positive integer",
            value: value.to_string(),
        });
    }
    Ok(value)
}

fn parse_live_action_budget(
    lookup: &mut impl FnMut(&str) -> Option<String>,
) -> Result<LiveActionBudget, ConfigError> {
    let raw = lookup("NPC_LIVE_ACTION_BUDGET").unwrap_or_else(|| "0".to_owned());
    if raw.eq_ignore_ascii_case("unlimited") {
        return Ok(LiveActionBudget::Unlimited);
    }
    raw.parse::<u32>()
        .map(LiveActionBudget::Limited)
        .map_err(|_| ConfigError::InvalidValue {
            name: "NPC_LIVE_ACTION_BUDGET",
            kind: "non-negative integer or the literal unlimited",
            value: raw,
        })
}

fn optional_nonempty(
    lookup: &mut impl FnMut(&str) -> Option<String>,
    name: &'static str,
) -> Option<String> {
    lookup(name).filter(|value| !value.trim().is_empty())
}

fn optional_positive_u64(
    lookup: &mut impl FnMut(&str) -> Option<String>,
    name: &'static str,
) -> Result<Option<u64>, ConfigError> {
    let Some(raw) = lookup(name).filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };
    let value = raw.parse::<u64>().map_err(|_| ConfigError::InvalidValue {
        name,
        kind: "positive integer",
        value: raw.clone(),
    })?;
    if value == 0 {
        return Err(ConfigError::InvalidValue {
            name,
            kind: "positive integer",
            value: raw,
        });
    }
    Ok(Some(value))
}

fn optional_positive_f64(
    lookup: &mut impl FnMut(&str) -> Option<String>,
    name: &'static str,
) -> Result<Option<f64>, ConfigError> {
    let Some(raw) = lookup(name).filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };
    let value = raw.parse::<f64>().map_err(|_| ConfigError::InvalidValue {
        name,
        kind: "finite number greater than zero",
        value: raw.clone(),
    })?;
    if !value.is_finite() || value <= 0.0 {
        return Err(ConfigError::InvalidValue {
            name,
            kind: "finite number greater than zero",
            value: raw,
        });
    }
    Ok(Some(value))
}

fn required(
    lookup: &mut impl FnMut(&str) -> Option<String>,
    name: &'static str,
) -> Result<String, ConfigError> {
    lookup(name)
        .filter(|value| !value.trim().is_empty())
        .ok_or(ConfigError::Missing(name))
}

fn parse_rate(
    lookup: &mut impl FnMut(&str) -> Option<String>,
    name: &'static str,
    default: f64,
) -> Result<f64, ConfigError> {
    let raw = lookup(name).unwrap_or_else(|| default.to_string());
    let value = raw.parse::<f64>().map_err(|_| ConfigError::InvalidRate {
        name,
        value: raw.clone(),
    })?;
    if !value.is_finite() || value <= 0.0 {
        return Err(ConfigError::InvalidRate { name, value: raw });
    }
    Ok(value)
}

fn parse_nonnegative_rate(
    lookup: &mut impl FnMut(&str) -> Option<String>,
    name: &'static str,
    default: f64,
) -> Result<f64, ConfigError> {
    let raw = lookup(name).unwrap_or_else(|| default.to_string());
    let value = raw.parse::<f64>().map_err(|_| ConfigError::InvalidRate {
        name,
        value: raw.clone(),
    })?;
    if !value.is_finite() || value < 0.0 {
        return Err(ConfigError::InvalidRate { name, value: raw });
    }
    Ok(value)
}

fn parse_value<T>(
    lookup: &mut impl FnMut(&str) -> Option<String>,
    name: &'static str,
    default: &str,
    kind: &'static str,
) -> Result<T, ConfigError>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    let raw = lookup(name).unwrap_or_else(|| default.to_owned());
    raw.parse().map_err(|_| ConfigError::InvalidValue {
        name,
        kind,
        value: raw,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn loads_defaults_and_legacy_strategist_model() {
        let values = HashMap::from([
            ("ARENA_API_KEY", "arena"),
            ("OPENROUTER_API_KEY", "router"),
            ("NPC_MODEL", "legacy/model"),
        ]);
        let config = HarnessConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
            .expect("valid configuration");

        assert_eq!(config.character, "guy");
        assert_eq!(config.character_sheet_path, None);
        assert_eq!(config.models.strategist_model, "legacy/model");
        assert_eq!(
            config.models.tactician_model,
            "google/gemini-3.1-flash-lite"
        );
        assert!((config.runtime.tactical_max_hz - 5.0).abs() < f64::EPSILON);
        assert_eq!(
            config.runtime.live_action_budget,
            LiveActionBudget::Limited(0)
        );
        assert_eq!(config.runtime.live_max_actions_per_packet, 1);
        assert_eq!(config.runtime.live_packet_max_age, Duration::from_secs(1));
        assert_eq!(config.runtime.live_expected_character_id, None);
        assert_eq!(config.runtime.run_max_openrouter_cost_usd, None);
        assert_eq!(config.runtime.perception_inventory_every_cycles, 10);
        assert_eq!(
            config.models.tactician_request_timeout,
            Duration::from_secs(5)
        );
        assert_eq!(
            config.models.strategist_request_timeout,
            Duration::from_mins(1)
        );
        assert_eq!(config.models.strategist_memory_max_tokens, 8_000);
        assert_eq!(config.models.strategist_max_output_tokens, 4_000);
        assert_eq!(
            config.models.strategist_reasoning,
            ModelReasoningConfig {
                enabled: true,
                effort: ReasoningEffort::Medium,
                exclude: true,
            }
        );
        assert_eq!(
            config.models.tactician_reasoning,
            ModelReasoningConfig {
                enabled: false,
                effort: ReasoningEffort::Minimal,
                exclude: true,
            }
        );
        assert!(config.models.prompt_caching_enabled);
        assert_eq!(config.models.local_input_capture, None);
        assert!(config.local_rag_enabled);
        assert!((config.local_rag_minimum_score - 0.25).abs() < f64::EPSILON);
    }

    #[test]
    fn selects_an_external_character_sheet_and_character_specific_memory_default() {
        let values = HashMap::from([
            ("ARENA_API_KEY", "arena"),
            ("NPC_CHARACTER", "ORIN"),
            ("NPC_CHARACTER_SHEET_PATH", "/run/characters/orin.json"),
        ]);
        let config = HarnessConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
            .expect("external character configuration");

        assert_eq!(config.character, "orin");
        assert_eq!(
            config.character_sheet_path,
            Some(PathBuf::from("/run/characters/orin.json"))
        );
        assert_eq!(config.memory_path, PathBuf::from("./var/orin.sqlite"));
    }

    #[test]
    fn rejects_zero_rate() {
        let values = HashMap::from([
            ("ARENA_API_KEY", "arena"),
            ("OPENROUTER_API_KEY", "router"),
            ("NPC_TACTICAL_MAX_HZ", "0"),
        ]);
        let error = HarnessConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
            .err()
            .expect("rate should be rejected");

        assert!(matches!(error, ConfigError::InvalidRate { .. }));
    }

    #[test]
    fn rejects_zero_inventory_refresh_cadence() {
        let values = HashMap::from([
            ("ARENA_API_KEY", "arena"),
            ("NPC_PERCEPTION_INVENTORY_EVERY_CYCLES", "0"),
        ]);
        let error = HarnessConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
            .err()
            .expect("zero cadence should be rejected");

        assert!(matches!(
            error,
            ConfigError::InvalidValue {
                name: "NPC_PERCEPTION_INVENTORY_EVERY_CYCLES",
                ..
            }
        ));
    }

    #[test]
    fn rejects_out_of_range_local_rag_score() {
        let values = HashMap::from([
            ("ARENA_API_KEY", "arena"),
            ("NPC_LOCAL_RAG_MINIMUM_SCORE", "1.01"),
        ]);
        let error = HarnessConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
            .err()
            .expect("out-of-range score should be rejected");

        assert!(matches!(
            error,
            ConfigError::InvalidValue {
                name: "NPC_LOCAL_RAG_MINIMUM_SCORE",
                ..
            }
        ));
    }

    #[test]
    fn rejects_zero_model_request_timeout() {
        let values = HashMap::from([
            ("ARENA_API_KEY", "arena"),
            ("NPC_TACTICIAN_TIMEOUT_MS", "0"),
        ]);
        let error = HarnessConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
            .err()
            .expect("zero timeout should be rejected");

        assert!(matches!(
            error,
            ConfigError::InvalidValue {
                name: "NPC_TACTICIAN_TIMEOUT_MS",
                ..
            }
        ));
    }

    #[test]
    fn rejects_unknown_reasoning_effort() {
        let values = HashMap::from([
            ("ARENA_API_KEY", "arena"),
            ("NPC_STRATEGIST_REASONING_EFFORT", "heroic"),
        ]);
        let error = HarnessConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
            .err()
            .expect("unknown reasoning effort must fail");

        assert!(matches!(
            error,
            ConfigError::InvalidValue {
                name: "NPC_STRATEGIST_REASONING_EFFORT",
                ..
            }
        ));
    }

    #[test]
    fn rejects_reasoning_budget_that_cannot_hold_thinking_and_output() {
        let values = HashMap::from([
            ("ARENA_API_KEY", "arena"),
            ("NPC_STRATEGIST_REASONING_ENABLED", "true"),
            ("NPC_STRATEGIST_REASONING_EFFORT", "high"),
            ("NPC_STRATEGIST_MAX_OUTPUT_TOKENS", "3999"),
        ]);
        let error = HarnessConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
            .err()
            .expect("undersized shared completion budget must fail");

        assert!(matches!(
            error,
            ConfigError::InvalidValue {
                name: "NPC_STRATEGIST_MAX_OUTPUT_TOKENS",
                ..
            }
        ));
    }

    #[test]
    fn permits_explicit_minimal_tactical_reasoning_with_a_bounded_budget() {
        let values = HashMap::from([
            ("ARENA_API_KEY", "arena"),
            ("NPC_TACTICIAN_REASONING_ENABLED", "true"),
            ("NPC_TACTICIAN_REASONING_EFFORT", "minimal"),
            ("NPC_TACTICIAN_REASONING_EXCLUDE", "false"),
            ("NPC_TACTICIAN_MAX_OUTPUT_TOKENS", "512"),
        ]);
        let config = HarnessConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
            .expect("valid tactical reasoning configuration");

        assert_eq!(
            config.models.tactician_reasoning,
            ModelReasoningConfig {
                enabled: true,
                effort: ReasoningEffort::Minimal,
                exclude: false,
            }
        );
    }

    #[test]
    fn local_model_input_capture_is_default_off_and_explicitly_located() {
        let defaults = HashMap::from([("ARENA_API_KEY", "arena")]);
        let config = HarnessConfig::from_lookup(|key| defaults.get(key).map(ToString::to_string))
            .expect("default configuration");
        assert_eq!(config.models.local_input_capture, None);

        let enabled = HashMap::from([
            ("ARENA_API_KEY", "arena"),
            ("NPC_LOCAL_MODEL_INPUT_CAPTURE_ENABLED", "true"),
            (
                "NPC_LOCAL_MODEL_INPUT_CAPTURE_PATH",
                "/tmp/private-model-inputs",
            ),
        ]);
        let config = HarnessConfig::from_lookup(|key| enabled.get(key).map(ToString::to_string))
            .expect("explicit capture configuration");
        assert_eq!(
            config.models.local_input_capture,
            Some(PathBuf::from("/tmp/private-model-inputs"))
        );
    }

    #[test]
    fn permits_disabling_idle_tactical_inference() {
        let values = HashMap::from([
            ("ARENA_API_KEY", "arena"),
            ("NPC_IDLE_TACTICAL_HZ", "0"),
            ("NPC_RUN_DURATION_SECONDS", "20"),
        ]);
        let config = HarnessConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
            .expect("valid bounded quiet runtime");

        assert!(config.runtime.idle_tactical_hz.abs() < f64::EPSILON);
        assert_eq!(config.runtime.run_duration, Some(Duration::from_secs(20)));
    }

    #[test]
    fn loads_positive_run_cost_ceiling() {
        let values = HashMap::from([
            ("ARENA_API_KEY", "arena"),
            ("NPC_RUN_MAX_OPENROUTER_COST_USD", "0.025"),
        ]);
        let config = HarnessConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
            .expect("bounded cost configuration");

        assert_eq!(config.runtime.run_max_openrouter_cost_usd, Some(0.025));
    }

    #[test]
    fn rejects_invalid_run_cost_ceiling() {
        for invalid in ["0", "-1", "NaN", "inf", "not-money"] {
            let values = HashMap::from([
                ("ARENA_API_KEY", "arena"),
                ("NPC_RUN_MAX_OPENROUTER_COST_USD", invalid),
            ]);
            let error = HarnessConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
                .err()
                .expect("invalid cost ceiling");
            assert!(matches!(
                error,
                ConfigError::InvalidValue {
                    name: "NPC_RUN_MAX_OPENROUTER_COST_USD",
                    ..
                }
            ));
        }
    }

    #[test]
    fn permits_mcp_only_runtime_without_model_credentials() {
        let values = HashMap::from([("ARENA_API_KEY", "arena")]);
        let config = HarnessConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
            .expect("MCP configuration");

        assert_eq!(config.models.openrouter_api_key, None);
        assert!(matches!(
            config.openrouter_api_key(),
            Err(ConfigError::Missing("OPENROUTER_API_KEY"))
        ));
    }

    #[test]
    fn loads_explicit_live_mutation_safety_facts() {
        let values = HashMap::from([
            ("ARENA_API_KEY", "arena"),
            ("NPC_LIVE_ACTION_BUDGET", "2"),
            ("NPC_LIVE_MAX_ACTIONS_PER_PACKET", "1"),
            ("NPC_LIVE_PACKET_MAX_AGE_MS", "750"),
            ("NPC_LIVE_EXPECTED_CHARACTER_ID", "guy"),
            ("NPC_LIVE_EXPECTED_PLAYER_NAME", "Guy Diagnostic"),
            ("NPC_LIVE_ALLOWED_SCENE", "combat-test"),
        ]);
        let config = HarnessConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
            .expect("live safety config");

        assert_eq!(
            config.runtime.live_action_budget,
            LiveActionBudget::Limited(2)
        );
        assert_eq!(
            config.runtime.live_packet_max_age,
            Duration::from_millis(750)
        );
        assert_eq!(
            config.runtime.live_expected_player_name.as_deref(),
            Some("Guy Diagnostic")
        );
        assert_eq!(
            config.runtime.live_allowed_scene.as_deref(),
            Some("combat-test")
        );
    }

    #[test]
    fn persistent_action_budget_requires_the_explicit_unlimited_literal() {
        let values = HashMap::from([
            ("ARENA_API_KEY", "arena"),
            ("NPC_LIVE_ACTION_BUDGET", "unlimited"),
        ]);
        let config = HarnessConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
            .expect("persistent budget");
        assert_eq!(
            config.runtime.live_action_budget,
            LiveActionBudget::Unlimited
        );

        let invalid = HashMap::from([
            ("ARENA_API_KEY", "arena"),
            ("NPC_LIVE_ACTION_BUDGET", "infinite"),
        ]);
        assert!(matches!(
            HarnessConfig::from_lookup(|key| invalid.get(key).map(ToString::to_string)),
            Err(ConfigError::InvalidValue {
                name: "NPC_LIVE_ACTION_BUDGET",
                ..
            })
        ));
    }
}
