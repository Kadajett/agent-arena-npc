/**
 * What a character says out loud.
 *
 * Two limits meet here. How much a character says at a stretch is a trait, and
 * is enforced by dropping whole sentences. How much fits in one chat line is
 * Reldens' 100 characters, and is enforced by sending several lines. Models
 * also narrate themselves in asterisks however firmly the persona tells them
 * not to. All three turn a good line into a bad one at the last possible
 * moment, so they are pinned here.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { toSpeech, CHAT_LINE_LIMIT, MAX_WORDS } from '../dist/harness/actions.js';

const words = (lines) => lines.join(' ').split(/\s+/).filter(Boolean).length;

test('a short line goes out as one chat line', () => {
  assert.deepEqual(toSpeech("It's turnip, Barnaby. It's always been turnip.", 35), [
    "It's turnip, Barnaby. It's always been turnip."
  ]);
});

test('every line fits the chat field', () => {
  const lines = toSpeech(
    'Four times is about four too many for those trees. The east gate is just '
      + 'as quiet, and the land out there is cheaper than anyone in this town '
      + 'thinks it is. I have been counting.',
    60
  );
  assert.ok(lines.length > 1, 'a long thought spans several lines');
  for (const line of lines) {
    assert.ok(line.length <= CHAT_LINE_LIMIT, `too long: ${line}`);
  }
});

test('a talkative character says more than a terse one', () => {
  const speech = 'One. Two. Three. Four. Five. Six. Seven. Eight. Nine. Ten.';
  const terse = toSpeech(speech, 3).join(' ');
  const chatty = toSpeech(speech, 20).join(' ');
  assert.ok(chatty.length > terse.length);
  assert.equal(terse, 'One. Two. Three.');
});

test('spends the budget on whole sentences, never half of one', () => {
  // Ten words of budget covers the first two sentences; the third would take
  // it to fifteen, so it is dropped whole rather than trailed off.
  const lines = toSpeech('I keep the fire going. Nobody asked me to. It is just what I do.', 10);
  assert.equal(lines.join(' '), 'I keep the fire going. Nobody asked me to.');
});

test('nobody may talk past the ceiling', () => {
  const lines = toSpeech('word word word word word word word word word word. '.repeat(40), 5000);
  assert.ok(words(lines) <= MAX_WORDS, `said ${words(lines)} words`);
});

test('throws away stage directions instead of speaking them', () => {
  assert.deepEqual(toSpeech("*Guy shrugs.* Fine. But I'm charging you for the walk.", 35), [
    "Fine. But I'm charging you for the walk."
  ]);
});

test('breaks an over-long sentence on a word', () => {
  const lines = toSpeech('word '.repeat(60).trim(), 120);
  for (const line of lines) {
    assert.ok(line.length <= CHAT_LINE_LIMIT);
  }
  assert.ok(
    lines.every((line) => !/\bwor$|^rd\b/.test(line)),
    'must not cut mid-word'
  );
});

test('says nothing when there is nothing to say', () => {
  assert.deepEqual(toSpeech('   ', 35), []);
  assert.deepEqual(toSpeech('**', 35), []);
});
