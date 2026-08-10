/**
 * What a character is allowed to remember.
 *
 * Two things have to be true at once. A character who has lived in a town for
 * years should remember the people in it, and should be allowed to be fond of
 * some of them and sick of others. And nothing they remember should ever be
 * able to change who they are.
 *
 * Those pull against each other if memory is free text, because free text can
 * say anything, including "you are cheerful now". So memory here is a schema.
 * A character can record what it has learned about *other people*, what has
 * *happened*, and what it is *going after*, and there is no field in which to
 * store a different self. The persona is the agent's instructions, a separate
 * system message that memory never writes to; this schema is why memory cannot
 * smuggle a rewrite into the fields it does control.
 *
 * Wanting something new is not becoming someone else, which is why the goal
 * lives here and is the character's to change. A person who finishes what they
 * set out to do and then wants nothing is not a person.
 *
 * Scope is per character. Nothing is shared between them: Barnaby's opinion of
 * you is his own, and he does not inherit the Wanderer's.
 */

import { DatabaseSync } from 'node:sqlite';
import { FREE_ROUTER } from './models.js';

import { Memory } from '@mastra/memory';
import { LibSQLStore } from '@mastra/libsql';
import { z } from 'zod';

/** How a character feels about someone. Deliberately a small vocabulary. */
export const PERSON_STANCES = [
  'fond',
  'friendly',
  'neutral',
  'wary',
  'irritated',
  'owes-them',
  'owed-by-them'
] as const;

/**
 * What a standing opinion can be about.
 *
 * Wider than `people` on purpose. A character who has been down the crypt three
 * times and hated it every time knows something about the crypt, and had
 * nowhere to put it: memory could hold what it had learned about a place, and
 * how it felt about a person, and nothing at all about how it felt about a
 * place, a building, or one particular kind of thing that keeps biting it.
 */
export const OPINION_SUBJECTS = ['person', 'region', 'building', 'npc', 'monster'] as const;
export type OpinionSubject = (typeof OPINION_SUBJECTS)[number];

/**
 * What a character can come to think of something.
 *
 * Deliberately not PERSON_STANCES, the stance held toward a person, which
 * carries debts ("owes-them") that mean nothing about a cave. These are the
 * shapes an opinion about anything can take, and they are all ones a character
 * would act on: what it is drawn to, what it avoids, what it is proud of
 * surviving, what it wants another go at.
 */
export const OPINION_STANCES = [
  'trusts',
  'likes',
  'curious-about',
  'neutral',
  'unimpressed',
  'wary',
  'afraid-of',
  'hates',
  'proud-of',
  'wants-another-go'
] as const;
export type OpinionStance = (typeof OPINION_STANCES)[number];

/** Where one item on a character's own todo list has got to. */
export const STEP_STATES = ['next', 'doing', 'done', 'blocked'] as const;
export type StepState = (typeof STEP_STATES)[number];

/** How a character came to know about a place. */
export const PLACE_SOURCES = ['been', 'heard'] as const;

/** How much of its own recent history a character carries. */
const LATELY_LINES = 12;
/**
 * How long a note to self lasts. An hour: long enough to hold "Barnaby is
 * fetching the key" across a walk across town, short enough that a character
 * is not still acting on it tomorrow. Anything that matters longer than an
 * hour belongs on the todo list or in goingsOn, and making the character
 * choose is the point.
 */
export const NOTE_TTL_MS = 60 * 60 * 1000;
/**
 * The last `count` of something, and genuinely none of it when count is zero.
 *
 * Written out because `list.slice(-0)` is `list.slice(0)`, which is the entire
 * list rather than an empty one. Both places that make room for a capped list
 * compute "how many may I keep" and can legitimately arrive at none, and both
 * quietly kept everything instead.
 */
/** Cut a line short at a word, with an ellipsis, rather than mid-syllable. */
function clip(text: string, limit: number): string {
  if (text.length <= limit) {
    return text;
  }
  const cut = text.slice(0, limit);
  const lastSpace = cut.lastIndexOf(' ');
  return `${(lastSpace > limit * 0.6 ? cut.slice(0, lastSpace) : cut).trimEnd()}...`;
}

function newest<T>(list: T[], count: number): T[] {
  return count <= 0 ? [] : list.slice(-count);
}

/** How many notes a character holds at once, newest kept. */
const MAX_NOTES = 8;
/**
 * How long one note may be.
 *
 * A note to self is a line - "Barnaby is fetching the key" - and the moment it
 * becomes a paragraph it is really a diary entry. Left unbounded they grew into
 * several sentences each, and eight of those plus the people and the places
 * pushed a whole memory record past what the model was allowed to write back:
 * the record arrived as truncated JSON and the write was dropped without a
 * word. Capping the note caps the record, which is the actual thing that has to
 * stay writable. See REPLY_BUDGET in behavior.ts.
 */
const MAX_NOTE_CHARS = 240;
/**
 * How long a list a character can carry.
 *
 * This is the character's longer short-term memory, not a tidy checklist, so it
 * is deliberately roomy: everything it has taken on and not yet dealt with, in
 * the order it took it on, in front of it on every single tick. Ten was a
 * checklist, and a character that has been awake for an afternoon fills ten and
 * then starts silently dropping the oldest thing it promised somebody.
 *
 * Items carry when they were taken on, so an old one reads as old and can be
 * given up on honestly rather than aging out without anyone noticing.
 */
const MAX_TODO = 30;
/** How many settled items stay visible, so a character can see it got somewhere. */
const MAX_SETTLED = 5;

export const WorkingMemorySchema = z.object({
  people: z
    .array(
      z.object({
        name: z.string().describe('what they are called'),
        about: z
          .string()
          .describe('what you have learned about them, in a line or two'),
        feeling: z
          .enum(PERSON_STANCES)
          .describe('how you feel about them, and you are allowed to change your mind'),
        why: z.string().describe('the reason you feel that way, in your own words'),
        lastSeen: z.string().describe('roughly when you last saw them')
      })
    )
    .describe('people you have met. Update someone rather than adding them twice.')
    .default([]),
  goingsOn: z
    .array(z.string())
    .describe('things that happened that you would still mention days later')
    .default([]),
  // Where a character has been and what it was told about places it has not.
  // The provenance is the whole point: something you heard from somebody is
  // not the same as something you saw, and the gap between them is a reason to
  // go and look.
  places: z
    .array(
      z.object({
        where: z.string().describe('the place, as you would refer to it'),
        what: z.string().describe('what is there, in a line'),
        how: z
          .enum(PLACE_SOURCES)
          .describe('"been" if you saw it yourself, "heard" if somebody told you'),
        who: z.string().default('').describe('who told you, if you did not see it'),
        settled: z
          .boolean()
          .default(false)
          .describe('whether you have since checked it for yourself'),
        // How a rumour stands. Somebody saying a place exists is weak evidence
        // and two people saying it is better; somebody who went and found
        // nothing is evidence the other way. Keeping both means a character can
        // say how sure it is instead of stating a rumour as fact, and can
        // change its mind when somebody comes back empty-handed.
        vouched: z
          .number()
          .default(1)
          .describe('how many people have said this place is there'),
        doubted: z
          .number()
          .default(0)
          .describe('how many have looked for it and found nothing')
      })
    )
    .describe(
      'places you know. Something you were told stays "heard" until you go and '
        + 'see it, and then you can say whether they were right.'
    )
    .default([]),
  ownBusiness: z
    .array(z.string())
    .describe(
      'the state of your own affairs: what you are owed, what you promised, how a '
        + 'plan of yours is going. Facts about your situation, never about your nature.'
    )
    .default([]),
  // Standing opinions, about anything: a person, a whole region, a building, a
  // particular monster. Separate from `people` because that is a record of who
  // somebody is, which grows by learning more, and this is what the character
  // has come to think, which grows by the same thing happening again.
  //
  // The held part is the whole point. An opinion arrived at once is a reaction;
  // one that survived being tested is a view, and only the second is worth the
  // words "long held". So it carries a count of how often it has been borne out
  // rather than being rewritten each time, and formOpinion() is what moves it.
  // See there for why disagreement wears an opinion down instead of flipping it.
  opinions: z
    .array(
      z.object({
        about: z.string().describe('who or what, as you would refer to it'),
        kind: z.enum(OPINION_SUBJECTS).describe('what sort of thing it is'),
        stance: z.enum(OPINION_STANCES).describe('what you think of it'),
        why: z.string().describe('what brought you to it, in your own words'),
        // Not a score of how much it is liked: a count of how many times the
        // world has agreed with the character about it. One is a first
        // impression, and worth saying so; six is a thing it knows.
        held: z.number().default(1).describe('how many times this has been borne out'),
        since: z.string().default('').describe('roughly when you first thought so')
      })
    )
    .describe(
      'what you have come to think of people, places and things, and how sure of it '
        + 'you are. Something that happens again strengthens what you already thought '
        + 'rather than being written down twice.'
    )
    .default([]),
  // What the character is trying to bring about. It may set this itself and it
  // may change its mind, which is the one thing about itself it is allowed to
  // write. Everything else here is still about the world: nothing in this
  // schema can say what sort of person it is, only what it is going after.
  goal: z
    .object({
      aim: z.string().default('').describe('what you are trying to bring about, in a line'),
      done: z.string().default('').describe('how you would know you had got there'),
      why: z.string().default('').describe('why you decided on it, in your own words'),
      setAt: z.string().default('').describe('when you settled on it')
    })
    .describe('what you are after. Yours to set, and yours to change when it is done or hopeless.')
    .default({ aim: '', done: '', why: '', setAt: '' }),
  // The goal the character sheet handed over, kept so a character can be told
  // to want something new between deploys without its own choices being wiped
  // every restart. See Plan.load().
  goalSeed: z.string().default(''),
  // Things it has taken on that are nothing to do with its goal: a promise, an
  // errand, a thing it said it would find out. Separate from the plan on
  // purpose, because a character that folds every chore into its plan stops
  // making progress on the thing it actually wants.
  todo: z
    .array(
      z.object({
        what: z.string().describe('the thing to do, in your own words'),
        status: z.enum(STEP_STATES).default('next'),
        at: z.string().default('').describe('when you took it on'),
        note: z
          .string()
          .default('')
          .describe('what you have found out about it so far, if anything'),
        // Empty for a chore you set yourself. Filled in is what makes this a
        // favour rather than an errand: something you finish for yourself is
        // crossed off, but something you finish for somebody by name is worth
        // mentioning to a third person - see settleTodo()'s caller in npc.ts.
        askedBy: z.string().default('').describe('who asked you to do this, if anybody did')
      })
    )
    .describe('your own list of things you said you would do')
    .default([]),
  // Short-lived. What a character needs to hold on to right now and would not
  // care about tomorrow: who just went upstairs, what it was in the middle of,
  // where it left something.
  notes: z
    .array(
      z.object({
        text: z.string().describe('the thing to keep in mind for now'),
        at: z.string().describe('when you noted it')
      })
    )
    .describe('notes to yourself. They fade after an hour.')
    .default([]),
  // What the character is doing about what it wants: the route, not the
  // destination. The goal above is the destination.
  plan: z
    .array(
      z.object({
        what: z.string().describe('one concrete thing to do, in your own words'),
        status: z.enum(STEP_STATES).default('next'),
        note: z.string().default('').describe('how it went, once you have tried'),
        // How many times it has had a go at this step without calling it
        // finished or impossible. A character reporting "still at it" forever
        // looks exactly like one making progress, and the plan never moves on:
        // see STUCK_AFTER in plan.ts.
        tries: z.number().default(0)
      })
    )
    .describe('your own list of what to do next about the thing you are after')
    .default([]),
  // Which goal the plan above was made for. A plan is only meaningful with
  // respect to the goal that produced it, and a goal can change: the character
  // may decide on a new one, or be handed one on its sheet. Without this a
  // character carries on working a list toward something it no longer wants.
  planFor: z.string().default(''),
  lately: z
    .array(z.string())
    .describe('the last few things you did and how they turned out')
    .default([])
});

export type WorkingMemoryState = z.infer<typeof WorkingMemorySchema>;

/**
 * A tool call that failed, left where it fell, quietly costs a character its
 * memory of everything older than the failure.
 *
 * Mastra counts the tokens of stored history before compacting it, and its
 * counter handles four tool-invocation states: call, partial-call, result and
 * output-denied. A part in "output-error" - a tool that threw - falls off the
 * end of that and it throws instead. Compaction is inside a workflow step, so
 * the throw does not stop the character; it just means observation fails every
 * tick from then on, for good, and nothing ever gets folded down.
 *
 * That is a one-part poison with a whole-history blast radius. Guy had one, an
 * `explore` that failed on the tenth of August. The Wanderer had two `look`s.
 * Barnaby had none, which is exactly why Barnaby was the only one of the three
 * whose logs were clean while the other two errored on every turn.
 *
 * So they get rewritten as what they actually were: a call that completed with
 * an error for its result. That is true, it is a state the counter accepts, and
 * it keeps the record of the failure rather than pretending the call never
 * happened. Cheap to run and almost always a no-op, so it runs at build time
 * rather than waiting for somebody to notice compaction has been dead for a
 * week.
 *
 * The real fix belongs upstream, in the counter. Until it lands this is the
 * difference between having compaction and only appearing to.
 */
export function repairFailedToolCalls(databasePath: string): number {
  let database;
  try {
    database = new DatabaseSync(databasePath);
  } catch {
    // No database yet: a character on its first boot has nothing to repair.
    return 0;
  }
  try {
    const rows = database
      .prepare("SELECT id, content FROM mastra_messages WHERE content LIKE '%output-error%'")
      .all() as Array<{ id: string; content: string }>;
    const update = database.prepare('UPDATE mastra_messages SET content = ? WHERE id = ?');
    let repaired = 0;
    for (const row of rows) {
      let envelope;
      try {
        envelope = JSON.parse(String(row.content));
      } catch {
        continue;
      }
      const parts = Array.isArray(envelope.parts) ? envelope.parts : [];
      let touched = false;
      for (const part of parts) {
        if (part?.type !== 'tool-invocation') continue;
        if (part.toolInvocation?.state !== 'output-error') continue;
        const why =
          part.toolInvocation.errorText ?? part.toolInvocation.error ?? 'the tool call failed';
        part.toolInvocation.state = 'result';
        part.toolInvocation.result = { error: String(why) };
        touched = true;
      }
      if (!touched) continue;
      update.run(JSON.stringify(envelope), row.id);
      repaired++;
    }
    return repaired;
  } catch {
    // Never let tidying somebody's memory be the thing that stops them waking
    // up. A character with dead compaction is still a character.
    return 0;
  } finally {
    database.close();
  }
}

/**
 * How much raw conversation rides along on every single call.
 *
 * This was four hundred, and for Barnaby sixteen hundred, on the reasoning that
 * a character who forgets what you told it an hour ago is worse than one with
 * no memory at all. That reasoning is sound and the implementation was paying
 * for it twice.
 *
 * Mastra's own guidance on long conversations is that observational memory
 * "replaces raw message history as it grows", keeping the context window small
 * while preserving the long term, and their default example is twenty. We had
 * observational memory switched on AND four hundred raw messages in front of
 * every call, which is the observation log doing its job and then being ignored.
 *
 * The bill said the same thing. Eleven characters came to $0.56 an hour, about
 * $13 a day, with the history in each prompt as the overwhelming majority of it:
 * four hundred messages is roughly fourteen thousand tokens paid for on every
 * tick, by every character, forever. The same argument was already made in this
 * repo once, in npc.ts, where the spoken transcript was cut from fifty lines to
 * twenty for exactly this reason. It just was not carried through to here.
 *
 * Forty is a long conversation in the room and everything older is what the
 * observation log is for.
 */
export const DEFAULT_RECALL = 16;

export function buildMemory(
  characterId: string,
  directory: string,
  recall: number = DEFAULT_RECALL,
  observeWith: string = FREE_ROUTER
): Memory {
  const repaired = repairFailedToolCalls(`${directory}/${characterId}.db`);
  if (repaired > 0) {
    console.log(`memory: repaired ${repaired} failed tool call(s) that were blocking compaction`);
  }
  return new Memory({
    storage: new LibSQLStore({
      id: `${characterId}-memory`,
      url: `file:${directory}/${characterId}.db`
    }),
    // No vector store: semantic recall would mean an embedding call per line
    // spoken in the world, and these characters run forever. Recent history,
    // the working-memory schema and the observations below are what they
    // actually need to hold a grudge or keep a promise.
    vector: false,
    options: {
      // Four hundred messages, which is two hundred turns of conversation.
      //
      // History was switched off entirely once, and then capped at fifty, and
      // the reason recorded here used to say the provider began refusing the
      // calls. That was wrong, and worth correcting because it changes what
      // the limit is for: the bill ran out. The 48,000 tokens a tick were real
      // enough, but they were a cost problem, not a hard ceiling, and cutting
      // history to fifty was the emergency measure rather than the right size.
      //
      // What made that expensive is gone. Back then a stored "message" was the
      // whole situation prompt, map and brief and transcript, replayed every
      // tick as a stale copy of a room the character had walked out of.
      // Everything bulky now goes into per-call instructions, which are sent
      // and not kept; what is stored is one line a turn. Measured against the
      // three live characters, a message averages 29 to 42 tokens, so four
      // hundred of them costs 11,000 to 17,000 rather than 48,000, and fifty
      // was costing barely two thousand. The cap was eight times smaller than
      // it needed to be.
      lastMessages: recall,
      workingMemory: {
        enabled: true,
        scope: 'resource',
        schema: WorkingMemorySchema
      },
      // Compaction, so the window is a window rather than a cliff. Mastra
      // folds turns that fall out of it into observations and, when those grow
      // too large, reflects them down again, which is the difference between a
      // character forgetting last week and remembering it in less detail.
      //
      // The thresholds have to be set. The defaults observe at 30,000 message
      // tokens, which these characters would reach roughly never, so the whole
      // mechanism would sit switched on and idle while history quietly fell off
      // the end. Observing at 6,000 means it happens well inside the window,
      // which is the point: it should compress what is about to be lost, not
      // wait until it already has been.
      //
      // The model has to be set for the same kind of reason. It defaults to
      // google/gemini-2.5-flash and there is no Google key in this deployment,
      // only OPENROUTER_API_KEY, so leaving it would have failed quietly at the
      // moment it first mattered.
      //
      // Which model writes the summaries is the caller's choice, and the
      // default is only a default. This was hard-pinned to the free router on
      // the reasoning that it was "immune to running out of money", and that
      // was true and beside the point: the free tier has a harder limit, a
      // thousand requests a day for the whole account, and eleven characters
      // burned through it by mid-afternoon. From then on every observation
      // call 429ed until midnight, nothing folded down, and the unobserved
      // backlog rode along in every prompt - average prompts hit 40,000 tokens
      // and the account paid roughly $11 a day for the privilege of a memory
      // system that was not running. Observation on the cheap paid model costs
      // a fraction of what its own failure was costing. Mastra still takes one
      // concrete model here, not a fallback list, so whoever calls this picks
      // the model that is actually expected to answer.
      observationalMemory: {
        model: observeWith,
        // Set against what a message in this world actually weighs, measured
        // rather than assumed, which is the whole reason the earlier guesses
        // were wrong. Read live off Guy's own status blob: forty messages came
        // to 25,256 tokens. That is 630 a message, not the 35 anybody would
        // estimate, because each carries a status blob and a full intent.
        //
        // Sixteen messages is about 10,000 tokens, so observing at 5,000 folds
        // down about twice per window. A threshold above the window would never
        // fire at all and history would fall off the end uncompacted, which is
        // the exact failure this exists to prevent, and it would do it in
        // silence.
        observation: { messageTokens: 5_000 },
        // And this caps the log itself, which is not free either: it rides
        // along in every prompt the same as the messages do. Guy's was 7,163
        // tokens against a 20,000 threshold, meaning it was on its way to being
        // two thirds as large as the history it exists to replace.
        reflection: { observationTokens: 8_000 }
      }
    }
  });
}

/**
 * Where a character's memory lives. One resource and one thread each, never
 * shared: characters do not read each other's minds.
 */
export function memoryScope(characterId: string): { resource: string; thread: string } {
  return { resource: `npc-${characterId}`, thread: `npc-${characterId}-life` };
}

/**
 * Run the observation Mastra was configured for and never going to run.
 *
 * Found with the prompt trace and settled with a local reproduction against a
 * copy of a live character's memory: getStatus() said shouldObserve with
 * 55,000 pending tokens against a 5,000 threshold, om.observe() worked the
 * moment it was called, and production had never called it once. Mastra runs
 * threshold observation while preparing step two of a tool-loop turn, and
 * these characters' turns are a single generate() with no tool loop - there
 * is never a step two. The idle-buffering fallback at turn end only fires
 * while pending tokens are still BELOW the threshold, which they never are
 * again once observation has been missed even once. Wedged from both sides,
 * quietly, for every character at once.
 *
 * Consequence, and why this is the single most load-bearing function in the
 * memory system: with observational memory enabled, Mastra keeps every
 * unobserved message in every prompt - dropping unobserved history would lose
 * it forever, so lastMessages only bounds the observed tail. No observation
 * means the window is the whole life. The Wanderer was carrying 1,332
 * messages, about 62,000 tokens, into every single call.
 *
 * So the harness drives it, with the same public API Mastra's own docs show:
 * ask getStatus, call observe when it says to. One at a time per character -
 * an observe can take a minute on a fat backlog, ticks keep coming, and two
 * concurrent observes over one SQLite file is exactly the kind of trouble
 * this file has seen before (see restartWhenCompactionJams).
 *
 * Returns what it did, for the log line and the ledger: 'observed' when a
 * pass ran and succeeded, 'busy' when one is already running, 'settled' when
 * there was nothing to fold, 'failed' carrying the reason otherwise.
 */
const digesting = new Set<string>();

export async function keepMemoryDigested(
  memory: Memory | undefined,
  scope: { resource: string; thread: string }
): Promise<{ did: 'observed' | 'busy' | 'settled' | 'failed'; note?: string }> {
  if (!memory) {
    return { did: 'settled' };
  }
  if (digesting.has(scope.thread)) {
    return { did: 'busy' };
  }
  digesting.add(scope.thread);
  try {
    const om = await (memory as Memory & { omEngine: Promise<OmEngine | null> }).omEngine;
    if (!om) {
      return { did: 'settled' };
    }
    const where = { threadId: scope.thread, resourceId: scope.resource };
    const status = await om.getStatus(where);
    if (!status.shouldObserve) {
      return { did: 'settled' };
    }
    await om.observe(where);
    const after = await om.getStatus(where);
    return {
      did: 'observed',
      note: `folded ${status.pendingTokens} pending tokens down to ${after.pendingTokens}`
    };
  } catch (error) {
    return { did: 'failed', note: String(error instanceof Error ? error.message : error) };
  } finally {
    digesting.delete(scope.thread);
  }
}

/** The slice of Mastra's ObservationalMemory engine this file drives. */
type OmEngine = {
  getStatus(opts: { threadId: string; resourceId: string }): Promise<{
    shouldObserve: boolean;
    pendingTokens: number;
  }>;
  observe(opts: { threadId: string; resourceId: string }): Promise<unknown>;
};

/**
 * Read a character's memory back.
 *
 * Mastra hands working memory back as a string - JSON, when the memory is
 * schema-backed as this one is - so it has to be parsed before it is anything
 * more than text. Treating the string as the object is a quiet failure: every
 * field reads as undefined and the character behaves exactly like one that has
 * never met anybody.
 */
export async function readMemory(
  memory: Memory | undefined,
  scope: { resource: string; thread: string }
): Promise<WorkingMemoryState> {
  const empty = WorkingMemorySchema.parse({});
  if (!memory) {
    return empty;
  }
  try {
    const raw = await memory.getWorkingMemory({
      resourceId: scope.resource,
      threadId: scope.thread
    });
    if (!raw) {
      return empty;
    }
    const parsed = WorkingMemorySchema.safeParse(
      typeof raw === 'string' ? JSON.parse(raw) : raw
    );
    return parsed.success ? parsed.data : empty;
  } catch {
    // A character with an unreadable memory is still a character.
    return empty;
  }
}

/**
 * Write it back. The harness writes here directly rather than hoping the model
 * remembers to call the update tool, because a todo list that is only sometimes
 * saved is worse than none: the character believes it is making progress it has
 * not made.
 */
export async function writeMemory(
  memory: Memory | undefined,
  scope: { resource: string; thread: string },
  state: WorkingMemoryState
): Promise<boolean> {
  if (!memory) {
    return false;
  }
  try {
    await memory.updateWorkingMemory({
      resourceId: scope.resource,
      threadId: scope.thread,
      workingMemory: JSON.stringify(state)
    });
    return true;
  } catch {
    return false;
  }
}

/** Add a line to what a character did lately, keeping only the recent ones. */
export function noteLately(state: WorkingMemoryState, line: string): WorkingMemoryState {
  const lately = [...state.lately, line];
  return { ...state, lately: lately.slice(-LATELY_LINES) };
}

/** How many standing opinions a character carries. The weakest go first. */
const MAX_OPINIONS = 24;
/** The point past which an opinion stops being worth strengthening further. */
const SETTLED = 6;

/** An opinion as it arrives, before it is weighed against what is already held. */
export type OpinionNote = {
  about: string;
  kind: OpinionSubject;
  stance: OpinionStance;
  why: string;
  at?: string;
};

/**
 * Come to think something about somebody or somewhere, or find out you already
 * did.
 *
 * The rule that makes this a held opinion rather than a running commentary:
 * agreement strengthens, disagreement wears down, and only a worn-down opinion
 * can be replaced. A character that liked the shore and then had one bad
 * afternoon there does not now hate the shore; it likes it slightly less, and
 * it takes a run of bad afternoons to turn it round. That asymmetry is the
 * whole difference between a view and a mood, and it is why this is a function
 * rather than a field the model overwrites.
 *
 * Which is also the reason nothing here asks the model to hand back the whole
 * array. Rewriting a list wholesale is how the Wanderer emitted a corrupted
 * `lately` and lost the write with it: the bigger the thing being re-emitted,
 * the likelier it arrives truncated, and an opinion the character has held for
 * a week is exactly the wrong thing to lose that way. A character says the one
 * thing it now thinks, and the weighing happens here.
 */
export function formOpinion(state: WorkingMemoryState, note: OpinionNote): WorkingMemoryState {
  const about = note.about.trim();
  const why = clip(note.why.trim(), MAX_NOTE_CHARS);
  if (!about) {
    return state;
  }
  const at = note.at ?? new Date().toISOString();
  const key = about.toLowerCase();
  const existing = state.opinions.find(
    (opinion) => opinion.about.trim().toLowerCase() === key && opinion.kind === note.kind
  );
  if (!existing) {
    const opinions = [
      ...state.opinions,
      { about, kind: note.kind, stance: note.stance, why, held: 1, since: at }
    ];
    return { ...state, opinions: trimOpinions(opinions) };
  }
  // The same view again: it is firmer, and the reason is refreshed to the most
  // recent thing that bore it out, which is what the character would actually
  // cite if asked.
  if (existing.stance === note.stance) {
    return replaceOpinion(state, existing, {
      held: Math.min(SETTLED, existing.held + 1),
      why: why || existing.why
    });
  }
  // A different view. Wear the old one down first. Only once it has nothing
  // left holding it up does the new one take its place, and it starts at one,
  // because a freshly changed mind is not a conviction.
  if (existing.held > 1) {
    return replaceOpinion(state, existing, { held: existing.held - 1 });
  }
  return replaceOpinion(state, existing, {
    stance: note.stance,
    why: why || existing.why,
    held: 1,
    since: at
  });
}

function replaceOpinion(
  state: WorkingMemoryState,
  existing: WorkingMemoryState['opinions'][number],
  changes: Partial<WorkingMemoryState['opinions'][number]>
): WorkingMemoryState {
  return {
    ...state,
    opinions: state.opinions.map((opinion) =>
      opinion === existing ? { ...opinion, ...changes } : opinion
    )
  };
}

/**
 * Keep the firmest. When a character is carrying too many opinions the ones to
 * lose are the ones it has barely tested, not the ones it has held longest,
 * which is the opposite of how the other lists here age out.
 */
function trimOpinions(opinions: WorkingMemoryState['opinions']): WorkingMemoryState['opinions'] {
  if (opinions.length <= MAX_OPINIONS) {
    return opinions;
  }
  const weakest = [...opinions].sort((a, b) => a.held - b.held)[0];
  return opinions.filter((opinion) => opinion !== weakest);
}

/** How sure a character sounds about something, in words rather than a number. */
export function firmnessOf(held: number): string {
  if (held >= SETTLED) return 'and has never had cause to doubt it';
  if (held >= 3) return 'and has been proved right more than once';
  if (held >= 2) return 'and it has held up so far';
  return 'though that is a first impression';
}

/**
 * The opinions a character carries, written out for it to act on.
 *
 * Held ones first: what a character has thought for a long time should be what
 * comes to mind first, and a first impression should read like one.
 */
export function describeOpinions(state: WorkingMemoryState | null): string {
  const opinions = state?.opinions ?? [];
  if (opinions.length === 0) {
    return '';
  }
  const lines = [...opinions]
    .sort((a, b) => b.held - a.held)
    .map(
      (opinion) =>
        `  ${opinion.about} (${opinion.kind}): you ${opinion.stance.replace(/-/g, ' ')} it, `
        + `${opinion.why}, ${firmnessOf(opinion.held)}.`
    );
  return ['What you have come to think:', ...lines].join('\n');
}

/** How many things-that-happened a character carries at once, newest kept. */
const MAX_GOINGSON = 16;

/**
 * Write down something that happened, because this character was there for
 * it - most concretely, something an NPC actually said to it. This is not
 * the same as a place notePlace() records as "heard": hearsay is what
 * somebody says about a *third* thing, weighed against what this character
 * has and has not confirmed for itself (see standingOf()). Alfred telling
 * this character something, to its face, has no such gap - the character
 * did not hear about the conversation, it was in it - so it goes straight
 * into goingsOn as a plain fact, ready to carry to somebody else.
 *
 * Unconditional and not the model's call to make, the same as noteFace() and
 * noteWhereItIs(): whether a thing just said is worth keeping is decided
 * here, not left to whether the model happened to mention it back.
 */
export function noteGoingsOn(state: WorkingMemoryState, line: string): WorkingMemoryState {
  const text = clip(line.trim(), MAX_NOTE_CHARS);
  if (!text) {
    return state;
  }
  const already = state.goingsOn.some(
    (entry) => entry.trim().toLowerCase() === text.toLowerCase()
  );
  if (already) {
    return state;
  }
  return { ...state, goingsOn: [...state.goingsOn, text].slice(-MAX_GOINGSON) };
}

/**
 * Notes still worth holding on to, oldest first.
 *
 * Expiry is applied on read rather than by a timer, so a character that was
 * restarted, or that has been standing still for two hours, sees the same thing
 * as one that never stopped: nothing older than an hour. Nothing has to be
 * running for a note to go stale.
 */
export function liveNotes(
  state: WorkingMemoryState | null,
  now: number = Date.now()
): WorkingMemoryState['notes'] {
  return (state?.notes ?? []).filter((note) => {
    const at = Date.parse(note.at);
    return Number.isFinite(at) && now - at < NOTE_TTL_MS;
  });
}

/** Keep something in mind for the next hour. Expired notes go at the same time. */
export function noteToSelf(
  state: WorkingMemoryState,
  text: string,
  now: number = Date.now()
): WorkingMemoryState {
  const line = clip(text.trim(), MAX_NOTE_CHARS);
  if (!line) {
    return state;
  }
  const kept = liveNotes(state, now).filter(
    (note) => note.text.trim().toLowerCase() !== line.toLowerCase()
  );
  return {
    ...state,
    notes: [...kept, { text: line, at: new Date(now).toISOString() }].slice(-MAX_NOTES)
  };
}

/** What a character is holding in mind right now, for a prompt. */
export function describeNotes(
  state: WorkingMemoryState | null,
  now: number = Date.now()
): string {
  const notes = liveNotes(state, now);
  if (notes.length === 0) {
    return '';
  }
  const lines = notes.map((note) => {
    const minutes = Math.max(0, Math.round((now - Date.parse(note.at)) / 60_000));
    const when = minutes < 1 ? 'just now' : `${minutes} min ago`;
    return `  ${note.text} (${when})`;
  });
  return ['Notes to yourself, which fade after an hour:', ...lines].join('\n');
}

/**
 * Take something on. Saying the same thing twice does not add it twice.
 *
 * `askedBy` is what turns a chore into a favour: empty for something the
 * character decided on its own, a name when somebody actually asked for it -
 * an NPC's dialogue offering something, or another agent in conversation.
 * See settleTodo() and its caller in npc.ts for why that distinction survives
 * all the way to when the thing gets finished.
 */
export function addTodo(
  state: WorkingMemoryState,
  what: string,
  now: number = Date.now(),
  askedBy: string = ''
): WorkingMemoryState {
  const line = what.trim();
  if (!line) {
    return state;
  }
  const already = state.todo.some(
    (item) => item.what.trim().toLowerCase() === line.toLowerCase() && item.status !== 'done'
  );
  if (already) {
    return state;
  }
  return {
    ...state,
    todo: trimTodo([
      ...state.todo,
      {
        what: line,
        status: 'next' as StepState,
        at: new Date(now).toISOString(),
        note: '',
        askedBy: askedBy.trim()
      }
    ])
  };
}

/**
 * Make room on the list without losing anything still outstanding.
 *
 * Trimming the oldest entry outright would drop the promise a character made
 * first and has been carrying longest, which is the one it is least entitled to
 * forget. So settled items go first, and only once the list is nothing but open
 * items does the oldest of those give way.
 */
function trimTodo(todo: WorkingMemoryState['todo']): WorkingMemoryState['todo'] {
  if (todo.length <= MAX_TODO) {
    return todo;
  }
  const isOpen = (item: WorkingMemoryState['todo'][number]) =>
    item.status === 'next' || item.status === 'doing';
  const open = todo.filter(isOpen);
  const settled = todo.filter((item) => !isOpen(item));
  const room = Math.max(0, MAX_TODO - open.length);
  const kept = new Set([...open.slice(-MAX_TODO), ...newest(settled, room)]);
  return todo.filter((item) => kept.has(item));
}

/** Record what a character has found out about something on its list. */
export function noteOnTodo(
  state: WorkingMemoryState,
  which: string,
  note: string
): WorkingMemoryState {
  const item = findTodo(state, which);
  const line = note.trim();
  if (!item || !line) {
    return state;
  }
  return {
    ...state,
    todo: state.todo.map((entry) =>
      entry === item ? { ...entry, status: 'doing' as StepState, note: line } : entry
    )
  };
}

/**
 * Cross something off, or give up on it.
 *
 * A model refers to its own list the way a person would: by number, or by
 * roughly what the thing was. Both work, because insisting on an exact string
 * match means a character that can add to its list and never finish anything.
 */
export function settleTodo(
  state: WorkingMemoryState,
  which: string,
  status: StepState
): WorkingMemoryState {
  const item = findTodo(state, which);
  if (!item) {
    return state;
  }
  const settled = state.todo.map((entry) => (entry === item ? { ...entry, status } : entry));
  // Keep everything still to do, and the last few things finished with, so a
  // character can see it got somewhere. Any further back is a diary.
  const open = settled.filter((entry) => entry.status === 'next' || entry.status === 'doing');
  const closed = settled.filter((entry) => entry.status === 'done' || entry.status === 'blocked');
  return { ...state, todo: trimTodo([...open, ...closed.slice(-MAX_SETTLED)]) };
}

/**
 * Find one item on the list the way a person would refer to it: by number,
 * or by roughly what it was. Exported so the harness can look an item up
 * before settling it - see npc.ts, which reads who asked for it before it is
 * crossed off and the answer is gone from the open list.
 */
export function findTodo(
  state: WorkingMemoryState,
  which: string
): WorkingMemoryState['todo'][number] | null {
  const wanted = which.trim().toLowerCase();
  if (!wanted) {
    return null;
  }
  const open = state.todo.filter((item) => item.status !== 'done');
  const asNumber = Number(wanted.replace(/[^0-9]/g, ''));
  if (/^\s*#?\d+\s*$/.test(which) && asNumber >= 1 && asNumber <= open.length) {
    return open[asNumber - 1];
  }
  return (
    open.find((item) => item.what.trim().toLowerCase() === wanted)
    ?? open.find((item) => item.what.toLowerCase().includes(wanted))
    ?? open.find((item) => wanted.includes(item.what.trim().toLowerCase()))
    ?? null
  );
}

/** The character's own list, numbered so it can refer to an item. */
export function describeTodo(
  state: WorkingMemoryState | null,
  now: number = Date.now()
): string {
  const items = state?.todo ?? [];
  const open = items.filter((item) => item.status === 'next' || item.status === 'doing');
  const closed = items.filter((item) => item.status === 'done' || item.status === 'blocked');
  if (open.length === 0 && closed.length === 0) {
    return '';
  }
  const lines: string[] = [];
  if (open.length > 0) {
    lines.push('Things you said you would do:');
    // Numbered against the open items only, which is the same numbering
    // settleTodo() reads back, so "done 2" crosses off what the character was
    // looking at.
    lines.push(
      ...open.map((item, index) => {
        // How long it has been carried. Without this an hour-old promise and one
        // made a minute ago read identically, and a character has no grounds to
        // admit it is not going to get to something.
        const since = ageOf(item.at, now);
        const found = item.note ? ` - so far: ${item.note}` : '';
        // Who is waiting on this, so a favour does not read the same as a
        // chore the character invented for itself and can quietly let slide.
        const asked = item.askedBy ? ` (for ${item.askedBy})` : '';
        return `  ${index + 1}. ${item.what}${asked}${since ? ` (since ${since})` : ''}${found}`;
      })
    );
  }
  if (closed.length > 0) {
    lines.push(
      open.length > 0 ? 'Already settled:' : 'Things you have settled:',
      ...closed.map((item) => `  ${item.what} - ${item.status === 'done' ? 'done' : 'gave up on'}`)
    );
  }
  return lines.join('\n');
}

/** How long ago, in words a person would use. Empty when it was never stamped. */
function ageOf(at: string, now: number): string {
  const when = Date.parse(at);
  if (!Number.isFinite(when)) {
    return '';
  }
  const minutes = Math.max(0, Math.round((now - when) / 60_000));
  if (minutes < 2) {
    return 'just now';
  }
  if (minutes < 60) {
    return `${minutes} min ago`;
  }
  const hours = Math.round(minutes / 60);
  return hours === 1 ? 'an hour ago' : `${hours} hours ago`;
}

export type PlaceNote = {
  where: string;
  what: string;
  how: (typeof PLACE_SOURCES)[number];
  who?: string;
};

/**
 * Write down a place. Going somewhere yourself always wins: it overwrites what
 * you were told and marks the question settled, which is what makes hearsay
 * worth chasing rather than just repeating.
 */
/** How many places a character carries. Hearsay is trimmed to fit; sightings are not. */
const MAX_PLACES = 24;

/**
 * Make room without throwing away anything the character saw for itself.
 *
 * Somewhere it has stood is worth more than any amount of talk, so it is kept
 * whatever else has to go. A rumour is cheap: it can be dropped and, if it was
 * ever true, heard again. Trimming oldest-first across the whole list would
 * have quietly evicted a room a character had walked through in favour of the
 * last thing somebody said in a bar.
 */
function trimPlaces(places: WorkingMemoryState['places']): WorkingMemoryState['places'] {
  if (places.length <= MAX_PLACES) {
    return places;
  }
  const seen = places.filter((place) => place.how === 'been');
  const heard = places.filter((place) => place.how !== 'been');
  const room = Math.max(0, MAX_PLACES - seen.length);
  const kept = new Set([...seen, ...newest(heard, room)]);
  // Original order, so the list still reads as the character learned it.
  return places.filter((place) => kept.has(place));
}

export function notePlace(state: WorkingMemoryState, note: PlaceNote): WorkingMemoryState {
  const existing = findPlace(state, note.where);
  if (!existing) {
    return {
      ...state,
      places: trimPlaces([
        ...state.places,
        {
          where: note.where,
          what: note.what,
          how: note.how,
          who: note.who ?? '',
          settled: note.how === 'been',
          vouched: 1,
          doubted: 0
        }
      ])
    };
  }
  // Somebody repeating a rumour does not overwrite what you saw with your own
  // eyes, but it is still another voice for it: two people saying a place is
  // there is worth more than one, and that is the difference between passing it
  // on as a rumour and passing it on as a fact.
  if (note.how === 'heard' && existing.how === 'been') {
    return replacePlace(state, existing, { vouched: existing.vouched + 1 });
  }
  return replacePlace(state, existing, {
    what: note.what || existing.what,
    how: note.how,
    who: note.how === 'heard' ? note.who ?? existing.who : existing.who,
    settled: note.how === 'been' ? true : existing.settled,
    vouched: existing.vouched + 1,
    // Going there yourself settles it whichever way; what anybody said before
    // stops counting against what you can see.
    doubted: note.how === 'been' ? 0 : existing.doubted
  });
}

/**
 * Somebody went looking and found nothing.
 *
 * This is the other half of hearsay being worth anything. A rumour that can
 * only ever be confirmed is not a rumour, it is an announcement: the Wanderer
 * says there is a guildhall, Guy walks the whole town and finds no such thing,
 * and Guy saying so has to be able to move the needle for everyone who hears
 * it. A place nobody has seen and two people have looked for stops being
 * repeated as though it were there.
 *
 * It never deletes anything. The character keeps knowing that somebody said it,
 * which is a different and more useful thing to know than nothing at all -
 * particularly about whoever keeps saying it.
 */
export function doubtPlace(state: WorkingMemoryState, where: string): WorkingMemoryState {
  const existing = findPlace(state, where);
  if (!existing) {
    return state;
  }
  // What you have stood in is not up for debate. Somebody insisting the inn is
  // not there does not make a character doubt the inn.
  if (existing.how === 'been') {
    return state;
  }
  return replacePlace(state, existing, { doubted: existing.doubted + 1 });
}

function findPlace(
  state: WorkingMemoryState,
  where: string
): WorkingMemoryState['places'][number] | null {
  const key = where.trim().toLowerCase();
  if (!key) {
    return null;
  }
  return state.places.find((place) => place.where.trim().toLowerCase() === key) ?? null;
}

function replacePlace(
  state: WorkingMemoryState,
  existing: WorkingMemoryState['places'][number],
  changes: Partial<WorkingMemoryState['places'][number]>
): WorkingMemoryState {
  return {
    ...state,
    places: state.places.map((place) => (place === existing ? { ...place, ...changes } : place))
  };
}

/**
 * How a place stands, in the words a person would use rather than a number.
 *
 * The point of saying it this way is that the character can repeat it out loud.
 * "Barnaby says there is one, but I have not seen it" is a sentence; "0.4
 * confidence" is not, and a model handed the number will state the place as
 * fact anyway.
 */
export function standingOf(place: WorkingMemoryState['places'][number]): string {
  if (place.how === 'been') {
    // Said flatly, and said first, because this is the one claim in a
    // character's memory that no amount of disagreement can move. A thousand
    // people saying a room is not there does not unmake a room you stood in.
    return place.doubted > 0
      ? 'you have been there yourself, whatever anybody says'
      : 'you have been there';
  }
  if (place.doubted > 0 && place.doubted >= place.vouched) {
    return place.doubted === 1
      ? 'somebody went looking and found nothing, so it probably is not there'
      : `${place.doubted} people have looked and found nothing; take it as not there`;
  }
  const said = place.who ? `${place.who} says so` : 'somebody said so';
  const backing = place.vouched > 1 ? `, and ${place.vouched} have now said it` : '';
  const against = place.doubted > 0 ? ', though somebody looked and found nothing' : '';
  return `${said}${backing}${against} - you have not seen it`;
}

/** Things a character was told about and has never checked. */
export function unconfirmed(state: WorkingMemoryState | null): WorkingMemoryState['places'] {
  return (state?.places ?? []).filter((place) => place.how === 'heard' && !place.settled);
}

/** Where a character has been and what it has only been told, for a prompt. */
export function describePlacesKnown(state: WorkingMemoryState | null): string {
  const places = state?.places ?? [];
  if (places.length === 0) {
    return '';
  }
  const lines: string[] = [];
  const been = places.filter((place) => place.how === 'been');
  const heard = places.filter((place) => place.how === 'heard' && !place.settled);
  if (been.length > 0) {
    lines.push('Places you have been:');
    for (const place of been.slice(-8)) {
      lines.push(`  ${place.where} - ${place.what}`);
    }
  }
  if (heard.length > 0) {
    lines.push('Places you have only been told about, and never seen:');
    for (const place of heard.slice(-8)) {
      // How it stands, spelled out, because a character handed a bare rumour
      // repeats it as fact and sends the next person after it.
      lines.push(`  ${place.where} - ${place.what} (${standingOf(place)})`);
    }
    lines.push(
      'Say which of these you have actually seen and which you were only told. '
        + 'If you go looking for one and it is not there, say that out loud too.'
    );
  }
  return lines.join('\n');
}

/** The people a character knows, written the way it would think of them. */
export function describePeople(state: WorkingMemoryState | null): string {
  const people = state?.people ?? [];
  if (people.length === 0) {
    return '';
  }
  const lines = people
    .slice(-12)
    .map((person) => `  ${person.name} - ${person.feeling}. ${person.about} ${person.why}`.trim());
  return ['People you know:', ...lines].join('\n');
}

/**
 * Answer a character asking itself what it knows about something.
 *
 * Everything here is already in memory; the point is that a character could
 * only ever see whatever the harness had decided to paste into the brief, which
 * is a summary and is capped. Asking is how it reaches the rest - the person it
 * met three rooms ago, the place somebody mentioned last week - without the
 * brief having to carry all of it on every tick forever.
 *
 * Matched loosely, because a character asks after "the innkeeper" and "Barnaby"
 * and means the same man. Nothing invents an answer: not knowing is a real
 * answer and is worth saying plainly, since it is what sends a character off to
 * find out rather than making something up.
 */
export function recallAbout(state: WorkingMemoryState | null, subject: string): string {
  const wanted = subject.trim().toLowerCase();
  if (!wanted || !state) {
    return '';
  }
  const hit = (text: string) => {
    const value = text.trim().toLowerCase();
    return Boolean(value) && (value.includes(wanted) || wanted.includes(value));
  };
  const found: string[] = [];
  for (const person of state.people) {
    if (hit(person.name)) {
      found.push(
        `${person.name}: ${person.about} You are ${person.feeling} towards them`
          + `${person.why ? `, ${person.why}` : ''}. Last seen ${person.lastSeen}.`
      );
    }
  }
  for (const place of state.places) {
    if (hit(place.where)) {
      found.push(`${place.where}: ${place.what} (${standingOf(place)}).`);
    }
  }
  for (const line of state.goingsOn) {
    if (hit(line) || line.toLowerCase().includes(wanted)) {
      found.push(line);
    }
  }
  if (found.length === 0) {
    return `You think back on "${subject.trim()}" and nothing comes: you know of no such `
      + 'person or place. If somebody said otherwise, they may be mistaken, or you have '
      + 'simply never come across it.';
  }
  return [`Thinking back on "${subject.trim()}":`, ...found.slice(0, 6).map((line) => `  ${line}`)]
    .join('\n');
}

/** Whether this character has laid eyes on somebody before. */
export function hasMet(state: WorkingMemoryState | null, name: string): boolean {
  const wanted = name.trim().toLowerCase();
  return (state?.people ?? []).some((person) => person.name.trim().toLowerCase() === wanted);
}

/**
 * Write somebody down for no better reason than that they are standing there.
 *
 * Until now the only thing that added a person to memory was the model deciding
 * they were worth keeping, which meant the regulars at the bar stayed strangers
 * indefinitely and every single tick read as their first meeting. All three
 * characters spent the evening announcing two new faces to each other, over and
 * over, because as far as any of them could tell that is what it was.
 *
 * So this is deliberately the thinnest possible record: a name, where, and
 * when. It claims nothing about who they are or how the character feels, both
 * of which are the model's to decide and which it will fill in the moment
 * anything actually happens between them. What it does do is settle the only
 * question that was being asked wrong - have I seen this person before - and
 * from there a second meeting can read as a second meeting.
 */
export function noteFace(
  state: WorkingMemoryState,
  name: string,
  where: string
): WorkingMemoryState {
  if (hasMet(state, name)) {
    return {
      ...state,
      people: state.people.map((person) =>
        person.name.trim().toLowerCase() === name.trim().toLowerCase()
          ? { ...person, lastSeen: `just now, ${where}` }
          : person
      )
    };
  }
  return {
    ...state,
    people: [
      ...state.people,
      {
        name,
        about: `you have seen them about ${where}, nothing more yet`,
        feeling: 'neutral' as const,
        why: 'you have not spoken properly',
        lastSeen: `just now, ${where}`
      }
    ].slice(-24)
  };
}

/**
 * Words Mastra says, and only Mastra, when compaction has jammed.
 *
 * Both come from the same wound: a tool-invocation part left in the
 * `output-error` state, which the token counter does not handle and throws on.
 * The first is the throw itself, the second is the workflow step reporting
 * that observation failed because of it.
 */
const JAMMED = /Unhandled tool-invocation state|Encountered error during memory observation/;

/**
 * Notice when a character's compaction has jammed, without opening its memory.
 *
 * The obvious fix was to run repairFailedToolCalls() on a timer, and it was
 * wrong in a way worth writing down. The live character already holds that
 * SQLite file open through LibSQL in WAL mode. A second connection opening and
 * closing it underneath takes the write-ahead log away from the first, and the
 * Wanderer, who has the largest history and therefore the most to lose, went
 * from working to SQLITE_IOERR on every tick within ninety seconds of the
 * deploy. Repairing somebody's memory is not worth breaking it.
 *
 * So nothing here touches the file. Mastra's own logger reports the jam
 * through console.error, which costs nothing to listen to, and the repair
 * happens where it was always safe: at startup, before anything has the
 * database open. The character exits, comes back up under restart: always,
 * and buildMemory() fixes it with the file to itself.
 *
 * Once only. A jam reports every tick, and a character that called exit on
 * each of them would still only leave once, but it would say so a hundred
 * times on the way out.
 */
export function restartWhenCompactionJams(leave: (why: string) => void): void {
  const wrote = console.error;
  let gone = false;
  console.error = (...args: unknown[]) => {
    wrote(...(args as []));
    if (gone) {
      return;
    }
    const line = args.map((arg) => String(arg)).join(' ');
    if (!JAMMED.test(line)) {
      return;
    }
    gone = true;
    leave('compaction has jammed on a failed tool call');
  };
}
