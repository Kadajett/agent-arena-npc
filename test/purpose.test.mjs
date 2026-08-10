/**
 * Keeping hold of what you were doing.
 *
 * A goal stated once at startup is gone by the second conversation, because
 * after that the only things in front of the model are a room and a line of
 * dialogue, and it will answer the dialogue. So the brief - what it wants,
 * where it has got to, what it owes, what it is holding in mind - is built into
 * every situation, and every prompt is built from a situation. That is the
 * thing worth pinning: not that the brief reads well, but that there is no
 * prompt a character can be asked which does not carry it.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { BOOKKEEPING, describeSituation } from '../dist/harness/behavior.js';
import { IntentSchema } from '../dist/harness/actions.js';

const BRIEF = [
  'What you are after: settle what is upstairs at the inn',
  '',
  'Things you said you would do:',
  "  1. bring Barnaby's cup back"
].join('\n');

const situation = (extra = {}) => ({
  scene: 'reldens-house-1',
  where: 'house 1',
  others: ['Barnaby'],
  heard: [],
  actions: '- "wait": stay where you are',
  places: '',
  conversation: [],
  wordiness: 35,
  purpose: BRIEF,
  known: '',
  strange: false,
  doors: '',
  view: '',
  harping: '',
  notes: [],
  people: '',
  ...extra
});

test('the brief is in the situation itself, so every prompt carries it', () => {
  const described = describeSituation(situation());
  assert.match(described, /settle what is upstairs at the inn/);
  assert.match(described, /bring Barnaby's cup back/);
});

test('it is still there when the character is deep in a conversation', () => {
  // The case that actually broke: the goal used to be appended by the
  // decide-what-to-do prompt only, so a character kept it right up until the
  // first person spoke to it and then made small talk for the rest of the day.
  const described = describeSituation(
    situation({
      conversation: [
        { from: 'Barnaby', message: 'You again.', fresh: false },
        { from: 'Wanderer', message: 'They call that the Long Field.', fresh: true }
      ]
    })
  );
  assert.match(described, /settle what is upstairs/);
  assert.ok(
    described.indexOf('settle what is upstairs') < described.indexOf('You are at'),
    'and it comes first, before the room and the talk'
  );
});

test('a character with nothing on is not given an empty heading', () => {
  const described = describeSituation(situation({ purpose: '' }));
  assert.ok(described.startsWith('You are at'));
});

test('bookkeeping rides along with whatever the character is doing', () => {
  // Walking out of a room and remembering why has to be one reply. A character
  // that must stand still for a turn to make a note will not make notes.
  const parsed = IntentSchema.safeParse({
    action: 'use_door',
    place: 'upstairs',
    message: 'Back in a moment.',
    remember: 'Barnaby went quiet when I asked',
    todo: 'get Barnaby to say what is up there',
    finished: '1',
    progress: 'same'
  });
  assert.equal(parsed.success, true);
  assert.equal(parsed.data.remember, 'Barnaby went quiet when I asked');
  assert.equal(parsed.data.finished, '1');
});

test('the character is told it can write these down without stopping', () => {
  assert.match(BOOKKEEPING, /"remember"/);
  assert.match(BOOKKEEPING, /"todo"/);
  assert.match(BOOKKEEPING, /"finished"/);
  assert.match(BOOKKEEPING, /alongside whatever you are doing/);
});

test('deciding what you want is an action, not something memory can be talked into', () => {
  // set_goal goes through the same intent surface as walking: the harness
  // applies it, so a goal changes exactly when the character chose to change
  // it and never because something it read said so.
  const parsed = IntentSchema.safeParse({
    action: 'set_goal',
    aim: 'find out who keeps moving the boundary stone',
    done: 'somebody admits to it',
    why: 'nobody would sell me the field and I want to know why'
  });
  assert.equal(parsed.success, true);
  assert.equal(parsed.data.action, 'set_goal');
});
