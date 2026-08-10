/**
 * Nerys: a real combatant living in town, not a reduced one.
 *
 * The brief asked for a player agent with the full set of controls a real
 * player's agent has, combat included, not a cut-down capability list. The
 * NPC harness's Capability type has exactly seven entries and Guy already
 * holds all of them, so "full" here means the same 7/7 set Guy has - that is
 * the most any CharacterSheet on this harness can be given today. True parity
 * with a raw MCP player agent (queueing a match, choosing a named skill
 * instead of the default basic attack, checking credit history) is not wired
 * into services/npc/src/harness/actions.ts at all yet, for anybody, and
 * adding that is a harness change outside what this file can do.
 *
 * Sprite: warlock. See deploy/world/assign-classes.mjs - warlock and warrior
 * were the only two class-path sheets nobody was wearing.
 */

import { Agent } from '@mastra/core/agent';
import { CharacterSheet } from '../harness/npc.js';
import { Autonomous } from '../harness/behavior.js';
import { TOWN } from '../harness/world.js';
import { loadPersona } from '../persona.js';

export const nerys: CharacterSheet = {
  id: 'nerys',
  playerName: process.env.ARENA_PLAYER_NAME ?? 'Nerys',
  // See guy.ts. Warlock, so fireball and no heal.
  classPath: 'warlock',
  homeScene: TOWN,
  persona: loadPersona('nerys'),
  model: process.env.NPC_MODEL ?? 'openrouter/openai/gpt-oss-120b',
  capabilities: ['speak', 'talk_to_folk', 'walk', 'doors', 'fight', 'duel', 'money', 'trade', 'purpose'],
  behavior: (agent: Agent) => new Autonomous(agent),
  goal: {
    aim: 'get far enough up the volcano to find out what is actually '
      + 'burning under the ordinary fire, without anyone who follows you '
      + 'there getting hurt for your curiosity instead of theirs.',
    done: 'you have gone far enough up the volcano yourself to know what '
      + 'the heat under it actually is.'
  },
  pace: { idle: 90, engaged: 9 },
  wordiness: 34,
  remembers: true
};
