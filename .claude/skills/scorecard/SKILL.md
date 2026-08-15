---
name: scorecard
description: Build a relationship scorecard for a character — how the town treats them, for viewers of the game. Use when asked how the town, another character, or "everyone" feels about a character, or for a character scorecard.
---

# Character scorecard

How the town treats one character, judged from what people actually said
to and around them, written for someone watching the game. Spec:
docs/character-scorecard.md.

## 1. Extract

```
node scripts/relations.mjs <claims-file.json> <CharacterName>
```

The claims file lives in the character's memory volume
(`<character>-claims.json`). Read it from the volume read-only; never
write to a memory volume.

## 2. Score each observer on three axes, 0 to 1

The score is the measurement; pick the noun(s) to fit the score at
report time. Tone descriptors ("playful", "gruff") are not axis values —
they may color the read, never the score.

- **Warmth** — a quick emotional temp check: 0 is absolute zero, 1 is
  surface of the sun. A warm line repeated verbatim toward several
  characters is habit, not feeling, and dampens the score.
- **Engagement** — frequency of direct engagement with the character.
  Show the utterance count beside the score.
- **Trust** — judged from unprompted sharing only: volunteering
  information, plans, or feelings. Being told things is the signal.
- **Salience** — behind-their-back mentions over a rolling three-day
  window: `node scripts/salience.mjs <Name>`. The output states its
  presence heuristic; repeat it in the report.

## 3. Report

One table row per observer — warmth / engagement / trust / salience /
a one-line read — every score and read backed by a quotable line, then
the insights that do not fit a cell. Thinness is per axis, flagged on
the cell: warmth and trust are thin under five directed utterances;
engagement never is, because the count is the measurement; the read
inherits the thinnest input. Cells stay short; prose lives below the
table. The salience cell shows words only, backfilled from the
behind-back rate — never a number — and reads N/A when the sample is
too thin to characterize.

Voice rules: written for a viewer. No harness or operator internals —
goals, prompts, model behavior — in the output. Quotes verbatim; a
paraphrase is never shown as a quote. State the blind spot: this is
conduct toward the character, not private opinion of them.
