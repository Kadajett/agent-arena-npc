# Model Input, Cache, and Timeout Audit: 2026-08-12

## Purpose

This audit must show the exact logical input that the harness gives to each model.

The audit must also explain model timeouts and zero cache use. These conditions can cause bad character behavior. They are not only cost problems.

## Strict JSON live check

The tactician now uses prompt `tactician/v12` and input protocol version 1.

The production check on 2026-08-12 produced this sequence:

1. Runtime restored the saved navigation mission to scene `reldens-house-1-2d-floor`, tile `(17,13)`.
2. Body sent one destination-level `move_to` operation.
3. Body reported arrival after one attempt.
4. Perception emitted `strategic.navigation_arrival_observed` with the mission and decision IDs.
5. Strategist received a typed `navigation_arrived` moment.
6. Two Gemini tactical decisions returned `continue`.
7. No tactical packet paused or replaced the strategic movement mission.

The captured tactical input contained:

- `protocol_version: 1`;
- `movement_control.owner: body_strategic_navigation`;
- the exact destination scene and tile;
- four allowed preemption fact types;
- an empty `tactical_preemption_facts_present` list.

The live check also found one `openai/gpt-oss-120b` call that reached the configured 60-second timeout. A later call failed with a provider transport or response error after approximately one second. The actor retained the prior strategic intent and used bounded retry backoff.

## Safety rules

- Exact input capture must be disabled by default.
- Exact input capture must write only to an ignored local directory.
- The capture directory must use mode `0700` on Unix.
- Each capture file must use mode `0600` on Unix.
- A capture must not contain an application programming interface key.
- Normal analytics must not contain prompt text, memory text, persona text, or model output text.
- Normal analytics can contain sizes, counts, versions, fingerprints, and provider accounting facts.

## Required logical input capture

Capture these fields before each provider request:

- cognitive role;
- requested model;
- prompt version;
- stable preamble;
- bounded conversation history in message order;
- current typed input after serialization;
- structured output schema;
- temperature;
- maximum output tokens;
- reasoning policy;
- provider parameters;
- character-scoped role session identifier;
- decision and revision identifiers.

The capture is the exact logical Rig request. If Rig or the provider changes the wire request, record that limitation. Do not call the logical capture an exact wire capture.

## Required safe analytics

Emit these facts for each assembled input:

- deterministic request fingerprint;
- total logical request bytes;
- estimated total tokens;
- preamble bytes;
- bounded history message count and bytes;
- current typed input bytes;
- output schema bytes;
- prompt version;
- requested model;
- reasoning policy;
- input capture state;
- cache-control state;
- session-stickiness state;
- estimated stable-prefix bytes and tokens.

The final provider accounting must include:

- actual model;
- actual provider;
- prompt tokens;
- completion tokens;
- reasoning tokens;
- cached prompt tokens;
- cache-write tokens when available;
- exact cost;
- cache discount;
- provider latency;
- generation time;
- finish reason when available.

## Cache investigation

Compare at least three consecutive tactical inputs and three consecutive strategic inputs.

For each role:

1. Find the longest identical prefix.
2. Find the first changing field.
3. Confirm message order.
4. Confirm that stable content is before dynamic content.
5. Confirm that the same role uses a stable OpenRouter session identifier.
6. Confirm whether the selected model and provider support implicit or explicit prompt caching.
7. Confirm the provider minimum cacheable prefix.
8. Confirm whether the request contains the required cache-control marker.
9. Compare immediate usage with the OpenRouter generation record.

Do not add filler text to reach a provider cache minimum. A cache must contain useful stable context.

## Timeout investigation

Record these facts for each model failure:

- cognitive role;
- requested model;
- actual provider when known;
- request start time;
- elapsed time;
- configured deadline;
- input size;
- history size;
- last successful plan age;
- consecutive model failures;
- retry and backoff state;
- whether a newer input superseded the request;
- whether the previous strategic intent remained active.

Report latency percentiles and the timeout rate by model and provider.

## Initial evidence

The local Guy run contained nine Nemotron strategist requests with a terminal result:

- Six requests succeeded.
- Three requests reached the exact 120-second deadline.
- The observed timeout rate was 33 percent.
- Successful latency ranged from 13.1 seconds to 115.5 seconds.
- Input grew from 7,231 tokens to 12,763 tokens.
- Several responses exceeded the requested output limit.

The run also showed these cache facts:

- Nemotron strategist requests used 4,608 to 10,368 cached tokens.
- Gemini tactician requests used zero cached tokens.
- The Gemini route changed between Google and Google AI Studio.
- The tactical request had no explicit Gemini cache-control breakpoint.

These facts can explain delayed or stale plans. They can also explain inconsistent tactical responses and unnecessary prompt cost.

## Acceptance criteria

- A local opt-in run produces private logical input captures.
- The normal log contains no sensitive input text.
- The pretty log shows model input metadata, native token accounting, reasoning tokens, cache use, and timeout deadlines.
- The audit identifies the first changing field for tactical and strategic requests.
- A live production navigation run confirms that the new planner and body use the captured inputs.
- The report separates poor model output from missing output, stale output, and timed-out output.
