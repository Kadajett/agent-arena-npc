# Strategic Planning and Navigation

The strict brain protocol and movement-ownership contract are defined in [Brain JSON Protocol and Coordination State](brain-json-protocol-and-coordination.md).

When Body reaches a destination, it emits a typed navigation-arrival fact. Perception forwards the fact to Strategist. Strategist then decides the next long-horizon action. Arrival alone does not mark an inspection, conversation, pickup, or other plan step complete.

State: In progress

## Purpose

This document defines the seam between long-horizon cognition and physical travel.

The StrategistActor decides where the character wants to go and why. The navigation module decides how to move the body to that destination. The BodyActor remains the only actor that can mutate the game.

## Fixed ownership

The strategic planning subsystem owns:

- the long-term goal;
- the ordered plan;
- proposed plan-step content and completion conditions;
- proposed re-evaluation conditions;
- re-evaluation conditions;
- the navigation destination and its purpose.

The navigation module owns:

- path preflight;
- selection of a grounded local waypoint;
- `arena_move_to` calls for normal travel;
- `arena_enter_door` calls for door crossings;
- concurrent movement monitoring;
- arrival detection;
- stall classification;
- bounded physical recovery;
- tactical preemption;
- navigation resumption after danger.

The navigation module must not select a character goal. It must not decide that loot, combat, trade, or exploration is valuable.

## Deep interface

The caller must submit one navigation mission. The caller must not orchestrate directional steps.

An illustrative interface is:

```rust
struct NavigationMission {
    mission_id: NavigationMissionId,
    strategic_revision: u64,
    destination: NavigationDestination,
    reason: String,
    completion: NavigationCompletion,
}

enum NavigationDestination {
    VisibleEntity { entity_id: String },
    SceneTile { scene: String, tile: TilePosition },
    SceneExit { scene: String },
    NamedPlace { scene: String, name: String, tile: Option<TilePosition> },
}

enum NavigationCommand {
    Pursue(NavigationMission),
    Cancel(NavigationCancelReason),
}
```

The exact types can change during implementation. The ownership rules must not change.

## Movement tool rules

Use `arena_move_to` for an ordinary reachable destination in the current scene.

Do not implement ordinary travel as a sequence of `arena_move` calls. A directional move is a low-level recovery tool. It is not the primary navigation tool.

Use `arena_enter_door` for a doorway. The backend excludes door tiles from ordinary pathfinding. The door tool owns approach and crossing behavior.

Use `arena_check_path` when the destination is not already proved reachable by the current authoritative frame.

Use `arena_stop` when a tactical packet preempts active travel.

Use `arena_unstick` only after bounded movement attempts produced explicit stall or blocked evidence.

## Mission lifetime

A navigation mission can outlive one tactical decision and one model call. It can span many perception frames.

The BodyActor must retain the mission while it performs local movement. The BodyActor must compare the mission's strategic revision with current strategic state before each new physical attempt.

A tactical danger response can suspend or replace current physical movement. The mission remains available for resumption after danger unless one of these events occurs:

- the strategist cancels or replaces the mission;
- the destination becomes materially invalid;
- the scene or route evidence proves that the mission cannot continue;
- the session reconnects and the runtime cannot revalidate the mission;
- the mission completes.

## Strategic plan contract

The strategist must not publish only an objective and one vague subgoal. It must maintain an ordered plan with typed state.

Each plan step must contain:

- a stable runtime-owned identifier;
- an action or information objective;
- a state such as `next`, `doing`, `done`, or `blocked`;
- a completion condition;
- evidence for the last state change;
- an attempt count;
- re-evaluation conditions.

The model proposes plan content. The runtime owns identifiers, revisions, timestamps, and evidence attachment.

The model output schema therefore does not contain step status, attempt count, or evidence fields. When a proposal retains a step with the same description, the runtime preserves its current status, attempt count, and observed evidence. A model completion claim is recorded as a claim and cannot set the durable goal-complete fact.

The strategist must receive:

- the current typed goal and plan;
- the current strategic intent;
- current world facts;
- bounded relevant episode memories;
- bounded relevant semantic memories with provenance;
- relevant relationships;
- recent plan outcomes and blocked reasons.

Current working state must not depend on semantic retrieval. SQLite remains the source of truth. A local retrieval index is derived state.

## Reasoning

Explicit model reasoning is permitted for the strategist. It is not required for the tactician.

The runtime must record:

- requested reasoning mode and effort;
- actual model and provider;
- reasoning tokens reported by OpenRouter;
- input, cached-input, output, and total tokens;
- exact reported cost;
- latency;
- decision, frame, strategy, plan, and character identifiers.

The runtime must not record private reasoning content.

### Runtime configuration

Reasoning is configured independently for the strategist and tactician through
the role-prefixed `REASONING_ENABLED`, `REASONING_EFFORT`, and
`REASONING_EXCLUDE` environment settings. The strategist defaults to medium
reasoning with a 4,000-token completion budget. The tactician defaults to no
reasoning so the live reaction path remains latency-oriented.

OpenRouter counts reasoning and the visible structured answer against the same
completion budget. Enabled minimal, low, medium, and high effort therefore
require at least 512, 1,000, 2,000, and 4,000 completion tokens respectively.
Configuration fails at startup when the budget is smaller. Known model and
effort mismatches also fail at startup. Other model IDs are sent with provider
parameter enforcement, so unsupported settings fail visibly at the provider
boundary.

Rig's OpenRouter adapter builds the request and carries the additional
reasoning parameters. The harness does not implement a second provider client.
Rig's prompt-caching extension emits the provider's explicit cache breakpoint
inside the final system-message content block. It does not send a top-level
cache-control field. A stable per-character and per-role `session_id` supplies
independent routing stickiness. Neither mechanism justifies padding a short
prompt merely to reach a provider cache threshold.
Each call records its normalized finish reason and emits a warning when the
shared completion budget is exhausted. The finalized OpenRouter generation
record supplies authoritative native reasoning and cache token counts, which
are reconciled into the per-character usage totals.

## Required causal chain

Every long journey must support this trace:

```text
memory recall
    -> strategic inference
    -> strategic plan revision
    -> navigation mission
    -> path preflight
    -> move_to or enter_door attempt
    -> perception progress
    -> arrival, preemption, stall, or failure
    -> plan-step evidence
    -> strategic re-evaluation when required
```

Do not log raw secrets, private model reasoning, full memory text, or unbounded MCP payloads.

## Acceptance tests

The implementation must prove these behaviors:

1. The strategist produces and persists a multi-step plan.
2. A restart restores the current step and its attempt count.
3. Relevant memory reaches the strategist without crossing character identity.
4. Ordinary travel calls `arena_move_to`, not repeated `arena_move`.
5. Door travel calls `arena_enter_door`.
6. Perception continues while a movement request is in flight.
7. Tactical danger stops physical movement but does not erase the strategic mission.
8. Safe state resumes a still-valid mission.
9. A stalled mission produces typed evidence and a strategic wake.
10. Every mission and attempt has a complete causal trace.
11. A live production test proves movement through more than one local waypoint.
12. A live production test proves that the character makes measurable progress toward a long-term plan.

## Implementation note: 2026-08-12

Strategic planning now uses the `strategist/v5` contract. Each strategic wake requests typed working state and bounded relevant memories from `MemoryActor` before model inference. Accepted plan content and strategic intent are saved atomically through the memory actor. A changed grounded navigation goal creates one body-owned navigation mission; repeated reflections do not restart an unchanged mission.

The recall interface is deliberately deeper than its first retrieval adapter. The first adapter performs deterministic, provenance-aware lexical ranking over a bounded recent candidate set inside the memory store. It requires no embedding provider call and does not add untracked model cost. The planned Rig vector index can replace this adapter behind the same `RecallQuery -> StrategicRecall` interface.

The BodyActor now accepts a typed `NavigationMissionRequest`. A mission can
contain a final named destination and an optional cross-scene waypoint route.
The body assigns mission and attempt identifiers, performs path preflight, uses
`arena_move_to` for ordinary tiles, uses `arena_enter_door` for transitions,
and monitors progress from perception frames without blocking its mailbox.
Equivalent mission requests are idempotent. Tactical packets pause active
travel with `arena_stop`; the retained mission resumes after tactical work.
Stalls receive bounded local retries, and terminal failures are forwarded to
the strategist as a blocked-goal wake. Unstick remains outside the ordinary
attempt path.

Operational state exposes the active mission ID and state. Telemetry records
mission starts, attempts, waypoints, pauses, resumes, duplicate suppression,
retries, and terminal results. Snapshot counters distinguish `move_to`, door,
preemption, arrival, failure, retry, and supersession activity.
