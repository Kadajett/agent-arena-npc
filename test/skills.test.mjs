/**
 * A mage's costume does not make it cast.
 *
 * Worth setting down because everyone assumed otherwise, including me, right
 * up until it was checked against the client. Every class-path spritesheet in
 * this world produces exactly four animations and all four are walking. There
 * is no attack frame on any of them. Swings and casts come from a separate set
 * of effects keyed by SKILL, so a mage in mage robes swinging the default
 * attack looks identical to a swordsman doing it.
 *
 * The skills are real rows: attackBullet, attackShort, fireball, heal, granted
 * per class path. Warlocks and sorcerers have fireball. A swordsman genuinely
 * does not. So a character can only use what it actually has, and asking for
 * anything else should be a plain no rather than a silent nothing.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';

import { Actions, IntentSchema } from '../dist/harness/actions.js';

const ASHLING = {
  label: 'Ashling',
  kind: 'enemy',
  objectIndex: 'arena-volcano-respawn-area_14_0',
  tileX: 10,
  tileY: 10
};

function arenaSeeing(objects) {
  const calls = [];
  return {
    calls,
    async call(tool, args) {
      calls.push({ tool, args });
      if (tool === 'arena_observe') {
        return { scene: 'arena-volcano', objects, ownPlayer: { state: { x: 0, y: 0 } } };
      }
      return {};
    }
  };
}

const mageWith = (arena, skills) =>
  new Actions(arena, 'agent-1', new Set(['fight']), undefined, undefined, skills);

test('a mage casting fireball sends the skill, not the default swing', async () => {
  const arena = arenaSeeing([ASHLING]);
  const actions = mageWith(arena, ['attackBullet', 'fireball', 'heal']);
  actions.notices([ASHLING]);
  const result = await actions.useSkill('fireball', 'Ashling');

  assert.equal(result.ok, true);
  assert.match(result.note, /used fireball on Ashling/);
  const sent = arena.calls.find((call) => call.tool === 'arena_use_action');
  assert.ok(sent, 'it goes out as a named action, which is what plays a cast');
  assert.equal(sent.args.action_type, 'fireball');
  assert.equal(
    sent.args.target_object_index,
    ASHLING.objectIndex,
    'targeted by objectIndex, same as a basic attack'
  );
  assert.ok(
    !arena.calls.some((call) => call.tool === 'arena_basic_attack'),
    'and never falls back to the generic swing'
  );
});

test('a swordsman asking for fireball is told no, not left wondering', async () => {
  const arena = arenaSeeing([ASHLING]);
  const actions = mageWith(arena, ['attackShort', 'heal']);
  actions.notices([ASHLING]);
  const result = await actions.useSkill('fireball', 'Ashling');

  assert.equal(result.ok, false);
  assert.match(result.note, /not something this character can do/);
  assert.match(result.note, /attackShort, heal/, 'and is told what it does have');
  assert.ok(!arena.calls.some((call) => call.tool === 'arena_use_action'));
});

test('a character with no skills at all says so plainly', async () => {
  const actions = mageWith(arenaSeeing([ASHLING]), []);
  const result = await actions.useSkill('fireball', 'Ashling');
  assert.equal(result.ok, false);
  assert.match(result.note, /no skills of its own/);
});

test('casting at nothing that is here is a no rather than a swing at air', async () => {
  const arena = arenaSeeing([]);
  const actions = mageWith(arena, ['fireball']);
  actions.notices([]);
  const result = await actions.useSkill('fireball', 'Ashling');
  assert.equal(result.ok, false);
  assert.match(result.note, /there is no "Ashling" here/);
});

test('somebody who does not fight cannot cast either', async () => {
  const actions = new Actions(arenaSeeing([ASHLING]), 'agent-1', new Set(['speak']), undefined, undefined, [
    'fireball'
  ]);
  const result = await actions.useSkill('fireball', 'Ashling');
  assert.equal(result.ok, false);
  assert.match(result.note, /does not fight/);
});

test('the intent carries a skill, and "cast" is read as meaning one', () => {
  const parsed = IntentSchema.safeParse({ action: 'cast', skill: 'fireball', target: 'Ashling' });
  assert.equal(parsed.success, true);
  assert.equal(parsed.data.action, 'use_skill', 'a near miss is read rather than thrown away');
  assert.equal(parsed.data.skill, 'fireball');
});
