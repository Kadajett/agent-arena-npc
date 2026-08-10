/**
 * Exploring the same room forever, four bearings at a time.
 *
 * Guy spent an afternoon in town going "had a look around to the south-west",
 * north-west, south-east, round and round. Every explore picks a fresh spot,
 * so every explore succeeds, so the model copies its own success; and the
 * circling detector keys on action plus place, so four different bearings
 * never read as the same move. Meanwhile the door to the forest - a room he
 * had never once stood in, with things in it to fight - sat in his door list
 * labelled "(never been)" the whole time.
 *
 * So the harness acts, the same way it does at the pickDoor fallthrough: the
 * fourth explore of a scene, when a never-visited unlocked door is right
 * there, becomes taking that door. Advice was tried first. Advice lost.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';

import { Actions } from '../dist/harness/actions.js';

const TOWN = 'reldens-town';

function inTown(doors, capabilities = ['walk', 'doors']) {
  const arena = {
    calls: [],
    async call(tool, args) {
      this.calls.push({ tool, args });
      if (tool === 'arena_observe') {
        return { scene: TOWN, objects: [], ownPlayer: { state: { x: 64, y: 64 } } };
      }
      if (tool === 'arena_enter_door') return { entered: true, scene: doors[0]?.leadsTo };
      if (tool === 'arena_path_check') return { walkable: true };
      return {};
    }
  };
  const actions = new Actions(arena, 'agent-1', new Set(capabilities));
  actions.sees({ doors, corners: [] });
  return { arena, actions };
}

const wentThroughADoor = (arena) => arena.calls.some((call) => call.tool === 'arena_enter_door');

test('the fourth explore of a room with an unseen door behind it takes the door', async () => {
  const { arena, actions } = inTown([{ leadsTo: 'reldens-forest', tileX: 2, tileY: 2 }]);
  for (let i = 0; i < 3; i++) {
    await actions.explore(TOWN);
    assert.equal(wentThroughADoor(arena), false, `wandering is still allowed on look ${i + 1}`);
  }
  const fourth = await actions.explore(TOWN);
  assert.equal(wentThroughADoor(arena), true, 'the fourth look becomes the door');
  assert.equal(fourth.ok, true);
  assert.match(fourth.note, /nothing left it had not seen/);
  assert.match(fourth.note, /somewhere it had never been/);
});

test('a room already visited does not pull, however often this one is wandered', async () => {
  const { arena, actions } = inTown([{ leadsTo: 'reldens-forest', tileX: 2, tileY: 2 }]);
  actions.remembersRooms(new Map([['reldens-forest', 'an hour ago']]));
  for (let i = 0; i < 6; i++) {
    await actions.explore(TOWN);
  }
  assert.equal(wentThroughADoor(arena), false, 'every door led somewhere already seen');
});

test('a locked door does not count as somewhere to be taken', async () => {
  const { arena, actions } = inTown([
    { leadsTo: 'reldens-forest', tileX: 2, tileY: 2, locked: true }
  ]);
  for (let i = 0; i < 6; i++) {
    await actions.explore(TOWN);
  }
  assert.equal(wentThroughADoor(arena), false);
});

test('without the doors capability it wanders and never converts', async () => {
  const { arena, actions } = inTown(
    [{ leadsTo: 'reldens-forest', tileX: 2, tileY: 2 }],
    ['walk']
  );
  for (let i = 0; i < 6; i++) {
    await actions.explore(TOWN);
  }
  assert.equal(wentThroughADoor(arena), false);
});

test('the conversion resets, so coming back means earning it again', async () => {
  const { arena, actions } = inTown([{ leadsTo: 'reldens-forest', tileX: 2, tileY: 2 }]);
  for (let i = 0; i < 4; i++) {
    await actions.explore(TOWN);
  }
  assert.equal(wentThroughADoor(arena), true);
  const before = arena.calls.filter((call) => call.tool === 'arena_enter_door').length;
  // Back in town with the forest now visited: three more wanders and no
  // conversion, because the one unseen door is unseen no longer.
  actions.remembersRooms(new Map([['reldens-forest', 'just now']]));
  for (let i = 0; i < 4; i++) {
    await actions.explore(TOWN);
  }
  const after = arena.calls.filter((call) => call.tool === 'arena_enter_door').length;
  assert.equal(after, before, 'nowhere unseen means wandering stays wandering');
});
