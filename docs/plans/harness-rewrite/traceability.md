# Source Traceability

This page maps the source plans and current TypeScript behavior to the rewrite phases.

## Architecture coverage

| Requirement | Primary phase | Supporting phase |
| --- | --- | --- |
| Ractor supervision tree | 01 | 11 |
| Rig and OpenRouter model access | 01 | 05, 07, 10 |
| Typed blackboard and revisions | 01 | 03, 05 |
| Typed MCP adapter and SSE support | 02 | 11 |
| Registration, login, and reconnect | 02 | 11 |
| Structured observation and map parity | 03 | 09 |
| Event accumulator | 03 | 09 |
| Body-only mutation | 04 | 11 |
| Packet validation and mid-packet invalidation | 04 | 05 |
| Concurrent movement monitoring | 04 | 06 |
| Event-driven tactician | 05 | 06 |
| Tactical model benchmark candidates | 05 | 10 |
| Live concurrent combat | 06 | 11 |
| Goals, plans, conversation, and social behavior | 07 | 08 |
| Durable multi-step plans and destination-level navigation | 07 | 04, 08 |
| SQLite memory migration | 08 | 12 |
| Ordered backend events and reducers | 09 | 10 |
| Replay corpus and model selection | 10 | 11 |
| Failure isolation and operational controls | 11 | 12 |
| Full behavior parity and TypeScript removal | 12 | All phases |

## Existing behavior parity

The Rust harness must account for each behavior before cutover.

| Existing behavior | Planned phase |
| --- | --- |
| Character identity and persona | 01, 07 |
| Character sheet and capabilities | 01, 02, 04 |
| Player registration and login | 02 |
| Reconnect | 02, 11 |
| Goals and plan persistence | 07, 08 |
| Task list and short-lived notes | 07, 08 |
| Relationships and remembered people | 07, 08 |
| Known places | 03, 07, 08 |
| Hearsay and firsthand knowledge | 07, 08 |
| Room conversation memory | 07, 08 |
| Anti-repetition behavior | 07 |
| Chat filtering | 03, 07 |
| Map exploration | 03, 07 |
| Door handling | 02, 04 |
| Fighting and skills | 02, 04, 05 |
| Duels | 02, 07 |
| Money, inventory, item use, pickup, and trade | 02, 04, 07 |
| Feeling or status indicator | 02, 07 |
| Memory persistence | 08 |

## Items that must not be literal ports

| Old implementation | Replacement |
| --- | --- |
| `goingInCircles()` for physical motion | Position progress and movement stall detection in Phase 04 |
| Blocking movement loops | Concurrent movement monitoring in Phase 04 |
| Hard-coded class skill maps | Backend-reported legal actions in Phases 02 and 03 |
| Shared combat chat history | Stateless tactical frames in Phase 05 |
| Model-direct combat MCP calls | Tactical proposal, action packet, and BodyActor in Phases 04 and 05 |
| Mastra memory repair workarounds | Purpose-built memory store and migration in Phase 08 |

## Tactical invariant coverage

| Invariant | Test phase |
| --- | --- |
| Only the BodyActor mutates the world. | 04 |
| Tactical and strategic inference run concurrently. | 06 |
| A slow strategist cannot stall combat. | 06 |
| A stale tactical result cannot execute. | 05 |
| A packet can stop midway after invalidation. | 04 |
| A model cannot select `agent_id`. | 02, 04 |
| An invalid target fails before mutation. | 04 |
| Polling cannot create unlimited inference calls. | 05 |
| Actor mailboxes continue during inference. | 01, 05 |
| Reconnect invalidates old decisions. | 11 |

## Strategic invariant coverage

| Invariant | Test phase |
| --- | --- |
| Personality survives restarts. | 07, 08 |
| Long-term goals survive restarts. | 07, 08 |
| Strategy revisions reach the tactician immediately. | 01, 07 |
| Tactical failure cannot rewrite personality. | 07, 08 |
| Combat noise cannot flood semantic memory. | 08, 09 |
| Relationships are durable and evidence-based. | 07, 08 |
| Hearsay and firsthand knowledge stay separate. | 07, 08 |

## Source plan reconciliation

The first source plan defined ten migration phases. The later dual-brain plan defined seven broader phases. This plan uses twelve phases.

The twelve-phase sequence keeps all earlier migration work. It adds separate phases for replay evaluation and operational hardening. It moves final TypeScript removal into its own cutover phase.

No source requirement is intentionally removed. If a new source requirement appears, add it to this page and to the applicable phase document.
