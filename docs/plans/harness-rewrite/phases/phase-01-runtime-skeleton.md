# Phase 01: Runtime Skeleton

State: In progress

## Goal

Build a safe Rust runtime skeleton for one player. Prove that all actors can run concurrently. Do not mutate the live game in this phase.

## Required result

The player runtime starts one supervised actor tree. The actors exchange typed messages. Model work does not block actor mailboxes. The runtime stops cleanly.

## Scope

This phase includes:

- the Rust crate and module layout;
- environment configuration;
- the Guy character sheet;
- typed capabilities;
- the PlayerSupervisor;
- the BodyActor;
- the PerceptionActor;
- the TacticianActor;
- the StrategistActor;
- the MemoryActor;
- the TelemetryActor;
- the typed hot blackboard;
- core messages and domain types;
- the `Brain<I, O>` interface;
- the Rig and OpenRouter adapter;
- versioned prompt files;
- clean shutdown;
- initial actor failure tests.

Session transport is part of Phase 02.

## Current implementation

The repository already contains these items:

- [x] A buildable `rust-harness` crate.
- [x] The six child actors and the PlayerSupervisor.
- [x] Typed actor messages.
- [x] A typed in-process blackboard.
- [x] Non-blocking tactical inference work.
- [x] Stale tactical result detection.
- [x] A tested tactician restart path.
- [x] The `Brain<I, O>` interface.
- [x] A Rig and OpenRouter JSON brain adapter.
- [x] Versioned tactical and strategic prompts.
- [x] Typed tactical, strategic, world, execution, and memory data.
- [x] Clean shutdown tests.

The following work remains:

- [ ] Add a scripted strategist brain for concurrency tests.
- [ ] Prove that a slow strategist and a fast tactician run at the same time.
- [ ] Add an actor dependency diagram to the crate documentation.
- [ ] Define the process-level failure rule for each child actor.
- [ ] Confirm that no Phase 1 code can create a live MCP transport.

## Interfaces

The phase must stabilize these interfaces:

```rust
trait Brain<I, O> {
    async fn decide(&self, input: &I) -> anyhow::Result<O>;
}
```

```rust
struct HotBlackboard {
    tactical_frame: TacticalFrame,
    strategic_intent: StrategicIntent,
    current_packet: Option<ActionPacket>,
}
```

The blackboard implementation can use `ArcSwap` or an actor-owned snapshot. Callers must not depend on that implementation choice.

## Configuration

Support these environment variables:

- `ARENA_API_KEY`;
- `ARENA_MCP_URL`;
- `OPENROUTER_API_KEY`;
- `NPC_CHARACTER`;
- `NPC_STRATEGIST_MODEL`;
- `NPC_TACTICIAN_MODEL`;
- `NPC_MEMORY_PATH`;
- `NPC_TACTICAL_MAX_HZ`;
- `NPC_IDLE_TACTICAL_HZ`;
- `NPC_TACTICIAN_TEMPERATURE`;
- `NPC_TACTICIAN_MAX_OUTPUT_TOKENS`;
- `RUST_LOG`.

Do not print secret values. Validate rates and numeric limits at startup.

## Supervision rule

The supervisor can restart the TacticianActor because the dependency restoration path has a test.

Use process-level restart for other failed child actors until their restoration paths have tests. Do not leave a silently half-functional player.

## Tests

Add or keep these tests:

- [x] All actors start.
- [x] The supervisor reports all active actors.
- [x] Typed request and reply messages work.
- [x] The tactician failure reaches the supervisor.
- [x] The supervisor restarts the tactician.
- [x] Clean shutdown stops the tree.
- [ ] A 30-second strategist task does not block tactical messages.
- [ ] A model failure does not stop the player runtime.

## Acceptance criteria

Phase 01 is complete when:

1. All required actors start under one PlayerSupervisor.
2. Each actor owns its mutable state.
3. Model work runs outside actor handlers.
4. The tactician can receive new frames while inference runs.
5. The player runtime stops without orphan actors.
6. A failed model call does not create an MCP mutation.
7. All Phase 01 tests pass.

## Out of scope

Do not add live MCP access, live combat, persistent memory, or production reconnect logic in this phase.
