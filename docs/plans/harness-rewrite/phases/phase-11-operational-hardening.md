# Phase 11: Operational Hardening

State: In progress

## Goal

Make the Rust harness safe for continuous production use.

## Required result

The harness has explicit behavior for actor failure, provider failure, MCP reconnect, backpressure, rate limits, shutdown, and state recovery. Operators can reconstruct each important decision.

## Dependencies

Phases 01 through 10 must provide stable interfaces and representative fixtures.

## Supervision tasks

- [ ] Define the restart policy for each actor.
- [ ] Test dependency restoration before child restart is enabled.
- [ ] Restart the tactician without losing strategy.
- [ ] Keep the body session alive after a tactical model failure.
- [ ] Keep movement alive after a memory failure.
- [ ] Use process-level restart for an unsafe partial recovery.
- [ ] Detect a silently missing required actor.
- [ ] Expose actor health in telemetry.

Test each actor failure separately:

- BodyActor;
- PerceptionActor;
- TacticianActor;
- StrategistActor;
- MemoryActor;
- TelemetryActor.

## MCP reconnect tasks

- [ ] Detect connection and session failure.
- [ ] Stop current mutations.
- [ ] Invalidate all active packets.
- [ ] Invalidate all in-flight inference results.
- [ ] Reconnect with bounded exponential backoff and jitter.
- [ ] Register or find the same character.
- [ ] Log in again.
- [ ] Reload authoritative observation and inventory.
- [ ] Rebuild perception state.
- [ ] Resume only after a fresh revision exists.
- [ ] Never resume an old packet after reconnect.

## Provider failure tasks

- [ ] Classify authentication, rate, quota, timeout, transport, and parse failures.
- [ ] Add bounded retry behavior.
- [ ] Add configured provider or model fallback where evidence supports it.
- [ ] Apply a failure backoff.
- [ ] Keep actor mailboxes responsive during backoff.
- [ ] Record which model and provider answered.
- [x] Apply an optional exact-cost limit to each runtime. Stop if a completed response has no exact charge while the limit is active.
- [ ] Apply request timeouts.

The strategist must not replace the tactician during a tactical provider failure.

## Backpressure and rate tasks

- [ ] Bound every event queue.
- [ ] Use latest-value semantics for tactical frames.
- [ ] Bound recent actions and events.
- [ ] Enforce tactical maximum frequency.
- [ ] Enforce idle tactical frequency.
- [ ] Prevent duplicate strategic work.
- [ ] Limit concurrent provider calls.
- [ ] Limit reconnect attempts.
- [ ] Make dropped or coalesced work visible in metrics.

## Observability tasks

Use these identifiers:

- `DecisionId`;
- `ActionPacketId`;
- `CorrelationId`;
- `FrameRevision`;
- `StrategicRevision`.

Record this causal chain:

```text
game events
→ tactical frame
→ strategic intent
→ model run
→ tactical proposal
→ action packet
→ validation
→ MCP calls
→ action outcomes
→ following game events
```

- [x] Emit structured JSON logs.
- [ ] Add latency and error metrics.
- [ ] Add actor restart metrics.
- [ ] Add queue depth and coalescing metrics.
- [x] Add model token categories, exact OpenRouter charge, generation audit, provider price snapshots, and per-character running totals.
- [x] Emit one terminal `runtime.run_summary` event with role totals, action results, movement results, packet results, and the observed 24-hour cost projection.
- [ ] Redact secrets and private memory.
- [ ] Preserve prompt and model versions.
- [ ] Prepare trace export for backend analytics.

## Daily cost measurement

Do not project daily cost from model list prices. Sum the exact charge returned
for each OpenRouter response. Reconcile it with finalized generation records.
Keep reference endpoint prices as time-stamped context only.

For each production soak, report:

- exact observed cost and connected hours;
- observed dollars per connected hour;
- projected dollars per 24 connected hours;
- cost by character, cognitive role, requested model, actual model, and actual
  provider;
- input, output, reasoning, cached-input, and cache-write tokens;
- request count, failure count, and calls with unknown exact cost;
- cost during idle, movement, conversation, and combat exposure;
- tactical decisions per minute in each activity state;
- strategist calls per hour and the wake reason for each call;
- p50, p90, p95, and p99 latency by role and provider.

Compute the simple 24-hour projection as:

```text
exact observed dollars / connected hours * 24
```

Also report an activity-weighted projection from idle, movement, conversation,
and combat rates. Label both values as projections. A 24-hour run is still an
observation, not a guarantee of the next day's provider price, cache behavior,
or player activity.

Run at least one continuous 24-hour soak with the intended production settings
before cutover. Preserve the configuration hash, run IDs, reconnect periods,
and any excluded downtime. Do not omit failed model calls or unknown-cost calls
from the report.

Set `NPC_RUN_MAX_OPENROUTER_COST_USD` for every unattended soak. The limit uses
the sum of exact charges on completed OpenRouter responses. If a completed
response has no exact charge, the runtime stops with `model_cost_unknown`. If
the sum reaches the limit, the runtime stops with
`model_cost_limit_exceeded`.

The terminal `runtime.run_summary` event reports separate tactician and
strategist totals. It also reports the combined request count, token classes,
exact-cost coverage, exact observed cost, action success rate, packet completion
rate, movement arrivals and stalls, connected duration, and the simple 24-hour
projection. The projection is an extrapolation from the current run. It is not
a provider quote or a guarantee of the next day's activity.

## State and shutdown tasks

- [ ] Take typed state snapshots where recovery needs them.
- [ ] Flush required durable memory on graceful shutdown.
- [ ] Stop new inference work during shutdown.
- [ ] Cancel or finish safe in-flight work.
- [ ] Stop mutations before disconnect.
- [ ] Disconnect MCP cleanly.
- [ ] Stop every actor within a fixed timeout.
- [ ] Report an unclean shutdown.

## Security tasks

- [ ] Keep `agent_id` out of model schemas.
- [ ] Keep API keys out of logs and fixtures.
- [ ] Verify capability checks before mutation.
- [ ] Use least-privilege container settings.
- [ ] Use a read-only application filesystem where practical.
- [ ] Write memory only to its configured volume.
- [ ] Reject an unexpected character identity after reconnect.

## Tests

- [ ] Kill each actor and verify the defined outcome.
- [ ] Run a 30-second strategist task during combat updates.
- [ ] Disconnect MCP during a packet.
- [ ] Verify that reconnect invalidates the packet.
- [ ] Return a provider timeout.
- [ ] Return a provider rate failure.
- [ ] Flood perception updates and verify bounded inference.
- [ ] Flood events and verify bounded queues.
- [ ] Stop during movement.
- [ ] Stop during model inference.
- [ ] Verify trace linkage from event to outcome.
- [ ] Scan logs and fixtures for secrets.

## Acceptance criteria

Phase 11 is complete when:

1. Each expected failure has a tested response.
2. Reconnect never resumes stale work.
3. Queue growth is bounded.
4. Graceful shutdown completes within its limit.
5. Model and memory failures do not destroy the body session.
6. Operators can reconstruct each important decision.
7. Logs and fixtures contain no secrets.
8. Production load tests remain within configured CPU, memory, rate, and cost limits.
9. A 24-hour production soak has an exact-cost report and an activity-weighted
   daily projection for each configured model.

## Out of scope

Do not add distributed actor clusters or multi-region execution in this phase.
