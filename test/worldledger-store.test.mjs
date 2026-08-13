/**
 * The world ledger's on-disk state: cursor round-tripping and claim
 * merging. Same isolated-temp-dir pattern the other file-backed tests use.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { loadClaims, loadCursor, mergeClaims, saveCursor } from '../dist/worldledger/store.js';

function tmp() {
  return mkdtempSync(join(tmpdir(), 'worldledger-test-'));
}

function claim(overrides) {
  return {
    text: 'a claim',
    tier: 'overheard',
    componentCount: 1,
    lineIds: [1],
    seededBy: null,
    authenticity: 'unexamined',
    contradicts: null,
    ...overrides
  };
}

test('a cursor that was never saved comes back as the honest default', () => {
  const cursor = loadCursor(tmp());
  assert.deepEqual(cursor, { chat: 0, activity: 0, thoughtsByPlayer: {} });
});

test('a saved cursor round-trips exactly', () => {
  const dir = tmp();
  saveCursor(dir, { chat: 42, activity: 17, thoughtsByPlayer: { Bolo: '3:1' } });
  assert.deepEqual(loadCursor(dir), { chat: 42, activity: 17, thoughtsByPlayer: { Bolo: '3:1' } });
});

test('a fresh claim is appended with the given tier and a seen-count of one', () => {
  const dir = tmp();
  const merged = mergeClaims(dir, [claim({ text: 'Barnaby pays suppliers late' })], '2026-01-01T10:00:00Z');
  assert.equal(merged.length, 1);
  assert.equal(merged[0].tier, 'overheard');
  assert.equal(merged[0].timesSeen, 1);
  assert.equal(merged[0].firstSeen, '2026-01-01T10:00:00Z');
});

test('the same claim text reseen later updates lastSeen and increments timesSeen', () => {
  const dir = tmp();
  mergeClaims(dir, [claim({ text: 'Barnaby pays suppliers late', lineIds: [1] })], '2026-01-01T10:00:00Z');
  const merged = mergeClaims(dir, [claim({ text: 'barnaby pays suppliers late', lineIds: [2] })], '2026-01-02T10:00:00Z');
  assert.equal(merged.length, 1, 'matched case-insensitively as the same claim');
  assert.equal(merged[0].timesSeen, 2);
  assert.equal(merged[0].firstSeen, '2026-01-01T10:00:00Z', 'first-seen does not move');
  assert.equal(merged[0].lastSeen, '2026-01-02T10:00:00Z');
});

test('a claim reseen at a stronger tier moves up, never back down', () => {
  const dir = tmp();
  mergeClaims(dir, [claim({ text: 'The east gate is closed', lineIds: [1] })], '2026-01-01T10:00:00Z');
  let merged = mergeClaims(dir, [claim({ text: 'The east gate is closed', tier: 'read', lineIds: [2] })], '2026-01-02T10:00:00Z');
  assert.equal(merged[0].tier, 'read');
  merged = mergeClaims(dir, [claim({ text: 'The east gate is closed', lineIds: [3] })], '2026-01-03T10:00:00Z');
  assert.equal(merged[0].tier, 'read', 'a weaker reseeing does not demote it');
});

test('a claim reseen at a stronger authenticity moves up, never back down', () => {
  const dir = tmp();
  mergeClaims(dir, [claim({ text: 'The count was seven', authenticity: 'stable', lineIds: [1, 2] })], '2026-01-01T10:00:00Z');
  let merged = mergeClaims(
    dir,
    [claim({ text: 'The count was seven', authenticity: 'drifting', lineIds: [3, 4] })],
    '2026-01-02T10:00:00Z'
  );
  assert.equal(merged[0].authenticity, 'drifting', 'evidence of drift is not erased by a calmer earlier reading');
  merged = mergeClaims(dir, [claim({ text: 'The count was seven', authenticity: 'stable', lineIds: [5, 6] })], '2026-01-03T10:00:00Z');
  assert.equal(merged[0].authenticity, 'drifting', 'a later stable reseeing does not un-flag it either');
});

test('a contradiction found once is kept even if a later cycle does not repeat it', () => {
  const dir = tmp();
  mergeClaims(
    dir,
    [claim({ text: 'Barnaby never kept his own books', authenticity: 'contradicted', contradicts: 'Barnaby has always kept his own books.' })],
    '2026-01-01T10:00:00Z'
  );
  const merged = mergeClaims(dir, [claim({ text: 'Barnaby never kept his own books' })], '2026-01-02T10:00:00Z');
  assert.equal(merged[0].contradicts, 'Barnaby has always kept his own books.');
});

test('a pre-existing row with no authenticity field is treated as unexamined, not thrown', () => {
  const dir = tmp();
  // Simulates a row written before authenticity tracking existed - no field
  // at all, not just a null one.
  writeFileSync(
    `${dir}/world-claims.json`,
    JSON.stringify([{ text: 'The east gate is closed', tier: 'overheard', componentCount: 1, timesSeen: 1, firstSeen: '2026-01-01T00:00:00Z', lastSeen: '2026-01-01T00:00:00Z', seededBy: null }])
  );
  const merged = mergeClaims(dir, [claim({ text: 'The east gate is closed', authenticity: 'stable', lineIds: [2, 3] })], '2026-01-02T10:00:00Z');
  assert.equal(merged[0].authenticity, 'stable');
});

test('reading claims that were never written comes back empty, not thrown', () => {
  assert.deepEqual(loadClaims(tmp()), []);
});

test('distinct claim texts stay as distinct rows', () => {
  const dir = tmp();
  const merged = mergeClaims(
    dir,
    [
      claim({ text: 'Barnaby pays suppliers late' }),
      claim({ text: 'The east gate is closed', tier: 'possibly-true', componentCount: 2, lineIds: [2, 3] })
    ],
    '2026-01-01T10:00:00Z'
  );
  assert.equal(merged.length, 2);
});
