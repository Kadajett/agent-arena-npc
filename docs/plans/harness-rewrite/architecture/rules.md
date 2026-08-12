# Architecture Rules

This page defines rules that apply to all implementation phases.

## Runtime model

Each player has one Ractor supervision tree:

```text
PlayerSupervisor
├── BodyActor
├── PerceptionActor
├── TacticianActor
├── StrategistActor
├── MemoryActor
└── TelemetryActor
```

The PlayerSupervisor owns identity and lifecycle. It does not make gameplay decisions.

Use `BodyActor` as the canonical name for the executor role. Some source plans use `ExecutorActor` or `MCP Body Actor`. These names refer to the same role.

## Actor ownership

Each actor must own its mutable state.

Actors must communicate with typed messages. Do not put `serde_json::Value` in actor messages. Raw JSON is permitted only at an external protocol seam.

The BodyActor is the only actor that can own a mutable MCP connection. The TacticianActor and the StrategistActor must send typed requests to the BodyActor.

The PerceptionActor must continue to receive state while model inference runs. A model call must not block an actor mailbox.

## Module seams

Use these seams:

| Seam | Interface | Production adapter | Test adapter |
| --- | --- | --- | --- |
| Game transport | `McpTransport` | HTTP and SSE MCP transport | Recorded or in-memory transport |
| Game actions | `ArenaGateway` typed methods | Character-bound MCP gateway | Recording gateway |
| Model access | `Brain<I, O>` | Rig and OpenRouter | Scripted replay brain |
| Conversation memory | Rig `ConversationMemory` | Local SQLite adapter with `rig-memory` policy | In-memory or SQLite test adapter |
| Event input | `EventSource` | Backend ordered stream | Fixture event source |
| Durable memory | `MemoryStore` | SQLite first | In-memory store |
| Semantic retrieval | Rig vector store interface | Local index rebuilt from SQLite | In-memory fixture index |

These are real seams because each one has a production adapter and a test adapter.

Keep the interface small. Hide transport, retry, parsing, and compatibility work inside the module that owns it.

## Truth and decisions

Deterministic code can answer these questions:

- What entities exist?
- Where are the entities?
- What is the player's health?
- What items does the player hold?
- What actions are legal?
- Is a path available?
- Did movement make progress?
- Did a target die?
- Is an action stale?
- What events occurred?

AI code answers these questions:

- Must the player fight or flee?
- Must the player heal now?
- Which target matters most?
- Is loot worth the risk?
- Must the player stop a chase?
- Must the player abandon an objective?
- What long-term goal matters?

Do not add a deterministic rule that chooses an AI answer. For example, do not add `if enemies > 3 { flee }`.

## Typed shared state

The brains share typed state. They do not share chat history.

The blackboard is a module with a typed interface. It does not have to be an actor. The current adapter uses `ArcSwap` for low-cost reads. An actor-owned adapter is permitted if later ownership rules require it. Callers must not depend on the adapter choice.

Hot state stays in memory. Hot state includes:

- the current tactical frame;
- the current strategic intent;
- the active navigation goal;
- the current action packet;
- recent action outcomes;
- the current combat episode facts.

Durable state stays in the memory store. Durable state includes:

- identity and personality;
- goals and plans;
- tasks and promises;
- relationships;
- known people and places;
- firsthand knowledge and hearsay;
- discoveries;
- episode summaries;
- important failures.

Do not use a database for hot tactical reads.

Rig manages strategist conversation history. The durable memory module supplies
the local `ConversationMemory` adapter. The strategist adapter supplies the
conversation identifier. The StrategistActor does not manage Rig messages.

Use a local Rig RAG index for relevant long-term facts. SQLite remains the
durable source for these facts. Do not use the index as a source of truth.

See [Rig Memory and Local RAG Decision](rig-memory-and-local-rag.md).

## Revisions and validity

Use monotonically increasing revisions for meaningful state:

- world revision;
- perception revision;
- strategic revision;
- map revision;
- inventory revision.

Record the relevant revisions when an inference starts.

When the inference completes, compare its revisions with current state. Discard the result if a material change made the result unsafe.

Do not invalidate a combat decision because an unrelated chat line arrived.

The following changes normally invalidate a tactical result:

- the player died;
- the scene changed;
- the target died or disappeared;
- health crossed a critical threshold;
- the required item disappeared;
- the path became invalid;
- a hard strategic constraint changed;
- the MCP session reconnected.

## Model output and runtime metadata

The tactical model returns a `TacticalProposal`.

The model can provide:

- tactical intent;
- requested actions;
- abort conditions;
- validity duration;
- an optional short rationale for development traces.

The model must not provide:

- packet IDs;
- decision IDs;
- correlation IDs;
- timestamps;
- world revisions;
- strategic revisions;
- agent IDs;
- validation results.

The runtime must add this metadata after it parses the proposal. The enriched internal type is an `ActionPacket`.

The current Phase 1 code uses one type for both forms. Phase 04 must split this type before live model execution.

## Character identity and capabilities

The runtime binds `agent_id` in the MCP adapter. A model schema must not contain `agent_id`.

Capability checks must occur in the BodyActor or the typed gateway. Prompt visibility is not a security control.

Use these capabilities:

- `Speak`;
- `TalkToFolk`;
- `Walk`;
- `Doors`;
- `Fight`;
- `Duel`;
- `Money`;
- `Trade`;
- `Purpose`.

## Backend authority and compatibility

The backend must remain authoritative for:

- health and maximum health;
- level and experience;
- class path;
- combat actions;
- cooldowns;
- equipment;
- inventory;
- movement state;
- combat outcomes;
- event sequence numbers.

Use `Option<T>` when the backend does not supply a field. Do not invent a value.

Keep temporary compatibility logic in one named adapter. Put it behind a feature flag when practical. Do not copy hard-coded skill tables into tactical code.

## Failure rules

A model failure must not destroy the game session.

A strategist timeout must not stop combat.

A memory write failure must not block tactical inference.

An MCP session failure must cause reconnect and full state reload. Reconnect must invalidate active packets and in-flight inference results.

Use a child restart only when dependency restoration has a test. The tactician restart path already has this test. Use process-level restart for an unproven dependency-heavy recovery path.

## Scaling rules

The first production release targets one or a small number of players.

Do not add distributed actors, NATS, model batching, or multi-region execution in the first release.

Keep player state inside one supervision subtree. This structure permits a later `WorldSupervisor` without a new player programming model.
