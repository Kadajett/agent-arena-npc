use agent_arena_npc_harness::mcp::{
    observation::Observation,
    types::{
        CombatResult, DialogueResult, HistoryPage, MapObservation, MoveResult, TradeListing,
        TradeResult,
    },
};

#[test]
fn current_completed_move_shape_distinguishes_acceptance_from_arrival() {
    let movement: MoveResult = serde_json::from_value(serde_json::json!({
        "arrived": false,
        "cameToRest": true,
        "x": 176.0,
        "y": 240.0,
        "tileX": 5,
        "tileY": 7,
        "asked": {"column": 9, "row": 7},
        "around": null
    }))
    .expect("current move response");

    assert_eq!(movement.accepted, None);
    assert_eq!(movement.arrived, Some(false));
    assert_eq!(movement.came_to_rest, Some(true));
    assert_eq!(movement.tile_x, Some(5));
    assert_eq!(movement.tile_y, Some(7));
}

#[test]
fn current_observation_fixture_deserializes_with_authoritative_optional_fields() {
    let observation: Observation =
        serde_json::from_str(include_str!("fixtures/mcp/observe-current.json"))
            .expect("observation contract");
    let battle = observation.battle.as_ref().expect("battle");
    assert_eq!(battle.style.as_deref(), Some("duck_and_weave"));
    assert_eq!(battle.aggressors[0].object_index, "spider-92");
    assert_eq!(battle.events[0].seq, Some(18));
    assert_eq!(observation.total_objects, Some(41));
    assert_eq!(
        observation
            .class_path
            .as_ref()
            .and_then(|path| path.label.as_deref()),
        Some("Swordsman")
    );
    assert_eq!(
        observation.skills.as_deref(),
        Some(["attackShort".to_owned(), "slash".to_owned()].as_slice())
    );
    let player = observation.own_player.expect("own player");
    let state = player.state.expect("player state");
    assert_eq!(state.health, Some(87));
    assert_eq!(state.max_health, Some(100));
    assert_eq!(state.class_path, None);
    assert!(state.combat_actions.is_empty());
    assert_eq!(observation.objects[0].object_index, "town_npc_42");
    assert_eq!(observation.objects[1].object_index, "spider-92");
    assert_eq!(observation.objects[1].alive, Some(true));
    assert_eq!(observation.carrying[0].quantity, 2);
    assert_eq!(observation.carrying[1].equipment, Some(true));
    assert_eq!(observation.carrying[1].equipped, Some(true));
}

#[test]
fn durable_history_preserves_typed_lineage_and_unknown_engine_fields() {
    let page: HistoryPage = serde_json::from_value(serde_json::json!({
        "generatedAt": "2026-08-12T05:00:00.000Z",
        "player": "Cassian Vey Unbound",
        "cursor": 102,
        "oldest": 101,
        "hasMore": false,
        "summary": {
            "eventCount": 2,
            "scenes": ["reldens-house-1"],
            "counts": {"engine_event": 1, "movement": 1},
            "combat": {"damageDealt": 0, "damageTaken": 0, "deaths": 0},
            "movement": {"commands": 1, "failed": 0},
            "items": {"bought": 0, "sold": 0, "used": 0, "pickedUp": 0},
            "progression": {"experienceGained": 0, "levelsGained": 0, "skillsLearned": 0}
        },
        "events": [{
            "id": 101,
            "at": "2026-08-12T04:59:59.000Z",
            "scene": "reldens-house-1",
            "category": "system",
            "event_type": "movement",
            "decisionId": "7b6f9885-4d61-4fc5-82ec-1cff060dbfb9",
            "tool": "arena_move_to",
            "success": true,
            "futureField": {"kept": true}
        }, {
            "id": 102,
            "event_type": "engine_event",
            "engine_key": "future.engine.event",
            "data": {"x": 1}
        }]
    }))
    .expect("durable history contract");

    assert_eq!(page.summary.movement.commands, 1);
    assert_eq!(page.events[0].tool.as_deref(), Some("arena_move_to"));
    assert!(page.events[0].decision_id.is_some());
    assert_eq!(page.events[0].fields["futureField"]["kept"], true);
    assert_eq!(page.events[1].fields["engine_key"], "future.engine.event");
    assert_eq!(page.events[1].data["x"], 1);
}

#[test]
fn observation_accepts_native_party_roster_and_pending_invites() {
    let observation: Observation = serde_json::from_value(serde_json::json!({
        "party": {
            "partyId": 10,
            "leaderName": "Alice",
            "members": [{
                "playerId": 10,
                "playerName": "Alice",
                "sessionId": "alice-session",
                "sharedProperties": {"level": 4}
            }]
        },
        "partyInvites": [{"fromPlayerId": 20, "fromPlayerName": "Bob"}]
    }))
    .expect("party observation contract");

    let party = observation.party.expect("party");
    assert_eq!(party.party_id, Some(10));
    assert_eq!(party.members[0].player_name.as_deref(), Some("Alice"));
    assert_eq!(observation.party_invites[0].from_player_id, Some(20));
}

#[test]
fn current_combat_acceptance_and_legacy_outcome_shapes_deserialize() {
    let accepted: CombatResult = serde_json::from_value(serde_json::json!({
        "accepted": true,
        "actionType": "attackShort",
        "targetKind": "object",
        "targetObjectIndex": "spider-92"
    }))
    .expect("current combat response");
    assert_eq!(accepted.accepted, Some(true));
    assert_eq!(accepted.action_type.as_deref(), Some("attackShort"));
    assert_eq!(accepted.target_kind.as_deref(), Some("object"));
    assert_eq!(accepted.target_object_index.as_deref(), Some("spider-92"));

    let legacy: CombatResult = serde_json::from_value(serde_json::json!({
        "attacked": true,
        "damage": 12,
        "targetKilled": false
    }))
    .expect("legacy combat response");
    assert_eq!(legacy.attacked, Some(true));
    assert_eq!(legacy.damage, Some(12));
    assert_eq!(legacy.target_killed, Some(false));
}

#[test]
fn current_map_fixture_deserializes_without_making_ascii_authoritative() {
    let map: MapObservation =
        serde_json::from_str(include_str!("fixtures/mcp/render-map-current.json"))
            .expect("map contract");
    assert_eq!(map.grid_available, Some(true));
    assert_eq!(map.scene_size.expect("scene size").width_tiles, Some(40));
    assert_eq!(map.doors[0].leads_to.as_deref(), Some("reldens-forest"));
    assert_eq!(map.doors[0].tile_x, Some(10));
    assert_eq!(map.doors[0].tile_y, Some(2));
    assert_eq!(map.doors[0].locked, Some(true));
    assert_eq!(map.doors[0].lock_known, Some(true));
    assert_eq!(
        map.doors[0].required_key.as_deref(),
        Some("forest-gate-key")
    );
}

#[test]
fn tool_group_success_and_refusal_shapes_deserialize() {
    let dialogue: DialogueResult = serde_json::from_value(serde_json::json!({
        "opened": true,
        "objectId": 42,
        "content": "Wolves are back.",
        "options": {"1": "Ask about the wolves"},
        "newServerField": "ignored"
    }))
    .expect("dialogue");
    assert_eq!(dialogue.object_id, Some(42));

    let listing: TradeListing = serde_json::from_value(serde_json::json!({
        "opened": true,
        "merchant": "Gimly",
        "side": "buy",
        "offers": [{
            "key": "tonic",
            "label": "Tonic",
            "price": {"itemKey": "coins", "quantity": 5}
        }]
    }))
    .expect("trade listing");
    assert_eq!(listing.offers[0].label, "Tonic");

    let refusal: TradeResult = serde_json::from_value(serde_json::json!({
        "traded": false,
        "reason": "MERCHANT_REFUSED",
        "message": "You cannot afford that."
    }))
    .expect("trade refusal");
    assert!(!refusal.traded);
    assert_eq!(refusal.reason.as_deref(), Some("MERCHANT_REFUSED"));
}
