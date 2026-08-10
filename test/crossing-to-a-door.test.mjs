/**
 * Getting to a door that is a long way off.
 *
 * When the gateway says a door is fine but too far, the character crosses the
 * room and tries again. That needs somewhere to stand beside the door, and the
 * eight tiles touching a doorway are the obvious candidates and often the worst
 * ones: half are the wall the door is set into, and in a narrow passage the
 * rest can be change points themselves.
 *
 * When none of them answered, the character used to be told "there is no way
 * through to the door" - a claim about the world, which it believes and acts
 * on, produced by a probe that had only looked one tile out. It was wrong often
 * enough to strand somebody in the volcano.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';

import { Actions } from '../dist/harness/actions.js';

const TILE = 32;
const centre = (tile) => tile * TILE + TILE / 2;

/**
 * An arena where only the named pixel spots are reachable. Records every
 * check so a test can say how far out the search actually went.
 */
function arenaReaching(reachable) {
  const probed = [];
  const spots = new Set(reachable.map(([x, y]) => `${x},${y}`));
  return {
    probed,
    async call(tool, args) {
      if (tool === 'arena_check_path') {
        probed.push([args.x, args.y]);
        return { reachable: spots.has(`${args.x},${args.y}`) };
      }
      if (tool === 'arena_move_to') return {};
      if (tool === 'arena_observe') {
        return { position: { x: args?.x ?? 0, y: args?.y ?? 0 }, scene: 'arena-volcano' };
      }
      if (tool === 'arena_enter_door') return { entered: true, scene: 'arena-depths' };
      return {};
    }
  };
}

const actionsWith = (arena) => new Actions(arena, 'test-agent', new Set(['doors', 'walk']));

// A door at tile (10, 10), with the whole ring around it blocked and the only
// standable ground two tiles out - a doorway set into a wall, which is what a
// doorway is.
const DOOR = { column: 10, row: 10, leadsTo: 'arena-depths', label: 'the depths' };

test('a door whose neighbours are all wall is still reachable from two tiles out', async () => {
  const arena = arenaReaching([[centre(12), centre(12)]]);
  const actions = actionsWith(arena);
  const spot = await actions.adjacentTile(DOOR.column, DOOR.row, 2);
  assert.deepEqual(spot, { x: centre(12), y: centre(12) }, 'the second ring is searched');
});

test('the first ring is still preferred when something there answers', async () => {
  const arena = arenaReaching([
    [centre(10), centre(11)],
    [centre(12), centre(12)]
  ]);
  const actions = actionsWith(arena);
  const spot = await actions.adjacentTile(DOOR.column, DOOR.row, 2);
  assert.deepEqual(spot, { x: centre(10), y: centre(11) }, 'nearest first, not merely any');
});

test('one ring is still the default, so nothing else pays for the wider search', async () => {
  const arena = arenaReaching([[centre(12), centre(12)]]);
  const actions = actionsWith(arena);
  assert.equal(await actions.adjacentTile(DOOR.column, DOOR.row), null);
  assert.equal(arena.probed.length, 8, 'exactly the eight touching tiles');
});

test('when nothing answers, it says what it established and not what is true', async () => {
  const actions = actionsWith(arenaReaching([]));
  const result = await actions.crossToDoor(DOOR);
  assert.equal(result.ok, false);
  assert.match(
    result.note,
    /could not find anywhere to stand beside the door/,
    'a fact about the probe'
  );
  assert.doesNotMatch(
    result.note,
    /there is no way through/,
    'and never the old claim about the world, which stranded a character believing it'
  );
  assert.match(result.note, /too far to work out yet/, 'leaving the door worth trying again');
});
