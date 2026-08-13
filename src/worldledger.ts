/**
 * The world-level claims aggregator: one process, no character, no game
 * login. It reads the same public feeds the spectator viewer does - chat,
 * activity, thoughts - across every character currently in the world, ours
 * and anybody else's, and folds what it finds into a running table of
 * claims with a confidence tier attached. See src/worldledger/graph.ts for
 * what a tier actually means and why.
 *
 * Runs on its own clock rather than reacting to anything: every
 * CYCLE_MS, whether or not the world has been busy. A quiet cycle with
 * nothing new since the last one is not an error, just nothing to fold in.
 */

import { fetchActivity, fetchChat, fetchThoughts, fetchWatchable } from './worldledger/api.js';
import type { ActivityCall, ChatLine, ThoughtEntry } from './worldledger/api.js';
import { extractClaims } from './worldledger/extract.js';
import { classifyClaim } from './worldledger/graph.js';
import { loadClaims, loadCursor, mergeClaims, saveCursor } from './worldledger/store.js';

const CYCLE_MS = 4 * 60 * 60 * 1000;
const MEMORY_DIR = process.env.NPC_MEMORY_DIR ?? '/npc/var';
/** Kept short on purpose - reference material for spotting a conflict, not a second transcript. */
const MAX_ESTABLISHED_CLAIMS = 40;

function log(...parts: unknown[]): void {
  console.log(new Date().toISOString().slice(11, 19), ...parts);
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function runCycle(): Promise<void> {
  const cursor = loadCursor(MEMORY_DIR);
  const [watchable, chat] = await Promise.all([fetchWatchable(), fetchChat(cursor.chat)]);
  if (0 === chat.lines.length) {
    log('nothing new since last cycle');
    saveCursor(MEMORY_DIR, cursor);
    return;
  }

  const names = [...new Set(watchable.characters.map((entry) => entry.character))];
  let activitySince = cursor.activity;
  const activityByPlayer = new Map<string, ActivityCall[]>();
  const thoughtsByPlayer = new Map<string, ThoughtEntry[]>();
  const nextThoughtsCursor: Record<string, string> = { ...cursor.thoughtsByPlayer };

  await Promise.all(
    names.map(async (name) => {
      const [activity, thoughts] = await Promise.all([
        fetchActivity(name, cursor.activity).catch(() => null),
        fetchThoughts(name, cursor.thoughtsByPlayer[name] ?? '0:0').catch(() => null)
      ]);
      if (activity) {
        activityByPlayer.set(name.toLowerCase(), activity.calls);
        activitySince = Math.max(activitySince, activity.cursor);
      }
      if (thoughts) {
        thoughtsByPlayer.set(name.toLowerCase(), thoughts.thoughts);
        nextThoughtsCursor[name] = thoughts.cursor;
      }
    })
  );

  const established = loadClaims(MEMORY_DIR)
    .filter((claim) => 'read' === claim.tier || 'possibly-true' === claim.tier)
    .sort((left, right) => right.lastSeen.localeCompare(left.lastSeen))
    .slice(0, MAX_ESTABLISHED_CLAIMS)
    .map((claim) => claim.text);

  const linesById = new Map<number, ChatLine>(chat.lines.map((line) => [line.id, line]));
  const extracted = await extractClaims(chat.lines, established);
  const generatedAt = new Date().toISOString();
  const classified = extracted.map((claim) =>
    classifyClaim(
      claim.text,
      claim.lineIds,
      linesById,
      activityByPlayer,
      thoughtsByPlayer,
      claim.consistent,
      claim.contradicts
    )
  );
  const merged = mergeClaims(MEMORY_DIR, classified, generatedAt);

  saveCursor(MEMORY_DIR, { chat: chat.cursor, activity: activitySince, thoughtsByPlayer: nextThoughtsCursor });

  for (const claim of classified) {
    if (claim.contradicts) {
      log(`contradiction flagged: "${claim.text}" vs established "${claim.contradicts}"`);
    }
  }
  const byTier = classified.reduce<Record<string, number>>((counts, claim) => {
    counts[claim.tier] = (counts[claim.tier] ?? 0) + 1;
    return counts;
  }, {});
  const byAuthenticity = classified.reduce<Record<string, number>>((counts, claim) => {
    counts[claim.authenticity] = (counts[claim.authenticity] ?? 0) + 1;
    return counts;
  }, {});
  log(
    `cycle done: ${chat.lines.length} lines from ${names.length} characters,`,
    `${extracted.length} claims this cycle, tier ${JSON.stringify(byTier)},`,
    `authenticity ${JSON.stringify(byAuthenticity)}, ${merged.length} claims total`
  );
}

async function main(): Promise<void> {
  log('world ledger starting, cycling every', CYCLE_MS / 3_600_000, 'hours');
  for (;;) {
    try {
      await runCycle();
    } catch (error) {
      log('cycle failed:', (error as Error)?.message ?? error);
    }
    await sleep(CYCLE_MS);
  }
}

main();
