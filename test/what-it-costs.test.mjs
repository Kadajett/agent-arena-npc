/**
 * The bill was reasoned about from outside twice and got wrong twice.
 *
 * Once the daily cost was estimated an order of magnitude low. Once a change
 * made specifically to save money raised it from $0.56 an hour to $0.75, and
 * the only reason anybody noticed was that the account emptied.
 *
 * There was nothing to reason with. The balance is one number for everything
 * at once, and OpenRouter refuses per-account activity to an inference key, so
 * an expensive prompt and a frequent one look identical from out there. Now
 * every call says what it cost, per character, and the question of which one
 * is eating the money stops being a matter of opinion.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import { costOf, meter, modelOf, tokensOf, spentSoFar, forgetSpending } from '../dist/harness/spend.js';

test('tokens are read whatever the SDK is calling them this week', () => {
  // These names have moved between versions, and a silent zero reads as a free
  // call, which is the wrong direction for a meter to be wrong in.
  assert.deepEqual(tokensOf({ usage: { promptTokens: 10, completionTokens: 2 } }), { prompt: 10, completion: 2 });
  assert.deepEqual(tokensOf({ usage: { inputTokens: 10, outputTokens: 2 } }), { prompt: 10, completion: 2 });
  assert.deepEqual(tokensOf({ usage: { prompt_tokens: 10, completion_tokens: 2 } }), { prompt: 10, completion: 2 });
});

test('a response with no usage at all is not counted as a free call', () => {
  assert.equal(tokensOf({}), null);
  assert.equal(tokensOf({ usage: {} }), null);
});

test('the model that answered is read off the reply, not off the agent', () => {
  // An agent names a preferred model with the free router underneath, so the
  // thing that answered may not be the thing that was asked. Costing a
  // fallback at the preferred price would misreport exactly the case the
  // fallback exists for.
  assert.equal(modelOf({ response: { modelId: 'inclusionai/ling-2.6-flash' } }), 'inclusionai/ling-2.6-flash');
  assert.equal(modelOf({ model: 'openrouter/free' }), 'openrouter/free');
  assert.equal(modelOf({}), '', 'and is honest about not knowing');
});

test('an unknown price reports as unknown rather than as free', () => {
  assert.equal(costOf('nobody/never-heard-of-it', 1000, 100), null);
});

test('a metered call adds to the running total', () => {
  forgetSpending();
  meter('Guy', 'thinking', { usage: { promptTokens: 1200, completionTokens: 40 } });
  meter('Guy', 'planning', { usage: { promptTokens: 800, completionTokens: 60 } });
  const spent = spentSoFar();
  assert.equal(spent.calls, 2);
  assert.equal(spent.prompt, 2000);
  assert.equal(spent.completion, 100);
});

test('a call with no usage does not inflate the call count', () => {
  forgetSpending();
  meter('Guy', 'thinking', { usage: { promptTokens: 100, completionTokens: 5 } });
  meter('Guy', 'thinking', {});
  assert.equal(spentSoFar().calls, 1, 'counting a call nobody can price hides the ones that count');
});

test('both places this library talks to a model are metered, not just the loud one', () => {
  // Deciding what to do and planning what to do next are different sizes of
  // prompt and were previously indistinguishable in one total.
  const behavior = readFileSync(new URL('../src/harness/behavior.ts', import.meta.url), 'utf8');
  const plan = readFileSync(new URL('../src/harness/plan.ts', import.meta.url), 'utf8');
  assert.match(behavior, /meter\([^)]*'thinking'/);
  assert.match(plan, /meter\([^)]*'planning'/);
  for (const [name, source] of [['behavior.ts', behavior], ['plan.ts', plan]]) {
    const calls = (source.match(/\.generate\(/g) ?? []).length;
    const meters = (source.match(/\bmeter\(/g) ?? []).length;
    assert.equal(meters, calls, `${name} has ${calls} model calls and ${meters} meters`);
  }
});

test('prices are fetched, not written down, so they cannot go stale in silence', () => {
  const source = readFileSync(new URL('../src/harness/spend.ts', import.meta.url), 'utf8');
  assert.match(source, /openrouter\.ai\/api\/v1\/models/);
  assert.ok(
    !/prompt:\s*0\.0000\d/.test(source),
    'a hardcoded price is wrong the first time somebody changes model, and wrong quietly'
  );
});

/**
 * The engine talks on the same channel people do.
 *
 * Reldens announces joins and leaves as bare keys - "chat.joinedRoom" - down
 * the channel dialogue arrives on. Nothing filtered them, so each one landed as
 * a line spoken by "someone", went into the transcript, and was written to
 * memory as something the character had heard. Guy's memory was largely this,
 * and every one of them was re-read, and paid for, on every tick after.
 */
test('the engine announcing a join is not somebody speaking', async () => {
  const { isEngineChatter } = await import('../dist/harness/arena.js');
  assert.equal(isEngineChatter('chat.joinedRoom'), true);
  assert.equal(isEngineChatter('"chat.joinedRoom"'), true, 'quoted, which is how it arrives');
  assert.equal(isEngineChatter('chat.leftRoom'), true, 'and whatever else it adds of that shape');
});

test('somebody actually talking is left alone', async () => {
  const { isEngineChatter } = await import('../dist/harness/arena.js');
  for (const said of [
    'Evening. Long Reach, eh. That is as good a name as any.',
    'I try the door.',
    'Barnaby used it? That is interesting.'
  ]) {
    assert.equal(isEngineChatter(said), false, said);
  }
});

test('compaction fires inside the window it is given, measured not assumed', async () => {
  const { buildMemory } = await import('../dist/harness/memory.js');
  const memory = buildMemory('cost-test', '/tmp');
  const config = memory.threadConfig ?? memory.getMergedThreadConfig?.() ?? {};
  // A message here averages about 420 tokens, so the window is the message
  // count times that, not the count times a guess. A threshold under the window
  // means compaction never catches up and runs flat out, which is what turned a
  // saving into a 34% increase.
  const windowTokens = config.lastMessages * 420;
  const fires = config.observationalMemory.observation.messageTokens;
  assert.ok(fires < windowTokens, `observing at ${fires} against a ${windowTokens} window would never catch up`);
  assert.ok(fires > windowTokens / 4, `observing at ${fires} against ${windowTokens} runs it flat out`);
});
