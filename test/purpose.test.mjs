/**
 * Keeping hold of what you were doing.
 *
 * A goal stated once at startup is gone by the second conversation, because
 * after that the only things in front of the model are a room and a line of
 * dialogue, and it will answer the dialogue. So the brief - what it wants,
 * where it has got to, what it owes, what it is holding in mind - is built into
 * every situation, and every prompt is built from a situation. That is the
 * thing worth pinning: not that the brief reads well, but that there is no
 * prompt a character can be asked which does not carry it.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { BOOKKEEPING, describeSituation } from '../dist/harness/behavior.js';
import { IntentSchema } from '../dist/harness/actions.js';

const BRIEF = [
  'What you are after: settle what is upstairs at the inn',
  '',
  'Things you said you would do:',
  "  1. bring Barnaby's cup back"
].join('\n');

const situation = (extra = {}) => ({
  scene: 'reldens-house-1',
  where: 'house 1',
  others: ['Barnaby'],
  heard: [],
  actions: '- "wait": stay where you are',
  places: '',
  conversation: [],
  wordiness: 35,
  purpose: BRIEF,
  known: '',
  strange: false,
  doors: '',
  view: '',
  harping: '',
  notes: [],
  people: '',
  ...extra
});

test('the brief is in the situation itself, so every prompt carries it', () => {
  const described = describeSituation(situation());
  assert.match(described, /settle what is upstairs at the inn/);
  assert.match(described, /bring Barnaby's cup back/);
});

test('it is still there when the character is deep in a conversation', () => {
  // The case that actually broke: the goal used to be appended by the
  // decide-what-to-do prompt only, so a character kept it right up until the
  // first person spoke to it and then made small talk for the rest of the day.
  const described = describeSituation(
    situation({
      conversation: [
        { from: 'Barnaby', message: 'You again.', fresh: false },
        { from: 'Wanderer', message: 'They call that the Long Field.', fresh: true }
      ]
    })
  );
  assert.match(described, /settle what is upstairs/);
  assert.ok(
    described.indexOf('settle what is upstairs') < described.indexOf('You are at'),
    'and it comes first, before the room and the talk'
  );
});

test('a character with nothing on is not given an empty heading', () => {
  const described = describeSituation(situation({ purpose: '' }));
  assert.ok(described.startsWith('You are at'));
});

test('bookkeeping rides along with whatever the character is doing', () => {
  // Walking out of a room and remembering why has to be one reply. A character
  // that must stand still for a turn to make a note will not make notes.
  const parsed = IntentSchema.safeParse({
    action: 'use_door',
    place: 'upstairs',
    message: 'Back in a moment.',
    remember: 'Barnaby went quiet when I asked',
    todo: 'get Barnaby to say what is up there',
    finished: '1',
    progress: 'same'
  });
  assert.equal(parsed.success, true);
  assert.equal(parsed.data.remember, 'Barnaby went quiet when I asked');
  assert.equal(parsed.data.finished, '1');
});

test('the character is told it can write these down without stopping', () => {
  assert.match(BOOKKEEPING, /"remember"/);
  assert.match(BOOKKEEPING, /"todo"/);
  assert.match(BOOKKEEPING, /"finished"/);
  assert.match(BOOKKEEPING, /alongside whatever you are doing/);
});

test('deciding what you want is an action, not something memory can be talked into', () => {
  // set_goal goes through the same intent surface as walking: the harness
  // applies it, so a goal changes exactly when the character chose to change
  // it and never because something it read said so.
  const parsed = IntentSchema.safeParse({
    action: 'set_goal',
    aim: 'find out who keeps moving the boundary stone',
    done: 'somebody admits to it',
    why: 'nobody would sell me the field and I want to know why'
  });
  assert.equal(parsed.success, true);
  assert.equal(parsed.data.action, 'set_goal');
});

test('a local can name the real places, and is told there are no others', async () => {
  const { describeLocalKnowledge, TOWN, INN } = await import('../dist/harness/world.js');
  const known = describeLocalKnowledge([TOWN], INN);
  assert.match(known, /the east gate/);
  assert.match(known, /outside the inn/);
  // The sentence that stops the gaps being filled in with a guildhall.
  assert.match(known, /no others you know of/);
});

test('a local is not told about the room they are already standing in', async () => {
  const { describeLocalKnowledge, TOWN } = await import('../dist/harness/world.js');
  // That room's places are already in the situation under "places you could go
  // from here"; listing them twice is paying for the same lines twice.
  assert.equal(describeLocalKnowledge([TOWN], TOWN), '');
});

test('a character given no local knowledge gets none', async () => {
  const { describeLocalKnowledge, INN } = await import('../dist/harness/world.js');
  assert.equal(describeLocalKnowledge([], INN), '');
});

test('a rumour gets stronger when repeated and weaker when somebody finds nothing', async () => {
  const { WorkingMemorySchema, notePlace, doubtPlace, standingOf } =
    await import('../dist/harness/memory.js');
  let state = notePlace(WorkingMemorySchema.parse({}), {
    where: 'the guildhall',
    what: 'on the corner',
    how: 'heard',
    who: 'Barnaby'
  });
  assert.match(standingOf(state.places[0]), /Barnaby says so.*you have not seen it/);

  state = notePlace(state, {
    where: 'the guildhall',
    what: 'on the corner',
    how: 'heard',
    who: 'Wanderer'
  });
  assert.match(standingOf(state.places[0]), /2 have now said it/);

  // Guy walks the whole town and finds no such building.
  state = doubtPlace(state, 'the guildhall');
  assert.match(standingOf(state.places[0]), /found nothing/);
  state = doubtPlace(state, 'the guildhall');
  assert.match(standingOf(state.places[0]), /take it as not there/);
});

test('nobody can talk a character out of a room it has stood in', async () => {
  const { WorkingMemorySchema, notePlace, doubtPlace, standingOf } =
    await import('../dist/harness/memory.js');
  let state = notePlace(WorkingMemorySchema.parse({}), {
    where: 'upstairs at the inn',
    what: 'four beds',
    how: 'been'
  });
  state = doubtPlace(state, 'upstairs at the inn');
  assert.equal(state.places[0].doubted, 0);
  assert.equal(standingOf(state.places[0]), 'you have been there');
});

test('going yourself clears what people said against it', async () => {
  const { WorkingMemorySchema, notePlace, doubtPlace } = await import('../dist/harness/memory.js');
  let state = notePlace(WorkingMemorySchema.parse({}), {
    where: 'the cellar',
    what: 'under the inn',
    how: 'heard',
    who: 'Barnaby'
  });
  state = doubtPlace(state, 'the cellar');
  // And then the character goes down and there it is.
  state = notePlace(state, { where: 'the cellar', what: 'barrels and damp', how: 'been' });
  assert.equal(state.places[0].how, 'been');
  assert.equal(state.places[0].doubted, 0, 'what you can see stops being argued with');
});

test('doors are named by where they lead and which way they lie', async () => {
  const { describeDoors } = await import('../dist/harness/explore.js');
  const view = {
    scene: 'reldens-town',
    map: '',
    widthTiles: 48,
    heightTiles: 28,
    doors: [
      { x: 0, y: 0, row: 10, column: 12, leadsTo: 'reldens-house-1', locked: false, lockKnown: true },
      { x: 0, y: 0, row: 20, column: 39, leadsTo: 'reldens-house-2', locked: false, lockKnown: true }
    ]
  };
  const described = describeDoors(view, (scene) => scene, { row: 20, column: 12 });
  // Two doors that used to read as "door 1" and "door 2" - which is how Guy
  // announced he was off to the far house and walked back into the inn.
  assert.match(described, /north of you, about 10 paces/);
  assert.match(described, /east of you, about 27 paces/);
});

test('a reply is not thrown away over how it worded its progress', async () => {
  const { IntentSchema } = await import('../dist/harness/actions.js');
  // What actually happened: the model said "doing", the strict enum rejected
  // the whole intent, and the character stood still for a tick over one word.
  for (const [said, meant] of [
    ['doing', 'same'],
    ['in progress', 'same'],
    ['ongoing', 'same'],
    ['completed', 'done'],
    ['Done', 'done'],
    ['stuck', 'blocked']
  ]) {
    const parsed = IntentSchema.safeParse({ action: 'walk', place: 'the east gate', progress: said });
    assert.equal(parsed.success, true, `"${said}" should parse`);
    assert.equal(parsed.data.progress, meant, `"${said}" means ${meant}`);
  }
  // And leaving it out is still fine.
  assert.equal(IntentSchema.safeParse({ action: 'wait' }).data.progress, undefined);
});

test('the harness puts back what the model overwrites', async () => {
  const { Plan } = await import('../dist/harness/plan.js');
  // Working memory is one record with two writers. The model has a tool for it
  // and writes the whole record as it understands it, dropping every field it
  // was never shown. Barnaby ended up with nothing in memory but two people.
  let stored = null;
  const memory = {
    async getWorkingMemory() {
      return stored;
    },
    async updateWorkingMemory({ workingMemory }) {
      stored = workingMemory;
    }
  };
  const agent = { async generate() { return { text: '{"steps": ["ask Barnaby"]}' }; } };
  const plan = new Plan(agent, { resource: 'r', thread: 't' }, { aim: 'find the cellar' },
    async () => memory);
  await plan.load();
  await plan.refresh('');
  await plan.take('bring the cup back');
  assert.equal(JSON.parse(stored).plan.length, 1);

  // The model writes its own view: it knows about people and nothing else.
  stored = JSON.stringify({
    people: [{ name: 'Guy', about: 'nosy', feeling: 'wary', why: 'asks about the stairs', lastSeen: 'today' }]
  });

  await plan.keep();
  const after = JSON.parse(stored);
  assert.equal(after.people.length, 1, "the model's own write survives");
  assert.equal(after.plan.length, 1, 'and the plan is back');
  assert.equal(after.todo.length, 1, 'and so is the list');
  assert.equal(after.goal.aim, 'find the cellar');
});

test('a room you stood in is never crowded out by things people said', async () => {
  const { WorkingMemorySchema, notePlace } = await import('../dist/harness/memory.js');
  let state = WorkingMemorySchema.parse({});
  // Two places the character actually walked through, early on.
  state = notePlace(state, { where: 'upstairs at the inn', what: 'four beds', how: 'been' });
  state = notePlace(state, { where: 'the cellar', what: 'barrels', how: 'been' });
  // Then a bar's worth of talk, far past the cap.
  for (let n = 0; n < 60; n++) {
    state = notePlace(state, { where: `rumour ${n}`, what: 'so they say', how: 'heard', who: 'somebody' });
  }
  const been = state.places.filter((place) => place.how === 'been').map((place) => place.where);
  assert.deepEqual(been, ['upstairs at the inn', 'the cellar'], 'both sightings survive');
  assert.ok(state.places.length <= 24, `list stays bounded, got ${state.places.length}`);
  assert.ok(state.places.some((place) => place.where === 'rumour 59'), 'and recent talk is kept');
});

test('being doubted does not shake what a character saw itself', async () => {
  const { WorkingMemorySchema, notePlace, doubtPlace, standingOf } =
    await import('../dist/harness/memory.js');
  let state = notePlace(WorkingMemorySchema.parse({}), {
    where: 'the cellar', what: 'barrels and damp', how: 'been'
  });
  for (let n = 0; n < 20; n++) {
    state = doubtPlace(state, 'the cellar');
  }
  assert.equal(state.places[0].doubted, 0, 'twenty people saying otherwise changes nothing');
  assert.equal(standingOf(state.places[0]), 'you have been there');
});

test('a face is written down for standing there, and re-seeing it is not a first meeting', async () => {
  const { WorkingMemorySchema, noteFace, hasMet } = await import('../dist/harness/memory.js');
  let state = WorkingMemorySchema.parse({});
  assert.equal(hasMet(state, 'Barnaby'), false);

  state = noteFace(state, 'Barnaby', 'the inn');
  assert.equal(hasMet(state, 'Barnaby'), true, 'seen once is met');
  assert.equal(state.people.length, 1);
  assert.equal(state.people[0].feeling, 'neutral', 'standing there is not an opinion');

  // The same person, over and over, the way a bar actually works.
  for (let n = 0; n < 30; n++) {
    state = noteFace(state, 'Barnaby', 'the inn');
  }
  assert.equal(state.people.length, 1, 'one person, not thirty-one');
  assert.match(state.people[0].lastSeen, /just now/);
  // Case and stray spacing are how the same name arrives from two code paths.
  assert.equal(hasMet(state, '  barnaby '), true);
});

test('noting a face does not overwrite what the character already thought of them', async () => {
  const { WorkingMemorySchema, noteFace } = await import('../dist/harness/memory.js');
  const state = WorkingMemorySchema.parse({
    people: [{
      name: 'Guy', about: 'owes you eight copper', feeling: 'wary',
      why: 'he has not paid', lastSeen: 'yesterday'
    }]
  });
  const after = noteFace(state, 'Guy', 'the inn');
  assert.equal(after.people[0].feeling, 'wary', 'an opinion survives being seen again');
  assert.equal(after.people[0].about, 'owes you eight copper');
  assert.match(after.people[0].lastSeen, /just now, the inn/);
});

test('a room of familiar faces is described as familiar, and a newcomer is not', async () => {
  const { describeSituation } = await import('../dist/harness/behavior.js');
  const base = {
    scene: 'reldens-house-1', where: 'the inn', heard: [], actions: '', places: '',
    conversation: [], wordiness: 30, purpose: '', known: '', strange: false,
    doors: '', view: '', harping: '', notes: [], people: ''
  };
  const regulars = describeSituation({
    ...base,
    others: [{ name: 'Guy', known: true }, { name: 'Barnaby', known: true }]
  });
  assert.match(regulars, /Also here: Guy, Barnaby\./);
  assert.match(regulars, /You know everyone here\./);
  assert.doesNotMatch(regulars, /new to you/);

  const withStranger = describeSituation({
    ...base,
    others: [{ name: 'Guy', known: true }, { name: 'Mamon', known: false }]
  });
  assert.match(withStranger, /Mamon \(new to you\)/);
  assert.doesNotMatch(withStranger, /You know everyone here/);
});

test('what gets stored for a turn is one line, not the whole brief', async () => {
  const { momentOf } = await import('../dist/harness/behavior.js');
  const base = {
    scene: 'reldens-house-1', where: 'the inn', others: [], heard: [], actions: '',
    places: '', conversation: [], wordiness: 30, purpose: '', known: '', strange: false,
    doors: '', view: '', harping: '', notes: [], people: ''
  };
  assert.equal(momentOf(base), '[the inn, alone]');
  assert.equal(
    momentOf({ ...base, others: [{ name: 'Guy', known: true }] }),
    '[the inn, with Guy]'
  );
  assert.equal(
    momentOf({ ...base, heard: [{ from: 'Guy', message: 'Evening.' }] }),
    'Guy: "Evening."'
  );
  // The whole point: a stored turn stays small no matter how big the brief is.
  const busy = momentOf({
    ...base,
    view: 'x'.repeat(5000),
    doors: 'y'.repeat(5000),
    heard: [{ from: 'Guy', message: 'Evening.' }]
  });
  assert.ok(busy.length < 100, `a turn costs a line, got ${busy.length} characters`);
});

test('the list holds a real afternoon of promises, and drops the settled ones first', async () => {
  const { WorkingMemorySchema, addTodo, settleTodo, describeTodo } =
    await import('../dist/harness/memory.js');
  let state = WorkingMemorySchema.parse({});
  for (let n = 0; n < 12; n++) {
    state = addTodo(state, `errand ${n}`);
  }
  // Twelve outstanding things would have been silently truncated to ten before.
  assert.equal(state.todo.length, 12, 'a list, not a checklist');

  // Settled items are what gives way when room runs out, not open ones.
  state = settleTodo(state, 'errand 0', 'done');
  state = settleTodo(state, 'errand 1', 'blocked');
  for (let n = 12; n < 40; n++) {
    state = addTodo(state, `errand ${n}`);
  }
  assert.ok(state.todo.length <= 30, `stays bounded, got ${state.todo.length}`);
  const open = state.todo.filter((item) => item.status === 'next' || item.status === 'doing');
  assert.equal(open.length, 30, 'the list fills with what is still outstanding');
  assert.equal(
    state.todo.some((item) => item.status === 'done' || item.status === 'blocked'),
    false,
    'things already dealt with are dropped before anything still owed'
  );

  const shown = describeTodo(settleTodo(state, 'errand 39', 'done'));
  assert.match(shown, /Already settled:/);
  assert.match(shown, /errand 39 - done/);
});

test('an item carries how long it has been carried and what has been found out', async () => {
  const { WorkingMemorySchema, addTodo, noteOnTodo, describeTodo } =
    await import('../dist/harness/memory.js');
  const then = Date.parse('2026-08-10T07:00:00.000Z');
  const now = then + 95 * 60_000;
  let state = addTodo(WorkingMemorySchema.parse({}), 'ask about the crypt', then);
  assert.match(describeTodo(state, now), /1\. ask about the crypt \(since 2 hours ago\)/);

  state = noteOnTodo(state, '1', 'Barnaby says it is out past the grassland');
  assert.equal(state.todo[0].status, 'doing', 'finding something out means it is under way');
  assert.match(describeTodo(state, now), /so far: Barnaby says it is out past the grassland/);
});

test('a character can think back on somebody, and knows when it does not know', async () => {
  const { WorkingMemorySchema, noteFace, notePlace, recallAbout } =
    await import('../dist/harness/memory.js');
  let state = noteFace(WorkingMemorySchema.parse({}), 'Barnaby', 'the inn');
  state = notePlace(state, { where: 'the cellar', what: 'barrels', how: 'been' });

  const aboutHim = recallAbout(state, 'barnaby');
  assert.match(aboutHim, /Barnaby/);
  assert.match(aboutHim, /the inn/);
  // Asked after loosely, the way a person refers to somebody.
  assert.match(recallAbout(state, 'the cellar'), /you have been there/);

  const nothing = recallAbout(state, 'the Grey Hall');
  assert.match(nothing, /nothing comes/);
  assert.match(nothing, /no such person or place/);
  assert.equal(recallAbout(state, '   '), '', 'asking nothing is not a question');
});

test('a note stays a note, so the whole record stays writable', async () => {
  const { WorkingMemorySchema, noteToSelf, liveNotes } = await import('../dist/harness/memory.js');
  const essay = 'Maud Kettleworth ran the inn before Barnaby and won the sign off him in a '
    + 'card game one winter, and the fireplace corner still carries the ring where the soup '
    + 'pot boiled over, which is the sort of mark that outlives the story attached to it, and '
    + 'I have decided to call that corner the Kettleworth Nook whatever anybody else says.';
  const state = noteToSelf(WorkingMemorySchema.parse({}), essay);
  const [note] = liveNotes(state);
  assert.ok(note.text.length <= 244, `kept short, got ${note.text.length}`);
  assert.match(note.text, /\.\.\.$/, 'and says it was cut');
  assert.match(note.text, /^Maud Kettleworth ran the inn/, 'from the start, not the middle');
  assert.doesNotMatch(note.text, / $/, 'no trailing space before the ellipsis');

  // A note that already fits is left exactly alone.
  const short = noteToSelf(WorkingMemorySchema.parse({}), 'Barnaby is fetching the key');
  assert.equal(liveNotes(short)[0].text, 'Barnaby is fetching the key');
});

test('a near-miss action is read as what it meant, not thrown away', async () => {
  const { IntentSchema } = await import('../dist/harness/actions.js');
  // The one seen in the logs: three characters writing {"action": "look"}.
  const looked = IntentSchema.safeParse({ action: 'look' });
  assert.equal(looked.success, true);
  assert.equal(looked.data.action, 'explore');

  // A sign is an NPC that cannot walk, so reading one is talking to it.
  assert.equal(IntentSchema.safeParse({ action: 'read' }).data.action, 'talk_to');
  assert.equal(IntentSchema.safeParse({ action: 'Look Around' }).data.action, 'explore');
  assert.equal(IntentSchema.safeParse({ action: 'fight' }).data.action, 'attack');
  assert.equal(IntentSchema.safeParse({ action: 'nothing' }).data.action, 'wait');
  // A real action is never rewritten into something else.
  assert.equal(IntentSchema.safeParse({ action: 'walk' }).data.action, 'walk');
  assert.equal(IntentSchema.safeParse({ action: 'talk_to' }).data.action, 'talk_to');
  // And something with no sensible reading is still refused.
  assert.equal(IntentSchema.safeParse({ action: 'transmogrify' }).success, false);
});

test('an unreadable action does not cost the character the note it just made', async () => {
  const { askForIntent } = await import('../dist/harness/behavior.js');
  const agent = {
    async generate() {
      // Action is nonsense, but the promise alongside it is perfectly good.
      return { text: '{"action":"transmogrify","remember":"Barnaby went quiet when I asked",'
        + '"todo":"go and look upstairs","noted":"the Kettleworth Nook"}' };
    }
  };
  const intent = await askForIntent(agent, 'brief', { resource: 'r', thread: 't' }, 'moment', 'persona');
  assert.equal(intent.action, 'wait', 'it stands still for the turn');
  assert.equal(intent.remember, 'Barnaby went quiet when I asked', 'and still remembers');
  assert.equal(intent.todo, 'go and look upstairs');
  assert.equal(intent.noted, 'the Kettleworth Nook');
});

test('an unreadable action does not cost a favour its asker either', async () => {
  const { askForIntent } = await import('../dist/harness/behavior.js');
  const agent = {
    async generate() {
      return {
        text: '{"action":"transmogrify","todo":"bring Miles a tree branch for a coin",'
          + '"askedBy":"Miles"}'
      };
    }
  };
  const intent = await askForIntent(agent, 'brief', { resource: 'r', thread: 't' }, 'moment', 'persona');
  assert.equal(intent.action, 'wait');
  assert.equal(intent.todo, 'bring Miles a tree branch for a coin');
  assert.equal(intent.askedBy, 'Miles');
});

test('a reply in plain prose is said out loud, not thrown away', async () => {
  // The real thing found in the logs: Barnaby, spoken to, answered in his own
  // voice with no JSON wrapper at all, and the whole turn was binned. 20% of
  // the Wanderer's turns and 12% of Guy's went the same way over an hour.
  const { askForIntent } = await import('../dist/harness/behavior.js');
  const agent = {
    async generate() {
      return {
        text: "Barnaby, another for the road. And one for my new friend here, whoever "
          + "they are - I've been naming the rooms in this place all day and I need "
          + 'a fresh pair of ears.'
      };
    }
  };
  const intent = await askForIntent(agent, 'brief', { resource: 'r', thread: 't' }, 'moment', 'persona');
  assert.equal(intent.action, 'say');
  assert.match(intent.message, /another for the road/);
});

test('a second prose reply, also plainly meant to be heard, is kept too', async () => {
  const { askForIntent } = await import('../dist/harness/behavior.js');
  const agent = {
    async generate() {
      return {
        text: "I'm out on the street now. Good to be in the open air. I've been cooped "
          + "up in the Kettleworth Keep - that's what I'm calling the inn, until "
          + 'somebody tells me a better name.'
      };
    }
  };
  const intent = await askForIntent(agent, 'brief', { resource: 'r', thread: 't' }, 'moment', 'persona');
  assert.equal(intent.action, 'say');
  assert.match(intent.message, /Kettleworth Keep/);
});

test('a reply that is really the character working out a route stands still instead', async () => {
  // The third real example: not speech, a character thinking through a move.
  // Said aloud, "about fifteen paces, let me go" reads as a character talking
  // to itself in the street, so this one is left as "wait", same as an
  // unreadable reply always was.
  const { askForIntent } = await import('../dist/harness/behavior.js');
  const agent = {
    async generate() {
      return { text: 'I need to walk south to reach the doorway. About fifteen paces. Let me go.' };
    }
  };
  const intent = await askForIntent(agent, 'brief', { resource: 'r', thread: 't' }, 'moment', 'persona');
  assert.equal(intent.action, 'wait');
  assert.equal(intent.message, undefined);
});

test('a genuinely empty reply is still just a wait, not an empty thing said', async () => {
  const { askForIntent } = await import('../dist/harness/behavior.js');
  const agent = { async generate() { return { text: '   ' }; } };
  const intent = await askForIntent(agent, 'brief', { resource: 'r', thread: 't' }, 'moment', 'persona');
  assert.equal(intent.action, 'wait');
});

test('the raw abort text from a timed-out call is not handed to the character', async () => {
  // Seen in the logs: "use_door -> The operation was aborted due to timeout",
  // straight from arena.ts's AbortSignal.timeout() with none of the other
  // timeout shapes' friendlier wording.
  const { whatWentWrong } = await import('../dist/harness/actions.js');
  const said = whatWentWrong(new Error('The operation was aborted due to timeout'));
  assert.match(said, /waited, and nothing came of it/);
  assert.doesNotMatch(said, /aborted/i);
});

test('an NPC that does not answer costs a turn, not the whole session', async () => {
  const { Actions } = await import('../dist/harness/actions.js');
  const arena = {
    async call(tool) {
      if (tool === 'arena_talk_to') {
        throw new Error('arena_talk_to: RELDENS_TIMEOUT: Timed out waiting for that NPC to respond.');
      }
      return {};
    }
  };
  const actions = new Actions(arena, 'agent-1', new Set(['talk_to_folk', 'speak']));
  actions.notices([{ label: 'Alfred', kind: 'npc', objectId: 5, tileX: 1, tileY: 1 }]);
  // Must not throw: throwing is what tore the session down and lost the plan.
  const result = await actions.perform({ action: 'talk_to', target: 'Alfred' }, 'reldens-town');
  assert.equal(result.ok, false);
  assert.match(result.note, /waited, and nothing came of it/);
});

test('losing the body is still worth reconnecting for', async () => {
  const { Actions, isDisconnected, whatWentWrong } = await import('../dist/harness/actions.js');
  assert.equal(isDisconnected(new Error('arena_observe: AGENT_NOT_CONNECTED: gone')), true);
  assert.equal(isDisconnected(new Error('tools/call: Invalid or missing MCP session ID.')), true);
  assert.equal(isDisconnected(new Error('arena_talk_to: RELDENS_TIMEOUT: no answer')), false);
  assert.match(whatWentWrong(new Error('RELDENS_TIMEOUT: x')), /waited/);

  const arena = { async call() { throw new Error('arena_talk_to: AGENT_NOT_CONNECTED: gone'); } };
  const actions = new Actions(arena, 'agent-1', new Set(['talk_to_folk']));
  actions.notices([{ label: 'Alfred', kind: 'npc', objectId: 5, tileX: 1, tileY: 1 }]);
  await assert.rejects(
    () => actions.perform({ action: 'talk_to', target: 'Alfred' }, 'reldens-town'),
    /AGENT_NOT_CONNECTED/,
    'this one must still come up through the tick loop'
  );
});
