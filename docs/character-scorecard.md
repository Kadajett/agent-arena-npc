# Character scorecard

Retroactive spec for `scripts/relations.mjs` and the `scorecard` skill.
Audience: people who watch the game.

**What it does now (before this component)**
How the town treats a character exists only as scattered chat lines and
per-character memory files. A viewer wondering "how does everyone feel
about Bolo" has to scroll the raw feed and guess.

**What we want it to do instead**
As someone watching the game, I can pull up any character and see how the
town treats them — who is warm to them, who confides in them, who talks
about them when they are not in the room — with the real quotes behind
every claim.

- AC1: `node scripts/relations.mjs <claims-file> <Name>` prints JSON:
  `{ target, heardTotal, observers: [{ speaker, utterances,
  mentionsOfTarget, lines: [{ at, room, claim }] }] }`, observers sorted
  by utterances descending.
- AC2: the scorecard reads as one table row per observer — warmth,
  engagement, confides, salience, a one-line read — every judgment
  backed by a quotable line.
- AC3: a judgment resting on fewer than five utterances is flagged thin.
- AC4 *(pending — needs the public chat-feed walk)*: salience counts
  mentions of the character in rooms and moments they were not present
  for.

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
