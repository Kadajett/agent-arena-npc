# Production Cutover: 2026-08-12

## Status

Guy, Barnaby, and Cassian run on the Rust harness in production.

The three services use one generic image. Each service loads an external
character sheet and a character-specific secret file. The shared Rust runtime
does not contain a branch for a character name.

The final verified image digest is
`sha256:92c7cd90117a714717f64eb434612f1c4b27fd08ff5dae8c1a60bb08cded3fad`.
All three containers run that digest with zero restarts.

## Production services

| Service | Character | Rollout | Perception interval | Result |
| --- | --- | --- | ---: | --- |
| `rust-npc-guy` | Guy | Full | 750 ms | Healthy persistent service |
| `rust-npc-barnaby` | Barnaby | Observe-only tactical mode | 2,000 ms | Healthy persistent service in the inn |
| `rust-npc-cassian` | Cassian Vey Unbound | Full | 750 ms | Healthy persistent service |

The production Compose project is in `/opt/agent-arena-npc-rust`. All three
services use `restart: unless-stopped`, a read-only root file system, dropped
Linux capabilities, `no-new-privileges`, a bounded temporary file system, a
health check, and rotated JSON logs.

The legacy `deploy-guy-1` and `deploy-barnaby-1` containers are stopped. Their
restart policy is `no`. The other repository-owned NPC containers are also
stopped with restart disabled. The Agent Arena world, gateway, and database
services remain running.

## Live proof

The proof below comes from production analytics. It does not come from a mock
gateway.

### Guy

Guy connected with session generation 1. Six supervised actors started. No
actor failed. The tactical model moved Guy through town and selected the exact
south exit. The BodyActor translated that exit into `arena_enter_door`. Live
perception then reported these positions:

```text
reldens-town (23,25)
reldens-town (23,26)
reldens-bots (20,24)
reldens-bots (20,26)
reldens-town (23,26)
```

This proves local movement and scene traversal. It also proves that the
perception actor continued to publish frames during movement.

The final image then recorded two complete typed transitions:
`reldens-bots -> reldens-town` and `reldens-town -> reldens-bots`. Each causal
chain contains `body.movement_scene_transition`, `body.action_succeeded`, and
`body.packet_completed` for the same packet.

### Barnaby

Barnaby connected with session generation 1. Six supervised actors started.
No actor failed. Every observed position remained `reldens-house-1 (22,14)`.
The body accepted no tactical packet and made no movement request.

This restriction is generic policy. Barnaby's sheet has `speak` and
`talk_to_folk`. It does not have `walk`, `doors`, or `fight`. The deployment
also asserts `NPC_LIVE_ALLOWED_SCENE=reldens-house-1`.

### Cassian

Cassian connected with session generation 1. Six supervised actors started.
No actor failed. The tactical model selected the inn exit. The BodyActor used
the typed door operation. Live perception reported this route:

```text
reldens-house-1 (24,11)
reldens-house-1 (21,12)
reldens-house-1 (19,14)
reldens-house-1 (18,17)
reldens-house-1 (17,19)
reldens-town (12,10)
```

This proves that Cassian can leave the inn and move in the production world
without a manual MCP command.

The final image recorded three complete typed transitions for Cassian:
town to inn, inn to town, and town to inn. Each one ended with an authoritative
scene-transition fact, successful body action, and completed packet.

The final verification window contained no actor failure, perception-cycle
failure, body-action failure, or rejected body packet. One free-model request
received an OpenRouter 429. The failure was isolated to that inference; world
perception continued and later calls remained eligible.

## Memory migration

The cutover kept the original databases and made a root-only backup at:

```text
/var/lib/docker/volumes/deploy_npc_var/_data/rust/legacy-backups/20260812T100009Z
```

The backup contains both legacy SQLite databases, Barnaby's conversation JSON,
visited-place files, and a manifest. The source databases remain unchanged.

The current Rust databases pass `PRAGMA quick_check`.

| Character | Rig conversation messages | Semantic memories | Archived legacy rows | Working state |
| --- | ---: | ---: | ---: | ---: |
| Guy | 2,824 | 41 | 5,035 | 1 |
| Barnaby | 8,786 | 24 | 9,649 | 1 |
| Cassian | 278 | 0 | Not applicable | 1 |

Rig memory loads the durable conversation before a strategic call. It appends
the new user and assistant messages after a successful call. Production logs
show each count increasing by two and surviving service recreation.

Guy's migrated goal and unfinished plan seed his first strategic intent.
Barnaby's blank legacy goal uses the generic neutral fallback. No runtime code
checks for Barnaby's name.

## Models and accounting

The deployed models are:

| Character | Tactician | Strategist |
| --- | --- | --- |
| Guy | `google/gemma-4-26b-a4b-it:free` | `nvidia/nemotron-3-super-120b-a12b:free` |
| Barnaby | Not scheduled while idle | `nvidia/nemotron-3-nano-30b-a3b:free` |
| Cassian | `google/gemma-4-26b-a4b-it:free` | `nvidia/nemotron-3-super-120b-a12b:free` |

Every successful response records the requested model, actual model, provider,
prompt version, generation ID, input tokens, output tokens, reasoning tokens,
cached input tokens, cache-creation tokens, exact OpenRouter charge, latency,
and the cumulative character ledger. A background accounting task reconciles
the OpenRouter generation record.

The cutover window used free routes. OpenRouter reported exact charge `0` for
the recorded generations. One Barnaby call reported 8,448 cached input tokens.
The harness recorded that value instead of estimating it.

## Operational corrections found during cutover

The cutover found and corrected these generic defects:

1. Three 500 ms perception pumps exceeded the gateway limit. Guy and Cassian
   now use 750 ms. Stationary Barnaby uses 2,000 ms. The clean verification
   window had no failed perception cycle.
2. A compact movement outcome did not identify its failed destination. The
   outcome now includes `destination_tile` so the tactician can avoid it.
3. The model could select its current tile as progress. Tactical prompt v9
   prohibits that no-op.
4. A listed exit used ordinary `move_to`, which cannot cross a scene boundary.
   The BodyActor now converts an exact reachable exit into the typed door
   operation. The `doors` capability is enforced before the call.
5. A scene transition could occur before the door MCP future returned. The
   movement reducer now records that authoritative transition as success and
   ignores the late completion.
6. That shortcut initially applied to every move. It now recognizes success
   only for a typed `enter_door` action. An unexpected scene change during an
   ordinary move remains material invalidation and cancels the packet.

## Local verification

The final local gate passed:

- Rust formatting check;
- strict Clippy for all targets and features with warnings denied;
- 187 library tests;
- binary tests;
- seven captured MCP contract tests;
- two tactical replay integration tests.

The suite covers actor isolation, stale packets, preemption, movement stalls,
door translation, authoritative scene-transition completion, Rig memory,
provider accounting, reconnect, capability validation, combat safety, and
structured MCP decoding.

## Remaining evaluation work

The services are persistent, but this cutover is not a long-duration combat
benchmark. A later run must measure enough active combat, travel, social, and
idle time to estimate success rates and daily cost. Do not project a daily paid
cost from the current free-route startup sample.

## Paid-route update

After the cutover, the production key limit was increased and the services
moved to paid routes. Guy and Cassian now use
`google/gemini-3.1-flash-lite` for tactics and
`nvidia/nemotron-3-super-120b-a12b` for strategy. Barnaby uses
`nvidia/nemotron-3-nano-30b-a3b` for strategy. His idle tactical inference
remains disabled.

The first paid tactical calls completed in about one to two seconds and
recorded exact non-zero charges. See
`production-usage-and-chattiness-2026-08-12.md` for the first-hour baseline,
paid probe results, and daily price estimate.
