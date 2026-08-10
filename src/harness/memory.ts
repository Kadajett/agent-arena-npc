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

import { Memory } from '@mastra/memory';
import { LibSQLStore } from '@mastra/libsql';
import { z } from 'zod';

/** How a character feels about someone. Deliberately a small vocabulary. */
export const FEELINGS = [
  'fond',
  'friendly',
  'neutral',
  'wary',
  'irritated',
  'owes-them',
  'owed-by-them'
] as const;

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
/** How many notes a character holds at once, newest kept. */
const MAX_NOTES = 8;
/** How many things it can have on its list before it has to finish some. */
const MAX_TODO = 10;

export const WorkingMemorySchema = z.object({
  people: z
    .array(
      z.object({
        name: z.string().describe('what they are called'),
        about: z
          .string()
          .describe('what you have learned about them, in a line or two'),
        feeling: z
          .enum(FEELINGS)
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
          .describe('whether you have since checked it for yourself')
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
        at: z.string().default('').describe('when you took it on')
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
        note: z.string().default('').describe('how it went, once you have tried')
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

export function buildMemory(characterId: string, directory: string): Memory {
  return new Memory({
    storage: new LibSQLStore({
      id: `${characterId}-memory`,
      url: `file:${directory}/${characterId}.db`
    }),
    // No vector store: semantic recall would mean an embedding call per line
    // spoken in the world, and these characters run forever. Recent history
    // plus the working-memory schema is what they actually need to hold a
    // grudge or keep a promise.
    vector: false,
    options: {
      // These models are cheap and the context is not the constraint, so a
      // character keeps a good stretch of what has been said to it rather than
      // the last handful of lines.
      lastMessages: 50,
      workingMemory: {
        enabled: true,
        scope: 'resource',
        schema: WorkingMemorySchema
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
  const line = text.trim();
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

/** Take something on. Saying the same thing twice does not add it twice. */
export function addTodo(
  state: WorkingMemoryState,
  what: string,
  now: number = Date.now()
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
    todo: [...state.todo, { what: line, status: 'next' as StepState, at: new Date(now).toISOString() }]
      .slice(-MAX_TODO)
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
  // Keep everything still to do, and the last couple of things finished with,
  // so a character can see it got somewhere. Any further back is a diary.
  const open = settled.filter((entry) => entry.status === 'next' || entry.status === 'doing');
  const closed = settled.filter((entry) => entry.status === 'done' || entry.status === 'blocked');
  return { ...state, todo: [...open, ...closed.slice(-2)].slice(-MAX_TODO) };
}

function findTodo(
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
export function describeTodo(state: WorkingMemoryState | null): string {
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
    lines.push(...open.map((item, index) => `  ${index + 1}. ${item.what}`));
  }
  if (closed.length > 0) {
    lines.push(
      open.length > 0 ? 'Already settled:' : 'Things you have settled:',
      ...closed.map((item) => `  ${item.what} - ${item.status === 'done' ? 'done' : 'gave up on'}`)
    );
  }
  return lines.join('\n');
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
export function notePlace(state: WorkingMemoryState, note: PlaceNote): WorkingMemoryState {
  const key = note.where.trim().toLowerCase();
  const existing = state.places.find((place) => place.where.trim().toLowerCase() === key);
  if (!existing) {
    return {
      ...state,
      places: [
        ...state.places,
        {
          where: note.where,
          what: note.what,
          how: note.how,
          who: note.who ?? '',
          settled: note.how === 'been'
        }
      ].slice(-24)
    };
  }
  // Somebody repeating a rumour does not overwrite what you saw with your own
  // eyes.
  if (note.how === 'heard' && existing.how === 'been') {
    return state;
  }
  return {
    ...state,
    places: state.places.map((place) =>
      place === existing
        ? {
            ...place,
            what: note.what || place.what,
            how: note.how,
            who: note.how === 'heard' ? note.who ?? place.who : place.who,
            settled: note.how === 'been' ? true : place.settled
          }
        : place
    )
  };
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
      lines.push(`  ${place.where} - ${place.what}${place.who ? ` (${place.who} said so)` : ''}`);
    }
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
