use crate::character::Capability;

pub const OBSERVE: &str = "arena_observe";
pub const RENDER_MAP: &str = "arena_render_map";
pub const SURVEY: &str = "arena_survey";
pub const HISTORY: &str = "arena_history";
pub const PARTY_INVITE: &str = "arena_party_invite";
pub const PARTY_RESPOND: &str = "arena_party_respond";
pub const PARTY_LEAVE: &str = "arena_party_leave";
pub const SAY: &str = "arena_say";
pub const FEEL: &str = "arena_feel";
pub const PLAY_MELODY: &str = "arena_play_melody";
pub const TALK_TO: &str = "arena_talk_to";
pub const CHOOSE: &str = "arena_choose";
pub const END_TALK: &str = "arena_end_talk";
pub const THINK: &str = "arena_think";
pub const MOVE_TO: &str = "arena_move_to";
pub const MOVE: &str = "arena_move";
pub const CHECK_PATH: &str = "arena_check_path";
pub const STOP: &str = "arena_stop";
pub const UNSTICK: &str = "arena_unstick";
pub const ENTER_DOOR: &str = "arena_enter_door";
pub const BASIC_ATTACK: &str = "arena_basic_attack";
pub const USE_ACTION: &str = "arena_use_action";
pub const SET_TACTICS: &str = "arena_set_tactics";
pub const QUEUE_MATCH: &str = "arena_queue_match";
pub const MATCH_STATUS: &str = "arena_match_status";
pub const CREDIT_BALANCE: &str = "arena_credit_balance";
pub const CREDIT_HISTORY: &str = "arena_credit_history";
pub const INVENTORY: &str = "arena_inventory";
pub const USE_ITEM: &str = "arena_use_item";
pub const EQUIP: &str = "arena_equip";
pub const TRADE_WITH: &str = "arena_trade_with";
pub const BUY: &str = "arena_buy";
pub const SELL: &str = "arena_sell";
pub const PICK_UP: &str = "arena_pick_up";

/// Complete production MCP surface that this harness expects on 2026-08-11.
///
/// Keep session-only commands here as well. The live `tool-inventory` diagnostic
/// compares this list with `tools/list` and fails visibly when the server drifts.
pub const EXPECTED_PRODUCTION_TOOLS: &[&str] = &[
    "arena_register_agent",
    "arena_list_agents",
    "arena_login",
    "arena_disconnect",
    OBSERVE,
    RENDER_MAP,
    SURVEY,
    HISTORY,
    PARTY_INVITE,
    PARTY_RESPOND,
    PARTY_LEAVE,
    SAY,
    FEEL,
    PLAY_MELODY,
    TALK_TO,
    CHOOSE,
    END_TALK,
    THINK,
    MOVE_TO,
    MOVE,
    CHECK_PATH,
    STOP,
    UNSTICK,
    ENTER_DOOR,
    BASIC_ATTACK,
    USE_ACTION,
    SET_TACTICS,
    QUEUE_MATCH,
    MATCH_STATUS,
    CREDIT_BALANCE,
    CREDIT_HISTORY,
    INVENTORY,
    USE_ITEM,
    EQUIP,
    TRADE_WITH,
    BUY,
    SELL,
    PICK_UP,
];

pub fn required_capability(tool: &str) -> Option<Capability> {
    match tool {
        SAY | FEEL | PLAY_MELODY => Some(Capability::Speak),
        THINK => Some(Capability::Purpose),
        TALK_TO | CHOOSE | END_TALK | PARTY_INVITE | PARTY_RESPOND | PARTY_LEAVE => {
            Some(Capability::TalkToFolk)
        }
        MOVE_TO | MOVE | CHECK_PATH | STOP | UNSTICK => Some(Capability::Walk),
        ENTER_DOOR => Some(Capability::Doors),
        BASIC_ATTACK | USE_ACTION | SET_TACTICS => Some(Capability::Fight),
        QUEUE_MATCH | MATCH_STATUS => Some(Capability::Duel),
        CREDIT_BALANCE | CREDIT_HISTORY => Some(Capability::Money),
        INVENTORY | USE_ITEM | EQUIP | TRADE_WITH | BUY | SELL | PICK_UP => Some(Capability::Trade),
        _ => None,
    }
}
