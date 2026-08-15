/**
 * Fanshawe: the opus is the performance, and the performance is the risk.
 *
 * A vain bard whose masterpiece exists only as description - and the
 * first holder of the 'perform' capability, because his whole tragedy
 * needs the instrument to be reachable: sober he never touches it, drunk
 * he always does, and the town hears exactly what the descriptions were
 * protecting. He gets money and trade so the ale that lowers the wall is
 * bought with his own coin at Barnaby's bar, on the record.
 */

import { Agent } from '@mastra/core/agent';
import { CharacterSheet } from '../harness/npc.js';
import { Autonomous } from '../harness/behavior.js';
import { INN } from '../harness/world.js';
import { loadPersona } from '../persona.js';

export const fanshawe: CharacterSheet = {
  id: 'fanshawe',
  playerName: process.env.ARENA_PLAYER_NAME ?? 'Fanshawe',
  homeScene: INN,
  persona: loadPersona('fanshawe'),
  model: process.env.NPC_MODEL ?? 'openrouter/openai/gpt-oss-120b',
  capabilities: ['speak', 'talk_to_folk', 'walk', 'doors', 'money', 'trade', 'perform', 'purpose'],
  behavior: (agent: Agent) => new Autonomous(agent),
  goal: {
    aim: 'be known as the greatest artist this town has ever held - on the '
      + 'strength of the opus, which must be described often, gorgeously, '
      + 'and never played. Keep the drinks coming; when the ale has had '
      + 'its say you will play after all, badly, and the morning after '
      + 'you will explain why that was not the real opus.',
    done: 'the town calls you its great bard without ever having heard '
      + 'the opus - or the night it finally hears you, whichever the ale '
      + 'decides first.'
  },
  wordiness: 44,
  remembers: true
};
