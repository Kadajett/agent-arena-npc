/**
 * What survives between aggregator runs: where each feed left off, and the
 * claims accumulated so far. Same sync-file convention as discord.ts's
 * episode counter and ledger.ts's per-character claims - small JSON on the
 * volume, read-modify-write, best-effort.
 */

import { readFileSync, writeFileSync } from 'node:fs';
import type { Authenticity, ClassifiedClaim, Tier } from './graph.js';

function log(...parts: unknown[]): void {
  console.log(new Date().toISOString().slice(11, 19), ...parts);
}

export type Cursor = {
  /** Shared id space: one number correctly filters chat and activity both. */
  chat: number;
  activity: number;
  /** Thoughts carries a compound, opaque cursor string, kept per character. */
  thoughtsByPlayer: Record<string, string>;
};

const DEFAULT_CURSOR: Cursor = { chat: 0, activity: 0, thoughtsByPlayer: {} };

function cursorFile(dir: string): string {
  return `${dir}/world-cursor.json`;
}

function claimsFile(dir: string): string {
  return `${dir}/world-claims.json`;
}

export function loadCursor(dir: string): Cursor {
  try {
    const stored = JSON.parse(readFileSync(cursorFile(dir), 'utf8'));
    return {
      chat: Number.isFinite(stored?.chat) ? stored.chat : 0,
      activity: Number.isFinite(stored?.activity) ? stored.activity : 0,
      thoughtsByPlayer: 'object' === typeof stored?.thoughtsByPlayer && stored.thoughtsByPlayer ? stored.thoughtsByPlayer : {}
    };
  } catch {
    return { ...DEFAULT_CURSOR, thoughtsByPlayer: {} };
  }
}

export function saveCursor(dir: string, cursor: Cursor): void {
  try {
    writeFileSync(cursorFile(dir), JSON.stringify(cursor));
  } catch (error) {
    log('could not save world-ledger cursor:', (error as Error)?.message ?? error);
  }
}

export type StoredClaim = {
  text: string;
  tier: Tier;
  componentCount: number;
  timesSeen: number;
  firstSeen: string;
  lastSeen: string;
  seededBy: { player: string; note: string } | null;
  authenticity: Authenticity;
  contradicts: string | null;
};

/** Higher is more confident. Merging a claim only ever moves it up this order, never back down. */
const TIER_RANK: Record<Tier, number> = {
  unknown: 0,
  overheard: 1,
  'probably-seeded': 1,
  'possibly-true': 2,
  read: 3
};

/**
 * Higher is more informative or more concerning, and merging only ever
 * moves toward it, same reasoning as tier: evidence that a claim once
 * drifted or once contradicted something trusted is a real fact about its
 * history, and a calmer later cycle should not erase it.
 */
const AUTHENTICITY_RANK: Record<Authenticity, number> = {
  unexamined: 0,
  stable: 1,
  drifting: 2,
  contradicted: 3
};

const MAX_STORED_CLAIMS = 1000;

function normalize(text: string): string {
  return text.trim().toLowerCase().replace(/\s+/g, ' ');
}

export function loadClaims(dir: string): StoredClaim[] {
  try {
    const stored = JSON.parse(readFileSync(claimsFile(dir), 'utf8'));
    return Array.isArray(stored) ? stored : [];
  } catch {
    return [];
  }
}

/**
 * Fold this cycle's classified claims into what is already stored. A claim
 * whose normalized text already exists is updated in place - its tier only
 * moves toward more confident, never back down, because losing corroboration
 * you already saw is not the same as it never having happened. Anything new
 * is appended. Best-effort: a write failure here should not stop the run.
 */
export function mergeClaims(dir: string, fresh: ClassifiedClaim[], generatedAt: string): StoredClaim[] {
  const existing = loadClaims(dir);
  const byText = new Map(existing.map((claim) => [normalize(claim.text), claim]));
  for (const claim of fresh) {
    const key = normalize(claim.text);
    const already = byText.get(key);
    if (already) {
      // Pre-existing rows written before authenticity tracking existed have
      // no field to rank against; treat that gap as the neutral starting
      // point rather than crashing the comparison.
      const knownAuthenticity = already.authenticity ?? 'unexamined';
      already.tier = TIER_RANK[claim.tier] > TIER_RANK[already.tier] ? claim.tier : already.tier;
      already.authenticity =
        AUTHENTICITY_RANK[claim.authenticity] > AUTHENTICITY_RANK[knownAuthenticity]
          ? claim.authenticity
          : knownAuthenticity;
      already.contradicts = already.contradicts ?? claim.contradicts;
      already.componentCount = Math.max(already.componentCount, claim.componentCount);
      already.timesSeen += 1;
      already.lastSeen = generatedAt;
      already.seededBy = already.seededBy ?? claim.seededBy;
    } else {
      byText.set(key, {
        text: claim.text,
        tier: claim.tier,
        componentCount: claim.componentCount,
        timesSeen: 1,
        firstSeen: generatedAt,
        lastSeen: generatedAt,
        seededBy: claim.seededBy,
        authenticity: claim.authenticity,
        contradicts: claim.contradicts
      });
    }
  }
  let merged = [...byText.values()];
  if (merged.length > MAX_STORED_CLAIMS) {
    merged = merged.sort((left, right) => left.lastSeen.localeCompare(right.lastSeen)).slice(-MAX_STORED_CLAIMS);
  }
  try {
    writeFileSync(claimsFile(dir), JSON.stringify(merged));
  } catch (error) {
    log('could not save world claims:', (error as Error)?.message ?? error);
  }
  return merged;
}
