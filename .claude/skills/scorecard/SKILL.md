---
name: scorecard
description: Build a relationship scorecard for a character — how others express feeling toward them. Use when asked how the town, another character, or "everyone" feels about a character, or for a character scorecard.
---

# Character scorecard

How the town feels about one character, judged from what people actually
said to and around them. Expressed feeling only: minds you do not host are
not readable, and the claims ledger holds only what the character was
present to hear.

## 1. Extract

```
node scripts/relations.mjs <claims-file.json> <CharacterName>
```

The claims file lives in the character's memory volume
(`<character>-claims.json`). Read it from the volume read-only; never write
to a memory volume.

## 2. Judge each observer on five stats

- **Warmth** — tone when addressing the target: greetings, acceptance,
  affection vs rebuffs or condescension. Note *non-differential* warmth: a
  speaker whose warm line is verbatim identical toward several characters
  scores as habit, not feeling.
- **Engagement** — utterance count, and whether replies are substantive or
  perfunctory.
- **Confide rate** — volunteering information, plans, or feelings
  unprompted. Being told things is the trust signal.
- **Goal echo** — read the target's persona (`personas/<name>.md`) for
  their long-run goal, then look for observers advancing or refuting it in
  their own words. Distinguish an echo (traceably downstream of the
  target's seeding) from an independent source saying similar things.
- **Salience** — mentionsOfTarget when the target is not the one being
  answered.

## 3. Report

One table row per observer (warmth / engagement / confides / echo / one-line
read), then the insights that do not fit a cell: refuted seeds, role drift,
witnessed glitches. Flag every judgment resting on fewer than five
utterances as thin. State the blind spot: this is conduct toward the
character, not private opinion of them.
