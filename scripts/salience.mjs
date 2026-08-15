/**
 * Who talks about a character when they are not in the room.
 *
 * Walks the public chat feed backwards and sorts every mention of the
 * character into in-presence or behind-their-back, using the character's own
 * lines as the record of where they were. Spec: docs/character-scorecard.md
 * AC5. Works for any character the feed has seen, resident or player - no
 * claims file needed.
 *
 * Presence is a heuristic, stated in the output: the character counts as
 * present in a scene if they spoke in that scene within PRESENCE_WINDOW_MS
 * of the mention. A silent character in the corner reads as absent; the
 * feed records speech, not bodies. The walk fetches one presence window
 * past the cutoff so a mention near the window edge is judged against
 * presence lines that fall just outside it.
 *
 * Usage: node scripts/salience.mjs <CharacterName> [days=3]
 *   CHAT_FEED_URL overrides the feed origin.
 */
import { pathToFileURL } from 'node:url';

const PRESENCE_WINDOW_MS = 10 * 60 * 1000;
const PAGE_LIMIT = 200;
const MAX_PAGES = 200;
const PAGE_PAUSE_MS = 300;

const PRESENCE_HEURISTIC =
  'present = the character spoke in that scene within '
  + `${PRESENCE_WINDOW_MS / 60_000} minutes of the mention; a silent character reads as absent`;

/** A whole-word match, so Ada never counts a line about adamant. */
function mentionPattern(target) {
  const escaped = target.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  return new RegExp(`\\b${escaped}\\b`, 'i');
}

/**
 * Sort mentions of the target into in-presence and behind-their-back.
 * Pure: lines in, classification out. Exported for the test.
 *
 * `mentionsSinceMs` drops mentions older than the reporting window while
 * still judging presence from every line given - the caller feeds lines
 * from one presence window before the cutoff for exactly that reason.
 */
export function classifySalience(lines, target, presenceWindowMs = PRESENCE_WINDOW_MS, mentionsSinceMs = null) {
  const pattern = mentionPattern(target);
  const presence = lines
    .filter((line) => line?.from === target)
    .map((line) => ({ scene: line.scene, at: Date.parse(line.at) }));
  const present = (scene, atMs) => presence.some(
    (mark) => mark.scene === scene && Math.abs(mark.at - atMs) <= presenceWindowMs
  );
  const bySpeaker = new Map();
  let scanned = 0;
  for (const line of lines) {
    const atMs = Date.parse(line?.at);
    if (mentionsSinceMs !== null && !(atMs >= mentionsSinceMs)) {
      continue;
    }
    scanned += 1;
    if (!line?.from || line.from === target) {
      continue;
    }
    if (!pattern.test(String(line.message ?? ''))) {
      continue;
    }
    const targetPresent = present(line.scene, atMs);
    const bucket = bySpeaker.get(line.from)
      ?? { speaker: line.from, mentions: 0, behindBack: 0, lines: [] };
    bucket.mentions += 1;
    if (!targetPresent) {
      bucket.behindBack += 1;
    }
    bucket.lines.push({
      at: line.at, scene: line.scene, message: line.message, targetPresent
    });
    bySpeaker.set(line.from, bucket);
  }
  const speakers = [...bySpeaker.values()].sort((a, b) => b.behindBack - a.behindBack);
  return {
    target,
    scanned,
    mentions: speakers.reduce((sum, s) => sum + s.mentions, 0),
    behindBack: speakers.reduce((sum, s) => sum + s.behindBack, 0),
    presenceWindowMinutes: presenceWindowMs / 60_000,
    presenceHeuristic: PRESENCE_HEURISTIC,
    bySpeaker: speakers
  };
}

async function walkFeed(origin, fetchCutoffMs) {
  const lines = [];
  let before = null;
  for (let page = 0; page < MAX_PAGES; page += 1) {
    const url = new URL('/api/chat', origin);
    url.searchParams.set('limit', String(PAGE_LIMIT));
    if (before !== null) {
      url.searchParams.set('before', String(before));
    }
    const response = await fetch(url);
    if (!response.ok) {
      throw new Error(`chat feed answered ${response.status}`);
    }
    const data = await response.json();
    const batch = data.lines ?? [];
    lines.push(...batch.filter((line) => Date.parse(line.at) >= fetchCutoffMs));
    // Trust one source for both progress and termination: the feed's own
    // cursor. A missing or non-advancing cursor would otherwise refetch the
    // same page MAX_PAGES times and multiply every count by 200.
    const next = data.oldest;
    const oldestAt = batch.length
      ? Math.min(...batch.map((line) => Date.parse(line.at)))
      : null;
    const stuck = next == null || (before !== null && !(next < before));
    if (!data.hasMore || stuck || oldestAt === null || oldestAt < fetchCutoffMs) {
      if (page === MAX_PAGES - 1 || (stuck && data.hasMore)) {
        console.error('salience: walk ended early; counts may be partial');
      }
      break;
    }
    before = next;
    await new Promise((resolve) => setTimeout(resolve, PAGE_PAUSE_MS));
  }
  return lines;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [target, daysArg] = process.argv.slice(2);
  const days = Number(daysArg ?? 3);
  if (!target || !Number.isFinite(days) || days <= 0) {
    console.error('Usage: node scripts/salience.mjs <CharacterName> [days=3]');
    process.exit(1);
  }
  const origin = process.env.CHAT_FEED_URL ?? 'https://chat.yougotserved.dev';
  const cutoffMs = Date.now() - days * 24 * 60 * 60 * 1000;
  const lines = await walkFeed(origin, cutoffMs - PRESENCE_WINDOW_MS);
  console.log(JSON.stringify(
    { windowDays: days, ...classifySalience(lines, target, PRESENCE_WINDOW_MS, cutoffMs) },
    null,
    2
  ));
}
