/**
 * Cutter: building a boat on a shore nobody else lives on.
 *
 * Home is arena-shore, the far end of the grassland/shore leg of the arena
 * chain. place-readables.mjs's beachcomber already mentions the tide handing
 * back driftwood and rope "never tied to anything anyone's found" - Cutter
 * is written to sit right next to that line without resolving it: he
 * wonders if he is building with the wreck of somebody else's boat and has
 * never asked her outright, which is a thread the beachcomber and Cutter can
 * now actually have, in the world, rather than one NPC's static paragraph
 * gesturing at nobody.
 *
 * Runs on OpenRouter's free router. See docker-compose.yml.
 */

import { Agent } from '@mastra/core/agent';
import { CharacterSheet } from '../harness/npc.js';
import { Autonomous } from '../harness/behavior.js';
import { loadPersona } from '../persona.js';

export const cutter: CharacterSheet = {
  id: 'cutter',
  playerName: process.env.ARENA_PLAYER_NAME ?? 'Cutter',
  homeScene: 'arena-shore',
  persona: loadPersona('cutter'),
  model: process.env.NPC_MODEL ?? 'openrouter/openai/gpt-oss-120b',
  capabilities: ['speak', 'talk_to_folk', 'walk', 'doors', 'fight', 'purpose'],
  behavior: (agent: Agent) => new Autonomous(agent),
  goal: {
    aim: 'finish the boat you have been building out of what the shore '
      + 'gives up, and get it into the water before you talk yourself out '
      + 'of it again.',
    done: 'you have gotten the boat into the water yourself and it did not '
      + 'immediately fall apart under you.'
  },
  pace: { idle: 90, engaged: 9 },
  wordiness: 38,
  remembers: true
};
