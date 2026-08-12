# Local Test Strategy

This page defines how to test the Rust harness during development.

## Test levels

Use four test levels.

### Level 1: Pure tests

Pure tests must not use a network or a model provider.

Use them for:

- validation;
- reducers;
- map conversion;
- revision rules;
- event derivation;
- memory rules;
- prompt input construction.

Run:

```bash
cd rust-harness
cargo test --all-features
```

### Level 2: Recorded adapter tests

Recorded adapter tests use captured MCP requests and responses.

Use them for:

- JSON-RPC parsing;
- SSE completion detection;
- tool argument names;
- optional backend fields;
- TypeScript-to-Rust normalization parity;
- reconnect and session invalidation.

Recorded tests must not need an Agent Arena key.

Remove or replace secrets before a fixture enters the repository.

### Level 3: Local harness tests

Local harness tests run the actor tree with test adapters.

Use them for:

- concurrent strategist and tactician work;
- body preemption;
- bounded tactical wake behavior;
- actor failure isolation;
- graceful shutdown;
- replay execution.

Use a scripted brain for deterministic tests. Use Rig only in explicit model integration tests.

### Level 4: Live Agent Arena tests

Live tests use a dedicated test character and an explicit environment switch.

Live tests must meet these controls:

- Use a non-production character.
- Use a separate Arena key.
- Bind the expected agent ID.
- Start in a known scene.
- Set a time limit.
- Set a model cost limit.
- Record all MCP calls.
- Stop on identity mismatch.
- Stop on reconnect loops.
- Run the tool-inventory diagnostic before mutation tests.

Use an environment variable such as `NPC_ALLOW_LIVE_MUTATION=1` to permit mutations. The default value must disable live mutations.

Do not run live mutation tests in the normal `cargo test` command.

The tool-inventory diagnostic must compare MCP `tools/list` with the typed Rust surface. It must fail on missing and unexpected names. This protects the harness from silently ignoring new production commands.

## Required commands

Run these commands before a phase can complete:

```bash
cd rust-harness
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Run the TypeScript regression suite until Phase 12 removes it:

```bash
npm test
```

## Fixture rules

Store fixtures by behavior:

```text
rust-harness/fixtures/
├── mcp/
├── combat/
├── movement/
├── exploration/
├── social/
└── migration/
```

Each fixture must state:

- its source;
- its schema version;
- its expected result;
- whether values were redacted;
- the bug or behavior that it covers.

Every important production failure must become a fixture when the failure can be reproduced safely.

## Test safety

Tests must not write to the user's production memory database.

Tests must use a temporary directory or an in-memory store.

Tests must not print API keys, authorization headers, or raw private conversation memory.

Tests that start actors must stop all actors before they finish.
