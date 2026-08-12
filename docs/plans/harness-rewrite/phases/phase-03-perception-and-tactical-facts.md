# Phase 03: Perception and Tactical Facts

State: In progress

## Goal

Convert MCP observations into a small, authoritative, and versioned tactical frame.

## Required result

The TacticianActor receives exact facts about the current physical situation. It does not parse the full MMO response.

## Dependencies

Phase 02 must provide typed observations and typed map results.

## Position and coordinate tasks

- [x] Define one explicit position type with pixel and tile coordinates.
- [x] Convert backend coordinates in one normalization module.
- [x] Reject invalid numeric coordinates.
- [x] Keep unknown coordinates as `None`.
- [x] Calculate relative tile positions.
- [x] Calculate distances from authoritative positions.

The model must not guess whether a coordinate uses pixels or tiles.

## Observation normalization tasks

- [x] Normalize the current scene.
- [x] Normalize player existence and life state.
- [x] Normalize health and maximum health.
- [x] Normalize level and experience.
- [x] Normalize class path.
- [x] Normalize legal combat actions and cooldowns.
- [x] Normalize inventory and equipped items.
- [x] Normalize visible players, NPCs, and enemies.
- [x] Normalize drops.
- [x] Normalize doors.
- [x] Normalize the current target when the backend supplies it.
- [x] Normalize movement state when the backend supplies it.
- [x] Normalize the backend battle state, automatic combat style, and control mode.
- [x] Filter engine chatter and feeling pings from player chat.
- [x] Keep real player speech as typed dialogue with source and time.
- [x] Classify backend melody stage directions as music rather than player speech.
- [x] Suppress repeated lines from overlapping observation windows.

Do not invent missing backend fields.

## Map tasks

- [x] Build a structured local map from the MCP map result.
- [x] Preserve exact origin, size, and tile positions.
- [x] Mark traversable, blocked, unlocked door, locked door, and unknown tiles.
- [x] Produce an ASCII view from the same structured map.
- [x] Add exact entity and ground-drop data beside the ASCII view.
- [x] Calculate reachable exits and local path lengths when data permits.
- [x] Keep map calculations free of fight, flee, heal, and loot decisions.

Use these ASCII symbols initially:

```text
@ = controlled player
S = hostile entity
* = ground item
D = door
L = locked door
# = blocked tile
. = traversable tile
```

## Current backend location contracts

`arena_render_map` does not show ground items. `arena_observe.drops` is the source for ground items. Each available drop has a drop ID, an item key, and pixel coordinates. The normalizer converts the pixel coordinates to an exact tile and a relative tile offset. It also puts `*` on the derived ASCII map when the drop is inside the local view.

The backend map uses `E` for an enemy. The normalizer keeps each enemy in the structured entity list. It uses `S` in the tactical ASCII map. The entity list also states whether the backend reports the enemy as alive. The harness must not attack a dead enemy only because the object is still present.

The backend survey uses the exact tag `<LOCKED DOOR>`. The tag applies to a way out. It does not name or expose an item behind the door. The structured map records the door tile, lock state, destination, and required key when the backend supplies those facts. Hidden content stays unknown.

The backend battle report is authoritative when it is present. The tactical frame records the active state, style, mode, aggressors, enemy health, cumulative damage dealt, and damage events from the last five seconds. The battle HP string can supply current and maximum health when the normal player state does not contain those fields. A missing battle report stays unknown. It does not become `false`.

The current backend reports class identity and legal skills at the top level of `arena_observe`. `classPath` contains the world-reported label and level. `skills` is either `null`, an empty list, or a list of legal action keys. `null` means that the world has not supplied the authoritative list. An empty list means that the world supplied a list with no legal skills. The Rust compatibility reader may accept the older `ownPlayer.state.combatActions` shape only when the top-level list is absent.

The current `skills` list establishes legality. It does not report a live cooldown or immediate availability value. The tactical frame therefore keeps availability, cooldown, and target kind unknown for these entries. It must not turn a legal skill into an invented ready skill with a zero cooldown.

The backend can limit the object list in a dense scene. The tactical frame records the reported total object count, the listed object count, and whether the list is incomplete. Players are a separate list. The frame does not interpret a short object list as an empty area.

Each visible entity has an exact identifier, kind, tile, relative tile, distance, life state, hostility state, target state, and merchant state when the backend supplies these fields. The structured entity list is the source of truth. The ASCII map is a compact model aid.

The observation chat window can contain old lines from the prior poll. The normalizer compares overlapping windows and emits only new dialogue. It preserves scene, global, private, and team channel provenance from the backend message type. It filters engine keys such as `chat.joinedRoom`. It also filters feeling emoji because they are status signals and not speech. It classifies the backend music stage direction as a melody. Telemetry records per-channel and melody counts. It does not record dialogue or melody text.

The local map reducer uses breadth-first search to calculate the path length to each known unlocked door. It uses only tiles that the backend marks as walkable. It does not call an MCP tool. It does not report a locked door or a door with unknown walkability as reachable.

## Event accumulator tasks

- [x] Keep 10 to 30 seconds of detailed events in memory.
- [x] Keep a bounded number of important world events.
- [x] Keep recent action outcomes.
- [x] Derive events from observation differences until Phase 09 supplies ordered backend events.
- [x] Mark derived events as derived.
- [x] Prevent duplicate derived events.
- [x] Ingest the current backend battle timeline once per event identity.
- [x] Prefer a backend event over an equivalent derived event.

Derived event types must include:

- damage taken and dealt;
- heal and item use;
- enemy seen, spawn, and despawn;
- target change and target death;
- loot drop and pickup;
- movement start, stop, and failure;
- scene enter and leave;
- player death and respawn;
- level and experience change.

The current backend battle timeline supplies ordered damage, death, and kill events. The reducer preserves the backend sequence and timestamp. It removes repeated timeline entries from later observations. Phase 09 will replace the remaining observation-derived events with the full ordered backend event source.

The combat episode reducer counts accepted damage, kills, hostile spawns, current health, and a reused enemy identifier that respawns within two seconds of its recorded kill. It reports those facts in the tactical frame. It does not turn the pattern into fight, flee, heal, or loot advice.

## Revision rules

Increment `world_revision` only for a meaningful world change.

Increment `perception_revision` for each accepted normalized snapshot.

Use separate map and inventory revisions when they help material invalidation.

Define material invalidation in code and tests. Do not invalidate combat because an unrelated chat line arrived.

## Tactical frame

The frame must contain:

```text
revision data
generation time
self state
combat state
visible entities
drops
structured local map
ASCII local map
reachable exits
recent events
recent action outcomes
current strategic intent
current episode facts
```

## Parity tests

- [ ] Compare TypeScript and Rust normalization for the same captured observation.
- [ ] Test an interior room.
- [ ] Test a narrow corridor.
- [ ] Test a large exterior map.
- [ ] Test a door and a multi-tile door.
- [ ] Test a map edge.
- [ ] Test a wall beside an NPC.
- [ ] Test a wall beside an enemy.
- [ ] Test absent health, combat, and equipment fields.
- [ ] Test death, respawn, scene change, and inventory change.

## Acceptance criteria

Phase 03 is complete when:

1. A captured MCP observation produces one deterministic tactical frame.
2. The TypeScript and Rust normalized facts agree for shared fields.
3. The structured map and ASCII map describe the same tiles.
4. Unknown backend data remains unknown.
5. Event and action windows stay bounded.
6. Material changes update the correct revisions.
7. The PerceptionActor makes no gameplay decision.

## Out of scope

Do not execute actions or call a live tactical model in this phase.
