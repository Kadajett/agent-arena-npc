# Phase 08: Memory and Migration

State: In progress

## Current implementation audit: 2026-08-12

The memory contracts, durable Rig conversation adapter, strategist recall path,
and local Rig semantic index exist.

- `MemoryStore` covers working state, episodes, relationships, and semantic
  records. `SqliteMemoryStore` implements these operations.
- `WorkingMemory`, goals, plan steps, tasks, notes, and relationship types exist.
- `MemoryActor` loads typed working state at startup and persists replacement
  working state, relationship evidence, and episode summaries.
- Memory write failures leave the actor alive and increment its visible failure
  counter.
- `NPC_MEMORY_PATH` is wired to the typed store at player runtime startup.
- `SqliteConversationMemory` stores Rig `Message` values in SQLite.
- `TokenWindowMemory` bounds loaded strategist history without deleting the
  durable message record.
- Conversation-memory tests prove restart persistence, bounded loads, isolated
  clear operations, tool-result pairing, and content-safe telemetry.
- `RigAgentBrain` proves automatic Rig memory load and append through the
  existing typed `Brain` interface.
- Production strategist wiring uses Rig conversation memory and preserves exact
  OpenRouter response charges and generation audits.
- The typed SQLite schema stores working state, idempotent episodes,
  relationships with evidence records, and provenance-bearing semantic memory.
- The typed store is wired to `MemoryActor` and runtime startup.
- The strategist requests typed recall before inference without blocking the
  tactical actor.
- `RigSemanticMemoryStore` builds a character-scoped Rig in-memory vector index
  over semantic records, relationships, and episodes with local FastEmbed
  embeddings. SQLite remains authoritative.
- Index construction and retrieval emit content-free latency, count, score,
  version, model, and fallback telemetry.
- Store and actor restart tests exist. Migration, backup, restore, and injected
  write-failure tests are still incomplete.

The newly deployed `arena_history` supplies durable exact world events. It does
not replace semantic character memory. Phase 08 should consume selected episode
summaries and relationship evidence from that history while leaving raw events
in the backend ledger.

Do not describe the current actor skeleton as persistent memory.

### Production source inventory: 2026-08-12

The owned production Compose project is `deploy` on the VPS at
`2.25.100.234`. The TypeScript NPC containers mount the named volume
`deploy_npc_var` at `/npc/var`. The source databases are live SQLite databases
with write-ahead logs. Do not copy only the `.db` files while a container is
running.

The read-only inventory found these migration sources:

| Character | Messages | Observation snapshots | Typed working-memory records | Source notes |
| --- | ---: | ---: | ---: | --- |
| Guy | 4,563 | 311 | 1 | Five people, ten current events, one place, one goal, one note, six plan steps, and twelve recent entries were present. |
| Barnaby | 9,170 | 168 | 1 | Twelve people, three current events, one place, one obligation, one goal, and two recent entries were present. A separate `barnaby.conversations.json` file was also present. |

The counts are a point-in-time inventory. The containers continue to write to
the databases, so the final migration must take a fresh backup and report fresh
counts.

Use SQLite's online backup operation or stop the source container cleanly before
copying the database, WAL, and shared-memory state. Verify the resulting backup
with an integrity check and table counts before migration. Keep the original
backup unchanged.

See [Rig Memory and Local RAG Decision](../architecture/rig-memory-and-local-rag.md)
for the accepted module design.

## Goal

Persist intentional character memory and migrate useful TypeScript memory without silent data loss.

## Required result

Guy keeps identity-related state, goals, plans, relationships, knowledge, and episode summaries across Rust harness restarts.

## Dependencies

Phase 07 must define the strategic memory domains and their invariants.

Captured or copied TypeScript SQLite data must be available for migration tests.

## Memory domains

Use separate types for these domains.

### Working state

- current goal;
- current plan;
- task list;
- short-lived notes;
- recent promises;
- current relationship state.

### Durable semantic memory

- people;
- places;
- discoveries;
- opinions;
- promises;
- important outcomes;
- economic knowledge;
- firsthand facts;
- rumors and provenance.

### Episode summaries

- meaningful combat episodes;
- exploration episodes;
- important social episodes;
- important failures.

### Exact event history

Do not store exact global event history as semantic memory. The backend event store owns exact events.

### Strategist conversation history

Rig owns the conversation-history workflow.

- Use Rig Agent for strategist prompts.
- Use Rig `ConversationMemory` for automatic load and append.
- Use the character-specific strategist conversation identifier.
- Use `rig-memory` policies to bound loaded history.
- Keep complete messages in SQLite.
- Do not manage `Vec<Message>` in the StrategistActor.
- Do not attach conversation memory to the tactician.

### Local semantic retrieval

Use a small local Rig RAG index for concrete durable memory.

- Keep typed facts and provenance in SQLite.
- Create embedding documents from the typed facts.
- Rebuild the local index from SQLite.
- Retrieve only for the strategist.
- Filter retrieval by character identifier.
- Keep exact backend history outside the index.
- Keep current goals and plans in typed working state.

## Store tasks

- [x] Implement the `MemoryStore` interface.
- [x] Implement a SQLite adapter.
- [ ] Implement an in-memory test adapter.
- [x] Implement a local SQLite adapter for Rig `ConversationMemory`.
- [x] Apply Rig's token-window policy without deleting the durable message record.
- [x] Implement a typed Rig Agent brain adapter with automatic memory load and append.
- [x] Wire the Rig Agent brain adapter into the production strategist.
- [x] Add a stable strategist conversation identifier per character.
- [ ] Add a deterministic compaction policy after restart tests pass.
- [x] Implement the typed semantic-memory schema.
- [x] Build a local Rig vector index from typed SQLite memory.
- [ ] Add character and provenance filters to semantic retrieval.
- [x] Inject retrieved context through the strategist adapter.
- [x] Create explicit schema versions.
- [x] Add transactional writes for relationship state and evidence.
- [x] Keep memory writes outside tactical work.
- [ ] Apply note expiry when notes are read.
- [ ] Bound lists that can grow.
- [ ] Preserve provenance for rumors and relationships.
- [ ] Add backup and restore instructions.

Postgres can replace SQLite later. Do not require Postgres for the first production release.

## MemoryActor tasks

- [x] Load working state at startup.
- [x] Persist replacement working state, including goal, plan, tasks, and notes.
- [x] Persist relationship changes and their evidence.
- [ ] Persist discoveries and place knowledge.
- [x] Record episode summaries.
- [x] Retrieve relevant memory for the strategist.
- [x] Return typed recall results.
- [x] Keep memory write failure isolated from tactical work.

## Migration tasks

- [x] Inspect the current production SQLite schema and record counts.
- [ ] Capture fixture-safe copies of the supported source record shapes.
- [ ] Map each old field to a new domain type.
- [ ] Mark fields that cannot be migrated safely.
- [ ] Produce a dry-run migration report.
- [ ] Count read, converted, skipped, and failed records.
- [ ] Keep the source database unchanged.
- [ ] Write to a new target database.
- [ ] Verify records after the write.
- [ ] Support a repeatable idempotent migration.
- [ ] Save warnings for manual review.

Do not silently discard memory.

Do not port Mastra-specific repair workarounds unless the new store has the same proven failure.

### TypeScript source mapping

The old Mastra database can contain both structured working memory and raw
conversation messages. Migrate these sources separately.

| Old field | New destination | Rule |
| --- | --- | --- |
| `people` | typed person and relationship records | Preserve name, description, stance, reason, and last-seen text. Do not invent a game entity identifier. |
| `goingsOn` | semantic event or discovery records | Preserve the source record. Mark provenance unknown when the old record does not prove it. |
| `places` | typed place records | Preserve `been` versus `heard`, speaker, settled state, vouchers, and doubts. |
| `ownBusiness` | typed promise, obligation, or working note | Classify conservatively. Keep the original text and migration warning when the type is ambiguous. |
| `opinions` | typed opinions | Preserve subject kind, stance, reason, evidence count, and start time. |
| `goal` | working goal | Preserve aim, done condition, reason, and set time. |
| `todo` | working tasks and promises | Preserve status, time, note, and requester. |
| `notes` | short-lived working notes | Preserve the original time. Apply expiry after migration. |
| `plan` and `planFor` | working plan | Preserve step status, note, tries, and goal association. |
| `lately` | recent working history | Preserve only the bounded newest entries. Do not promote all entries to semantic memory. |
| Mastra messages | Rig conversation messages when lossless conversion is safe | Convert supported text and paired tool records. Archive unsupported records and emit a warning. |
| Mastra observations or reflections | semantic episode candidates | Keep the original artifact and provenance. Do not treat model summaries as exact backend events. |

Run the migration for Guy and Barnaby before their cutover. Cassian has no
TypeScript Mastra database unless a live deployment created one. Use the new
SQLite memory directly when no source exists.

## Recall rules

The strategist can request relevant durable memory.

The tactician must not perform general memory retrieval in the hot path.

The memory module must not rewrite the persona.

Rig conversation history is not a replacement for typed working state. The
strategist receives current goals and plans as structured input.

The local RAG index is derived data. SQLite remains the source for semantic
memory.

Short-term combat noise must not enter durable semantic memory unless an episode reducer marks the episode as meaningful.

## Tests

- [ ] Empty memory starts safely.
- [x] Rig messages survive a SQLite close and reopen.
- [x] Rig token policy bounds loaded history without deleting durable messages.
- [x] Clearing one Rig conversation does not clear another conversation.
- [x] Conversation telemetry excludes message content and conversation identifiers.
- [x] Tool-call and tool-result pairs survive the configured history policy.
- [ ] The local RAG index rebuilds from typed SQLite records.
- [x] Semantic retrieval cannot cross character identifiers.
- [x] Relevant semantic memory reaches the strategist adapter.
- [ ] The tactician has no memory or RAG dependency.
- [x] Goal and plan survive a store and actor restart.
- [ ] Tasks and requester names survive restart.
- [ ] Notes expire at the correct time.
- [ ] Relationship evidence survives restart.
- [ ] Hearsay and firsthand facts stay separate.
- [x] Episode summaries persist idempotently.
- [x] The typed SQLite adapter isolates characters.
- [x] Semantic record identifiers make repeated writes idempotent.
- [x] Typed memory telemetry excludes memory content and source identifiers.
- [ ] Tactical processing continues during a slow write.
- [ ] A failed write returns a visible failure.
- [ ] Migration dry-run does not write.
- [ ] Migration is idempotent.
- [ ] Migration reports every skipped record.
- [ ] The source database remains unchanged.

## Acceptance criteria

Phase 08 is complete when:

1. Rust memory survives restart.
2. Strategic invariants pass against SQLite and the in-memory adapter.
3. Memory writes cannot block tactical inference.
4. Migration produces a complete report.
5. Useful old memories appear in the new typed domains.
6. No record disappears without a warning or a mapped result.
7. The migration has a rollback procedure.
8. Rig Agent loads and appends strategist conversation history automatically.
9. The local RAG index can be rebuilt from typed SQLite memory.
10. Retrieval preserves character identity and provenance.

## Out of scope

Do not add a remote or large vector store in this phase.

Do not add global event search. Use the backend history interface for exact
events.
