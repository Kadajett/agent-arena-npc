/**
 * A character's own record of what it has been told, and how it arrived.
 *
 * classifyTier() is the one piece of judgment in the whole file, and it is
 * deliberately not judgment at all: it is a pattern match on the claim's own
 * wording, the same for a claim heard or said. noteClaim()/readClaims() are
 * the file-backed round trip everything else in the ledger depends on.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { classifyTier, noteClaim, readClaims } from '../dist/harness/ledger.js';

test('a flat statement is told, not overheard', () => {
  assert.equal(classifyTier("Barnaby's books don't add up."), 'told');
});

test('hearsay language reads as overheard, whoever says it', () => {
  assert.equal(classifyTier("I heard somebody say the books don't add up."), 'overheard');
  assert.equal(classifyTier('Word is he pays late.'), 'overheard');
  assert.equal(classifyTier('Rumour has it the cellar is short.'), 'overheard');
  assert.equal(classifyTier('I wonder if he keeps two sets of books.'), 'overheard');
});

test('the same underlying claim tiers differently depending on how it was said', () => {
  assert.equal(classifyTier('He pays his suppliers late.'), 'told');
  assert.equal(classifyTier('Apparently he pays his suppliers late.'), 'overheard');
});

test('a claim written to the ledger comes back with its tier attached', () => {
  const dir = mkdtempSync(join(tmpdir(), 'ledger-test-'));
  noteClaim(dir, 'bolo', {
    claim: 'Somebody said the inn is short on coin.',
    direction: 'said',
    speaker: 'Bolo',
    room: 'reldens-house-1',
    at: '2026-01-01T00:00:00.000Z'
  });
  const claims = readClaims(dir, 'bolo');
  assert.equal(claims.length, 1);
  assert.equal(claims[0].tier, 'overheard');
  assert.equal(claims[0].direction, 'said');
  assert.equal(claims[0].speaker, 'Bolo');
  assert.equal(claims[0].room, 'reldens-house-1');
});

test('claims accumulate across calls and persist between reads', () => {
  const dir = mkdtempSync(join(tmpdir(), 'ledger-test-'));
  noteClaim(dir, 'bolo', {
    claim: 'Barnaby keeps his own books.',
    direction: 'heard',
    speaker: 'Aveline',
    room: 'reldens-house-1',
    at: '2026-01-01T00:00:00.000Z'
  });
  noteClaim(dir, 'bolo', {
    claim: 'I heard he pays late.',
    direction: 'heard',
    speaker: null,
    room: 'reldens-town',
    at: '2026-01-01T00:05:00.000Z'
  });
  const claims = readClaims(dir, 'bolo');
  assert.equal(claims.length, 2);
  assert.equal(claims[0].tier, 'told');
  assert.equal(claims[1].tier, 'overheard');
  assert.equal(claims[1].speaker, null);
});

test('an unattributed speaker is recorded as told when the wording is flat', () => {
  const dir = mkdtempSync(join(tmpdir(), 'ledger-test-'));
  noteClaim(dir, 'bolo', {
    claim: 'The east gate is closed for repairs.',
    direction: 'heard',
    speaker: null,
    room: 'reldens-town',
    at: '2026-01-01T00:00:00.000Z'
  });
  assert.equal(readClaims(dir, 'bolo')[0].tier, 'told');
});

test('an empty or blank claim is not written at all', () => {
  const dir = mkdtempSync(join(tmpdir(), 'ledger-test-'));
  noteClaim(dir, 'bolo', { claim: '   ', direction: 'said', speaker: 'Bolo', room: 'reldens-town', at: '2026-01-01T00:00:00.000Z' });
  assert.equal(readClaims(dir, 'bolo').length, 0);
});

test('two characters keep separate ledgers in the same memory directory', () => {
  const dir = mkdtempSync(join(tmpdir(), 'ledger-test-'));
  noteClaim(dir, 'bolo', { claim: 'Bolo said this.', direction: 'said', speaker: 'Bolo', room: 'reldens-town', at: '2026-01-01T00:00:00.000Z' });
  noteClaim(dir, 'sanejack', { claim: 'Jack said this.', direction: 'said', speaker: 'SaneJack', room: 'reldens-town', at: '2026-01-01T00:00:00.000Z' });
  assert.equal(readClaims(dir, 'bolo').length, 1);
  assert.equal(readClaims(dir, 'sanejack').length, 1);
  assert.equal(readClaims(dir, 'bolo')[0].claim, 'Bolo said this.');
});

test('reading a ledger that was never written comes back empty, not thrown', () => {
  const dir = mkdtempSync(join(tmpdir(), 'ledger-test-'));
  assert.deepEqual(readClaims(dir, 'nobody'), []);
});

test('the file on disk is a plain JSON array a human could open', () => {
  const dir = mkdtempSync(join(tmpdir(), 'ledger-test-'));
  noteClaim(dir, 'bolo', { claim: 'Something happened.', direction: 'said', speaker: 'Bolo', room: 'reldens-town', at: '2026-01-01T00:00:00.000Z' });
  const raw = JSON.parse(readFileSync(join(dir, 'bolo-claims.json'), 'utf8'));
  assert.ok(Array.isArray(raw));
  assert.equal(raw[0].claim, 'Something happened.');
});
