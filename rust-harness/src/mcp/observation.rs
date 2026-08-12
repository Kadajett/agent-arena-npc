use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    pub own_player: Option<ObservedPlayer>,
    pub scene_name: Option<String>,
    pub recently_died: Option<bool>,
    pub battle: Option<ObservedBattle>,
    /// The class identity most recently reported by the game world.
    pub class_path: Option<ObservedClassPath>,
    /// The complete legal skill list reported by the game world.
    ///
    /// `None` means the world has not supplied the list. `Some(vec![])` means
    /// the world supplied an authoritative empty list.
    pub skills: Option<Vec<String>>,
    pub total_players: Option<u32>,
    pub total_objects: Option<u32>,
    #[serde(default)]
    pub players: Vec<ObservedPlayer>,
    #[serde(default)]
    pub chat: Vec<ChatLine>,
    #[serde(default)]
    pub recent_chat: Vec<ChatLine>,
    #[serde(default)]
    pub objects: Vec<ObservedObject>,
    #[serde(default)]
    pub carrying: Vec<ObservedItem>,
    #[serde(default)]
    pub drops: Vec<ObservedDrop>,
    pub party: Option<ObservedParty>,
    #[serde(default)]
    pub party_invites: Vec<ObservedPartyInvite>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedParty {
    pub party_id: Option<i64>,
    pub leader_name: Option<String>,
    #[serde(default)]
    pub members: Vec<ObservedPartyMember>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedPartyMember {
    #[serde(default, deserialize_with = "optional_i64_from_number_or_string")]
    pub player_id: Option<i64>,
    pub player_name: Option<String>,
    pub session_id: Option<String>,
    #[serde(default)]
    pub shared_properties: serde_json::Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedPartyInvite {
    #[serde(default, deserialize_with = "optional_i64_from_number_or_string")]
    pub from_player_id: Option<i64>,
    pub from_player_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedClassPath {
    pub key: Option<String>,
    pub label: Option<String>,
    pub level: Option<i32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedBattle {
    pub in_battle: Option<bool>,
    pub style: Option<String>,
    pub style_is_own_choice: Option<bool>,
    pub mode: Option<String>,
    pub hp: Option<String>,
    #[serde(default)]
    pub aggressors: Vec<ObservedAggressor>,
    #[serde(default)]
    pub enemy_health: Vec<ObservedEnemyHealth>,
    #[serde(default)]
    pub damage_dealt: Vec<ObservedDamageDealt>,
    #[serde(default)]
    pub events: Vec<ObservedBattleEvent>,
    #[serde(default)]
    pub recent_hits: Vec<ObservedRecentHit>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedAggressor {
    pub label: Option<String>,
    pub object_index: String,
    pub damage_dealt_to_you: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedEnemyHealth {
    pub object_index: String,
    pub label: Option<String>,
    pub hp: Option<i64>,
    pub max: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedDamageDealt {
    pub object_index: String,
    pub label: Option<String>,
    pub amount: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedBattleEvent {
    pub seq: Option<u64>,
    pub at: Option<String>,
    pub event_type: Option<String>,
    pub actor_id: Option<String>,
    pub target_id: Option<String>,
    pub amount: Option<i64>,
    pub label: Option<String>,
    #[serde(rename = "damageYouDealt")]
    pub damage_you_dealt: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedRecentHit {
    pub at: Option<String>,
    pub from: Option<String>,
    pub amount: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedPlayer {
    pub name: Option<String>,
    pub label: Option<String>,
    pub player_name: Option<String>,
    pub session_id: Option<String>,
    #[serde(default, deserialize_with = "optional_i64_from_number_or_string")]
    pub player_id: Option<i64>,
    pub state: Option<ObservedPlayerState>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedPlayerState {
    pub scene: Option<String>,
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub health: Option<i32>,
    pub max_health: Option<i32>,
    pub level: Option<i32>,
    pub experience: Option<i64>,
    pub class_path: Option<String>,
    pub alive: Option<bool>,
    pub combat_active: Option<bool>,
    pub current_target_id: Option<String>,
    pub moving: Option<bool>,
    #[serde(default)]
    pub combat_actions: Vec<ObservedCombatAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedCombatAction {
    pub id: String,
    pub available: Option<bool>,
    pub cooldown_remaining_ms: Option<u64>,
    pub target_kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatLine {
    pub channel: Option<String>,
    #[serde(rename = "type")]
    pub message_type: Option<u8>,
    pub from: Option<String>,
    pub message: Option<String>,
    pub received_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedObject {
    pub object_id: Option<i64>,
    pub object_index: String,
    pub label: String,
    pub kind: String,
    pub alive: Option<bool>,
    pub is_merchant: Option<bool>,
    pub interactable: Option<bool>,
    pub distance_from_self: Option<f32>,
    pub tile_x: i32,
    pub tile_y: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedItem {
    pub key: String,
    pub label: String,
    pub description: Option<String>,
    pub quantity: u32,
    pub usable: Option<bool>,
    pub equipment: Option<bool>,
    pub equipped: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedDrop {
    pub drop_id: String,
    pub item_key: Option<String>,
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub tile_x: Option<i32>,
    pub tile_y: Option<i32>,
    pub distance_from_self: Option<f32>,
}

fn optional_i64_from_number_or_string<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(number)) => number.as_i64().map(Some).ok_or_else(|| {
            serde::de::Error::custom("player id is outside the signed integer range")
        }),
        Some(serde_json::Value::String(text)) => text
            .parse::<i64>()
            .map(Some)
            .map_err(serde::de::Error::custom),
        Some(_) => Err(serde::de::Error::custom(
            "player id must be an integer or integer string",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_id_accepts_current_backend_string_representation() {
        let player: ObservedPlayer = serde_json::from_value(serde_json::json!({
            "playerId": "2080"
        }))
        .expect("player");
        assert_eq!(player.player_id, Some(2_080));
    }
}
