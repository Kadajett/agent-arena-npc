/**
 * Pursuing something over weeks.
 *
 * The goal, the plan toward it, the character's own list and its short-lived
 * notes all live in memory, are written by the harness rather than by the model
 * remembering to, and survive a restart. These pin the parts that quietly break
 * otherwise: that a character picks its plan back up where it left off, that a
 * step it reported finished is not offered again, that a plan which has stopped
 * working gets replaced rather than retried forever, and that a goal it chose
 * for itself is not wiped every time the process restarts.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Plan } from '../dist/harness/plan.js';
import { WorkingMemorySchema } from '../dist/harness/memory.js';

/** A memory store that behaves like Mastra's: strings in, strings out. */
function fakeMemory(initial = null) {
  let stored = initial === null ? null : JSON.stringify(initial);
  return {
    async getWorkingMemory() {
      return stored;
    },
    async updateWorkingMemory({ workingMemory }) {
      stored = workingMemory;
    },
    read: () => (stored === null ? null : JSON.parse(stored))
  };
}

function fakeAgent(replies) {
  const asked = [];
  return {
    asked,
    async generate(prompt) {
      asked.push(prompt);
      return { text: replies.shift() ?? '{}' };
    }
  };
}

const scope = { resource: 'npc-test', thread: 'npc-test-life' };
const goal = { aim: 'buy the field past the east gate', done: 'you own it' };
const planFor = (agent, memory, aim = goal) =>
  new Plan(agent, scope, aim, async () => memory);

test('makes a plan when it has none, and works the first step', async () => {
  const memory = fakeMemory();
  const agent = fakeAgent(['{"steps": ["ask Barnaby who owns it", "count what you have"]}']);
  const plan = planFor(agent, memory);
  await plan.load();

  assert.equal(await plan.refresh('You are at town.'), true);
  assert.equal(plan.current().what, 'ask Barnaby who owns it');
  assert.match(plan.describe(), /buy the field past the east gate/);
  assert.match(plan.describe(), /Work on this now: ask Barnaby/);
});

test('a finished step is not offered again', async () => {
  const memory = fakeMemory();
  const plan = planFor(fakeAgent(['{"steps": ["ask Barnaby", "count your money"]}']), memory);
  await plan.load();
  await plan.refresh('');

  await plan.record('say', 'said: who owns the field?', 'done');
  assert.equal(plan.current().what, 'count your money');
});

test('picks the plan back up after a restart', async () => {
  const memory = fakeMemory();
  const first = planFor(fakeAgent(['{"steps": ["ask Barnaby", "count your money"]}']), memory);
  await first.load();
  await first.refresh('');
  await first.record('say', 'asked him', 'done');

  // A new process, the same memory file.
  const second = planFor(fakeAgent([]), memory);
  await second.load();
  assert.equal(second.current().what, 'count your money');
});

test('a plan that keeps getting blocked is replaced', async () => {
  const memory = fakeMemory();
  const agent = fakeAgent([
    '{"steps": ["walk to the east gate", "walk to the docks", "walk to the mine"]}',
    '{"steps": ["ask someone instead"]}'
  ]);
  const plan = planFor(agent, memory);
  await plan.load();
  await plan.refresh('');

  for (let attempt = 0; attempt < 3; attempt++) {
    await plan.record('walk', 'there is no way through', 'blocked');
  }
  assert.equal(await plan.refresh('You are at town.'), true);
  assert.equal(plan.current().what, 'ask someone instead');
  // What failed is put in front of the model so it does not plan it again.
  assert.match(agent.asked[1], /do not plan these again/);
  assert.match(agent.asked[1], /walk to the east gate/);
});

test('does not spend a call replanning while the plan is still good', async () => {
  const memory = fakeMemory();
  const agent = fakeAgent(['{"steps": ["ask Barnaby", "count your money"]}']);
  const plan = planFor(agent, memory);
  await plan.load();
  await plan.refresh('');

  assert.equal(await plan.refresh(''), false);
  assert.equal(agent.asked.length, 1, 'asked once, not twice');
});

test('an unreadable reply leaves the old plan alone', async () => {
  const memory = fakeMemory();
  const agent = fakeAgent(['{"steps": ["ask Barnaby"]}', 'I am afraid I cannot do that']);
  const plan = planFor(agent, memory);
  await plan.load();
  await plan.refresh('');
  await plan.record('say', 'asked him', 'done');

  assert.equal(await plan.refresh(''), false);
  assert.deepEqual(
    plan.steps.map((step) => step.status),
    ['done']
  );
});

test('changing the goal retires the plan made for the old one', async () => {
  const memory = fakeMemory();
  const first = planFor(fakeAgent(['{"steps": ["ask the price of the field"]}']), memory);
  await first.load();
  await first.refresh('');
  assert.equal(first.current().what, 'ask the price of the field');

  // The same character, redeployed with a different goal on its sheet.
  const second = new Plan(
    fakeAgent(['{"steps": ["go up the inn stairs and look"]}']),
    scope,
    { aim: 'settle what is upstairs at the inn' },
    async () => memory
  );
  await second.load();
  assert.equal(second.current(), null, 'the old plan is gone, not carried on with');

  assert.equal(await second.refresh(''), true);
  assert.equal(second.current().what, 'go up the inn stairs and look');
});

test('the same goal keeps its plan across a restart', async () => {
  const memory = fakeMemory();
  const first = planFor(fakeAgent(['{"steps": ["ask Barnaby", "count your money"]}']), memory);
  await first.load();
  await first.refresh('');
  await first.record('say', 'asked him', 'done');

  const second = planFor(fakeAgent([]), memory);
  await second.load();
  assert.equal(second.current().what, 'count your money');
});

test('a character with no goal keeps no plan', async () => {
  const memory = fakeMemory();
  const agent = fakeAgent(['{"steps": ["something"]}']);
  const plan = new Plan(agent, scope, undefined, async () => memory);
  await plan.load();

  assert.equal(plan.hasGoal, false);
  assert.equal(await plan.refresh(''), false);
  assert.equal(plan.describe(), '');
  assert.equal(agent.asked.length, 0);
});

test('what it did lately is remembered, and does not grow forever', async () => {
  const memory = fakeMemory();
  const plan = planFor(fakeAgent(['{"steps": ["ask Barnaby"]}']), memory);
  await plan.load();
  await plan.refresh('');
  for (let tick = 0; tick < 30; tick++) {
    await plan.record('walk', `attempt ${tick}`, 'same');
  }
  const stored = WorkingMemorySchema.parse(memory.read());
  assert.ok(stored.lately.length <= 12, `kept ${stored.lately.length}`);
  assert.match(stored.lately.at(-1), /attempt 29/);
});

test('the goal is kept in memory, where the character can reach it', async () => {
  const memory = fakeMemory();
  const plan = planFor(fakeAgent(['{"steps": ["ask Barnaby"]}']), memory);
  await plan.load();
  await plan.refresh('');
  const stored = memory.read();
  assert.ok(Array.isArray(stored.plan));
  assert.equal(stored.goal.aim, goal.aim);
  // Wanting something is not being someone. There is still no field here in
  // which to write a different self: the persona is a system message memory
  // never touches.
  assert.equal('persona' in stored, false);
  assert.equal('instructions' in stored, false);
});

test('a character can decide it wants something else, and the old plan goes with it', async () => {
  const memory = fakeMemory();
  const plan = planFor(fakeAgent(['{"steps": ["ask the price of the field"]}']), memory);
  await plan.load();
  await plan.refresh('');
  assert.equal(plan.goalIsOwn, true, 'seeded from the sheet, but held as its own');

  const changed = await plan.setGoal(
    'find out who keeps moving the boundary stone',
    'somebody admits to moving it',
    'nobody would sell me the field and I want to know why'
  );
  assert.equal(changed, true);
  assert.equal(plan.current(), null, 'the steps toward the old goal are gone');
  assert.match(plan.describe(), /boundary stone/);
  // In its own words, because a character talks itself out of an instruction
  // more easily than out of its own reason.
  assert.match(plan.describe(), /You settled on this yourself because: nobody would sell me/);
});

test('a goal it chose survives a restart, and is not overwritten by its sheet', async () => {
  const memory = fakeMemory();
  const first = planFor(fakeAgent(['{"steps": ["ask the price"]}']), memory);
  await first.load();
  await first.setGoal('learn to fish', 'you have caught one', 'the field was a dead end');

  // Same character, same sheet, restarted. The sheet has not changed, so the
  // goal it settled on stands: taking it away every restart makes choosing one
  // pointless.
  const second = planFor(fakeAgent([]), memory);
  await second.load();
  assert.equal(second.goal.aim, 'learn to fish');
});

test('editing the sheet still redirects a character that has settled on something', async () => {
  const memory = fakeMemory();
  const first = planFor(fakeAgent([]), memory);
  await first.load();
  await first.setGoal('learn to fish', 'you have caught one', 'it looked restful');

  // Redeployed with a different goal on the sheet: the operator has changed
  // their mind, and that is the one way to point a character somewhere new.
  const second = new Plan(
    fakeAgent([]),
    scope,
    { aim: 'settle what is upstairs at the inn' },
    async () => memory
  );
  await second.load();
  assert.equal(second.goal.aim, 'settle what is upstairs at the inn');
  assert.equal(second.current(), null, 'and the fishing plan does not come with it');
});

test('the same aim set twice is not a change of heart', async () => {
  const memory = fakeMemory();
  const plan = planFor(fakeAgent(['{"steps": ["ask Barnaby"]}']), memory);
  await plan.load();
  await plan.refresh('');

  assert.equal(await plan.setGoal(goal.aim.toUpperCase(), '', ''), false);
  assert.equal(plan.current().what, 'ask Barnaby', 'the plan is left alone');
});

test('the list and the notes are in every brief, alongside the goal', async () => {
  const memory = fakeMemory();
  const plan = planFor(fakeAgent(['{"steps": ["ask Barnaby"]}']), memory);
  await plan.load();
  await plan.refresh('');
  await plan.take("bring Barnaby's cup back");
  await plan.note('the Wanderer went upstairs');

  const brief = plan.describe();
  assert.match(brief, /buy the field past the east gate/);
  assert.match(brief, /Work on this now: ask Barnaby/);
  assert.match(brief, /1\. bring Barnaby's cup back/);
  assert.match(brief, /the Wanderer went upstairs \(just now\)/);
  // The reason all of this is repeated on every single call.
  assert.match(brief, /does not replace it/);
});

test('an item is crossed off by its number, the way it was shown', async () => {
  const memory = fakeMemory();
  const plan = planFor(fakeAgent([]), memory);
  await plan.load();
  await plan.take('find the key');
  await plan.take("bring Barnaby's cup back");

  await plan.settle('2', 'done');
  const brief = plan.describe();
  assert.match(brief, /1\. find the key/);
  assert.doesNotMatch(brief, /1\. bring Barnaby's cup back|2\. /);
  assert.match(brief, /cup back - done/);
});

test('a note is gone an hour later, without anything having to run', async () => {
  const memory = fakeMemory();
  const plan = planFor(fakeAgent([]), memory);
  await plan.load();
  await plan.note('Barnaby is fetching the key');
  assert.match(plan.describe(), /Barnaby is fetching the key/);

  const anHourAndAbit = Date.now() + 61 * 60 * 1000;
  assert.doesNotMatch(plan.describe(anHourAndAbit), /fetching the key/);
});

test('the plan does not overwrite what the rest of memory wrote', async () => {
  const memory = fakeMemory();
  const plan = planFor(fakeAgent(['{"steps": ["ask Barnaby"]}']), memory);
  await plan.load();

  // The harness notes a place between the plan being loaded and saved, which
  // is the ordinary case: a character walks into a room every few seconds.
  const state = WorkingMemorySchema.parse(memory.read() ?? {});
  await memory.updateWorkingMemory({
    workingMemory: JSON.stringify({
      ...state,
      places: [{ where: 'upstairs at the inn', what: 'you have been in', how: 'been', who: '', settled: true }]
    })
  });

  await plan.refresh('');
  const stored = WorkingMemorySchema.parse(memory.read());
  assert.equal(stored.places.length, 1, 'the room it found is still there');
  assert.equal(stored.plan.length, 1, 'and so is the plan');
});
