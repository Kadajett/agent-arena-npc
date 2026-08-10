/**
 * Reaching somebody standing nearby, outside a character's own town.
 *
 * placesIn() only ever has entries for home turf (see world.ts's own module
 * doc) - deliberately, so a character does not arrive omniscient about a room
 * it has never stood in. But that means everywhere else, "walk" had nothing
 * at all to send a character to except an unvisited patch of ground: eleven
 * NPCs went live across the arena regions and none of them could be walked
 * to by name, so `talk_to` kept failing as too far away with nowhere for a
 * character to go to close the distance. This is the other half of that fix:
 * walk() now tries a person before it gives up and wanders off exploring.
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

const OLD_FERRO = {
  objectId: 11,
  objectIndex: 'npcs4',
  label: 'Old Ferro',
  kind: 'npc',
  interactable: true,
  distanceFromSelf: 300,
  tileX: 10,
  tileY: 10
};

const NORTH_GUARD = {
  objectId: 12,
  objectIndex: 'npcs5',
  label: 'North Guard',
  kind: 'npc',
  interactable: true,
  distanceFromSelf: 200,
  tileX: 4,
  tileY: 4
};

const SOUTH_GUARD = {
  objectId: 13,
  objectIndex: 'npcs6',
  label: 'South Guard',
  kind: 'npc',
  interactable: true,
  distanceFromSelf: 220,
  tileX: 6,
  tileY: 6
};

function actionsWith(replies = []) {
  const arena = new FakeArena(replies);
  const actions = new Actions(arena, 'agent-1', new Set(['walk', 'talk_to_folk']));
  return { arena, actions };
}

test('somebody standing here is reached by name, not exploring at random', async () => {
  const { arena, actions } = actionsWith([
    { reachable: true }, // arena_check_path, first tile tried beside Old Ferro
    {}, // arena_move_to
    { ownPlayer: { state: { x: 336, y: 368 } } } // arena_observe, already there
  ]);
  actions.notices([OLD_FERRO]);
  // reldens-volcano is not home turf: placesIn() is empty, so this name can
  // only ever resolve to somebody standing here.
  const result = await actions.walk('old ferro', 'reldens-volcano');
  assert.equal(result.ok, true);
  assert.match(result.note, /walked over to Old Ferro/);
  assert.deepEqual(arena.calls[0], {
    name: 'arena_check_path',
    args: { agent_id: 'agent-1', x: 336, y: 368 } // tile (10, 11): one south of him
  });
  assert.deepEqual(arena.calls[1], {
    name: 'arena_move_to',
    args: { agent_id: 'agent-1', x: 336, y: 368 }
  });
});

test('a name loosely given still finds the one person it could mean', async () => {
  const { actions } = actionsWith([
    { reachable: true },
    {},
    { ownPlayer: { state: { x: 336, y: 368 } } }
  ]);
  actions.notices([OLD_FERRO]);
  const result = await actions.walk('the old sellsword Ferro', 'reldens-volcano');
  assert.equal(result.ok, true);
});

test('two people who could both fit the name are named back, not guessed', async () => {
  const { arena, actions } = actionsWith([]);
  actions.notices([NORTH_GUARD, SOUTH_GUARD]);
  const result = await actions.walk('the guard', 'reldens-volcano');
  assert.equal(result.ok, false);
  assert.match(result.note, /"North Guard"/);
  assert.match(result.note, /"South Guard"/);
  assert.equal(arena.calls.length, 0, 'never guessed and set off toward either one');
});

test('a name matching nobody, with somebody actually standing here, says who that is', async () => {
  const { arena, actions } = actionsWith([]);
  actions.notices([OLD_FERRO]);
  const result = await actions.walk('the blacksmith', 'reldens-volcano');
  assert.equal(result.ok, false);
  assert.match(result.note, /nobody here it would call "the blacksmith"/);
  assert.match(result.note, /"Old Ferro"/);
  assert.equal(arena.calls.length, 0);
});

test('an empty room falls through to exploring, same as an unfamiliar place always has', async () => {
  const { actions } = actionsWith([]); // no arena_observe reply queued at all
  actions.notices([]);
  // where() swallows the FakeArena error and reports "nowhere", which is
  // exactly what explore() does with nowhere to start from - the pre-existing
  // behaviour for a name nobody nearby matches, left untouched.
  const result = await actions.walk('the north path', 'reldens-volcano');
  assert.equal(result.ok, false);
  assert.match(result.note, /could not tell where it was standing/);
});

test('every side blocked is said plainly, not silently given up on', async () => {
  const { arena, actions } = actionsWith(Array(8).fill({ reachable: false }));
  actions.notices([OLD_FERRO]);
  const result = await actions.walk('Old Ferro', 'reldens-volcano');
  assert.equal(result.ok, false);
  assert.match(result.note, /could not find a way to stand next to Old Ferro/);
  assert.equal(arena.calls.length, 8, 'tried every side before giving up');
  assert.ok(arena.calls.every((call) => call.name === 'arena_check_path'));
});

test('walking to an enemy by the same name is not this path - attack is', async () => {
  const { arena, actions } = actionsWith([]);
  const wolf = { ...OLD_FERRO, label: 'a wolf', kind: 'enemy' };
  actions.notices([wolf]);
  const result = await actions.walk('a wolf', 'reldens-volcano');
  // Nobody (of the kind this looks at) matches, and nobody else is standing
  // here either, so this is the same "nowhere to go" as an empty room.
  assert.equal(result.ok, false);
  assert.doesNotMatch(result.note, /wolf/);
});

test('a heading is not somebody’s name, even with people standing here', async () => {
  // Production: Guy, in the volcano with two people in the room, asked to
  // walk south and was told there was nobody here called "south" - twice in
  // consecutive turns, with both their names read back at him. A bearing
  // belongs to explore(), which knows what to do with one.
  for (const heading of ['south', 'south-east', 'northwest', 'the north']) {
    const { arena, actions } = actionsWith([]);
    actions.notices([OLD_FERRO, NORTH_GUARD]);
    const result = await actions.perform({ action: 'walk', place: heading }, 'arena-volcano');
    assert.doesNotMatch(result.note ?? '', /nobody here it would call/, `"${heading}" was treated as a name`);
    assert.ok(
      arena.calls.every((call) => call.name !== 'arena_move_to'),
      `"${heading}" should not have walked to a person`
    );
  }
});
