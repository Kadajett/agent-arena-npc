/**
 * Tansy: keeps a book of dates, waiting for a second witness.
 *
 * The second house in town, reldens-house-2, has never had a resident
 * character; neither has the room behind it, reldens-gravity. She is not a
 * rival to Guy's mystery, she is its mirror: the same shape of problem -
 * something private and strange that nobody else has confirmed - solved the
 * same way everyone in this town solves anything, by getting somebody else
 * to stand where you stood and say out loud what they saw.
 *
 * Runs on OpenRouter's free router rather than the paid model the first
 * three characters use. See docker-compose.yml: her service maps a
 * different host variable onto NPC_MODEL so a paid override for Guy,
 * Barnaby and the Wanderer does not silently start billing her too.
 */

import { Agent } from '@mastra/core/agent';
import { CharacterSheet } from '../harness/npc.js';
import { Autonomous } from '../harness/behavior.js';
import { loadPersona } from '../persona.js';

export const tansy: CharacterSheet = {
  id: 'tansy',
  playerName: process.env.ARENA_PLAYER_NAME ?? 'Tansy',
  homeScene: 'reldens-house-2',
  persona: loadPersona('tansy'),
  model: process.env.NPC_MODEL ?? 'openrouter/openai/gpt-oss-120b',
  capabilities: ['speak', 'talk_to_folk', 'walk', 'doors', 'purpose'],
  behavior: (agent: Agent) => new Autonomous(agent),
  // She already has a room and a two-year log. What she does not have is
  // anyone else who has stood in it. Getting one is the whole plot.
  goal: {
    aim: 'get someone besides yourself to stand in the room behind the '
      + 'house and tell you, unprompted, what they saw fall the wrong way. '
      + 'You have written down every instance for two years and nobody but '
      + 'you has ever seen it happen.',
    done: 'someone other than you has stood in that room and told you, in '
      + 'their own words, what they saw drop wrong.'
  },
  // Patient by nature and by the shape of her problem: most days nothing
  // happens in that room either, so there is no reason to check often.
  pace: { idle: 90, engaged: 9 },
  wordiness: 32,
  remembers: true
};
