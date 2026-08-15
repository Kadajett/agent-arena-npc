/**
 * What the town has said to and about one character, grouped by who said it.
 *
 * Mechanical extraction only: judgment about warmth or trust belongs to
 * whoever reads the output (see .claude/skills/scorecard). Reads the claims
 * ledger a character's harness already keeps, so it can only report what the
 * character was present to hear - expressed feeling, not private feeling.
 *
 * Usage: node scripts/relations.mjs <claims-file.json> <CharacterName>
 * Output: JSON on stdout - { target, heardTotal, observers: [{ speaker,
 * utterances, mentionsOfTarget, lines: [{ at, room, claim }] }] }
 */
import { readFileSync } from 'node:fs';

const [claimsPath, target] = process.argv.slice(2);
if (!claimsPath || !target) {
  console.error('Usage: node scripts/relations.mjs <claims-file.json> <CharacterName>');
  process.exit(1);
}

const raw = JSON.parse(readFileSync(claimsPath, 'utf8'));
const claims = Array.isArray(raw) ? raw : raw.claims ?? [];
const heard = claims.filter((entry) => entry?.direction === 'heard' && entry?.speaker);

const bySpeaker = new Map();
for (const entry of heard) {
  const speaker = String(entry.speaker);
  if (speaker === target) {
    continue;
  }
  const bucket = bySpeaker.get(speaker) ?? { speaker, utterances: 0, mentionsOfTarget: 0, lines: [] };
  bucket.utterances += 1;
  if (String(entry.claim ?? '').toLowerCase().includes(target.toLowerCase())) {
    bucket.mentionsOfTarget += 1;
  }
  bucket.lines.push({ at: entry.at ?? null, room: entry.room ?? null, claim: entry.claim ?? '' });
  bySpeaker.set(speaker, bucket);
}

const observers = [...bySpeaker.values()].sort((a, b) => b.utterances - a.utterances);
console.log(JSON.stringify({ target, heardTotal: heard.length, observers }, null, 2));
