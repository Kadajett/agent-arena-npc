# Phase 12: Parity and Cutover

State: Planned

## Goal

Prove user-visible parity, move production to the Rust harness, and retire the TypeScript harness safely.

## Required result

Guy runs on the Rust harness in production. The Rust harness preserves required character behavior and improves immediate reactions. Operators can roll back during the cutover window.

## Dependencies

Phases 01 through 11 must be complete.

The replay corpus and live controlled tests must pass with the selected production configuration.

## Behavior parity checklist

- [ ] Character identity.
- [ ] Persona.
- [ ] Character sheet.
- [ ] Capabilities.
- [ ] Player registration.
- [ ] Login.
- [ ] Reconnect.
- [ ] Goals.
- [ ] Plan persistence.
- [ ] Task list.
- [ ] Short-lived notes.
- [ ] Relationships.
- [ ] Remembered people.
- [ ] Known places.
- [ ] Hearsay and firsthand knowledge.
- [ ] Room conversation memory.
- [ ] Anti-repetition behavior.
- [ ] Chat filtering.
- [ ] Map exploration.
- [ ] Door handling.
- [ ] Fighting.
- [ ] Backend-authoritative skills.
- [ ] Duels.
- [ ] Money.
- [ ] Inventory.
- [ ] Item use.
- [ ] Pickup.
- [ ] Trade.
- [ ] Feeling or status indicator.
- [ ] Durable memory.

Each item must have one of these records:

- a Rust parity test;
- a live acceptance test;
- an explicit decision that the old behavior is obsolete.

## Success behavior

The cutover candidate must show these behaviors:

- Guy moves while the strategist thinks.
- Guy reacts to combat without waiting for strategic reasoning.
- Guy notices repeated enemy replacement spawns.
- Guy decides whether to fight, loot, heal, or flee.
- Guy uses only legal class actions.
- Equipped gear works through backend combat rules.
- Guy can stop or recover from stalled movement.
- Guy preserves personality, plans, relationships, and memory.
- Guy sees an accurate structured and ASCII local map.
- Actor failures do not destroy unrelated state without need.
- Tactical decisions are replayable.
- MCP remains the only game access seam.

## Deployment tasks

- [ ] Build a production Rust container image.
- [ ] Preserve least-privilege container settings.
- [ ] Mount the new memory path.
- [ ] Add a health check.
- [ ] Add graceful termination time.
- [ ] Add production environment documentation.
- [ ] Add model and rate configuration.
- [ ] Add event and trace destinations.
- [ ] Run the memory migration dry-run.
- [ ] Back up the old memory database.
- [ ] Run the final memory migration.

## Character migration tasks

Guy is the first production target. TypeScript removal also requires a decision for every deployed character.

- [ ] List every deployed character.
- [ ] Port each required character sheet and persona.
- [ ] Preserve stationary behavior where it is intentional.
- [ ] Preserve deterministic routine behavior where it is intentional.
- [ ] Preserve autonomous behavior where it is intentional.
- [ ] Run identity and capability tests for each migrated character.
- [ ] Record characters that are intentionally retired.

Do not remove the shared TypeScript runtime while a required deployed character still depends on it.

### Authorized production set

The first persistent Rust deployment has three characters.

| Character | Required behavior | Cutover rule |
| --- | --- | --- |
| Guy | Full autonomous strategic and tactical behavior | Migrate typed memory and conversation history where safe. Complete the live movement, combat, social, memory-restart, and accounting gates. |
| Barnaby | Innkeeper conversation and durable social memory | Keep `Speak` and `TalkToFolk`. Do not grant `Walk` or `Doors`. Assert the inn scene at startup. Migrate typed memory and conversation history where safe. |
| Cassian | Full autonomous strategic and tactical behavior | Complete the same live safety and memory gates as Guy. Then release him as a persistent Rust character. |

Barnaby's confinement is a body capability policy. It is not a prompt request
and it is not a movement-failure workaround. His knowledge of other places does
not grant physical movement.

### VPS process retirement

After the three cutovers pass, stop the other NPC harness deployments that the
operator owns on the production VPS. This authorization does not apply to
unrelated services, Agent Arena backend processes, or characters owned by other
users.

- [ ] Inventory running containers and process supervisors without changing them.
- [ ] Match each target to this repository, a character identifier, and an owned deployment record.
- [ ] Mark Guy, Barnaby, and Cassian as retained targets.
- [ ] Record the last backend history cursor for every retirement target.
- [ ] Back up each retirement target's memory volume or database.
- [ ] Verify each backup can be opened and list its record counts.
- [ ] Stop only the positively identified NPC harness process or container.
- [ ] Disable its automatic restart policy when necessary.
- [ ] Verify that its model-call count stops increasing.
- [ ] Keep the backup. Do not delete the character or its game account.
- [ ] Produce a report with process or container name, character, backup path,
  history cursor, stop time, and stop result.

Do not use a broad process-name kill. Do not stop a container from image name
alone. Do not stop a process when character ownership is unknown.

### Production inventory: 2026-08-12

The owned production stack is the Docker Compose project `deploy` in
`/opt/agentArena/deploy` on `2.25.100.234`.

Retain the infrastructure services `deploy-world-1`, `deploy-gateway-1`, and
`deploy-db-1`. They are not NPC retirement targets.

The point-in-time NPC inventory contained these containers:

- `deploy-guy-1`;
- `deploy-barnaby-1`;
- `deploy-ash-1`;
- `deploy-tansy-1`;
- `deploy-cutter-1`;
- `deploy-hollis-1`;
- `deploy-wanderer-1`;
- `deploy-marren-1`;
- `deploy-aveline-1`;
- `deploy-doran-1`;
- `deploy-nerys-1`.

All listed NPC containers belong to the same Compose project and mount the
shared `deploy_npc_var` volume. Their restart policy is `always`. Stopping a
retired container is incomplete unless Compose no longer recreates it or its
restart policy is changed through the managed deployment definition.

This inventory does not authorize stopping the infrastructure services or any
process outside this Compose project. Reconfirm the service labels and
character identity immediately before cutover because the inventory can change.

## Cutover stages

Use these stages:

1. Run the Rust harness in observe-only mode.
2. Run shadow tactical decisions.
3. Run a dedicated Rust test character.
4. Run Guy on Rust for a limited window.
5. Keep the TypeScript image ready for rollback.
6. Increase the Rust production window.
7. Declare Rust the production harness.
8. Remove the TypeScript runtime after the rollback window ends.

Never run two mutating harnesses for the same character.

## Rollback plan

- [ ] Stop the Rust BodyActor before rollback.
- [ ] Disconnect the Rust MCP session.
- [ ] Preserve Rust traces and memory writes.
- [ ] Restore the last compatible TypeScript memory backup when required.
- [ ] Start the TypeScript harness with the same character identity.
- [ ] Verify a fresh observation before normal operation.
- [ ] Record the rollback reason as a fixture or issue.

## Removal tasks

After the rollback window:

- [ ] Remove Mastra runtime dependencies.
- [ ] Remove the TypeScript harness source.
- [ ] Remove obsolete TypeScript tests.
- [ ] Keep useful captured fixtures.
- [ ] Keep migration tools and reports.
- [ ] Update the root README.
- [ ] Update Docker and deployment files.
- [ ] Remove obsolete environment variables.
- [ ] Archive the final parity record.

Do not delete the TypeScript harness before the cutover criteria pass.

## Final tests

- [ ] Full Rust test suite.
- [ ] Full tactical replay corpus.
- [ ] Live movement suite.
- [ ] Live combat suite.
- [ ] Live social suite.
- [ ] Reconnect and failure suite.
- [ ] Memory restart and migration suite.
- [ ] Container health and shutdown suite.
- [ ] Rollback rehearsal.

## Post-deploy lore and treasure audit

Run this audit after the production deployment has accumulated enough activity.

- [ ] List every managed character and its authoritative game identity.
- [ ] Read each character's ordered backend history from the saved deployment cursor.
- [ ] Read each character's persisted goal, plan, working state, and semantic memory.
- [ ] List lore learned through firsthand events.
- [ ] List lore learned through hearsay and retain the source.
- [ ] List locations visited and locations known only from reports.
- [ ] List treasure clues, their sources, and their current status.
- [ ] List treasure and valuable items confirmed by inventory or pickup events.
- [ ] List active, completed, abandoned, and blocked treasure leads.
- [ ] Report movement, social, combat, loot, and exploration success rates.
- [ ] Report model calls, tokens, cache use, and exact cost for the same period.
- [ ] Record unknown or unsupported facts instead of estimating them.

Produce one per-character section and one comparison table. Keep these evidence
classes separate:

1. `confirmed`: The backend event or inventory state proves the fact.
2. `firsthand_memory`: The character observed the fact, but current world state
   does not independently confirm it.
3. `hearsay`: Another character or non-player character supplied the fact.
4. `strategic_belief`: The strategist inferred or proposed the fact.
5. `unknown`: The available data does not prove the fact.

Do not report a model claim as a confirmed treasure discovery. Link each
confirmed result to stable backend event identifiers and each memory result to
stable memory identifiers.

## Acceptance criteria

Phase 12 is complete when:

1. Every parity item has a recorded result.
2. Guy meets the target success behavior in production.
3. Rust traces explain important actions.
4. Memory migration is verified and backed up.
5. The rollback rehearsal passes.
6. The production observation window has no unresolved critical failure.
7. The TypeScript runtime is no longer required.
8. Repository and deployment documentation describe Rust as the production harness.

## First-release non-goals

Do not add:

- distributed actor clusters;
- orchestration for thousands of agents;
- a giant vector store;
- global semantic search over all events;
- multi-region execution;
- automatic fine-tuning;
- a large deterministic behavior tree;
- a complex model routing system.

Keep interfaces open to later adapters. Do not pay the implementation cost now.

## Final architecture statement

The backend owns reality.

The PerceptionActor turns reality into facts.

The StrategistActor decides what Guy wants.

The TacticianActor decides what Guy must do now.

The BodyActor is Guy's hands.

Ractor keeps these modules independent.

Rig gives each brain the configured model.

MCP is the only doorway into the game.
