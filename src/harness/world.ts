/**
 * The little a character is born knowing.
 *
 * This file used to be a gazetteer of the whole world, which quietly made
 * every character omniscient: they could name rooms they had never entered and
 * walk straight to landmarks nobody had shown them. That empties the world out.
 * If everyone already knows what is upstairs, nobody ever goes to look, and
 * nothing anybody says about it is worth hearing.
 *
 * So what is here is only home: the town these characters live in and the inn
 * they drink in, the way you know your own street. Everything past those doors
 * is found out by walking through them - see explore.ts for how a character
 * gets around a room it has never been in, and memory.ts for where what it
 * finds is kept.
 *
 * Doors are deliberately not listed. A character can see the doorways in
 * whatever room it is standing in, and where each one leads, because the
 * gateway reports them; that is reading the sign over the door. What is behind
 * it is not knowledge until somebody goes and looks.
 */

export const TOWN = process.env.ARENA_TOWN_SCENE ?? 'reldens-town';
export const INN = process.env.ARENA_INN_SCENE ?? 'reldens-house-1';

export type Place = { x: number; y: number; description: string };

/**
 * Home turf. Every coordinate was checked against the map's collision layers
 * and confirmed reachable on foot *from the door a character arrives through* -
 * not merely unblocked, which is weaker and lets a tile sit behind a wall.
 */
export const PLACES: Record<string, Record<string, Place>> = {
  [TOWN]: {
    'the west road': { x: 16, y: 592, description: 'where the road leaves town westward' },
    'the south field': { x: 496, y: 880, description: 'open grass south of the houses' },
    'the east gate': { x: 1168, y: 656, description: 'the east way out of town' },
    'the north path': { x: 816, y: 176, description: 'the top of town, near the trees' },
    'outside the inn': { x: 400, y: 336, description: "the street in front of Barnaby's door" },
    'outside the second house': {
      x: 1264,
      y: 656,
      description: 'the door of the other house, over on the east side'
    }
  },
  [INN]: {
    'the bar': { x: 720, y: 464, description: 'where Barnaby stands' },
    'the back table': { x: 624, y: 432, description: 'a table on the far side of the room' },
    'the fireplace corner': { x: 496, y: 592, description: 'the quiet corner near the front' },
    'the foot of the stairs': {
      x: 752,
      y: 464,
      description: 'the bottom of the stairs up, right beside the bar'
    },
    'the inn door': { x: 528, y: 624, description: 'the way back out to the street' }
  }
};

export function placesIn(scene: string): Record<string, Place> {
  return PLACES[scene] ?? {};
}

/** Whether this is somewhere the character grew up knowing its way around. */
export function isHomeTurf(scene: string): boolean {
  return scene in PLACES;
}

export function describePlaces(scene: string): string {
  const places = placesIn(scene);
  const names = Object.keys(places);
  if (names.length === 0) {
    return '';
  }
  return names.map((name) => `- "${name}": ${places[name].description}`).join('\n');
}

/** Which room a named place is in, of the rooms a character knows by heart. */
export function roomOf(place: string): string | null {
  const wanted = place.trim().toLowerCase();
  for (const [scene, places] of Object.entries(PLACES)) {
    if (Object.keys(places).some((name) => name.toLowerCase() === wanted)) {
      return scene;
    }
  }
  return null;
}

/** A room's name as a person would say it, not as the database spells it. */
export function plainSceneName(scene: string): string {
  const known: Record<string, string> = {
    [TOWN]: 'town',
    [INN]: "Barnaby's inn"
  };
  return known[scene] ?? scene.replace(/^reldens-/, '').replace(/-/g, ' ');
}
