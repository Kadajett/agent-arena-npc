/**
 * Guy: free to go where he likes, with something he is saving for.
 *
 * The most autonomous of the three. Everything he does is chosen fresh each
 * time, which is what makes him worth watching and what makes him the one to
 * hand new abilities to first.
 */

import { Agent } from '@mastra/core/agent';
import { CharacterSheet } from '../harness/npc.js';
import { Autonomous } from '../harness/behavior.js';
import { TOWN } from '../harness/world.js';
import { loadPersona } from '../persona.js';

export const guy: CharacterSheet = {
  id: 'guy',
  playerName: process.env.ARENA_PLAYER_NAME ?? 'Guy',
  homeScene: TOWN,
  persona: loadPersona('guy'),
  model: process.env.NPC_MODEL ?? 'openrouter/deepseek/deepseek-v4-flash',
  capabilities: ['speak', 'walk', 'doors', 'money'],
  behavior: (agent: Agent) => new Autonomous(agent),
  // The thing he is actually here for. He keeps a list of what to do about it
  // and works one item at a time; the list survives restarts and he cannot talk
  // himself out of the goal behind it.
  goal: {
    aim: "settle what is upstairs at Barnaby's inn. He has never once mentioned "
      + 'it, which you have decided is the interesting part. Go up and look, then '
      + 'get somebody to say out loud that you were right about it.',
    done: 'you have been up there yourself and somebody else has admitted what is up there'
  },
  pace: { idle: 12, engaged: 4 },
  // Says his piece and gets on with it.
  wordiness: 35,
  remembers: true
};
