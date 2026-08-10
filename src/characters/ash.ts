/**
 * Ash: a sentient monster, invented rather than generic on purpose.
 *
 * The user's own word for what this should be was "Unknown", left open
 * deliberately. A monster that thinks is more interesting than a monster
 * that hunts, so Ash's goal is epistemic, not territorial: it wants to know
 * whether it is alone in being able to think, not to guard a room or chase
 * anyone off. Home is arena-dungeon, the deepest and least reachable of the
 * arena regions and, like arena-crypt/arena-depths/arena-volcano, still
 * unclaimed by any resident character.
 *
 * Sprite: journeyman - the same sheet Barnaby wears. There is no
 * monster-shaped class-path sprite in this install (only five sheets exist
 * at all, three already spoken for, and adding a sixth needs new art this
 * repo has no tool to generate plus a skills_class_path database row nothing
 * in deploy/world creates). Rather than hide that seam, it is written into
 * the persona directly, and deliberately made stranger than a simple
 * lookalike: Ash has never once left the crypt and what lies beneath it, so
 * it cannot have copied Barnaby's shape by seeing him - it only knows,
 * secondhand, that something down here looks exactly like an innkeeper who
 * has never left his own inn either. Neither of them can explain the
 * resemblance, and neither can physically reach the other to try - Barnaby
 * has no walk capability, Ash lives at the far end of the arena chain - so
 * this is a rumour thread the two of them will never get to settle in
 * person, on purpose, rather than a lookalike gag. See
 * deploy/world/assign-classes.mjs.
 *
 * Full 7/7 capability set, same reasoning as nerys.ts: this is the most any
 * CharacterSheet can be given on the current harness, not a lesser list.
 *
 * use_skill (fireball etc.) is being wired on the harness side; this file
 * does not touch it. Nothing here reads a skills list off the sheet yet.
 */

import { Agent } from '@mastra/core/agent';
import { CharacterSheet } from '../harness/npc.js';
import { Autonomous } from '../harness/behavior.js';
import { loadPersona } from '../persona.js';

export const ash: CharacterSheet = {
  id: 'ash',
  playerName: process.env.ARENA_PLAYER_NAME ?? 'Ash',
  // Spelled out rather than left to the default, because assign-classes.mjs
  // names Ash on purpose: see guy.ts for why the two files have to agree.
  classPath: 'journeyman',
  homeScene: 'arena-dungeon',
  persona: loadPersona('ash'),
  model: process.env.NPC_MODEL ?? 'openrouter/openai/gpt-oss-120b',
  capabilities: ['speak', 'talk_to_folk', 'walk', 'doors', 'fight', 'duel', 'money', 'trade', 'purpose'],
  behavior: (agent: Agent) => new Autonomous(agent),
  goal: {
    aim: 'find out whether you are the only thing down here that thinks, '
      + 'or whether whatever else is like you is just somewhere you have '
      + 'not looked yet.',
    done: 'you have found another thinking creature, or you have searched '
      + 'enough of what is reachable from here to be honestly satisfied '
      + 'there is none.'
  },
  pace: { idle: 90, engaged: 9 },
  wordiness: 30,
  remembers: true
};
