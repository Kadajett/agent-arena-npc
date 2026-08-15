/**
 * The digest's briefing must sort the cast into the right doors.
 *
 * The chronicle names who is new and who went quiet. An arrival counted as a
 * regular reads as the town ignoring a stranger; a regular counted as an
 * arrival reads as the town forgetting its own. Both are wrong in the way a
 * reader notices instantly, so the mechanical sort is pinned here: arrivals
 * are names the persisted roster has never seen - never inferred from
 * bounded history, which is how a resident returning from four days of
 * silence got announced as a newcomer - the quiet spoke before the window
 * and not in it, and window line-counts are exact.
 */
import assert from 'node:assert/strict';
import test from 'node:test';
import { buildBriefing } from '../dist/world-digest.js';

const hour = 3600_000;
const t0 = Date.UTC(2026, 0, 10, 0, 0);
const at = (h) => new Date(t0 + h * hour).toISOString();
const line = (h, from, message, scene = 'town') => ({ at: at(h), scene, from, message });

const lines = [
  line(-30, 'Oldtimer', 'I was here before the window.'),
  line(-2, 'Regular', 'Before the window.'),
  line(2, 'Regular', 'And inside it.'),
  line(3, 'Newcomer', 'First words ever.'),
  line(4, 'Newcomer', 'Second words.'),
  line(4.5, 'Veteran', 'Back after a long silence.'),
  line(5, null, 'A system line nobody said.')
];
const roster = new Set(['Oldtimer', 'Regular', 'Veteran']);

test('arrivals, regulars, and the quiet each land in their own door', () => {
  const briefing = buildBriefing(lines, t0, t0 + 6 * hour, roster);
  assert.deepEqual(briefing.arrivals, ['Newcomer'], 'a name the roster has never seen');
  assert.deepEqual(
    briefing.spoke,
    [
      { name: 'Newcomer', lines: 2 },
      { name: 'Regular', lines: 1 },
      { name: 'Veteran', lines: 1 }
    ],
    'window counts exact; the nameless line counts for nobody'
  );
  assert.deepEqual(
    briefing.quiet.map((q) => q.name),
    ['Oldtimer'],
    'spoke before the window and not within it; Regular is not quiet'
  );
  assert.match(briefing.transcript, /Newcomer: First words ever/);
  assert.ok(!briefing.transcript.includes('Before the window'), 'transcript is the window only');
});

test('a returning veteran is never announced as a newcomer', () => {
  // The failure this file is named for: Veteran's only line in bounded
  // history falls inside the window, which is exactly what a first-ever
  // line looks like. The roster is what tells them apart.
  const briefing = buildBriefing(lines, t0, t0 + 6 * hour, roster);
  assert.ok(!briefing.arrivals.includes('Veteran'));
});

test('with no roster, everyone active is an arrival - the honest cold start', () => {
  const briefing = buildBriefing(lines, t0, t0 + 6 * hour);
  assert.deepEqual([...briefing.arrivals].sort(), ['Newcomer', 'Regular', 'Veteran']);
});

test('the character talking about themselves is not salience', () => {
  const briefing = buildBriefing(lines, t0, t0 + 6 * hour, roster);
  assert.ok(briefing.spoke.every((s) => s.name !== null));
});
