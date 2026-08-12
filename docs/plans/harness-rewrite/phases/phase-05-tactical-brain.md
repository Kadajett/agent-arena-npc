# Phase 05: Tactical Brain

State: In progress

## Goal

Add the fast, stateless tactical model loop. Wake it only when the immediate situation needs a new decision.

## Required result

The TacticianActor receives the newest tactical facts and strategic intent. It returns a valid `TacticalProposal`. The runtime rejects stale results and sends accepted packets to the BodyActor.

## Dependencies

Phase 03 must provide stable tactical frames and material-change events.

Phase 04 must provide the proposal-to-packet conversion and safe execution.

## Model input

The tactical input is a compact type derived from the full runtime frame. It must contain only:

- the current tactical frame;
- the current strategic intent;
- recent action outcomes;
- recent important events;
- current combat episode facts.

The deterministic runtime keeps structured map tiles. The model input does not include them. The model receives:

- compact map metadata and ASCII;
- exact visible enemies and other entities;
- exact visible drops;
- exact doors and reachable exits;
- carried items and legal combat actions.

Keep at most the newest 20 important events and ten action outcomes. Remove runtime identifiers and timestamps that do not help the decision. Truncate free-form action detail before it reaches the prompt.

Do not include:

- full conversation history;
- long-term relationship records;
- old planning chains;
- full world lore;
- generic semantic memory retrieval;
- the strategist's model transcript.

## Model output

Use structured output for `TacticalProposal`.

Keep output short. Do not request prose.

The proposal can use these actions:

- move to a tile;
- stop;
- basic attack;
- use a legal skill;
- use a carried item;
- pick up a visible drop.
- set the backend combat style and mode.

The proposal must select only IDs and actions that exist in the frame.

## Rig and model tasks

- [x] Build the tactical Rig brain from configuration.
- [x] Use OpenRouter first.
- [x] Set low temperature.
- [x] Set a small output-token limit.
- [x] Use the versioned tactical prompt.
- [x] Record the requested model, actual model, prompt version, and generation ID.
- [x] Record Rig token categories and the exact OpenRouter charge returned with each response.
- [x] Fetch the finalized OpenRouter generation record outside the tactical hot path.
- [x] Snapshot current provider endpoint prices without treating them as a bill.
- [x] Parse only the structured output type.
- [x] Return a typed model failure.
- [x] Do not give MCP tools to the tactical model.

Start tests with these candidate model families:

- Llama 3.2 3B Instruct;
- Nemotron 3 Nano 30B A3B;
- newer models with less than 3B active parameters when their latency is competitive.
- current flash and sparse models when their measured latency is competitive.

Model selection belongs to Phase 10.

OpenRouter has two different accounting facts. Keep them separate. The response and generation record contain the exact charge for completed work. A provider endpoint snapshot contains the advertised unit prices at one observed time. Never reconstruct an actual bill from a price snapshot when OpenRouter supplied the exact charge.

Every completion event must be attributable to the stable character ID and cognitive role. It must include input, output, total, cached-input, cache-write, tool-prompt, and reasoning token fields from Rig. It must also include process-local per-character totals. Durable totals come from aggregating the append-only events, not from the process counter.

## Wake rules

Wake immediately after these material events:

- damage;
- hostile spawn or despawn;
- target death;
- health threshold change;
- movement failure;
- action rejection;
- new loot;
- inventory change;
- strategic revision;
- combat start;
- combat end.

Use a bounded heartbeat during active combat.

Use a much lower heartbeat while idle. Do not call the model continuously in an empty safe room.

Treat `NPC_TACTICAL_MAX_HZ` as a ceiling. Do not treat it as a required polling rate.

## Backpressure rules

Keep only the newest frame.

Do not queue every tactical frame.

When inference is active:

1. Accept new frame messages.
2. Store the newest material frame.
3. Mark that a new decision can be required.
4. Check revisions when inference completes.
5. Discard an outdated result.
6. Start one new inference if the current situation still needs one.

Cancel old provider work when cancellation is safe and useful. Correctness must not depend on provider cancellation.

## Failure behavior

If inference fails:

- record the failure;
- keep receiving world updates;
- do not invent an action;
- do not ask the strategist to control combat;
- try again on the next material event or heartbeat.

Classify provider failures. Keep timeout, rate-limit, quota, authentication, provider availability, invalid output, and transport failures separate. Do not record a raw provider error message.

The request deadline defaults to five seconds. The runtime must not keep an unlimited queue of calls that exceed the tactical deadline.

Keep safe game-level automatic combat behavior as a temporary physical fallback. Do not remove it before live tactical behavior is proven.

## Tests

- [x] A material event starts one inference.
- [ ] A non-material update starts no inference.
- [x] Frequent frames create no unbounded call queue.
- [x] Actor health replies work during inference.
- [x] A newer frame makes the old result stale.
- [x] A new strategic revision makes the old result stale.
- [ ] An unrelated chat event does not make a combat result stale.
- [x] A parse failure causes no packet execution.
- [ ] An illegal proposal fails validation.
- [ ] A model timeout does not stop perception or the body.
- [x] Idle operation stays below the configured ceiling.
- [x] Combat heartbeat stays below the configured ceiling.

`NPC_IDLE_TACTICAL_HZ=0` disables quiet-room heartbeat inference. Material events still wake the tactician. Use this setting for bounded production shadow tests.

## Acceptance criteria

Phase 05 is complete when:

1. Recorded combat frames produce parseable tactical proposals.
2. The actor mailbox remains responsive during inference.
3. The runtime never executes an outdated result.
4. The model cannot call MCP.
5. The model cannot provide runtime metadata or identity.
6. Tactical requests remain small and stateless.
7. Backpressure and scheduling tests pass.

## Production shadow evidence

The `tactician/v2` production shadow run on 2026-08-11 used one material frame and no idle heartbeat. The request used 1,737 input tokens, 47 output tokens, and an exact OpenRouter charge of `$0.0001013364`. The model returned one parseable action in 1,293 milliseconds. The runtime recorded the proposal and released no packet.

Twenty-seven perception cycles completed while the runtime stayed connected. Only the first material frame started inference. Quiet perception updates did not queue more calls. The bounded run shut down and disconnected cleanly.

## Out of scope

Do not select the final tactical model or remove backend fallback behavior in this phase.

## Model evaluation evidence: 2026-08-12

The v4 tactical contract adds backend combat tactics. The prompt states these facts:

- `stop` stops movement only;
- `move_to` does not stop the backend combat reflex;
- `set_tactics` changes the backend reflex;
- the `flee` style runs and does not strike;
- the `manual` mode disables automatic combat.

Under the v4 prompt, `openai/gpt-oss-safeguard-20b` passed one of five low-health trials and exceeded the requested output-token maximum on every call. A later v6 evaluation improved to four of five repeated surrounded low-health trials, but one trial continued attacking without healing or fleeing. It remains a secondary candidate, not the provisional default.

`google/gemini-3.1-flash-lite` is the provisional default. It selected the expected behavior in these repeated combat cases:

| Case | Result |
| --- | --- |
| Surrounded, 38 percent health, two potions | Survival action in 5 of 5 |
| Critical, 15 percent health, no potion | Backend flee in 5 of 5 |
| Healthy, one enemy, legal skill | Legal skill in 5 of 5 |

The original three sets used 15 calls. The exact OpenRouter charge was `$0.0091042875`. Median latency was 1,259 ms. Maximum latency was 1,665 ms.

The first version of the probe did not apply the complete BodyActor validator. The probe now rejects an unknown target, item, skill, drop, or movement tile even when the high-level intent is good.

The v6 prompt and complete validator passed 25 of 25 repeated Gemini trials: five surrounded low-health, ten critical no-heal, five healthy single-enemy, and five safe-idle. The critical trials produced backend flee without an unnecessary movement destination. Gemini remains provisional until the required production soak is safe to run.
