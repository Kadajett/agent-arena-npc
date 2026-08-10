/**
 * The one action that does not have to obey the map.
 *
 * Guy spent days in the volcano because every route out was refused and nothing
 * he had could tell the difference between "try that again" and "there is no
 * way out of here". This is that difference, made available once, and it is
 * built to give away as little as it can: it names no coordinate, the world
 * lands the character on the inn's own return point, and the wait afterwards is
 * long enough that walking is always the cheaper option.
 *
 * That wait is the whole safeguard. Without it every locked door in the world
 * becomes a free trip home.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';

import { Actions, IntentSchema } from '../dist/harness/actions.js';

function arena(reply = { moved: true, from: 'arena-volcano', scene: 'reldens-house-1' }) {
  const calls = [];
  return {
    calls,
    async call(tool, args) {
      calls.push({ tool, args });
      if (tool === 'arena_unstick') return reply;
      if (tool === 'arena_observe') return { scene: 'arena-volcano', objects: [], ownPlayer: {} };
      return {};
    }
  };
}

const walker = (client) => new Actions(client, 'agent-1', new Set(['walk', 'doors']));

test('a character that has run out of ways out gets back to the inn', async () => {
  const client = arena();
  const result = await walker(client).giveUpAndWalkBack();

  assert.equal(result.ok, true);
  assert.match(result.note, /walked back to the inn/);
  const sent = client.calls.find((call) => call.tool === 'arena_unstick');
  assert.ok(sent, 'the world does the moving, not the harness');
  assert.deepEqual(Object.keys(sent.args), ['agent_id'], 'and is told nothing about where to put it');
});

test('it cannot be done twice in a row, which is what stops it being a shortcut', async () => {
  const actions = walker(arena());
  assert.equal((await actions.giveUpAndWalkBack()).ok, true);

  const again = await actions.giveUpAndWalkBack();
  assert.equal(again.ok, false);
  assert.match(again.note, /too soon/);
  assert.match(again.note, /has to be walked out of/, 'and says what to do instead');
  assert.match(again.note, /\d+ more minute/, 'and how long is left, rather than just no');
});

test('a character walled in on its first tick does not serve a wait it never earned', async () => {
  // The clock starts at zero, not at construction. A character that comes up
  // already stuck would otherwise have to sit there for the full hour first.
  const result = await walker(arena()).giveUpAndWalkBack();
  assert.equal(result.ok, true);
});

test('being already at the inn costs nothing, since nothing moved', async () => {
  const actions = walker(arena({ moved: false, reason: 'ALREADY_AT_THE_INN' }));
  const first = await actions.giveUpAndWalkBack();
  assert.equal(first.ok, false);
  assert.match(first.note, /already at the inn/);

  // The hour is spent on having been moved. Nothing moved, so it is not spent.
  const client = arena();
  const second = new Actions(client, 'agent-1', new Set(['walk', 'doors']));
  assert.equal((await second.giveUpAndWalkBack()).ok, true);
});

test('somebody who never leaves where they are cannot give up on it', async () => {
  // Barnaby has no doors. An innkeeper who can warp himself out of his own inn
  // is not an innkeeper.
  const client = arena();
  const barnaby = new Actions(client, 'agent-1', new Set(['speak']));
  const result = await barnaby.giveUpAndWalkBack();
  assert.equal(result.ok, false);
  assert.match(result.note, /does not leave where it is/);
  assert.ok(!client.calls.some((call) => call.tool === 'arena_unstick'));
});

test('what it thought was in the room does not come with it', async () => {
  const actions = walker(arena());
  actions.notices([{ label: 'Ashling', kind: 'enemy', objectIndex: 'x', tileX: 1, tileY: 1 }]);
  await actions.giveUpAndWalkBack();
  const result = await actions.attack('Ashling');
  assert.equal(result.ok, false, 'the volcano is a room away now');
});

test('"stuck" is read as meaning this, since that is the word anybody would use', () => {
  for (const said of ['stuck', 'unstick', 'give up']) {
    const parsed = IntentSchema.safeParse({ action: said });
    assert.equal(parsed.success, true, said);
    assert.equal(parsed.data.action, 'give_up_and_walk_back', said);
  }
});

test('it is offered last, and worded as an admission rather than an option', () => {
  const offered = walker(arena()).describe('arena-volcano');
  assert.match(offered, /give_up_and_walk_back/);
  assert.match(offered, /only if you have genuinely tried/);
  assert.ok(
    offered.indexOf('give_up_and_walk_back') > offered.indexOf('explore'),
    'listed after the ways of actually walking, not next to them'
  );
});
