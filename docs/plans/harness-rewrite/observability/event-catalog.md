# Observability Event Catalog

This file defines the stable event vocabulary for the Rust harness.

Observability is part of each feature. It is not a final hardening task.

## Event envelope

Each event has these fields:

- `process_run_id`: One identifier shared by every event from one harness process.
- `event_id`: A unique event identifier.
- `occurred_at`: A Coordinated Universal Time timestamp.
- `name`: A stable event name from this catalog.
- `level`: `debug`, `info`, `warn`, or `error`.
- `character_id`: The stable character sheet identifier when available.
- `correlation_id`: The identifier that links one cause and its effects.
- `attributes`: Event-specific, non-secret dimensions.

Do not use a log message as an identifier. Queries must use `name` and typed dimensions.

## Causal identifiers

Use one correlation identifier for a typed MCP tool call and its protocol request.

Use the action packet identifier as the correlation identifier for packet validation and execution.

Record these identifiers when the domain supplies them:

- runtime identifier;
- session generation;
- MCP request identifier;
- perception pump identifier;
- observation cycle identifier and sequence;
- frame revision;
- strategic revision;
- decision identifier;
- action packet identifier.

A reconnect increments the session generation. It also emits `runtime.decisions_invalidated`. No action from an older session generation can resume.

## Runtime and actor events

| Event name | Level | Required attributes |
| --- | --- | --- |
| `runtime.started` | info | `runtime_id` |
| `runtime.shutdown_started` | info | none |
| `runtime.shutdown_requested` | info | `reason` |
| `runtime.shutdown_completed` | info | `duration_ms` |
| `runtime.decisions_invalidated` | warn | `generation`, `reason` |
| `runtime.controlled_packet_decided` | info or warn | decision and packet ids, revisions, action count, packet lifetime, release request, result, reason, remaining action budget |
| `runtime.model_packet_decided` | info or warn | decision and packet ids, revisions, action count, packet lifetime, release result, reason, remaining action budget |
| `runtime.safety_stop_triggered` | error | `runtime_id`, `reason_code`, frame and strategy revisions, combat and health-known facts, scene-known fact |
| `runtime.safety_fallback_started` | warn | `runtime_id`, `reason_code`, `fallback_style`, `fallback_mode` |
| `runtime.safety_fallback_completed` | warn | `runtime_id`, `reason_code`, causal decision, packet, and action ids, revisions, duration, terminal status and reason |
| `runtime.safety_fallback_failed` | error | `reason_code`, `error_class` |
| `actor.started` | info | `actor` |
| `actor.failed` | error | `actor`, `reason` |
| `actor.terminated` | warn | `actor`; `reason` when available |

## Session events

| Event name | Level | Required attributes |
| --- | --- | --- |
| `mcp.session_connecting` | info | `reconnect` |
| `mcp.session_connected` | info | `generation`, `duration_ms`, `protocol_version` |
| `mcp.session_connect_failed` | error | `error_class` |
| `mcp.session_reconnect_attempted` | warn | `attempt` |
| `mcp.session_reconnect_failed` | warn | `attempt`, `error` |
| `mcp.session_reconnected` | info | `generation`, `duration_ms`, `protocol_version` |
| `mcp.session_disconnected` | info | `generation` |
| `mcp.session_tool_started` | debug | `tool` |
| `mcp.session_tool_completed` | debug | `tool`, `duration_ms` |
| `mcp.session_tool_failed` | warn | `tool`, `duration_ms`, `error_class` |
| `mcp.session_tool_decode_failed` | warn | `tool`, `duration_ms` |

## Protocol events

| Event name | Level | Required attributes |
| --- | --- | --- |
| `mcp.request_started` | debug | `method`, `request_id` |
| `mcp.request_completed` | debug | `method`, `request_id`, `duration_ms`, `response_mode`, `session_changed` |
| `mcp.request_failed` | warn | `method`, `duration_ms`, `error_class`; `request_id` when applicable |
| `mcp.notification_started` | debug | `method` |
| `mcp.notification_completed` | debug | `method`, `duration_ms` |
| `mcp.notification_failed` | warn | `method`, `duration_ms`, `error_class` |

The valid error classes are:

- `timeout`;
- `transport`;
- `http_status`;
- `protocol`;
- `json_rpc`;
- `tool`.

Every started protocol operation must emit one completion event or one failure event.

## Typed tool events

| Event name | Level | Required attributes |
| --- | --- | --- |
| `mcp.tool_started` | debug | `tool`, `argument_count` |
| `mcp.tool_completed` | debug | `tool`, `duration_ms` |
| `mcp.tool_failed` | warn | `tool`, `duration_ms`, `error_class` |
| `mcp.tool_decode_failed` | warn | `tool`, `duration_ms` |
| `mcp.tool_rejected` | warn | `tool`, `reason`; `capability` for a capability rejection |

## Cognitive and body events

| Event name | Level | Required attributes |
| --- | --- | --- |
| `perception.pump_started` | info | `pump_id`, `interval_ms`, `map_radius`, `inventory_every_cycles` |
| `perception.cycle_started` | debug | `pump_id`, `observation_cycle_id`, `observation_cycle_sequence`, `duration_ms`, `map_radius`, `inventory_requested` |
| `perception.cycle_completed` | debug or warn | `pump_id`, `observation_cycle_id`, `observation_cycle_sequence`, `duration_ms`, `status`, `inventory_requested`, `inventory_available`, `inventory_every_cycles`, `strategic_revision`; `inventory_error_class` when degraded |
| `perception.cycle_failed` | warn or error | `pump_id`, `observation_cycle_id`, `observation_cycle_sequence`, `duration_ms`, `failure_stage`; safe operation-specific error classes for a read failure |
| `perception.cycle_cancelled` | info | `pump_id`, `observation_cycle_id`, `observation_cycle_sequence`, `duration_ms`, `reason` |
| `perception.pump_stopped` | info | `pump_id`, `reason`, `cycles_started` |
| `perception.frame_published` | debug | `observation_cycle_known`; `observation_cycle_id` and `observation_cycle_sequence` for an MCP-derived frame; `frame_revision`, `perception_revision`, `strategic_revision`, `inventory_revision`, `map_revision`, `material_change`, `derived_event_count`, `backend_event_count`, `new_dialogue_count`, `new_melody_count`, `new_scene_chat_count`, `new_global_chat_count`, `new_private_chat_count`, `new_team_chat_count`, `new_unknown_chat_count`, `filtered_chat_count`, `visible_entity_count`, `visible_hostile_count`, `visible_player_count`, `visible_npc_count`, `visible_merchant_count`, `visible_enemy_count`, `visible_unknown_count`, `drop_count`, `positioned_drop_count`, `unpositioned_drop_count`, `carried_item_count`, `carried_item_units`, `door_count`, `locked_door_count`, `unknown_lock_door_count`, `reported_total_object_count_known`, `reported_total_object_count`, `object_list_truncated_known`, `object_list_truncated`, `reachable_exit_count`, `nearest_exit_path_length_known`, `nearest_exit_path_length`, `map_tile_count` |
| `perception.snapshot_rejected` | warn | `observation_cycle_id`, `observation_cycle_sequence`, `error_class` |
| `strategy.published` | info | `decision_id`, `input_revision`, `strategic_revision` |
| `strategic.inference_started` | info | `decision_id`, `input_revision`, `base_strategic_revision`, `moment_count` |
| `strategic.inference_coalesced` | debug | active and pending input revisions, `base_strategic_revision`, `pending_moment_count` |
| `strategic.inference_superseded` | info | strategic causal fields, `duration_ms`, `reason_code` |
| `strategic.inference_completed` | info | strategic causal fields, `duration_ms`, `published`, `published_revision` |
| `strategic.inference_failed` | warn | strategic causal fields, `duration_ms`, `error_class` |
| `tactical.wake_requested` | debug | `signal_id`, `frame_revision`, `strategic_revision`, `wake_reason`, `activity` |
| `tactical.wake_suppressed` | debug | `signal_id`, `frame_revision`, `strategic_revision`, `suppression_reason` |
| `tactical.wake_deferred` | debug | `signal_id`, `frame_revision`, `strategic_revision`, `deferral_reason`, `eligible_after_ms_known`, `eligible_after_ms`, `coalesced_reason_count` |
| `tactical.wake_coalesced` | debug | `signal_id`, source and pending revisions, `coalesced_reason_count` |
| `tactical.heartbeat_generated` | debug | `signal_id`, `frame_revision`, `strategic_revision`, `activity` |
| `tactical.inference_started` | info | `trigger_signal_id`, `decision_id`, `scheduler_inference_id`, `frame_revision`, `strategic_revision`, bounded wake reasons |
| `tactical.inference_completed` | info | `decision_id`, `frame_revision`, `strategic_revision`, `duration_ms`, `action_count` |
| `tactical.inference_superseded` | info | `decision_id`, `frame_revision`, `strategic_revision`, `duration_ms`, `reason_code` |
| `tactical.inference_failed` | warn | `decision_id`, `frame_revision`, `strategic_revision`, `duration_ms`, `error_class` |
| `tactical.packet_release_decided` | info | `decision_id`, `packet_id`, revisions, `rollout_mode`, `release_policy`, `action_count`, `released`, `reason_code` |
| `body.packet_accepted` | info | `packet_id`, `decision_id`, `frame_revision`, `strategic_revision` |
| `body.packet_rejected` | warn | `packet_id`, `decision_id`, `frame_revision`, `strategic_revision`, `reason` |
| `body.packet_completed` | info | `packet_id`, `decision_id`, `frame_revision`, `strategic_revision`, `status` |
| `body.packet_failed` | warn | packet causal fields, `status`, `reason_code` when known |
| `body.packet_aborted` | warn | packet causal fields, `status`, `reason_code` |
| `body.packet_cancelled` | warn | packet causal fields, `status`, `reason_code` |
| `body.packet_superseded` | warn | packet causal fields, `status`, `superseded_by` |
| `body.action_started` | info | `session_generation`, `decision_id`, `packet_id`, `action_id`, `action_index`, `frame_revision`, `strategic_revision`, `action_kind` |
| `body.action_succeeded` | info | action causal fields, `action_kind`, `status`, `duration_ms`, following-frame fields |
| `body.action_failed` | warn | action causal fields, `action_kind`, `status`, `duration_ms`, `reason_code` |
| `body.action_rejected` | warn | action causal fields, `action_kind`, `status`, `reason_code` |
| `body.action_cancelled` | warn | action causal fields, `action_kind`, `status`, `reason_code` |
| `body.action_superseded` | warn | action causal fields, `action_kind`, `status`, `reason_code` |

Later phases must add execution outcomes and following world-event identifiers. Model events already carry latency, identity, prompt version, token categories, and exact provider accounting.

## Durable history events

| Event name | Level | Required attributes |
| --- | --- | --- |
| `history.read_requested` | info | `after_known`, `before_known`, `time_range_known`, `requested_limit` |
| `history.read_completed` | info | `event_count`, `has_more`, `cursor`, `oldest` |
| `history.read_failed` | warn | no payload fields; correlated MCP failure events carry the safe error class |

History events never contain speech, raw engine rows, item payloads, or other event data. Those values remain inside the typed history response until Phase 09 converts supported kinds into typed `GameEvent` values.

## Memory and retrieval events

| Event name | Level | Required attributes |
| --- | --- | --- |
| `memory.conversation_operation_started` | debug | `operation` |
| `memory.conversation_operation_completed` | debug | `operation`, `duration_ms`, `message_count`, `serialized_bytes` |
| `memory.conversation_operation_failed` | warn | `operation`, `duration_ms`, `error_class` |
| `memory.typed_operation_started` | debug | `operation` |
| `memory.typed_operation_completed` | debug | `operation`, `duration_ms`, `record_count` |
| `memory.typed_operation_failed` | warn | `operation`, `duration_ms`, `error_class` |
| `memory.semantic_write_started` | debug | `memory_kind`, `source_kind`, `provenance_known` |
| `memory.semantic_write_completed` | info | `memory_kind`, `source_kind`, `provenance_known`, `duration_ms`, `inserted`, `updated` |
| `memory.semantic_write_failed` | warn | `memory_kind`, `source_kind`, `duration_ms`, `error_class` |
| `memory.index_rebuild_started` | info | `index_version`, `embedding_model` |
| `memory.index_rebuild_completed` | info | `index_version`, `embedding_model`, `document_count`, `duration_ms` |
| `memory.index_rebuild_failed` | warn | `index_version`, `embedding_model`, `duration_ms`, `error_class` |
| `memory.recall_started` | debug | `index_version`, `embedding_model`, `requested_count`, `memory_kind_filter_count` |
| `memory.recall_completed` | info | `index_version`, `embedding_model`, `requested_count`, `returned_count`, `duration_ms`, `provenance_coverage`, `score_range_known`, `minimum_score`, `maximum_score` |
| `memory.recall_failed` | warn | `index_version`, `embedding_model`, `requested_count`, `duration_ms`, `error_class` |

Memory events never contain a conversation identifier, conversation text,
memory text, recall query text, an embedding, private speech, or model output.

## Model events

| Event name | Level | Required attributes |
| --- | --- | --- |
| `model.brain_configured` | info | character ID, provider, cognitive role, requested model, prompt version, completion budget, timeout, and requested reasoning enabled/effort/exclusion facts |
| `model.input_assembled` | debug | model causal fields, deterministic logical-request fingerprint, byte/token estimates, bounded-history message count, prompt/schema byte counts, and local-capture enabled/success facts; never request content |
| `model.input_capture_failed` | warn | model causal fields, request fingerprint, and stable local-write error class; never path or content |
| `model.call_started` | info | `decision_id`, `frame_revision_known`, `frame_revision`, `strategic_revision_known`, `strategic_revision`, `provider`, `requested_model`, `cognitive_role`, `prompt_version`, `input_serialized_bytes`, and requested reasoning enabled/effort/exclusion/support facts |
| `model.call_completed` | info | `decision_id`, `frame_revision_known`, `frame_revision`, `strategic_revision_known`, `strategic_revision`, `provider`, `requested_model`, `actual_model`, `generation_id`, `cognitive_role`, `prompt_version`, `latency_ms`, finish reason and completion-budget exhaustion, all Rig token categories, exact OpenRouter charge with a known flag, effective charge per total token when defined, and per-character process totals |
| `model.call_failed` | warn | `decision_id`, `frame_revision_known`, `frame_revision`, `strategic_revision_known`, `strategic_revision`, `provider`, `requested_model`, `cognitive_role`, `prompt_version`, `latency_ms`, `error_class`, `timeout_configured`, `http_status_known`, `http_status`, `provider_error_code_known`, `provider_error_code`, `rate_limited`, `quota_exhausted`, `response_received`, `usage_accounted` |
| `model.usage_anomaly` | warn | model causal fields, `generation_id`, `anomaly_class`, requested maximum and reported token count |
| `model.background_tasks_drained` | info | `completed`, `failed`, `aborted`, `active_model_calls_remaining` |
| `model.response_received` | info | model causal fields, `generation_id`, all available token categories, exact OpenRouter charge, latency, and per-character totals |
| `model.output_parse_completed` | debug | model causal fields, `generation_id`, parse format |
| `model.output_parse_failed` | warn | model causal fields, `generation_id`, stable parse category and location; never raw output |
| `model.generation_accounted` | info | `generation_id`, `actual_provider`, `model`, `service_tier`, token counts, native token counts, exact OpenRouter charge, BYOK applicability, upstream cost when applicable, cache discount, and provider latency |
| `model.native_usage_reconciled` | info | full model causal fields, generation ID, actual provider/model, Rig and native reasoning/cache token counts, reconciled deltas, and updated per-character totals |
| `model.generation_accounting_failed` | warn | `generation_id`, `error_class` |
| `model.price_snapshot` | info | `snapshot_id`, `observed_at`, model, provider endpoint, status, quantization, capacity, cache support, recent service measures, and every advertised price as exact decimal text plus a numeric query value |
| `model.price_snapshot_completed` | info | `snapshot_id`, `requested_model`, `endpoint_count` |
| `model.price_snapshot_failed` | warn | `requested_model`, `error_class` |

## Rig Agent events

| Event name | Level | Required attributes |
| --- | --- | --- |
| `rig.agent_run_started` | info | decision and revision facts, `max_turns`, `requested_max_output_tokens`, `timeout_ms` |
| `rig.agent_run_completed` | info | decision and revision facts, `duration_ms`, `model_request_count`, all Rig token categories |
| `rig.agent_run_failed` | warn | decision and revision facts, `duration_ms`, `error_class` |

These events describe one agent run. Provider-call events remain the source for
exact OpenRouter charges. Do not add the Rig aggregate usage to the provider
charge ledger a second time.

## Runtime run-control events

| Event name | Level | Required attributes |
| --- | --- | --- |
| `runtime.safety_stop_triggered` | error | `runtime_id`, `reason_code`, frame and strategy revisions, combat and health-known facts, configured model-cost limit, accounted model cost |
| `runtime.run_summary` | info | `runtime_id`, `shutdown_reason`, connected duration, separate tactician and strategist calls and cost, combined token categories and exact-cost coverage, observed cost, simple 24-hour projection, actor failures, packet outcomes, action outcomes and success rate, movement arrivals, stalls, stops and stop failures |
| `runtime.shutdown_started` | info | runtime correlation and character ID |
| `runtime.shutdown_completed` | info | runtime correlation, character ID, shutdown duration |

`runtime.run_summary` is the terminal aggregate for one process runtime. The
causal events remain the source for model, provider, activity, and interaction
class breakdowns. A projection in this event is an extrapolation from observed
connected time. It is not a price quote.

The response charge and finalized generation charge are billing facts. Provider price snapshots are time-stamped reference data. Do not substitute one for the other.

## Chat events

| Event name | Level | Required attributes |
| --- | --- | --- |
| `chat.send_requested` | info | `channel`, `recipient_known`, `message_character_count` |
| `chat.send_completed` | info | `channel`, `recipient_known` |
| `chat.send_failed` | warn | `channel`, `recipient_known` |

Chat events share their correlation ID with the underlying `arena_say` MCP events. They never contain message text or a recipient name.

## Music events

| Event name | Level | Required attributes |
| --- | --- | --- |
| `music.performance_requested` | info | `instrument`, `times`, `note_count`, `melody_character_count` |
| `music.performance_completed` | info | `instrument`, `times`, `note_count` |
| `music.performance_failed` | warn | `instrument`, `times`, `note_count` |

Music events share their correlation ID with the underlying `arena_play_melody` MCP events. They never contain the melody or the backend display text.

Do not record the prompt or raw model output in these events. Replay storage is a separate controlled data path.

The TacticianActor supplies its decision ID as the model-call correlation ID. The model event also uses the stable character-sheet ID. A standalone model probe creates a new decision ID and marks frame and strategy revisions as unknown.

## Diagnostic events

| Event name | Level | Required attributes |
| --- | --- | --- |
| `diagnostic.perception_frame_normalized` | info | `frame_revision`, `perception_revision`, `inventory_revision`, `map_revision`, `map_width`, `map_height`, `map_origin_x`, `map_origin_y`, `source_scene_width`, `source_scene_height`, `self_tile_x`, `self_tile_y`, `material_change`, `derived_event_count`, `backend_event_count`, `new_dialogue_count`, `new_melody_count`, `filtered_chat_count`, `visible_entity_count`, `visible_hostile_count`, `visible_player_count`, `visible_npc_count`, `visible_merchant_count`, `visible_enemy_count`, `visible_unknown_count`, `drop_count`, `positioned_drop_count`, `unpositioned_drop_count`, `carried_item_count`, `carried_item_units`, `door_count`, `locked_door_count`, `unknown_lock_door_count`, `reported_total_object_count_known`, `reported_total_object_count`, `object_list_truncated_known`, `object_list_truncated`, `reachable_exit_count`, `nearest_exit_path_length_known`, `nearest_exit_path_length`, `map_tile_count`, `health_known`, `position_known`, `combat_known` |
| `diagnostic.map_line_shape` | debug | `line_index`, `character_count`, `wall_count`, `floor_count`, `self_count`, `player_count`, `npc_count`, `enemy_count`, `door_count`, `locked_door_count`, `space_count`, `other_count`, `scene_width`, `scene_height` |
| `diagnostic.production_navigation` | info | `target_tile_x`, `target_tile_y`, `before_pixel_x`, `before_pixel_y`, `before_tile_x`, `before_tile_y`, `after_pixel_x`, `after_pixel_y`, `after_tile_x`, `after_tile_y`, `progressed`, `changed_tile`, `reached_target`, `backend_result_known`, `backend_arrived_known`, `backend_arrived`, `backend_came_to_rest_known`, `backend_came_to_rest` |
| `diagnostic.production_survey` | info | `scene_known`, `grid_available`, `enemy_count`, `people_count`, `readable_count`, `drop_count`, `other_player_count`, `way_out_count`, `structured_way_out_count`, `locked_way_out_count`, `contains_locked_door_marker` |
| `diagnostic.production_melody_read` | info | `instrument`, `heard_melody_count` |
| `diagnostic.production_tool_inventory` | info | `advertised_tool_count`, `expected_tool_count`, `missing_tool_count`, `unexpected_tool_count`, `missing_tools`, `unexpected_tools` |
| `diagnostic.production_history_roundtrip` | info | `movement_event_id`, `decision_id_present`, `scene_present`, `scene_matches`, `position_changed`, `reached_target`, `backend_arrived` |

Diagnostic events must follow the same payload and secret rules as production events.

## Data safety

Do not record these values:

- API keys;
- authorization headers;
- MCP session identifiers;
- raw tool arguments;
- raw tool results;
- private conversation text;
- model prompts;
- complete model outputs.

Replace configured secrets with `[REDACTED]` before an external error enters an event or log.

Record safe counts and identifiers instead of payloads. For example, record `argument_count`, not the arguments.

## Test rule

Each new operation must have tests for its terminal event.

Each failure path must prove these properties:

1. The error class is stable.
2. One terminal failure event exists.
3. The event has a correlation identifier when the operation has one.
4. The event and returned error do not contain configured secrets.
