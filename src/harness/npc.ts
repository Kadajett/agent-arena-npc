/**
 * The core an NPC runs on.
 *
 * Every character does the same three things forever: look at the room, answer
 * anyone who spoke to it, and then do whatever its behaviour says to do next.
 * Barnaby, the Wanderer and Guy differ only in the behaviour they are given and
 * the actions they are allowed, so the differences between them live in their
 * own files and not in here.
 *
 * The loop is written to outlive the game as it is today. Fighting and money
 * are already actions; when the world grows something worth hitting or a way to
 * be paid, a character gets that capability and starts using it without this
 * file changing.
 */

import { Agent } from '@mastra/core/agent';
import { ArenaClient, Observation, othersIn, sceneOf, spokenLines } from './arena.js';
import {
  Actions,
  Capability,
  DEFAULT_WORDS,
  Intent,
  harpingOn,
  isTooSimilar,
  sleep,
  toSpeech
} from './actions.js';
import { Explorer, Perception, RoomView, describeDoors } from './explore.js';
import {
  BOOKKEEPING,
  Behavior,
  MemoryScope,
  Situation,
  askForIntent,
  momentOf,
  describeSituation,
  lengthGuidance
} from './behavior.js';
import {
  WorkingMemoryState,
  buildMemory,
  keepMemoryDigested,
  describePeople,
  describePlacesKnown,
  doubtPlace,
  findTodo,
  hasMet,
  noteFace,
  noteGoingsOn,
  memoryScope,
  notePlace,
  recallAbout,
  readMemory,
  restartWhenCompactionJams,
  writeMemory
} from './memory.js';
import { Goal, Plan } from './plan.js';
import { STEPS_PER_TURN, arenaToolbox } from './agentic.js';
import { withPrimer } from './primer.js';
import { withFallback } from './models.js';
import { skillsFor } from './skills.js';
import { learnPrices, meter, note } from './spend.js';
import { markEpisodeUsed, nextEpisode, postToDiscord } from './discord.js';
import { z } from 'zod';
import {
  describeLocalKnowledge,
  describePlaces,
  isHomeTurf,
  plainSceneName,
  sceneNamed
} from './world.js';

export type CharacterSheet = {
  /** Stable id, also the memory resource. */
  id: string;
  /** The name above their head in the world. */
  playerName: string;
  classPath?: string;
  homeScene: string;
  /** The system prompt: who this is. */
  persona: string;
  model: string;
  capabilities: Capability[];
  /** How they spend their time. Built with the agent, so it can use the model. */
  behavior: (agent: Agent) => Behavior;
  /**
   * What they start out trying to bring about, if anything. A seed: it fills an
   * empty memory, and a character given the 'purpose' capability can settle on
   * something else once this is done or hopeless. Editing it here still
   * redirects a character that has chosen its own. See plan.ts.
   */
  goal?: Goal;
  /** Seconds between decisions when idle, and when mid-conversation. */
  pace?: { idle?: number; engaged?: number };
  /**
   * How much they say at a stretch, in words. A trait, not a limit: a talkative
   * innkeeper and a taciturn wanderer are different people, and this is part of
   * how. Capped at MAX_WORDS whatever is set here.
   */
  wordiness?: number;
  /** Whether they remember anything between restarts. */
  remembers?: boolean;
  /**
   * How many past messages ride along on every call. Left unset for almost
   * everybody: see DEFAULT_RECALL in memory.ts for who needs more and what it
   * costs.
   */
  recall?: number;
  /**
   * Facts held in the system message rather than in memory, so they cannot fall
   * out of a window or be overwritten by something a guest asserted
   * confidently. For a character whose job is being right about this world.
   */
  pinned?: string;
  /**
   * Rooms whose real places this character knows by heart without standing in
   * them. For a local, not for everybody: see describeLocalKnowledge().
   */
  localKnowledge?: string[];
};

const RECONNECT_SECONDS = 15;
const DEFAULT_IDLE_SECONDS = 90;
const DEFAULT_ENGAGED_SECONDS = 4;
const RECENT_LINES = 8;
/**
 * How often a character posts a plain-language digest of itself to Discord,
 * for whoever set DISCORD_WEBHOOK_URL - checked once a tick rather than run
 * on its own timer, so it never overlaps the character's own turn and never
 * needs a second connection to anything. Five minutes and not the pace a
 * character's own sheet sets, because a carouser ticking every twelve
 * seconds and a guard ticking every ninety should still report on the same
 * clock; nobody watching a dashboard wants Bolo's version of five minutes
 * to be six times as talkative as Doran's.
 */
const DIGEST_INTERVAL_MS = 5 * 60 * 1000;
/**
 * Attempts allowed within one digest cycle before giving up on it entirely.
 * The language guard was catching roughly half of all attempts and simply
 * dropping them - correct, in that garbage never reached Discord, but it
 * meant a five-minute stretch just vanished from the record instead of
 * being retried inside the same cycle it already paid the interval for.
 * Two independent attempts at a coin-flip failure rate clears it more
 * often than not without meaningfully raising the cost of a cycle that
 * succeeds on the first try.
 */
const DIGEST_MAX_ATTEMPTS = 2;
/**
 * The reply itself is short - one or two sentences - but the budget cannot
 * be cut down to match it. Confirmed against Bolo directly: a 200-token cap
 * came back with 200 tokens spent and no text at all, because a reasoning
 * model burns budget thinking before it writes the answer, and once the cap
 * lands mid-thought there is nothing left to write the answer with. Plan's
 * own no-tool call (replan(), in plan.ts) hits the same wall and is given
 * REPLY_BUDGET's 4096 for it; this is shorter than a plan, not exempt from
 * the reason a plan needs the room.
 */
const DIGEST_BUDGET = { maxOutputTokens: 1024 };
/** One digest, as an outside viewer would file it rather than as loose prose. */
const DigestSchema = z.object({
  title: z.string().min(1).max(80).describe('three to six words, the vibe of the stretch, not a mini-summary'),
  synopsis: z
    .string()
    .min(1)
    .describe(
      'the actual report: at most two short sentences, third person, past '
        + 'tense. Pick the single thing most worth mentioning rather than '
        + 'listing everything that happened - a synopsis, not a transcript'
    )
});
/**
 * The instruction above is a request, not a guarantee - Episode 1 came back
 * as a full paragraph despite being asked for "one or two sentences", so
 * length is also enforced here rather than trusted to the model. Same idea
 * as clip() in memory.ts, kept local because MAX_NOTE_CHARS's reasoning
 * (fits the reply budget, not the room to read it) does not apply here.
 */
const SYNOPSIS_CHAR_LIMIT = 280;

function clipSynopsis(text: string): string {
  if (text.length <= SYNOPSIS_CHAR_LIMIT) {
    return text;
  }
  const cut = text.slice(0, SYNOPSIS_CHAR_LIMIT);
  const lastSpace = cut.lastIndexOf(' ');
  return `${(lastSpace > SYNOPSIS_CHAR_LIMIT * 0.6 ? cut.slice(0, lastSpace) : cut).trimEnd()}...`;
}

/**
 * Rough, not a real language detector: true unless too much of the text is
 * CJK script to plausibly be the English the digest prompt asked for. Cheap
 * on purpose - this exists to catch the specific failure already seen
 * (fluent, well-formed Chinese, twice running), not to referee prose.
 */
function looksEnglish(text: string): boolean {
  const han = text.match(/[\u4e00-\u9fff]/g)?.length ?? 0;
  return han <= text.length * 0.1;
}
/**
 * How much of the conversation a character carries in its head.
 *
 * This was fifty, on the reasoning that the models are cheap and losing the
 * thread is the expensive failure. Both halves were true and the sum was not:
 * the transcript goes into every prompt, so fifty lines is fifty lines paid for
 * on every tick, forever, by every character. Twenty covers the exchange a
 * character is actually in, and what matters beyond it has already been written
 * into memory, which is what memory is for.
 */
const TRANSCRIPT_LINES = 20;
const MEMORY_DIR = process.env.NPC_MEMORY_DIR ?? '/npc/var';

/**
 * How many recent moves count as "lately" when deciding somebody is circling.
 * Eight is long enough to catch a there-and-back-again and short enough that a
 * character which has genuinely moved on stops being nagged about it.
 */
const CIRCLING_WINDOW = 8;

/** How many lines of a room's own conversation come back on walking in again. */
const ROOM_LINES = 4;

/** Plain English for how long ago, because "1786376624125" tells nobody anything. */
function howLongSince(when: number): string {
  const minutes = Math.round((Date.now() - when) / 60_000);
  if (minutes < 1) {
    return 'a moment ago';
  }
  if (minutes < 60) {
    return `about ${minutes} minute${1 === minutes ? '' : 's'} ago`;
  }
  const hours = Math.round(minutes / 60);
  return `about ${hours} hour${1 === hours ? '' : 's'} ago`;
}

function log(...parts: unknown[]): void {
  console.log(new Date().toISOString().slice(11, 19), ...parts);
}

/** Same job as the private one in plan.ts: a reply that is not JSON is not a crash. */
function safeJson(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

/** Which tile the character is standing on, for saying which way a door lies. */
function tileOf(observation: Observation): { row: number; column: number } | null {
  const state = observation.ownPlayer?.state;
  const x = Number(state?.x);
  const y = Number(state?.y);
  if (!Number.isFinite(x) || !Number.isFinite(y)) {
    return null;
  }
  return { row: Math.floor(y / 32), column: Math.floor(x / 32) };
}

export class Npc {
  private readonly agent: Agent;
  private readonly behavior: Behavior;
  private readonly seen = new Set<string>();
  private readonly recentlySaid: string[] = [];
  /** Everything said in earshot lately, this character's own lines included. */
  private readonly transcript: Array<{ from: string; message: string; fresh: boolean }> = [];
  private notes: string[] = [];
  private readonly memory: MemoryScope;
  private readonly wordiness: number;
  /** The character's own todo list toward its goal, kept across restarts. */
  private readonly plan: Plan;
  /** What it can see of each room it has stood in. */
  private readonly perception = new Perception();
  /** Which corners of which rooms it has already been to. */
  private readonly explorer = new Explorer();
  /**
   * Rooms it has already written down this run, so it does not rewrite one
   * every tick. Checked against memory as well, because this set says nothing
   * about whether the write survived: the model can overwrite the whole record
   * from under it. See noteWhereItIs().
   */
  private readonly recorded = new Set<string>();
  /** Who this character is, kept because every model call now overrides it. */
  private readonly persona: string;
  /**
   * Faces already written down this run, keyed by room, so a bar full of
   * regulars is not rewritten to memory every twelve seconds. Guarded the same
   * way rooms are, and for the same reason - the model can overwrite the whole
   * record - by noteWhoIsHere() re-reading memory through remit().
   */
  private readonly faces = new Set<string>();
  /** The last action+note that failed, and how many times running. */
  /** The last few moves, for noticing a character crossing its own tracks. */
  private readonly lately: string[] = [];
  /** The character's Mastra memory, when it has one. See the constructor. */
  private readonly recollection?: ReturnType<typeof buildMemory>;
  /**
   * The gateway's MCP tools, filtered to this character's capabilities and
   * bound to its agent_id. Filled in by live() once the character is
   * registered, because the binding needs the agent_id and the Agent needs
   * the reference at construction - which is why the Agent gets a function.
   * Empty means not connected yet, and an agentic turn with no tools is a
   * turn that can only talk to itself, so live() fills this before the first
   * tick.
   */
  private toolbox: Record<string, unknown> = {};
  /** Per room: when it was last stood in, and what was said there. */
  private readonly rooms = new Map<string, { lastHere: number; said: string[] }>();
  private standingIn = '';
  private lastFailure = '';
  private failedWith = 0;
  /** What it has thought and done since the last Discord digest, raw. */
  private readonly sinceDigest: string[] = [];
  private lastDigestAt = Date.now();

  constructor(private readonly sheet: CharacterSheet) {
    this.wordiness = sheet.wordiness ?? DEFAULT_WORDS;
    this.persona = withPrimer(sheet.persona + (sheet.pinned ? `\n\n${sheet.pinned}` : ''));
    // Kept on the instance rather than inlined into the Agent, because the
    // harness has to drive observation itself every tick - Mastra never runs
    // it for single-step turns. See keepMemoryDigested() in memory.ts.
    this.recollection =
      sheet.remembers === false
        ? undefined
        : buildMemory(sheet.id, MEMORY_DIR, sheet.recall, sheet.model);
    this.agent = new Agent({
      id: sheet.id,
      name: sheet.playerName,
      // Who they are, and then how to read the world they are standing in.
      // The primer is the same for everyone and never changes, so it lives in
      // the system message rather than being repeated in every situation.
      instructions: this.persona,
      // Never one model. The free router sits underneath whatever this
      // character would rather use, so an empty account makes it think more
      // cheaply instead of making the whole town stand still. See models.ts.
      model: withFallback(sheet.model),
      // The gateway's own MCP tools, arriving after registration; a function
      // because the toolbox is bound to an agent_id this constructor does not
      // have yet. See agentic.ts for what a character gets and why.
      tools: () => this.toolbox as never,
      // Memory that survives a restart. A character who has lived in a town
      // for years and forgets you between deploys is worse than one with no
      // memory at all. What it may remember is constrained by a schema, so
      // nothing it learns can rewrite who it is; see memory.ts.
      // Observation runs on the character's own cheap model, not the free
      // router: the free tier's thousand-a-day account cap killed compaction
      // by mid-afternoon and the unobserved backlog cost more than paid
      // observation ever would. See buildMemory().
      ...(this.recollection ? { memory: this.recollection } : {})
    });
    this.behavior = sheet.behavior(this.agent);
    // Every model call now overrides the agent's instructions so the situation
    // can ride along without being stored, which means whoever builds a prompt
    // has to carry the persona with it or the character answers as nobody.
    this.behavior.answersAs?.(this.persona);
    this.memory = memoryScope(sheet.id);
    this.plan = new Plan(
      this.agent,
      this.memory,
      sheet.goal,
      () => this.memoryStore(),
      this.persona
    );
  }





  /**
   * Rooms this character has stood in before today, from memory.
   *
   * Without this a redeploy tells everybody they have never been anywhere.
   * Guy was informed he had walked into town for the first time, having lived
   * there for days, which is worse than saying nothing: it is a confident
   * falsehood, and the door labels are built off the same record.
   *
   * The time is not recovered, only the fact, because memory keeps what a place
   * is rather than when it was last seen. "Been there before" without a time is
   * honest and is the half that matters when choosing a door.
   */
  private async rememberWhereItHasBeen(): Promise<void> {
    const state = await readMemory(await this.memoryStore(), this.memory).catch(() => null);
    for (const place of state?.places ?? []) {
      if (place.how !== 'been') {
        continue;
      }
      const scene = sceneNamed(place.where);
      if (scene && !this.rooms.has(scene)) {
        this.rooms.set(scene, { lastHere: 0, said: [] });
      }
    }
  }

  /**
   * What a character knows about the room it has just walked into.
   *
   * Paid for on arrival and not otherwise, which is the whole point. The
   * alternative, and what this world was doing, is carrying every scrap of
   * context in the rolling history so it happens to be there when needed: an
   * average call was reading thirty thousand tokens to have this much on hand.
   * Walking through a door is rare. Standing in a room is constant. Putting the
   * context on the rare thing is most of the saving available here.
   *
   * It is also better context than the history gave. A character re-reading two
   * hundred messages has to work out for itself which of them happened in this
   * room; this hands it that directly, with how long it has been.
   */
  private arrivedSomewhere(scene: string): string | null {
    const known = this.rooms.get(scene);
    this.rooms.set(scene, { lastHere: Date.now(), said: known?.said ?? [] });
    if (scene === this.standingIn) {
      return null;
    }
    const first = !known;
    this.standingIn = scene;
    const name = plainSceneName(scene);
    if (first) {
      return `[You have walked into ${name}. You have not been here before.]`;
    }
    const lines: string[] = [
      0 === known.lastHere
        ? `[You are back in ${name}, which you have been in before.`
        : `[You are back in ${name}, ${howLongSince(known.lastHere)}.`
    ];
    if (known.said.length > 0) {
      lines.push('Last time you were here:');
      lines.push(...known.said.slice(-ROOM_LINES).map((line) => `  ${line}`));
    } else {
      lines.push('Nothing was said here last time.');
    }
    lines.push(']');
    return lines.join('\n');
  }

  /** Keep what was said where it was said, so walking back in can recall it. */
  private heardHere(scene: string, from: string, message: string): void {
    const room = this.rooms.get(scene) ?? { lastHere: Date.now(), said: [] };
    room.said.push(`${from}: ${message}`);
    if (room.said.length > ROOM_LINES * 2) {
      room.said.splice(0, room.said.length - ROOM_LINES * 2);
    }
    this.rooms.set(scene, room);
  }

  /**
   * Doing the same thing over and over and getting away with it.
   *
   * The failure counter above cannot see this, because none of it fails. A
   * character crossing back and forth through the same doorway is succeeding
   * each time and going nowhere, and the only thing that distinguishes it from
   * somebody with a reason is that nothing about the world changes.
   *
   * Two of the same move in a row is a person changing their mind. Four is a
   * loop, and by then it has usually been running much longer than that.
   */
  private goingInCircles(intent: Intent, alone: boolean): string | null {
    // Every say is one move for this purpose, whatever the words were. The
    // words are always different - that is what a language model is for - and
    // keying on them would make small talk the one rut that never reads as a
    // rut. Guy proved it: fresh greeting after fresh greeting to an empty
    // street, each one a different sentence and all of them the same move.
    const move = intent.action === 'say'
      ? 'say:'
      : `${intent.action}:${(intent.place ?? intent.target ?? '').toLowerCase()}`;
    this.lately.push(move);
    if (this.lately.length > CIRCLING_WINDOW) {
      this.lately.shift();
    }
    if (intent.action === 'wait') {
      // Standing still is allowed to repeat. This is about a character
      // spending itself and getting nowhere.
      return null;
    }
    if (intent.action === 'say' && !alone) {
      // Talking to people who are actually there is conversation, however
      // long it runs. Barnaby's whole job lives in this branch.
      return null;
    }
    const same = this.lately.filter((one) => one === move).length;
    if (same < 3) {
      return null;
    }
    if (intent.action === 'say') {
      return (
        `[You have now said your piece ${same} times in the last ${this.lately.length} moves `
        + `and there is nobody here to hear any of it. Talking is not doing. Go where the `
        + `people are, or the fights, or the thing you said you were after.]`
      );
    }
    return (
      `[You have done this ${same} times in the last ${this.lately.length} moves and you are `
      + `exactly where you started. Whatever you are looking for is not through there, or you `
      + `would have found it by now. Go somewhere you have not been, or get on with what you `
      + `actually said you were going to do.]`
    );
  }

  /**
   * The same failure twice is worth saying twice as plainly.
   *
   * A character that hears "there is no door here called the pantry" once may
   * reasonably try a slightly different phrasing. One that has heard it four
   * times is not going to get a different answer, and telling it so in the same
   * neutral words each time is how a loop stays a loop.
   */
  private saidBefore(action: string, note: string): string {
    const same = `${action}:${note}`;
    this.failedWith = same === this.lastFailure ? this.failedWith + 1 : 1;
    this.lastFailure = same;
    if (this.failedWith < 3) {
      return note;
    }
    return (
      `${note}\n[You have now tried this ${this.failedWith} times and been told the same `
      + `thing every time. It is not going to work. Whatever you were trying to do this `
      + `way cannot be done this way, so either find another way or give it up and do `
      + `something else.]`
    );
  }

  private memoryStore(): Promise<any> {
    return Promise.resolve(this.agent.getMemory?.());
  }


  async run(): Promise<void> {
    // Warm the price list before anybody thinks, so the very first call is
    // costed rather than reported as "price unknown". One request, once.
    await learnPrices();
    // Compaction jams on the first tool call that comes back failed and stays
    // jammed, so a character carrying one bad part never folds its history
    // down again. Repairing at startup alone was not enough - Cutter poisoned
    // himself three minutes into his first boot - and repairing on a timer
    // broke the Wanderer outright, because a second connection to a database
    // the character already has open is not a repair. Leaving and coming back
    // is: the repair at startup then has the file to itself.
    restartWhenCompactionJams((why) => {
      log(`${why}; leaving so it can be put right on the way back in`);
      // Give the line a moment to reach the logs before the process goes.
      setTimeout(() => process.exit(0), 250).unref();
    });
    for (;;) {
      try {
        await this.live();
      } catch (error) {
        log('reconnecting after error:', (error as Error)?.message ?? error);
        await sleep(RECONNECT_SECONDS * 1000);
      }
    }
  }

  private async live(): Promise<void> {
    const arena = new ArenaClient();
    await arena.start();
    const agentId = await this.ensureRegistered(arena);
    await arena.call('arena_login', { agent_id: agentId });
    // The character's own hands: the gateway's MCP tools, capability-filtered
    // and bound to this agent_id. Built after login because the gateway's
    // session is keyed by agent, so the tools land on the live session the
    // login above just established. A failure here throws to run()'s
    // reconnect loop the same as any other connection problem.
    this.toolbox = await arenaToolbox({
      url: process.env.ARENA_MCP_URL ?? 'https://mcp.yougotserved.dev/mcp',
      apiKey: process.env.ARENA_API_KEY ?? '',
      agentId,
      capabilities: this.sheet.capabilities
    });
    log(
      `${this.sheet.playerName} is in the world, holding ${Object.keys(this.toolbox).length} tools:`,
      Object.keys(this.toolbox).join(', ')
    );

    const actions = new Actions(
      arena,
      agentId,
      new Set(this.sheet.capabilities),
      this.wordiness,
      this.explorer,
      // Without this every character reported having no skills of its own and
      // use_skill could never fire, which made the whole thing look like it
      // worked in tests and did nothing in the world.
      skillsFor(this.sheet.classPath)
    );
    if (actions.can('money')) {
      await this.refreshSavings(actions);
    }
    // Pick the plan back up where it was left, which after a deploy is usually
    // partway through something.
    await this.rememberWhereItHasBeen();
    await this.plan.load();
    if (this.plan.hasGoal) {
      log(`after: ${this.plan.goal?.aim}${this.plan.goalIsOwn ? ' (its own idea)' : ''}`);
      const current = this.plan.current();
      log(current ? `still working on: ${current.what}` : 'no plan yet');
    }

    for (;;) {
      const observation = await actions.observe();
      const scene = sceneOf(observation) || this.sheet.homeScene;
      // Look around, every tick. What is in front of a character changes as it
      // walks, so this is a window that travels with it rather than a snapshot
      // of the doorway it came in by. It is also what tells it which doorways
      // exist, without anybody having written them down.
      const view = await this.perception.look(arena, agentId, scene);
      actions.sees(view);
      // Who and what is standing here, from the same observation already in
      // hand, so naming somebody to talk to or hit costs no extra round trip.
      actions.notices(observation.objects);
      // And the people, with the ids a duel needs to aim at one of them.
      actions.meets(observation.players);
      // What is in its pockets and what is on the floor. Both ride along with
      // the observation, so a character knowing what it is holding costs
      // nothing extra and can be true on every tick rather than only after it
      // has thought to look.
      actions.holds(observation.carrying, observation.drops);
      // What is behind each door, for a character choosing which to take. Built
      // fresh each tick from where it has actually stood, never from anything
      // the model claimed.
      actions.remembersRooms(
        new Map(
          [...this.rooms].map(([where, room]) => [where, howLongSince(room.lastHere)])
        )
      );
      const arrived = this.arrivedSomewhere(scene);
      if (arrived) {
        // Into the notes here, before the situation is built, because this is
        // the tick it is true on. Added after the action instead, it arrived a
        // tick late and told a character it had just walked into the room it
        // had by then already left.
        log('walked in somewhere:', arrived.split('\n')[0].replace(/^\[/, ''));
        note(this.sheet.playerName, 'arrived', arrived.split('\n')[0].replace(/^\[/, ''));
        this.notes = [...this.notes, arrived];
      }
      await this.noteWhereItIs(scene, view, observation);
      await this.noteWhoIsHere(othersIn(observation, this.sheet.playerName), scene);
      const heard = this.freshLines(observation);
      // Whatever was new last time around is old news now.
      for (const line of this.transcript) {
        line.fresh = false;
      }
      for (const line of heard) {
        this.record(line.from, line.message);
        this.heardHere(scene, line.from, line.message);
      }
      const situation: Situation = {
        scene,
        where: scene.replace('reldens-', '').replace(/-/g, ' '),
        others: await this.describeOthers(othersIn(observation, this.sheet.playerName)),
        heard,
        actions: actions.describe(scene),
        places: describePlaces(scene),
        conversation: this.transcript.map((line) => ({ ...line })),
        wordiness: this.wordiness,
        purpose: '',
        notes: [...this.notes],
        people: await this.knownPeople(),
        // What it knows about the world beyond this room, and what it has only
        // been told. The gap between the two is what sends a character across
        // town to find out for itself.
        known: this.whatItKnows(scene, await this.recall()),
        // Somewhere it has never stood, so it knows to look rather than to
        // pretend it remembers.
        strange: !isHomeTurf(scene) && this.explorer.cornersKnown(scene) <= 1,
        doors: describeDoors(view, plainSceneName, tileOf(observation)),
        view: view?.map ?? '',
        carrying: actions.carryingLine(),
        harping: harpingOn(this.recentlySaid)
      };
      // Make a plan when the last one is finished or has stopped working. This
      // is what keeps a character pointed at the same thing over weeks: it does
      // not rethink its purpose every twelve seconds, only its next few steps
      // when the ones it had run out.
      if (this.plan.hasGoal && (await this.plan.refresh(describeSituation(situation)))) {
        const current = this.plan.current();
        log('new plan ->', current?.what ?? 'nothing');
      }
      situation.purpose = this.plan.describe();

      // One agentic turn: the situation goes in once, and the character acts
      // through the gateway's own tools until it is done or out of steps,
      // seeing every result in-band. This is the whole replacement for the
      // intent-parse-execute-correct pipeline below it, which stays only
      // until every character has proven out on this path.
      await this.liveOneTurn(situation);
      // Fold history down when it is due. Multi-step turns give Mastra's own
      // observation trigger a real chance to run in-band; this stays as the
      // backstop for quiet stretches. Fire-and-forget with a busy-guard
      // inside; see keepMemoryDigested().
      void keepMemoryDigested(this.recollection, this.memory).then((digest) => {
        if (digest.did === 'observed' || digest.did === 'failed') {
          log(`memory digestion ${digest.did}: ${digest.note ?? ''}`);
          note(this.sheet.playerName, digest.did === 'observed' ? 'observed' : 'failed', digest.note ?? 'memory digestion');
        }
      });

      // Put back whatever the model's own memory write may have dropped. It
      // costs one local SQLite write per tick and it is the only thing standing
      // between a character and quietly losing everything the harness knows
      // about it every time it says something.
      await this.plan.keep();

      await this.maybeDigest();

      const pace = this.sheet.pace ?? {};
      await sleep(
        1000 *
          (heard.length > 0
            ? pace.engaged ?? DEFAULT_ENGAGED_SECONDS
            : pace.idle ?? DEFAULT_IDLE_SECONDS)
      );
    }
  }

  /**
   * Keep what somebody said about a place it has never been.
   *
   * This is the whole social half of exploring. Guy comes down from upstairs
   * and says there is nothing up there; the Wanderer, who has never been up,
   * now knows of a room he has only been told about, by name, with Guy's name
   * on it. That is a thing to go and check, and checking it is worth talking
   * about either way.
   */
  private async noteHearsay(
    intent: Intent,
    heard: Array<{ from: string; message: string }>
  ): Promise<void> {
    const noted = intent.noted?.trim();
    const notThere = intent.notThere?.trim();
    const from = heard.at(-1)?.from ?? 'somebody';
    if (noted) {
      await this.remit((state) =>
        notePlace(state, { where: noted, what: `${from} says so`, how: 'heard', who: from })
      );
      log('noted:', `${noted} (from ${from})`);
    }
    // Somebody looked and found nothing. Either this character did, or it just
    // heard somebody say they had; both count against the rumour, which is what
    // stops one confident remark about a guildhall circulating forever.
    if (notThere) {
      await this.remit((state) => doubtPlace(state, notThere));
      log('found nothing at:', notThere);
    }
  }

  /**
   * Something an NPC actually said to this character, straight into memory
   * as a thing that happened - not a place somebody mentioned in passing,
   * which stays hearsay until this character goes and checks for itself (see
   * noteHearsay() and standingOf() in memory.ts), but a fact gathered
   * first-hand because it was standing right there in the conversation.
   * Alfred telling this character something is not a rumour about Alfred;
   * the character was there. That is what lets it carry the fact on to
   * somebody else rather than only ever repeating that Alfred said something.
   *
   * Deliberately unconditional, the same as noteFace() and noteWhereItIs():
   * whether a reply is worth keeping is not the model's call to make in the
   * middle of the conversation it is having.
   */
  private async noteToldByNpc(told: { from: string; said: string } | null): Promise<void> {
    if (!told) {
      return;
    }
    await this.remit((state) => noteGoingsOn(state, `${told.from} told you: ${told.said}`));
    log('told by', told.from, ':', told.said);
  }

  /**
   * A quest, finished: the same first-hand write as noteToldByNpc(), fired
   * from the other end of it. Miles asks for a tree branch and offers a coin;
   * the character takes that on with `askedBy: "Miles"` (see addTodo() in
   * memory.ts), works it however it likes, and the moment it is crossed off
   * this turns "a todo item done" into "something Miles asked for, and you
   * came through" - a fact this character was there for and can now tell
   * whoever it runs into next. Unconditional, the same as noteFace() and
   * noteWhereItIs(): whether finishing a favour is worth mentioning later is
   * not left for the model to remember to say out loud.
   */
  private async noteQuestDone(askedBy: string, what: string): Promise<void> {
    await this.remit((state) => noteGoingsOn(state, `${askedBy} asked you to ${what}, and you did.`));
    log('came through for', askedBy, ':', what);
  }

  /**
   * Let a character decide it wants something else now.
   *
   * Rare and deliberate: it costs the whole plan, and a character that changes
   * its mind every afternoon never gets anywhere. But one that finished what it
   * set out to do, or ran into a wall it cannot get past, has to be able to
   * pick something new or it stands in the town square forever having won.
   */
  private async reconsider(intent: Intent): Promise<void> {
    if (intent.action !== 'set_goal') {
      return;
    }
    const aim = intent.aim?.trim() ?? '';
    if (!aim) {
      log('wanted to change tack but did not say to what');
      return;
    }
    const was = this.plan.goal?.aim;
    const changed = await this.plan.setGoal(aim, intent.done ?? '', intent.why ?? '');
    if (changed) {
      log(was ? `done with "${was}". now after: ${aim}` : `decided to go after: ${aim}`);
    }
  }

  /**
   * Everything the character wanted written down while it was doing something
   * else: a note for the next hour, a thing it has taken on, a thing it has
   * finished with. These ride along with any action, so making a note never
   * costs a character a turn of standing still.
   */
  private async keepBooks(intent: Intent): Promise<void> {
    if (intent.remember?.trim()) {
      await this.plan.note(intent.remember);
      log('keeping in mind:', intent.remember.trim());
    }
    if (intent.todo?.trim()) {
      const askedBy = intent.askedBy?.trim();
      await this.plan.take(intent.todo, askedBy);
      log('took on:', intent.todo.trim(), askedBy ? `(for ${askedBy})` : '');
    }
    if (intent.finished?.trim()) {
      // Read who, if anybody, asked for this before it is crossed off - the
      // open-list lookup is gone the moment settle() marks it done.
      const item = findTodo(await this.recall(), intent.finished);
      await this.plan.settle(intent.finished, 'done');
      log('crossed off:', intent.finished.trim());
      if (item?.askedBy) {
        await this.noteQuestDone(item.askedBy, item.what);
      }
    }
    if (intent.gaveUpOn?.trim()) {
      await this.plan.settle(intent.gaveUpOn, 'blocked');
      log('gave up on:', intent.gaveUpOn.trim());
    }
    if (intent.progressOn?.trim() && intent.learned?.trim()) {
      await this.plan.gotSomewhere(intent.progressOn, intent.learned);
      log('progress on', intent.progressOn.trim(), '->', intent.learned.trim());
    }
    if (intent.recall?.trim()) {
      // Answered into the notes rather than returned, because the character is
      // in the middle of doing something else and this is the harness thinking
      // on its behalf. It reads it a moment later, which is about right.
      const answer = recallAbout(await this.recall(), intent.recall);
      if (answer) {
        this.notes = [...this.notes.filter((note) => !note.startsWith('Thinking back')), answer];
      }
      log('thought back on:', intent.recall.trim());
    }
  }

  /**
   * Whether this is worth saying out loud. A model never repeats itself word
   * for word, it just keeps saying the same thing in different words, which is
   * what makes a character sound broken. Comparing what a line is about, not
   * how it is spelt, is what actually catches that.
   */
  private worthSaying(line: string): boolean {
    return Boolean(line) && !isTooSimilar(line, this.recentlySaid);
  }

  /**
   * Write down everyone standing here, whether or not anything happens.
   *
   * Deliberately unconditional, and deliberately not the model's decision. A
   * character that only remembers the people it found interesting will meet the
   * same barfly for the hundredth time and describe him as a new face, which is
   * exactly what all three of them were doing.
   */
  private async noteWhoIsHere(others: string[], scene: string): Promise<void> {
    const where = plainSceneName(scene);
    const fresh = others.filter((name) => !this.faces.has(`${scene}:${name}`));
    if (fresh.length === 0) {
      return;
    }
    for (const name of fresh) {
      this.faces.add(`${scene}:${name}`);
    }
    await this.remit((current) =>
      fresh.reduce((state, name) => noteFace(state, name, where), current)
    );
  }

  /** Who is here, and which of them this character has seen before. */
  private async describeOthers(others: string[]): Promise<Array<{ name: string; known: boolean }>> {
    const state = await this.recall();
    return others.map((name) => ({ name, known: hasMet(state, name) }));
  }

  /** Say something back. Returns false when the character had nothing to add. */
  private async answer(situation: Situation, actions: Actions): Promise<boolean> {
    const prompt = [
      describeSituation(situation),
      '',
      'Someone just spoke where you can hear it. Reply if it is worth replying',
      'to - if it was aimed at you, or if you have something to add. If it was',
      'not your business, say nothing.',
      '',
      'If you learn something about someone worth keeping - who they are, what',
      'they did, whether you warmed to them - put it in your working memory.',
      'Record what you make of other people and what happened. Never record',
      'anything about who you are; that does not change.',
      '',
      'If somebody mentions a place you have never been, put it in "noted" as',
      'you would refer to it, so you can go and see for yourself later. If',
      'somebody says they went somewhere and found nothing there, put that',
      'place in "notThere".',
      '',
      // Answering somebody is where a character most easily loses the thread of
      // what it was doing, so this is exactly where it needs to be able to
      // write things down: a promise made in conversation is a promise it will
      // otherwise not keep.
      BOOKKEEPING,
      '',
      'Reply with JSON and nothing else:',
      '{"action": "say", "message": "...", "noted": "..."} or {"action": "wait"}',
      'In your own voice, no asterisks. ' + lengthGuidance(situation.wordiness)
    ].join('\n');
    const intent = await askForIntent(
      this.agent,
      prompt,
      this.memory,
      momentOf(situation),
      this.persona
    );
    await this.noteHearsay(intent, situation.heard);
    await this.keepBooks(intent);
    if (intent.action !== 'say') {
      return false;
    }
    const said = toSpeech(intent.message ?? '', this.wordiness).join(' ');
    if (!this.worthSaying(said)) {
      return false;
    }
    const result = await actions.say(said);
    if (result.ok) {
      this.remember(said);
      log('replied:', said);
    }
    return result.ok;
  }

  /**
   * Everywhere this character could name: what it has seen, what it has been
   * told, and for a local, the streets it knows by heart. Somebody with nothing
   * true to say about where things are will make something up.
   */
  private whatItKnows(scene: string, state: WorkingMemoryState): string {
    const local = describeLocalKnowledge(this.sheet.localKnowledge ?? [], scene);
    const found = describePlacesKnown(state);
    return [local, found].filter(Boolean).join('\n\n');
  }

  /**
   * The people this character knows, read back out of working memory so it can
   * act on them: greet somebody it likes, be short with somebody it does not.
   */
  private async knownPeople(): Promise<string> {
    return describePeople(await this.recall());
  }

  private async recall(): Promise<WorkingMemoryState> {
    return readMemory(await this.memoryStore(), this.memory);
  }

  private async remit(change: (state: WorkingMemoryState) => WorkingMemoryState): Promise<void> {
    const state = await this.recall();
    await writeMemory(await this.memoryStore(), this.memory, change(state));
  }

  /**
   * Write down that it has been here. Standing somewhere yourself beats
   * anything you were told about it, so this settles any rumour it had been
   * carrying about this room.
   */
  private async noteWhereItIs(
    scene: string,
    view: RoomView | null,
    observation: Observation
  ): Promise<void> {
    const state = observation.ownPlayer?.state;
    if (Number.isFinite(Number(state?.x))) {
      this.explorer.markHere(scene, Number(state?.x), Number(state?.y));
    }
    // Both conditions, and the memory one is not redundant. Working memory is a
    // single record and the model can write it too; when it does, it writes the
    // whole thing as it understands it, which silently drops every field it was
    // not shown. Barnaby lost his own inn that way and, guarded only by the set
    // above, never wrote it again for the life of the process.
    const known = (await this.recall()).places.some(
      (place) => place.where.trim().toLowerCase() === plainSceneName(scene).trim().toLowerCase()
    );
    if (this.recorded.has(scene) && known) {
      return;
    }
    this.recorded.add(scene);
    const ways = (view?.doors ?? [])
      .map((door) => (door.leadsTo ? plainSceneName(door.leadsTo) : null))
      .filter((where): where is string => Boolean(where));
    await this.remit((current) =>
      notePlace(current, {
        where: plainSceneName(scene),
        what: ways.length > 0 ? `you have been in. Doors to ${ways.join(', ')}.` : 'you have been in',
        how: 'been'
      })
    );
  }

  /** Note something this character said, so it hears its own side of it too. */
  private remember(line: string): void {
    const text = line.trim();
    if (!text) {
      return;
    }
    this.recentlySaid.push(text);
    while (this.recentlySaid.length > RECENT_LINES) {
      this.recentlySaid.shift();
    }
    this.record('you', text);
  }

  private record(from: string, message: string): void {
    this.transcript.push({ from, message, fresh: true });
    while (this.transcript.length > TRANSCRIPT_LINES) {
      this.transcript.shift();
    }
  }

  private async refreshSavings(actions: Actions): Promise<void> {
    const result = await actions.checkMoney().catch(() => null);
    this.notes = result?.ok ? [result.note] : [];
  }

  /** Lines spoken since the last look, never including this character's own. */
  private freshLines(observation: Observation): Array<{ from: string; message: string }> {
    const fresh: Array<{ from: string; message: string }> = [];
    for (const line of spokenLines(observation)) {
      if (line.from === this.sheet.playerName) {
        continue;
      }
      const key = `${line.at}|${line.from}|${line.message}`;
      if (this.seen.has(key)) {
        continue;
      }
      this.seen.add(key);
      fresh.push({ from: line.from, message: line.message });
    }
    if (this.seen.size > 500) {
      this.seen.clear();
    }
    return fresh;
  }

  /**
   * One agentic turn: everything the character does this tick.
   *
   * The situation rides in the per-call instructions, which are sent and not
   * stored - the same cost lesson this file already paid for once, recorded
   * at askForIntent(): what lands in memory should be the turn, not the
   * scenery. The user message is the small moment line, and the tool calls
   * and their results are stored by Mastra as the multi-step turn they are,
   * which is exactly the shape its observational memory was built to fold.
   *
   * Speech is a tool now. Anything the model writes outside arena_say is
   * private thought, and the guidance says so plainly, because the
   * alternative - broadcasting the model's inner monologue - was this
   * project's very first bug.
   */
  private async liveOneTurn(situation: Situation): Promise<void> {
    // Written to the character, not to a model operating one. The tool layer
    // is plumbing and stays invisible: walking is walking, speaking is
    // speaking, and the one mechanical fact worth stating - words outside
    // arena_say are unvoiced thought - is stated once, plainly, because its
    // opposite (broadcasting inner monologue) was this project's first bug.
    const guidance = [
      'Live the next moment. Do what you would actually do: look closer, walk',
      'where you mean to go, swing at what needs hitting, say what you have to',
      'say out loud (arena_say is your voice; words written anywhere else are',
      'thoughts, and nobody hears them). React to how the world answers you - a',
      'door that refuses, a swing that lands, a price you cannot pay. Stop when',
      'the moment is spent rather than acting for the sake of it.',
      lengthGuidance(situation.wordiness)
    ].join('\n');
    try {
      const response = await this.agent.generate(momentOf(situation), {
        memory: this.memory,
        // Replaces the agent's instructions rather than adding to them, so the
        // persona has to come along or the character acts as nobody.
        instructions: [this.persona, describeSituation(situation), guidance].join('\n\n'),
        maxSteps: STEPS_PER_TURN,
        modelSettings: { temperature: 0.3 }
      });
      meter(this.sheet.playerName, 'living', response);
      const called = (response as { toolCalls?: Array<{ toolName?: string }> }).toolCalls ?? [];
      const names = called.map((call) => call?.toolName ?? 'tool').join(', ');
      const thought = String((response as { text?: string }).text ?? '').trim();
      log(`turn: ${called.length} action(s)${names ? ` (${names})` : ''}`);
      if (thought) {
        log('thought:', thought.slice(0, 160));
      }
      note(this.sheet.playerName, 'did', `turn: ${names || 'nothing but thinking'}`);
      // Only worth keeping if something is actually going to read it.
      // maybeDigest() only clears this when DISCORD_WEBHOOK_URL is set, so
      // buffering unconditionally left it growing for the life of the
      // process on any deployment that never turns the feature on at all.
      if (process.env.DISCORD_WEBHOOK_URL) {
        this.sinceDigest.push(
          thought
            ? `${names ? `[${names}] ` : ''}${thought}`
            : `[${names || 'nothing but thinking'}]`
        );
      }
      // The notes were delivered with the situation this turn was given; a
      // correction that has been seen once should not nag forever.
      this.notes = [];
    } catch (error) {
      const why = String((error as Error)?.message ?? error).slice(0, 200);
      log('turn failed:', why);
      note(this.sheet.playerName, 'failed', `turn: ${why}`);
    }
  }

  /**
   * Post what this character has been up to, in plain language, to Discord -
   * if DISCORD_WEBHOOK_URL is set and five minutes have actually passed.
   * Read fresh from the environment rather than cached at startup, so
   * turning the webhook on or off only ever needs a redeploy, not a code
   * change. Silent when there is nothing to say: a quiet five minutes is
   * not worth a line in a channel meant for what happened.
   *
   * The call this makes is read-only and tool-free, the same shape Plan
   * uses to think without acting - see replan() in plan.ts - because summing
   * up the last five minutes is not itself a moment to live through.
   */
  private async maybeDigest(): Promise<void> {
    const webhookUrl = process.env.DISCORD_WEBHOOK_URL;
    if (!webhookUrl || Date.now() - this.lastDigestAt < DIGEST_INTERVAL_MS) {
      return;
    }
    const raw = [...this.sinceDigest];
    this.sinceDigest.length = 0;
    this.lastDigestAt = Date.now();
    if (raw.length === 0) {
      return;
    }
    for (let attempt = 1; attempt <= DIGEST_MAX_ATTEMPTS; attempt++) {
      const filed = await this.attemptDigest(raw);
      if (filed) {
        const episode = nextEpisode(MEMORY_DIR, this.sheet.id);
        log(`digest: episode ${episode} - ${filed.title} - ${filed.synopsis}`);
        const delivered = await postToDiscord(
          webhookUrl,
          `**${this.sheet.playerName} — Episode ${episode}: ${filed.title}**\n${filed.synopsis}`
        );
        // Only spend the episode number on a post that actually landed.
        // Discord refusing or timing out is not the same as nothing worth
        // filing - it is a delivery failure, and marking it used anyway
        // would drop this episode forever and leave a silent gap in the
        // numbering, discovered later with no way to tell what was lost.
        if (delivered) {
          markEpisodeUsed(MEMORY_DIR, this.sheet.id, episode);
        } else {
          log(`digest: episode ${episode} was not delivered, its number is still free`);
        }
        return;
      }
      if (attempt < DIGEST_MAX_ATTEMPTS) {
        log(`digest: attempt ${attempt} of ${DIGEST_MAX_ATTEMPTS} unusable, trying again`);
      }
    }
    log(`digest: gave up after ${DIGEST_MAX_ATTEMPTS} attempts, this cycle goes unrecorded`);
  }

  /** One try at filing an episode from the raw record. Null on any failure - see the callers of looksEnglish(). */
  private async attemptDigest(raw: string[]): Promise<{ title: string; synopsis: string } | null> {
    try {
      const prompt = [
        `Below is a raw record of what ${this.sheet.playerName} has thought`,
        'and done over the last few minutes, one line per moment:',
        '',
        raw.join('\n'),
        '',
        `File this as one episode about ${this.sheet.playerName}, for somebody`,
        'glancing at a dashboard who was not there and cannot see the lines',
        'above. Nothing claimed that is not actually shown in them. Keep the',
        'synopsis short - two sentences at the very most, the one thing',
        'worth knowing, not a recap of every line above. Write both fields',
        'in English, regardless of what language anything above is in.',
        '',
        'Reply with JSON and nothing else:',
        '{"title": "...", "synopsis": "..."}'
      ].join('\n');
      const response = await this.agent.generate('[filing an episode for the last few minutes]', {
        memory: { ...this.memory, options: { readOnly: true } },
        toolChoice: 'none',
        instructions: `${this.persona}\n\n${prompt}`,
        // Temperature pinned, same as the living turn - the one setting
        // DIGEST_BUDGET does not cover. Left to the provider's own default
        // this call came back fluent, well-formed, and entirely in Chinese
        // twice running: a known failure mode of reasoning-model families
        // trained heavily on Chinese data, more likely to surface on a
        // short, tool-free, analytical call like this one than on the
        // living turn, which is longer, has tools, and is pinned already.
        modelSettings: { ...DIGEST_BUDGET, temperature: 0.3 },
        // This call is awaited straight from the tick loop, same as the
        // living turn's own generate() - but unlike that one, a stalled
        // digest is not the character's actual turn, and has no business
        // holding the whole loop hostage if the provider hangs. Same
        // budget arena.ts gives its own gateway calls, see REQUEST_TIMEOUT_MS.
        abortSignal: AbortSignal.timeout(60_000)
      });
      meter(this.sheet.playerName, 'digesting', response);
      const text = String((response as { text?: string }).text ?? '');
      const json = text.slice(text.indexOf('{'), text.lastIndexOf('}') + 1);
      const parsed = json ? DigestSchema.safeParse(safeJson(json)) : null;
      if (parsed?.success && looksEnglish(parsed.data.title) && looksEnglish(parsed.data.synopsis)) {
        return { title: parsed.data.title, synopsis: clipSynopsis(parsed.data.synopsis.trim()) };
      }
      if (parsed?.success) {
        // Well-formed JSON, asked for in English, answered in another
        // script anyway - the instruction asking nicely is not the actual
        // guarantee here, the same lesson clipSynopsis() already learned
        // about "two sentences at the very most".
        log('digest: model answered in the wrong language:', parsed.data.title);
      } else {
        // Distinct from "nothing happened" (raw.length === 0 in the caller,
        // which never gets this far). This spent real tokens and still came
        // back with nothing usable, which is a budget or model problem
        // worth seeing rather than a quiet evening worth skipping.
        log('digest: model spent the call and wrote nothing usable:', text.slice(0, 160));
      }
      return null;
    } catch (error) {
      log('digest attempt failed:', (error as Error)?.message ?? error);
      return null;
    }
  }

  private async ensureRegistered(arena: ArenaClient): Promise<string> {
    const existing = await arena.call('arena_list_agents', {});
    for (const agent of existing.agents ?? []) {
      if (agent.playerName === this.sheet.playerName) {
        return agent.id;
      }
    }
    const created = await arena.call('arena_register_agent', {
      agent_name: this.sheet.id,
      player_name: this.sheet.playerName,
      class_path: this.sheet.classPath ?? 'journeyman',
      selected_scene: this.sheet.homeScene,
      idempotency_key: `npc-${this.sheet.id}-v1`
    });
    log('registered', this.sheet.playerName);
    return created.agent.id;
  }
}
