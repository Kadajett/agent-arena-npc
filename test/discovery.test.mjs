/**
 * Finding things out, and hearing about them from somebody else.
 *
 * The world is only worth talking about if nobody starts out knowing it. So a
 * character records where it has been, records separately what it was merely
 * told, and keeps the two apart - because the gap between them is the reason to
 * walk across town and look. Going somewhere yourself settles the question;
 * somebody repeating a rumour does not.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  WorkingMemorySchema,
  describePlacesKnown,
  notePlace,
  unconfirmed
} from '../dist/harness/memory.js';
import { dedupeDoors, describeDoors } from '../dist/harness/explore.js';
import { PLACES, isHomeTurf, plainSceneName } from '../dist/harness/world.js';

const empty = () => WorkingMemorySchema.parse({});

test('a character starts out knowing only its own town', () => {
  // Everything else is discovered. If this list grows to cover the map, every
  // character is omniscient again and nothing is worth telling anybody.
  assert.deepEqual(Object.keys(PLACES).sort(), ['reldens-house-1', 'reldens-town']);
  assert.equal(isHomeTurf('reldens-town'), true);
  assert.equal(isHomeTurf('reldens-forest'), false);
  assert.equal(isHomeTurf('reldens-house-2'), false);
});

test('what you were told is kept apart from what you saw', () => {
  let state = notePlace(empty(), {
    where: 'upstairs at the inn',
    what: 'full of something',
    how: 'heard',
    who: 'Guy'
  });
  assert.equal(unconfirmed(state).length, 1);
  assert.match(describePlacesKnown(state), /only been told about/);
  assert.match(describePlacesKnown(state), /Guy said so/);
});

test('going yourself settles it', () => {
  let state = notePlace(empty(), {
    where: 'upstairs at the inn',
    what: 'full of something',
    how: 'heard',
    who: 'Guy'
  });
  state = notePlace(state, {
    where: 'upstairs at the inn',
    what: 'two empty rooms and a landing',
    how: 'been'
  });
  assert.equal(unconfirmed(state).length, 0, 'nothing left to check');
  assert.equal(state.places.length, 1, 'the same place, not a second copy');
  assert.equal(state.places[0].what, 'two empty rooms and a landing');
  assert.match(describePlacesKnown(state), /Places you have been/);
});

test('a rumour does not overwrite what you saw with your own eyes', () => {
  let state = notePlace(empty(), {
    where: 'the forest',
    what: 'trees and a river',
    how: 'been'
  });
  state = notePlace(state, {
    where: 'the forest',
    what: 'full of wolves, apparently',
    how: 'heard',
    who: 'the Wanderer'
  });
  assert.equal(state.places[0].how, 'been');
  assert.equal(state.places[0].what, 'trees and a river');
});

test('a two-tile gateway is one door, not two', () => {
  const doors = dedupeDoors([
    { x: 592, y: 16, row: 0, column: 18, leadsTo: 'reldens-forest', locked: false, lockKnown: true },
    { x: 624, y: 16, row: 0, column: 19, leadsTo: 'reldens-forest', locked: false, lockKnown: true },
    { x: 400, y: 304, row: 9, column: 12, leadsTo: 'reldens-house-1', locked: false, lockKnown: true }
  ]);
  assert.equal(doors.length, 2);
});

test('doors are described by where they go', () => {
  const described = describeDoors(
    {
      scene: 'reldens-town',
      doors: [
        { x: 400, y: 304, row: 9, column: 12, leadsTo: 'reldens-house-1', locked: false, lockKnown: true },
        { x: 592, y: 16, row: 0, column: 18, leadsTo: 'reldens-forest', locked: true, lockKnown: true }
      ],
      map: '',
      widthTiles: 48,
      heightTiles: 28
    },
    plainSceneName
  );
  assert.match(described, /Barnaby's inn/);
  assert.match(described, /forest.*locked/);
});

test('nothing to see means nothing claimed', () => {
  assert.equal(describeDoors(null, plainSceneName), '');
  assert.equal(describePlacesKnown(empty()), '');
});
