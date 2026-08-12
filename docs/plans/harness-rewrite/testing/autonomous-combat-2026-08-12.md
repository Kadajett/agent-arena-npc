# Autonomous Combat Test: 2026-08-12

## Status

The first autonomous production run failed its safety goal.

Do not use this run to estimate daily production cost. The run ended after approximately 111 seconds. It did not reach the planned ten-minute duration.

## Test configuration

- Character: `Cassian Vey Unbound`
- Character sheet: `cassian`
- Scene: northern forest combat area
- Tactical model: `meta-llama/llama-3.2-3b-instruct`
- Strategic model: `openai/gpt-oss-20b`
- Tactical prompt: `tactician/v3`
- Strategic prompt: `strategist/v2`
- Production process run ID: `910f4d26-405e-4070-996e-8e5f6a5e5b0d`
- Planned duration: ten minutes
- Actual useful duration: approximately 111 seconds

The test used a dedicated character. The test key did not enter the repository.

## Observed result

Cassian entered combat and fought. He did not disengage when his health became low. He died before the planned run ended.

The trace does not prove that the model ignored known health. The trace proves that the model did not receive health:

- 180 perception frames were published.
- 117 frames reported active combat.
- Zero combat frames contained known health.
- Zero combat frames contained known maximum health.

The backend also continued to report active battle data after death and a scene transition. This backend defect is [Agent Arena issue 31](https://github.com/Kadajett/agentArena/issues/31).

## Runtime findings

The run found these harness defects:

1. The ActionPacket did not include `arena_set_tactics`.
2. `Stop` stopped movement only. It did not stop the backend combat reflex.
3. A local move did not disable the backend combat reflex.
4. The scheduler discarded too many tactical responses because each newer perception revision made the previous response stale.
5. The production `distanceFromSelf` value used pixels. The tactical contract treated it as tile distance.
6. A successful attack command was reported as confirmed success. The MCP result confirmed command acceptance only.
7. Repeated inventory calls contributed to MCP rate limits.
8. A safety shutdown could disconnect while the backend combat reflex remained active.

## Corrections

The harness now has these corrections:

- The tactical action schema includes typed `set_tactics` style and mode fields.
- The v4 prompt explains `stop`, `move_to`, `manual`, and `flee` semantics.
- A disengage proposal must use backend flee mode or a local move. `Stop` is not a disengage action.
- The BodyActor validates and executes `set_tactics` through the character-bound gateway.
- A full live run stops if active combat has unknown health.
- Before a safety-triggered disconnect, the BodyActor changes the backend reflex to `flee` and `semi_auto`.
- The safety fallback has decision, packet, action, frame, strategy, duration, status, and reason fields.
- The scheduler uses material invalidation. A newer observation number alone does not discard a tactical response.
- Perception calculates tile distance from exact coordinates.
- Attack and skill commands use the `accepted` terminal state until a following frame confirms an effect.
- Dedicated inventory reads occur on the first cycle and then at a configured cadence.
- Tactical and strategic model requests have deadlines.
- Provider failures identify timeouts, rate limits, quota exhaustion, HTTP status, and provider error code without recording the provider message.

## Cost and model facts

The failed production run used:

| Role | Calls started or completed | Input tokens | Output tokens | Cached input tokens | Exact charge |
| --- | ---: | ---: | ---: | ---: | ---: |
| Tactician | 35 started; 2 current results | 172,224 | 1,448 | 8,256 | `$0.0089981496` |
| Strategist | 1 completed | 1,165 | 391 | 0 | `$0.0001041975` |
| Total | Not applicable | Not applicable | Not applicable | Not applicable | `$0.0091023471` |

The tactical latency median was 801.5 ms. The p95 was 1,548 ms. The maximum was 2,129 ms. The strategic call took 8,556 ms.

A direct projection gives approximately `$0.2946` per hour and `$7.07` per day. Do not use this projection for planning. The short failed run had abnormal combat frequency, excessive stale tactical calls, MCP rate limits, and no successful idle period.

## Tactical model search

The test harness uses repeated structured-output trials. A model fails the survival case if it only stops movement, continues combat at critical health, invents an item, or produces invalid structure.

| Model | Main result |
| --- | --- |
| `openai/gpt-oss-safeguard-20b` | Completed 5 of 5 low-health trials. Passed 1. Returned 1,323 to 2,283 output tokens despite a 150-token request. Exact charge was `$0.00280598175`. |
| `meta-llama/llama-3.2-3b-instruct` | Produced short output, but did not choose a real escape in repeated v4 trials. |
| `qwen/qwen3.5-9b` | Timed out in 5 of 5 trials. |
| `mistralai/ministral-3b-2512` | Produced three truncated outputs. Two parsed outputs stopped movement only. |
| `ibm-granite/granite-4.1-8b` | Completed 5 of 5 trials. It did not choose a real escape. |
| `nvidia/nemotron-3.5-lightning` | Timed out twice. Three parsed outputs did not choose a real escape. |
| `openai/gpt-5.6-luna` | Timed out four times. One parsed output did not choose a real escape. |
| `qwen/qwen3.5-35b-a3b` | Timed out in 5 of 5 trials. |
| `bytedance-seed/seed-2.0-mini` | Timed out in 5 of 5 trials. |
| `mistralai/mistral-small-2603` | Passed 4 of 5 first low-health trials. It failed the critical no-heal case 5 of 5 times. |
| `google/gemini-3.1-flash-lite` | Selected a survival action in 5 of 5 surrounded low-health trials. It selected backend flee in 5 of 5 critical no-heal trials. It selected the legal skill in 5 of 5 healthy single-enemy trials. |

The 15-call Gemini combat set used 23,729 input tokens and 2,176 output tokens. OpenRouter reported an exact charge of `$0.0091042875`. Median latency was 1,259 ms. Maximum latency was 1,665 ms.

The first benchmark version did not run the complete proposal through the BodyActor validator. This matters for the critical case because Gemini paired backend flee with local movement. The benchmark now performs full runtime validation. Do not claim that the original five critical packets were executable. Their raw coordinates were intentionally not persisted.

The first idle Gemini test did not run because that OpenRouter key reached its configured total limit. A later funded-key run completed the missing tests; see the v6 correction below.

### v6 correction and expanded sample

The complete runtime validator found one blocked destination in five repeated
v5 critical-health calls, even though every call correctly selected backend
flee mode. Prompt v6 made flee mode sufficient and prohibited local movement
unless the structured input explicitly lists the exact destination as
reachable.

Gemini 3.1 Flash Lite passed 25 of 25 v6 probes: 5 surrounded low-health, 10
critical no-heal, 5 healthy single-enemy, and 5 safe-idle. Critical decision
latency ranged from 765 to 1,347 ms. All ten critical proposals contained only
`set_tactics(flee, semi_auto)`.

GPT-OSS Safeguard passed four of five repeated surrounded low-health probes. Its
failure continued attacking without a survival action. A representative
successful Gemini call cost `$0.00054125`. A representative successful
Safeguard call cost `$0.000361875`, but it reported 970 output tokens despite a
150-token request. Exact per-generation telemetry is authoritative because the
funded key is shared with other production processes.

### Production v6 shadow

A ten-second production shadow gave Cassian the real live tactical frame while
mutation remained disabled. Gemini returned `continue` with no actions on both
decisions. The release gate recorded both proposals and released neither. The
run completed 21 perception cycles, drained every accounting task, and
disconnected cleanly.

The two calls used 5,587 input tokens and 197 output tokens and cost exactly
`$0.00169225`. The direct 24-hour projection was `$14.62`, but a ten-second run
is dominated by startup and state hydration. It is evidence that the accounting
path works, not a daily budget estimate.

## Production correction checks

After the fixes, the harness performed two controlled production checks in `reldens-house-1`:

1. The BodyActor set Cassian's tactics to `flee` and `semi_auto`. Production accepted the action in 149 ms.
2. The BodyActor moved Cassian from tile `(17,19)` to `(17,18)`. A perception frame confirmed arrival after 818 ms.

Both runs used exact character, player-name, scene, packet-size, and action-budget gates. Both runs disconnected cleanly. A concurrent model call failed because of quota exhaustion. That failure did not stop the body action.

## Next production gate

Do not start another autonomous forest combat run until all these conditions are true:

1. The backend supplies authoritative health and maximum health in combat.
2. Issue 31 is fixed and verified in production.
3. [Satisfied] The idle model fixture and the full-validator combat fixtures pass.
4. The runtime safety fallback test passes against production.
5. The run has a fixed duration, action limit, and cost limit.
6. The run report separates active combat, travel, social activity, and idle time.
7. Cassian starts the run in backend flee mode and changes mode only through a validated tactical packet.

The next cost report must use exact response charges. It must report cost per wall-clock hour, cost per active-combat minute, calls per activity class, and a confidence interval from a sufficiently long successful run.
