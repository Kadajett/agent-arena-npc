# Phase 04: Body Execution and Movement

State: In progress

## Goal

Make the BodyActor the only safe mutation path. Add validation, preemption, cancellation, and concurrent movement monitoring.

## Required result

A typed tactical proposal becomes a runtime-owned action packet. The BodyActor validates each action before it calls MCP. It stops a packet when the world changes.

## Dependencies

Phase 02 must provide the typed gateway.

Phase 03 must provide authoritative frames and revision rules.

## Type split

Split model output from runtime metadata.

The model returns:

```rust
struct TacticalProposal {
    intent: TacticalIntent,
    actions: Vec<TacticalAction>,
    valid_for_ms: u32,
    abort_if: Vec<AbortCondition>,
    rationale: Option<String>,
}
```

The runtime creates:

```rust
struct ActionPacket {
    packet_id: ActionPacketId,
    decision_id: DecisionId,
    frame_revision: u64,
    strategic_revision: u64,
    created_at: Instant,
    proposal: TacticalProposal,
}
```

The model and runtime types now have this split. Rig deserializes only `TacticalProposal`. After the stale-response check, the TacticianActor adds the decision ID, packet ID, frame revision, strategic revision, scene, and creation time. A schema test makes sure that model output has no identity or revision fields.

## Validation tasks

- [x] Check packet age and validity duration.
- [ ] Check the minimum valid world revision.
- [x] Check the strategic revision.
- [x] Check player existence and life state.
- [x] Check the current scene.
- [x] Check character capabilities.
- [x] Check target existence in the current frame.
- [x] Check item existence and quantity.
- [x] Check skill legality and availability.
- [x] Check drop existence.
- [x] Reject movement coordinates outside the authoritative local map and tiles that are explicitly not walkable. The production gateway still performs backend path preflight before mutation.
- [x] Return a typed rejection without an MCP mutation.

Validation must not decide whether an action is wise.

## Execution tasks

- [x] Execute actions in packet order.
- [x] Record one outcome for each attempted action.
- [x] Publish outcomes to PerceptionActor and TelemetryActor.
- [x] Check freshness before every action.
- [ ] Check abort conditions between actions.
- [x] Stop the remaining packet after material invalidation.
- [x] Mark replaced packets as superseded.
- [x] Support a higher-priority tactical packet.
- [ ] Keep a strategic navigation goal after a temporary tactical override.

## Preemption example

Packet A contains:

```text
attack
pick up
move north
```

Health then drops materially. Packet B contains:

```text
use potion
stop
move west
```

The BodyActor must stop Packet A. It must validate and start Packet B.

## Movement tasks

- [x] Run path preflight before production `move_to`; do not mutate when the backend reports unreachable.
- [x] Start `arena_move_to` without blocking perception.
- [x] Observe position and movement state concurrently.
- [x] Detect arrival in the pure movement reducer.
- [x] Detect progress in the pure movement reducer.
- [x] Detect a stopped path in the pure movement reducer.
- [x] Classify blocked, stalled, cancelled, interrupted, and scene-transition movement facts.
- [ ] Try a local adjacent reachable tile when the requested interaction needs it.
- [x] Report a visible failure when movement stalls or the backend explicitly fails.
- [ ] Keep `arena_unstick` as a last resort.
- [x] Represent and test a process-local unstick cooldown. Body wiring remains.
- [ ] Allow the tactician to stop, heal, or change direction during movement.

Do not port a 45-second blocking movement loop.

Do not use high-level action repetition as the primary physical stall detector.

## Strategic navigation

Represent long travel as a navigation goal. The strategist owns the destination and reason.

The tactician can override local motion during danger. The BodyActor must resume the strategic navigation goal after danger ends, unless the strategist cancels it.

## Tests

- [x] Valid packet produces the expected body-gateway calls.
- [ ] Invalid target produces no MCP call.
- [x] Missing capability produces no MCP call.
- [x] Expired packet produces no MCP call.
- [ ] Scene change stops a packet midway.
- [ ] Target death stops dependent actions.
- [ ] New hard constraint stops a packet.
- [x] Higher-priority packet preempts the old packet and ignores late completion.
- [x] Reachable movement arrives from an authoritative perception frame or explicit backend arrival.
- [ ] Unreachable movement fails.
- [x] Movement stall is detected and stopped without advancing the packet.
- [ ] Door transition completes.
- [ ] Combat interrupts movement.
- [ ] Unstick is not the first recovery action.

## Current implementation note

The BodyActor now owns the small `BodyGateway` interface. Production uses the character-bound `ArenaGateway`; tests use a recording adapter. An action runs in spawned work and returns a typed completion message, so the actor mailbox remains available for cancellation, health checks, and replacement packets.

Each action has a runtime-generated action ID. That ID is also the MCP correlation ID. Packet, decision, session-generation, frame, strategy, and action-index fields remain attached to the action and terminal outcome. A packet and every attempted action now have explicit terminal telemetry.

Movement fact reduction is now wired into the BodyActor. A successful command acceptance, `moved: true`, or partial `arrived: false` response records that movement started but does not complete the action. The packet advances only after an explicit backend `arrived: true` fact or an authoritative perception frame places the player on the requested tile. Perception-derived stalls fail the action, fail the packet, and issue one typed safety stop; late move completions are ignored by action identity.

Movement emits typed progress, arrival, stall, and stop telemetry containing causal identity and reduced physical facts rather than raw MCP payloads. Recovery selection, doors, strategic route resumption, and complete abort-condition evaluation remain.

## Acceptance criteria

Phase 04 is complete when:

1. No actor other than BodyActor can call a mutating gateway method.
2. Model output contains no runtime metadata.
3. Invalid actions fail before mutation.
4. Freshness checks run between packet actions.
5. Tactical preemption works.
6. Perception continues during movement.
7. Movement can stop before its original destination.
8. All execution and movement tests pass.

## Out of scope

Do not add live tactical model scheduling in this phase.
