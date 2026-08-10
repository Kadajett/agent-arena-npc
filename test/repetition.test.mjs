/**
 * Not saying the same thing over and over.
 *
 * A model never repeats itself word for word. It says "the road's the same as
 * ever", then "same road as always", then "road hasn't changed", and a
 * character sounds like a broken toy while every exact-match check passes. So
 * the test is what a line is *about*, not how it is spelt.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { harpingOn, isTooSimilar } from '../dist/harness/actions.js';

test('catches the same remark in different words', () => {
  const before = ["The road's the same as ever."];
  assert.equal(isTooSimilar('Same road as always.', before), true);
});

test('a fresh line on the same subject is not a repeat', () => {
  // Two mechanisms, deliberately: near-duplicates are dropped outright, but a
  // new thought about a subject already raised is how conversation works and
  // must get through. Somebody who will not stop bringing the subject up is
  // caught by harpingOn instead.
  const before = ["The road's the same as ever."];
  assert.equal(isTooSimilar("Road washed out past the bridge, though.", before), false);
});

test('lets a genuinely new line through', () => {
  const before = ["The road's the same as ever.", 'Ground is soft out west.'];
  assert.equal(isTooSimilar('Barnaby, the soup is off again.', before), false);
});

test('a short repeat of a long line still counts as a repeat', () => {
  const before = ['I have been telling people about the upstairs at the inn for weeks now.'];
  assert.equal(isTooSimilar('The upstairs at the inn.', before), true);
});

test('says nothing about an empty history', () => {
  assert.equal(isTooSimilar('Anything at all.', []), false);
});

test('names the subject a character keeps circling', () => {
  const said = [
    "The road's fine.",
    'Road was quiet today.',
    'Nothing on the road.',
    'Soup is off.'
  ];
  const nagging = harpingOn(said);
  assert.match(nagging, /"road"/);
  assert.match(nagging, /something else/);
});

test('stays quiet when a character is not repeating itself', () => {
  assert.equal(harpingOn(['Soup is off.', 'Rain tomorrow.', 'Pip owes me.']), '');
  assert.equal(harpingOn(['Only one line.']), '');
});
