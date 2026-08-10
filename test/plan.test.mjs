/**
 * Pursuing something over weeks.
 *
 * The goal lives on the character sheet and nothing the character learns can
 * touch it. The plan toward it lives in memory, is written by the harness
 * rather than by the model remembering to, and survives a restart. These pin
 * both halves: that a character picks its plan back up where it left off, that
 * a step it reports finished is not offered again, and that a plan which has
 * stopped working gets replaced instead of retried forever.
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

test('the plan lives in memory but the goal never does', async () => {
  const memory = fakeMemory();
  const plan = planFor(fakeAgent(['{"steps": ["ask Barnaby"]}']), memory);
  await plan.load();
  await plan.refresh('');
  const stored = memory.read();
  assert.ok(Array.isArray(stored.plan));
  // Nothing written to memory can restate what the character is for: the goal
  // is on the sheet, and there is no field here to put a different one in.
  assert.equal('aim' in stored, false);
  assert.equal('goal' in stored, false);
});
