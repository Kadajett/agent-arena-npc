/**
 * A character's own record of what it has been told, and how it arrived.
 *
 * Not memory in the Mastra sense - that is the model's own running account of
 * the world, prose it can misremember or drift on a compaction. This is a
 * small, mechanical, append-only log kept beside it: one line per thing said
 * or heard, tagged with how it arrived, written with no model call at all.
 * The point is not to judge what is true. It is to keep the raw material -
 * who said what, whether they said it as fact or relayed it as something they
 * only heard themselves - somewhere a character (or, later, something reading
 * across every character in the world) can actually consult it, instead of
 * every doubt being reconstructed from scratch out of conversational memory
 * every time.
 *
 * `tier` is classified from the claim's own wording, the one signal available
 * without asking a model to judge anything: the same line lands as `overheard`
 * whether the character said it or heard it, because the tell is in how it was
 * phrased, not who was speaking. Bolo's whole technique - "somebody said",
 * "I heard", a half-loaded question - reads as hearsay here even when he is
 * the one saying it, which is the point: this is a record of how a claim was
 * carried, not a verdict on whether it is so.
 *
 * `read` exists in the type as a placeholder for a claim traced to an actual
 * fixed object in the world rather than to anybody's mouth - the strongest
 * tier there is - but nothing here produces it yet. There is no reliable
 * signal today for telling a read object apart from a person in conversation;
 * inventing one rather than admitting the gap would make every claim in this
 * file look more solid than it is.
 */

import { readFileSync, writeFileSync } from 'node:fs';

function log(...parts: unknown[]): void {
  console.log(new Date().toISOString().slice(11, 19), ...parts);
}

export type ClaimTier = 'told' | 'overheard' | 'read';
export type ClaimDirection = 'heard' | 'said';

export type ClaimEntry = {
  claim: string;
  tier: ClaimTier;
  direction: ClaimDirection;
  /** Who said it - the character itself for `said`, the other party for `heard`, or null when unattributed. */
  speaker: string | null;
  room: string;
  at: string;
};

/** Per character. Old entries fall off the front; a rumour that never came back is not worth keeping forever. */
const MAX_CLAIMS = 500;

/**
 * Phrases a speaker reaches for when relaying something rather than
 * asserting it. Matched against the claim's own text - the same line said as
 * hearsay by one person and as flat fact by another is two different tiers,
 * not one.
 */
const HEDGE_PATTERNS = [
  /\bsome(?:body|one)\s+(?:said|told|mentioned|reckons?)\b/i,
  /\bi\s+heard\b/i,
  /\bpeople\s+(?:are\s+|were\s+)?say(?:ing)?\b/i,
  /\bfolks?\s+(?:are\s+|were\s+)?say(?:ing)?\b/i,
  /\bthey\s+say\b/i,
  /\bword\s+is\b/i,
  /\brumou?r\s+has\s+it\b/i,
  /\bapparently\b/i,
  /\bsupposedly\b/i,
  /\ballegedly\b/i,
  /\bmight\s+be\s+nothing\b/i,
  /\bcould\s+be\s+nothing\b/i,
  /\bi\s+wonder(?:ed)?\s+if\b/i,
  /\bwondered\s+whether\b/i
];

/** Told as flat fact, or relayed as something the speaker only heard themselves. */
export function classifyTier(message: string): ClaimTier {
  return HEDGE_PATTERNS.some((pattern) => pattern.test(message)) ? 'overheard' : 'told';
}

function claimsFile(memoryDir: string, characterId: string): string {
  return `${memoryDir}/${characterId}-claims.json`;
}

function loadClaims(memoryDir: string, characterId: string): ClaimEntry[] {
  try {
    const stored = JSON.parse(readFileSync(claimsFile(memoryDir, characterId), 'utf8'));
    return Array.isArray(stored) ? stored : [];
  } catch {
    return [];
  }
}

/**
 * Add one line to a character's own claim ledger. Mechanical and best-effort:
 * a character's turn should never fail because its own bookkeeping could not
 * be written, so this only ever logs on failure rather than throwing.
 */
export function noteClaim(
  memoryDir: string,
  characterId: string,
  entry: { claim: string; direction: ClaimDirection; speaker: string | null; room: string; at: string }
): void {
  const claim = entry.claim.trim();
  if (!claim) {
    return;
  }
  try {
    const claims = loadClaims(memoryDir, characterId);
    claims.push({ ...entry, claim, tier: classifyTier(claim) });
    if (claims.length > MAX_CLAIMS) {
      claims.splice(0, claims.length - MAX_CLAIMS);
    }
    writeFileSync(claimsFile(memoryDir, characterId), JSON.stringify(claims));
  } catch (error) {
    log('could not save claim:', (error as Error)?.message ?? error);
  }
}

/** Everything a character's ledger holds today, oldest first. */
export function readClaims(memoryDir: string, characterId: string): ClaimEntry[] {
  return loadClaims(memoryDir, characterId);
}
