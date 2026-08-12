# Phase 02: Typed MCP and Session Layer

State: In progress

The local implementation and contract tests pass. Registration, login, observation, survey, map, durable history, inventory, speech, music, party contracts, and map-guided movement pass on the live server. A live inventory check confirms that all 38 advertised production tools have typed coverage. The controlled live attack check remains.

## Goal

Build one typed Rust adapter for the complete MCP surface that the current harness uses. Keep raw JSON-RPC data inside the adapter.

## Required result

Guy can initialize an MCP session, register or find his character, log in, read state, perform each permitted operation, disconnect, and reconnect through typed Rust methods.

## Dependencies

Phase 01 must supply a stable player runtime, character sheet, capability set, and shutdown path.

Captured MCP responses from the current harness must be available for contract tests.

## Transport tasks

- [x] Implement the HTTP MCP transport.
- [x] Send the required protocol version header.
- [x] Send the Arena bearer token.
- [x] Store and resend the MCP session ID.
- [x] Support JSON responses.
- [x] Support Server-Sent Event responses.
- [x] Stop SSE reading after the matching JSON-RPC response event.
- [x] Apply request timeouts.
- [x] Classify timeout, transport, HTTP, protocol, JSON-RPC, tool, and decode failures.
- [x] Redact secrets from logs and errors.

## Session tasks

- [x] Send MCP `initialize`.
- [x] Send `notifications/initialized`.
- [x] List registered agents.
- [x] Register the selected character when required.
- [x] Use an idempotency key for registration.
- [x] Log in with the bound character ID.
- [x] Disconnect cleanly.
- [x] Classify an expired or lost session.
- [x] Reconnect with bounded backoff.
- [x] Notify the runtime after reconnect.

Keep these MCP tools in session plumbing and out of model decisions:

- [x] `arena_login`
- [x] `arena_register_agent`
- [x] `arena_disconnect`
- [x] `arena_list_agents`
- [x] Confirm that `arena_create_watch_code` no longer exists. Watching is public in the current backend.

Session plumbing must remain outside model decisions.

## Typed tool surface

Implement typed methods for these operations:

### Observation

- [x] `arena_observe`
- [x] `arena_render_map`
- [x] `arena_survey`
- [x] `arena_history`

`arena_history` uses cursor or time-range paging and does not require a live body session. The typed response exposes stable common fields and preserves unknown engine-event fields losslessly at the external MCP boundary. Event payloads do not enter actor messages directly.

### Native parties

- [x] `arena_party_invite`
- [x] `arena_party_respond`
- [x] `arena_party_leave`
- [x] Read party rosters and pending invitations from `arena_observe`.

Party mutations require `TalkToFolk` until the character capability vocabulary gains a dedicated party capability. Models still cannot call these gateway methods directly.

### Speech and status

- [x] `arena_say`
- [x] Send scene chat with a typed channel.
- [x] Send global chat through the shared world chat room.
- [x] Send private chat to one named player.
- [x] Preserve received scene, global, private, and team channel provenance.
- [x] `arena_feel`
- [x] `arena_play_melody`
- [x] `arena_think`

`arena_play_melody` accepts one through 24 note tokens, one through four repetitions, and the `lute`, `flute`, `horn`, or `bell` instrument. The melody is scene-visible content. Telemetry records its instrument, repetition count, note count, and character count. It does not record the melody.

### NPC dialogue

- [x] `arena_talk_to`
- [x] `arena_choose`
- [x] `arena_end_talk`

### Movement and doors

- [x] `arena_move_to`
- [x] `arena_move`
- [x] `arena_check_path`
- [x] `arena_stop`
- [x] `arena_unstick`
- [x] `arena_enter_door`

### Combat and duels

- [x] `arena_basic_attack`
- [x] `arena_use_action`
- [x] `arena_set_tactics`
- [x] `arena_queue_match`
- [x] `arena_match_status`

### Money, inventory, and trade

- [x] `arena_credit_balance`
- [x] `arena_credit_history`
- [x] `arena_inventory`
- [x] `arena_use_item`
- [x] `arena_equip`
- [x] `arena_trade_with`
- [x] `arena_buy`
- [x] `arena_sell`
- [x] `arena_pick_up`

## Identity and capability rules

Construct the adapter for one character:

```rust
ArenaGateway::for_character(
    transport,
    agent_id,
    character_id,
    capabilities,
    analytics,
)
```

The model must not provide `agent_id`. The adapter must inject it for every character operation.

The `character_id` is the stable character sheet identifier for analytics. Do not use the backend agent identifier as the analytics character dimension.

The typed gateway or the BodyActor must reject an operation when the character lacks its capability.

Do not rely on a hidden tool prompt as the capability check.

## Backend compatibility

Use backend-reported combat actions, cooldowns, and equipment when the response contains them.

Use `Option<T>` for absent data.

If the live backend does not yet report legal skills, add one feature-gated compatibility adapter. Do not copy the skill map into the tactician.

## Contract tests

- [x] Deserialize current observation fixtures.
- [x] Deserialize current map fixtures.
- [x] Deserialize representative success and failure results for each tool group.
- [x] Verify exact request argument names.
- [x] Verify SSE parsing with split data chunks.
- [x] Verify that SSE parsing stops before stream closure.
- [x] Verify session ID capture and reuse through a local HTTP server.
- [x] Verify identity injection.
- [x] Verify that a caller cannot replace identity.
- [x] Verify capability rejection before transport use.
- [x] Verify that unknown optional fields do not break parsing.
- [x] Verify durable history cursor, reducer, lineage, and unknown-field decoding.
- [x] Verify native party roster and invitation decoding.
- [x] Verify timeout classification and one terminal failure event.
- [x] Verify secret redaction in errors and analytics.
- [x] Compare the live `tools/list` result with the complete expected tool inventory.
- [x] Fail the live compatibility diagnostic when a tool is missing or new.

## Observability implementation

All Phase 02 operations use one injected `AnalyticsSink`.

The current combat mutation response confirms acceptance. It reports `accepted`, `actionType`, `targetKind`, and the selected object or player identifier. The typed response also accepts the older immediate-outcome fields as compatibility data. Callers must not treat successful JSON decoding as proof that the backend accepted or applied an attack.

The transport records request start, request completion, notification completion, failure class, duration, response mode, request ID, correlation ID, and session change. The typed gateway records tool start, tool completion, capability rejection, tool failure, and decode failure. The session layer records connect, registration operations, login, reconnect attempts, reconnect completion, disconnect, and decision invalidation.

The harness does not record tool arguments or tool results. These values can contain speech, dialogue, identity data, or future secret fields. The harness records safe dimensions only.

Chat telemetry records the channel, whether a recipient exists, and the message length. It does not record the message or recipient. Received chat telemetry records per-channel counts. Reldens type `1` is scene chat, type `4` is private chat, type `8` is team chat, and type `9` is global chat. The current `arena_say` contract can write scene, global, and private messages. Team messages can be read, but the MCP API does not expose a team-write operation.

Music telemetry uses one correlation identifier for the typed performance and the MCP request. A production read-after-write diagnostic confirms that the performance returns through perception. It records counts, never note content.

The production tool-inventory diagnostic calls MCP `tools/list`, follows pagination, and compares the advertised names with the typed surface. It emits missing and unexpected tool names. This check makes new commands visible before an actor or model depends on them.

History reads emit `history.read_requested`, `history.read_completed`, or `history.read_failed` with safe cursor and count facts. They never log event payloads. A production movement/history diagnostic records only the durable event ID, lineage-presence facts, scene match, and movement outcome.

See [Observability Event Catalog](../observability/event-catalog.md).

## Acceptance criteria

Phase 02 is complete when:

1. Raw MCP JSON does not enter actor messages.
2. The typed adapter covers every current harness operation.
3. Registration and login work with a dedicated live test character.
4. Observe, survey, map, say, move, attack, and inventory pass live smoke tests.
5. SSE responses complete without waiting for stream closure.
6. Identity and capability tests pass.
7. Reconnect emits a runtime invalidation event.

Local criteria 1, 2, 5, 6, and 7 pass. Criterion 3 passes. Observe, survey, map, say, melody, map-guided movement, and inventory in criterion 4 pass. The attack check remains. See [Phase 02 Live Smoke Test: 2026-08-11](../testing/live-smoke-2026-08-11.md).

The live run identified more compatibility rules. The adapter accepts a numeric player identifier as a number or decimal string. Directional movement sends `up`, `down`, `left`, or `right`. A path result uses `pathLengthTiles`. A map door uses `column` and `row`. Successful plain-text MCP content becomes a typed message result. `arena_end_talk` requires `object_id`. The current backend also adds survey, durable history, parties, equipment, tactics, spectator-thought, and melody tools. Contract tests cover these rules.

## Out of scope

Do not implement tactical decisions or long-running movement recovery in this phase.
