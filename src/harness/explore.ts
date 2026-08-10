/**
 * Finding your way around a room nobody has surveyed.
 *
 * A character knows its own town by heart (see world.ts) and knows nothing
 * else. That is the point: a world where everyone can already name every room
 * has nothing worth telling anyone. So everywhere else, a character works with
 * what it can actually perceive - the walls and open ground around it, and the
 * doorways it can see - and finds out the rest by walking.
 *
 * Two pieces here. `Perception` is what the character can see from where it is
 * standing, cached per room because walls do not move. `Explorer` picks
 * somewhere in the room it has not been yet and confirms the route exists
 * before setting off, so exploring never means walking into a wall for a
 * minute.
 *
 * Nothing in here knows anything about this particular game's maps, which is
 * the property that matters: drop a character into a room that was generated
 * yesterday and it explores that too.
 */

import { ArenaClient } from './arena.js';

export type SeenDoor = {
  x: number;
  y: number;
  row: number;
  column: number;
  leadsTo: string | null;
  locked: boolean;
  lockKnown: boolean;
};

export type RoomView = {
  scene: string;
  doors: SeenDoor[];
  map: string;
  widthTiles: number;
  heightTiles: number;
};

const TILE = 32;
/**
 * How far afield to try, in tiles, nearest first.
 *
 * These start short on purpose. An earlier version began at six tiles, which
 * is wider than the upstairs of the inn: every candidate landed inside a wall,
 * every path check failed, and the character stood on the landing reporting
 * that there was nowhere to go. Interiors are small. Corridors are one tile
 * wide.
 */
const RANGES = [2, 3, 5, 8, 12, 17];
/** Eight ways to look, as tile offsets. */
const BEARINGS: Array<[number, number, string]> = [
  [0, -1, 'north'],
  [1, -1, 'north-east'],
  [1, 0, 'east'],
  [1, 1, 'south-east'],
  [0, 1, 'south'],
  [-1, 1, 'south-west'],
  [-1, 0, 'west'],
  [-1, -1, 'north-west']
];
/** Bound the pathfinding calls one decision may cost. */
const MAX_PROBES = 14;
/** How big a patch counts as "been there", in tiles. */
const CELL = 3;
/**
 * How far a character can see, in tiles. Sixteen gives a 33x33 window: most of
 * a street and the buildings down both sides of it, or a whole room and its
 * doors. Small enough to still be a view from where it stands rather than a
 * survey of the map, and these models are cheap enough that the extra is not
 * worth being stingy about.
 */
const SIGHT = 16;

/**
 * What a character can see from where it is standing.
 *
 * Deliberately a local window, and deliberately re-read every tick rather than
 * cached per room. A character walks; what is in front of it changes as it
 * goes, and a view fetched once on arrival is a photograph of the doorway it
 * came in by. Caching it also hid the far side of every room the character had
 * not crossed yet, which is the opposite of the point.
 */
export class Perception {
  private lastSeen: RoomView | null = null;

  async look(arena: ArenaClient, agentId: string, scene: string): Promise<RoomView | null> {
    try {
      const rendered = await arena.call('arena_render_map', {
        agent_id: agentId,
        level: 'room',
        radius: SIGHT
      });
      if (!rendered?.gridAvailable) {
        return null;
      }
      const view: RoomView = {
        scene,
        doors: dedupeDoors(rendered.doors ?? []),
        map: String(rendered.map ?? ''),
        widthTiles: Number(rendered.sceneSize?.widthTiles ?? 0),
        heightTiles: Number(rendered.sceneSize?.heightTiles ?? 0)
      };
      this.lastSeen = view;
      return view;
    } catch {
      // One unanswered look is not blindness. Carry on with what it last saw
      // rather than forgetting the room exists.
      return this.lastSeen?.scene === scene ? this.lastSeen : null;
    }
  }
}

/**
 * Doors come a tile at a time, so a two-tile-wide gateway arrives as two
 * doors to the same place. A person sees one door.
 */
export function dedupeDoors(raw: SeenDoor[]): SeenDoor[] {
  const kept: SeenDoor[] = [];
  for (const door of raw) {
    const near = kept.find(
      (other) =>
        other.leadsTo === door.leadsTo
        && Math.abs(other.row - door.row) <= 1
        && Math.abs(other.column - door.column) <= 1
    );
    if (!near) {
      kept.push(door);
    }
  }
  return kept;
}

/** Where a door goes, said the way a person would say it. */
export function describeDoors(view: RoomView | null, plainName: (scene: string) => string): string {
  const doors = view?.doors ?? [];
  if (doors.length === 0) {
    return '';
  }
  return doors
    .map((door, index) => {
      const where = door.leadsTo ? plainName(door.leadsTo) : 'somewhere';
      const lock = door.locked ? ', locked' : '';
      return `- door ${index + 1}: leads to ${where}${lock}`;
    })
    .join('\n');
}

/** Keeps track of where a character has already been, room by room. */
export class Explorer {
  private readonly beenTo = new Map<string, Set<string>>();

  private cell(x: number, y: number): string {
    return `${Math.floor(x / (TILE * CELL))},${Math.floor(y / (TILE * CELL))}`;
  }

  /** Note that the character is standing here. */
  markHere(scene: string, x: number, y: number): void {
    const been = this.beenTo.get(scene) ?? new Set<string>();
    been.add(this.cell(x, y));
    this.beenTo.set(scene, been);
  }

  hasBeenNear(scene: string, x: number, y: number): boolean {
    return this.beenTo.get(scene)?.has(this.cell(x, y)) ?? false;
  }

  /** How much of this room the character has stood in. */
  cornersKnown(scene: string): number {
    return this.beenTo.get(scene)?.size ?? 0;
  }

  /**
   * Somewhere in this room the character has not been, that it can actually
   * walk to. Tries the near ranges first so exploring reads as wandering
   * rather than teleporting across the room, and gives up rather than
   * spending the whole tick pathfinding.
   */
  async somewhereNew(
    arena: ArenaClient,
    agentId: string,
    scene: string,
    from: { x: number; y: number }
  ): Promise<{ x: number; y: number; bearing: string } | null> {
    // Nearest ring first, and the bearings rotated within each ring. Sorting
    // the whole lot together would spend the probe budget on far corners and
    // find nothing in a room the size of a landing.
    const tried: Array<{ x: number; y: number; bearing: string }> = [];
    for (const range of RANGES) {
      for (const [dx, dy, bearing] of shuffle([...BEARINGS])) {
        const x = from.x + dx * range * TILE;
        const y = from.y + dy * range * TILE;
        if (x < TILE || y < TILE) {
          continue;
        }
        tried.push({ x, y, bearing });
      }
    }
    // Somewhere new first; failing that, anywhere reachable, because a
    // character that has been everywhere should still be able to move.
    const fresh = tried.filter((spot) => !this.hasBeenNear(scene, spot.x, spot.y));
    const order = [...fresh, ...tried];

    let probes = 0;
    for (const spot of order) {
      if (probes >= MAX_PROBES) {
        break;
      }
      probes++;
      try {
        const path = await arena.call('arena_check_path', {
          agent_id: agentId,
          x: spot.x,
          y: spot.y
        });
        if (path?.reachable) {
          return spot;
        }
      } catch {
        // Treat an unanswerable probe as unreachable and try the next.
      }
    }
    return null;
  }
}

/**
 * Rotate the order rather than randomising it, so a character does not try
 * north every single time but two runs of the same situation still behave the
 * same. Reproducible wandering is much easier to debug than random wandering,
 * and from the outside they look identical.
 */
let rotation = 0;
function shuffle<T>(items: T[]): T[] {
  if (items.length < 2) {
    return items;
  }
  rotation = (rotation + 3) % items.length;
  return [...items.slice(rotation), ...items.slice(0, rotation)];
}
