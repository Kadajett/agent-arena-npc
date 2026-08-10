/**
 * Hitting something by name.
 *
 * attack() used to send {agent_id, target} straight to arena_basic_attack,
 * which the gateway's schema has never accepted - it wants target_object_index,
 * the layer_name+tile_index string, not a bare name. It also never told a
 * character it was allowed to fight at all: "fight" was a capability nothing
 * ever checked in describe(). Both are fixed here: an enemy is found by name
 * against what notices() was just told, the same way talk_to finds an NPC,
 * and its objectIndex - not its name - is what actually goes out on the wire.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Actions } from '../dist/harness/actions.js';

class FakeArena {
  constructor(replies = []) {
    this.replies = [...replies];
    this.calls = [];
  }

  async call(name, args) {
    this.calls.push({ name, args });
    if (this.replies.length === 0) {
      throw new Error(`FakeArena: no reply queued for ${name}`);
    }
    return this.replies.shift();
  }
}

const WOLF = {
  objectId: null,
  objectIndex: 'enemies7',
  label: 'a wolf',
  kind: 'enemy',
  interactable: false,
  distanceFromSelf: 30
};

const ALFRED = {
  objectId: 42,
  objectIndex: 'npcs12',
  label: 'Alfred',
  kind: 'npc',
  interactable: true,
  distanceFromSelf: 10
};

test('a character without "fight" cannot attack, whatever is standing there', async () => {
  const arena = new FakeArena();
  const actions = new Actions(arena, 'agent-1', new Set(['talk_to_folk']));
  actions.notices([WOLF]);
  const result = await actions.attack('a wolf');
  assert.equal(result.ok, false);
  assert.match(result.note, /does not fight/);
  assert.equal(arena.calls.length, 0);
});

test('attacking something not actually here fails by name', async () => {
  const arena = new FakeArena();
  const actions = new Actions(arena, 'agent-1', new Set(['fight']));
  actions.notices([ALFRED]);
  const result = await actions.attack('a wolf');
  assert.equal(result.ok, false);
  assert.match(result.note, /no "a wolf" here/);
});

test('an enemy standing by is hit by objectIndex, never by its name', async () => {
  const arena = new FakeArena([{ dealt: 4 }]);
  const actions = new Actions(arena, 'agent-1', new Set(['fight']));
  actions.notices([WOLF]);
  const result = await actions.attack('a wolf');
  assert.equal(result.ok, true);
  assert.deepEqual(arena.calls[0], {
    name: 'arena_basic_attack',
    args: { agent_id: 'agent-1', target_object_index: 'enemies7' }
  });
});

test('an NPC is not a valid attack target, even by the same lookup', async () => {
  const arena = new FakeArena();
  const actions = new Actions(arena, 'agent-1', new Set(['fight']));
  actions.notices([ALFRED]);
  const result = await actions.attack('Alfred');
  assert.equal(result.ok, false);
  assert.match(result.note, /no "Alfred" here/);
  assert.equal(arena.calls.length, 0, 'never sent an attack at an NPC');
});

test('describe() only offers "attack" once the character can fight and something is here to hit', () => {
  const withoutFight = new Actions(new FakeArena(), 'agent-1', new Set([]));
  withoutFight.notices([WOLF]);
  assert.ok(!withoutFight.describe('reldens-forest').includes('"attack"'));

  const canFightAlone = new Actions(new FakeArena(), 'agent-1', new Set(['fight']));
  canFightAlone.notices([]);
  assert.ok(!canFightAlone.describe('reldens-forest').includes('"attack"'), 'nothing here to hit');

  const readyToFight = new Actions(new FakeArena(), 'agent-1', new Set(['fight']));
  readyToFight.notices([WOLF]);
  assert.match(readyToFight.describe('reldens-forest'), /"attack": attack something here\. Needs: target, one of "a wolf"/);
});

test('perform() routes attack through the same intent shape as everything else', async () => {
  const arena = new FakeArena([{ dealt: 4 }]);
  const actions = new Actions(arena, 'agent-1', new Set(['fight']));
  actions.notices([WOLF]);
  const result = await actions.perform({ action: 'attack', target: 'a wolf' }, 'reldens-forest');
  assert.equal(result.ok, true);
  assert.match(result.note, /swung at a wolf/);
});
