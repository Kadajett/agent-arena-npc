/**
 * Doran: a wandering knight, walked on Routine rather than decided on.
 *
 * The brief's own reasoning: a creature of habit walking a fixed round costs
 * no tokens per step, which matters because these characters run on
 * OpenRouter's free router and the point is partly to see how far that goes
 * without paying for a mind these two don't need. Routine never asks the
 * model what to do next; the model is only spent when somebody actually
 * speaks to him, through the harness's own answer() path, same as any other
 * character.
 *
 * The round below uses only the six named places world.ts's PLACES table
 * already defines for reldens-town - real, checked coordinates, not new
 * ones. Doran and Aveline (see aveline.ts) walk the same six stops in
 * opposite order, so the town has two guards passing each other at
 * different points of their rounds rather than one guard's coverage
 * doubled.
 *
 * Sprite: warrior - the one class-path sheet closest to "guard" of the two
 * that were unclaimed. Shared with Aveline on purpose: two knights in
 * matching armour reads as a uniform, not a bug. See
 * deploy/world/assign-classes.mjs.
 */

import { CharacterSheet } from '../harness/npc.js';
import { Autonomous } from '../harness/behavior.js';
import { TOWN } from '../harness/world.js';
import { loadPersona } from '../persona.js';

export const doran: CharacterSheet = {
  id: 'doran',
  playerName: process.env.ARENA_PLAYER_NAME ?? 'Doran',
  // See guy.ts. Warrior, so no fireball.
  classPath: 'warrior',
  homeScene: TOWN,
  persona: loadPersona('doran'),
  model: process.env.NPC_MODEL ?? 'openrouter/openai/gpt-oss-120b',
  capabilities: ['speak', 'talk_to_folk', 'walk', 'doors', 'fight'],
  behavior: (agent) => new Autonomous(agent),
  // The round used to be a hardcoded Routine, no model consulted per step.
  // Now the character walks it itself, and the round is pinned rather than
  // remembered so no amount of conversation can talk him off his beat. The
  // places are the six the situation's own map lists, with coordinates.
  pinned: [
    'YOUR ROUND, WHICH IS THE JOB:',
    'Walk town in this order, forever: the west road, the north path, the east',
    'gate, the south field, outside the inn (pause a while), outside the second',
    'house (pause a while), then start again. A remark at a post - "East gate.',
    'All clear." - is in character; a speech is not. You stop for people who',
    'speak to you, and for trouble, and for nothing else. When in doubt, the',
    'next stop on the round is always the right answer.'
  ].join('\n'),
  // No goal: the round is the whole job, same reasoning as Barnaby having
  // none. Nothing here is working toward being finished with.
  pace: { idle: 90, engaged: 9 },
  wordiness: 22,
  remembers: true
};
