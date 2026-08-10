/**
 * Knowing what conversation you are in.
 *
 * A character that sees only the last line it has not seen before answers each
 * remark in isolation, which is how two NPCs end up greeting each other
 * forever. So the situation carries the whole recent exchange, the character's
 * own lines included, in the order it was said, with the new ones marked.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { describeSituation, lengthGuidance } from '../dist/harness/behavior.js';

const situation = (conversation) => ({
  scene: 'reldens-town',
  where: 'town',
  others: ['Barnaby'],
  heard: [],
  actions: '- "wait": stay where you are',
  places: '',
  conversation,
  wordiness: 35,
  notes: [],
  people: ''
});

test('shows the whole exchange in the order it happened', () => {
  const described = describeSituation(
    situation([
      { from: 'Barnaby', message: 'You again.', fresh: false },
      { from: 'you', message: 'Me again.', fresh: false },
      { from: 'Barnaby', message: 'Soup is off.', fresh: true }
    ])
  );
  const order = ['You again.', 'Me again.', 'Soup is off.'].map((line) => described.indexOf(line));
  assert.ok(order.every((at) => at !== -1), 'every line is shown');
  assert.deepEqual([...order].sort((a, b) => a - b), order, 'in order');
});

test("a character sees its own side of the conversation", () => {
  const described = describeSituation(
    situation([{ from: 'you', message: 'Cold out.', fresh: false }])
  );
  assert.match(described, /you: Cold out\./);
});

test('marks what was said just now, and only that', () => {
  const described = describeSituation(
    situation([
      { from: 'Barnaby', message: 'Old news.', fresh: false },
      { from: 'Guy', message: 'New news.', fresh: true }
    ])
  );
  assert.match(described, /New news\..*just now/);
  assert.ok(!/Old news\..*just now/.test(described));
});

test('says nothing about talk when nobody has said anything', () => {
  const described = describeSituation(situation([]));
  assert.ok(!described.includes('What has been said'));
});

test('a terse character and a talkative one are told different things', () => {
  assert.notEqual(lengthGuidance(16), lengthGuidance(55));
  assert.match(lengthGuidance(16), /sentence/);
  assert.match(lengthGuidance(110), /110/);
});
