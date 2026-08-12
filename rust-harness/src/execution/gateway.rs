use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    execution::packet::{TacticalMode, TacticalStyle},
    mcp::types::MoveDirection,
    world::{PixelPosition, TilePosition},
};

/// Runtime-owned causal identity for one physical action attempt.
///
/// The action ID is also the MCP correlation ID. This makes the packet-to-MCP
/// chain reconstructable without copying model or tool payloads into telemetry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionContext {
    pub session_generation: u64,
    pub decision_id: Uuid,
    pub packet_id: Uuid,
    pub action_id: Uuid,
    pub action_index: usize,
    pub frame_revision: u64,
    pub strategic_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BodyCommand {
    MoveDirection {
        direction: MoveDirection,
    },
    Think {
        thought: String,
    },
    Say {
        message: String,
        channel: BodySpeechChannel,
        to_player: Option<String>,
    },
    TalkTo {
        object_id: i64,
    },
    CheckPath {
        destination: TilePosition,
    },
    MoveTo {
        destination: TilePosition,
    },
    EnterDoor {
        destination: TilePosition,
    },
    QueueDuel {
        scene_name: String,
    },
    Stop,
    Attack {
        target_object_index: String,
    },
    UseSkill {
        skill_id: String,
        target_object_index: Option<String>,
    },
    UseItem {
        item_id: String,
    },
    PickUp {
        drop_id: String,
    },
    SetTactics {
        style: TacticalStyle,
        mode: TacticalMode,
    },
}

impl BodyCommand {
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::MoveDirection { .. } => "move",
            Self::Think { .. } => "think",
            Self::Say { .. } => "say",
            Self::TalkTo { .. } => "talk_to",
            Self::CheckPath { .. } => "check_path",
            Self::MoveTo { .. } => "move_to",
            Self::EnterDoor { .. } => "enter_door",
            Self::QueueDuel { .. } => "queue_duel",
            Self::Stop => "stop",
            Self::Attack { .. } => "attack",
            Self::UseSkill { .. } => "use_skill",
            Self::UseItem { .. } => "use_item",
            Self::PickUp { .. } => "pick_up",
            Self::SetTactics { .. } => "set_tactics",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodySpeechChannel {
    Scene,
    Global,
    Private,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BodyCommandResult {
    pub accepted: Option<bool>,
    pub reachable: Option<bool>,
    pub path_length_tiles: Option<u32>,
    pub moved: Option<bool>,
    pub moving: Option<bool>,
    pub arrived: Option<bool>,
    pub tile_x: Option<i32>,
    pub tile_y: Option<i32>,
    pub came_to_rest: Option<bool>,
    pub stopped: Option<bool>,
    pub position: Option<PixelPosition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("body gateway operation failed with class {class}")]
pub struct BodyGatewayError {
    pub class: String,
}

/// The one mutation seam used by the `BodyActor`.
///
/// Production uses the character-bound MCP gateway. Tests use a recording
/// adapter. Callers provide causal identity but never provide an Arena agent ID.
#[async_trait]
pub trait BodyGateway: Send + Sync {
    async fn execute(
        &self,
        command: BodyCommand,
        context: ExecutionContext,
    ) -> Result<BodyCommandResult, BodyGatewayError>;
}

#[derive(Debug, Default)]
pub struct DisabledBodyGateway;

#[async_trait]
impl BodyGateway for DisabledBodyGateway {
    async fn execute(
        &self,
        _command: BodyCommand,
        _context: ExecutionContext,
    ) -> Result<BodyCommandResult, BodyGatewayError> {
        Err(BodyGatewayError {
            class: "body_gateway_disabled".to_owned(),
        })
    }
}
