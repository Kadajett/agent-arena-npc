# OpenRouter Accounting Smoke Test: 2026-08-11

## Scope

This test used the configured OpenRouter key without printing or storing it. It called both proposed fast-model families through Rig. It also called OpenRouter's provider endpoint and generation accounting APIs through the Rust accounting adapter.

## Models

| Model | Structured proposal | Response accounting | Final generation accounting |
| --- | --- | --- | --- |
| `meta-llama/llama-3.2-3b-instruct` | Pass | Pass | Pass |
| `nvidia/nemotron-3-nano-30b-a3b` | Pass | Pass | Pass |

## Verified facts

- The Llama provider lookup returned two current provider endpoints.
- The Nemotron provider lookup returned three current provider endpoints.
- Each endpoint event preserved the exact decimal price text from OpenRouter.
- Each endpoint event also supplied numeric values for analytics queries.
- The Rig completion event recorded input, output, total, cached-input, cache-write, tool-prompt, and reasoning token fields.
- The Rig completion event recorded the exact charge returned in OpenRouter's response.
- The completion event used Cassian's stable character ID and the `tactician` role.
- The finalized generation lookup recorded the actual upstream provider, native token counts, cache count, provider latency, generation time, and exact finalized charge.
- The process-local ledger accumulated token and cost totals under Cassian's character ID.

The Nemotron run shows why the finalized audit is useful. Rig's generic OpenRouter conversion reported zero reasoning tokens. The later OpenRouter generation record reported 470 native reasoning tokens. The harness keeps both facts with their sources. It does not replace one field silently.

## Accounting rule

The exact response charge and the finalized generation charge are billable usage facts. A provider endpoint price is a time-stamped reference rate. The harness does not estimate a completed call from the current reference rate when OpenRouter supplied an exact charge.

The generation record can become available several seconds after completion. The production audit retries in a background task. It does not block the tactical model call.

The runtime now tracks those background tasks and the active model calls that
can create them. A short diagnostic drains them with a deadline and reports
completed, failed, explicitly aborted, and still-active counts. It continues
accepting accounting tasks registered by a response during shutdown. It no
longer takes one task snapshot or uses a fixed sleep and assumes the final audit
finished.

A production shutdown reproduction started the drain while the strategist was
still waiting on OpenRouter. The response arrived about 17 seconds after the
perception pump stopped. The runtime then completed the strategist generation
audit and disconnected. The drain reported four completed tasks, zero failed,
zero aborted, and zero active model calls remaining.

## Compact tactical model comparison

The `tactician/v2` fixture comparison used the same surrounded, low-health combat facts for both candidate families.

| Model | Latency | Input tokens | Reported output tokens | Exact response charge | Parsed intent |
| --- | ---: | ---: | ---: | ---: | --- |
| Llama 3.2 3B Instruct | 921 ms | 875 | 48 | `$0.0000589941` | stop |
| Nemotron 3 Nano 30B A3B | 4,314 ms | 1,018 | 1,122 | `$0.000272547` | attack |

Both responses parsed into one legal output shape. This single fixture does not establish behavioral quality. It does show that Nemotron was about 4.7 times slower and 4.6 times more expensive for this call. It is not the preferred hot-loop candidate from this sample.

The Nemotron response reported 1,122 output tokens even though the request set a 150-token maximum. The harness now records the requested maximum on response events and emits `model.usage_anomaly` when the provider-reported count exceeds it. This is an observed provider/accounting fact. The harness does not assume why the counts differ.
