/**
 * The harness folds history down itself, because Mastra never will.
 *
 * Found with the prompt trace, settled with a local reproduction against a
 * copy of Ash's live memory: getStatus() said shouldObserve with 55,179
 * pending tokens against a 5,000 threshold, om.observe() worked the moment it
 * was called, and production had never called it once. Mastra's threshold
 * observation runs while preparing step two of a tool-loop turn; these
 * characters' turns are one generate() with no tool loop, so there is never a
 * step two. Its idle-buffering fallback only fires while pending is still
 * BELOW threshold, which it never is again once observation has been missed
 * once. Wedged from both sides, for every character at once, silently -
 * which is how the Wanderer came to carry 1,332 messages into every prompt
 * while lastMessages said sixteen.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import { keepMemoryDigested } from '../dist/harness/memory.js';

const SCOPE = { resource: 'npc-test', thread: 'npc-test-life' };

function memoryWhoseEngine(engine) {
  return { omEngine: Promise.resolve(engine) };
}

test('when observation is due, it runs, and says what it folded', async () => {
  const observed = [];
  let pending = 55179;
  const digest = await keepMemoryDigested(
    memoryWhoseEngine({
      getStatus: async () => ({ shouldObserve: pending > 5000, pendingTokens: pending }),
      observe: async (where) => {
        observed.push(where);
        pending = 3000;
      }
    }),
    SCOPE
  );
  assert.equal(digest.did, 'observed');
  assert.deepEqual(observed, [{ threadId: SCOPE.thread, resourceId: SCOPE.resource }]);
  assert.match(digest.note, /55179/, 'what it started with');
  assert.match(digest.note, /3000/, 'and what was left');
});

test('nothing due means nothing run', async () => {
  let called = 0;
  const digest = await keepMemoryDigested(
    memoryWhoseEngine({
      getStatus: async () => ({ shouldObserve: false, pendingTokens: 900 }),
      observe: async () => called++
    }),
    SCOPE
  );
  assert.equal(digest.did, 'settled');
  assert.equal(called, 0);
});

test('a second call while one is running backs off instead of doubling up', async () => {
  // Two concurrent observes over one SQLite file is the WAL trouble this
  // codebase has already paid for once. The guard is per thread.
  let release;
  const gate = new Promise((resolve) => { release = resolve; });
  const slow = memoryWhoseEngine({
    getStatus: async () => ({ shouldObserve: true, pendingTokens: 10_000 }),
    observe: () => gate
  });
  const first = keepMemoryDigested(slow, SCOPE);
  const second = await keepMemoryDigested(slow, SCOPE);
  assert.equal(second.did, 'busy');
  release();
  assert.equal((await first).did, 'observed');
});

test('two characters digest independently', async () => {
  let release;
  const gate = new Promise((resolve) => { release = resolve; });
  const slow = memoryWhoseEngine({
    getStatus: async () => ({ shouldObserve: true, pendingTokens: 10_000 }),
    observe: () => gate
  });
  const first = keepMemoryDigested(slow, SCOPE);
  const other = await keepMemoryDigested(
    memoryWhoseEngine({
      getStatus: async () => ({ shouldObserve: false, pendingTokens: 0 }),
      observe: async () => {}
    }),
    { resource: 'npc-other', thread: 'npc-other-life' }
  );
  assert.equal(other.did, 'settled', 'a different thread is not blocked');
  release();
  await first;
});

test('a failing observe reports failure and releases the guard', async () => {
  const memory = memoryWhoseEngine({
    getStatus: async () => ({ shouldObserve: true, pendingTokens: 10_000 }),
    observe: async () => {
      throw new Error('context length exceeded');
    }
  });
  const failed = await keepMemoryDigested(memory, SCOPE);
  assert.equal(failed.did, 'failed');
  assert.match(failed.note, /context length exceeded/);
  const again = await keepMemoryDigested(memory, SCOPE);
  assert.equal(again.did, 'failed', 'the guard was released, so it can try again');
});

test('no memory and no engine are both quiet non-events', async () => {
  assert.equal((await keepMemoryDigested(undefined, SCOPE)).did, 'settled');
  assert.equal((await keepMemoryDigested(memoryWhoseEngine(null), SCOPE)).did, 'settled');
});

test('the tick actually drives it, without awaiting it', () => {
  const source = readFileSync(new URL('../src/harness/npc.ts', import.meta.url), 'utf8');
  assert.match(
    source,
    /void keepMemoryDigested\(this\.recollection, this\.memory\)/,
    'fired every tick, fire-and-forget: an observe can take a minute and the ticks must not stop'
  );
});
