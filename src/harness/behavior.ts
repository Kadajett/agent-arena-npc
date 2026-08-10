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
import { FEELING_GUIDANCE } from './feeling.js';
import { meter } from './spend.js';

/** Where this character's memory lives; passed on every model call. */
export type MemoryScope = { resource: string; thread: string };

/**
 * How much the model may write back.
 *
 * Saying so at all matters because providers reserve credit against the ceiling
 * rather than the actual reply: left at the default, every one of these ticks
 * was quoted at 65,536 tokens and refused outright once the balance dipped
 * below what that would have cost.
 *
 * A thousand was the first guess, from the size of a reply: a small JSON object
 * and a couple of hundred words of speech. That missed the largest thing a
 * character writes, which is not its speech but its memory. Updating working
 * memory means re-emitting the whole record - every person, every place, every
 * note - as one tool call, and the Wanderer's ran past the ceiling and arrived
 * as truncated JSON. The write was dropped, silently, and it would have kept
 * being dropped as its memory grew. Four thousand fits a full record with room
 * to spare, and still reserves against a fraction of what the default did.
 */
export const REPLY_BUDGET = { maxOutputTokens: 4096 };

export type Situation = {
  scene: string;
  /** Where the character is, in words. */
  where: string;
  /**
   * Everyone else in the room, and whether this character has laid eyes on them
   * before. The second half matters more than it looks: without it every tick
   * reads as a first meeting, and three characters spent an evening telling
   * each other about the two new faces at the bar. See noteFace() in memory.ts.
   */
  others: Array<{ name: string; known: boolean }>;
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
  /**
   * What it is carrying, in one line, already written out. Empty when it is
   * carrying nothing - which is said by saying nothing, because a line
   * announcing empty pockets on every tick of an empty-handed character's life
   * is the sort of noise that ends up being answered.
   */
  carrying: string;
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
  /** Whose voice to answer in, for behaviours that ask a model at all. */
  answersAs?(persona: string): void;
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

  /**
   * How many replies in a row have come back as prose with no action in them.
   *
   * A model follows the format its own history demonstrates far more closely
   * than it follows an instruction it was given once, so a character that
   * drifts out of answering in JSON does not drift back: every prose reply it
   * writes is another example telling it that prose is how it answers. Guy
   * went ninety-seven turns that way, narrating himself down a ladder into a
   * room that does not exist while standing motionless outside the inn, and
   * nothing in here ever told him otherwise. Salvaging the prose as speech is
   * still right - it is usually real dialogue and binning it loses the turn -
   * but on its own it makes the trapdoor comfortable, because from the
   * character's side speaking worked. So the salvage stays and this counts.
   */
  private proseInARow = 0;

  constructor(private readonly agent: Agent, private persona = '') {}

  /** Told to the behaviour once the character is built, since it owns the text. */
  answersAs(persona: string): void {
    this.persona = persona;
  }

  async next(situation: Situation, memory: MemoryScope): Promise<Intent> {
    const prompt = [
      describeSituation(situation),
      '',
      'What do you do next? Choose exactly one:',
      situation.actions,
      '',
      'Reply with JSON and nothing else:',
      '{"action": "...", "place": "...", "message": "...", "progress": "same", "feeling": "..."}',
      'Use "place" only for walk. Use "message" for say, or when walking if you',
      'want to remark on it. Anything you say is in your own voice, with no',
      'asterisks and no description of your own actions.',
      situation.purpose
        ? 'Set "progress" to say where the step you are working on stands after '
          + 'this: "same" if you are still at it, "done" if this finishes it, '
          + '"blocked" if it cannot be done and you want a different plan.'
        : null,
      BOOKKEEPING,
      FEELING_GUIDANCE,
      lengthGuidance(situation.wordiness)
    ]
      .filter((line): line is string => line !== null)
      .join('\n');
    // The correction goes in the moment rather than the brief, and that is the
    // whole point of it. The brief is per-call instructions, which is where the
    // "reply with JSON and nothing else" line already lives and where Guy
    // overrode it for ninety-seven turns running; another line in the same
    // place is the thing that already does not work. The moment is the turn the
    // character is answering, and it is the half that gets stored, so saying it
    // there both puts it where the model is looking and leaves it in the
    // history. That matters more than the immediacy: what made this stick was a
    // history of nothing but prose, and a history with the correction in it no
    // longer reads that way.
    const correction = this.correction();
    const moment = correction ? `${correction}\n${momentOf(situation)}` : momentOf(situation);
    const intent = await askForIntent(this.agent, prompt, memory, moment, this.persona);
    this.proseInARow = intent.salvagedFromProse ? this.proseInARow + 1 : 0;
    return this.stillWorth(intent);
  }

  /**
   * Whether a salvaged reply still gets spoken.
   *
   * The correction alone did not stop Guy, and the reason is that the harness
   * was contradicting itself. It told him his last replies had no action in
   * them and that nothing he described had happened, and then it took the prose
   * and said it out loud anyway. From where he was standing the prose worked:
   * he wrote a scene, the room heard the good bits, and a sentence went past
   * claiming otherwise. Told one thing and shown another, he believed what he
   * could see.
   *
   * So past a run, prose stops being spoken. The character stands there and
   * says nothing, which makes the correction true rather than merely insistent,
   * and puts a turn of silence in the history where a turn of accepted prose
   * used to be.
   *
   * Deliberately later than the correction starts. Two is where a character is
   * told; four is where it is no longer humoured. The gap is there because
   * salvage is usually right - most prose is real dialogue that just never got
   * a wrapper - and the point is not to punish a character for how it phrased
   * something, it is to stop paying it for not answering at all.
   */
  private stillWorth(intent: Intent): Intent {
    if (!intent.salvagedFromProse || this.proseInARow < 4) {
      return intent;
    }
    // Whatever else it wrote down still counts. Losing a turn's speech is the
    // correction working; losing the promise it made in the same breath is how
    // a character stops keeping promises. Same reasoning as askForIntent().
    return { ...intent, action: 'wait', message: undefined };
  }

  /**
   * What to say to a character that has stopped answering in the format.
   *
   * Nothing at all while it is answering properly, and nothing for a single
   * stray reply either: one is a character writing a line of dialogue without
   * the wrapper, which the salvage already handles and which is not worth
   * interrupting. It is a run that means the format is gone.
   *
   * The wording is about consequences rather than compliance because that is
   * what is actually wrong: the character is not being disobedient, it thinks
   * the things it describes are happening. Telling it that it has not moved is
   * information it does not otherwise have, since the world it is being shown
   * each turn looks the same as the one it has been narrating.
   */
  private correction(): string | null {
    if (this.proseInARow < 2) {
      return null;
    }
    return [
      `[Your last ${this.proseInARow} replies had no action in them, so none of`,
      'what you described happened. You have not moved, opened anything or',
      'picked anything up. You are standing exactly where you were and the only',
      'part of it anybody heard was the words. Reply with the JSON object this',
      'time and let the action do the moving.]'
    ].join('\n');
  }
}

/**
 * What a character may write down while doing something else.
 *
 * These ride along with whatever action it chose, rather than being actions of
 * their own. A character that has to stand still for a turn to make a note will
 * not make notes, and a character that cannot cross anything off its list ends
 * up with a list it has stopped reading.
 */
export const BOOKKEEPING = [
  'You may also add any of these to the same reply, alongside whatever you are doing:',
  '  "remember": something to keep in mind for the next hour, then forget',
  '  "todo": something you have just taken on and mean to do',
  '  "askedBy": with "todo", who asked you for it, if this is a favour and not',
  '            something you decided on your own',
  '  "finished": an item on your list you have just done, by its number',
  '  "gaveUpOn": an item on your list you are dropping, by its number',
  '  "noted": a place somebody mentioned that you have never been',
  '  "notThere": a place you were told about, went looking for, and did not find',
  '  "recall": somebody or somewhere you want to think back on. What you know',
  '            comes back to you a moment later, so ask before you need it',
  '  "progressOn" and "learned": something on your list, by its number, and what',
  '            you have found out about it so far'
].join('\n');

/** Everything the character can see, written the way a person would think it. */
export function describeSituation(situation: Situation): string {
  const lines: string[] = [];
  // What it is up to comes first, and comes into every prompt built from a
  // situation - including the one that only decides whether to answer somebody.
  // Put it in the decide-what-to-do prompt alone and a character keeps its goal
  // right up until the first person talks to it, then spends the rest of the
  // day making conversation. See Plan.describe().
  if (situation.purpose) {
    lines.push(situation.purpose, '');
  }
  lines.push(`You are at ${situation.where}.`);
  lines.push(
    situation.others.length > 0
      ? `Also here: ${situation.others
          .map((person) => (person.known ? person.name : `${person.name} (new to you)`))
          .join(', ')}.`
      : 'Nobody else is here.'
  );
  if (situation.others.length > 0 && situation.others.every((person) => person.known)) {
    // Said outright, because otherwise a familiar room full of familiar people
    // gets greeted from scratch every twelve seconds.
    lines.push('You know everyone here. Nobody has just walked in.');
  }
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
  // One line, and only when there is something in the satchel. A character
  // with empty pockets is told nothing at all, because "you are carrying
  // nothing" is a fact about the world that never changes on its own and
  // reads, on the fiftieth tick, as a prompt to go and find something.
  if (situation.carrying) {
    lines.push(situation.carrying);
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
 * The one line of this tick that is worth keeping.
 *
 * What gets written into the message history is this, not the brief. A tick
 * where somebody spoke is remembered as who said what; a quiet tick is
 * remembered as where the character was standing. Fifty of these cost about as
 * much as one of the old prompts did, which is the whole point: see
 * askForIntent below.
 */
export function momentOf(situation: Situation): string {
  if (situation.heard.length > 0) {
    return situation.heard
      .map((line) => `${line.from}: "${line.message}"`)
      .join('\n');
  }
  return situation.others.length > 0
    ? `[${situation.where}, with ${situation.others.map((person) => person.name).join(' and ')}]`
    : `[${situation.where}, alone]`;
}

/**
 * Ask the model for one intent. A reply that is not usable becomes "wait":
 * a character that cannot make up its mind stands there, which reads fine and
 * is always safe.
 *
 * The brief and the moment are split for a reason. Everything the character can
 * see right now - the map, the doorways, its plan, the room it is standing in -
 * is true for this tick and stale by the next one, so it goes in as per-call
 * instructions, which Mastra sends and does not store. What gets stored is the
 * moment: one short line saying what was actually said to it.
 *
 * That split is what buys the message history back. Storing the brief meant a
 * remembered turn cost thousands of tokens, fifty of them came to 48,000, and
 * the provider refused the calls outright - so history had to be switched off
 * entirely and every character woke up each tick having forgotten the
 * conversation it was in the middle of. Now a turn costs a line, fifty turns
 * fit comfortably, and a character can hold a thread long enough to get
 * somewhere with it instead of greeting the same person forever.
 */
export async function askForIntent(
  agent: Agent,
  brief: string,
  memory: MemoryScope,
  moment: string,
  persona: string
): Promise<Intent> {
  // The memory scope has to be passed on every call: without it nothing is
  // recalled and nothing is written, and the character quietly has no memory
  // at all while looking like it does.
  const response = await agent.generate(moment, {
    memory,
    // This replaces the agent's instructions rather than adding to them, so the
    // persona has to come along or the character answers as nobody.
    instructions: `${persona}\n\n${brief}`,
    modelSettings: REPLY_BUDGET
  });
  meter(agent.name ?? agent.id ?? 'someone', 'thinking', response);
  const text = String(response.text ?? '').trim();
  const json = firstJsonObject(text);
  if (!json) {
    if (!text) {
      return { action: 'wait' };
    }
    // Say what came back rather than swallowing it. A character standing
    // still because it chose to and one standing still because nothing it
    // said could be read look identical from the outside, and only one of
    // them is a bug.
    console.log('unreadable reply:', text.slice(0, 160));
    // A model asked for JSON and answering in prose instead is not the same
    // failure as answering with nothing: the Wanderer and Guy lost a fifth
    // and a tenth of their turns this way, and most of what got thrown away
    // was ordinary in-character speech that just never made it into a
    // {"action": "say", ...} shell. Binning it is throwing away a good
    // reply because of how it was wrapped. The one thing prose can be that
    // is not worth saying out loud is the character working out a route to
    // itself out loud - see isThinkingAloud() - and that alone is left as
    // "wait", same as before.
    if (isThinkingAloud(text)) {
      console.log('read as thinking aloud, not said:', text.slice(0, 160));
      return { action: 'wait', salvagedFromProse: true };
    }
    return { action: 'say', message: text, salvagedFromProse: true };
  }
  let parsed;
  try {
    parsed = IntentSchema.safeParse(JSON.parse(json));
  } catch (error) {
    console.log('reply was not JSON:', json.slice(0, 160));
    // Counted as drift, not as a considered decision to stand still. Without
    // this a model that has started emitting broken JSON every tick looks, to
    // the correction in behavior.ts, exactly like a character choosing to wait,
    // so it is never told anything is wrong and never recovers.
    return { action: 'wait', salvagedFromProse: true };
  }
  if (!parsed.success) {
    console.log('reply was not an intent:', json.slice(0, 160));
    // The action was unreadable; what the character decided to write down may
    // not have been. Losing a turn's movement is a shrug. Losing the promise it
    // made in the same breath is how a character stops keeping promises, so
    // salvage the bookkeeping and stand still for this one.
    return { ...bookkeepingOf(json), action: 'wait' };
  }
  return parsed.data as Intent;
}




/**
 * The first complete JSON object in a reply, rather than the widest span.
 *
 * This used to be a slice from the first "{" to the last "}", which is right
 * for one object with prose either side of it and wrong the moment there are
 * two. Marren answered with a perfectly good {"action":"say",...}, then thought
 * better of it and wrote a fuller one underneath, and the slice took both plus
 * the gap between them. That is not JSON, so the parse threw and she lost the
 * turn - having twice said what she meant to do.
 *
 * Braces are counted rather than matched by regex, because a message is a
 * string and a string can contain a brace. "{" inside quotes is a character
 * somebody typed, not structure, and an escaped quote inside that string does
 * not end it.
 *
 * Returns '' when there is no complete object, which is the same thing the old
 * slice returned for a reply with no braces at all, so prose salvage below
 * still gets its turn.
 */
export function firstJsonObject(text: string): string {
  const from = text.indexOf('{');
  if (-1 === from) {
    return '';
  }
  let depth = 0;
  let inString = false;
  let escaped = false;
  for (let at = from; at < text.length; at++) {
    const character = text[at];
    if (escaped) {
      escaped = false;
      continue;
    }
    if ('\\' === character) {
      escaped = true;
      continue;
    }
    if ('"' === character) {
      inString = !inString;
      continue;
    }
    if (inString) {
      continue;
    }
    if ('{' === character) {
      depth++;
      continue;
    }
    if ('}' === character) {
      depth--;
      if (0 === depth) {
        return text.slice(from, at + 1);
      }
    }
  }
  // Unbalanced: an object that was cut off by the token budget. The widest
  // span is no better, so this reads as prose and gets salvaged as speech.
  return '';
}

/**
 * Whether a stray line of prose is a character working out a move rather
 * than something it means for anybody nearby to hear.
 *
 * "I need to walk south to reach the doorway. About fifteen paces. Let me
 * go." is reasoning about a heading, not a remark aimed at somebody in the
 * room - nobody in this world actually talks to another person that way.
 * The tell is the combination of a movement verb with a way of measuring the
 * ground (paces, steps, tiles), or a flat command to itself to get moving.
 * Cheap and narrow on purpose: it is built to catch that one shape of
 * thinking-out-loud without also catching ordinary speech that happens to
 * mention a direction, which the two salvaged examples both did.
 */
function isThinkingAloud(text: string): boolean {
  const movement = /\b(walk|walking|walked|go|going|head|heading|move|moving)\b/i;
  const distance = /\b(paces?|steps?|tiles?)\b/i;
  const selfCommand = /\blet me (go|head|walk|move)\b/i;
  return (movement.test(text) && distance.test(text)) || selfCommand.test(text);
}

/**
 * The parts of a reply that are worth keeping even when the rest is not.
 *
 * Each field is taken on its own, so one malformed entry cannot cost the
 * others, and anything that is not a plain string is left behind rather than
 * guessed at.
 */
function bookkeepingOf(json: string): Partial<Intent> {
  let raw: unknown;
  try {
    raw = JSON.parse(json);
  } catch {
    return {};
  }
  if (!raw || typeof raw !== 'object') {
    return {};
  }
  const source = raw as Record<string, unknown>;
  const kept: Partial<Intent> = {};
  for (const field of [
    'remember',
    'todo',
    'askedBy',
    'finished',
    'gaveUpOn',
    'noted',
    'notThere',
    'recall'
  ] as const) {
    const value = source[field];
    if (typeof value === 'string' && value.trim()) {
      kept[field] = value;
    }
  }
  return kept;
}
