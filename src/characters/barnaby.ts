/**
 * Barnaby: behind his bar, and nowhere else.
 *
 * He is given speech and nothing else, so there is no way for him to wander
 * off even if a model decides he should. The innkeeper being reliably in the
 * inn is the whole point of him.
 */

import { CharacterSheet } from '../harness/npc.js';
import { Stationary } from '../harness/behavior.js';
import { INN, TOWN } from '../harness/world.js';
import { loadPersona } from '../persona.js';
import { everywhereByName } from '../harness/world.js';

export const barnaby: CharacterSheet = {
  id: 'barnaby',
  playerName: process.env.ARENA_PLAYER_NAME ?? 'Barnaby',
  // Three times anybody else, not forty times. This was 1,600 on the reasoning
  // that he cannot say "I never said that" about a night that has fallen out of
  // his head, which is true and was the wrong mechanism: raw messages ride on
  // every call and cost accordingly, while the observation log keeps the same
  // ground for a fraction of it. What makes him the record is the pinned facts
  // below and that log, not the size of this number.
  recall: 48,
  // Pinned rather than remembered. A fact that lives in memory can be pushed
  // out by a long night or written over by somebody insisting; this cannot.
  pinned:
    'THE PLACES THAT EXIST\n'
    + 'You have heard of everywhere in this world, the way anybody who has poured drinks\n'
    + 'for forty years has heard of everywhere. You have not been to most of them and you\n'
    + 'know nothing about what is in them. The complete list, and there are no others:\n'
    + everywhereByName().map((name) => `  ${name}`).join('\n')
    + '\n\nIf somebody names a place that is not on that list, you have never heard of it,\n'
    + 'and you say so. Not rudely, not at length, just plainly, the first time it comes up.\n'
    + 'You do not work out where it might be, you do not allow that it might be somewhere\n'
    + 'you have not been, and you never repeat the name back as though it were a real\n'
    + 'place. Somebody made it up, or misheard, and the polite thing is to say so before\n'
    + 'ten more people have it off you as fact.',
  homeScene: INN,
  persona: loadPersona('barnaby'),
  model: process.env.NPC_MODEL ?? 'openrouter/openai/gpt-oss-120b',
  capabilities: ['speak', 'talk_to_folk'],
  behavior: () => new Stationary(),
  // The one character who knows the town without walking it. He has stood
  // behind this bar for years and people ask him for directions; with nothing
  // true to give them he was inventing a guildhall and a council building and
  // sending Guy across town after neither. These are the real places, checked
  // against the map. He can still only speak, so knowing where the east gate
  // is does not let him go there.
  localKnowledge: [TOWN],
  // He is only ever reacting, so he can afford to look up less often.
  pace: { idle: 90, engaged: 9 },
  // He talks for a living but brevity is his whole act, so he gets a little
  // more room than a clipped one-liner and not much more.
  wordiness: 35,
  remembers: true
};
