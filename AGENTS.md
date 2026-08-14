# agent-arena-npc

NPC agents that live in Agent Arena over MCP. The pi harness lives in `scripts/pi-npc.sh`; per-character settings in `characters/*.conf`; pi extensions (todo, memory, autonomy, voice) in `.pi/extensions/`.

## Writing issues

An initial ticket **describes a problem**. It does not propose a fix, plan the work, or argue for its own importance. Prescribe a solution only when the issue's owner asks for one on that ticket.

Run these gates in order. Do not skip ahead.

**1. Search before writing.** `gh issue list --repo <repo> --state all --limit 100`, then read the near neighbours in full. State in the ticket whether this is a duplicate (comment on the existing issue instead), which group it joins, and what blocks it. Groups already in use: `economy-surface`, `movement-result-shape`, `attack-result-contract`, `content-audit`, `payload-size`, `npc-context`. Join one rather than inventing a name.

**2. Two proofs and an adversarial check.** Delegate to subagents running the `diagnosing-bugs` discipline from [mattpocock/skills](https://github.com/mattpocock/skills) (`skills/engineering/diagnosing-bugs`), one agent per issue. Each returns two *independent* proofs that were measured, not reasoned: a command actually run with its real output, a DB query, a replayed payload. Reading code is the weakest form and never counts as both. Each must also try hard to **refute** the claim and name every sub-claim that did not survive. Numbers produced by estimating rather than measuring do not go in a ticket.

Have each agent **write its report to a file** and reply with the path. With
`CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS` and `teammateMode: "tmux"`, giving an
agent a name makes it an interactive teammate: it prints its answer into a tmux
pane, sends only an idle notification, and its work never returns as a tool
result. The pane keeps about 12 lines of scrollback, so the report is then lost.
An agent that has done excellent work is indistinguishable from one that hung.

When that happens, do not verify the claim yourself. Recover the report.

**3. Write it in Simplified Technical English (ASD-STE100).** One idea per sentence, active voice, present tense, no metaphor, no second word for a term already used. Format:

```
Group: <name> (#A + #B). <one line: the shared shape, what to fix first>

Problem: <2-4 sentences. The defect only.>

Cause: <2-3 sentences. The mechanism.>

Evidence:
- path/file.js:LINE - what this line shows
- <command or query> - what the output shows

Correction to the field report: <every sub-claim that failed gate 2, including your own>
```

Omit `Group:` when nothing relates. Omit `Cause:` when gate 2 did not establish one, because an unproven cause is worse than none. Never omit `Correction:` when a claim failed.

**4. Cut by half.** Remove 50% of the draft's length without losing a fact. This gate is expected to fail on the first pass. Delete on sight: "Why this matters", acceptance-criteria checklists, projected costs, option tables, background the repo already knows, and any sentence arguing the issue deserves attention. Evidence lines and corrections are never what gets cut.

**5. Title the defect, not the remedy.** "Item stats are never sent to agents", not "Add item stats to inventory". Prefix with the area word when one fits: `Combat:`, `Movement:`, `Items:`, `Perception:`, `Content:`, `Gateway:`.

### Why the adversarial gate is not optional

Issue #112 refuted its own field report ("the world caps at L18") after measurement.

#127 is the better lesson, because it took three rounds. It first prescribed TOON encoding. A self-review then "refuted" that on a 20-payload sample, reporting TOON as 11% larger than compact JSON. A subagent measuring 742 payloads found that number inverted: TOON is 13.9% smaller by characters corpus-wide, and eligibility is 33.4%, not 17.8%. The original conclusion survived, but for a reason neither earlier pass had — TOON buys only 4.3 points over simply dropping the pretty-printer. The self-review was wrong in the same confident register as the thing it corrected.

agent-arena-npc#4 claimed compaction "never fires" when the session logs hold 18 compaction records.

The rule this produces: **the author of a claim does not verify it.** Delegate gate 2 and accept the result even when it contradicts you. Self-verification reliably finds the evidence that agrees.

### Repo facts worth not rediscovering

- The DB container has no `mysql` binary: `docker exec deploy-db-1 sh -lc 'mariadb -uroot -p"$MARIADB_ROOT_PASSWORD" reldens -e "..."'`
- Gateway source is `services/mcp-gateway/`; the world bridge is `src/reldens/headless-client.js`, where most evidence lines land.
- `agentArena` is private. `agent-arena-npc` is public: keep keys, agent ids, and host addresses out of its issues.

### Model for gate 2

Run gate 2's subagents on your provider's **top reasoning tier**, not its cheap
or fast tier. A weaker model agrees with a plausible claim instead of breaking
it, which defeats the gate. The tier matters, not the brand:

| provider | use |
|---|---|
| Anthropic | Claude Opus (top tier), not Haiku |
| OpenAI | the flagship reasoning model, not a mini/nano variant |
| Google | Gemini Pro tier, not Flash |
| open weights | a frontier-scale model; small local models fail this gate |

If only a cheap tier is available, say so in the ticket rather than presenting
an unverified claim as measured.
