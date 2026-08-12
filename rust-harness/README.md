# Agent Arena Rust harness

This crate is the side-by-side Rig + Ractor replacement for the TypeScript/Mastra NPC harness. The TypeScript runtime remains the production entrypoint until contract, replay, and live parity are proven.

The authoritative implementation plan is in [the harness rewrite plan](../docs/plans/harness-rewrite/README.md). Update the applicable phase page when implementation changes a planned interface, test, or exit criterion.

## Current milestone: Phases 4 and 5

Implemented:

- one supervised Ractor subtree for a player;
- body, perception, tactician, strategist, memory, and telemetry actors;
- typed actor messages and clean shutdown;
- a versioned, latest-value in-process blackboard;
- asynchronous tactical inference that does not block its actor mailbox;
- stale tactical response discard and immediate newest-frame evaluation;
- tactician restart without replacing the blackboard or strategist;
- typed tactical frames, strategic intents, packets, outcomes, events, and maps;
- body-side packet, capability, target, item, and skill validation;
- character-bound typed MCP adapter interface;
- a Rig/OpenRouter structured-output brain adapter;
- versioned tactician and strategist prompts;
- a Streamable HTTP MCP transport with JSON and persistent SSE support;
- MCP initialization, registration, login, disconnect, and bounded reconnect;
- MCP session ID capture and reuse;
- typed methods for the complete current gameplay tool surface;
- character identity injection and capability enforcement at the gateway;
- classified transport, protocol, JSON-RPC, tool, and decode errors;
- structured, correlated analytics for the runtime, actors, session, protocol, and typed tools;
- secret redaction and payload-safe telemetry;
- local HTTP, session, streaming, contract, identity, and analytics tests.
- normalized observations, maps, entities, enemies, drops, doors, inventory, chat channels, and melodies;
- current production class, legal-skill, equipment, combat-result, damage, kill, and respawn contracts;
- deterministic combat-episode and movement fact reducers;
- asynchronous ordered BodyActor execution with per-action revalidation, outcomes, and packet preemption;
- packet-to-action-to-MCP causal correlation and terminal telemetry;
- production tool-surface drift detection;
- exact OpenRouter price, token, cache, reasoning, provider, and per-character cost accounting.
- durable Rig conversation memory plus a character-scoped local Rig/FastEmbed
  index over typed facts, relationships, and episode summaries;

Not complete yet:

- the controlled live attack smoke test;
- frame-driven movement monitoring and recovery wiring;
- controlled packet-release gates and combat-safe live mutation acceptance;
- strategist behavior and persistent memory.

The binary now initializes MCP, registers or finds the selected character, logs it in, and starts continuous perception. The body actor owns the only mutating gateway and can execute validated packets. The default `observe_only` rollout never asks the model to decide. `shadow` runs the model but never releases its proposal. `full` is rejected unless `NPC_ALLOW_LIVE_MUTATION=true`. The live attack check remains deferred until a verified hostile target is available.

## Verify

```bash
cd rust-harness
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

## Run the session and skeleton

```bash
ARENA_API_KEY=... \
NPC_CHARACTER=guy \
NPC_CHARACTER_SHEET_PATH=characters/guy.json \
cargo run
```

The process connects and logs the character in. It starts the actor tree and waits for Ctrl-C. Set `NPC_RUN_DURATION_SECONDS` to a positive integer for a bounded run. On shutdown, it disconnects the character. `OPENROUTER_API_KEY` is required when the rollout mode permits model inference. Model configuration is checked before the game session is created.

Use `NPC_IDLE_TACTICAL_HZ=0` to disable quiet-room heartbeat inference. Material events can still wake the tactician.

## Controlled MCP smoke test

The smoke binary is explicit. It disconnects after one command. The default command is read-only.

```bash
cargo run --bin mcp-smoke -- read-only
cargo run --bin mcp-smoke -- tool-inventory
cargo run --bin mcp-smoke -- history 50
cargo run --bin mcp-smoke -- history-roundtrip
```

The read-only command tests login, observation, map, and inventory. It prints counts and status only. It does not print the payload.

Use mutation commands only with an approved test character and known targets:

```bash
cargo run --bin mcp-smoke -- say "Rust harness smoke test."
cargo run --bin mcp-smoke -- say-global "A short world message."
cargo run --bin mcp-smoke -- say-private "Cassian Vey Unbound" "A private test."
cargo run --bin mcp-smoke -- chat-read
cargo run --bin mcp-smoke -- play-melody lute 1 "C E G C5"
cargo run --bin mcp-smoke -- move left
cargo run --bin mcp-smoke -- live-walk
cargo run --bin mcp-smoke -- live-perception
cargo run --bin mcp-smoke -- move-to 336 368
cargo run --bin mcp-smoke -- attack-object spider_layer_92
```

All commands emit the same correlated MCP analytics events as the runtime.
The `live-walk` command keeps the test character connected and alternates short left and right movements until Ctrl-C. It exists only for controlled live visibility tests. It is not character behavior or a production movement policy.
The `live-perception` command adds observe, map, inventory, normalization, revision, and derived-event checks to the same bounded walk. It records counts and known-field flags. It does not record raw world payloads.

## Tactical model probe

The probe sends one fixed combat frame to each configured fast model. It does not connect to Agent Arena and cannot mutate the game.

```bash
OPENROUTER_API_KEY=... cargo run --bin tactical-probe
```

The current default candidates are Gemini 3.1 Flash Lite and GPT-OSS Safeguard 20B. The probe records requested and actual model identifiers, prompt version, latency, token use, cached tokens, exact OpenRouter charge, parse success, intent, and action count. It does not log prompts or raw model output.

Each real model call records every token category that Rig exposes, the exact OpenRouter response charge, the generation ID, and per-character running totals. A background accounting call records the finalized provider, native token counts, cache data, service tier, provider latency, and exact generation charge. It does not block tactical inference.

The strategist uses explicit OpenRouter reasoning by default. The tactician does
not, so immediate decisions retain their latency-oriented policy. Configure the
roles independently:

```bash
NPC_STRATEGIST_REASONING_ENABLED=true
NPC_STRATEGIST_REASONING_EFFORT=medium
NPC_STRATEGIST_REASONING_EXCLUDE=true
NPC_STRATEGIST_MAX_OUTPUT_TOKENS=4000

NPC_TACTICIAN_REASONING_ENABLED=false
NPC_TACTICIAN_REASONING_EFFORT=minimal
NPC_TACTICIAN_REASONING_EXCLUDE=true
```

Effort must be `minimal`, `low`, `medium`, or `high`. The completion budget is
shared by reasoning and the structured answer, so enabled reasoning requires at
least 512, 1,000, 2,000, or 4,000 tokens respectively. Known model and effort
mismatches fail at startup. Unknown models are sent through Rig with
OpenRouter's `provider.require_parameters=true`, so an endpoint cannot silently
drop the reasoning request.

Every model lifecycle event records requested reasoning state, effort,
exclusion, completion budget, finish reason, model, role, prompt version,
latency, character, and causal decision/revision IDs. Finalized OpenRouter
generation accounting reconciles native reasoning and cached-token counts into
per-character totals. The harness never records private reasoning content.

To confirm the exact logical request assembled for a local diagnostic, enable:

```bash
NPC_LOCAL_MODEL_INPUT_CAPTURE_ENABLED=true
NPC_LOCAL_MODEL_INPUT_CAPTURE_PATH=./var/model-input-captures
```

Each capture contains the preamble, Rig-bounded history, current serialized
typed input, output schema, provider parameters, and non-secret model settings.
The ignored directory and files use modes 0700 and 0600 on Unix. This is
sensitive character/context data and must not be enabled in normal production
operation. Normal analytics receive only a deterministic request fingerprint,
byte and token estimates, history-message count, prompt/schema facts, and cache
accounting—never the request content or API key.

A first request cannot reuse a provider prompt cache. Later requests may still
show no cache use when their stable prefix is below a provider threshold, the
bounded conversation window changed, routing selected a different provider, or
the provider did not expose cache details in the immediate response. The
background generation audit records the actual routed provider and reconciles
its native cache count with Rig's immediate cache fields.

`NPC_OPENROUTER_PROMPT_CACHING_ENABLED=true` uses Rig 0.41's native `OpenRouter`
support to serialize the system message as a content-block array and place an
ephemeral `cache_control` breakpoint on its final content block. This is the
per-message/content shape required by OpenRouter; it is not a top-level
`cache_control` request parameter. The harness also supplies a stable,
non-secret OpenRouter
`session_id` per character and cognitive role to improve routing stickiness.
The cache telemetry records the stable-prefix estimate and whether it reaches
the usual 4,096-token Gemini threshold. The harness does not pad prompts just
to manufacture cache eligibility; short tactical prompts may correctly remain
uncached.

## Strategic memory retrieval

The default build includes the `local-rag` feature. On the first strategist
recall that has durable long-tail records, Rig initializes FastEmbed's quantized
All-MiniLM-L6-v2 model and builds an in-memory vector index from SQLite. The
model assets are about 24 MB and are cached by the local model loader. Empty
memory and ordinary tests do not initialize or download the model.

```bash
NPC_LOCAL_RAG_ENABLED=true
NPC_LOCAL_RAG_MINIMUM_SCORE=0.25
```

SQLite remains authoritative. The derived index is character-filtered, rebuilt
lazily, and invalidated after fact, relationship, or episode writes. If local
model initialization or vector search fails, the strategist uses the bounded
deterministic lexical result and records the fallback reason and latency. Build
with `--no-default-features` for a lean binary that always uses the lexical
adapter; startup emits `memory.rag_feature_unavailable` when RAG was configured
but omitted at compile time.

RAG events record model/index version, latency, requested and returned counts,
kind counts, score range, and stable error class. They never record queries,
memory text, payloads, or embeddings.

Check current provider endpoint prices without making a model call:

```bash
OPENROUTER_API_KEY=... cargo run --bin openrouter-accounting
```

The price snapshot keeps OpenRouter's exact decimal strings and numeric query values. It is reference data, not an estimated bill.

## Module seams

- `mcp::client::ArenaGateway` is the only module that knows MCP tool arguments or injects `agent_id`.
- `mcp::transport::HttpMcpTransport` owns JSON-RPC, HTTP headers, SSE termination, timeouts, and the MCP session identifier.
- `mcp::session::ArenaSession` owns initialize, registration, login, reconnect generation, invalidation, and disconnect.
- `observability::AnalyticsSink` is the event seam. Production uses structured tracing. Tests use an in-memory recorder.
- `actors::body::BodyActor` is the only actor permitted to own that gateway once transport lands.
- `runtime::blackboard::HotBlackboard` is shared typed truth, not shared chat history.
- `brain::Brain<I, O>` is the model seam. Rig is one adapter; replay fakes will be another.
- `execution::validator` is deterministic and decides legality only. It never decides what Guy should want or do.

The remaining Phase 2 milestone is the controlled live attack check. Phase 3 then normalizes observations and maps into tactical facts.
