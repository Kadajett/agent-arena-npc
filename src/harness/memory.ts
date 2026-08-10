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
 * A character can record what it has learned about *other people* and what has
 * *happened*, and there is no field in which to store a different self. The
 * persona is the agent's instructions, a separate system message that memory
 * never writes to; this schema is why memory cannot smuggle a rewrite into the
 * fields it does control.
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
  // What the character is doing about what it wants. The goal itself is not in
  // here: that lives on the character sheet, where memory cannot reach it. This
  // is only the working out.
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
  // respect to the goal that produced it, and goals live on the character
  // sheet where they can be edited between deploys. Without this a character
  // carries on working a list toward something nobody wants any more.
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
