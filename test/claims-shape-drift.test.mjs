/**
 * The scorecard extractor must notice when the claims ledger changes shape.
 *
 * relations.mjs trusts the fields the harness ledger writes (direction,
 * speaker, claim, at, room). If the ledger writer renames one, the extractor
 * would not crash - it would quietly report that nobody ever spoke to the
 * character, and a scorecard built on that reads as a town full of silence.
 * This pins the exact output for a known ledger, so a shape drift on either
 * side fails loudly instead.
 */
import assert from 'node:assert/strict';
import test from 'node:test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const fixture = [
  { claim: 'Pull up a stool, Ada.', direction: 'said', speaker: 'Bram', room: 'town', at: 't1' },
  { claim: 'Bram, you remembered my name.', direction: 'heard', speaker: 'Ada', room: 'town', at: 't2' },
  { claim: 'Anyone seen Bram tonight?', direction: 'heard', speaker: 'Cole', room: 'inn', at: 't3' },
  { claim: 'The rain is back.', direction: 'heard', speaker: 'Cole', room: 'inn', at: 't4' },
  { claim: 'I am talking to myself again.', direction: 'heard', speaker: 'Bram', room: 'inn', at: 't5' }
];

test('a known ledger produces exactly the expected scorecard input', () => {
  const dir = mkdtempSync(join(tmpdir(), 'claims-'));
  const file = join(dir, 'bram-claims.json');
  writeFileSync(file, JSON.stringify(fixture));
  const output = JSON.parse(
    execFileSync(process.execPath, ['scripts/relations.mjs', file, 'Bram'], { encoding: 'utf8' })
  );
  assert.deepEqual(output, {
    target: 'Bram',
    heardTotal: 4,
    observers: [
      {
        speaker: 'Cole',
        utterances: 2,
        mentionsOfTarget: 1,
        lines: [
          { at: 't3', room: 'inn', claim: 'Anyone seen Bram tonight?' },
          { at: 't4', room: 'inn', claim: 'The rain is back.' }
        ]
      },
      {
        speaker: 'Ada',
        utterances: 1,
        mentionsOfTarget: 1,
        lines: [{ at: 't2', room: 'town', claim: 'Bram, you remembered my name.' }]
      }
    ]
  }, 'the target’s own lines stay out, said-direction lines stay out, and counts are exact');
});
