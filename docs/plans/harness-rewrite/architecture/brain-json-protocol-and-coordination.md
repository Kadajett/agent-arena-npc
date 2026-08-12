# Brain JSON Protocol and Coordination State

## Purpose

This document defines the communication boundary between the runtime and each brain.

The runtime sends one typed JSON document to a brain. The brain returns one typed JSON document. The runtime does not use model prose as a control protocol.

## Transport and validation

Rig sends the input document as the content of one user message. OpenRouter returns the model response as assistant text. The response text must contain one JSON document.

The runtime applies these checks in order:

1. The response must be JSON.
2. The JSON must match the Rust output type.
3. Unknown fields must cause rejection.
4. The value must pass semantic validation.
5. The proposal must pass current-world validation.
6. The body must check material invalidation before each action.

The runtime must reject Markdown fences. The runtime must also reject a JSON string that contains another JSON document. These forms hide provider failures and make model comparisons inaccurate.

The runtime owns IDs, revisions, timestamps, packet lifetimes, and character identity. A model cannot supply these values.

## Brain inputs and outputs

The tactician receives `TacticalInput` and returns `TacticalProposal`.

The strategist receives `StrategicInput` and returns `StrategicProposal`.

Each input contains an explicit `protocol_version`. Prompt versions and output-schema fingerprints identify the corresponding output contract in telemetry.

The strategist does not send a message directly to the tactician. The runtime converts an accepted strategic proposal into typed durable working memory and a revisioned `StrategicIntent`. Perception includes that intent in the next tactical input.

The body sends a typed `NavigationArrival` to perception when it reaches a destination. Perception forwards the fact to the strategist as a `navigation_arrived` moment. The moment contains exact destination and arrival fields. It does not use a prose-only success message.

Arrival wakes the strategist. Arrival does not automatically complete a plan step. For example, arrival near a person does not prove that a conversation occurred.

## Movement coordination

`MovementControl` states which subsystem owns local movement.

Example:

```json
{
  "owner": "body_strategic_navigation",
  "state": "assigned",
  "strategic_revision": 10,
  "destination_scene": "bot-forest",
  "destination_tile": {
    "x": 17,
    "y": 13
  },
  "tactical_preemption_allowed_for": [
    "combat_active",
    "hostile_targeting_self",
    "damage_taken",
    "movement_failure"
  ],
  "tactical_preemption_facts_present": []
}
```

The two fact lists have different meanings.

`tactical_preemption_allowed_for` is runtime policy. `tactical_preemption_facts_present` is current evidence.

The tactician can propose a movement override. The executor permits that override only when the current frame contains an allowed fact. The executor uses the same deterministic reducer that creates `tactical_preemption_facts_present`.

This rule does not decide if Guy should flee. It only decides if the tactician can take movement control from an active strategic navigation mission.

## Context bounds

The runtime must bound context with typed collections and token budgets. It must not silently cut a string at an arbitrary character position.

The current tactical projection keeps the last 20 event records and the last 10 action records. It preserves each selected record. The strategist keeps a bounded queue of complete moments.

If a future provider limit requires a smaller input, the runtime must remove complete low-priority records or replace them with a typed summary artifact. The artifact must include its source range and compaction method. The runtime must record this operation in telemetry.

## Observability

For each brain call, record:

- input type and schema version;
- prompt version;
- input fingerprint and byte count;
- output schema fingerprint and byte count;
- provider and model;
- parse result;
- semantic validation result;
- world validation result;
- revisions and causal IDs;
- model tokens, cached tokens, reasoning tokens, cost, and latency.

Exact input capture stays optional because it can contain private dialogue and character memory. When enabled, files must remain outside version control and must use private file permissions.

## Next state-machine step

The first coordination state is movement ownership. Later states can use the same pattern for combat control, dialogue ownership, and plan-step execution.

Add a new state only when live traces show a repeated ownership conflict. Do not add a state to encode a gameplay choice that belongs to a brain.

### Live finding: plan execution needs a state machine

The first production run with this protocol showed the next required state machine.

Strategist produced an eight-step durable plan. Body reached the first destination. Strategist received the typed arrival fact and updated the plan. However, the first step still contained two operations in prose: move to a wall, then interact with the wall. Only the navigation goal was executable. No runtime state connected arrival to a validated interaction operation.

Strategic output now includes a typed `actions` array. The initial immediate
operation is:

```json
{"type":"interact","target_id":"ground537"}
```

The target must be an exact, visible, interactable world object within two
tiles. Players and enemies are rejected by the body executor. The legacy
`interaction_target_id` field remains accepted for migration, but new prompts
must prefer `actions`.

Future protocol revisions may add typed `next_operation` variants such as:

- navigate to a validated destination;
- approach a visible entity at a validated adjacent tile;
- start dialogue with a visible and interactable NPC;
- send a reply through the runtime-selected channel;
- wait for a named observable condition.

The runtime must own operation state:

```text
pending
  -> dispatching
  -> awaiting_evidence
  -> completed | blocked | superseded
```

Each operation must refer to one runtime-reconciled plan step. The model must not supply the step UUID, status, attempt count, or evidence. The runtime must reject an impossible operation before an MCP mutation and return a typed rejection fact to Strategist.

This state machine must not decide which object is interesting or which person to question. Strategist makes those choices. The state machine only validates and tracks the operation that Strategist selected.
## Thought channels

The runtime distinguishes two different kinds of thought data:

1. `arena_think` is an explicit, short, spectator-visible thought that the
   character chooses to save. It is an action-side effect and may be emitted
   immediately before a strategic body action.
2. Provider reasoning is an automatic model-run hook. When Rig/OpenRouter
   exposes reasoning content, the harness records it against the model
   decision, prompt version, provider generation, token usage, and cost. It is
   not a substitute for an action and must not be fabricated when the provider
   returns only usage counts.

The two channels share correlation IDs but remain separate in analytics. This
lets the UI show model reasoning, explicit character thoughts, and physical
actions without implying that any one of them caused the others. Reasoning
content capture is opt-in and redacted by default; token counts and whether
reasoning was available are always recorded.
