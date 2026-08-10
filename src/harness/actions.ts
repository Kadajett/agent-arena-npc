/**
 * Everything an NPC is able to do, and nothing else.
 *
 * The gateway exposes a lot of tools. A character in a town needs a handful of
 * them, expressed the way a person would think about them: go somewhere, go
 * through that door, say something, stand still. Each action here wraps the
 * gateway call and constrains its inputs, so a character asks for "the east
 * gate" rather than a pixel coordinate and a badly chosen action sends someone
 * to the wrong end of town instead of into a wall.
 *
 * A character declares which of these it may use. Barnaby cannot walk out of
 * his own inn because he was never given the door.
 */

import { z } from 'zod';
import { ArenaClient, Observation, sceneOf } from './arena.js';
import { Explorer, RoomView, SeenDoor } from './explore.js';
import { placesIn, plainSceneName, roomOf } from './world.js';

export const CAPABILITIES = ['speak', 'walk', 'doors', 'fight', 'money'] as const;
export type Capability = (typeof CAPABILITIES)[number];

/**
 * How the step a character is working on stands after this action. It is the
 * character that says, because only it knows what it was trying to do; the
 * harness holds the list and does the writing down.
 */
export type Progress = 'same' | 'done' | 'blocked';

const ACTIONS = [
  'say',
  'walk',
  'explore',
  'use_door',
  'attack',
  'check_money',
  'wait'
] as const;

/** What a character decided to do this tick. */
export type Intent = {
  action: (typeof ACTIONS)[number];
  place?: string;
  target?: string;
  message?: string;
  progress?: Progress;
  /**
   * Something the character just learned about a place, in its own words. This
   * is how hearsay gets into memory: somebody mentions a room upstairs, the
   * character notes that it heard it and from whom, and can go and check later.
   */
  noted?: string;
};

export const IntentSchema = z.object({
  action: z.enum(ACTIONS),
  place: z.string().optional(),
  target: z.string().optional(),
  message: z.string().optional(),
  progress: z.enum(['same', 'done', 'blocked']).optional(),
  noted: z.string().optional()
});

const ARRIVAL_PIXELS = 40;
const WALK_POLL_MS = 1500;
const STILL_POLLS = 3;
const LEG_TIMEOUT_MS = 45_000;
/** Let a body finish sliding before anything reads its position. */
const SETTLE_MS = 600;
/** Reldens clips chat at 100 characters (config chat/messages/characterLimit). */
export const CHAT_LINE_LIMIT = 100;
/**
 * How much anyone may say at a stretch. A character's own wordiness sets what
 * it usually says; this is the ceiling none of them may pass, because a wall of
 * text arrives as a stack of chat bubbles nobody reads.
 */
export const MAX_WORDS = 120;
export const DEFAULT_WORDS = 35;
/** Long enough to read as one person talking, short enough not to be a speech. */
const MAX_LINES = 6;
const BETWEEN_LINES_MS = 1400;

export type ActionResult = { ok: boolean; note: string };

export class Actions {
  /** What the character can see of the room it is in. Set each tick. */
  private view: RoomView | null = null;

  constructor(
    private readonly arena: ArenaClient,
    private readonly agentId: string,
    private readonly capabilities: Set<Capability>,
    /** How much this character says at a stretch, in words. */
    private readonly wordiness: number = DEFAULT_WORDS,
    private readonly explorer: Explorer = new Explorer()
  ) {}

  can(capability: Capability): boolean {
    return this.capabilities.has(capability);
  }

  sees(view: RoomView | null): void {
    this.view = view;
  }

  /** The doorways out of here, as the character can see them. */
  doors(): SeenDoor[] {
    return this.view?.doors ?? [];
  }

  /** The list of actions to offer a character, given where it is standing. */
  describe(scene: string): string {
    const lines: string[] = [];
    if (this.can('speak')) {
      lines.push('- "say": say something out loud. Needs: message');
    }
    if (this.can('walk')) {
      const names = Object.keys(placesIn(scene));
      if (names.length > 0) {
        lines.push(`- "walk": go somewhere in this room. Needs: place, one of ${names
          .map((name) => `"${name}"`)
          .join(', ')}`);
      }
      // Anywhere that is not home is found out by walking around it. This is
      // the only way to see a room nobody has written down.
      lines.push('- "explore": wander to a part of this room you have not seen');
    }
    if (this.can('doors')) {
      const doors = this.doors();
      if (doors.length === 1) {
        lines.push(`- "use_door": go through to ${this.doorLabel(doors[0])}`);
      } else if (doors.length > 1) {
        lines.push(
          '- "use_door": go through a door. Needs: place, one of '
            + doors.map((door) => `"${this.doorLabel(door)}"`).join(', ')
        );
      }
    }
    if (this.can('money')) {
      lines.push('- "check_money": count what you have saved');
    }
    lines.push('- "wait": stay where you are');
    return lines.join('\n');
  }

  private doorLabel(door: SeenDoor): string {
    return door.leadsTo ? plainSceneName(door.leadsTo) : 'somewhere else';
  }

  async perform(intent: Intent, scene: string): Promise<ActionResult> {
    switch (intent.action) {
      case 'say':
        return this.say(intent.message);
      case 'walk':
        return this.walk(intent.place, scene, intent.message);
      case 'explore':
        return this.explore(scene, intent.message);
      case 'use_door':
        return this.useDoor(scene, intent.place, intent.message);
      case 'attack':
        return this.attack(intent.target);
      case 'check_money':
        return this.checkMoney();
      default:
        return { ok: true, note: 'stayed put' };
    }
  }

  /**
   * Wander somewhere in this room the character has not been. The harness picks
   * the spot off the real collision grid and confirms the route first, so this
   * is exploring rather than walking hopefully into a wall.
   */
  async explore(scene: string, message?: string): Promise<ActionResult> {
    if (!this.can('walk')) {
      return { ok: false, note: 'this character stays where it is' };
    }
    const here = await this.where();
    if (!here) {
      return { ok: false, note: 'could not tell where it was standing' };
    }
    this.explorer.markHere(scene, here.x, here.y);
    const spot = await this.explorer.somewhereNew(this.arena, this.agentId, scene, here);
    if (!spot) {
      return { ok: false, note: 'there was nowhere new to go from here' };
    }
    const talking = this.alsoSay(message);
    await this.arena.call('arena_move_to', { agent_id: this.agentId, x: spot.x, y: spot.y });
    const arrived = await this.waitForArrival(spot.x, spot.y);
    await talking;
    // Mark where it actually ended up, not where it meant to go. A character
    // that stalls against a corner has still moved, and recording the target
    // would tell it it had seen a patch it never reached.
    const landed = await this.where();
    if (landed) {
      this.explorer.markHere(scene, landed.x, landed.y);
    }
    return {
      ok: arrived,
      note: arrived
        ? `had a look around to the ${spot.bearing}`
        : `set off ${spot.bearing} and did not get there`
    };
  }

  private async where(): Promise<{ x: number; y: number } | null> {
    const observation = await this.observe().catch(() => null);
    const state = observation?.ownPlayer?.state;
    if (!state || !Number.isFinite(Number(state.x))) {
      return null;
    }
    return { x: Number(state.x), y: Number(state.y) };
  }

  /**
   * Speak. A long thought goes out as several chat lines in a row, paced like
   * someone actually saying it, because the chat field takes 100 characters and
   * a character who talks in paragraphs would otherwise arrive cut mid-word.
   */
  async say(message: string | undefined): Promise<ActionResult> {
    if (!this.can('speak')) {
      return { ok: false, note: 'this character does not speak' };
    }
    const lines = toSpeech(message ?? '', this.wordiness);
    if (lines.length === 0) {
      return { ok: false, note: 'nothing worth saying' };
    }
    for (const [index, line] of lines.entries()) {
      if (index > 0) {
        await sleep(BETWEEN_LINES_MS);
      }
      await this.arena.call('arena_say', { agent_id: this.agentId, message: line });
    }
    return { ok: true, note: `said: ${lines.join(' ')}` };
  }

  async walk(place: string | undefined, scene: string, message?: string): Promise<ActionResult> {
    if (!this.can('walk')) {
      return { ok: false, note: 'this character stays where it is' };
    }
    const places = placesIn(scene);
    const key = Object.keys(places).find(
      (name) => name.toLowerCase() === String(place ?? '').trim().toLowerCase()
    );
    if (!key) {
      // The place may simply be in the next room, and the way to another room
      // is through the door. A character heading for the bar from the street
      // should walk in rather than report that the bar does not exist.
      const elsewhere = roomOf(String(place ?? ''));
      if (elsewhere && elsewhere !== scene && this.can('doors')) {
        const through = this.doors().find((door) => door.leadsTo === elsewhere);
        if (through) {
          return this.useDoor(scene, place, message);
        }
      }
      // Somewhere it has heard of but cannot place: look around for it rather
      // than announcing that it does not exist.
      if (this.can('walk') && place) {
        return this.explore(scene, message);
      }
      return { ok: false, note: `there is no "${place}" here` };
    }
    // Talk on the way. Waiting for a three-line remark to finish before taking
    // a step means standing in the street reciting, and it reads as a stall.
    const talking = this.alsoSay(message);
    const target = places[key];
    await this.arena.call('arena_move_to', {
      agent_id: this.agentId,
      x: target.x,
      y: target.y
    });
    const arrived = await this.waitForArrival(target.x, target.y);
    await talking;
    return { ok: arrived, note: arrived ? `walked to ${key}` : `set off for ${key} but stopped short` };
  }

  /** Start saying something without waiting for it to finish. */
  private alsoSay(message: string | undefined): Promise<unknown> {
    return message ? this.say(message).catch(() => undefined) : Promise.resolve();
  }

  /**
   * Go through a doorway the character can see. Which doors exist comes from
   * looking at the room, not from a table somebody wrote out in advance, so
   * this works the same in a room nobody has ever surveyed.
   */
  async useDoor(scene: string, which?: string, message?: string): Promise<ActionResult> {
    if (!this.can('doors')) {
      return { ok: false, note: 'this character does not leave this room' };
    }
    const door = this.pickDoor(which);
    if (!door) {
      const doors = this.doors();
      return {
        ok: false,
        note: doors.length === 0
          ? 'there is no way out of here that it can see'
          : `it could not tell which door "${which}" meant`
      };
    }
    if (door.locked) {
      return { ok: false, note: `the door to ${this.doorLabel(door)} is locked` };
    }
    if (message) {
      await this.say(message).catch(() => undefined);
    }
    // The gateway routes to the door, steps through, and retries: door tiles
    // are excluded from path-finding on purpose, so they can only be walked
    // into. See arena_enter_door.
    const result = await this.arena.call('arena_enter_door', {
      agent_id: this.agentId,
      x: door.x,
      y: door.y
    });
    return result.entered
      ? { ok: true, note: `went through into ${plainSceneName(result.scene)}` }
      : { ok: false, note: `the door did not open: ${result.message ?? result.reason}` };
  }

  /** Match what the character asked for against the doorways it can see. */
  private pickDoor(which: string | undefined): SeenDoor | null {
    const doors = this.doors();
    if (doors.length === 0) {
      return null;
    }
    const wanted = String(which ?? '').trim().toLowerCase();
    if (!wanted) {
      // Only one way out: they meant that one. More than one and a character
      // that did not say which has not really decided.
      return doors.length === 1 ? doors[0] : null;
    }
    const named = doors.find((door) => {
      const label = this.doorLabel(door).toLowerCase();
      return label === wanted || label.includes(wanted) || wanted.includes(label);
    });
    if (named) {
      return named;
    }
    // "the door out", "outside", "back" when there is somewhere it came from.
    return doors.length === 1 ? doors[0] : null;
  }

  async attack(target: string | undefined): Promise<ActionResult> {
    if (!this.can('fight')) {
      return { ok: false, note: 'this character does not fight' };
    }
    if (!target) {
      return { ok: false, note: 'nothing named to hit' };
    }
    await this.arena.call('arena_basic_attack', { agent_id: this.agentId, target });
    return { ok: true, note: `swung at ${target}` };
  }

  async checkMoney(): Promise<ActionResult> {
    if (!this.can('money')) {
      return { ok: false, note: 'this character has no purse' };
    }
    const balance = await this.arena.call('arena_credit_balance', { agent_id: this.agentId });
    return { ok: true, note: `has ${balance.balance} saved` };
  }

  async observe(): Promise<Observation> {
    return this.arena.call('arena_observe', { agent_id: this.agentId });
  }

  /**
   * Watch until the body arrives or stops moving, then let it settle.
   *
   * The settle matters: a body still sliding when the next decision is made
   * reports a position it is about to leave, and the character decides where
   * to go next from where it briefly was. Waiting for it to actually stop
   * costs half a second and makes every following observation true.
   */
  private async waitForArrival(x: number, y: number): Promise<boolean> {
    const deadline = Date.now() + LEG_TIMEOUT_MS;
    let last: string | null = null;
    let still = 0;
    while (Date.now() < deadline) {
      await sleep(WALK_POLL_MS);
      const observation = await this.observe();
      const state = observation.ownPlayer?.state ?? {};
      const here = { x: Number(state.x ?? 0), y: Number(state.y ?? 0) };
      if (Math.abs(here.x - x) <= ARRIVAL_PIXELS && Math.abs(here.y - y) <= ARRIVAL_PIXELS) {
        await sleep(SETTLE_MS);
        return true;
      }
      // Leaving the room mid-walk counts as done; something else moved us.
      if (sceneOf(observation) === '') {
        return false;
      }
      const key = `${here.x},${here.y}`;
      if (key === last) {
        still += 1;
        if (still >= STILL_POLLS) {
          return false;
        }
      } else {
        still = 0;
      }
      last = key;
    }
    return false;
  }
}

export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * Turn what a character meant to say into the chat lines it actually sends.
 *
 * Two separate limits are at work. How much a character says is a trait: an
 * innkeeper who has told the same story for twenty years runs on, and a
 * wanderer who has spoken to nobody for a week does not. That is `maxWords`,
 * and it is enforced by dropping whole sentences, never by cutting one short.
 *
 * How much fits in one chat line is Reldens': 100 characters. So the kept
 * sentences are packed into lines under that limit and sent in sequence, which
 * is what a person typing in a chat box does anyway.
 */
export function toSpeech(raw: string, maxWords: number = DEFAULT_WORDS): string[] {
  // Models narrate themselves in asterisks however firmly they are told not
  // to. Cut what is between them, not just the markers: stripping only the
  // asterisks turns "*Guy shrugs.* Fine." into a character announcing that he
  // shrugs, which is worse than the stage direction was.
  const text = raw
    .replace(/\*[^*]*\*/g, ' ')
    .replace(/\*/g, '')
    .replace(/\s+/g, ' ')
    .trim()
    .replace(/^["“”]|["“”]$/g, '');
  if (!text) {
    return [];
  }
  const budget = Math.max(1, Math.min(Math.round(maxWords), MAX_WORDS));
  const kept: string[] = [];
  let spent = 0;
  for (const sentence of splitSentences(text)) {
    const words = countWords(sentence);
    // The first sentence always goes, however long: a character cut off
    // before it has said anything is worse than one that ran over.
    if (kept.length > 0 && spent + words > budget) {
      break;
    }
    kept.push(sentence);
    spent += words;
    if (spent >= budget) {
      break;
    }
  }
  return packIntoLines(kept).slice(0, MAX_LINES);
}

/** Words too common to say anything about what a line is about. */
const FILLER = new Set([
  'the', 'a', 'an', 'and', 'but', 'or', 'so', 'is', 'it', 'its', 'was', 'be',
  'been', 'to', 'of', 'in', 'on', 'at', 'for', 'with', 'that', 'this', 'there',
  'here', 'you', 'your', 'i', 'im', 'me', 'my', 'we', 'they', 'he', 'she',
  'not', 'no', 'yes', 'do', 'does', 'did', 'have', 'has', 'had', 'will',
  'would', 'can', 'could', 'if', 'as', 'up', 'out', 'about', 'just', 'like',
  'what', 'who', 'how', 'why', 'when', 'where', 'still', 'got', 'get', 'one'
]);

function contentWords(line: string): Set<string> {
  return new Set(
    line
      .toLowerCase()
      .replace(/[^a-z\s]/g, ' ')
      .split(/\s+/)
      .filter((word) => word.length > 2 && !FILLER.has(word))
  );
}

/**
 * Whether a character is about to say something it has effectively just said.
 *
 * Exact-match checks catch nothing, because a model never repeats itself
 * word for word: it says "the road's the same as ever" and then "same road as
 * always" and sounds like a broken toy. Comparing what a line is *about*
 * catches that.
 */
export function isTooSimilar(line: string, recent: string[], threshold = 0.6): boolean {
  const words = contentWords(line);
  if (words.size === 0) {
    return recent.some((said) => said.toLowerCase() === line.toLowerCase());
  }
  for (const said of recent) {
    const before = contentWords(said);
    if (before.size === 0) {
      continue;
    }
    let shared = 0;
    for (const word of words) {
      if (before.has(word)) {
        shared++;
      }
    }
    // Against the shorter line, so a long rambling repeat of a short remark
    // still counts as the same remark.
    if (shared / Math.min(words.size, before.size) >= threshold) {
      return true;
    }
  }
  return false;
}

/**
 * A subject a character keeps coming back to. Naming it in the prompt works
 * far better than a general plea not to repeat itself, which models agree to
 * and then ignore.
 */
export function harpingOn(recent: string[], minLines = 3): string {
  if (recent.length < minLines) {
    return '';
  }
  const counts = new Map<string, number>();
  for (const line of recent.slice(-6)) {
    for (const word of contentWords(line)) {
      counts.set(word, (counts.get(word) ?? 0) + 1);
    }
  }
  const stuck = [...counts.entries()]
    .filter(([, count]) => count >= minLines)
    .sort((left, right) => right[1] - left[1])
    .slice(0, 2)
    .map(([word]) => word);
  return stuck.length === 0
    ? ''
    : `You have brought up ${stuck.map((word) => `"${word}"`).join(' and ')} in most of `
      + 'your last few lines. Talk about something else, or say nothing.';
}

function splitSentences(text: string): string[] {
  const sentences: string[] = [];
  let current = '';
  for (const character of text) {
    current += character;
    if ('.!?'.includes(character)) {
      sentences.push(current.trim());
      current = '';
    }
  }
  if (current.trim()) {
    sentences.push(current.trim());
  }
  return sentences;
}

function countWords(text: string): number {
  return text.split(/\s+/).filter(Boolean).length;
}

/** Fill each chat line as full as it will go, breaking only between words. */
function packIntoLines(sentences: string[]): string[] {
  const lines: string[] = [];
  let line = '';
  const push = () => {
    if (line) {
      lines.push(line);
      line = '';
    }
  };
  for (const sentence of sentences) {
    for (const piece of sentence.length > CHAT_LINE_LIMIT ? breakUp(sentence) : [sentence]) {
      const candidate = line ? `${line} ${piece}` : piece;
      if (candidate.length > CHAT_LINE_LIMIT) {
        push();
        line = piece;
        continue;
      }
      line = candidate;
    }
  }
  push();
  return lines;
}

/** A sentence too long for one line, split on words as late as it can be. */
function breakUp(sentence: string): string[] {
  const pieces: string[] = [];
  let rest = sentence;
  while (rest.length > CHAT_LINE_LIMIT) {
    let cut = rest.lastIndexOf(' ', CHAT_LINE_LIMIT);
    if (cut <= 0) {
      cut = CHAT_LINE_LIMIT;
    }
    pieces.push(rest.slice(0, cut).trim());
    rest = rest.slice(cut).trim();
  }
  if (rest) {
    pieces.push(rest);
  }
  return pieces;
}
