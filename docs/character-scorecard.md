# Character scorecard

Retroactive spec for `scripts/relations.mjs` and the `scorecard` skill.
Audience: people who watch the game.

**What it does now (before this component)**
How the town treats a character exists only as scattered chat lines and
per-character memory files. A viewer wondering "how does everyone feel
about Bolo" has to scroll the raw feed and guess.

**What we want it to do instead**

Every column except the read is an axis scored 0 to 1. The score is the
measurement; the words are chosen at invocation to fit the score. Tone
descriptors ("playful", "gruff") are not axis values — they may color the
read, never the score.

- **Warmth** — As a viewer, I get a quick emotional temp check on how
  each character feels toward my character: absolute zero to surface of
  the sun.
  - AC1: each observer carries a warmth score in [0, 1]; the component
    backfills the fitting noun(s) from the score.
  - AC2: a warm line repeated verbatim toward several characters is
    habit, not feeling, and dampens the score.
- **Engagement** — As a viewer, I can see how often each character
  directly engages my character, so I know who seeks them out and who
  brushes them off.
  - AC3: each observer carries an engagement score in [0, 1] driven by
    frequency of direct engagement, with the utterance count shown
    beside it.
- **Trust** — As a viewer, I can see how much each character trusts my
  character, so I can watch trust build or break.
  - AC4: each observer carries a trust score in [0, 1], judged from
    unprompted sharing only; nouns and phrases backfill against the
    score.
- **Salience** — As a viewer, I can see how other characters speak about
  my agent when they aren't around, so I can better understand their
  true feelings toward my character.
  - AC5: `node scripts/salience.mjs <Name> [days=3]` walks the public
    chat feed over a rolling three-day window and reports mentions of my
    character sorted into in-presence and behind-their-back, with the
    presence heuristic stated in the output.
- **The read** — As a viewer, I get a one-line read on every pair and can
  reach the actual quotes behind it, so nothing asks me to take the
  scorecard's word for anything.
  - AC6: every score and every read is backed by a quotable line.
  - AC7: a judgment resting on fewer than five utterances is flagged
    thin.
- **The data** — As someone building views for watchers, the raw
  material is one predictable shape.
  - AC8: `node scripts/relations.mjs <claims-file> <Name>` prints JSON:
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
- Should scorecards for player characters (no claims file available)
  build from the chat feed alone? (salience.mjs already works from the
  feed alone; the other axes still need a claims file.)
