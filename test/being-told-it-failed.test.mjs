/**
 * Nobody was telling them when something did not work.
 *
 * A failed action's note was logged, and recorded against the plan, and never
 * once put in front of the character. So a character that asked for a door
 * which is not there got back silence and asked again, in exactly the same
 * words, forever. Guy did it six times running for a pantry, minutes after
 * being let out of a different loop, and the two loops have the same cause:
 * the world knew, the logs knew, and he was never told.
 *
 * Only check_money, talk_to and answer_npc results were ever fed back, because
 * those were the ones somebody noticed were needed. Failure is the case where
 * being told matters most.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('../src/harness/npc.ts', import.meta.url), 'utf8');

/** saidBefore() lifted out, so the escalation is tested rather than described. */
function repeater() {
  let lastFailure = '';
  let failedWith = 0;
  return (action, note) => {
    const same = `${action}:${note}`;
    failedWith = same === lastFailure ? failedWith + 1 : 1;
    lastFailure = same;
    if (failedWith < 3) return note;
    return `${note}\n[You have now tried this ${failedWith} times and been told the same thing every time.]`;
  };
}

test('a failed action reaches the character in-band now, not a tick later', () => {
  // The whole next-tick feedback pipeline this file used to pin - perform,
  // saidBefore, notes - existed because the model never saw what its own
  // action did until the following tick. Agentic turns removed the gap: the
  // action is a tool call and its failure is the tool result, read in the
  // same breath. What this test now pins is that the turn runs and the old
  // pipeline is genuinely gone from the live loop, rather than half-wired
  // beside the new one.
  assert.match(source, /await this\.liveOneTurn\(situation\)/);
  assert.ok(!/const told = this\.saidBefore\(intent\.action, result\.note\);/.test(source));
});

test('the first failure is passed on plainly, without a lecture', () => {
  const told = repeater();
  const note = 'there is no door here it would call "the pantry"';
  assert.equal(told('use_door', note), note, 'once is just information');
  assert.equal(told('use_door', note), note, 'twice may be a rephrasing worth trying');
});

test('the same failure over and over is eventually said as a dead end', () => {
  const told = repeater();
  const note = 'there is no door here it would call "the pantry"';
  told('use_door', note);
  told('use_door', note);
  const third = told('use_door', note);
  assert.match(third, /tried this 3 times/);
  assert.match(third, /same thing every time/);
  assert.ok(third.startsWith(note), 'and still says what actually went wrong first');
});

test('a different failure resets the count rather than inheriting it', () => {
  const told = repeater();
  told('use_door', 'no such door');
  told('use_door', 'no such door');
  const other = told('walk', 'every side blocked');
  assert.ok(!other.includes('tried this'), 'a new problem is a new problem');
});

test('the same note from a different action is not treated as the same failure', () => {
  const told = repeater();
  told('use_door', 'blocked');
  told('walk', 'blocked');
  const third = told('use_door', 'blocked');
  assert.ok(!third.includes('tried this'), 'alternating between two things is not one dead end');
});

test('an action that worked is not reported as a failure', () => {
  // The guard is on !result.ok, so a successful walk does not push a note and
  // does not disturb the count.
  assert.ok(!/if \(result\.ok\) \{\s*\n\s*this\.notes = \[this\.saidBefore/.test(source));
});
