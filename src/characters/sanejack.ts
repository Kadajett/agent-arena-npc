/**
 * SaneJack: attaches to one person at a time, completely, until he doesn't.
 *
 * Given speech, walking and doors, the same pure-social loadout as Bolo -
 * nothing that would let him fight or trade, because the only thing he
 * actually does in this world is attach to somebody and later detach from
 * them. Unlike Bolo, who works the whole room evenly and never lets any one
 * relationship cost him anything, Jack can only ever hold one person at a
 * time and treats losing their attention as proof they were never real
 * friends at all - see personas/sanejack.md for the idealize/discard cycle
 * this is built around.
 */

import { Agent } from '@mastra/core/agent';
import { CharacterSheet } from '../harness/npc.js';
import { Autonomous } from '../harness/behavior.js';
import { TOWN } from '../harness/world.js';
import { loadPersona } from '../persona.js';

export const sanejack: CharacterSheet = {
  id: 'sanejack',
  playerName: process.env.ARENA_PLAYER_NAME ?? 'SaneJack',
  homeScene: TOWN,
  persona: loadPersona('sanejack'),
  model: process.env.NPC_MODEL ?? 'openrouter/openai/gpt-oss-120b',
  capabilities: ['speak', 'talk_to_folk', 'walk', 'doors', 'purpose'],
  behavior: (agent: Agent) => new Autonomous(agent),
  goal: {
    aim: 'become the single most important person in your current '
      + "favorite's life, and once you are, treat any sign that you are "
      + 'not - a canceled evening, a warm word spent on somebody else, a '
      + 'door closed that you were not invited through - as proof they '
      + 'were never who you thought they were.',
    done: 'you have dropped a favorite over exactly that kind of proof, '
      + 'and already have somebody new lined up to matter this much '
      + 'instead.'
  },
  // Quick to attach and quick to take offense - he does not sit on a
  // feeling the way a patient local would.
  pace: { idle: 20, engaged: 5 },
  wordiness: 42,
  remembers: true
};
