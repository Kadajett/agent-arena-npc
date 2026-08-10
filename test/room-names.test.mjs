/**
 * No character should ever say a word that belongs to the engine.
 *
 * Guy was heard talking about "bots". Nothing had leaked into his memory:
 * reldens-bots is a real room, and the fallback that turns a scene key into a
 * name strips the prefix and the dashes and hands back "bots", so the world
 * told him there was a place called that and he repeated it. The same fallback
 * had "gravity", "arena crypt", and one room that came out as "bots forest
 * house 01 n0", which no person has ever said out loud.
 *
 * These pin the whole set, because the failure is not in any one name. It is
 * that a room without a name gets one made out of its database key, silently,
 * and the first anybody hears of it is a character saying it in public.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';

import { SCENE_NAMES, plainSceneName, rawSceneName } from '../dist/harness/world.js';

/** Every room in the live world, from `SELECT name FROM rooms` on the VPS. */
const EVERY_ROOM = [
  'reldens-town',
  'reldens-house-1',
  'reldens-house-1-2d-floor',
  'reldens-house-2',
  'reldens-forest',
  'reldens-bots',
  'reldens-bots-forest',
  'reldens-bots-forest-house-01-n0',
  'reldens-gravity',
  'arena-grassland',
  'arena-crypt',
  'arena-depths',
  'arena-shore',
  'arena-volcano',
  'arena-dungeon'
];

/** Words that give the game away as software rather than a place. */
const OUT_OF_CHARACTER = /\b(bots?|reldens|arena|n0|2d floor|house 01)\b|\d/i;

test('every room in the world has a name a person would say', () => {
  for (const scene of EVERY_ROOM) {
    const name = plainSceneName(scene);
    assert.ok(SCENE_NAMES[scene], `${scene} has no name of its own`);
    assert.doesNotMatch(
      name,
      OUT_OF_CHARACTER,
      `${scene} reads as "${name}", which is engine vocabulary`
    );
  }
});

test('the room that started it is not called bots', () => {
  assert.equal(plainSceneName('reldens-bots'), 'the clearing');
  assert.equal(rawSceneName('reldens-bots'), 'bots', 'the raw form is why this happened');
});

test('the worst of them is no longer a serial number', () => {
  assert.equal(plainSceneName('reldens-bots-forest-house-01-n0'), "the woodcutter's hut");
});

test('a room nobody has named still falls back rather than throwing', () => {
  // The fallback is not being removed, only emptied of rooms that exist. A
  // region added tomorrow should appear under a rough name rather than break a
  // character mid-sentence.
  assert.equal(plainSceneName('arena-somewhere-new'), 'somewhere new');
});

test('the names are distinct, so a character can tell two doors apart', () => {
  const names = EVERY_ROOM.map(plainSceneName);
  assert.equal(new Set(names).size, names.length, 'two rooms sharing a name is a door you cannot pick');
});
