/**
 * The hard rule: everyone acts and speaks in the first person.
 *
 * Guy started writing whole scenes, other people included: "I push through the
 * inn door, pull up my usual stool. Barnaby's behind the bar, wiping a cup with
 * a rag. He glances up, sees my face, and sets the cup down." Barnaby was not
 * consulted and did none of it. Meanwhile Guy had not moved.
 *
 * A version of this rule already lived in the brief, which is the per-call
 * instructions - the same place a drifting model demonstrably ignores, and the
 * reason the format correction had to move into the moment. So it belongs in
 * the primer: the system message, the same for everyone, always present.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';

import { READING_THE_WORLD, withPrimer } from '../dist/harness/primer.js';

test('the rule is in the primer, so it is in front of every character always', () => {
  const primed = withPrimer('You are Guy, a swordsman.');
  assert.ok(primed.includes('You are Guy, a swordsman.'), 'the persona still leads');
  assert.ok(
    /first person/i.test(primed),
    'and the first-person rule rides along in the system message'
  );
});

test('it forbids narrating your own actions, not just asterisks', () => {
  // The old wording only banned asterisks and "description of your own
  // actions", which Guy satisfied to the letter while narrating constantly.
  assert.ok(/asterisks/i.test(READING_THE_WORLD), 'asterisks still called out');
  assert.ok(
    /stage directions/i.test(READING_THE_WORLD),
    'and stage directions by name, which the old wording never covered'
  );
  // Line-wrapped in the source, so match on the collapsed text rather than
  // letting a newline in the middle of a sentence decide whether this passes.
  const flowing = READING_THE_WORLD.replace(/\s+/g, ' ');
  assert.ok(
    /only what actually leaves your mouth/i.test(flowing),
    'with a test the character can apply to its own reply'
  );
});

test('it forbids writing other characters at all, which nothing did before', () => {
  assert.ok(
    /Never write what somebody else does, says, thinks or notices/i.test(READING_THE_WORLD),
    'the gap that let Guy author Barnaby'
  );
});

test('it says plainly that describing a thing is not doing it', () => {
  // This is the same lesson as the format-drift correction, in the one place
  // every character reads every turn: prose does not move anybody.
  assert.ok(/Describing a thing is not doing it/i.test(READING_THE_WORLD));
});
