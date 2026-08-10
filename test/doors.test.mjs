/**
 * Naming a door the way a person would, not the way the map spells it.
 *
 * A door only ever has a label built from where it leads (see doorLabel() /
 * plainSceneName()), so a character that calls it "the inn door" or "the
 * tavern door" is describing it, not quoting it. Matching has to meet that
 * halfway - strip the words that carry nothing ("the", "door"), fall back to
 * whatever the name and the label actually share - and when it still cannot
 * settle on one, it has to say which candidates it is choosing between or
 * what is actually there, never just that it "could not tell".
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

const INN_DOOR = { x: 10, y: 10, row: 0, column: 0, leadsTo: 'reldens-house-1', locked: false, lockKnown: true };
const FOREST_DOOR = { x: 20, y: 10, row: 0, column: 1, leadsTo: 'reldens-forest', locked: false, lockKnown: true };
const TAVERN_DOOR = { x: 30, y: 10, row: 0, column: 2, leadsTo: 'reldens-tavern', locked: false, lockKnown: true };
const TAVERN_CELLAR_DOOR = {
  x: 40, y: 10, row: 0, column: 3, leadsTo: 'reldens-tavern-cellar', locked: false, lockKnown: true
};
const NORTH_TAVERN_DOOR = {
  x: 50, y: 10, row: 0, column: 4, leadsTo: 'reldens-north-tavern', locked: false, lockKnown: true
};
const SOUTH_TAVERN_DOOR = {
  x: 60, y: 10, row: 0, column: 5, leadsTo: 'reldens-south-tavern', locked: false, lockKnown: true
};

function actionsAt(doors, replies = []) {
  const arena = new FakeArena(replies);
  const actions = new Actions(arena, 'agent-1', new Set(['doors', 'speak']));
  actions.sees({ scene: 'reldens-town', doors, map: '', widthTiles: 10, heightTiles: 10 });
  return { arena, actions };
}

test('a door named loosely, with "the" and "door" wrapped around it, is still found', async () => {
  const { arena, actions } = actionsAt(
    [INN_DOOR, FOREST_DOOR],
    [{ entered: true, scene: 'reldens-house-1' }]
  );
  // The real case from the logs: "the inn door" against a door whose only
  // label is "Barnaby's inn".
  const result = await actions.useDoor('reldens-town', 'the inn door');
  assert.equal(result.ok, true);
  assert.match(result.note, /Barnaby's inn/);
  assert.deepEqual(arena.calls[0].args, { agent_id: 'agent-1', x: INN_DOOR.x, y: INN_DOOR.y });
});

test('with only one way out, whatever it is called reaches it', async () => {
  const { actions } = actionsAt([FOREST_DOOR], [{ entered: true, scene: 'reldens-forest' }]);
  const result = await actions.useDoor('reldens-town', 'the way out');
  assert.equal(result.ok, true);
});

test('two doors that could both fit the name are named back, not just refused', async () => {
  const { arena, actions } = actionsAt([NORTH_TAVERN_DOOR, SOUTH_TAVERN_DOOR]);
  const result = await actions.useDoor('reldens-town', 'the tavern door');
  assert.equal(result.ok, false);
  assert.match(result.note, /"north tavern"/);
  assert.match(result.note, /"south tavern"/);
  assert.equal(arena.calls.length, 0, 'never guessed and went through one of them');
});

test('a name that matches nothing here lists what actually is here', async () => {
  const { arena, actions } = actionsAt([INN_DOOR, FOREST_DOOR]);
  const result = await actions.useDoor('reldens-town', 'the cellar stairs');
  assert.equal(result.ok, false);
  assert.match(result.note, /no door here it would call "the cellar stairs"/);
  assert.match(result.note, /"Barnaby's inn"/);
  assert.match(result.note, /"the woods"/);
  assert.equal(arena.calls.length, 0);
});

test('saying nothing at all with more than one door names the choices rather than guessing', async () => {
  const { arena, actions } = actionsAt([INN_DOOR, FOREST_DOOR]);
  const result = await actions.useDoor('reldens-town');
  assert.equal(result.ok, false);
  assert.match(result.note, /"Barnaby's inn"/);
  assert.match(result.note, /"the woods"/);
  assert.equal(arena.calls.length, 0);
});

test('an exact label still wins outright over any looser match', async () => {
  const { actions } = actionsAt([TAVERN_DOOR, TAVERN_CELLAR_DOOR], [{ entered: true, scene: 'reldens-tavern' }]);
  const result = await actions.useDoor('reldens-town', 'tavern');
  assert.equal(result.ok, true);
  assert.match(result.note, /tavern/);
});

test('case does not matter any more than the wrapping words do', async () => {
  const { actions } = actionsAt([INN_DOOR, FOREST_DOOR], [{ entered: true, scene: 'reldens-house-1' }]);
  const result = await actions.useDoor('reldens-town', 'THE INN');
  assert.equal(result.ok, true);
});

test('a door is also found by the raw name a room reads as, not only its pretty one', async () => {
  // plainSceneName('reldens-forest') overrides the room to "the woods" -
  // which is what doorLabel() shows and what listDoors() names it as - but a
  // character's own memory of standing in that same room is written with
  // rawSceneName() instead (see notePlace() in npc.ts), which never applies
  // that override and calls it "forest". Two doors, so this actually has to
  // pick FOREST_DOOR out by name rather than being the only door in the room.
  const { actions } = actionsAt([INN_DOOR, FOREST_DOOR], [{ entered: true, scene: 'reldens-forest' }]);
  const result = await actions.useDoor('reldens-town', 'forest');
  assert.equal(result.ok, true);
  assert.match(result.note, /the woods/, 'still reported by its pretty name; only the matching changed');
});

test('a door to a room with no pretty override is found by the exact string a character stored for it', async () => {
  // A real fragment of production memory: a character had "bots forest" and
  // "bots forest house 01 n0" written down for a room with no hardcoded
  // pretty name at all - rawSceneName() is the only name it was ever going
  // to have, on both sides.
  const BOTS_FOREST_DOOR = {
    x: 70, y: 10, row: 0, column: 6, leadsTo: 'reldens-bots-forest', locked: false, lockKnown: true
  };
  const { actions } = actionsAt(
    [INN_DOOR, BOTS_FOREST_DOOR],
    [{ entered: true, scene: 'reldens-bots-forest' }]
  );
  const result = await actions.useDoor('reldens-town', 'bots forest');
  assert.equal(result.ok, true);
});

test('a door too far off is crossed to, beside it rather than onto it, and then tried again', async () => {
  // The real production shape: the gateway ran out of its own budget getting
  // there and says so plainly, rather than the client aborting mid-call the
  // way it used to. That is not a refusal, it is a reason to get closer. The
  // walk aims at a tile BESIDE the door: a change point is walked into, not
  // stood on, so aiming at the door itself either fails or trips the
  // transition halfway through the leg and leaves the retry in the wrong room.
  const { arena, actions } = actionsAt(
    [FOREST_DOOR],
    [
      {
        entered: false,
        reason: 'DOOR_TOO_FAR',
        message: 'Set off for that door but ran out of time before reaching it. '
          + 'It is a long way across this room; get closer first, then try the door.'
      },
      { reachable: true }, // arena_check_path, for the tile beside the door
      {}, // arena_move_to
      { ownPlayer: { state: { x: FOREST_DOOR.x, y: FOREST_DOOR.y + 32 } } }, // arena_observe: arrived beside it
      { entered: true, scene: 'reldens-forest' } // arena_enter_door, tried again
    ]
  );
  const result = await actions.useDoor('reldens-town', 'the woods');
  assert.equal(result.ok, true);
  assert.match(result.note, /the woods/);
  assert.deepEqual(
    arena.calls.map((call) => call.name),
    ['arena_enter_door', 'arena_check_path', 'arena_move_to', 'arena_observe', 'arena_enter_door'],
    'crossed to a tile beside the door before trying it again'
  );
  const moved = arena.calls.find((call) => call.name === 'arena_move_to');
  assert.notDeepEqual(
    { x: moved.args.x, y: moved.args.y },
    { x: FOREST_DOOR.x, y: FOREST_DOOR.y },
    'walked beside the door, not onto it'
  );
});

test('a crossing that does not arrive ends the turn instead of stacking another door attempt', async () => {
  // Guy spent three minutes inside one use_door in the volcano: forty five
  // seconds of walking and two forty second door budgets, during which he
  // could not hear anybody or look at anything. Getting most of the way is
  // real progress, and the next tick picks the door up from close enough.
  const { arena, actions } = actionsAt(
    [FOREST_DOOR],
    [
      { entered: false, reason: 'DOOR_TOO_FAR', message: 'ran out of time' },
      { reachable: true }, // arena_check_path
      {}, // arena_move_to
      { ownPlayer: { state: { x: 0, y: 0 } } }, // arena_observe: nowhere near it yet
      { ownPlayer: { state: { x: 0, y: 0 } } },
      { ownPlayer: { state: { x: 0, y: 0 } } },
      { ownPlayer: { state: { x: 0, y: 0 } } }
    ]
  );
  const result = await actions.useDoor('reldens-town', 'the woods');
  assert.equal(result.ok, true, 'crossing most of a room is progress, not a failed turn');
  assert.match(result.note, /still ahead/);
  assert.equal(
    arena.calls.filter((call) => call.name === 'arena_enter_door').length,
    1,
    'did not chain a second door attempt onto a crossing that had not arrived'
  );
});

test('a retry that still fails reports the retry, not the stale "too far" message', async () => {
  const { actions } = actionsAt(
    [FOREST_DOOR],
    [
      { entered: false, reason: 'DOOR_TOO_FAR', message: 'ran out of time getting there' },
      { reachable: true }, // arena_check_path
      {}, // arena_move_to
      { ownPlayer: { state: { x: FOREST_DOOR.x, y: FOREST_DOOR.y + 32 } } }, // arena_observe
      { entered: false, reason: 'DOOR_DID_NOT_OPEN', message: 'Something may be in the way.' }
    ]
  );
  const result = await actions.useDoor('reldens-town', 'the woods');
  assert.equal(result.ok, false);
  assert.match(result.note, /Something may be in the way/);
  assert.doesNotMatch(result.note, /ran out of time/);
});

test('only "too far" gets a second try - a door that just did not open is not retried', async () => {
  const { arena, actions } = actionsAt(
    [FOREST_DOOR],
    [{ entered: false, reason: 'DOOR_UNREACHABLE', message: 'There is no walkable tile next to that door.' }]
  );
  const result = await actions.useDoor('reldens-town', 'the woods');
  assert.equal(result.ok, false);
  assert.match(result.note, /no walkable tile/);
  assert.equal(arena.calls.length, 1, 'no approach and no retry - getting closer would not help this one');
});

test('a doorway two tiles wide is one door, not a choice between "town" and "town"', async () => {
  // Straight from production: the Wanderer asked for "town" and was asked
  // back which of "town" or "town" it meant, which is not a question. A
  // doorway more than one tile across is one change point per tile, so it
  // arrives here as several doors carrying the same label.
  const LEFT_HALF = { x: 10, y: 90, row: 9, column: 1, leadsTo: 'reldens-town', locked: false, lockKnown: true };
  const RIGHT_HALF = { x: 42, y: 90, row: 9, column: 2, leadsTo: 'reldens-town', locked: false, lockKnown: true };
  const { arena, actions } = actionsAt([LEFT_HALF, RIGHT_HALF, FOREST_DOOR], [{ entered: true, scene: 'reldens-town' }]);
  const result = await actions.perform({ action: 'use_door', place: 'town' }, 'reldens-house-2');
  assert.equal(result.ok, true);
  assert.match(result.note, /went through into town/);
  assert.equal(arena.calls[0].name, 'arena_enter_door');
});

test('two doors that really do lead somewhere different are still a genuine choice', async () => {
  // The same-label shortcut must not swallow a real ambiguity: these two both
  // answer to "tavern" and go to different rooms, so the character has to be
  // told what it is choosing between.
  const { actions } = actionsAt([NORTH_TAVERN_DOOR, SOUTH_TAVERN_DOOR]);
  const result = await actions.perform({ action: 'use_door', place: 'tavern' }, 'reldens-town');
  assert.equal(result.ok, false);
  assert.match(result.note, /could not tell which door/);
});
