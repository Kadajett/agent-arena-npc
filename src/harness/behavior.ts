/**
 * How a character spends its time.
 *
 * This is the part that differs between characters, and the only part. Barnaby
 * stands behind a bar and answers people. The Wanderer walks a fixed circuit
 * and always has. Guy decides for himself. All three observe the world, react
 * to being spoken to, and act through the same tools; the difference is only
 * what they choose when nobody is talking to them.
 *
 * A behaviour is asked one question - what next? - and answers with an intent.
 * It may ask a model, or it may not: a routine is a state machine and never
 * spends a token deciding to take the next step of a walk it has taken every
 * day for years.
 */

import { Agent } from '@mastra/core/agent';
import { Intent, IntentSchema } from './actions.js';

/** Where this character's memory lives; passed on every model call. */
export type MemoryScope = { resource: string; thread: string };

export type Situation = {
  scene: string;
  /** Where the character is, in words. */
  where: string;
  /** Everyone else in the room. */
  others: string[];
  /** Lines spoken since the last look, oldest first. */
  heard: Array<{ from: string; message: string }>;
  /** What this character can do right now, already written out. */
  actions: string;
  /** Places in this room, already written out. */
  places: string;
  /**
   * Everything said in earshot lately, this character's own lines included, in
   * the order it was said. Without the whole exchange a character answers the
   * last thing it heard with no idea what it is part of, which is how you get
   * two NPCs greeting each other forever.
   */
  conversation: Array<{ from: string; message: string; fresh: boolean }>;
  /** How many words this character tends to use at a stretch. */
  wordiness: number;
  /**
   * What this character is after, the plan it is working through, and how the
   * last few attempts went. Already written out; empty for a character with no
   * goal. See plan.ts.
   */
  purpose: string;
  /** Places it has been and places it has only been told about. */
  known: string;
  /** Whether this room is somewhere it has never stood before. */
  strange: boolean;
  /** The doorways it can see from here, already written out. */
  doors: string;
  /**
   * What is around it right now, as a small map centred on where it stands.
   * Travels with the character, so it shows what is in front of it rather than
   * where it came in.
   */
  view: string;
  /** A subject it keeps circling back to, if it has one. */
  harping: string;
  /** Free-form character state, e.g. savings toward a goal. */
  notes: string[];
  /** The people this character remembers, and how it feels about them. */
  people: string;
};

export interface Behavior {
  readonly kind: string;
  /** Decide what to do when nobody is demanding anything. */
  next(situation: Situation, memory: MemoryScope): Promise<Intent>;
  /** Called after an action runs, so a routine can advance. */
  completed?(intent: Intent, ok: boolean): void;
}

/**
 * Stands still. Reacts when spoken to and otherwise does nothing at all.
 * An innkeeper is not restless.
 */
export class Stationary implements Behavior {
  readonly kind = 'stationary';

  async next(): Promise<Intent> {
    return { action: 'wait' };
  }
}

export type RoutineStep =
  | { go: string; saying?: string }
  | { door: true; saying?: string }
  | { rest: number; saying?: string };

/**
 * A fixed round, walked forever. Deterministic: no model is consulted to take
 * the next step, because the whole character of a creature of habit is that it
 * does not reconsider. Speech still goes through the model, so it can answer
 * someone without breaking stride.
 */
export class Routine implements Behavior {
  readonly kind = 'routine';
  private index = 0;
  private restUntil = 0;
  /**
   * Whether the last answer was a leg of the round or just waiting out a rest.
   * Without this the round advances on every tick spent standing still, so a
   * minute sat at the bar burns ten steps and the character never walks again.
   */
  private walking = false;

  constructor(private readonly steps: RoutineStep[]) {
    if (steps.length === 0) {
      throw new Error('A routine needs at least one step.');
    }
  }

  async next(): Promise<Intent> {
    if (Date.now() < this.restUntil) {
      this.walking = false;
      return { action: 'wait' };
    }
    const step = this.steps[this.index % this.steps.length];
    if ('rest' in step) {
      // Sitting down is itself the step, and it is done the moment it starts.
      this.restUntil = Date.now() + step.rest * 1000;
      this.index += 1;
      this.walking = false;
      return step.saying ? { action: 'say', message: step.saying } : { action: 'wait' };
    }
    this.walking = true;
    if ('door' in step) {
      return { action: 'use_door', message: step.saying };
    }
    return { action: 'walk', place: step.go, message: step.saying };
  }

  completed(_intent: Intent, _ok: boolean): void {
    if (!this.walking) {
      return;
    }
    // Move on either way. A character that cannot get somewhere today shrugs
    // and carries on with its round rather than standing at the obstacle
    // forever, which is what a person would do.
    this.walking = false;
    this.index += 1;
  }
}

/**
 * Decides for itself, every time, working through the plan it has made toward
 * the goal on its character sheet. The goal comes from the sheet rather than
 * from here, so a character keeps it whichever behaviour it is given.
 */
export class Autonomous implements Behavior {
  readonly kind = 'autonomous';

  constructor(private readonly agent: Agent) {}

  async next(situation: Situation, memory: MemoryScope): Promise<Intent> {
    const prompt = [
      describeSituation(situation),
      situation.purpose ? `\n${situation.purpose}` : null,
      '',
      'What do you do next? Choose exactly one:',
      situation.actions,
      '',
      'Reply with JSON and nothing else:',
      '{"action": "...", "place": "...", "message": "...", "progress": "same"}',
      'Use "place" only for walk. Use "message" for say, or when walking if you',
      'want to remark on it. Anything you say is in your own voice, with no',
      'asterisks and no description of your own actions.',
      situation.purpose
        ? 'Set "progress" to say where the step you are working on stands after '
          + 'this: "same" if you are still at it, "done" if this finishes it, '
          + '"blocked" if it cannot be done and you want a different plan.'
        : null,
      lengthGuidance(situation.wordiness)
    ]
      .filter((line): line is string => line !== null)
      .join('\n');
    return askForIntent(this.agent, prompt, memory);
  }
}

/** Everything the character can see, written the way a person would think it. */
export function describeSituation(situation: Situation): string {
  const lines = [`You are at ${situation.where}.`];
  lines.push(
    situation.others.length > 0
      ? `Also here: ${situation.others.join(', ')}.`
      : 'Nobody else is here.'
  );
  if (situation.strange) {
    lines.push(
      'You have never been in here before. You do not know what is in this room '
        + 'until you have walked around it and looked.'
    );
  }
  if (situation.places) {
    lines.push('', 'Places you could go from here:', situation.places);
  }
  if (situation.doors) {
    lines.push('', 'Ways out of this room:', situation.doors);
  }
  if (situation.view) {
    lines.push('', 'What is around you:', situation.view);
  }
  if (situation.known) {
    lines.push('', situation.known);
  }
  for (const note of situation.notes) {
    lines.push(note);
  }
  if (situation.people) {
    lines.push('', situation.people);
  }
  if (situation.conversation.length > 0) {
    // The whole exchange, oldest first, so a reply lands in the conversation
    // it belongs to rather than answering one stray line out of context.
    lines.push('', 'What has been said here, oldest first:');
    for (const line of situation.conversation) {
      lines.push(`  ${line.from}: ${line.message}${line.fresh ? '   <- just now' : ''}`);
    }
    lines.push('Do not repeat yourself, and do not answer something twice.');
  }
  if (situation.harping) {
    lines.push('', situation.harping);
  }
  return lines.join('\n');
}

/** How long this character talks for, said the way a person would think it. */
export function lengthGuidance(wordiness: number): string {
  if (wordiness <= 15) {
    return 'You are short with people. A sentence, usually less.';
  }
  if (wordiness <= 30) {
    return 'Keep it to a couple of sentences.';
  }
  if (wordiness <= 60) {
    return `You talk in a few sentences at a time. Around ${wordiness} words, never far past it.`;
  }
  return `You run on when you get going, but stop before ${wordiness} words.`;
}

/**
 * Ask the model for one intent. A reply that is not usable becomes "wait":
 * a character that cannot make up its mind stands there, which reads fine and
 * is always safe.
 */
export async function askForIntent(
  agent: Agent,
  prompt: string,
  memory: MemoryScope
): Promise<Intent> {
  // The memory scope has to be passed on every call: without it nothing is
  // recalled and nothing is written, and the character quietly has no memory
  // at all while looking like it does.
  const response = await agent.generate(prompt, { memory });
  const text = String(response.text ?? '').trim();
  const json = text.slice(text.indexOf('{'), text.lastIndexOf('}') + 1);
  if (!json) {
    // Say what came back rather than swallowing it. A character standing
    // still because it chose to and one standing still because nothing it
    // said could be read look identical from the outside, and only one of
    // them is a bug.
    console.log('unreadable reply:', text.slice(0, 160));
    return { action: 'wait' };
  }
  let parsed;
  try {
    parsed = IntentSchema.safeParse(JSON.parse(json));
  } catch (error) {
    console.log('reply was not JSON:', json.slice(0, 160));
    return { action: 'wait' };
  }
  if (!parsed.success) {
    console.log('reply was not an intent:', json.slice(0, 160));
    return { action: 'wait' };
  }
  return parsed.data as Intent;
}
