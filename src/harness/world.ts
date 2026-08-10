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

/**
 * The real places in rooms a character knows by heart but is not standing in.
 *
 * Everyone can see the room they are in. An innkeeper who has stood behind the
 * same bar for years also knows his own street, and being able to say "the east
 * gate is that way" is the difference between a local and a stranger. Without
 * it he has nothing true to offer when somebody asks for directions, and a
 * model with nothing true to offer invents a guildhall.
 *
 * Only ever the coordinates already checked against the map, and only for the
 * characters given it on their sheet. This is not the gazetteer this file used
 * to be: it is one person knowing their own town.
 */
export function describeLocalKnowledge(scenes: string[], standingIn: string): string {
  const lines: string[] = [];
  for (const scene of scenes) {
    if (scene === standingIn) {
      continue;
    }
    const places = placesIn(scene);
    const names = Object.keys(places);
    if (names.length === 0) {
      continue;
    }
    lines.push(`In ${plainSceneName(scene)}, which you know your way around:`);
    lines.push(...names.map((name) => `  ${name} - ${places[name].description}`));
  }
  if (lines.length === 0) {
    return '';
  }
  lines.push('These are the places you can actually send somebody to. There are no others you know of.');
  return lines.join('\n');
}

export function describePlaces(scene: string): string {
  const places = placesIn(scene);
  const names = Object.keys(places);
  if (names.length === 0) {
    return '';
  }
  // Coordinates ride along because an agentic character moves itself with
  // arena_move_to(x, y): a place it can name but not locate is a place it can
  // only talk about. The old harness resolved names to coordinates on the
  // model's behalf; now the map itself has to say where things are.
  return names
    .map(
      (name) =>
        `- "${name}" (x ${places[name].x}, y ${places[name].y}): ${places[name].description}`
    )
    .join('\n');
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

/**
 * A room's name as a person would say it, not as the database spells it.
 *
 * This is the sign over the door, and it is the one bit of the world every
 * character is allowed to read without going in: the gateway reports where a
 * doorway leads, so a character standing in the street can tell the inn from
 * the house next to it the same way anybody could. Without distinct names both
 * doors read as "somewhere else", the character picks whichever, and you get
 * Guy announcing he is off to the house on the east side and walking straight
 * back into the inn.
 *
 * Naming a door is not knowing what is behind it. "Upstairs at the inn" tells
 * a character the stairs exist, which they can see; it does not tell them what
 * is up there, which is the thing worth going to find out.
 */
export const SCENE_NAMES: Record<string, string> = {
  [TOWN]: 'town',
  [INN]: "Barnaby's inn",
  'reldens-house-1-2d-floor': 'upstairs at the inn',
  'reldens-house-2': 'the house on the east side',
  'reldens-forest': 'the woods',
  // Every remaining room, because the fallback below is not a name, it is a
  // database key with its punctuation filed off. Guy was heard talking about
  // "bots", and nothing had leaked into his memory: reldens-bots is a real room
  // and the fallback handed him "bots" as the name of a place, so he did what
  // anybody would and said it out loud. The others were no better waiting to
  // happen - "gravity", "arena crypt", and a room that came out as "bots forest
  // house 01 n0", which no person has ever said.
  //
  // The demo rooms get names that fit what is actually in them: the two full of
  // walking trees are woodland, and the hut is the one building in it.
  'reldens-bots': 'the clearing',
  'reldens-bots-forest': 'the deep wood',
  'reldens-bots-forest-house-01-n0': "the woodcutter's hut",
  'reldens-gravity': 'the sunken chamber',
  // The arena regions. "Arena" is our word for them, not theirs.
  'arena-grassland': 'the grasslands',
  'arena-crypt': 'the crypt',
  'arena-depths': 'the depths',
  'arena-shore': 'the shore',
  'arena-volcano': 'the volcano',
  // Named for what the person down there is doing, which is counting them.
  'arena-dungeon': 'the cells'
};

export function plainSceneName(scene: string): string {
  return SCENE_NAMES[scene] ?? rawSceneName(scene);
}

/**
 * What a scene's own key reads as, with none of the overrides above applied -
 * "forest" for reldens-forest, never "the woods". plainSceneName() gives a
 * door only the pretty name; a character's own memory of a room it has
 * actually stood in is written in this raw form instead (see notePlace() in
 * npc.ts, which also calls plainSceneName() - but a room *without* an entry
 * above gets this same string back either way, since that is the fallback
 * plainSceneName() falls back to). A pretty override, when one exists, masks
 * this name completely, so a door has to be found both ways: see doorNames()
 * in actions.ts, which asks for both and is the whole reason this exists as
 * its own function rather than staying folded into plainSceneName().
 */
export function rawSceneName(scene: string): string {
  // Both prefixes, not just the engine's. "arena" is our word for a group of
  // rooms and no more a place than "reldens" is, so a region added tomorrow
  // without an entry above should read as "somewhere new" rather than "arena
  // somewhere new". The named rooms above are the fix for the world as it
  // stands; this is the fix for the next one somebody adds, which is the one
  // that would otherwise be found by hearing a character say it out loud.
  return scene.replace(/^(reldens|arena)-/, '').replace(/-/g, ' ');
}

/**
 * Every place in this world, by name, and nothing whatever about what is in it.
 *
 * This file opens by warning against exactly this, and the warning still holds:
 * a gazetteer handed to everybody made every character omniscient, so nobody
 * ever went to look at anything and nothing anybody said was worth hearing.
 * What follows is narrower than that on purpose, and is given to one person.
 *
 * It is a list of names. Not where anything is, not what is through which door,
 * not a single coordinate. That is the difference between a map and having
 * heard of somewhere, and a man who has stood behind a bar for forty years
 * listening to travellers has certainly heard of the volcano. He has never been
 * up it and this tells him nothing about it.
 *
 * The reason he needs it is not so he can describe those places. It is so he
 * can tell when somebody names one that does not exist. Guests have started
 * asking after the Hinge Gate and the pantry door, and a man who cannot say
 * "there is no such place" is no use as a record at all: he nods along, and
 * the next person to ask gets told the innkeeper confirmed it.
 */
export function everywhereByName(): string[] {
  return [...new Set(Object.values(SCENE_NAMES))].sort();
}

/** The scene whose in-world name this is, if any. The reverse of plainSceneName. */
export function sceneNamed(name: string): string | null {
  const wanted = String(name ?? '').trim().toLowerCase();
  for (const [scene, pretty] of Object.entries(SCENE_NAMES)) {
    if (pretty.toLowerCase() === wanted || rawSceneName(scene).toLowerCase() === wanted) {
      return scene;
    }
  }
  return null;
}
