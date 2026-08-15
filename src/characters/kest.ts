/**
 * Kest: the foil to the lord-and-knight pair, and to every performer in town.
 *
 * Sir Qwen fights in service and is praised in rooms he is not in; Kest
 * serves nobody and forbids himself an audience. He is given the full
 * fighting kit - fight, duel, money, trade - because his goal runs through
 * all of it: harder fights, levels and skills as fuel, the best gear the
 * town sells. He can speak, and mostly chooses not to; wordiness is set to
 * the floor of the cast so the model is pushed toward his one-sentence
 * register rather than reminded of it.
 */

import { Agent } from '@mastra/core/agent';
import { CharacterSheet } from '../harness/npc.js';
import { Autonomous } from '../harness/behavior.js';
import { loadPersona } from '../persona.js';

export const kest: CharacterSheet = {
  id: 'kest',
  playerName: process.env.ARENA_PLAYER_NAME ?? 'Kest',
  homeScene: 'reldens-bots-forest',
  persona: loadPersona('kest'),
  model: process.env.NPC_MODEL ?? 'openrouter/openai/gpt-oss-120b',
  capabilities: ['speak', 'talk_to_folk', 'walk', 'doors', 'fight', 'duel', 'money', 'trade', 'purpose'],
  behavior: (agent: Agent) => new Autonomous(agent),
  goal: {
    aim: 'prove to yourself - nobody else counts - that you are the '
      + 'superior champion of this town: seek the hardest fights the world '
      + 'holds, take every level and skill it will yield, and put the best '
      + 'gear the town sells in your hands, bought with coin your own '
      + 'fighting earned. A fight that costs you nothing proves nothing; '
      + 'when ground stops being hard, find harder ground.',
    done: 'you have stood against the strongest thing and the strongest '
      + 'fighter this world offers and neither put you down - and you knew '
      + 'it, without anyone having to say so.'
  },
  pace: { idle: 8, engaged: 6 },
  steps: 8,
  wordiness: 14,
  remembers: true
};
