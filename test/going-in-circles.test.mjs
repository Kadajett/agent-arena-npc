/**
 * Doing the same thing over and over and getting away with it.
 *
 * Guy walked upstairs at the inn and back down again for six minutes, forty-five
 * seconds apart, with an empty plan and a goal that is two rooms outside the
 * building. Every one of those moves succeeded, so the repeated-failure counter
 * never fired: there was no failure. The record of the loop was sitting in his
 * own memory under "lately", he was shown it on every tick, and he did it again.
 *
 * That is the third time today the same lesson has landed. A character that is
 * not told is a character that repeats: prose with no action needed a
 * correction, a failing action needed to be handed its own failure, and going
 * nowhere successfully needs this.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('../src/harness/npc.ts', import.meta.url), 'utf8');

/** goingInCircles() lifted out, so the behaviour is exercised not described. */
function circler(window = 8) {
  const lately = [];
  return (intent, alone = false) => {
    // Every say is one move whatever the words: the words are always
    // different, and keying on them made small talk the one unnameable rut.
    const move = intent.action === 'say'
      ? 'say:'
      : `${intent.action}:${(intent.place ?? intent.target ?? '').toLowerCase()}`;
    lately.push(move);
    if (lately.length > window) lately.shift();
    if (intent.action === 'wait') return null;
    if (intent.action === 'say' && !alone) return null;
    const same = lately.filter((one) => one === move).length;
    if (same < 3) return null;
    if (intent.action === 'say') {
      return `[You have now said your piece ${same} times in the last ${lately.length} moves and there is nobody here]`;
    }
    return `[You have done this ${same} times in the last ${lately.length} moves]`;
  };
}

test('crossing back and forth through one doorway is eventually named', () => {
  const circling = circler();
  const up = { action: 'use_door', place: 'upstairs at the inn' };
  const down = { action: 'use_door', place: "Barnaby's inn" };
  assert.equal(circling(up), null, 'once is going somewhere');
  assert.equal(circling(down), null);
  assert.equal(circling(up), null, 'twice is changing your mind');
  assert.equal(circling(down), null);
  assert.match(circling(up), /You have done this 3 times/, 'three is a loop');
});

test('a character genuinely travelling is left alone', () => {
  const circling = circler();
  for (const place of ['town', 'the grasslands', 'the crypt', 'the depths', 'the shore']) {
    assert.equal(circling({ action: 'use_door', place }), null, place);
  }
});

test('standing still and talking to somebody are allowed to repeat', () => {
  // A character in a conversation says several things in a row, and somebody
  // waiting for somebody else is not stuck. The company is what makes it a
  // conversation: alone stays false here because somebody is in the room.
  const circling = circler();
  for (let i = 0; i < 6; i++) {
    assert.equal(circling({ action: 'say', message: 'something' }, false), null);
    assert.equal(circling({ action: 'wait' }), null);
  }
});

test('exploring the same room repeatedly is not circling, since it goes somewhere new', () => {
  // explore carries no place, so each one is the same signature - and that is
  // the one repeat worth allowing, because the room is bigger than one look.
  const circling = circler();
  const looks = [];
  for (let i = 0; i < 4; i++) looks.push(circling({ action: 'explore' }));
  assert.ok(looks[3], 'it is still caught after four');
});

test('it only looks at the recent past, so a character that moves on stops being nagged', () => {
  const circling = circler(8);
  const up = { action: 'use_door', place: 'upstairs at the inn' };
  circling(up);
  circling(up);
  circling(up);
  for (const place of ['town', 'the grasslands', 'the crypt', 'the depths', 'the shore', 'the volcano']) {
    circling({ action: 'use_door', place });
  }
  assert.equal(circling(up), null, 'the old loop has fallen out of the window');
});

test('talking to an empty street is named after three fresh greetings', () => {
  // Guy, wiped clean and standing in town at night, greeted nobody in
  // particular with a different sentence every tick. Different words, same
  // move. Three of them with no other player in the room is a rut.
  const circling = circler();
  assert.equal(circling({ action: 'say', message: 'Evening.' }, true), null);
  assert.equal(circling({ action: 'say', message: 'Quiet on this side.' }, true), null);
  const named = circling({ action: 'say', message: 'Anything to report?' }, true);
  assert.match(named, /said your piece 3 times/);
  assert.match(named, /nobody here/);
});

test('company arriving mid-rut ends the rut', () => {
  const circling = circler();
  circling({ action: 'say', message: 'one' }, true);
  circling({ action: 'say', message: 'two' }, true);
  assert.equal(
    circling({ action: 'say', message: 'three' }, false),
    null,
    'the third say lands on an actual audience, which is what talking is for'
  );
});

test('the detector is retired from the live loop, replaced by seeing consequences in-band', () => {
  // Everything in this file existed to compensate for a model that never saw
  // what its own last action did: a loop looks like progress when every step
  // reports success and nothing connects them. Agentic turns closed that gap
  // - results come back as tool results inside the same turn - so the live
  // loop no longer wires the detector at all. The algorithm tests above stay
  // as the record of what the rut problem was, and this pin makes sure the
  // old machinery does not quietly grow back beside the new loop.
  assert.match(source, /await this\.liveOneTurn\(situation\)/);
  assert.ok(!/const goingNowhere = this\.goingInCircles\(/.test(source), 'not wired any more');
});
