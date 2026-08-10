/**
 * Guy asked for "the east gate" thirteen times from inside a house.
 *
 * Each time he was told there is no door here called that, which is true, and
 * useless. The east gate is a spot in town, through a door he could see the
 * whole time. He had been told plainly it had failed three, four, five times,
 * and told he was going in circles, and none of it helped, because none of it
 * told him the one thing he needed: that the name he was using is a place, not
 * a door, and how to get to it.
 *
 * A character that knows where somewhere is and cannot reach it is worse off
 * than one that has never heard of the place.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';

import { Actions } from '../dist/harness/actions.js';

const HOUSE = 'reldens-house-2';

function inHouseWithDoors(doors) {
  const arena = {
    calls: [],
    async call(tool) {
      this.calls.push(tool);
      if (tool === 'arena_observe') return { scene: HOUSE, objects: [], ownPlayer: { state: { x: 0, y: 0 } } };
      return {};
    }
  };
  const actions = new Actions(arena, 'agent-1', new Set(['walk', 'doors']));
  actions.sees({ doors, corners: [] });
  return { arena, actions };
}

test('asking for a spot in the next room takes the door to that room', async () => {
  // Not advice. Guy was handed the advice - "go through the door to town
  // first, then walk to it" - word for word, and asked for the same door five
  // more times, because a model copies the pattern its own history shows and
  // his history was thirteen attempts at it. There is no guesswork here: the
  // name is a place, the place is in a room, and one door goes to that room.
  const { arena, actions } = inHouseWithDoors([
    { leadsTo: 'reldens-town', tileX: 5, tileY: 5 },
    { leadsTo: 'reldens-gravity', tileX: 9, tileY: 9 }
  ]);
  await actions.useDoor(HOUSE, 'the east gate');
  assert.ok(
    arena.calls.includes('arena_enter_door'),
    'it should walk through the door to town, not explain that it could have'
  );
});

test('a real place with no door from here says both halves', async () => {
  // Two doors, because one door is taken whatever it is called: with a single
  // way out there is nothing to disambiguate and the name never gets resolved.
  const { actions } = inHouseWithDoors([
    { leadsTo: 'reldens-gravity', tileX: 9, tileY: 9 },
    { leadsTo: 'reldens-gravity', tileX: 10, tileY: 9 }
  ]);
  const result = await actions.useDoor(HOUSE, 'the east gate');
  assert.equal(result.ok, false);
  assert.match(result.note, /a spot in town/);
  assert.match(result.note, /no door to there from here/);
  assert.match(result.note, /the sunken chamber/, 'and says where the doors do go');
});

test('a name that is nowhere at all still gets the old plain answer', async () => {
  const { actions } = inHouseWithDoors([
    { leadsTo: 'reldens-town', tileX: 5, tileY: 5 },
    { leadsTo: 'reldens-gravity', tileX: 9, tileY: 9 }
  ]);
  const result = await actions.useDoor(HOUSE, 'the Hinge Gate');
  assert.equal(result.ok, false);
  assert.match(result.note, /no door here it would call "the Hinge Gate"/);
  assert.match(result.note, /It can see: /);
});

test('asking for a door to a spot in this very room becomes the walk it meant', async () => {
  // Guy stood in town asking for a door to the south field, forty tiles away
  // across the grass he was looking at, and was told there was no door to
  // there from here. True, in the way only useless things are true.
  const arena = {
    calls: [],
    async call(tool, args) {
      this.calls.push({ tool, args });
      if (tool === 'arena_observe') return { scene: 'reldens-town', objects: [], ownPlayer: { state: { x: 0, y: 0 } } };
      return {};
    }
  };
  const actions = new Actions(arena, 'agent-1', new Set(['walk', 'doors']));
  actions.sees({ doors: [
    { leadsTo: 'reldens-house-1', tileX: 5, tileY: 5 },
    { leadsTo: 'reldens-forest', tileX: 9, tileY: 9 }
  ], corners: [] });
  const result = await actions.useDoor('reldens-town', 'the south field');
  const moved = arena.calls.find((call) => call.tool === 'arena_move_to');
  assert.ok(moved, 'it should walk there rather than discuss doors');
  // The mock arena never actually moves the body, so the walk reports
  // stopping short. What matters is that it walked and the note is about the
  // walk, not about doors.
  assert.match(result.note, /the south field/);
  assert.ok(!/door/.test(result.note), 'no more talking about doors');
});
