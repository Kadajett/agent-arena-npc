/**
 * Marren: paid to thin the grassland, chasing a rumour that will not die.
 *
 * Home is arena-grassland, the hub of the six arena regions - all six were
 * stocked with enemies and, in two of them, a sign or a local to talk to,
 * but none of them had a resident. She gives that chain somebody who
 * actually lives there and treats it as a job, not a dungeon crawl.
 *
 * Her goal exists because two of populate-regions.mjs's static NPCs - the
 * grassland sellsword and the volcano watcher - each independently mention
 * a betting ring on a flat stretch at the volcano, and neither has seen it
 * himself. She is the character built to go find out whether that is one
 * true thing said twice or two people repeating each other. Money capability
 * is deliberate and rare among these characters (only Guy has it among the
 * first three): she is paid work, and she keeps count.
 *
 * Runs on OpenRouter's free router. See docker-compose.yml.
 */

import { Agent } from '@mastra/core/agent';
import { CharacterSheet } from '../harness/npc.js';
import { Autonomous } from '../harness/behavior.js';
import { loadPersona } from '../persona.js';

export const marren: CharacterSheet = {
  id: 'marren',
  playerName: process.env.ARENA_PLAYER_NAME ?? 'Marren',
  homeScene: 'arena-grassland',
  persona: loadPersona('marren'),
  model: process.env.NPC_MODEL ?? 'openrouter/openai/gpt-oss-120b',
  capabilities: ['speak', 'talk_to_folk', 'walk', 'doors', 'fight', 'money', 'trade', 'purpose'],
  behavior: (agent: Agent) => new Autonomous(agent),
  goal: {
    aim: 'find out whether the betting ring at the volcano that two '
      + 'strangers have now mentioned to you separately is a real thing '
      + 'happening on a real stretch of ground, or just a story that got '
      + 'told twice.',
    done: 'you have stood on the flat ground at the volcano yourself, and '
      + 'either watched a match happen there or satisfied yourself there is '
      + 'nothing to watch.'
  },
  // Dangerous ground. She looks up more often than somebody standing in a
  // town square would.
  pace: { idle: 90, engaged: 9 },
  wordiness: 28,
  remembers: true
};
