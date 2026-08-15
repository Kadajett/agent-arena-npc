/**
 * A mention must land on the right side of "were they in the room".
 *
 * Salience exists to show an owner how the town speaks about their character
 * when the character is not around. Misclassifying presence poisons that in
 * both directions: an in-presence compliment counted as behind-the-back
 * flattery, or a behind-the-back dig hidden because the character spoke in
 * that scene an hour earlier. This pins the presence window on both sides,
 * and pins that the character's own lines never count as mentions.
 */
import assert from 'node:assert/strict';
import test from 'node:test';
import { classifySalience } from '../scripts/salience.mjs';

const at = (minutes) => new Date(Date.UTC(2026, 0, 1, 12, minutes)).toISOString();

const lines = [
  { from: 'Bram', scene: 'inn', at: at(0), message: 'Evening, all.' },
  { from: 'Ada', scene: 'inn', at: at(5), message: 'Bram buys a round again, of course.' },
  { from: 'Ada', scene: 'inn', at: at(30), message: 'Bram always leaves before the bill.' },
  { from: 'Cole', scene: 'town', at: at(6), message: 'Bram asks too many questions.' },
  { from: 'Cole', scene: 'town', at: at(7), message: 'The rain is back.' },
  { from: 'Bram', scene: 'inn', at: at(8), message: 'I am the Bram they warn you about.' }
];

test('mentions land on the right side of the presence window', () => {
  const result = classifySalience(lines, 'Bram', 10 * 60 * 1000);
  assert.equal(result.mentions, 3);
  assert.equal(result.behindBack, 2, 'the late inn line and the town line; not the in-presence one');
  const ada = result.bySpeaker.find((s) => s.speaker === 'Ada');
  assert.deepEqual(
    ada.lines.map((l) => l.targetPresent),
    [true, false],
    'same speaker, same scene: presence decided per mention, not per speaker'
  );
  const cole = result.bySpeaker.find((s) => s.speaker === 'Cole');
  assert.equal(cole.behindBack, 1, 'Bram never spoke in town; Cole spoke behind his back');
  assert.equal(cole.mentions, 1, 'the rain line mentions nobody');
});

test('the character talking about themselves is not salience', () => {
  const result = classifySalience(lines, 'Bram');
  assert.ok(result.bySpeaker.every((s) => s.speaker !== 'Bram'));
  assert.equal(result.scanned, 6);
});

test('a mention at the window edge is judged against presence just outside it', () => {
  // The character spoke 3 minutes before the reporting cutoff; the mention
  // lands 5 minutes after it. Filtering the presence line away first would
  // call this behind-their-back - the misclassification this file exists
  // to prevent. Presence reads all lines; the cutoff drops mentions only.
  const cutoff = Date.parse(at(10));
  const edge = [
    { from: 'Bram', scene: 'inn', at: at(7), message: 'Still here.' },
    { from: 'Ada', scene: 'inn', at: at(15), message: 'Bram never misses a night.' },
    { from: 'Ada', scene: 'inn', at: at(5), message: 'Bram is early today.' }
  ];
  const result = classifySalience(edge, 'Bram', 10 * 60 * 1000, cutoff);
  assert.equal(result.mentions, 1, 'the pre-cutoff mention is dropped from the report');
  assert.equal(result.behindBack, 0, 'but presence just before the cutoff still counts');
  assert.equal(result.scanned, 1);
});

test('short names only match as whole words', () => {
  const result = classifySalience(
    [{ from: 'Cole', scene: 'inn', at: at(0), message: 'These brambles are thick.' }],
    'Bram'
  );
  assert.equal(result.mentions, 0);
});

test('the output states its presence heuristic in words', () => {
  const result = classifySalience([], 'Bram');
  assert.match(result.presenceHeuristic, /spoke in that scene within/);
});
