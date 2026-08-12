# Phase 02 Live Smoke Test: 2026-08-11

## Scope

This test used a dedicated test character on the live Agent Arena servers.

The test did not store the Arena API key in the repository. The test did not record the backend agent identifier, the MCP session identifier, tool payloads, or speech text in analytics.

## Character

- Character sheet ID: `cassian`
- Player name: `Cassian Vey Unbound`
- Class path: `swordsman`
- Purpose: Phase 02 live integration tests

An interrupted first run registered `Cassian Vey` as a warrior before the class correction. The gateway does not provide a character deletion or class-change operation. That unused character was disconnected. The registration version was incremented, and the corrected swordsman was registered with a new name and idempotency key.

## Passed checks

| Check | Result | Notes |
| --- | --- | --- |
| MCP initialization | Pass | The client captured and reused the MCP session identifier. |
| Agent listing | Pass | The client found or registered the bound character. |
| Agent registration | Pass | The server registered the swordsman test character. |
| Login | Pass | The bound character logged in. |
| Observe | Pass | The response decoded after the compatibility correction below. |
| Render map | Pass | The server returned a map with ten doors. |
| Survey | Pass | The server returned typed counts and six grouped ways out without exposing raw survey text in analytics. |
| Inventory | Pass | The server returned an empty inventory. |
| Say | Pass | The server accepted one in-character arrival line. |
| Play melody | Pass | The server played Cassian's lute and flute phrases, and perception returned each music event. |
| Tool inventory | Pass | All 34 production commands match the typed Rust surface. |
| Move | Pass | The character moved left. |
| Map-guided navigation | Pass | The harness selected server-confirmed reachable tiles and verified the resulting observed positions. |
| Disconnect | Pass | Each smoke command disconnected cleanly. |

The observation reported scene `reldens-town`, six visible objects, and five visible players. The live observation did not include class, health, or maximum health. The harness preserves these fields as unknown. It does not invent values.

The live survey reported zero enemies, zero ground drops, four people, two readable objects, six other players, and six grouped ways out. The map represented those six grouped exits as ten door tiles. All ten map door tiles had known lock state. None was locked.

A later bounded perception run reported six total backend objects and listed all six. The harness therefore recorded `object_list_truncated=false`. The same frame listed four other players separately. The normalized frame had ten visible entities: six non-player objects and four other players. This confirms that the census does not combine the two backend lists.

The latest live run identified one merchant in the six-object list. It calculated five reachable door tiles from tile `(5,8)`. The nearest door had a 12-tile local path. After Cassian moved to `(0,7)`, the local view contained three reachable door tiles and the nearest path had ten tiles. These values changed with the local view, as expected. The trace also counted one filtered non-speech chat line and recorded no dialogue text.

## Chat channel round trip

The Rust gateway sent one scene message, one global message, and one private message from Cassian to Cassian. It kept one MCP session open while it read the resulting observation. The observation contained one scene line, one global line, and two private copies. The private count is two because Reldens sends the private message to both sender and recipient, and Cassian was both.

The channel normalizer used the backend message type. It did not infer a private channel from message text. The live diagnostic recorded only counts. It did not record message text or the recipient name.

Team chat type `8` is supported on the read path and has a local contract test. The current MCP `arena_say` schema does not offer a team-write channel, so the live test did not send team chat.

## Melody and tool-surface checks

Production advertised `arena_play_melody` before the local backend checkout contained it. The Rust gateway now accepts one through 24 notes, one through four repetitions, and the four advertised instruments. Cassian played short lute and flute phrases. Each command returned `played=true`, and the following observation contained one normalized melody event.

The command emits correlated request and completion events. It records the instrument, repetition count, note count, and input character count. It does not record the tune. The read-after-write diagnostic records only the number of melody events heard.

The live `tool-inventory` command follows MCP `tools/list` pagination and compares production with the expected typed surface. Production advertised 34 commands. No command was missing, and no command was unexpected. A future addition or removal makes this diagnostic fail and records the exact changed tool names.

## Latest navigation confirmation

After the chat check, Cassian started a continuous map-guided production walk. The first confirmed move changed his tile from `(2,8)` to `(0,12)`. The next confirmed move changed it from `(0,12)` to `(6,12)`. The server reported arrival and rest for both moves. The process remained connected and continued to alternate safe, server-validated destinations.

After the first Phase 04 executor slice, the rebuilt harness completed another bounded production check. Cassian moved from `(5,11)` to `(0,10)`, then from `(0,10)` to `(6,10)`. Both targets passed the backend path check. The authoritative following observations reported the requested tiles, arrival, and rest. The session then disconnected cleanly at its two-cycle limit.

The BodyActor executor itself now has recording-adapter tests for ordered multi-action execution and replacement-packet preemption. The production `BodyGateway` has a contract test that proves a move calls `arena_check_path` before `arena_move_to` and uses the runtime action ID as the correlation ID for both MCP operations. A dedicated live BodyActor packet command is still required before Phase 04 acceptance; the bounded navigation check continues to use the production diagnostic driver.

## Phase 03 production navigation check

The first direction-step probe changed Cassian's pixel position but kept him in tile `(12,10)`. This was not accepted as a navigation pass.

The updated probe used the normalized production map. It selected a traversable target and called `arena_check_path`. It called `arena_move_to` only after the server returned `reachable: true`. It then observed Cassian again and compared the new position with the old position.

The production trace recorded these movements:

| Start tile | Requested tile | Observed end tile | Result |
| --- | --- | --- | --- |
| `(12,10)` | `(6,10)` | `(9,10)` | Progressed and came to rest. The server reported that it did not arrive. |
| `(9,10)` | `(14,11)` | `(14,11)` | Arrived and came to rest. |
| `(14,11)` | `(8,11)` | `(8,11)` | Arrived and came to rest. |
| `(8,11)` | `(14,11)` | `(14,11)` | Arrived and came to rest. |
| `(3,12)` | `(0,9)` | `(0,9)` | Arrived and came to rest. |
| `(0,9)` | `(5,8)` | `(5,8)` | Arrived and came to rest. |

The trace includes the requested tile, the observed start and end pixels, the observed start and end tiles, and the backend arrival and rest results. Cassian continued to move on the safe route after the check.

The production probe now accepts an optional positive cycle limit. A bounded run emits terminal navigation and session events and then disconnects. This makes the full production trace reviewable without a manual signal.

## Item, enemy, and locked-door contracts

The production map response contains a 33 by 33 local grid for radius 16. The parser removes the presentation header and legend. It keeps 1,089 map tiles.

The backend map has symbols for enemies and doors. It has no symbol for a ground item. The harness reads ground items from `arena_observe.drops`. The backend supplies `dropId`, `itemKey`, `x`, and `y`. The harness converts `x` and `y` to a tile and a relative offset. The live town had no drops, so the position conversion was verified with a captured contract fixture and a normalization test.

The backend survey uses the exact marker `<LOCKED DOOR>`. This marker identifies a way out. It does not expose or rename an item behind the door. The map contract supplies door coordinates and can supply `locked`, `lockKnown`, and `requiredKey`. The harness keeps each field as typed data. It keeps absent fields unknown.

## Contract corrections

### Player identifier

The live server returned `playerId` as a decimal string. The representative fixture used a JSON number.

The observation decoder now accepts a number or a decimal string. It rejects other values.

### Movement direction

The live `arena_move` contract accepts these values:

- `up`
- `down`
- `left`
- `right`

The Rust type now uses these values. The smoke command also accepts north, south, east, and west as command-line aliases. The adapter sends only the backend values.

### Successful text results

Some successful tools return plain text in MCP content instead of JSON text or structured content.

The transport now wraps successful plain text in a typed message object. It still treats malformed JSON-looking text as a protocol error.

### Analytics character identity

The first live run used the backend agent identifier as the analytics character dimension. The session now keeps two identities:

- the backend agent identifier for MCP calls;
- the stable character sheet ID for analytics.

This correction keeps event queries stable across registrations and environments.

## Deferred checks

The live attack check is deferred. The test did not have a verified hostile target that was safe to attack. Do not attack an arbitrary visible object or player to complete a smoke-test checklist.

The tactical model and accounting checks now pass. See [OpenRouter Accounting Smoke Test: 2026-08-11](model-accounting-smoke-2026-08-11.md).

## First tactical shadow run

The connected Rust runtime ran Cassian with the production perception pump and the configured Llama 3.2 3B tactical model. The rollout mode was `shadow`. Model proposals were recorded, but no action packet was released to the BodyActor.

The observability chain caught a contract error before live mutation was enabled. Each quiet-room request used about 22,700 input tokens because the full 33 by 33 structured map tile list entered the prompt. Five responses used 113,717 input tokens, 181 output tokens, and 592 cached input tokens. OpenRouter reported an exact combined charge of `$0.0056881242`. One provider request failed and produced terminal model and tactical failure events. One successful response was discarded after its source frame became stale.

The runtime now derives a separate compact `TacticalInput`. It excludes structured map tiles while preserving map ASCII, exact entities, enemies, drops, doors, exits, inventory, legal skills, combat facts, and bounded recent history. This change uses tactical prompt version `tactician/v2`. `model.call_started` records the serialized input byte count without recording the prompt.

The second production shadow run used a 20-second limit and disabled idle heartbeat inference. It produced one decision from the first material frame. Results:

| Fact | First contract | Compact contract |
| --- | ---: | ---: |
| Serialized input | Not recorded | 5,849 bytes |
| OpenRouter input tokens | About 22,730 per call | 1,737 |
| Output tokens | 33 on the first prior call | 47 |
| Exact charge | `$0.0011359161` on the first prior call | `$0.0001013364` |
| Tactical latency | Not used as the acceptance value | 1,293 ms |
| Packet released | No | No |

The compact input reduced input tokens by about 92%. The response parsed as one action. The shadow gate recorded `release_policy=record_only`, `released=false`, and `rollout_mode=shadow`.

The perception pump completed 27 cycles during the bounded run. Every published frame included its observation-cycle identifier and sequence. Quiet, non-material observations increased the perception revision without increasing the world revision or starting more model calls. The pump stopped, the runtime shut down, and the MCP session disconnected cleanly.

After the shadow run, the bounded production navigation diagnostic moved Cassian through two server-approved targets:

| Start tile | Requested tile | Observed end tile | Result |
| --- | --- | --- | --- |
| `(0,10)` | `(6,10)` | `(6,10)` | Arrived and came to rest. |
| `(6,10)` | `(12,10)` | `(12,10)` | Arrived and came to rest. |

Both cycles had exact perception lineage, changed the authoritative frame revision, and disconnected cleanly after the configured cycle limit.

## Controlled BodyActor production gate

The one-action production gate completed against Cassian in `reldens-town`.
The runtime asserted character id `cassian`, player name `Cassian Vey
Unbound`, and the exact scene before it released one `Stop` action. The action
budget started at one and ended at zero.

The body accepted packet `ba23ffcd-12a3-4e29-91af-6660e9a3916d`, completed
the stop action in 162 ms, and recorded the packet as completed. The diagnostic
waited for that exact terminal packet id and status before shutdown. It did not
treat mailbox submission as execution success.

All events in the run carried process run id
`bad4539e-4429-4c86-adf6-f3d5c96c304f`. The trace included controlled release,
packet acceptance, action success, packet completion, perception-pump stop,
background accounting drain, runtime shutdown, and MCP disconnect. The run had
no failed or aborted accounting tasks.

The production observation omitted the player `alive` field. The validator now
accepts unknown as unknown. It rejects only explicit death or a reported recent
death. This preserves backend authority without inventing life state.

## Bounded dual-brain production shadow

The linked production binary ran Cassian for 35 seconds with Llama 3.2 3B as
the tactician and GPT-OSS 20B as the strategist. The rollout mode was `shadow`.
Neither model could release an action to the body.

The strategist completed one inference in 8,648 ms and published strategic
revision 2. During that inference, the perception pump continued and the
tactician completed decisions. The pump started 71 observation cycles during
the bounded session. A tactical decision after publication used strategic
revision 2, which proves that the fast brain received the new structured
intent.

The strategist response used 1,161 input tokens, 725 reported output tokens,
64 cached input tokens, and an exact `$0.0001277892`. The finalized OpenRouter
record identified CoreWeave and reported 618 native reasoning tokens. Rig's
generic response usage reported zero reasoning tokens. Both source facts remain
in the trace.

The tactician made ten calls. Its per-character ledger ended at 19,847 input
tokens, 562 output tokens, 1,424 cached input tokens, and exact cost
`$0.0011660319`. The exact total for both roles was `$0.0012938211`. No tactical
packet was released.

An earlier launch used a stale linked executable after `cargo check`. Its trace
had no strategic events, so it is excluded from dual-brain evidence. Production
smokes now rebuild the executable before launch.

The valid run exposed a shutdown accounting race. One response arrived after
the original drain had taken its task snapshot. The task registry now counts
active model calls and continues draining tasks registered by those calls. A
12-second production reproduction shut down while the strategist was still in
flight. The strategist response and both final generation audits were recorded
before disconnect. The terminal drain reported four completed tasks, zero
failed tasks, zero aborted tasks, and zero active model calls remaining.

Strategic, tactical, model-call, and body-action start events now use info level.
An in-flight brain decision receives a terminal `runtime_shutdown` supersession
event before its actor stops. Telemetry stops only after the other child actors
finish, so terminal events are not lost during normal shutdown.

## Evidence

The live commands emitted correlated events for session setup, protocol requests, typed tools, failures, completion, and disconnect. The code does not persist raw live payloads. Contract fixtures contain only reviewed representative data.

Local verification after the corrections:

```text
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Both commands pass. The current local suite has 164 unit tests, six MCP
contract tests, and two replay-fixture integration tests.

## Durable history deployment check: 2026-08-12

The production tool inventory increased from 34 to 38 operations. The new
tools are `arena_history`, `arena_party_invite`, `arena_party_respond`, and
`arena_party_leave`. The Rust compatibility layer now covers all 38, and the
live drift diagnostic passes.

An initial `arena_history` read returned 50 events across two scenes. The
backend reducer reported two movement commands, 150 damage dealt, and 24 damage
taken for that page. The harness logged only counts and cursors.

The stronger read-after-write diagnostic selected a server-approved reachable
tile and moved Cassian from `(17,17)` to `(16,11)`. Production reported arrival.
History then returned movement event `95420` with the correct scene and a
non-empty decision ID. The session disconnected cleanly.

The backend ID is generated after the MCP operation and is not the harness's
decision ID. The missing causal-ID round trip is tracked in
[Agent Arena issue #45](https://github.com/Kadajett/agentArena/issues/45).

## Phase 06 safety and movement correction checks: 2026-08-12

Cassian was in `reldens-house-1`. The controlled production gate first set backend tactics to `flee` and `semi_auto`. The BodyActor completed the action in 149 ms.

The next controlled run moved Cassian from tile `(17,19)` to `(17,18)`. A perception frame confirmed arrival after 818 ms. The run observed the character for two more seconds and disconnected cleanly.

Both runs asserted character sheet ID `cassian`, player name `Cassian Vey Unbound`, and the exact scene. Each run had an action budget of one. Each packet had one action.

The OpenRouter key had reached its configured total limit. A concurrent tactical inference failed. The failure telemetry recorded HTTP 403, provider code 403, `quota_exhausted=true`, and `rate_limited=false`. It did not record the provider message. The model failure did not stop the BodyActor operation.

See [Autonomous Combat Test: 2026-08-12](autonomous-combat-2026-08-12.md) for the failed combat run and the safety corrections.

## Bounded run summary and cost gate check: 2026-08-12

A four-second production observation-only run connected Cassian, completed
perception work, emitted a terminal summary, and disconnected cleanly. The
summary reported zero model calls, zero exact cost, zero actions, and a zero
24-hour projection. This proves the aggregate does not invent usage during an
MCP-only run.

Two later login attempts failed. The first returned `Service Unavailable`. The
second reported that the gateway connection to the internal game service was
refused. The incident is recorded as
[agentArena issue 37](https://github.com/Kadajett/agentArena/issues/37). A later
attempt connected successfully.

The first controlled movement attempt exposed a harness diagnostic defect. The
helper created a packet with a 5,000 ms lifetime while the live gate allowed
1,000 ms. The gate rejected the packet with `lifetime_too_long` and did not
consume the action budget. The helper now bounds generated packet lifetime by
the configured gate maximum.

The repeated production run moved Cassian from tile `(17,18)` to `(17,17)` in
`reldens-house-1`. A later perception frame confirmed arrival after 932 ms. The
terminal summary reported:

- one accepted and completed packet;
- one started and successful action;
- action success rate `1.0`;
- packet completion rate `1.0`;
- one movement arrival and no stall;
- zero billed model calls and zero model cost;
- one isolated tactical provider failure caused by the exhausted OpenRouter
  account limit.

The exact character, player name, and scene were asserted before release. The
single-action budget ended at zero. Cassian disconnected cleanly in the inn.
