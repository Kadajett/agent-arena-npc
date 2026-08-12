# Rig Memory and Local RAG Decision

State: Accepted on 2026-08-12

## Decision

Use Rig 0.41 for strategist conversation memory and retrieval.

Use these Rig modules:

- `ConversationMemory` for strategist conversation history;
- `rig-memory` for the token window and later compaction;
- Rig Agent for the strategist prompt loop;
- Rig vector store interfaces for semantic retrieval;
- Rig extended response details and hooks for model telemetry.

Do not add conversation memory to the tactician.

## Module seams

The strategist actor continues to use one small model interface:

```rust
Brain<StrategicInput, StrategicProposal>
```

A strategist adapter hides these implementation details:

- the Rig Agent;
- the conversation identifier;
- the `ConversationMemory` adapter;
- the token-window policy;
- the local RAG index;
- retrieved context injection;
- structured output;
- Rig hooks and usage details.

The actor must not manage a `Vec<Message>`. The actor must not query a vector
store directly.

The tactical adapter remains stateless. This preserves the small tactical
context and predictable tactical cost.

## Durable sources

SQLite is the durable local source for character memory.

Store these records as typed data:

- working goal, plan, tasks, and notes;
- people and relationships;
- places;
- promises;
- discoveries;
- firsthand facts;
- hearsay and its source;
- episode summaries;
- important failures.

Store Rig conversation messages in a separate SQLite table. Rig loads and
appends these messages through `ConversationMemory`.

The Agent Arena backend remains the source for exact game history. Do not copy
all `arena_history` rows into conversation memory or semantic memory.

The persona file remains the source for personality. Memory cannot replace or
edit the persona.

## Bounded conversation history

Use `TokenWindowMemory` with `HeuristicTokenCounter::openai()` for the first
adapter. The configured token count is a prompt budget. It is not an exact
provider charge.

Keep the complete conversation in SQLite. Apply the token window when Rig
loads history. The policy must preserve valid tool-call and tool-result pairs.

Add deterministic `TemplateCompactor` only after the basic adapter is wired to
the strategist and restart tests pass. Do not use an LLM compactor at first. An
LLM compactor runs in the load path and adds latency and cost to strategic
inference.

If the system later demotes old messages to semantic memory, use Rig's
`DemotingPolicyMemory`. The demotion hook must be idempotent across process
restarts.

## Local RAG

Use a local Rig vector index for semantic recall. SQLite remains the durable
source. Rebuild the index from SQLite at startup or after an index-version
change.

Each indexed document must have:

- a stable memory identifier;
- a character identifier;
- a memory kind;
- factual text for embedding;
- provenance;
- firsthand or hearsay status where applicable;
- creation and update times;
- an embedding model identifier;
- an index schema version.

The first implementation can use Rig's in-memory vector store. This store is a
local derived index. It is acceptable for one player because SQLite can rebuild
it. A local durable vector adapter can replace it later without changing the
strategist interface.

Use semantic retrieval for concrete long-term records. Do not use it for hot
tactical state, exact event replay, the current goal, or the current plan.

### Implemented retrieval adapter

`RigSemanticMemoryStore` decorates the durable `MemoryStore` seam. It uses
Rig's `EmbeddingsBuilder`, `InMemoryVectorStore`, and `VectorStoreIndex` with
Rig FastEmbed's quantized All-MiniLM-L6-v2 local model. The index contains typed
semantic facts, relationships, and episode summaries. SQLite remains the source
of truth.

The index builds lazily on the first non-empty recall and is invalidated after a
fact, relationship, or episode write. Retrieval is filtered by character and
bounded independently by memory kind. If model initialization, embedding, or
search fails, the store returns the existing bounded deterministic lexical
result and emits a degraded-mode event.

The default Cargo feature set includes `local-rag`. A lean
`--no-default-features` build omits the native ONNX dependency and emits a
visible feature-unavailable event when RAG is configured. The quantized model
asset is about 24 MB and is downloaded only when a non-empty index first needs
to build, never merely by running ordinary tests.

## Retrieval rules

Retrieve memory only for the strategist.

Filter all retrieval by character identifier. Use metadata filters for memory
kind and provenance when the query requires them.

Return a small result set. Include the stable memory identifier and provenance
with each result. Do not pass an embedding or a similarity implementation
detail to the model.

The strategist can use retrieved facts as evidence. It must not treat hearsay
as firsthand knowledge.

## Failure rules

A conversation-memory failure fails the affected strategic call visibly. It
must not stop the body, perception, or tactician.

A RAG failure must produce a visible strategic failure or a configured
degraded strategic call. The selected behavior must be explicit in telemetry.

A memory write runs outside the tactical path. A slow write cannot block a
tactical decision.

## Observability

Emit terminal events for every memory and retrieval operation.

Record these conversation facts:

- operation;
- duration;
- message count;
- serialized byte count;
- success or stable error class.

Record these RAG facts:

- index version;
- embedding model;
- operation;
- duration;
- requested result count;
- returned result count;
- memory-kind counts;
- minimum and maximum returned score when available;
- provenance coverage;
- success or stable error class.

Do not record conversation identifiers, message text, memory text, query text,
embeddings, private speech, prompts, or complete model output.

Use Rig extended response details and hooks for model-call and tool-call
telemetry. Reconcile their usage with the existing OpenRouter generation
accounting. Do not count the same provider charge twice.

## Tests

The implementation must prove these results:

1. Rig messages survive a SQLite close and reopen.
2. The token policy bounds loaded history without deleting durable messages.
3. A clear operation removes only the selected conversation.
4. Tool calls and tool results remain paired after the policy runs.
5. Analytics contains counts and latency but no memory content.
6. The local index rebuilds from SQLite.
7. Retrieval cannot cross character identifiers.
8. Firsthand facts and hearsay retain different provenance.
9. The strategist receives relevant facts and no unrelated raw history.
10. The tactician has no conversation-memory or RAG dependency.
11. A slow or failed memory operation does not stop tactical processing.
12. Model token and cost accounting remains exact after Rig Agent integration.

## Rejected designs

Do not manage strategist history by hand. Rig already supplies this behavior.

Do not use only `InMemoryConversationMemory` in production. It loses history on
restart.

Do not make the vector index the durable memory source. Re-embedding, model
changes, or index loss must not delete character memory.

Do not put all exact backend events into RAG. The backend history interface is
the exact event source.

Do not give the tactician dynamic context. This adds latency, cost, and stale
long-term details to the tactical path.
