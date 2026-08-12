use std::{collections::HashSet, sync::Arc, time::Instant};

use async_trait::async_trait;
use num_traits::ToPrimitive;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    character::Capability,
    execution::{
        gateway::{
            BodyCommand, BodyCommandResult, BodyGateway, BodyGatewayError, BodySpeechChannel,
            ExecutionContext,
        },
        packet::{TacticalMode as PacketTacticalMode, TacticalStyle as PacketTacticalStyle},
    },
    mcp::{
        observation::Observation,
        tools,
        transport::{McpError, McpTransport},
        types::{
            ChatChannel, CombatResult, CombatTarget, CreditBalance, CreditHistory, DialogueResult,
            DoorResult, EndDialogueResult, EquipResult, FeelResult, HistoryPage, HistoryQuery,
            InventoryResult, MapObservation, MatchResult, MelodyInstrument, MoveDirection,
            MoveResult, PartyActionResult, PathResult, PickupResult, PlayMelodyResult, SayResult,
            StopResult, SurveyResult, TacticsMode, TacticsResult, TacticsStyle, ThinkResult,
            TradeListing, TradeResult, TradeSide, UnstickResult, UseItemResult,
        },
    },
    observability::{AnalyticsEvent, AnalyticsSink, EventLevel},
    world::{PixelPosition, TilePosition, perception::TILE_SIZE_PIXELS},
};

#[derive(Clone)]
pub struct ArenaGateway {
    transport: Arc<dyn McpTransport>,
    agent_id: String,
    analytics_character_id: String,
    capabilities: HashSet<Capability>,
    analytics: Arc<dyn AnalyticsSink>,
}

#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("tool {tool} requires capability {capability:?}")]
    MissingCapability {
        tool: &'static str,
        capability: Capability,
    },
    #[error(transparent)]
    Mcp(#[from] McpError),
    #[error("MCP tool {tool} returned an incompatible response: {source}")]
    Decode {
        tool: String,
        source: serde_json::Error,
    },
    #[error("typed gateway generated invalid arguments for MCP tool {tool}")]
    InvalidArguments { tool: &'static str },
}

impl ArenaGateway {
    pub fn for_character(
        transport: Arc<dyn McpTransport>,
        agent_id: impl Into<String>,
        analytics_character_id: impl Into<String>,
        capabilities: HashSet<Capability>,
        analytics: Arc<dyn AnalyticsSink>,
    ) -> Self {
        Self {
            transport,
            agent_id: agent_id.into(),
            analytics_character_id: analytics_character_id.into(),
            capabilities,
            analytics,
        }
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// Read the bound character's authoritative observation.
    ///
    /// # Errors
    /// Returns a gateway error when MCP fails or the response is incompatible.
    pub async fn observe(&self) -> Result<Observation, GatewayError> {
        self.call(tools::OBSERVE, json!({}), true).await
    }

    /// Render the bound character's local room map.
    ///
    /// # Errors
    /// Returns a gateway error when MCP fails or the response is incompatible.
    pub async fn render_map(&self, radius: u32) -> Result<MapObservation, GatewayError> {
        let mut result: MapObservation = self
            .call(
                tools::RENDER_MAP,
                json!({ "level": "room", "radius": radius }),
                true,
            )
            .await?;
        result.requested_radius = Some(radius);
        Ok(result)
    }

    /// Survey the current scene and return the backend's complete legible listing.
    ///
    /// # Errors
    /// Returns a gateway error when MCP fails or the response is incompatible.
    pub async fn survey(&self, within: Option<u32>) -> Result<SurveyResult, GatewayError> {
        let mut arguments = Map::new();
        if let Some(within) = within {
            arguments.insert("within".to_owned(), json!(within));
        }
        self.call(tools::SURVEY, Value::Object(arguments), true)
            .await
    }

    /// Read one cursor-paged slice of the bound character's durable world history.
    ///
    /// This operation does not require a live body session. Unknown future event
    /// fields remain lossless at the external protocol boundary.
    ///
    /// # Errors
    /// Returns a gateway error when MCP fails or the response is incompatible.
    pub async fn history(&self, query: &HistoryQuery) -> Result<HistoryPage, GatewayError> {
        let correlation_id = Uuid::new_v4();
        let mut arguments = Map::new();
        for (name, value) in [("after", query.after), ("before", query.before)] {
            if let Some(value) = value {
                arguments.insert(name.to_owned(), json!(value));
            }
        }
        for (name, value) in [("since", &query.since), ("until", &query.until)] {
            if let Some(value) = value {
                arguments.insert(name.to_owned(), json!(value));
            }
        }
        if let Some(limit) = query.limit {
            arguments.insert("limit".to_owned(), json!(limit));
        }
        self.analytics.record(
            AnalyticsEvent::new("history.read_requested", EventLevel::Info)
                .character(&self.analytics_character_id)
                .correlation(correlation_id)
                .attribute("after_known", query.after.is_some())
                .attribute("before_known", query.before.is_some())
                .attribute(
                    "time_range_known",
                    query.since.is_some() || query.until.is_some(),
                )
                .attribute("requested_limit", u64::from(query.limit.unwrap_or(100))),
        );
        let result = self
            .call_correlated(
                tools::HISTORY,
                Value::Object(arguments),
                true,
                correlation_id,
            )
            .await;
        let event = AnalyticsEvent::new(
            if result.is_ok() {
                "history.read_completed"
            } else {
                "history.read_failed"
            },
            if result.is_ok() {
                EventLevel::Info
            } else {
                EventLevel::Warn
            },
        )
        .character(&self.analytics_character_id)
        .correlation(correlation_id);
        self.analytics
            .record(result.as_ref().map_or(event.clone(), |page: &HistoryPage| {
                event
                    .attribute("event_count", page.events.len())
                    .attribute("has_more", page.has_more)
                    .attribute("cursor", page.cursor)
                    .attribute("oldest", page.oldest)
            }));
        result
    }

    /// Invite one visible player by the backend's player ID.
    ///
    /// # Errors
    /// Returns a gateway error when social operations are forbidden or MCP fails.
    pub async fn party_invite(
        &self,
        target_player_id: i64,
    ) -> Result<PartyActionResult, GatewayError> {
        self.call(
            tools::PARTY_INVITE,
            json!({ "target_player_id": target_player_id }),
            true,
        )
        .await
    }

    /// Accept or reject one invitation reported by `arena_observe`.
    ///
    /// # Errors
    /// Returns a gateway error when social operations are forbidden or MCP fails.
    pub async fn party_respond(
        &self,
        from_player_id: i64,
        accept: bool,
    ) -> Result<PartyActionResult, GatewayError> {
        self.call(
            tools::PARTY_RESPOND,
            json!({ "from_player_id": from_player_id, "accept": accept }),
            true,
        )
        .await
    }

    /// Leave the current party or remove one member when this character leads it.
    ///
    /// # Errors
    /// Returns a gateway error when social operations are forbidden or MCP fails.
    pub async fn party_leave(
        &self,
        remove_player_id: Option<i64>,
    ) -> Result<PartyActionResult, GatewayError> {
        let mut arguments = Map::new();
        if let Some(remove_player_id) = remove_player_id {
            arguments.insert("remove_player_id".to_owned(), json!(remove_player_id));
        }
        self.call(tools::PARTY_LEAVE, Value::Object(arguments), true)
            .await
    }

    /// Speak one message as the bound character.
    ///
    /// # Errors
    /// Returns a gateway error when speech is forbidden or MCP fails.
    pub async fn say(&self, message: &str) -> Result<SayResult, GatewayError> {
        self.say_in(ChatChannel::Scene, message, None).await
    }

    /// Speak to every player in the world chat room.
    ///
    /// # Errors
    /// Returns a gateway error when speech is forbidden or MCP fails.
    pub async fn say_global(&self, message: &str) -> Result<SayResult, GatewayError> {
        self.say_in(ChatChannel::Global, message, None).await
    }

    /// Whisper to one named player through the world chat room.
    ///
    /// # Errors
    /// Returns a gateway error when speech is forbidden or MCP fails.
    pub async fn say_private(
        &self,
        to_player: &str,
        message: &str,
    ) -> Result<SayResult, GatewayError> {
        self.say_in(ChatChannel::Private, message, Some(to_player))
            .await
    }

    async fn say_in(
        &self,
        channel: ChatChannel,
        message: &str,
        to_player: Option<&str>,
    ) -> Result<SayResult, GatewayError> {
        self.say_in_correlated(channel, message, to_player, Uuid::new_v4())
            .await
    }

    async fn say_in_correlated(
        &self,
        channel: ChatChannel,
        message: &str,
        to_player: Option<&str>,
        correlation_id: Uuid,
    ) -> Result<SayResult, GatewayError> {
        let mut arguments = json!({ "message": message, "channel": channel });
        if let Some(to_player) = to_player {
            arguments
                .as_object_mut()
                .expect("typed chat arguments are an object")
                .insert("to_player".to_owned(), json!(to_player));
        }
        self.analytics.record(
            AnalyticsEvent::new("chat.send_requested", EventLevel::Info)
                .character(&self.analytics_character_id)
                .correlation(correlation_id)
                .attribute("channel", channel.as_str())
                .attribute("recipient_known", to_player.is_some())
                .attribute("message_character_count", message.chars().count()),
        );
        let result = self
            .call_correlated(tools::SAY, arguments, true, correlation_id)
            .await;
        self.analytics.record(
            AnalyticsEvent::new(
                if result.is_ok() {
                    "chat.send_completed"
                } else {
                    "chat.send_failed"
                },
                if result.is_ok() {
                    EventLevel::Info
                } else {
                    EventLevel::Warn
                },
            )
            .character(&self.analytics_character_id)
            .correlation(correlation_id)
            .attribute("channel", channel.as_str())
            .attribute("recipient_known", to_player.is_some()),
        );
        result
    }

    /// Perform one backend-validated melody in the current scene.
    ///
    /// # Errors
    /// Returns a gateway error when performance is forbidden or MCP fails.
    pub async fn play_melody(
        &self,
        melody: &str,
        times: u8,
        instrument: MelodyInstrument,
    ) -> Result<PlayMelodyResult, GatewayError> {
        let correlation_id = Uuid::new_v4();
        let note_count = melody.split_whitespace().count();
        self.analytics.record(
            AnalyticsEvent::new("music.performance_requested", EventLevel::Info)
                .character(&self.analytics_character_id)
                .correlation(correlation_id)
                .attribute("instrument", instrument.as_str())
                .attribute("times", times)
                .attribute("note_count", note_count)
                .attribute("melody_character_count", melody.chars().count()),
        );
        let result = self
            .call_correlated(
                tools::PLAY_MELODY,
                json!({
                    "melody": melody,
                    "times": times,
                    "instrument": instrument,
                }),
                true,
                correlation_id,
            )
            .await
            .map(|mut result: PlayMelodyResult| {
                // Production currently reports `played`, while an earlier draft
                // contract reported `accepted`. Keep one success fact for callers.
                if result.accepted.is_none() {
                    result.accepted = result.played;
                }
                if result.note_count.is_none() {
                    result.note_count = u8::try_from(note_count).ok();
                }
                result
            });
        self.analytics.record(
            AnalyticsEvent::new(
                if result.is_ok() {
                    "music.performance_completed"
                } else {
                    "music.performance_failed"
                },
                if result.is_ok() {
                    EventLevel::Info
                } else {
                    EventLevel::Warn
                },
            )
            .character(&self.analytics_character_id)
            .correlation(correlation_id)
            .attribute("instrument", instrument.as_str())
            .attribute("times", times)
            .attribute("note_count", note_count),
        );
        result
    }

    /// Publish the bound character's visible feeling.
    ///
    /// # Errors
    /// Returns a gateway error when speech is forbidden or MCP fails.
    pub async fn feel(&self, feeling: &str) -> Result<FeelResult, GatewayError> {
        self.call(tools::FEEL, json!({ "feeling": feeling }), true)
            .await
    }

    /// Open dialogue with a backend object id.
    ///
    /// # Errors
    /// Returns a gateway error when dialogue is forbidden or MCP fails.
    pub async fn talk_to(&self, object_id: i64) -> Result<DialogueResult, GatewayError> {
        self.call(tools::TALK_TO, json!({ "object_id": object_id }), true)
            .await
    }

    /// Choose one backend-reported dialogue option.
    ///
    /// # Errors
    /// Returns a gateway error when dialogue is forbidden or MCP fails.
    pub async fn choose(
        &self,
        object_id: i64,
        option_key: &str,
    ) -> Result<DialogueResult, GatewayError> {
        self.call(
            tools::CHOOSE,
            json!({ "object_id": object_id, "option_key": option_key }),
            true,
        )
        .await
    }

    /// End the current NPC dialogue.
    ///
    /// # Errors
    /// Returns a gateway error when dialogue is forbidden or MCP fails.
    pub async fn end_talk(&self, object_id: i64) -> Result<EndDialogueResult, GatewayError> {
        self.call(tools::END_TALK, json!({ "object_id": object_id }), true)
            .await
    }

    /// Record one private-to-the-world thought for spectators.
    ///
    /// # Errors
    /// Returns a gateway error when purpose is forbidden or MCP fails.
    pub async fn think(&self, thought: &str) -> Result<ThinkResult, GatewayError> {
        self.call(tools::THINK, json!({ "thought": thought }), true)
            .await
    }

    /// Begin movement toward an authoritative pixel position.
    ///
    /// # Errors
    /// Returns a gateway error when walking is forbidden or MCP fails.
    pub async fn move_to(&self, target: PixelPosition) -> Result<MoveResult, GatewayError> {
        self.call(
            tools::MOVE_TO,
            json!({ "x": target.x, "y": target.y }),
            true,
        )
        .await
    }

    /// Move in one cardinal direction.
    ///
    /// # Errors
    /// Returns a gateway error when walking is forbidden or MCP fails.
    pub async fn move_direction(
        &self,
        direction: MoveDirection,
    ) -> Result<MoveResult, GatewayError> {
        self.call(tools::MOVE, json!({ "direction": direction }), true)
            .await
    }

    /// Move in one cardinal direction for an explicit backend-bounded duration.
    ///
    /// # Errors
    /// Returns a gateway error when walking is forbidden or MCP fails.
    pub async fn move_direction_for(
        &self,
        direction: MoveDirection,
        duration_ms: u32,
    ) -> Result<MoveResult, GatewayError> {
        self.call(
            tools::MOVE,
            json!({ "direction": direction, "duration_ms": duration_ms }),
            true,
        )
        .await
    }

    /// Ask the backend whether a pixel position is reachable.
    ///
    /// # Errors
    /// Returns a gateway error when walking is forbidden or MCP fails.
    pub async fn check_path(&self, target: PixelPosition) -> Result<PathResult, GatewayError> {
        self.call(
            tools::CHECK_PATH,
            json!({ "x": target.x, "y": target.y }),
            true,
        )
        .await
    }

    /// Stop current movement.
    ///
    /// # Errors
    /// Returns a gateway error when walking is forbidden or MCP fails.
    pub async fn stop(&self) -> Result<StopResult, GatewayError> {
        self.call(tools::STOP, json!({}), true).await
    }

    /// Invoke the backend's last-resort unstick operation.
    ///
    /// # Errors
    /// Returns a gateway error when walking is forbidden or MCP fails.
    pub async fn unstick(&self) -> Result<UnstickResult, GatewayError> {
        self.call(tools::UNSTICK, json!({}), true).await
    }

    /// Enter the door at an authoritative pixel position.
    ///
    /// # Errors
    /// Returns a gateway error when doors are forbidden or MCP fails.
    pub async fn enter_door(&self, target: PixelPosition) -> Result<DoorResult, GatewayError> {
        self.call(
            tools::ENTER_DOOR,
            json!({ "x": target.x, "y": target.y }),
            true,
        )
        .await
    }

    /// Enter the door at an authoritative tile row and column.
    ///
    /// # Errors
    /// Returns a gateway error when doors are forbidden or MCP fails.
    pub async fn enter_door_tile(&self, target: TilePosition) -> Result<DoorResult, GatewayError> {
        self.call(
            tools::ENTER_DOOR,
            json!({ "row": target.y, "column": target.x }),
            true,
        )
        .await
    }

    /// Basic-attack one typed object or player target.
    ///
    /// # Errors
    /// Returns a gateway error when fighting is forbidden or MCP fails.
    pub async fn basic_attack(&self, target: &CombatTarget) -> Result<CombatResult, GatewayError> {
        self.call(
            tools::BASIC_ATTACK,
            Value::Object(target_arguments(target)),
            true,
        )
        .await
    }

    /// Use one backend-reported action against an optional typed target.
    ///
    /// # Errors
    /// Returns a gateway error when fighting is forbidden or MCP fails.
    pub async fn use_action(
        &self,
        action: &str,
        target: Option<&CombatTarget>,
    ) -> Result<CombatResult, GatewayError> {
        let mut arguments = target.map_or_else(Map::new, target_arguments);
        arguments.insert("action_type".to_owned(), json!(action));
        self.call(tools::USE_ACTION, Value::Object(arguments), true)
            .await
    }

    /// Select the backend's deterministic combat style or control mode.
    ///
    /// # Errors
    /// Returns a gateway error when fighting is forbidden or MCP fails.
    pub async fn set_tactics(
        &self,
        style: Option<TacticsStyle>,
        mode: Option<TacticsMode>,
    ) -> Result<TacticsResult, GatewayError> {
        let mut arguments = Map::new();
        if let Some(style) = style {
            arguments.insert("style".to_owned(), json!(style));
        }
        if let Some(mode) = mode {
            arguments.insert("mode".to_owned(), json!(mode));
        }
        self.call(tools::SET_TACTICS, Value::Object(arguments), true)
            .await
    }

    /// Queue for a duel in the current authoritative scene.
    ///
    /// # Errors
    /// Returns a gateway error when duelling is forbidden or MCP fails.
    pub async fn queue_match(&self, scene_name: &str) -> Result<MatchResult, GatewayError> {
        self.call(
            tools::QUEUE_MATCH,
            json!({ "scene_name": scene_name }),
            true,
        )
        .await
    }

    /// Read one known duel's status.
    ///
    /// # Errors
    /// Returns a gateway error when duelling is forbidden or MCP fails.
    pub async fn match_status(&self, match_id: &str) -> Result<MatchResult, GatewayError> {
        self.call(tools::MATCH_STATUS, json!({ "match_id": match_id }), false)
            .await
    }

    /// Read the bound character's credit balance.
    ///
    /// # Errors
    /// Returns a gateway error when money access is forbidden or MCP fails.
    pub async fn credit_balance(&self) -> Result<CreditBalance, GatewayError> {
        self.call(tools::CREDIT_BALANCE, json!({}), true).await
    }

    /// Read recent credit entries with an optional result limit.
    ///
    /// # Errors
    /// Returns a gateway error when money access is forbidden or MCP fails.
    pub async fn credit_history(&self, limit: Option<u32>) -> Result<CreditHistory, GatewayError> {
        let mut arguments = Map::new();
        if let Some(limit) = limit {
            arguments.insert("limit".to_owned(), json!(limit));
        }
        self.call(tools::CREDIT_HISTORY, Value::Object(arguments), true)
            .await
    }

    /// Read the bound character's inventory.
    ///
    /// # Errors
    /// Returns a gateway error when inventory access is forbidden or MCP fails.
    pub async fn inventory(&self) -> Result<InventoryResult, GatewayError> {
        self.call(tools::INVENTORY, json!({}), true).await
    }

    /// Use one backend item key from the bound inventory.
    ///
    /// # Errors
    /// Returns a gateway error when item use is forbidden or MCP fails.
    pub async fn use_item(&self, item: &str) -> Result<UseItemResult, GatewayError> {
        self.call(tools::USE_ITEM, json!({ "item": item }), true)
            .await
    }

    /// Equip or remove one backend-reported carried item.
    ///
    /// # Errors
    /// Returns a gateway error when equipment access is forbidden or MCP fails.
    pub async fn equip(&self, item: &str, take_off: bool) -> Result<EquipResult, GatewayError> {
        self.call(
            tools::EQUIP,
            json!({ "item": item, "take_off": take_off }),
            true,
        )
        .await
    }

    /// Read one merchant's buy or sell offers.
    ///
    /// # Errors
    /// Returns a gateway error when trade is forbidden or MCP fails.
    pub async fn trade_with(
        &self,
        object_id: i64,
        side: TradeSide,
    ) -> Result<TradeListing, GatewayError> {
        self.call(
            tools::TRADE_WITH,
            json!({ "object_id": object_id, "side": side }),
            true,
        )
        .await
    }

    /// Buy an item from one backend merchant object.
    ///
    /// # Errors
    /// Returns a gateway error when trade is forbidden or MCP fails.
    pub async fn buy(
        &self,
        object_id: i64,
        item: &str,
        quantity: u32,
    ) -> Result<TradeResult, GatewayError> {
        self.call(
            tools::BUY,
            json!({ "object_id": object_id, "item": item, "quantity": quantity }),
            true,
        )
        .await
    }

    /// Sell an item to one backend merchant object.
    ///
    /// # Errors
    /// Returns a gateway error when trade is forbidden or MCP fails.
    pub async fn sell(
        &self,
        object_id: i64,
        item: &str,
        quantity: u32,
    ) -> Result<TradeResult, GatewayError> {
        self.call(
            tools::SELL,
            json!({ "object_id": object_id, "item": item, "quantity": quantity }),
            true,
        )
        .await
    }

    /// Pick up one backend-reported drop.
    ///
    /// # Errors
    /// Returns a gateway error when pickup is forbidden or MCP fails.
    pub async fn pick_up(&self, drop_id: &str) -> Result<PickupResult, GatewayError> {
        self.call(tools::PICK_UP, json!({ "drop_id": drop_id }), true)
            .await
    }

    async fn call<T: DeserializeOwned>(
        &self,
        tool: &'static str,
        arguments: Value,
        inject_identity: bool,
    ) -> Result<T, GatewayError> {
        let correlation_id = Uuid::new_v4();
        self.call_correlated(tool, arguments, inject_identity, correlation_id)
            .await
    }

    async fn call_correlated<T: DeserializeOwned>(
        &self,
        tool: &'static str,
        mut arguments: Value,
        inject_identity: bool,
        correlation_id: Uuid,
    ) -> Result<T, GatewayError> {
        if let Some(capability) = tools::required_capability(tool)
            && !self.capabilities.contains(&capability)
        {
            self.analytics.record(
                AnalyticsEvent::new("mcp.tool_rejected", EventLevel::Warn)
                    .character(&self.analytics_character_id)
                    .correlation(correlation_id)
                    .attribute("tool", tool)
                    .attribute("reason", "missing_capability")
                    .attribute("capability", format!("{capability:?}")),
            );
            return Err(GatewayError::MissingCapability { tool, capability });
        }

        let Some(object) = arguments.as_object_mut() else {
            self.analytics.record(
                AnalyticsEvent::new("mcp.tool_rejected", EventLevel::Error)
                    .character(&self.analytics_character_id)
                    .correlation(correlation_id)
                    .attribute("tool", tool)
                    .attribute("reason", "invalid_typed_arguments"),
            );
            return Err(GatewayError::InvalidArguments { tool });
        };
        if inject_identity {
            object.insert("agent_id".to_owned(), json!(self.agent_id));
        }
        let argument_count = object.len();
        let started = Instant::now();
        self.analytics.record(
            AnalyticsEvent::new("mcp.tool_started", EventLevel::Debug)
                .character(&self.analytics_character_id)
                .correlation(correlation_id)
                .attribute("tool", tool)
                .attribute("argument_count", argument_count),
        );
        let value = match self
            .transport
            .call_tool(tool, arguments, correlation_id)
            .await
        {
            Ok(value) => value,
            Err(error) => {
                self.analytics.record(
                    AnalyticsEvent::new("mcp.tool_failed", EventLevel::Warn)
                        .character(&self.analytics_character_id)
                        .correlation(correlation_id)
                        .attribute("tool", tool)
                        .attribute("duration_ms", elapsed_ms(started))
                        .attribute("error_class", error.class()),
                );
                return Err(error.into());
            }
        };
        let decoded = serde_json::from_value(value).map_err(|source| GatewayError::Decode {
            tool: tool.to_owned(),
            source,
        });
        let decode_error = decoded.as_ref().err().and_then(|error| match error {
            GatewayError::Decode { source, .. } => Some(source),
            _ => None,
        });
        self.record_decode_result(tool, correlation_id, started, decode_error);
        decoded
    }

    fn record_decode_result(
        &self,
        tool: &'static str,
        correlation_id: Uuid,
        started: Instant,
        decode_error: Option<&serde_json::Error>,
    ) {
        let decode_diagnostics = decode_error.map(|source| {
            (
                format!("{:?}", source.classify()).to_lowercase(),
                source.line(),
                source.column(),
            )
        });
        self.analytics.record(
            AnalyticsEvent::new(
                if decode_error.is_none() {
                    "mcp.tool_completed"
                } else {
                    "mcp.tool_decode_failed"
                },
                if decode_error.is_none() {
                    EventLevel::Debug
                } else {
                    EventLevel::Warn
                },
            )
            .character(&self.analytics_character_id)
            .correlation(correlation_id)
            .attribute("tool", tool)
            .attribute("duration_ms", elapsed_ms(started))
            .attribute(
                "decode_category",
                decode_diagnostics
                    .as_ref()
                    .map_or("", |diagnostics| diagnostics.0.as_str()),
            )
            .attribute(
                "decode_line",
                u64::try_from(
                    decode_diagnostics
                        .as_ref()
                        .map_or(0, |diagnostics| diagnostics.1),
                )
                .unwrap_or(u64::MAX),
            )
            .attribute(
                "decode_column",
                u64::try_from(
                    decode_diagnostics
                        .as_ref()
                        .map_or(0, |diagnostics| diagnostics.2),
                )
                .unwrap_or(u64::MAX),
            ),
        );
    }
}

#[async_trait]
impl BodyGateway for ArenaGateway {
    #[allow(
        clippy::too_many_lines,
        reason = "one exhaustive command dispatcher keeps the mutation seam auditable"
    )]
    async fn execute(
        &self,
        command: BodyCommand,
        context: ExecutionContext,
    ) -> Result<BodyCommandResult, BodyGatewayError> {
        let correlation_id = context.action_id;
        let result = match command {
            BodyCommand::MoveDirection { direction } => {
                let result: MoveResult = self
                    .call_correlated(
                        tools::MOVE,
                        json!({ "direction": direction }),
                        true,
                        correlation_id,
                    )
                    .await
                    .map_err(body_gateway_error)?;
                BodyCommandResult {
                    accepted: result.accepted.or(result.moved),
                    moved: result.moved,
                    moving: result.moving,
                    arrived: result.arrived,
                    tile_x: result.tile_x,
                    tile_y: result.tile_y,
                    ..BodyCommandResult::default()
                }
            }
            BodyCommand::Think { thought } => {
                let result: ThinkResult = self
                    .call_correlated(
                        tools::THINK,
                        json!({ "thought": thought }),
                        true,
                        correlation_id,
                    )
                    .await
                    .map_err(body_gateway_error)?;
                BodyCommandResult {
                    accepted: result.recorded,
                    ..BodyCommandResult::default()
                }
            }
            BodyCommand::Say {
                message,
                channel,
                to_player,
            } => {
                let channel = match channel {
                    BodySpeechChannel::Scene => ChatChannel::Scene,
                    BodySpeechChannel::Global => ChatChannel::Global,
                    BodySpeechChannel::Private => ChatChannel::Private,
                };
                let result: SayResult = self
                    .say_in_correlated(channel, &message, to_player.as_deref(), correlation_id)
                    .await
                    .map_err(body_gateway_error)?;
                BodyCommandResult {
                    accepted: result.accepted.or(result.said),
                    ..BodyCommandResult::default()
                }
            }
            BodyCommand::TalkTo { object_id } => {
                let result: DialogueResult = self
                    .call_correlated(
                        tools::TALK_TO,
                        json!({ "object_id": object_id }),
                        true,
                        correlation_id,
                    )
                    .await
                    .map_err(body_gateway_error)?;
                BodyCommandResult {
                    accepted: Some(result.opened),
                    ..BodyCommandResult::default()
                }
            }
            BodyCommand::CheckPath { destination } => {
                let result: PathResult = self
                    .call_correlated(
                        tools::CHECK_PATH,
                        pixel_arguments(tile_center(destination)?),
                        true,
                        correlation_id,
                    )
                    .await
                    .map_err(body_gateway_error)?;
                BodyCommandResult {
                    accepted: result.reachable,
                    reachable: result.reachable,
                    path_length_tiles: result.path_length_tiles,
                    ..BodyCommandResult::default()
                }
            }
            BodyCommand::MoveTo { destination } => {
                let arguments = pixel_arguments(tile_center(destination)?);
                let path: PathResult = self
                    .call_correlated(tools::CHECK_PATH, arguments.clone(), true, correlation_id)
                    .await
                    .map_err(body_gateway_error)?;
                if path.reachable == Some(false) {
                    return Ok(BodyCommandResult {
                        accepted: Some(false),
                        reachable: Some(false),
                        path_length_tiles: path.path_length_tiles,
                        ..BodyCommandResult::default()
                    });
                }
                let result: MoveResult = self
                    .call_correlated(tools::MOVE_TO, arguments, true, correlation_id)
                    .await
                    .map_err(body_gateway_error)?;
                let position = result.x.zip(result.y).map(|(x, y)| PixelPosition { x, y });
                BodyCommandResult {
                    // A completed `arena_move_to` call is an accepted command even when the
                    // backend reports `arrived: false`: that value means the body came to rest
                    // short of its destination.  Only an explicit backend refusal overrides
                    // successful command delivery.
                    accepted: result.accepted.or(Some(true)),
                    reachable: path.reachable,
                    path_length_tiles: path.path_length_tiles,
                    moved: result.moved,
                    moving: result.moving,
                    arrived: result.arrived,
                    came_to_rest: result.came_to_rest,
                    tile_x: result.tile_x,
                    tile_y: result.tile_y,
                    position,
                    ..BodyCommandResult::default()
                }
            }
            BodyCommand::EnterDoor { destination } => {
                let result: DoorResult = self
                    .call_correlated(
                        tools::ENTER_DOOR,
                        json!({ "row": destination.y, "column": destination.x }),
                        true,
                        correlation_id,
                    )
                    .await
                    .map_err(body_gateway_error)?;
                BodyCommandResult {
                    accepted: result.entered,
                    moved: result.entered,
                    arrived: result.entered,
                    came_to_rest: result.entered,
                    ..BodyCommandResult::default()
                }
            }
            BodyCommand::QueueDuel { scene_name } => {
                self.queue_match(&scene_name)
                    .await
                    .map_err(body_gateway_error)?;
                BodyCommandResult {
                    accepted: Some(true),
                    ..BodyCommandResult::default()
                }
            }
            BodyCommand::Stop => {
                let result: StopResult = self
                    .call_correlated(tools::STOP, json!({}), true, correlation_id)
                    .await
                    .map_err(body_gateway_error)?;
                BodyCommandResult {
                    accepted: result.stopped,
                    stopped: result.stopped,
                    ..BodyCommandResult::default()
                }
            }
            BodyCommand::Attack {
                target_object_index,
            } => {
                let result: CombatResult = self
                    .call_correlated(
                        tools::BASIC_ATTACK,
                        Value::Object(target_arguments(&CombatTarget::Object {
                            object_index: target_object_index,
                        })),
                        true,
                        correlation_id,
                    )
                    .await
                    .map_err(body_gateway_error)?;
                BodyCommandResult {
                    accepted: result.accepted.or(result.attacked),
                    ..BodyCommandResult::default()
                }
            }
            BodyCommand::UseSkill {
                skill_id,
                target_object_index,
            } => {
                let target =
                    target_object_index.map(|object_index| CombatTarget::Object { object_index });
                let mut arguments = target.as_ref().map_or_else(Map::new, target_arguments);
                arguments.insert("action_type".to_owned(), json!(skill_id));
                let result: CombatResult = self
                    .call_correlated(
                        tools::USE_ACTION,
                        Value::Object(arguments),
                        true,
                        correlation_id,
                    )
                    .await
                    .map_err(body_gateway_error)?;
                BodyCommandResult {
                    accepted: result.accepted.or(result.attacked),
                    ..BodyCommandResult::default()
                }
            }
            BodyCommand::UseItem { item_id } => {
                let result: UseItemResult = self
                    .call_correlated(
                        tools::USE_ITEM,
                        json!({ "item": item_id }),
                        true,
                        correlation_id,
                    )
                    .await
                    .map_err(body_gateway_error)?;
                BodyCommandResult {
                    accepted: result.used,
                    ..BodyCommandResult::default()
                }
            }
            BodyCommand::PickUp { drop_id } => {
                let result: PickupResult = self
                    .call_correlated(
                        tools::PICK_UP,
                        json!({ "drop_id": drop_id }),
                        true,
                        correlation_id,
                    )
                    .await
                    .map_err(body_gateway_error)?;
                BodyCommandResult {
                    accepted: result.picked_up,
                    ..BodyCommandResult::default()
                }
            }
            BodyCommand::SetTactics { style, mode } => {
                let mut arguments = Map::new();
                arguments.insert("style".to_owned(), json!(style));
                arguments.insert("mode".to_owned(), json!(mode));
                let result: TacticsResult = self
                    .call_correlated(
                        tools::SET_TACTICS,
                        Value::Object(arguments),
                        true,
                        correlation_id,
                    )
                    .await
                    .map_err(body_gateway_error)?;
                BodyCommandResult {
                    accepted: Some(tactics_result_matches(&result, style, mode)),
                    ..BodyCommandResult::default()
                }
            }
        };
        Ok(result)
    }
}

fn tactics_result_matches(
    result: &TacticsResult,
    style: PacketTacticalStyle,
    mode: PacketTacticalMode,
) -> bool {
    let expected_style = match style {
        PacketTacticalStyle::CloseUp => "close_up",
        PacketTacticalStyle::LongRange => "long_range",
        PacketTacticalStyle::DuckAndWeave => "duck_and_weave",
        PacketTacticalStyle::Flee => "flee",
    };
    let style_matches = result.style.as_deref() == Some(expected_style);
    let expected = match mode {
        PacketTacticalMode::SemiAuto => "semi_auto",
        PacketTacticalMode::Manual => "manual",
    };
    let mode_matches = result.mode.as_deref() == Some(expected);
    style_matches && mode_matches
}

fn tile_center(destination: crate::world::TilePosition) -> Result<PixelPosition, BodyGatewayError> {
    let x = destination.x.to_f32().ok_or_else(|| BodyGatewayError {
        class: "invalid_coordinate".to_owned(),
    })?;
    let y = destination.y.to_f32().ok_or_else(|| BodyGatewayError {
        class: "invalid_coordinate".to_owned(),
    })?;
    Ok(PixelPosition {
        x: x.mul_add(TILE_SIZE_PIXELS, TILE_SIZE_PIXELS / 2.0),
        y: y.mul_add(TILE_SIZE_PIXELS, TILE_SIZE_PIXELS / 2.0),
    })
}

fn pixel_arguments(target: PixelPosition) -> Value {
    json!({ "x": target.x, "y": target.y })
}

fn body_gateway_error(error: GatewayError) -> BodyGatewayError {
    let class = match error {
        GatewayError::MissingCapability { .. } => "missing_capability",
        GatewayError::Mcp(error) => error.class(),
        GatewayError::Decode { .. } => "decode",
        GatewayError::InvalidArguments { .. } => "invalid_arguments",
    };
    BodyGatewayError {
        class: class.to_owned(),
    }
}

fn target_arguments(target: &CombatTarget) -> Map<String, Value> {
    match target {
        CombatTarget::Object { object_index } => {
            Map::from_iter([("target_object_index".to_owned(), json!(object_index))])
        }
        CombatTarget::Player {
            session_id,
            player_id,
        } => Map::from_iter([
            ("target_session_id".to_owned(), json!(session_id)),
            ("target_player_id".to_owned(), json!(player_id)),
        ]),
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use crate::observability::RecordingAnalyticsSink;

    use super::*;

    #[derive(Default)]
    struct RecordingTransport {
        calls: Mutex<Vec<(String, Value)>>,
    }

    #[async_trait]
    impl McpTransport for RecordingTransport {
        async fn request(
            &self,
            method: &str,
            params: Value,
            _correlation_id: Uuid,
        ) -> Result<Value, McpError> {
            let tool = params["name"].as_str().unwrap_or_default().to_owned();
            self.calls
                .lock()
                .expect("recording lock")
                .push((method.to_owned(), params));
            let body = match tool.as_str() {
                tools::TALK_TO | tools::CHOOSE | tools::TRADE_WITH => json!({"opened": false}),
                tools::SURVEY => json!({"survey": ""}),
                tools::HISTORY => json!({
                    "player": "Guy",
                    "cursor": 0,
                    "oldest": 0,
                    "hasMore": false,
                    "summary": {},
                    "events": []
                }),
                tools::QUEUE_MATCH | tools::MATCH_STATUS => json!({"status": "queued"}),
                tools::CREDIT_BALANCE => json!({"balance": 0}),
                tools::BUY | tools::SELL => json!({"traded": false}),
                tools::CHECK_PATH => json!({"reachable": true, "pathLengthTiles": 3}),
                tools::MOVE_TO => json!({
                    "arrived": false,
                    "cameToRest": true,
                    "x": 48.0,
                    "y": 80.0,
                    "tileX": 1,
                    "tileY": 2
                }),
                tools::SAY => json!({"accepted": true}),
                _ => json!({}),
            };
            Ok(json!({
                "content": [{"type": "text", "text": body.to_string()}]
            }))
        }

        async fn notify(
            &self,
            _method: &str,
            _params: Value,
            _correlation_id: Uuid,
        ) -> Result<(), McpError> {
            Ok(())
        }

        async fn reset_session(&self) {}

        async fn session_id(&self) -> Option<String> {
            None
        }
    }

    fn gateway(
        transport: Arc<RecordingTransport>,
        capabilities: HashSet<Capability>,
    ) -> (ArenaGateway, Arc<RecordingAnalyticsSink>) {
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        (
            ArenaGateway::for_character(
                transport,
                "guy-agent-id",
                "guy",
                capabilities,
                analytics.clone(),
            ),
            analytics,
        )
    }

    #[tokio::test]
    async fn identity_is_injected_and_never_supplied_by_callers() {
        let transport = Arc::new(RecordingTransport::default());
        let (gateway, _) = gateway(transport.clone(), HashSet::from([Capability::Walk]));

        gateway.stop().await.expect("stop call");

        let calls = transport.calls.lock().expect("recording lock");
        assert_eq!(calls[0].0, "tools/call");
        assert_eq!(calls[0].1["arguments"]["agent_id"], "guy-agent-id");
    }

    #[tokio::test]
    async fn body_move_preflights_path_and_preserves_action_correlation() {
        let transport = Arc::new(RecordingTransport::default());
        let (gateway, analytics) = gateway(transport.clone(), HashSet::from([Capability::Walk]));
        let action_id = Uuid::new_v4();
        let result = BodyGateway::execute(
            &gateway,
            BodyCommand::MoveTo {
                destination: crate::world::TilePosition { x: 2, y: 3 },
            },
            ExecutionContext {
                session_generation: 9,
                decision_id: Uuid::new_v4(),
                packet_id: Uuid::new_v4(),
                action_id,
                action_index: 0,
                frame_revision: 12,
                strategic_revision: 4,
            },
        )
        .await
        .expect("body move");

        assert_eq!(result.accepted, Some(true));
        assert_eq!(result.reachable, Some(true));
        assert_eq!(result.arrived, Some(false));
        assert_eq!(result.came_to_rest, Some(true));
        let calls = transport.calls.lock().expect("calls");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].1["name"], tools::CHECK_PATH);
        assert_eq!(calls[1].1["name"], tools::MOVE_TO);
        assert_eq!(calls[0].1["arguments"]["x"], 80.0);
        assert_eq!(calls[0].1["arguments"]["y"], 112.0);
        drop(calls);
        let correlated = analytics
            .events()
            .into_iter()
            .filter(|event| event.correlation_id == Some(action_id))
            .collect::<Vec<_>>();
        assert_eq!(correlated.len(), 4);
    }

    #[tokio::test]
    async fn capabilities_reject_before_transport_and_emit_analytics() {
        let transport = Arc::new(RecordingTransport::default());
        let (gateway, analytics) = gateway(transport.clone(), HashSet::new());

        let error = gateway
            .move_to(PixelPosition { x: 2.0, y: 3.0 })
            .await
            .expect_err("walk should be refused");
        assert!(matches!(
            error,
            GatewayError::MissingCapability {
                capability: Capability::Walk,
                ..
            }
        ));
        assert!(transport.calls.lock().expect("calls").is_empty());
        assert_eq!(analytics.events()[0].name, "mcp.tool_rejected");
    }

    #[tokio::test]
    async fn typed_chat_channels_use_the_mcp_contract_without_logging_message_text() {
        let transport = Arc::new(RecordingTransport::default());
        let (gateway, analytics) = gateway(transport.clone(), HashSet::from([Capability::Speak]));

        gateway.say("nearby words").await.expect("scene chat");
        gateway
            .say_global("world words")
            .await
            .expect("global chat");
        gateway
            .say_private("Cassian Vey Unbound", "private words")
            .await
            .expect("private chat");

        let calls = transport.calls.lock().expect("calls");
        assert_eq!(calls[0].1["arguments"]["channel"], "scene");
        assert_eq!(calls[1].1["arguments"]["channel"], "global");
        assert_eq!(calls[2].1["arguments"]["channel"], "private");
        assert_eq!(calls[2].1["arguments"]["to_player"], "Cassian Vey Unbound");
        drop(calls);

        let encoded_events = serde_json::to_string(&analytics.events()).expect("events serialize");
        assert!(!encoded_events.contains("nearby words"));
        assert!(!encoded_events.contains("world words"));
        assert!(!encoded_events.contains("private words"));
        assert!(analytics.events().iter().any(|event| {
            event.name == "chat.send_completed" && event.attributes["channel"] == "private"
        }));
    }

    #[tokio::test]
    async fn body_mediated_speech_keeps_action_lineage_and_content_safe_analytics() {
        let transport = Arc::new(RecordingTransport::default());
        let (gateway, analytics) = gateway(transport.clone(), HashSet::from([Capability::Speak]));
        let action_id = Uuid::new_v4();
        let result = BodyGateway::execute(
            &gateway,
            BodyCommand::Say {
                message: "Do not record this dialogue".to_owned(),
                channel: BodySpeechChannel::Private,
                to_player: Some("Orin".to_owned()),
            },
            ExecutionContext {
                session_generation: 3,
                decision_id: Uuid::new_v4(),
                packet_id: Uuid::new_v4(),
                action_id,
                action_index: 0,
                frame_revision: 22,
                strategic_revision: 9,
            },
        )
        .await
        .expect("body speech");

        assert_eq!(result.accepted, Some(true));
        let calls = transport.calls.lock().expect("calls");
        assert_eq!(calls[0].1["name"], tools::SAY);
        assert_eq!(calls[0].1["arguments"]["channel"], "private");
        assert_eq!(calls[0].1["arguments"]["to_player"], "Orin");
        drop(calls);
        let events = analytics.events();
        assert!(events.iter().any(|event| {
            event.name == "chat.send_completed" && event.correlation_id == Some(action_id)
        }));
        assert!(
            !serde_json::to_string(&events)
                .expect("events serialize")
                .contains("Do not record this dialogue")
        );
    }

    #[tokio::test]
    async fn melody_is_typed_capability_gated_and_payload_safe_in_analytics() {
        let transport = Arc::new(RecordingTransport::default());
        let (gateway, analytics) = gateway(transport.clone(), HashSet::from([Capability::Speak]));

        gateway
            .play_melody("C E G C5", 2, MelodyInstrument::Lute)
            .await
            .expect("melody");

        let calls = transport.calls.lock().expect("calls");
        assert_eq!(calls[0].1["name"], tools::PLAY_MELODY);
        assert_eq!(calls[0].1["arguments"]["melody"], "C E G C5");
        assert_eq!(calls[0].1["arguments"]["times"], 2);
        assert_eq!(calls[0].1["arguments"]["instrument"], "lute");
        drop(calls);
        let encoded_events = serde_json::to_string(&analytics.events()).expect("events serialize");
        assert!(!encoded_events.contains("C E G C5"));
        assert!(analytics.events().iter().any(|event| {
            event.name == "music.performance_completed"
                && event.attributes["instrument"] == "lute"
                && event.attributes["note_count"] == 4
        }));
    }

    #[tokio::test]
    async fn player_target_uses_player_ids_without_object_id() {
        let transport = Arc::new(RecordingTransport::default());
        let (gateway, _) = gateway(transport.clone(), HashSet::from([Capability::Fight]));
        gateway
            .basic_attack(&CombatTarget::Player {
                session_id: "session-ash".to_owned(),
                player_id: 20,
            })
            .await
            .expect("attack");

        let calls = transport.calls.lock().expect("calls");
        let arguments = &calls[0].1["arguments"];
        assert_eq!(arguments["target_session_id"], "session-ash");
        assert_eq!(arguments["target_player_id"], 20);
        assert!(arguments.get("target_object_index").is_none());
    }

    #[tokio::test]
    async fn door_tile_uses_backend_row_and_column_contract() {
        let transport = Arc::new(RecordingTransport::default());
        let (gateway, _) = gateway(transport.clone(), HashSet::from([Capability::Doors]));

        gateway
            .enter_door_tile(TilePosition { x: 17, y: 29 })
            .await
            .expect("enter door");

        let calls = transport.calls.lock().expect("calls");
        assert_eq!(calls[0].1["name"], tools::ENTER_DOOR);
        assert_eq!(calls[0].1["arguments"]["row"], 29);
        assert_eq!(calls[0].1["arguments"]["column"], 17);
        assert!(calls[0].1["arguments"].get("x").is_none());
        assert!(calls[0].1["arguments"].get("y").is_none());
    }

    #[tokio::test]
    #[allow(
        clippy::too_many_lines,
        reason = "one exhaustive contract test keeps the complete typed production tool surface auditable"
    )]
    async fn complete_tool_surface_uses_current_contract_argument_names() {
        let transport = Arc::new(RecordingTransport::default());
        let capabilities = HashSet::from([
            Capability::Speak,
            Capability::TalkToFolk,
            Capability::Walk,
            Capability::Doors,
            Capability::Fight,
            Capability::Duel,
            Capability::Money,
            Capability::Trade,
            Capability::Purpose,
        ]);
        let (gateway, _) = gateway(transport.clone(), capabilities);

        gateway.observe().await.expect("observe");
        gateway.render_map(16).await.expect("map");
        gateway.survey(None).await.expect("survey");
        gateway
            .history(&HistoryQuery {
                after: Some(40),
                limit: Some(25),
                ..HistoryQuery::default()
            })
            .await
            .expect("world history");
        gateway.party_invite(20).await.expect("party invite");
        gateway
            .party_respond(20, true)
            .await
            .expect("party response");
        gateway.party_leave(None).await.expect("party leave");
        gateway.say("hello").await.expect("say");
        gateway.feel("wary").await.expect("feel");
        gateway.talk_to(42).await.expect("talk");
        gateway.choose(42, "1").await.expect("choose");
        gateway.end_talk(42).await.expect("end talk");
        gateway.think("I am testing.").await.expect("think");
        gateway
            .move_to(PixelPosition { x: 32.0, y: 64.0 })
            .await
            .expect("move to");
        gateway
            .move_direction(MoveDirection::Left)
            .await
            .expect("move");
        gateway
            .check_path(PixelPosition { x: 96.0, y: 128.0 })
            .await
            .expect("path");
        gateway.stop().await.expect("stop");
        gateway.unstick().await.expect("unstick");
        gateway
            .enter_door(PixelPosition { x: 8.0, y: 9.0 })
            .await
            .expect("door");
        let enemy = CombatTarget::Object {
            object_index: "spider_9".to_owned(),
        };
        gateway.basic_attack(&enemy).await.expect("attack");
        gateway
            .use_action("slash", Some(&enemy))
            .await
            .expect("skill");
        gateway
            .set_tactics(
                Some(TacticsStyle::DuckAndWeave),
                Some(TacticsMode::SemiAuto),
            )
            .await
            .expect("tactics");
        gateway.queue_match("arena-volcano").await.expect("queue");
        gateway.match_status("match-1").await.expect("status");
        gateway.credit_balance().await.expect("balance");
        gateway.credit_history(Some(20)).await.expect("history");
        gateway.inventory().await.expect("inventory");
        gateway.use_item("tonic").await.expect("item");
        gateway.equip("sword", false).await.expect("equip");
        gateway
            .trade_with(10, TradeSide::Buy)
            .await
            .expect("trade listing");
        gateway.buy(10, "tonic", 2).await.expect("buy");
        gateway.sell(10, "pelt", 1).await.expect("sell");
        gateway.pick_up("drop-1").await.expect("pickup");

        let calls = transport.calls.lock().expect("calls");
        let call = |index: usize| (&calls[index].1["name"], &calls[index].1["arguments"]);
        assert_eq!(call(0).0, tools::OBSERVE);
        assert_eq!(call(0).1, &json!({"agent_id": "guy-agent-id"}));
        assert_eq!(call(1).0, tools::RENDER_MAP);
        assert_eq!(
            call(1).1,
            &json!({
                "agent_id": "guy-agent-id",
                "level": "room",
                "radius": 16
            })
        );
        assert_eq!(call(2).0, tools::SURVEY);
        assert_eq!(call(3).0, tools::HISTORY);
        assert_eq!(call(3).1["after"], 40);
        assert_eq!(call(3).1["limit"], 25);
        assert_eq!(call(4).1["target_player_id"], 20);
        assert_eq!(call(5).1["from_player_id"], 20);
        assert_eq!(call(5).1["accept"], true);
        assert_eq!(call(6).1, &json!({"agent_id": "guy-agent-id"}));
        assert_eq!(call(9).1["object_id"], 42);
        assert_eq!(call(10).1["option_key"], "1");
        assert_eq!(call(11).1["object_id"], 42);
        assert_eq!(call(12).1["thought"], "I am testing.");
        assert_eq!(call(13).1["x"], 32.0);
        assert_eq!(call(14).1["direction"], "left");
        assert_eq!(call(19).1["target_object_index"], "spider_9");
        assert_eq!(call(20).1["action_type"], "slash");
        assert_eq!(call(21).1["style"], "duck_and_weave");
        assert_eq!(call(21).1["mode"], "semi_auto");
        assert_eq!(call(22).1["scene_name"], "arena-volcano");
        assert_eq!(call(23).1, &json!({"match_id": "match-1"}));
        assert_eq!(call(25).1["limit"], 20);
        assert_eq!(call(28).1["item"], "sword");
        assert_eq!(call(29).1["side"], "buy");
        assert_eq!(call(30).1["quantity"], 2);
        assert_eq!(call(32).1["drop_id"], "drop-1");
    }
}
