/**
 * What a character knows on walking through a door.
 *
 * The alternative, and what this world was doing, is carrying every scrap of
 * context in the rolling history so it happens to be there when needed. An
 * average call was reading thirty thousand tokens to have this much on hand.
 *
 * Walking through a door is rare. Standing in a room is constant. Putting the
 * context on the rare thing is most of the saving available here, and it is
 * better context besides: a character re-reading two hundred messages has to
 * work out for itself which of them happened in this room, and this hands it
 * that directly, with how long it has been.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('../src/harness/npc.ts', import.meta.url), 'utf8');

test('the card is built on arrival, not on every tick', () => {
  // arrivedSomewhere returns null when the scene has not changed, which is the
  // whole economy of this: a character standing still pays nothing for it.
  assert.match(source, /if \(scene === this\.standingIn\) \{\s*\n\s*return null;/);
});

test('a room never seen before says so plainly rather than inventing a history', () => {
  assert.match(source, /You have not been here before/);
});

test('coming back carries how long it has been and what was said here', () => {
  assert.match(source, /You are back in \$\{name\}, \$\{howLongSince\(known\.lastHere\)\}/);
  assert.match(source, /Last time you were here:/);
  assert.match(source, /Nothing was said here last time/, 'and is honest when there is nothing');
});

test('how long ago reads as English, not as a timestamp', () => {
  assert.match(source, /a moment ago/);
  assert.match(source, /minute\$\{1 === minutes \? '' : 's'\} ago/);
  assert.match(source, /hour\$\{1 === hours \? '' : 's'\} ago/);
});

test('what was said is kept per room, not in one pile', () => {
  assert.match(source, /private heardHere\(scene: string, from: string, message: string\)/);
  assert.match(source, /this\.heardHere\(scene, line\.from, line\.message\)/);
});

test('a room does not accumulate conversation forever', () => {
  // Unbounded would recreate the problem this exists to solve, one room at a time.
  assert.match(source, /room\.said\.splice\(0, room\.said\.length - ROOM_LINES \* 2\)/);
});

test('nobody thinks fifteen times a minute mid-conversation any more', () => {
  const sheets = readFileSync(new URL('../src/characters/guy.ts', import.meta.url), 'utf8');
  const engaged = Number(/engaged: (\d+)/.exec(sheets)?.[1]);
  assert.ok(engaged >= 8, `engaged pace of ${engaged}s is a model call every ${engaged} seconds while talking`);
});

/**
 * A redeploy used to tell everybody they had never been anywhere.
 *
 * Guy was informed he had walked into town for the first time, having lived
 * there for days. That is worse than saying nothing, because it is a confident
 * falsehood, and the door labels are built off the same record: every door
 * would have read "never been" straight after a deploy, which is exactly the
 * signal that was added to stop him picking the nearest door forever.
 */
test('rooms it has stood in before today are remembered across a restart', () => {
  assert.match(source, /private async rememberWhereItHasBeen\(\)/);
  assert.match(source, /place\.how !== 'been'/, 'only places it actually went, not ones it was told about');
  assert.match(source, /await this\.rememberWhereItHasBeen\(\);/);
});

test('a remembered room says so without inventing a time it does not have', () => {
  // Memory keeps what a place is, not when it was last seen. "Before" is
  // honest; "about 4 minutes ago" would not be.
  assert.match(source, /which you have been in before/);
  assert.match(source, /0 === known\.lastHere/);
});

test('the card is in front of the character on the tick it is true, not the next one', () => {
  // Added after the action it arrived a tick late, telling a character it had
  // just walked into the room it had by then already left.
  const build = source.indexOf('const situation: Situation');
  const card = source.indexOf('this.notes = [...this.notes, arrived]');
  assert.ok(card > 0 && card < build, 'the card must be set before the situation is built');
});
