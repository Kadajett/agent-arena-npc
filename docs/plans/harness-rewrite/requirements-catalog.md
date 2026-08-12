# Requirements Catalog

This page preserves requirements that apply across more than one phase.

## Objective

Build a long-running AI player harness for Agent Arena.

The player must think over long time periods. The player must also react quickly to combat, danger, loot, movement failure, and other immediate changes.

The system uses two independent forms of cognition:

- Strategic cognition is slow, persistent, personality-driven, and concerned with long-term behavior.
- Tactical cognition is fast, low-cost, short-context, and concerned with the next few actions.

Neither form of cognition can block the other.

## Technology

- Use Rust for the new harness.
- Use Ractor for actors, supervision, isolation, and message passing.
- Use Tokio for asynchronous input and output.
- Use Rig for model-provider access and structured model output.
- Use OpenRouter for the first strategic and tactical model adapters.
- Use the existing MCP gateway for all game access.
- Do not create a second game server in the harness.

## Actor responsibilities

### PlayerSupervisor

- Start and stop the player actor tree.
- Receive supervision events.
- Apply only tested restart rules.
- Prevent a silent half-functional player.
- Keep gameplay decisions out of the supervisor.

### PerceptionActor

- Read typed MCP observations and events.
- Normalize coordinates, health, entities, inventory, equipment, movement, combat, doors, and maps.
- Build the tactical frame.
- Keep bounded recent events and outcomes.
- Build deterministic episode facts.
- Make no gameplay choice.

### TacticianActor

- Use the newest tactical frame and strategic intent.
- Run short, stateless model calls.
- Wake after material events and bounded combat heartbeats.
- Keep its mailbox active during inference.
- Discard stale results.
- Send tactical proposals to the BodyActor.
- Handle no normal conversation.

### StrategistActor

- Use the character persona.
- Own goals, plans, relationships, promises, exploration, conversation, and economic intent.
- Publish revisioned strategic intent.
- Run slowly without blocking the tactician.
- Send social and strategic commands through the BodyActor.
- Never own a mutable MCP connection.

### BodyActor

- Own the mutable MCP gateway.
- Bind the character identity.
- Enforce capabilities.
- Enrich tactical proposals with runtime metadata.
- Validate actions.
- Execute actions in order.
- Stop stale or invalid packets.
- Support preemption and cancellation.
- Monitor movement concurrently.
- Record action outcomes.

### MemoryActor

- Persist working and durable strategic memory.
- Persist relationships, knowledge, and episode summaries.
- Retrieve typed memory for the strategist.
- Keep writes outside tactical work.
- Migrate useful TypeScript memory.

### TelemetryActor

- Record actor, decision, validation, action, and failure events.
- Preserve causal identifiers.
- Emit structured logs and metrics.
- Keep secrets out of output.

## Tactical frame requirements

The tactical frame must include available values for:

- revisions and generation time;
- current scene;
- pixel and tile position;
- health and maximum health;
- level and experience;
- class path;
- legal combat actions and cooldowns;
- inventory and equipped items;
- current target;
- automatic combat state and mode;
- visible players, NPCs, enemies, and drops;
- structured local map;
- ASCII local map;
- reachable exits;
- movement state;
- recent events;
- recent action outcomes;
- current combat episode facts;
- strategic intent.

Use `Option<T>` for data that the backend does not supply.

## Tactical proposal requirements

The model can request:

- continue;
- attack;
- use a legal skill;
- use a carried item;
- pick up a visible drop;
- reposition;
- disengage;
- stop.

The model must not supply identity or runtime metadata.

The model must not call MCP tools directly.

## Scheduling requirements

Wake the tactician after material changes. Use latest-value semantics.

Do not queue a model request for each perception poll.

Use a bounded combat heartbeat. Use a low idle ceiling.

Run only one strategic inference task under normal conditions.

Do not wake the strategist for ordinary combat noise.

## Movement requirements

- Models choose destinations and tactical direction.
- The runtime handles collision, path checks, movement progress, and cancellation.
- Perception must continue during movement.
- The tactician can interrupt long movement.
- The runtime can find an adjacent reachable interaction tile.
- The runtime can report a blocked or stalled path.
- `arena_unstick` is a last resort.
- A local tactical override must not erase the strategic navigation goal.

## Combat requirements

- Backend data is authoritative for legal skills and equipment effects.
- The harness must not fake weapon effects.
- The tactician must see health, targets, damage, legal actions, loot, escape paths, respawns, and recent outcomes.
- The model decides whether to fight, heal, loot, chase, switch targets, or disengage.
- Safe game-level automatic combat can remain as a temporary lowest reflex layer.

## Memory requirements

- Personality cannot be rewritten by memory.
- Goals and plans survive restarts.
- Relationships require evidence.
- Hearsay and firsthand knowledge remain separate.
- Short-lived notes expire.
- Raw events remain outside semantic memory.
- Meaningful episode summaries can enter durable memory.
- Tactical inference does not query long-term memory by default.

## Observability requirements

Each meaningful action must link:

```text
events
→ frame
→ strategy
→ inference
→ proposal
→ packet
→ validation
→ MCP calls
→ outcomes
→ later events
```

Record prompt version, model, provider, latency, tokens, and cost where available.

## Backend compatibility requirements

The harness must run before all backend improvements exist.

Expected backend additions include:

- richer structured observation;
- a structured map;
- explicit coordinate systems;
- health and maximum health;
- level and experience;
- class path;
- legal actions and cooldowns;
- equipment;
- movement and combat state;
- authoritative outcomes;
- ordered events with stable identifiers.

Keep each compatibility gap explicit. Do not hide a gap with an invented value.

## Final success example

The strategist can set this direction:

```text
Collect eight spider silks and return to town.
Put survival before the haul.
Keep one potion in reserve.
```

The tactician can then see these facts:

```text
Health changed from 100 to 72 to 51.
Three spiders died.
Three nearby spiders appeared after the kills.
Two spiders now target Guy.
Guy holds five silk and two healing potions.
The west path is reachable.
```

The tactician can decide to finish one target, take loot, use one potion, and disengage west.

No deterministic rule states that three spiders require retreat.

The strategist can continue a separate long model call during all of these actions.
