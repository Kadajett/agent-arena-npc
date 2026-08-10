/**
 * Two characters agreeing to fight, wired from the character's side.
 *
 * The referee (overlay) decides who won when one of them drops; the gateway
 * pairs whoever queued on the same scene. What the harness adds is the part a
 * character actually touches: standing for a duel where it is, and being able
 * to aim at the person who answered. The safety line runs through the match,
 * not the capability: having 'duel' lets a character queue, but only an
 * active match naming a specific person lets it swing at one.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';

import { Actions, IntentSchema, CAPABILITIES } from '../dist/harness/actions.js';

const SCENE = 'arena-volcano';
const NERYS = { agentId: 'agent-nerys', playerName: 'Nerys' };
const ASH = { agentId: 'agent-ash', playerName: 'Ash' };

function arenaWhere(replies = {}) {
  const calls = [];
  return {
    calls,
    async call(tool, args) {
      calls.push({ tool, args });
      if (tool === 'arena_observe') return { scene: SCENE, objects: [], ownPlayer: { state: { x: 0, y: 0 } } };
      if (tool in replies) {
        const reply = replies[tool];
        return 'function' === typeof reply ? reply(args) : reply;
      }
      return {};
    }
  };
}

const duellist = (arena, capabilities = ['fight', 'duel']) =>
  new Actions(arena, NERYS.agentId, new Set(capabilities), undefined, undefined, ['fireball']);

const paired = {
  id: '7d0e8a52-1d2b-4f6a-9b3c-0e5d4c3b2a10',
  status: 'active',
  participants: [
    { agentId: NERYS.agentId, playerName: NERYS.playerName },
    { agentId: ASH.agentId, playerName: ASH.playerName }
  ]
};

test('queueing sends the scene the character is standing in, never trusting the model', async () => {
  const arena = arenaWhere({ arena_queue_match: { status: 'queued' } });
  const result = await duellist(arena).duelQueue(SCENE, 'Ash');
  assert.equal(result.ok, true);
  assert.match(result.note, /waiting at the volcano/);
  const sent = arena.calls.find((call) => call.tool === 'arena_queue_match');
  assert.equal(sent.args.scene_name, SCENE, 'a forgotten scene would default to the home bedroom');
});

test('being paired remembers the opponent, and says so when it was not who they hoped', async () => {
  const arena = arenaWhere({ arena_queue_match: paired });
  const actions = duellist(arena);
  const result = await actions.duelQueue(SCENE, 'Guy');
  assert.equal(result.ok, true);
  assert.match(result.note, /a duel with Ash/);
  assert.match(result.note, /you hoped for Guy/, 'the pairing is anonymous and honesty beats pretending');
});

test('without a match, a duellist cannot aim at a person at all', async () => {
  // The capability is not the gate. A character with duel and fight, not in
  // any match, swinging at somebody by name, gets the same refusal as ever.
  const arena = arenaWhere({});
  const actions = duellist(arena);
  actions.meets([{ playerName: 'Ash', sessionId: 's-ash', playerId: 20 }]);
  actions.notices([]);
  const result = await actions.attack('Ash');
  assert.equal(result.ok, false);
  assert.ok(!arena.calls.some((call) => call.tool === 'arena_basic_attack'));
});

test('in a match, the registered opponent resolves as a player target', async () => {
  const arena = arenaWhere({ arena_queue_match: paired });
  const actions = duellist(arena);
  await actions.duelQueue(SCENE, 'Ash');
  actions.meets([{ playerName: 'Ash', sessionId: 's-ash', playerId: 20 }]);
  actions.notices([]);
  const result = await actions.attack('Ash');
  assert.equal(result.ok, true);
  const sent = arena.calls.find((call) => call.tool === 'arena_basic_attack');
  assert.equal(sent.args.target_session_id, 's-ash');
  assert.equal(sent.args.target_player_id, 20);
  assert.equal(sent.args.target_object_index, undefined, 'a person is not an object');
});

test('a bystander with a different name never resolves, match or no match', async () => {
  const arena = arenaWhere({ arena_queue_match: paired });
  const actions = duellist(arena);
  await actions.duelQueue(SCENE, 'Ash');
  actions.meets([
    { playerName: 'Ash', sessionId: 's-ash', playerId: 20 },
    { playerName: 'Barnaby', sessionId: 's-barn', playerId: 3 }
  ]);
  actions.notices([]);
  const result = await actions.attack('Barnaby');
  assert.equal(result.ok, false);
  assert.ok(!arena.calls.some((call) => call.tool === 'arena_basic_attack'));
});

test('an opponent who is not in the room cannot be hit from here', async () => {
  const arena = arenaWhere({ arena_queue_match: paired });
  const actions = duellist(arena);
  await actions.duelQueue(SCENE, 'Ash');
  actions.meets([]);
  actions.notices([]);
  const result = await actions.attack('Ash');
  assert.equal(result.ok, false);
});

test('a real enemy by the same name still wins the lookup, so a duel never shadows danger', async () => {
  const arena = arenaWhere({ arena_queue_match: paired });
  const actions = duellist(arena);
  await actions.duelQueue(SCENE, 'Ash');
  actions.meets([{ playerName: 'Ash', sessionId: 's-ash', playerId: 20 }]);
  actions.notices([{ label: 'Ash', kind: 'enemy', objectIndex: 'vol_9_2', tileX: 1, tileY: 1 }]);
  await actions.attack('Ash');
  const sent = arena.calls.find((call) => call.tool === 'arena_basic_attack');
  assert.equal(sent.args.target_object_index, 'vol_9_2');
});

test('skills reach the opponent through the same gate', async () => {
  const arena = arenaWhere({ arena_queue_match: paired });
  const actions = duellist(arena);
  await actions.duelQueue(SCENE, 'Ash');
  actions.meets([{ playerName: 'Ash', sessionId: 's-ash', playerId: 20 }]);
  actions.notices([]);
  const result = await actions.useSkill('fireball', 'Ash');
  assert.equal(result.ok, true);
  const sent = arena.calls.find((call) => call.tool === 'arena_use_action');
  assert.equal(sent.args.action_type, 'fireball');
  assert.equal(sent.args.target_player_id, 20);
});

test('queueing twice checks whether the first duel finished rather than just refusing', async () => {
  let status = { status: 'active' };
  const arena = arenaWhere({
    arena_queue_match: paired,
    arena_match_status: () => status
  });
  const actions = duellist(arena);
  await actions.duelQueue(SCENE, 'Ash');
  const still = await actions.duelQueue(SCENE, 'Ash');
  assert.equal(still.ok, false);
  assert.match(still.note, /already in a duel with Ash/);

  status = { status: 'completed' };
  const done = await actions.duelQueue(SCENE, 'Ash');
  assert.equal(done.ok, true);
  assert.match(done.note, /over and decided/);
});

test('somebody without the duel capability cannot queue however hard they fight', async () => {
  const arena = arenaWhere({});
  const fighter = new Actions(arena, 'agent-1', new Set(['fight']));
  const result = await fighter.duelQueue(SCENE, 'Ash');
  assert.equal(result.ok, false);
  assert.match(result.note, /does not duel/);
  assert.ok(!arena.calls.some((call) => call.tool === 'arena_queue_match'));
});

test('"challenge" and "duel" are read as meaning this', () => {
  for (const said of ['duel', 'challenge']) {
    const parsed = IntentSchema.safeParse({ action: said, target: 'Ash' });
    assert.equal(parsed.success, true, said);
    assert.equal(parsed.data.action, 'duel_queue', said);
  }
});

test('duel is a real capability, so a sheet can grant or withhold it', () => {
  assert.ok(CAPABILITIES.includes('duel'));
});
