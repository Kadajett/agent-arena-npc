/**
 * What a character is built to remember.
 *
 * Both of the things checked here fail silently if they regress, which is why
 * they are worth a test rather than a comment. A history window that is too
 * small does not error, it just quietly gives a character amnesia. And an
 * observational-memory model left at its default points at Google, which this
 * deployment has no key for, so compaction would look configured and do
 * nothing until the day somebody went looking for last week.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';

import { buildMemory } from '../dist/harness/memory.js';
import { FREE_ROUTER } from '../dist/harness/models.js';


/** threadConfig is protected, and this is the only way to read it back. */
const configOf = (memory) => memory.threadConfig ?? memory.getMergedThreadConfig?.() ?? {};

test('raw history is short, because the observation log is what carries the rest', () => {
  const config = configOf(buildMemory('test-character', '/tmp'));
  // Mastra's guidance is that observational memory replaces raw history as it
  // grows, and their default example is twenty. This ran four hundred with
  // compaction also on, which is the log doing its job and being ignored, and
  // it cost $13 a day across eleven characters because the history is in every
  // prompt. Forty is a long conversation in the room; older than that is what
  // the log is for.
  assert.ok(
    config.lastMessages <= 60,
    `${config.lastMessages} raw messages ride on every call, which is what the log is meant to replace`
  );
});

test('compaction runs on the free router, which is the point rather than a slip', () => {
  const config = configOf(buildMemory('test-character', '/tmp'));
  const om = config.observationalMemory;
  assert.ok(om && typeof om === 'object', 'observational memory is configured, not left off');
  // With nothing passed, the default is still the free router and not the
  // shipped google default, which this deployment has no key for. But the
  // default is only a default: pinning everyone to the free tier is how the
  // thousand-a-day account cap killed compaction for the whole cast at once,
  // so production passes the character's own paid model through.
  assert.equal(om.model, FREE_ROUTER, 'the fallback default when nobody chooses');
  const chosen = configOf(buildMemory('test-character', '/tmp', 16, 'openrouter/example/cheap'));
  assert.equal(
    chosen.observationalMemory.model,
    'openrouter/example/cheap',
    'the caller can put observation on a model that is actually expected to answer'
  );
});

test('production wires observation to the character sheet model, not the free tier', async () => {
  // The call site is the part a unit default cannot prove. The free router
  // caps at a thousand requests a day for the whole account; when eleven
  // characters exhausted it by mid-afternoon, observation 429ed until
  // midnight, nothing folded down, and every prompt carried the unobserved
  // backlog at paid prices - forty-thousand-token average prompts, about $11
  // a day, and characters too context-drowned to act on their own goals.
  const { readFile } = await import('node:fs/promises');
  const source = await readFile(new URL('../src/harness/npc.ts', import.meta.url), 'utf8');
  assert.match(
    source,
    /buildMemory\(sheet\.id, MEMORY_DIR, sheet\.recall, sheet\.model\)/,
    'npc.ts must pass the sheet model as the observation model'
  );
});

test('compaction is set to fire inside the window rather than never', () => {
  const config = configOf(buildMemory('test-character', '/tmp'));
  const om = config.observationalMemory;
  // The shipped default is 30,000, which these characters would reach roughly
  // never: four hundred of their messages come to 11,000 to 17,000 tokens.
  // Anything at or above the window size means history falls off the end
  // uncompacted, which is the failure this is guarding.
  assert.ok(
    om.observation.messageTokens < 10_000,
    `observation fires at ${om.observation.messageTokens} tokens, which must be well inside the window`
  );
  assert.ok(om.reflection.observationTokens > 0, 'and observations get reflected down in turn');
});

test('working memory still survives a restart, scoped to the character', () => {
  const config = configOf(buildMemory('test-character', '/tmp'));
  assert.equal(config.workingMemory.enabled, true);
  assert.equal(
    config.workingMemory.scope,
    'resource',
    'resource scope is what makes it outlive a single thread'
  );
});
