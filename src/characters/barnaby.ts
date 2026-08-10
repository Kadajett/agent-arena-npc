/**
 * Barnaby: behind his bar, and nowhere else.
 *
 * He is given speech and nothing else, so there is no way for him to wander
 * off even if a model decides he should. The innkeeper being reliably in the
 * inn is the whole point of him.
 */

import { CharacterSheet } from '../harness/npc.js';
import { Stationary } from '../harness/behavior.js';
import { INN } from '../harness/world.js';
import { loadPersona } from '../persona.js';

export const barnaby: CharacterSheet = {
  id: 'barnaby',
  playerName: process.env.ARENA_PLAYER_NAME ?? 'Barnaby',
  homeScene: INN,
  persona: loadPersona('barnaby'),
  model: process.env.NPC_MODEL ?? 'openrouter/deepseek/deepseek-v4-flash',
  capabilities: ['speak'],
  behavior: () => new Stationary(),
  // He is only ever reacting, so he can afford to look up less often.
  pace: { idle: 20, engaged: 4 },
  // He talks for a living but brevity is his whole act, so he gets a little
  // more room than a clipped one-liner and not much more.
  wordiness: 35,
  remembers: true
};
