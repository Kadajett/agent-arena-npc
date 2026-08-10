/**
 * Hollis: cuts wood in a stand of trees that hit back.
 *
 * Home is reldens-bots-forest-house-01-n0, the one house at the far end of
 * the reldens-bots / reldens-bots-forest chain running off the south side
 * of town - a stretch that had nobody living in it at all. His trouble is
 * the "Tree" and "Tree Punch" enemies already stocked in both of those
 * rooms: he has decided, in his own voice and without the harness ever
 * confirming it for him, that something is wrong with his patch of woods
 * specifically. Whether that is true is not this file's business; it is
 * his belief, checked against nothing, same as every other character's
 * pet theory in this world.
 *
 * Runs on OpenRouter's free router. See docker-compose.yml.
 */

import { Agent } from '@mastra/core/agent';
import { CharacterSheet } from '../harness/npc.js';
import { Autonomous } from '../harness/behavior.js';
import { loadPersona } from '../persona.js';

export const hollis: CharacterSheet = {
  id: 'hollis',
  playerName: process.env.ARENA_PLAYER_NAME ?? 'Hollis',
  homeScene: 'reldens-bots-forest-house-01-n0',
  persona: loadPersona('hollis'),
  model: process.env.NPC_MODEL ?? 'openrouter/openai/gpt-oss-120b',
  capabilities: ['speak', 'talk_to_folk', 'walk', 'doors', 'fight', 'purpose'],
  behavior: (agent: Agent) => new Autonomous(agent),
  // Miles, in reldens-forest, is a real static NPC he has only heard of by
  // trade. Reaching him means the long walk: his own woods, into
  // reldens-bots, into town, out again to the forest on the other side.
  goal: {
    aim: 'find somebody else who has swung an axe at a tree in these woods '
      + 'and had the tree swing back, and find out whether it is just your '
      + 'patch of forest or the whole business is like this everywhere.',
    done: 'you have found another person who has fought a tree themselves '
      + 'and compared what happened to each of you.'
  },
  pace: { idle: 90, engaged: 9 },
  wordiness: 30,
  remembers: true
};
