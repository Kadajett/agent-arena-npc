/**
 * The digest's briefing must sort the cast into the right doors.
 *
 * The chronicle names who is new and who went quiet. An arrival counted as a
 * regular reads as the town ignoring a stranger; a regular counted as an
 * arrival reads as the town forgetting its own. Both are wrong in the way a
 * reader notices instantly, so the mechanical sort is pinned here: arrivals
 * spoke first inside the window, the quiet spoke before it and not in it,
 * and the character's window line-counts are exact.
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
  line(5, null, 'A system line nobody said.')
];

test('arrivals, regulars, and the quiet each land in their own door', () => {
  const briefing = buildBriefing(lines, t0, t0 + 6 * hour);
  assert.deepEqual(briefing.arrivals, ['Newcomer'], 'first-ever line inside the window');
  assert.deepEqual(
    briefing.spoke,
    [{ name: 'Newcomer', lines: 2 }, { name: 'Regular', lines: 1 }],
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
