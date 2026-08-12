use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{config::HarnessConfig, memory::working::Goal};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Speak,
    TalkToFolk,
    Walk,
    Doors,
    Fight,
    Duel,
    Money,
    Trade,
    Purpose,
}

#[derive(Debug, Clone)]
pub struct CharacterSheet {
    pub id: String,
    pub player_name: String,
    pub registration_version: u32,
    pub class_path: Option<String>,
    pub home_scene: String,
    pub persona: String,
    pub capabilities: HashSet<Capability>,
    pub strategist_model: String,
    pub tactician_model: String,
    pub initial_goal: Option<Goal>,
    pub remembers: bool,
}

#[derive(Debug, Error)]
pub enum CharacterError {
    #[error("could not read character sheet {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("character sheet {path} is not valid JSON: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("character sheet {path} is invalid: {reason}")]
    Invalid { path: PathBuf, reason: String },
    #[error("could not read persona {path}: {source}")]
    ReadPersona {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Serializable character configuration used by general-purpose deployments.
///
/// Secrets and backend identity are intentionally absent. The harness binds
/// those from its session configuration after it loads this document.
#[derive(Debug, Deserialize)]
struct CharacterSheetDocument {
    id: String,
    player_name: String,
    #[serde(default = "default_registration_version")]
    registration_version: u32,
    class_path: Option<String>,
    home_scene: String,
    #[serde(default)]
    persona: Option<String>,
    #[serde(default)]
    persona_file: Option<PathBuf>,
    capabilities: HashSet<Capability>,
    #[serde(default)]
    strategist_model: Option<String>,
    #[serde(default)]
    tactician_model: Option<String>,
    #[serde(default)]
    initial_goal: Option<Goal>,
    #[serde(default = "default_remembers")]
    remembers: bool,
}

const fn default_registration_version() -> u32 {
    1
}

const fn default_remembers() -> bool {
    true
}

impl CharacterSheet {
    /// Load a character sheet from a JSON document.
    ///
    /// A relative `persona_file` is resolved from the document's directory.
    /// Model fields may be omitted to inherit the harness-wide configured
    /// strategist and tactician models. `ARENA_PLAYER_NAME` remains an
    /// operational override so one sheet can be used for an isolated test
    /// registration without editing the document.
    ///
    /// # Errors
    ///
    /// Returns an error when the document or persona cannot be read, the JSON
    /// cannot be decoded, or required identity and policy fields are empty.
    pub fn from_file(
        path: impl AsRef<Path>,
        config: &HarnessConfig,
    ) -> Result<Self, CharacterError> {
        let path = path.as_ref();
        let bytes = fs::read(path).map_err(|source| CharacterError::Read {
            path: path.to_owned(),
            source,
        })?;
        let document: CharacterSheetDocument =
            serde_json::from_slice(&bytes).map_err(|source| CharacterError::Parse {
                path: path.to_owned(),
                source,
            })?;
        document.into_sheet(path, config)
    }
}

impl CharacterSheetDocument {
    fn into_sheet(
        self,
        document_path: &Path,
        config: &HarnessConfig,
    ) -> Result<CharacterSheet, CharacterError> {
        require_nonempty(document_path, "id", &self.id)?;
        require_nonempty(document_path, "player_name", &self.player_name)?;
        require_nonempty(document_path, "home_scene", &self.home_scene)?;
        if self.registration_version == 0 {
            return Err(invalid(
                document_path,
                "registration_version must be positive",
            ));
        }
        if self.capabilities.is_empty() {
            return Err(invalid(document_path, "capabilities must not be empty"));
        }
        let persona = match (self.persona, self.persona_file) {
            (Some(persona), None) if !persona.trim().is_empty() => persona,
            (None, Some(persona_file)) => {
                let persona_path = if persona_file.is_absolute() {
                    persona_file
                } else {
                    document_path
                        .parent()
                        .unwrap_or_else(|| Path::new("."))
                        .join(persona_file)
                };
                fs::read_to_string(&persona_path).map_err(|source| CharacterError::ReadPersona {
                    path: persona_path,
                    source,
                })?
            }
            (Some(_), Some(_)) => {
                return Err(invalid(
                    document_path,
                    "set exactly one of persona or persona_file",
                ));
            }
            _ => {
                return Err(invalid(
                    document_path,
                    "set a non-empty persona or a persona_file",
                ));
            }
        };
        let player_name = std::env::var("ARENA_PLAYER_NAME")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(self.player_name);

        Ok(CharacterSheet {
            id: self.id,
            player_name,
            registration_version: self.registration_version,
            class_path: self.class_path,
            home_scene: self.home_scene,
            persona,
            capabilities: self.capabilities,
            strategist_model: nonempty_or(self.strategist_model, &config.models.strategist_model),
            tactician_model: nonempty_or(self.tactician_model, &config.models.tactician_model),
            initial_goal: self.initial_goal,
            remembers: self.remembers,
        })
    }
}

fn require_nonempty(path: &Path, field: &str, value: &str) -> Result<(), CharacterError> {
    if value.trim().is_empty() {
        return Err(invalid(path, &format!("{field} must not be empty")));
    }
    Ok(())
}

fn invalid(path: &Path, reason: &str) -> CharacterError {
    CharacterError::Invalid {
        path: path.to_owned(),
        reason: reason.to_owned(),
    }
}

fn nonempty_or(value: Option<String>, fallback: &str) -> String {
    value
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| fallback.to_owned())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn barnaby_is_confined_to_the_inn_by_body_capabilities() {
        let values = HashMap::from([("ARENA_API_KEY", "arena"), ("OPENROUTER_API_KEY", "router")]);
        let config = HarnessConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
            .expect("configuration");
        let barnaby = CharacterSheet::from_file(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("characters/barnaby.json"),
            &config,
        )
        .expect("Barnaby sheet");

        assert_eq!(barnaby.class_path.as_deref(), Some("journeyman"));
        assert_eq!(barnaby.home_scene, "reldens-house-1");
        assert!(barnaby.capabilities.contains(&Capability::Speak));
        assert!(barnaby.capabilities.contains(&Capability::TalkToFolk));
        assert!(!barnaby.capabilities.contains(&Capability::Walk));
        assert!(!barnaby.capabilities.contains(&Capability::Doors));
        assert!(!barnaby.capabilities.contains(&Capability::Fight));
        assert!(barnaby.initial_goal.is_none());
        assert!(barnaby.remembers);
    }

    #[test]
    fn loads_an_arbitrary_character_without_a_rust_registry_entry() {
        let values = HashMap::from([
            ("ARENA_API_KEY", "arena"),
            ("NPC_STRATEGIST_MODEL", "provider/strategist"),
            ("NPC_TACTICIAN_MODEL", "provider/tactician"),
        ]);
        let config = HarnessConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
            .expect("configuration");
        let directory = tempfile::tempdir().expect("temporary directory");
        let persona = directory.path().join("persona.md");
        fs::write(&persona, "A cartographer who records only firsthand facts.")
            .expect("write persona");
        let document = directory.path().join("character.json");
        fs::write(
            &document,
            r#"{
                "id": "orin",
                "player_name": "Orin",
                "class_path": "sorcerer",
                "home_scene": "reldens-town",
                "persona_file": "persona.md",
                "capabilities": ["speak", "walk", "purpose"]
            }"#,
        )
        .expect("write character sheet");

        let sheet = CharacterSheet::from_file(&document, &config).expect("load character sheet");

        assert_eq!(sheet.id, "orin");
        assert_eq!(sheet.class_path.as_deref(), Some("sorcerer"));
        assert!(sheet.persona.contains("firsthand"));
        assert_eq!(sheet.strategist_model, "provider/strategist");
        assert_eq!(sheet.tactician_model, "provider/tactician");
        assert_eq!(
            sheet.capabilities,
            HashSet::from([Capability::Speak, Capability::Walk, Capability::Purpose])
        );
    }
}
