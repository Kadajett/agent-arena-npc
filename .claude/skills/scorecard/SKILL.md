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

## 2. Judge each observer on four stats

- **Warmth** — tone when addressing the character: greetings, acceptance,
  affection vs rebuffs or condescension. Note *non-differential* warmth: a
  speaker whose warm line is verbatim identical toward several characters
  scores as habit, not feeling.
- **Engagement** — utterance count, and whether replies are substantive
  or perfunctory.
- **Confides** — volunteering information, plans, or feelings unprompted.
  Being told things is the trust signal.
- **Salience** — `mentionsOfTarget` while the character is not the one
  being addressed. (Behind-their-back mentions arrive with the public
  chat-feed walk — see the spec's AC4.)

## 3. Report

One table row per observer — warmth / engagement / confides / salience /
a one-line read — every judgment backed by a quotable line, then the
insights that do not fit a cell. Flag every judgment resting on fewer
than five utterances as thin.

Voice rules: written for a viewer. No harness or operator internals —
goals, prompts, model behavior — in the output. Quotes verbatim; a
paraphrase is never shown as a quote. State the blind spot: this is
conduct toward the character, not private opinion of them.
