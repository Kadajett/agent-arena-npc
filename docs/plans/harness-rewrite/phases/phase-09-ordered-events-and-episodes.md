# Phase 09: Ordered Events and Episodes

State: In progress

## Goal

Consume the backend ordered event stream. Replace observation-derived events when authoritative events are available. Build deterministic episode facts and summaries.

## Required result

The harness receives ordered game events with stable sequence data. The PerceptionActor uses these events for tactical facts. Reducers produce useful episode facts without choosing behavior.

## Dependencies

The backend must expose an ordered event transport or captured event fixtures.

Phase 03 must define the event types and the temporary observation-difference adapter.

Phase 08 must provide episode summary storage.

## Backend delivery: 2026-08-12

Agent Arena issue #1 was implemented by merged PR #33 and extended by PR #34.
Production now exposes `arena_history`, backed by the durable indexed Reldens
chat ledger. It provides:

- ascending events within each cursor-paged response;
- numeric `after` and `before` cursors;
- ISO-8601 `since` and `until` filters;
- a deterministic factual reducer;
- persisted movement, combat, scene, communication, inventory, item, trade,
  dialogue, progression, and skill history;
- lossless unknown engine events;
- complete ordered duel replay persistence.

The Rust typed MCP adapter now decodes this contract and preserves unknown
fields only at the external protocol boundary. `history.read_requested`,
`history.read_completed`, and `history.read_failed` expose safe operational
facts without logging history payloads.

A production round trip moved Cassian from tile `(17,17)` to `(16,11)`. The
authoritative observation confirmed the destination. `arena_history` then
returned movement event `95420` with the correct scene and a decision ID. A
separate read returned 50 events across two scenes with deterministic totals
for movement and damage.

The backend currently generates a new `decisionId` after the tool returns. It
cannot preserve the harness decision, packet, action, or correlation IDs because
mutation schemas accept none of them. This blocks complete causal joining and is
reported as [Agent Arena issue #45](https://github.com/Kadajett/agentArena/issues/45).

## Event source interface

Define one event input interface:

```rust
trait EventSource {
    async fn next_event(&mut self) -> anyhow::Result<GameEvent>;
}
```

Implement at least two adapters:

- a backend transport adapter;
- a fixture adapter for tests and replay.

The backend adapter can use SSE, WebSocket, NATS, Redis Streams, or another ordered transport. The rest of the runtime must not depend on the transport choice.

## Event requirements

Events must include the available values from this list:

- sequence number;
- event ID;
- correlation ID;
- character ID;
- scene;
- entity IDs;
- event time;
- event kind;
- typed event data.

Support these event categories:

- spawn and despawn;
- movement and movement failure;
- damage and heal;
- death and respawn;
- loot drop and pickup;
- item use;
- skill use;
- level and experience change;
- combat start and end;
- scene enter and leave;
- chat;
- interaction.

## Ingestion tasks

- [x] Connect and authenticate to the durable history source through typed MCP.
- [ ] Track the last accepted sequence.
- [ ] Reject duplicate events.
- [ ] Detect sequence gaps.
- [ ] Report out-of-order events.
- [ ] Reconnect with bounded backoff.
- [ ] Resume from the last accepted history cursor.
- [ ] Invalidate tactical state after an unrecoverable gap.
- [ ] Keep the recent event window in memory.
- [ ] Keep global event storage outside the harness.

The tactical runtime must not query an OLAP database during combat.

The initial backend adapter will use bounded cursor polling over
`arena_history`. This is a compatibility transport, not an OLAP query: it reads
the character's indexed operational ledger. Keep it behind `EventSource` so a
future push stream can replace polling without changing reducers or actors.

## Compatibility transition

Run authoritative events and derived events together during a comparison period.

- [ ] Mark the source of each event.
- [ ] Compare derived events with authoritative events.
- [ ] Record mismatches.
- [ ] Prefer authoritative events after parity is proven.
- [ ] Keep derived observation events as a fallback for missing backend categories.
- [ ] Remove a derived category only after its authoritative replacement passes tests.

## Combat reducer tasks

Build a deterministic combat episode reducer.

Track:

- start and end time;
- starting and current health;
- kills;
- hostile spawns;
- current hostiles;
- damage dealt;
- damage received;
- recent damage windows;
- loot collected;
- items used;
- movement failures;
- death and respawn;
- spawn-after-kill pairs.

The reducer can state:

```text
Three kills were followed by nearby hostile spawns within two seconds.
```

The reducer must not state:

```text
The player must flee.
```

## Episode summary tasks

- [ ] Define episode start and end rules.
- [ ] Produce a typed deterministic episode record.
- [ ] Create a concise natural-language summary outside the tactical hot path.
- [ ] Persist meaningful summaries through MemoryActor.
- [ ] Link summaries to their exact event range.
- [ ] Keep raw events out of semantic memory.
- [ ] Notify StrategistActor when an important episode ends.

## Tests

- [ ] Ordered events reduce deterministically.
- [ ] Duplicate events do not change state twice.
- [ ] A sequence gap is visible.
- [ ] Reconnect resumes correctly when supported.
- [ ] A kill followed by a nearby spawn increments the pair count.
- [ ] An unrelated spawn does not increment the pair count.
- [ ] Damage windows expire correctly.
- [ ] Loot and item use appear in the episode.
- [ ] Death closes or marks the episode correctly.
- [ ] Observation-derived and authoritative events match for captured cases.
- [ ] The reducer contains no behavior choice.

## Acceptance criteria

Phase 09 is complete when:

1. The runtime consumes ordered backend events through `EventSource`.
2. Sequence gaps and duplicates have explicit behavior.
3. Tactical facts use authoritative events for supported categories.
4. Combat episodes summarize repeated spawns and other trends.
5. Episode summaries link to exact event ranges.
6. The tactician receives reduced facts instead of a large raw log.
7. Reducer and transition tests pass.

## Out of scope

Do not build the backend event warehouse in this repository.
