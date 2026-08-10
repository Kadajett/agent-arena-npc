/**
 * The four characters added to shake up the world's empty rooms.
 *
 * Not a test of behaviour - that is what the rest of this suite covers via
 * the harness itself. This is a sanity check on the sheets: each one wires a
 * distinct id and scene, sticks to capabilities the harness actually knows
 * about, and defaults to the free model router rather than the one Guy,
 * Barnaby and the Wanderer pay for.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { CAPABILITIES } from '../dist/harness/actions.js';
import { guy } from '../dist/characters/guy.js';
import { barnaby } from '../dist/characters/barnaby.js';
import { wanderer } from '../dist/characters/wanderer.js';
import { tansy } from '../dist/characters/tansy.js';
import { hollis } from '../dist/characters/hollis.js';
import { marren } from '../dist/characters/marren.js';
import { cutter } from '../dist/characters/cutter.js';

const NEW_CAST = { tansy, hollis, marren, cutter };
const WHOLE_CAST = { guy, barnaby, wanderer, ...NEW_CAST };
const LING = 'openrouter/openai/gpt-oss-120b';

/**
 * There used to be two tiers here: a paid model for the original three and the
 * free router for everybody else. Both halves went wrong. The free router turned
 * out to be capped at 1,000 requests a day for the whole account, which six
 * autonomous characters exhaust before lunch, and the paid model was thirteen
 * times the price of one that answers just as well and slightly faster, checked
 * over fourteen runs of this world's own reply format. So the tier bought
 * nothing worth the split and everybody thinks with the same thing now.
 *
 * The free router has not gone anywhere. It sits underneath every character as
 * the fallback, which is where it was always useful. See models.ts.
 */
test('everybody in this world thinks with the same model, with no tier left over', () => {
  for (const [id, sheet] of Object.entries(WHOLE_CAST)) {
    assert.equal(sheet.model, LING, `${id} is on a model of its own`);
  }
});

test('every character sheet has a unique id and a unique player name', () => {
  const ids = Object.values(WHOLE_CAST).map((sheet) => sheet.id);
  const names = Object.values(WHOLE_CAST).map((sheet) => sheet.playerName);
  assert.equal(new Set(ids).size, ids.length, 'duplicate character id');
  assert.equal(new Set(names).size, names.length, 'duplicate player name');
});

test('the new characters are spread across distinct, previously empty rooms', () => {
  const scenes = Object.values(NEW_CAST).map((sheet) => sheet.homeScene);
  assert.equal(new Set(scenes).size, scenes.length, 'two new characters share a home scene');
  assert.deepEqual(
    new Set(scenes),
    new Set(['reldens-house-2', 'reldens-bots-forest-house-01-n0', 'arena-grassland', 'arena-shore'])
  );
  // None of them starts in town - the point was to stop putting everybody there.
  for (const scene of scenes) {
    assert.notEqual(scene, 'reldens-town');
  }
});

test('every capability on every new sheet is one the harness actually implements', () => {
  for (const [id, sheet] of Object.entries(NEW_CAST)) {
    for (const capability of sheet.capabilities) {
      assert.ok(
        CAPABILITIES.includes(capability),
        `${id} was given "${capability}", which is not a real capability`
      );
    }
    // Nobody gets by without these two: no capability to speak or to talk to
    // folk makes every other capability on the sheet unreachable.
    assert.ok(sheet.capabilities.includes('speak'));
    assert.ok(sheet.capabilities.includes('talk_to_folk'));
  }
});

test('every new sheet has a goal seed with something to work toward', () => {
  for (const [id, sheet] of Object.entries(NEW_CAST)) {
    assert.ok(sheet.goal?.aim?.trim(), `${id} has no goal aim`);
    assert.ok(sheet.goal?.done?.trim(), `${id} has no way to tell it is done`);
  }
});

test('every new persona loads real prose, not a missing-file stand-in', () => {
  for (const [id, sheet] of Object.entries(NEW_CAST)) {
    assert.ok(sheet.persona.length > 200, `${id}'s persona looks too short to be real`);
    assert.match(sheet.persona, /^You are/, `${id}'s persona should open in the established voice`);
  }
});
