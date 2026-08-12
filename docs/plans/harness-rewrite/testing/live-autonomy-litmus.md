# Live Autonomy Litmus Test

`scripts/live-litmus.py` is the bounded black-box acceptance check for a live
character runner. It reads structured JSON logs only; it never sends an MCP
command and cannot change the character.

## What it proves

A run is considered successful only when the log shows:

1. perception frames and at least one body action;
2. strategic navigation or physical movement;
3. a hostile zone being observed;
4. combat becoming active or a real attack action;
5. no unresolved model/runtime failure that invalidates the run.

The report also includes scene transitions, action counts, social/chat activity,
model calls, input/output/cache/reasoning tokens, exact OpenRouter cost, and
failure classes. Unknown health is reported as unknown; `health=0` with
`health_known=false` is never treated as a death.

## Usage

For a one-shot report over the current file:

```bash
python3 scripts/live-litmus.py var/local-guy/guy.log
```

To follow a newly started runner for fifteen minutes:

```bash
python3 scripts/live-litmus.py var/local-guy/guy.log \
  --run-id "$PROCESS_RUN_ID" --watch --seconds 900 --interval 5
```

The command exits zero only after the success criteria are observed. A bounded
run exits non-zero with explicit reasons such as `no_strategic_navigation_or_movement`,
`no_hostile_zone_seen`, `no_combat_started`, or
`model_or_runtime_failures_present`.

## Protocol boundary

The watcher does not relax model output validation. Brain outputs are Rust
enums serialized as strict JSON and must pass deserialization and semantic
validation before the body can act. Provider JSON-schema or response-format
features can improve adherence, but they are not trusted as the authority for
legal actions or enum values.
