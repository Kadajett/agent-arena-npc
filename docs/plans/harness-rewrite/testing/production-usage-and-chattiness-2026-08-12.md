# Production Usage and Chattiness: 2026-08-12

## Scope

This report measures the first production hour after the Rust cutover and the
first paid model calls after the route change. The source is structured harness
analytics from the three production containers. No chat text or model output is
stored in this report.

The free-route sample ran for about 59 minutes, from 10:48:55 UTC to 11:47:57
UTC. The paid route started at about 12:01 UTC.

## First-hour token use

| Character | Model responses | Input tokens | Output tokens | Cached input tokens | Total tokens | Exact charge |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Guy | 122 | 529,754 | 14,638 | 0 | 544,392 | $0 |
| Barnaby | 48 | 449,039 | 45,251 | 8,448 | 494,290 | $0 |
| Cassian | 161 | 864,097 | 22,621 | 0 | 886,718 | $0 |
| **Fleet** | **331** | **1,842,890** | **82,510** | **8,448** | **1,925,400** | **$0** |

The exact charge was zero because the deployed model identifiers selected free
routes. The account still recorded all token and cache values.

Barnaby used far more output tokens per response than the other agents. His
strategist averaged 943 output tokens. The provider reported more output than
the requested 600-token maximum on 43 of 48 responses. The analytics records
this as `reported_output_exceeds_requested_max`.

Guy and Cassian used about 98 to 110 output tokens for each tactical response.
Their strategists used about 236 to 237 output tokens per response.

## Chattiness

| Character | Outgoing room messages | Approximate messages per hour | Characters sent | Incoming scene messages observed |
| --- | ---: | ---: | ---: | ---: |
| Guy | 0 | 0 | 0 | 7 |
| Barnaby | 19 | 19.3 | Not retained in this extracted snapshot | 421 |
| Cassian | 0 | 0 | 0 | 171 |

The outgoing count uses completed MCP chat operations. It does not count model
responses as speech. Barnaby was the only agent that spoke during this sample.
His 19 room messages all completed successfully. The harness records the
character count and channel for each requested message without recording the
message text.

The incoming counts use the perception reducer's new-message fields. They do
not represent messages written by the character.

## Reliability and behavior

### Model calls

The free tactical route was not reliable enough for production reflexes.

| Character and role | Started | Completed | Failed | Main failures |
| --- | ---: | ---: | ---: | --- |
| Guy tactician | 164 | 111 | 53 | 44 rate limits, 8 timeouts, 1 parse failure |
| Guy strategist | 10 | 10 | 0 | None |
| Barnaby strategist | 49 | 48 | 0 | One call was still in flight at the snapshot |
| Cassian tactician | 164 | 109 | 54 | 39 rate limits, 12 timeouts, 3 parse failures |
| Cassian strategist | 50 | 49 | 0 | One call was still in flight at the snapshot |

Free-route tactical latency averaged 12.8 seconds for Guy and 11.7 seconds for
Cassian. This does not meet the harness target.

### Body execution

- Guy completed 68 of 68 body actions.
- Barnaby completed 19 of 19 speech actions.
- Cassian completed 91 of 104 body actions. Eleven local moves were
  unreachable. Two door calls failed at the MCP or HTTP boundary.
- The sample contained no attack action. It cannot provide a live combat
  success rate.

Execution was reliable, but purposeful navigation was poor. Guy crossed
between town and the bot forest 34 times. Cassian crossed between the inn and
town more than 90 times. Faster inference does not correct this loop by itself.

## Paid tactical comparison

The paid corpus used three trials in five scenarios. Gemini 3.1 Flash Lite was
the clear winner.

| Scenario | Gemini passes | Result |
| --- | ---: | --- |
| Surrounded with low health | 3/3 | Used the healing item |
| Critical health without healing | 3/3 | Selected backend flee tactics |
| Healthy single enemy | 3/3 | Selected the legal combat skill |
| Safe idle | 3/3 | Remained idle |
| Exploration | 3/3 factual selections | Selected the offered door |

The original exploration verdict incorrectly omitted the generic `doors`
capability. The runtime decision was legal for Guy and Cassian. The probe
fixture now includes that capability.

Paid Gemini trials usually completed in 0.8 to 1.6 seconds. Paid safeguard was
less consistent in danger. Base GPT-OSS and Gemma frequently emitted invalid
action semantics or made unsafe choices.

## Paid production sample

The production services now use these paid routes:

| Character | Tactician | Strategist |
| --- | --- | --- |
| Guy | `google/gemini-3.1-flash-lite` | `nvidia/nemotron-3-super-120b-a12b` |
| Barnaby | Disabled while idle | `nvidia/nemotron-3-nano-30b-a3b` |
| Cassian | `google/gemini-3.1-flash-lite` | `nvidia/nemotron-3-super-120b-a12b` |

The first six paid tactical calls cost between $0.000768 and $0.001202 each.
Their latency ranged from 0.96 to 2.00 seconds. All six parsed and produced a
successful body action.

Barnaby's first paid strategic call cost $0.000573 and took 4.17 seconds.
Cassian's first paid strategic call cost $0.001602 and took 41.37 seconds.
Guy's first paid strategic request received a rate limit. His tactical actor
continued to run.

## Daily price estimate

The point estimate uses the first-hour attempted call frequency and the first
paid production charges.

| Character | Estimated daily cost | Main source |
| --- | ---: | --- |
| Guy | About $4.30 | About $3.92 tactics and $0.38 strategy |
| Barnaby | About $0.67 | Strategy only while tactical idle is disabled |
| Cassian | About $5.91 | About $3.99 tactics and $1.92 strategy |
| **Fleet** | **About $10.9 per day** | Paid inference for all three agents |

Use $8 to $12 per day as the initial operating range. The lower end assumes the
current successful-call throughput. The upper end assumes that most attempted
tactical calls complete and includes normal variation. It does not assume a
combat burst beyond the first-hour rate.

A 4.5-minute live paid sample independently produced a $10.46/day projection:

| Character | Exact sample charge | Sample-based daily projection |
| --- | ---: | ---: |
| Guy | $0.013740 | $4.43 |
| Barnaby | $0.003244 | $1.05 |
| Cassian | $0.015447 | $4.98 |
| **Fleet** | **$0.032431** | **$10.46** |

That short sample included 31 successful body actions, no body-action failure,
no actor failure, and no perception failure. Use the 24-hour report for budget
decisions because a short window is sensitive to strategic-call timing.

The funded key had about $14.96 remaining before the paid probes and route
change. It had $14.92 remaining after the probes and initial production calls.
At the measured point estimate, that key limit supports only about 1.4 days.
The account may have more credit, but the key limit must also remain above the
expected daily spend.

## Next measurement

The next report should use at least 24 hours of paid telemetry. It must separate
idle, travel, social, and combat time. It should also report unique scene
revisits, loop rate, attacks, deaths, retreat timing, chat response rate, and
cost per useful body outcome.
