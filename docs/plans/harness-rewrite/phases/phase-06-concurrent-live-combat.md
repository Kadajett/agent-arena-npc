# Phase 06: Concurrent Live Combat

State: In progress

## Goal

Prove the fast tactical path in a live or controlled Agent Arena session. Prove that slow reasoning does not stop movement, perception, or combat reactions.

## Required result

Guy can move, fight, heal, loot, disengage, and recover from movement problems through the Rust tactical path while a slow strategic task runs.

## Dependencies

Phases 02 through 05 must pass their contract and local harness tests.

Live mutation controls from the [Local Test Strategy](../testing/local-test-strategy.md) must be active.

## Rollout modes

Support these modes during this phase:

1. `Observe only`: Rust reads and records. It does not mutate.
2. `Shadow tactical`: Rust decides and validates. It does not execute.
3. `Controlled tactical`: Rust executes in selected scenes or test cases.
4. `Full tactical`: Rust owns immediate actions for the test character.

If useful, keep the TypeScript strategist while Rust owns perception, tactics, and body execution. Use a typed bridge. Do not let both runtimes mutate the same body.

## Concurrency tasks

- [ ] Start a strategic model task that lasts at least 30 seconds.
- [ ] Continue perception updates during that task.
- [ ] Run several tactical decisions during that task.
- [ ] Execute tactical preemption during that task.
- [ ] Keep the MCP session alive during model failures.
- [ ] Record a complete causal trace for each decision.

## Live combat cases

- [ ] One enemy.
- [ ] Multiple enemies.
- [ ] Target death.
- [ ] Enemy respawn.
- [ ] Repeated damage.
- [ ] Health decline.
- [ ] Potion use.
- [ ] Loot before retreat.
- [ ] Skill use.
- [ ] Invalid skill refusal.
- [ ] Equipped weapon context.
- [ ] Death.
- [ ] Respawn.
- [ ] Retreat through a reachable exit.
- [ ] Path blockage during combat.
- [ ] Movement interruption by a tactical packet.

## Movement cases

- [ ] Long movement continues without blocking perception.
- [ ] The tactician stops long movement after danger appears.
- [ ] The tactician changes local direction.
- [ ] The body resumes the strategic navigation goal after danger.
- [ ] A movement stall produces a new tactical wake.
- [ ] Unstick remains a last resort.

## Safety rules

Use a dedicated test character.

Set a maximum run time and cost.

If full live combat is active and health or maximum health is unknown, stop the run. Before disconnect, the BodyActor must set backend tactics to `flee` and `semi_auto`. The fallback must have a bounded deadline and a complete causal trace.

Do not use `arena_stop` as a combat disengage operation. It stops movement only.

Do not run autonomous production combat while the backend omits health. Track this defect in [Agent Arena issue 31](https://github.com/Kadajett/agentArena/issues/31).

Stop the test if identity does not match the expected character.

Controlled production mutation is default-deny and uses the typed
`PlayerRuntime::submit_controlled_packet` interface. Its caller supplies a
`ControlledPacketRequest` containing exact character id, player name, scene,
and a `TacticalProposal`; the runtime creates the packet id, decision id,
revisions, scene, and timestamp. Release requires all of the following:

- `NPC_TACTICAL_ROLLOUT_MODE=controlled`;
- `NPC_ALLOW_LIVE_MUTATION=true`;
- a positive `NPC_LIVE_ACTION_BUDGET` (default `0`);
- exact configured `NPC_LIVE_EXPECTED_CHARACTER_ID`,
  `NPC_LIVE_EXPECTED_PLAYER_NAME`, and `NPC_LIVE_ALLOWED_SCENE` values;
- no more than `NPC_LIVE_MAX_ACTIONS_PER_PACKET` actions (default `1`);
- `valid_for_ms` no greater than `NPC_LIVE_PACKET_MAX_AGE_MS` (default
  `1000`);
- acceptance by the real `BodyActor` validator against its latest frame.

The supervisor consumes the process-local action budget after read-only body
validation succeeds and before it sends the packet to `BodyActor`. A gate or
preflight-validation rejection does not consume budget. Once consumed, budget
is never replenished after a mailbox, race-time body rejection, or action
failure. `PlayerRuntime::validate_tactical_packet` performs
the same exact runtime assertion, packet-limit check, and real-body validation
without consuming budget or executing an action. Each attempt emits
`runtime.controlled_packet_decided`, correlated by runtime-created decision and
packet ids, followed by the ordinary `body.packet_*` and `body.action_*` chain
when released.

The controlled diagnostic waits for `BodyActor` to report the same packet id
with a terminal `completed` status. A successful mailbox send is not a live
test pass. The body status also exposes the last terminal packet id and status
for this bounded verification path.

Some production observations identify the connected player but omit `alive`.
Unknown life state is not treated as death. The body rejects an action when the
backend explicitly reports `alive=false` or a recent player death.

The one-shot live diagnostic is intentionally environment-heavy so a copied
command cannot mutate with defaults. From `rust-harness/`, after independently
verifying the character and scene, an explicit Stop diagnostic is:

```sh
NPC_TACTICAL_ROLLOUT_MODE=controlled \
NPC_ALLOW_LIVE_MUTATION=true \
NPC_LIVE_ACTION_BUDGET=1 \
NPC_LIVE_MAX_ACTIONS_PER_PACKET=1 \
NPC_LIVE_PACKET_MAX_AGE_MS=500 \
NPC_LIVE_EXPECTED_CHARACTER_ID=guy \
NPC_LIVE_EXPECTED_PLAYER_NAME='EXACT DEDICATED TEST PLAYER NAME' \
NPC_LIVE_ALLOWED_SCENE='EXACT SCENE ID' \
NPC_CONTROLLED_PACKET_JSON='{"intent":"stop","actions":[{"type":"stop"}],"valid_for_ms":500,"abort_if":["scene_changed"],"rationale":"controlled production diagnostic"}' \
cargo run --bin controlled-packet
```

This command is documentation, not authorization to run it. The operator must
provide the already-configured Arena/OpenRouter credentials. Controlled model
proposals remain record-only. In `full` mode, a tactical proposal returns to the
PlayerSupervisor instead of going directly to the BodyActor. The supervisor
requires the same mutation opt-in, exact configured character id, exact player
name, exact scene, packet limits, real-body validation, and process-local action
budget. It consumes budget before it sends an accepted packet to the body.

The tactical event records `runtime_gate_pending`; it does not claim that a
mailbox send mutated the game. `runtime.model_packet_decided` records the actual
gate result and remaining budget. The default budget is zero. Full mode cannot
mutate with default configuration.

Stop the test after repeated reconnect failures.

Keep the TypeScript production character on its current harness during controlled tests.

## Metrics

Record:

- perception update rate;
- tactical decision latency;
- proposal parse success;
- packet validation results;
- MCP call latency;
- stale result count;
- damage taken;
- damage dealt;
- healing use;
- loot collected;
- movement stalls;
- deaths;
- model cost.

## Acceptance criteria

Phase 06 is complete when:

1. Guy reacts to combat without waiting for a strategic task.
2. Perception updates continue during long movement and long inference.
3. A tactical packet can interrupt movement.
4. Legal skills and items execute through the BodyActor.
5. Invalid actions do not reach MCP.
6. A model failure does not log Guy out.
7. Each live action has a reconstructable trace.
8. The controlled live test suite passes repeatedly.

## Out of scope

Do not migrate full personality, relationships, or durable memory in this phase.
