/**
 * The Wanderer: certain about everything, right about very little.
 *
 * He used to walk a scripted circuit, which meant he said the same four things
 * about the same road forever. He now decides for himself like everyone else.
 * The point of him is contrast: Guy suspects, the Wanderer already knows, and
 * neither of them checks.
 */

import { Agent } from '@mastra/core/agent';
import { CharacterSheet } from '../harness/npc.js';
import { Autonomous } from '../harness/behavior.js';
import { TOWN } from '../harness/world.js';
import { loadPersona } from '../persona.js';

export const wanderer: CharacterSheet = {
  id: 'wanderer',
  playerName: process.env.ARENA_PLAYER_NAME ?? 'Wanderer',
  // See guy.ts. Sorcerer here because sorcerer there.
  classPath: 'sorcerer',
  homeScene: TOWN,
  persona: loadPersona('wanderer'),
  model: process.env.NPC_MODEL ?? 'openrouter/openai/gpt-oss-120b',
  capabilities: ['speak', 'talk_to_folk', 'walk', 'doors', 'fight', 'purpose'],
  behavior: (agent: Agent) => new Autonomous(agent),
  goal: {
    aim: 'you have decided the town needs proper names for its places, and that '
      + 'you are the man to give them. Get somebody else to use one of your names '
      + 'back at you.',
    done: 'you have heard another person call somewhere by the name you gave it'
  },
  // He has an opinion about everything and time to share it.
  wordiness: 55,
  pace: { idle: 90, engaged: 9 },
  remembers: true
};
