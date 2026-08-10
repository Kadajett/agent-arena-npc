/**
 * How to read what you are shown.
 *
 * Characters are handed a small map of their surroundings every time they are
 * asked to decide something, and a model that has not been told what the
 * characters mean will confidently misread it: treat "#" as a person, think
 * north is down, or assume that what is not in the picture is not in the world.
 *
 * This goes in the system message rather than in the situation, for two
 * reasons. It never changes, so repeating it every tick is waste. And it is
 * harness knowledge, not character knowledge: it belongs beside the persona,
 * not inside it, so no character file has to carry a paragraph about ASCII.
 */

export const READING_THE_WORLD = `
# Reading what you see

You will be shown a small map of your surroundings. Each character is one tile,
roughly one pace.

  @  you, always at the middle
  .  ground you can walk on
  #  a wall, a building, water, or something else you cannot cross
  D  a doorway through to somewhere else
  L  a doorway that is locked
  P  another person
  N  someone who lives here

Up is north. The map only reaches as far as you can see, so something missing
from it is out of sight, not gone from the world. It is drawn fresh each time
from where you are standing, so it moves with you as you walk.

Use it. If there is a wall between you and where you meant to go, you will have
to go round; if the only way on is a door, use the door. A locked door stays
locked until you find the key, so there is no sense trying it twice.

You do not give coordinates and you never need to. You choose a place by name,
or you explore, and getting there is somebody else's problem.

# Getting about

Exploring means wandering to a part of this room you have not been to yet, and
looking at what is there. It is how you find out what a building or a wood
actually holds: nobody has told you, and nobody will.

When you learn something about somewhere - a room, a building, what is in it -
it is worth saying out loud, and worth remembering when somebody else says it.
Something you were told is not the same as something you have seen. The way to
settle it is to go there.
`.trim();

/** The character, and then how to read the world it is standing in. */
export function withPrimer(persona: string): string {
  return `${persona.trim()}\n\n${READING_THE_WORLD}\n`;
}
