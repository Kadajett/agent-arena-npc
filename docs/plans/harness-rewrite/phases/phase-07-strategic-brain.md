# Phase 07: Strategic Brain

State: In progress

## Goal

Move Guy's long-horizon cognition and social behavior to the Rust StrategistActor.

## Required result

Guy remains the same recognizable character. He keeps goals, plans, relationships, promises, knowledge, and curiosity while the tactician owns immediate survival.

## Dependencies

Phase 06 must prove that the tactical path can keep the character safe during slow strategic work.

Phase 08 can develop its storage adapter in parallel. Phase 07 must not depend on unbounded generic chat history.

## Strategic responsibilities

The StrategistActor owns:

- identity and persona use;
- long-term goals;
- plan creation and revision;
- task lists;
- promises and errands;
- exploration motivation;
- relationships;
- trust and grudges;
- curiosity;
- social responses;
- economic goals;
- discovery interpretation;
- goal completion;
- new goal selection;
- strategic navigation goals;
- `StrategicIntent` publication.

The StrategistActor must not issue immediate combat button presses.

## Strategic intent tasks

- [x] Define model output for objective, subgoals, priorities, constraints, preferred targets, avoid items, and risk tolerance.
- [x] Add an optional navigation goal.
- [x] Add an expiry time when the intent is temporary.
- [x] Let the runtime increment the strategic revision for each material change.
- [x] Publish the new intent to the blackboard.
- [x] Notify the tactician immediately.
- [x] Preserve the intent while new strategic inference runs.

The tactician receives the structured intent. It does not receive the strategist transcript.

## Goal and plan tasks

- [ ] Seed an empty memory from the character sheet goal.
- [ ] Preserve a goal that Guy selected himself.
- [ ] Let a changed sheet goal redirect the character.
- [ ] Drop a plan that belongs to an old goal.
- [ ] Track plan steps and their outcomes.
- [ ] Detect repeated blocked steps.
- [ ] Detect a step with no progress.
- [ ] Replan with named failures.
- [ ] Let Guy select a new goal when the current goal ends.
- [ ] Keep tasks that do not belong to the main goal in a separate list.

## Social tasks

- [ ] Receive important chat and dialogue events.
- [ ] Filter engine chatter.
- [ ] Keep a bounded active conversation.
- [ ] Remember recurring people.
- [ ] Update relationships from evidence.
- [ ] Preserve grudges and trust changes.
- [ ] Track who requested a favor.
- [ ] Keep room-specific conversation context.
- [ ] Prevent repeated speech on the same subject.
- [ ] Route `say`, `talk_to`, `choose`, and `end_talk` through the BodyActor.

Normal conversation must not enter the tactician input.

## Knowledge tasks

- [ ] Track visited places.
- [ ] Track places known by local knowledge.
- [ ] Track rumors with their source.
- [ ] Keep firsthand knowledge separate from hearsay.
- [ ] Prevent hearsay from replacing firsthand knowledge.
- [ ] Track discoveries and important failures.
- [ ] Keep short-lived notes with expiry.

## Economic and exploration tasks

- [ ] Read balance and inventory facts.
- [ ] Decide what to buy or sell.
- [ ] Decide where to explore.
- [ ] Produce navigation goals instead of local movement commands.
- [ ] Decide when an exploration objective is complete.
- [ ] Decide whether a completed combat episode changes the larger plan.

## Wake rules

Wake the strategist after:

- important chat;
- meaningful dialogue;
- goal completion;
- plan exhaustion;
- repeated strategy failure;
- a new person;
- a new place;
- an important discovery;
- a completed combat episode;
- a tactician report that the strategy is impossible;
- a slow periodic reflection interval.

Do not wake it after every damage event.

Run only one strategic inference task under normal conditions.

## Implemented inference lane

The strategic model receives `StrategicInput`. The input contains the character ID, the persona, the current intent, and at most 32 recent strategic moments. Each moment summary contains at most 500 characters. The input does not contain an MCP client.

The model returns `StrategicProposal`. The proposal does not contain a revision. The StrategistActor compares the proposal with the current intent. The actor assigns the next revision only when the content changed.

The actor starts model work in a Tokio task. The actor mailbox stays available while the task runs. The actor permits one model call at a time. If a newer wake arrives, the actor keeps the newest bounded input and marks the older result as superseded. A provider or parse failure does not stop the actor and does not start an automatic retry loop.

The lane uses prompt `strategist/v2`. Telemetry contains the decision ID, input revision, base strategic revision, duration, result class, and published revision when present. Telemetry does not contain the prompt, model output, persona, or moment text.

The player runtime builds `OpenRouterJsonBrain<StrategicInput, StrategicProposal>` through the existing Rig adapter. It uses cognitive role `strategist` and prompt version `strategist/v2`. The supervisor installs the brain with `StrategistMsg::InstallBrain` and sends the initial reflection. `NPC_STRATEGIST_ENABLED` controls this lane and defaults to `false`. The strategist does not receive `ArenaGateway`, `BodyGateway`, or another MCP transport.

## Production evidence

A bounded production shadow ran Cassian with GPT-OSS 20B as strategist and
Llama 3.2 3B as tactician. The strategist published revision 2 after 8,648 ms.
Perception and tactical inference continued during the strategic call. A later
tactical input used strategic revision 2. The models had no production mutation
authority in this run.

The trace recorded separate role, prompt version, tokens, cached tokens, exact
cost, actual provider, and finalized native reasoning facts. The total exact
charge for the run was `$0.0012938211`.

Normal shutdown terminalizes any in-flight strategic or tactical decision with
reason `runtime_shutdown`. The accounting registry then waits for active calls
and the final-audit tasks they register. It reports any active calls that remain
at the deadline.

## Tests

- [ ] Personality stays stable across restart.
- [ ] The initial goal seeds empty memory.
- [ ] A self-selected goal survives restart.
- [ ] A changed sheet goal replaces old work.
- [ ] A plan survives restart.
- [ ] Repeated failure causes a replan.
- [ ] A task remains separate from the main plan.
- [ ] Relationship changes require evidence.
- [ ] Hearsay cannot replace a visited place.
- [x] Strategy revision reaches the tactician.
- [x] Slow strategic inference does not block the strategist mailbox or tactical work.
- [x] The strategist has no direct mutable MCP adapter.

## Acceptance criteria

Phase 07 is complete when:

1. Guy behaves according to his persona and current goal.
2. Goals and plans persist through the memory interface.
3. Social behavior preserves important current TypeScript behavior.
4. The strategist publishes structured intent.
5. The tactician receives strategy changes immediately.
6. Immediate combat remains outside the strategist.
7. All strategic invariants pass.

## Out of scope

Do not migrate old SQLite records in this phase. Phase 08 owns data migration.
