/**
 * The trace sees every OpenRouter call, and never the key.
 *
 * The spend ledger meters what the harness sends; it cannot see what Mastra
 * sends on its own behalf, and those calls - observation, reflection - are
 * exactly the ones that failed invisibly for most of a day. The trace wraps
 * fetch underneath everything, so there is no such thing as a call it does
 * not see.
 *
 * The safety property is the same one the spend ledger was built around: the
 * API key travels in the Authorization header and only there, so a trace
 * that logs bodies and never headers cannot leak it however hard it tries.
 * That property is asserted here, not assumed.
 */
import { test, before } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const SECRET = 'sk-or-v1-if-this-string-is-ever-seen-in-a-trace-the-design-failed';
const directory = mkdtempSync(join(tmpdir(), 'trace-test-'));
const seen = [];

before(async () => {
  globalThis.fetch = async (input, init) => {
    seen.push({ input, init });
    return new Response('{"ok":true}', { status: seen.length === 3 ? 429 : 200 });
  };
  process.env.NPC_TRACE = '1';
  const { installTrace } = await import('../dist/harness/trace.js');
  installTrace('test-character', directory);

  const headers = { authorization: `Bearer ${SECRET}` };
  await fetch('https://openrouter.ai/api/v1/chat/completions', {
    method: 'POST',
    headers,
    body: JSON.stringify({ model: 'openrouter/example', messages: [{ role: 'user', content: 'hello' }] })
  });
  await fetch('https://example.com/not-openrouter', { method: 'POST', headers, body: 'ignored' });
  await fetch('https://openrouter.ai/api/v1/chat/completions', {
    method: 'POST',
    headers,
    body: JSON.stringify({
      model: 'openrouter/example',
      messages: [{ role: 'system', content: 'The following observations block contains your memory' }]
    })
  });
});

const lines = () =>
  readFileSync(join(directory, 'trace', 'test-character.jsonl'), 'utf8')
    .trim().split('\n').map((line) => JSON.parse(line));

test('every OpenRouter call lands in the trace, and only OpenRouter calls', () => {
  assert.equal(seen.length, 3, 'all three fetches reached the real fetch underneath');
  assert.equal(lines().length, 2, 'and the non-OpenRouter one was not recorded');
});

test('the body is on the record, the key is nowhere in the file', () => {
  const raw = readFileSync(join(directory, 'trace', 'test-character.jsonl'), 'utf8');
  assert.match(raw, /hello/, 'the prompt itself is recorded');
  assert.ok(!raw.includes(SECRET), 'the Authorization header never reaches the file');
  assert.ok(!raw.toLowerCase().includes('bearer'), 'no header material of any kind');
});

test('an observation call is named as one, so the question of the day is answerable', () => {
  const [, observation] = lines();
  assert.equal(observation.purpose, 'observation');
  assert.equal(observation.status, 429, 'and its failure status is right there');
});

test('production installs the trace before the character exists', () => {
  const source = readFileSync(new URL('../src/index.ts', import.meta.url), 'utf8');
  assert.match(source, /installTrace\(sheet\.id/, 'index.ts must install it');
  const position = source.indexOf('installTrace');
  assert.ok(position < source.indexOf('new Npc(sheet).run()'), 'and before the run starts');
});
