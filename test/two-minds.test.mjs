/**
 * A character that answers twice loses the turn.
 *
 * Marren said what she meant to do, then thought better of it and wrote a
 * fuller version underneath. Both were valid. The reply was read as the span
 * from the first brace to the last one, which took both objects and the blank
 * line between them, and that is not JSON, so the parse threw and she stood
 * still having decided twice what to do.
 *
 * Worse than the lost turn: that path returned a plain wait, so the drift
 * correction could not tell it apart from a character choosing to stand still.
 * A model emitting broken JSON on every tick would never have been told.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';

import { firstJsonObject } from '../dist/harness/behavior.js';

test('two objects in one reply reads as the first, not as neither', () => {
  const said = '{"action":"say","message":"Any word on the volcano betting ring?"}\n\n'
    + '{"action":"say","message":"Any word on the volcano betting ring?","progress":"same"}';
  const found = firstJsonObject(said);
  assert.doesNotThrow(() => JSON.parse(found));
  assert.equal(JSON.parse(found).action, 'say');
});

test('an object with prose either side of it still reads, as it always did', () => {
  const said = 'Let me think. {"action":"walk","place":"the east gate"} That should do it.';
  assert.deepEqual(JSON.parse(firstJsonObject(said)), { action: 'walk', place: 'the east gate' });
});

test('a nested object is not cut off at the first inner brace', () => {
  const said = '{"action":"say","message":"hello","extra":{"deep":{"deeper":1}}}';
  assert.equal(JSON.parse(firstJsonObject(said)).extra.deep.deeper, 1);
});

test('a brace inside something a character says is a character, not structure', () => {
  const said = '{"action":"say","message":"the sign reads {closed}"}';
  const parsed = JSON.parse(firstJsonObject(said));
  assert.equal(parsed.message, 'the sign reads {closed}');
});

test('an escaped quote does not end the string early', () => {
  const said = '{"action":"say","message":"he said \\"go\\" and I went"}';
  const parsed = JSON.parse(firstJsonObject(said));
  assert.match(parsed.message, /he said "go" and I went/);
});

test('a reply cut off mid-object is left for the prose salvage', () => {
  // The old widest-span read would have handed this to JSON.parse too, and it
  // would have thrown. Returning nothing sends it down the path that at least
  // says the words out loud.
  assert.equal(firstJsonObject('{"action":"say","message":"I was about to'), '');
});

test('a reply with no braces at all is unchanged', () => {
  assert.equal(firstJsonObject('I walk to the east gate and look around.'), '');
});
