/**
 * Who talks about a character when they are not in the room.
 *
 * Walks the public chat feed backwards and sorts every mention of the
 * character into in-presence or behind-their-back, using the character's own
 * lines as the record of where they were. Spec: docs/character-scorecard.md
 * AC5. Works for any character the feed has seen, resident or player - no
 * claims file needed.
 *
 * Presence is a heuristic, stated plainly in the output: the character
 * counts as present in a scene if they spoke in that scene within
 * PRESENCE_WINDOW_MS of the mention. A silent character in the corner reads
 * as absent; the feed records speech, not bodies.
 *
 * Usage: node scripts/salience.mjs <CharacterName> [days=3]
 *   CHAT_FEED_URL overrides the feed origin.
 */

const PRESENCE_WINDOW_MS = 10 * 60 * 1000;
const PAGE_LIMIT = 200;
const MAX_PAGES = 200;
const PAGE_PAUSE_MS = 300;

/**
 * Sort mentions of the target into in-presence and behind-their-back.
 * Pure: lines in, classification out. Exported for the test.
 */
export function classifySalience(lines, target, presenceWindowMs = PRESENCE_WINDOW_MS) {
  const wanted = target.toLowerCase();
  const presence = lines
    .filter((line) => line?.from === target)
    .map((line) => ({ scene: line.scene, at: Date.parse(line.at) }));
  const present = (scene, atMs) => presence.some(
    (mark) => mark.scene === scene && Math.abs(mark.at - atMs) <= presenceWindowMs
  );
  const bySpeaker = new Map();
  for (const line of lines) {
    if (!line?.from || line.from === target) {
      continue;
    }
    if (!String(line.message ?? '').toLowerCase().includes(wanted)) {
      continue;
    }
    const targetPresent = present(line.scene, Date.parse(line.at));
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
    scanned: lines.length,
    mentions: speakers.reduce((sum, s) => sum + s.mentions, 0),
    behindBack: speakers.reduce((sum, s) => sum + s.behindBack, 0),
    presenceWindowMinutes: presenceWindowMs / 60_000,
    bySpeaker: speakers
  };
}

async function walkFeed(origin, cutoffMs) {
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
    lines.push(...batch.filter((line) => Date.parse(line.at) >= cutoffMs));
    const oldestAt = batch.length ? Date.parse(batch[0].at) : null;
    if (!data.hasMore || oldestAt === null || oldestAt < cutoffMs) {
      break;
    }
    before = data.oldest;
    await new Promise((resolve) => setTimeout(resolve, PAGE_PAUSE_MS));
  }
  return lines;
}

const invokedDirectly = process.argv[1] && import.meta.url.endsWith(process.argv[1].split('/').pop());
if (invokedDirectly) {
  const [target, daysArg] = process.argv.slice(2);
  if (!target) {
    console.error('Usage: node scripts/salience.mjs <CharacterName> [days=3]');
    process.exit(1);
  }
  const days = Number(daysArg ?? 3);
  const origin = process.env.CHAT_FEED_URL ?? 'https://chat.yougotserved.dev';
  const cutoffMs = Date.now() - days * 24 * 60 * 60 * 1000;
  const lines = await walkFeed(origin, cutoffMs);
  console.log(JSON.stringify({ windowDays: days, ...classifySalience(lines, target) }, null, 2));
}
