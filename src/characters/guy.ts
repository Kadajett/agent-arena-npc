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
  // Matches what the world actually put him on, in assign-classes.mjs. The
  // two have to agree: this decides which skills he is offered, that decides
  // which the world will accept, and a character offered fireball it cannot
  // cast just gets refused somewhere it cannot see the refusal.
  classPath: 'swordsman',
  homeScene: TOWN,
  persona: loadPersona('guy'),
  model: process.env.NPC_MODEL ?? 'openrouter/openai/gpt-oss-120b',
  capabilities: ['speak', 'talk_to_folk', 'walk', 'doors', 'fight', 'duel', 'money', 'trade', 'purpose'],
  behavior: (agent: Agent) => new Autonomous(agent),
  // Where he starts. He keeps a list of what to do about it and works one item
  // at a time, and the list survives restarts. Once he has settled this he
  // picks the next thing himself, which for Guy will be whatever he has decided
  // somebody is being cagey about.
  //
  // This used to be the sealed door upstairs at the inn, and it broke him in a
  // way worth writing down, because the shape of the mistake will happen again.
  // The aim was fine. The done was "somebody else has admitted what is up
  // there", and there is nothing up there: the second floor's only two change
  // points are the stairs back down. A condition nobody can meet is not a hard
  // goal, it is a loop, and he ran it for days - street, inn, upstairs, inn,
  // street - while the one character built to go out and find trouble never
  // left three rooms.
  //
  // It cost more than Guy. His goal told him to get somebody to say out loud
  // that he was right about the door, so he asked everybody, for days, and that
  // is where the whole town's talk of doors and keys and the Hinge Gate came
  // from. One unreachable done condition became this world's largest rumour.
  //
  // So: never write a done that another character has to satisfy. He is the
  // only one who can decide when this one is finished, and getting there walks
  // him through most of the world, which is what he is for.
  goal: {
    // His standing ambition, set after the volcano goal had served its purpose
    // of walking him out of town. Strength and wealth is permanent in the way
    // that matters: there is always something stronger to beat and always
    // another credit to earn, every step of it is his to take without anybody
    // else's say-so, and the done below is a milestone rather than an ending.
    // He has the purpose capability, so when he reaches it he will set the
    // next one himself, presumably larger.
    aim: 'become strong and rich. You came out of that volcano with nothing '
      + 'and got put down outside an inn like a stray cat. Never again. Fight '
      + 'what can be fought, take what work pays, count your money, and make '
      + 'sure nobody in this world mistakes you for a man to be carried twice.',
    done: 'you have a thousand credits to your name and you have beaten, in a fair fight, '
      + 'everything in this world that has ever knocked you down'
  },
  pace: { idle: 90, engaged: 9 },
  // Says his piece and gets on with it.
  wordiness: 35,
  remembers: true
};
