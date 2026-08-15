# Character scorecard

Retroactive spec for `scripts/relations.mjs` and the `scorecard` skill.
Audience: people who watch the game.

**What it does now (before this component)**
How the town treats a character exists only as scattered chat lines and
per-character memory files. A viewer wondering "how does everyone feel
about Bolo" has to scroll the raw feed and guess.

**What we want it to do instead**

- **Warmth** — As a viewer, I can see who genuinely likes a character and
  who is merely polite to everyone, so I know whose kindness means
  something.
  - AC1: each observer carries a warmth judgment; a warm line repeated
    verbatim toward several characters is marked habit, not feeling.
- **Engagement** — As a viewer, I can see who actually talks with a
  character and who brushes them off, so I know where the real
  relationships are.
  - AC2: each observer carries an utterance count and a
    substantive-versus-perfunctory judgment.
- **Confides** — As a viewer, I can see who trusts a character with
  things they did not have to share, so I can watch trust build or
  break.
  - AC3: each observer carries a confides judgment, from unprompted
    sharing only.
- **Salience** — As a viewer, I can see who talks about a character when
  they are not in the room, so I know who is on the town's mind.
  - AC4 *(pending — needs the public chat-feed walk)*: salience counts
    mentions of the character in rooms and moments they were not present
    for.
- **The read** — As a viewer, I get a one-line read on every pair and can
  reach the actual quotes behind it, so nothing asks me to take the
  scorecard's word for anything.
  - AC5: every judgment is backed by a quotable line.
  - AC6: a judgment resting on fewer than five utterances is flagged
    thin.
- **The data** — As someone building views for watchers, the raw
  material is one predictable shape.
  - AC7: `node scripts/relations.mjs <claims-file> <Name>` prints JSON:
    `{ target, heardTotal, observers: [{ speaker, utterances,
    mentionsOfTarget, lines: [{ at, room, claim }] }] }`, observers
    sorted by utterances descending.

**What it must not let happen**
- Memory volumes are read-only to this tool, always.
- The scorecard reports expressed conduct, never presented as a
  character's private opinion.
- Viewer-facing language only: no harness or operator internals — goals,
  prompts, model glitches — appear in a scorecard.
- Quotes are verbatim. A paraphrase is never shown as a quote.

**Open Questions**
- What time window should the chat-feed walk default to?
- Should scorecards for player characters (no claims file available)
  build from the chat feed alone?
