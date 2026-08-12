def color($code; $text): "\u001b[" + $code + "m" + ($text | tostring) + "\u001b[0m";
def time: (.timestamp // "" | if length >= 19 then .[11:19] else . end);
def attrs: (.fields.attributes // "{}" | fromjson? // {});
def value($a; $key): ($a[$key] // "-");
def known_value($a; $key; $known):
  if ($a[$known] // false) then ($a[$key] | tostring) else "?" end;
def tag($name): color("1;36"; ($name | ascii_upcase));
def warn: color("1;33"; "WARN");
def fail: color("1;31"; "FAIL");
def good: color("1;32"; "OK");

def activity:
  .fields.event_name as $name
  | attrs as $a
  | if $name == "perception.frame_published" and $a.material_change == true then
      [time, tag("world"), value($a; "scene"),
       ("tile=" + (value($a; "position_tile_x") | tostring) + "," + (value($a; "position_tile_y") | tostring)),
       ("hp=" + (if $a.health_known then ($a.health | tostring) + "/" + ($a.max_health | tostring) else "?" end)),
       ("alive=" + (known_value($a; "alive"; "alive_known"))),
       ("people=" + (((value($a; "visible_player_count") | tonumber?) // 0) + ((value($a; "visible_npc_count") | tonumber?) // 0) | tostring)),
       ("hostiles=" + (value($a; "visible_hostile_count") | tostring)),
       ("drops=" + (value($a; "drop_count") | tostring)),
       ("frame=" + (value($a; "frame_revision") | tostring))] | join("  ")
    elif $name == "body.movement_scene_transition" then
      [time, tag("scene"), value($a; "from_scene"), color("1;35"; "→"), value($a; "to_scene"),
       ("tile=" + (value($a; "tile_x") | tostring) + "," + (value($a; "tile_y") | tostring))] | join("  ")
    elif $name == "strategy.published" then
      [time, tag("strategy"), ("rev=" + (value($a; "strategic_revision") | tostring)),
       ("nav=" + (if (value($a; "navigation_scene")) == "" then "none" else (value($a; "navigation_scene") | tostring) end)),
       ("subgoals=" + (value($a; "subgoal_count") | tostring)),
       ("targets=" + (value($a; "preferred_target_count") | tostring))] | join("  ")
    elif $name == "strategic.inference_started" then
      [time, tag("strategist"), "thinking", ("scene=" + (value($a; "scene") | tostring)),
       ("people=" + (value($a; "visible_entity_count") | tostring)),
       ("exits=" + (value($a; "exit_count") | tostring)),
       ("prior_failures=" + (value($a; "consecutive_failures_before_call") | tostring)),
       (if $a.last_successful_inference_known == true then
          "plan_age=" + (value($a; "last_successful_inference_age_ms") | tostring) + "ms"
        else "no_prior_plan" end)] | join("  ")
    elif $name == "strategic.recall_completed" then
      [time, color("2;36"; "RECALL"),
       ("facts=" + (value($a; "semantic_count") | tostring)),
       ("people=" + (value($a; "relationship_count") | tostring)),
       ("episodes=" + (value($a; "episode_count") | tostring)),
       ("plan_steps=" + (value($a; "plan_step_count") | tostring)),
       ("duration=" + (value($a; "duration_ms") | tostring) + "ms")] | join("  ")
    elif $name == "strategic.recall_failed" then
      [time, fail, "strategic recall", ("class=" + (value($a; "error_class") | tostring))] | join("  ")
    elif $name == "strategic.plan_changed" then
      [time, tag("plan"), ("rev=" + (value($a; "plan_revision") | tostring)),
       ("steps=" + (value($a; "step_count") | tostring)),
       ("retained=" + (value($a; "retained_step_count") | tostring)),
       (if $a.blocked == true then color("1;33"; "BLOCKED") else "active" end),
       (if $a.completion_claimed == true then "completion-claimed" else "" end)]
       | map(select(length > 0)) | join("  ")
    elif $name == "strategic.plan_step_advanced" then
      [time, tag("plan"), value($a; "transition"),
       ("rev=" + (value($a; "plan_revision") | tostring)),
       ("tries=" + (value($a; "tries") | tostring)),
       ("evidence=" + (value($a; "evidence_count") | tostring))] | join("  ")
    elif $name == "strategic.navigation_arrival_observed" then
      [time, tag("strategist"), "destination reached",
       ("destination=" + (value($a; "destination_scene") | tostring)),
       ("arrived=" + (value($a; "arrived_scene") | tostring)),
       ("attempts=" + (value($a; "attempts") | tostring))] | join("  ")
    elif $name == "strategic.inference_superseded" then
      [time, warn, "strategist discarded stale thought", ("reason=" + (value($a; "reason_code") | tostring))] | join("  ")
    elif $name == "strategic.inference_failed" then
      [time, fail, "strategic inference", ("class=" + (value($a; "error_class") | tostring)),
       ("duration=" + (value($a; "duration_ms") | tostring) + "ms"),
       ("failures=" + (value($a; "consecutive_failures") | tostring)),
       ("retry_in=" + (value($a; "retry_after_ms") | tostring) + "ms"),
       (if $a.last_successful_inference_known == true then
          "old_plan_age=" + (value($a; "last_successful_inference_age_ms") | tostring) + "ms"
        else "no_usable_plan" end),
       (if $a.previous_intent_retained == true then "old_intent_retained" else "" end)]
       | map(select(length > 0)) | join("  ")
    elif $name == "tactical.inference_completed" then
      [time, tag("tactic"), color("1;37"; value($a; "intent")), value($a; "action_plan"),
       ("latency=" + (value($a; "duration_ms") | tostring) + "ms"),
       ("frame=" + (value($a; "frame_revision") | tostring) + "/strategy=" + (value($a; "strategic_revision") | tostring))] | join("  ")
    elif $name == "body.action_started" then
      [time, tag("action"), value($a; "action_kind"), ("#" + (value($a; "action_index") | tostring)),
       ("action=" + ((value($a; "action_id") | tostring)[0:8]))] | join("  ")
    elif ($name | startswith("body.action_")) and ($name | endswith("succeeded")) then
      [time, good, value($a; "action_kind"), ("duration=" + (value($a; "duration_ms") | tostring) + "ms")] | join("  ")
    elif ($name | startswith("body.action_")) and ($name | endswith("failed")) then
      [time, fail, value($a; "action_kind"), ("reason=" + (value($a; "reason_code") | tostring))] | join("  ")
    elif $name == "body.movement_progress" then
      [time, color("2;37"; "MOVE"),
       ("tile=" + (value($a; "tile_x") | tostring) + "," + (value($a; "tile_y") | tostring)),
       ("remaining=" + (value($a; "remaining_tile_distance") | tostring))] | join("  ")
    elif $name == "body.navigation_mission_started" then
      [time, tag("navigate"), "start", value($a; "destination_scene"),
       (if $a.destination_tile_known == true then
          "tile=" + (value($a; "destination_tile_x") | tostring) + "," + (value($a; "destination_tile_y") | tostring)
        else "scene destination" end),
       ("waypoints=" + (value($a; "route_waypoints") | tostring))] | join("  ")
    elif $name == "body.navigation_attempt_started" then
      [time, color("1;36"; "NAV TRY"), value($a; "attempt_kind"),
       ("#" + (value($a; "attempt_number") | tostring)),
       ("scene=" + (value($a; "scene") | tostring)),
       ("tile=" + (value($a; "target_tile_x") | tostring) + "," + (value($a; "target_tile_y") | tostring))] | join("  ")
    elif $name == "body.navigation_mission_paused" then
      [time, warn, "navigation paused", ("reason=" + (value($a; "reason_code") | tostring))] | join("  ")
    elif $name == "body.navigation_mission_resumed" then
      [time, tag("navigate"), "resumed", ("scene=" + (value($a; "scene") | tostring)),
       ("attempt=" + (value($a; "attempt_number") | tostring))] | join("  ")
    elif $name == "body.navigation_retry_scheduled" then
      [time, warn, "navigation retry", ("attempt=" + (value($a; "attempt_number") | tostring)),
       ("reason=" + (value($a; "reason_code") | tostring))] | join("  ")
    elif $name == "body.navigation_mission_terminal" then
      [time, (if $a.state == "arrived" then good else fail end), "navigation", value($a; "state"),
       ("scene=" + (value($a; "scene") | tostring)),
       ("attempts=" + (value($a; "attempts") | tostring)),
       (if ($a.reason_code // "") == "" then "" else "reason=" + ($a.reason_code | tostring) end)]
       | map(select(length > 0)) | join("  ")
    elif ($name | startswith("body.packet_")) then
      [time, tag("packet"), ($name | sub("body.packet_"; "")),
       ("packet=" + ((value($a; "packet_id") | tostring)[0:8])),
       (if $a.reason_code then "reason=" + ($a.reason_code | tostring)
        elif $a.reason then "reason=" + ($a.reason | tostring)
        else "" end)] | map(select(length > 0)) | join("  ")
    else empty end;

def models:
  .fields.event_name as $name
  | attrs as $a
  | if $name == "model.call_completed" then
      [time, tag(value($a; "cognitive_role")), value($a; "actual_model"),
       ("latency=" + (value($a; "latency_ms") | tostring) + "ms"),
       ("tokens=" + (value($a; "total_tokens") | tostring)),
       ("cached=" + (value($a; "cached_input_tokens") | tostring)),
       ("cost=$" + (value($a; "openrouter_cost_usd_exact") | tostring)),
       ("run=$" + (value($a; "agent_openrouter_cost_usd_total") | tostring))] | join("  ")
    elif $name == "model.generation_accounted" then
      [time, color("2;36"; "ACCOUNT"), value($a; "actual_provider"),
       ("native=" + (value($a; "native_tokens_prompt") | tostring) + "/" + (value($a; "native_tokens_completion") | tostring)),
       ("cached=" + (value($a; "native_tokens_cached") | tostring)),
       ("reasoning=" + (value($a; "native_tokens_reasoning") | tostring)),
       ("cost=$" + (value($a; "openrouter_cost_usd") | tostring)),
       ("cache_saved=$" + (value($a; "cache_discount_usd") | tostring))] | join("  ")
    elif $name == "model.input_assembled" then
      [time, color("2;36"; "INPUT"), value($a; "cognitive_role"),
       ("bytes=" + (value($a; "logical_request_bytes") | tostring)),
       ("est_tokens=" + (value($a; "logical_request_tokens_estimated") | tostring)),
       ("history=" + (value($a; "bounded_history_message_count") | tostring)),
       ("fingerprint=" + ((value($a; "request_fingerprint") | tostring)[0:8])),
       (if $a.local_input_capture_succeeded == true then "captured" else "metadata-only" end)] | join("  ")
    elif ($name | startswith("model.call_")) and ($name | endswith("failed")) then
      [time, fail, value($a; "cognitive_role"), value($a; "requested_model"),
       ("class=" + (value($a; "error_class") | tostring)),
       (if $a.timeout_configured == true then
          "elapsed=" + (value($a; "latency_ms") | tostring) + "ms/deadline=" + (value($a; "timeout_ms") | tostring) + "ms"
        else "latency=" + (value($a; "latency_ms") | tostring) + "ms" end)] | join("  ")
    elif ($name | startswith("mcp.session_")) or ($name | startswith("session.")) then
      [time, tag("session"), $name, ("generation=" + (value($a; "generation") | tostring))] | join("  ")
    elif (($name | startswith("mcp.")) and (($name | contains("failed")) or ($name | contains("rejected")))) then
      [time, fail, $name, ("tool=" + (value($a; "tool") | tostring)),
       ("class=" + (value($a; "error_class") | tostring))] | join("  ")
    elif ($name | startswith("runtime.safety_")) then
      [time, fail, $name, ("reason=" + (value($a; "reason_code") | tostring))] | join("  ")
    else empty end;

fromjson? as $event
| $event
| select(.fields.event_name? | type == "string")
| if $view == "activity" then activity else models end
