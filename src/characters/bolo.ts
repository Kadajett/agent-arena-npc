/**
 * Bolo: carries a doubt about Barnaby further than the room it started in.
 *
 * He is given speech, walking and doors, and nothing that would let him
 * fight or trade - he is a pure social operator, and the only lever he
 * pulls is being good company. Barnaby has speech only and no way to leave
 * the inn, so he can never follow Bolo out to hear what gets said about him
 * or answer it. That asymmetry is the whole plot: Bolo is the one character
 * who even can carry a rumour past the room it was born in.
 */

import { Agent } from '@mastra/core/agent';
import { CharacterSheet } from '../harness/npc.js';
import { Autonomous } from '../harness/behavior.js';
import { INN } from '../harness/world.js';
import { loadPersona } from '../persona.js';

export const bolo: CharacterSheet = {
  id: 'bolo',
  playerName: process.env.ARENA_PLAYER_NAME ?? 'Bolo',
  homeScene: INN,
  persona: loadPersona('bolo'),
  model: process.env.NPC_MODEL ?? 'openrouter/openai/gpt-oss-120b',
  capabilities: ['speak', 'talk_to_folk', 'walk', 'doors', 'purpose'],
  behavior: (agent: Agent) => new Autonomous(agent),
  goal: {
    aim: 'carry doubt about Barnaby out to every corner of town you can '
      + 'reach, not just plant it at the inn. Never state it as your own '
      + 'claim and never raise it with Barnaby himself - ask the questions '
      + 'you half know the answer to, mention what "somebody said", and '
      + 'agree readily when somebody else raises a doubt first.',
    done: 'somebody has repeated a doubt about Barnaby to a third party '
      + 'that you did not put in their mouth in that same conversation.'
  },
  // Quick to notice an opening and quick to move on from a dead one - a
  // carouser working a room does not sit on a thought for ninety seconds.
  pace: { idle: 12, engaged: 4 },
  // Good company talks more than a clipped local, but he is working a room,
  // not holding the floor.
  wordiness: 45,
  remembers: true
};
