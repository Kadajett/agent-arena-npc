# Rust Harness Rewrite Plan

This directory is the authoritative plan for the Agent Arena NPC harness rewrite.

The plan replaces the TypeScript and Mastra runtime with a Rust runtime. The new runtime uses Ractor for actors and Rig for model access. The existing MCP gateway remains the only doorway into the game.

## Writing rules

These documents use ASD-STE100-style technical English where practical.

- Use short sentences.
- Use one term for one concept.
- Use active voice.
- Use `must` for a requirement.
- Use `can` for a permitted action.
- Use `do not` for a prohibition.
- Do not use an undefined abbreviation.

## Fixed system rules

The following rules apply to every phase:

1. The backend owns reality.
2. The PerceptionActor turns reality into typed facts.
3. The StrategistActor decides what the character wants.
4. The TacticianActor decides what the character must do now.
5. The BodyActor is the only actor that can mutate the game.
6. MCP is the only game access seam.
7. Ractor owns runtime concurrency and actor lifecycle.
8. Rig supplies model access. Rig does not own the runtime.
9. Deterministic code validates facts and actions. It does not choose gameplay behavior.
10. The TypeScript harness remains available until the Rust harness passes the cutover phase.

See [Architecture Rules](architecture/rules.md) for the complete rule set.

## Phase sequence

| Phase | Name | State | Main result |
| --- | --- | --- | --- |
| 01 | [Runtime skeleton](phases/phase-01-runtime-skeleton.md) | In progress | The actor tree starts, communicates, and stops safely. |
| 02 | [Typed MCP and session layer](phases/phase-02-typed-mcp-and-session.md) | In progress | Local checks and safe live checks pass. The live attack check remains. |
| 03 | [Perception and tactical facts](phases/phase-03-perception-and-tactical-facts.md) | In progress | MCP state becomes an authoritative tactical frame. |
| 04 | [Body execution and movement](phases/phase-04-body-execution-and-movement.md) | In progress | Valid action packets execute safely and support preemption. |
| 05 | [Tactical brain](phases/phase-05-tactical-brain.md) | In progress | The fast model makes short, event-driven decisions. |
| 06 | [Concurrent live combat](phases/phase-06-concurrent-live-combat.md) | In progress | Guy reacts while other reasoning continues. |
| 07 | [Strategic brain](phases/phase-07-strategic-brain.md) | In progress | Long-horizon character behavior moves to Rust. |
| 08 | [Memory and migration](phases/phase-08-memory-and-migration.md) | In progress | Durable memory survives restarts and migration. |
| 09 | [Ordered events and episodes](phases/phase-09-ordered-events-and-episodes.md) | In progress | Authoritative events produce deterministic episode facts. |
| 10 | [Replay evaluation](phases/phase-10-replay-evaluation.md) | In progress | Recorded game cases select tactical models. |
| 11 | [Operational hardening](phases/phase-11-operational-hardening.md) | In progress | Failures, reconnects, limits, and telemetry are production-safe. |
| 12 | [Parity and cutover](phases/phase-12-parity-and-cutover.md) | Planned | Rust becomes the production harness and TypeScript is retired. |

Do not start a later phase if its required input from an earlier phase is not stable. Small parallel tasks are permitted when they do not create a second source of truth.

## Supporting plans

- [Architecture Rules](architecture/rules.md) defines actor roles, seams, revisions, and invariants.
- [Rig Memory and Local RAG Decision](architecture/rig-memory-and-local-rag.md) defines the conversation-memory, typed-memory, and semantic-retrieval split.
- [Strategic Planning and Navigation](architecture/strategic-planning-and-navigation.md) defines the seam between long-horizon plans and `move_to`-first body execution.
- [Requirements Catalog](requirements-catalog.md) preserves the cross-phase requirements from both source plans.
- [Local Test Strategy](testing/local-test-strategy.md) defines safe local tests and live test controls.
- [Phase 02 Live Smoke Test](testing/live-smoke-2026-08-11.md) records the live contract findings without credentials or private payloads.
- [OpenRouter Accounting Smoke Test](testing/model-accounting-smoke-2026-08-11.md) records live provider, token, cache, and exact-cost verification.
- [Model Input, Cache, and Timeout Audit](testing/model-input-cache-and-timeout-audit-2026-08-12.md) verifies exact logical model inputs, stable prefixes, cache eligibility, and strategist timeout behavior.
- [Autonomous Combat Test: 2026-08-12](testing/autonomous-combat-2026-08-12.md) records the first combat run, its cost, the safety corrections, and the current model evaluation.
- [Engine Bug Reporting](testing/engine-bug-reporting.md) defines when and how to file a GitHub issue against Agent Arena.
- [Source Traceability](traceability.md) maps the source plans and current behavior to the phase files.
- [Observability Event Catalog](observability/event-catalog.md) defines event names, required dimensions, redaction, and causal links.

## Plan maintenance

Update a phase document when implementation work changes its interface, tasks, tests, or exit criteria.

Use these state values:

- `Planned`: No required implementation is complete.
- `In progress`: Some required implementation is complete.
- `Blocked`: A named external dependency prevents useful work.
- `Complete`: All exit criteria pass.

Do not mark a phase complete because its code compiles. Mark it complete only after all acceptance tests and exit criteria pass.
