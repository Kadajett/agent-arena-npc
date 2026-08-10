/**
 * Nerys the mage, Ash the sentient monster, and the two wandering knights.
 *
 * Same spirit as cast.test.mjs: this does not re-test the harness, it checks
 * the sheets are wired the way this batch was supposed to be wired. Nerys and
 * Ash get the full capability set, not a cut-down one; the knights run on
 * Routine rather than Autonomous and never call the model to decide where to
 * walk; and every patrol stop is a real place from world.ts, not one made up
 * for this file.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { CAPABILITIES } from '../dist/harness/actions.js';
import { Routine, Autonomous } from '../dist/harness/behavior.js';
import { PLACES, TOWN } from '../dist/harness/world.js';
import { guy } from '../dist/characters/guy.js';
import { barnaby } from '../dist/characters/barnaby.js';
import { wanderer } from '../dist/characters/wanderer.js';
import { tansy } from '../dist/characters/tansy.js';
import { hollis } from '../dist/characters/hollis.js';
import { marren } from '../dist/characters/marren.js';
import { cutter } from '../dist/characters/cutter.js';
import { nerys } from '../dist/characters/nerys.js';
import { ash } from '../dist/characters/ash.js';
import { doran } from '../dist/characters/doran.js';
import { aveline } from '../dist/characters/aveline.js';

const PLAYER_AGENTS = { nerys, ash };
const KNIGHTS = { doran, aveline };
const WHOLE_CAST = {
  guy,
  barnaby,
  wanderer,
  tansy,
  hollis,
  marren,
  cutter,
  ...PLAYER_AGENTS,
  ...KNIGHTS
};

test('the whole cast, all ten characters, still has unique ids and player names', () => {
  const ids = Object.values(WHOLE_CAST).map((sheet) => sheet.id);
  const names = Object.values(WHOLE_CAST).map((sheet) => sheet.playerName);
  assert.equal(new Set(ids).size, ids.length, 'duplicate character id');
  assert.equal(new Set(names).size, names.length, 'duplicate player name');
});

test('Nerys and Ash carry the full capability list, not a reduced one', () => {
  for (const [id, sheet] of Object.entries(PLAYER_AGENTS)) {
    assert.deepEqual(
      new Set(sheet.capabilities),
      new Set(CAPABILITIES),
      `${id} should have every capability the harness knows about, same as Guy`
    );
  }
  // The claim only means something if it actually matches Guy's own set.
  assert.deepEqual(new Set(nerys.capabilities), new Set(guy.capabilities));
});

test('Nerys and Ash run Autonomous - they decide for themselves, same as Guy', () => {
  for (const sheet of Object.values(PLAYER_AGENTS)) {
    const behavior = sheet.behavior({});
    assert.equal(behavior.kind, 'autonomous');
    assert.ok(behavior instanceof Autonomous);
  }
});

test('the knights walk their own rounds now, with the round pinned instead of hardcoded', () => {
  // They ran Routine when a model call per step was money the free tier did
  // not have. Agentic turns replaced the whole intent machinery, so the
  // round became pinned prose the character follows itself; Routine remains
  // in behavior.ts for anything that ever genuinely should not think.
  for (const [id, sheet] of Object.entries(KNIGHTS)) {
    const behavior = sheet.behavior();
    assert.equal(behavior.kind, 'autonomous', `${id} decides its own steps now`);
    assert.ok(!(behavior instanceof Routine), `${id} is not on rails any more`);
  }
});

test('the knights have no goal seed - the round is the whole job, same as Barnaby', () => {
  for (const [id, sheet] of Object.entries(KNIGHTS)) {
    assert.equal(sheet.goal, undefined, `${id} should not have a goal to work toward`);
  }
});

test('the knights are not given purpose or money - duty, not ambition or a wage', () => {
  for (const sheet of Object.values(KNIGHTS)) {
    assert.ok(!sheet.capabilities.includes('purpose'));
    assert.ok(!sheet.capabilities.includes('money'));
  }
});

/**
 * The rounds moved from Routine steps into pinned prose when the knights
 * became agentic: the character walks its own beat now, and the beat is
 * pinned so no conversation can talk them off it. The invariants survive the
 * move: every stop is a real place from world.ts, both knights cover the
 * same ground, and they walk it in different orders.
 */
function stopsInOrder(pinned) {
  const realPlaces = Object.keys(PLACES[TOWN]);
  // Pinned blocks are wrapped prose: a place name may break across a line,
  // so whitespace is flattened before looking for names.
  const text = pinned.toLowerCase().replace(/\s+/g, ' ');
  return realPlaces
    .map((place) => ({ place, at: text.indexOf(place) }))
    .filter(({ at }) => at >= 0)
    .sort((a, b) => a.at - b.at)
    .map(({ place }) => place);
}

test('every patrol stop on both rounds is a real place from world.ts, pinned where talk cannot move it', () => {
  for (const [id, sheet] of Object.entries(KNIGHTS)) {
    assert.ok(sheet.pinned, `${id} must carry the round pinned`);
    const stops = stopsInOrder(sheet.pinned);
    assert.equal(stops.length, 6, `${id}'s round should name all six town places, found: ${stops.join(', ')}`);
  }
});

test('the two knights walk the same six stops in opposite order, not in lockstep', () => {
  const doranStops = stopsInOrder(doran.pinned);
  const avelineStops = stopsInOrder(aveline.pinned);
  assert.deepEqual(new Set(doranStops), new Set(avelineStops), 'both should cover the same ground');
  assert.notDeepEqual(doranStops, avelineStops, 'walking the identical order is one guard, not two');
});

// Not "openrouter/free". A model string is provider first, then the provider's
// own name for the model, and OpenRouter's free router is called
// "openrouter/free", so that string asks the openrouter provider for a model
// named "free". It answers 502, checked against the live endpoint. The free
// router is still the fallback under everybody; it is just not what anybody
// reaches for first any more.
test('the four new characters from this batch default to the shared cheap model', () => {
  for (const [id, sheet] of Object.entries({ ...PLAYER_AGENTS, ...KNIGHTS })) {
    assert.equal(sheet.model, 'openrouter/openai/gpt-oss-120b', `${id} should default to the shared cheap model`);
  }
});

test('Ash and Nerys are spread out, not standing next to Guy and Barnaby', () => {
  assert.equal(ash.homeScene, 'arena-dungeon');
  assert.notEqual(ash.homeScene, TOWN);
});

test('every new persona in this batch loads real prose in the established voice', () => {
  for (const [id, sheet] of Object.entries({ ...PLAYER_AGENTS, ...KNIGHTS })) {
    assert.ok(sheet.persona.length > 200, `${id}'s persona looks too short to be real`);
    assert.match(sheet.persona, /^You are/, `${id}'s persona should open in the established voice`);
  }
});
