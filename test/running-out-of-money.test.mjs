/**
 * This world has gone quiet before because an account emptied.
 *
 * It is the worst way for it to fail. Not one character behaving oddly, which
 * anybody would notice: a town where nobody moves and nothing looks broken,
 * with the provider's refusal the only sign, buried in eleven containers' logs.
 *
 * So no character names one model any more. It names the one it would rather
 * think with and the free router sits underneath, and Mastra walks the list.
 * A character whose account has run dry keeps talking, more cheaply and
 * probably less well, which beats standing still by a distance.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { readdirSync } from 'node:fs';

import { FREE_ROUTER, SECOND_CHOICE, withFallback } from '../dist/harness/models.js';

async function wholeCast() {
  const names = readdirSync(new URL('../dist/characters/', import.meta.url))
    .filter((file) => file.endsWith('.js'))
    .map((file) => file.replace(/\.js$/, ''));
  const sheets = [];
  for (const name of names) {
    const loaded = await import(`../dist/characters/${name}.js`);
    sheets.push(...Object.values(loaded).filter((value) => value?.playerName));
  }
  return sheets;
}

test('a paid character falls to a second paid model before the free floor', () => {
  // Falling straight to the free router sounds fine until you remember it is
  // capped at 1,000 requests a day for the whole account, which is its normal
  // state. Then a provider 429 lasting seconds - a hiccup, not an outage -
  // drops a character onto something that cannot answer at all and it stands
  // there. That happened to Hollis, Cutter and Ash.
  const list = withFallback('openrouter/inclusionai/ling-2.6-flash');
  assert.equal(list.length, 3);
  assert.equal(list[0].model, 'openrouter/inclusionai/ling-2.6-flash', 'preferred first');
  assert.equal(list[1].model, SECOND_CHOICE, 'then something else that can actually answer');
  assert.equal(list[2].model, FREE_ROUTER, 'and the floor last');
});

test('the second choice does not fall back to itself either', () => {
  const list = withFallback(SECOND_CHOICE);
  assert.deepEqual(list.map((one) => one.model), [SECOND_CHOICE, FREE_ROUTER]);
});

test('a character already on the free router does not fall back to itself', () => {
  // Falling back from free to free would only double the wait before giving up.
  const list = withFallback(FREE_ROUTER);
  assert.equal(list.length, 1);
  assert.equal(list[0].model, FREE_ROUTER);
});

test('an empty model still leaves somewhere to think', () => {
  const list = withFallback('');
  assert.equal(list.length, 1);
  assert.equal(list[0].model, FREE_ROUTER);
});

test('it retries the preferred model before giving up on it', () => {
  // One try is too few: a single blip would move a character onto the free
  // router for no good reason. Many is too many, because each retry is time
  // spent not answering somebody.
  const list = withFallback('openrouter/deepseek/deepseek-v4-flash');
  assert.ok(list[0].maxRetries >= 2, 'a blip should not be enough to demote it');
  assert.ok(list[0].maxRetries <= 3, 'and a character should not go quiet while it retries');
});

test('every character in the world goes through the fallback, none named directly', async () => {
  const source = readFileSync(new URL('../src/harness/npc.ts', import.meta.url), 'utf8');
  assert.match(source, /model: withFallback\(sheet\.model\)/);
  assert.ok(
    !/model: sheet\.model[,\s]/.test(source),
    'a character wired straight to one model is one that stops when the money does'
  );
  // And every sheet still names something real for it to prefer.
  for (const sheet of await wholeCast()) {
    assert.ok(sheet.model && sheet.model.startsWith('openrouter/'), `${sheet.playerName} has no model`);
  }
});
