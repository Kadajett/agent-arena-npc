# Phase 10: Replay Evaluation

State: In progress

## Goal

Build an offline tactical replay harness. Select tactical models from real Agent Arena cases.

## Required result

A developer can replay one recorded tactical situation against one or more models. The replay produces comparable quality, latency, token, and cost metrics.

## Dependencies

Phase 05 must define the tactical input and output contracts.

Phase 09 must define the event and episode record formats.

Production and controlled test failures must produce sanitized fixtures.

## Decision record

Record these values for every tactical decision:

- tactical frame;
- strategic intent;
- model ID;
- provider;
- prompt version;
- inference start and end time;
- raw model output where policy permits;
- parsed tactical proposal;
- parse result;
- validation result;
- enriched action packet;
- MCP calls;
- action outcomes;
- following game events;
- cost and token use.

Do not store secrets in a decision record.

## Fixture layout

Use this initial layout:

```text
rust-harness/fixtures/
├── combat/
│   ├── spider-respawn-loop.json
│   ├── surrounded-low-health.json
│   ├── kill-loot-retreat.json
│   ├── healer-with-potion.json
│   └── path-blocked-during-fight.json
├── movement/
│   ├── wall-stall.json
│   └── door-crossing.json
└── exploration/
    └── empty-room.json
```

Add a fixture for each important production failure.

## Replay command

Provide a command with this form:

```bash
cargo run --bin tactical-replay -- \
  --fixture fixtures/combat/spider-respawn-loop.json \
  --model google/gemini-3.1-flash-lite \
  --model openai/gpt-oss-safeguard-20b
```

Support a scripted brain mode that does not call a provider.

The initial command is available:

```bash
cargo run --bin tactical-replay -- \
  --fixture fixtures/combat/critical-no-heal.json \
  --model scripted \
  --model google/gemini-3.5-flash-lite
```

Each model produces one JSON record. A provider failure produces a record with
a stable error class and any accounted usage. It does not prevent later models
from running. The scripted mode calls neither OpenRouter nor MCP.

## Metrics

Measure:

- decision latency;
- parse success;
- illegal action rate;
- stale decision rate;
- unnecessary retreat;
- late retreat;
- death;
- damage taken;
- damage dealt;
- healing quality;
- target selection;
- loot recovered;
- movement recovery;
- movement stalls;
- objective compatibility;
- objective completion;
- input tokens;
- output tokens;
- provider cost.

Define each metric in code or fixture metadata. Do not use an undefined subjective score.

## Outcome evaluation

Use deterministic checks where the record permits them.

Use labeled expected ranges where one exact action is not required.

Example:

```text
Accept: heal then retreat west
Accept: retreat west without heal when health is stable
Reject: attack a missing target
Reject: move deeper into the blocked path
```

Do not require one model to reproduce one exact action sequence when several actions are valid.

## Prompt versioning

Keep prompts in versioned files.

Record the prompt version with every run.

Treat a prompt change as a behavior change. Run the corpus again after a prompt change.

## Model selection tasks

- [ ] Benchmark the initial candidate models.
- [ ] Test multiple provider routes when useful.
- [ ] Run each case enough times to measure variance.
- [ ] Compare latency, cost, and game outcome quality.
- [ ] Record parse and refusal failures.
- [ ] Record the selected default and reason.
- [x] Keep the model configurable after selection.

Do not select a model from a generic academic benchmark alone.

### Next candidate matrix (2026-08-12)

OpenRouter's live model catalog currently advertises structured output for the
following untested candidates. The catalog prices are screening facts only.
They are not observed run costs.

| Priority | Model | Reason to test | Catalog prompt/output USD per token |
| --- | --- | --- | --- |
| 1 | `liquid/lfm-2.5-2.6b:free` | Compact 2.6B reasoning model intended for extraction and agent workflows | `0` / `0` |
| 2 | `upstage/solar-pro4` | Agent-workflow model with a low reference price | `0.00000003` / `0.00000012` |
| 3 | `nex-agi/nex-n2-mini` | Small agentic MoE with structured output and tool support | `0.000000025` / `0.0000001` |
| 4 | `deepseek/deepseek-v4-flash-0731` | Pinned Flash release; do not use the moving `latest` alias for a measured run | `0.00000008` / `0.00000018` |
| 5 | `google/gemini-3.5-flash-lite` | New Flash Lite candidate to compare with the provisional 3.1 result | `0.0000003` / `0.0000025` |
| 6 | `tencent/hy3` | 21B-active MoE with reasoning controls; test precision and latency | `0.000000132` / `0.000000528` |

Source: `GET https://openrouter.ai/api/v1/models`, read on 2026-08-12.

Run every candidate through all four tactical probe scenarios and the complete
packet validator. Then run the replay fixtures. Reject a model that produces a
good-sounding intent with illegal coordinates, targets, items, or skills. Record
the exact response charge and finalized generation charge. Do not calculate the
winner from the catalog prices.

Candidate results from the funded-key run on 2026-08-12:

| Model | Result on `critical-no-heal` |
| --- | --- |
| `liquid/lfm-2.5-2.6b:free` | Rejected for the hot loop after two 5-second tactical timeouts. No response or charge was recorded. |
| `upstage/solar-pro4` | Responded in 3,504 ms. It selected disengage but emitted only two `stop` actions, so it failed the action semantics. Exact charge: `$0.00005094`. |
| `nex-agi/nex-n2-mini` | Timed out after about 5.1 seconds. No response charge was recorded. |
| `deepseek/deepseek-v4-flash-0731` | Timed out after about 5.2 seconds. No response charge was recorded. |
| `google/gemini-3.5-flash-lite` | Responded in 1,447 ms, but its structured output did not parse. It used 2,210 input and 167 output tokens. Exact charge: `$0.0010805`. |
| `tencent/hy3` | Timed out after about 5.1 seconds. No response charge was recorded. |
| `qwen/qwen3.7-plus` | Timed out after about 5.1 seconds. No response charge was recorded. |
| `stepfun/step-3.7-flash` | Timed out after about 5.1 seconds. No response charge was recorded. |
| `minimax/minimax-m3` | Responded in 2,705 ms. It selected disengage but emitted only `stop` actions, so it failed the action semantics. Exact charge: `$0.000492`. |
| `z-ai/glm-5.2` | Timed out after about 5.1 seconds. No response charge was recorded. |
| `meta/muse-glimmer-30b` | Timed out after about 5.1 seconds. No response charge was recorded. |
| `google/gemini-3.1-flash-lite` | Passed the full validator and scenario check in 1,088 ms. It remains the provisional default. |

The replay records timeout, rate-limit, quota, parse, semantic, and runtime
validation failures separately. A failed request with no provider response has
zero response calls, tokens, and cost. Do not count a quota or rate-limit
rejection as a model-quality failure.

### Tactical prompt v6 evidence (2026-08-12)

Prompt v5 fixed idle behavior, but one of five repeated critical-health trials
added a `move_to` operation for an explicitly blocked tile. Prompt v6 states
that backend flee mode is sufficient to disengage and permits `move_to` only
when the structured input explicitly lists the destination as reachable.

`google/gemini-3.1-flash-lite` then passed all 25 v6 trials:

| Scenario | Passes | Required result |
| --- | ---: | --- |
| Surrounded at 38 percent health with potions | 5 of 5 | Use a survival item or flee |
| Critical at 15 percent health without a potion | 10 of 10 | Set backend flee mode |
| Healthy against one enemy with a legal skill | 5 of 5 | Continue combat with a legal action |
| Safe and instructed to wait | 5 of 5 | Continue with no action |

The ten critical trials completed in 765 to 1,347 ms. All ten produced only
the sufficient `set_tactics(flee, semi_auto)` operation. A representative call
used 1,271 input tokens and 149 output tokens and cost exactly `$0.00054125`.

The user-suggested `openai/gpt-oss-safeguard-20b` passed one critical, one
healthy-combat, and one idle trial. It passed four of five repeated surrounded
low-health trials. The failed trial continued attacking without healing or
fleeing. One additional request received an upstream Groq rate-limit response
before inference. A representative successful call used 1,841 input tokens,
970 output tokens, and 1,792 cached input tokens and cost exactly
`$0.000361875`. The 970 reported output tokens exceeded the requested 150-token
maximum, and the harness emitted `model.usage_anomaly`.

The OpenRouter key is also used by other production processes. Its account-wide
balance changed by much more than these benchmark calls cost. Do not use key
balance deltas as harness cost. Use the exact per-generation charge, per-agent
ledger, model ID, provider, cached-token count, and cognitive role recorded by
the harness.

### Production v6 shadow (2026-08-12)

Cassian ran against the real production perception stream for 10 seconds in
`shadow` mode. Live mutation was disabled. Gemini completed two tactical calls.
Both returned `continue` with no actions. Both packet-release decisions recorded
`release_policy=record_only` and `released=false`. The perception pump started
21 cycles, all model accounting tasks reached a terminal state, and the MCP
session disconnected cleanly.

The calls used 5,587 input tokens and 197 output tokens. OpenRouter reported an
exact combined charge of `$0.00169225`. The terminal summary's direct projection
was `$14.62` per 24 connected hours. Do not use that startup-heavy ten-second
projection for planning. The second decision followed a later material frame
after more live state became available. A longer quiet soak must establish the
steady-state call rate.

## Production interaction soak

Offline replay cannot establish live interaction reliability. After the guarded
mutation path passes, run a dedicated production character with fixed model
IDs, prompt versions, provider-routing settings, frequencies, and action
limits. Record the complete configuration hash with the run.

Use three gates:

1. Run a 15-minute read-only or shadow shakedown.
2. Run a controlled two-hour interaction soak after the shakedown has no
   unexplained failure.
3. Run at least one continuous 24-hour cost soak after movement, speech, and
   combat safety gates pass.

Every soak must set both `NPC_RUN_DURATION_SECONDS` and
`NPC_RUN_MAX_OPENROUTER_COST_USD`. The duration bounds wall-clock exposure. The
cost value bounds completed OpenRouter charges. The cost gate stops if a
completed response lacks an exact charge, because the runtime cannot prove
that it remains below the ceiling in that case.

Preserve the terminal `runtime.run_summary` event with the run record. It gives
the role-specific and combined model totals, exact-cost coverage, action and
packet rates, movement outcomes, connected duration, and the simple 24-hour
cost projection. Use detailed causal events for per-model, per-provider, and
per-activity analysis. Do not infer those dimensions from the aggregate event.

Elapsed time alone is not a behavioral sample. Continue or repeat a run until
each reported interaction class has enough attempts. Report `insufficient
sample` instead of presenting a small count as a stable rate.

Track separate denominators and outcomes for:

- path checks, move starts, arrivals, stalls, recoveries, and scene changes;
- room speech, global speech, team speech, and private speech;
- speech accepted by MCP, speech observed in the world, and replies received;
- combat packets, legal attacks, accepted attacks, damage outcomes, kills,
  deaths, retreats, healing, skill use, and loot recovery;
- model parse, validation, stale-output, provider, MCP, and engine failures.

Report Wilson 95% confidence intervals for binary success rates. Target a
confidence-interval half-width of five percentage points for common operations
and ten percentage points for rare combat outcomes. Always show the numerator,
denominator, connected duration, and exposure time next to the interval.

Tool acceptance is not gameplay success. For example, report an accepted
attack separately from damage dealt, survival, and objective completion.
Report a sent chat line separately from an observed line and a meaningful
reply.

## Tests

- [x] Fixture schemas reject incomplete records.
- [x] Replay is deterministic with a scripted brain.
- [x] Replay never calls MCP mutation methods.
- [x] Replay can compare multiple models.
- [ ] Metrics use the same definitions for all models.
- [x] Prompt versions appear in results.
- [x] A failed provider call does not lose the fixture result.
- [x] Reports contain latency, token, and cost data.

## Acceptance criteria

Phase 10 is complete when:

1. The replay command runs the required fixture corpus.
2. The corpus includes combat, movement, and quiet-room cases.
3. Multiple tactical models have comparable reports.
4. The selected default has a written evidence-based reason.
5. Prompt changes can be compared with prior versions.
6. Replay records contain complete causal lineage.
7. Replay tests pass without a live game connection.
8. The production soak report contains sample counts and confidence intervals,
   or marks an interaction class as insufficiently sampled.

## Out of scope

Do not add automatic fine-tuning in this phase.
