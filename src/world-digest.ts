/**
 * One chronicle of the whole town, every few hours, to Discord.
 *
 * Spec: docs/world-digest.md. A standalone entrypoint (CMD override in
 * docker-compose), not a character: it reads the public chat feed, asks a
 * model to write the town's last window in-world, and posts one message.
 * The per-character self-digest in harness/discord.ts stays available but
 * off; this replaces it as the thing people actually read.
 */

import { readFileSync, writeFileSync } from 'node:fs';
import { postToDiscord } from './harness/discord.js';

const FEED = process.env.CHAT_FEED_URL ?? 'https://chat.yougotserved.dev';
const WEBHOOK = process.env.WORLD_DIGEST_WEBHOOK_URL ?? process.env.DISCORD_WEBHOOK_URL ?? '';
const INTERVAL_MS = Number(process.env.DIGEST_INTERVAL_HOURS ?? 6) * 3600_000;
const MODEL = process.env.DIGEST_MODEL ?? 'openai/gpt-oss-120b';
const CURSOR = `${process.env.NPC_MEMORY_DIR ?? '/npc/var'}/world-digest-cursor.json`;
/** Context beyond the window, so arrivals and silences are decidable. */
const LOOKBACK_MS = 3 * 24 * 3600_000;

type Line = { at: string; scene: string; from: string | null; message: string };

const VOICE = `You write a short chronicle of a small game town for the people
who follow it from outside. Rules:
- Entirely in-world. The characters are people. Never mention models, agents,
  AI, containers, operators, or anything technical.
- Three to five short paragraphs, each opening with a bold header phrase in
  **stars**, markdown-style.
- Wry, concrete, affectionate. Specific over general.
- Quote a line verbatim when it carries weight; never invent a quote.
- Name who is new, who has gone quiet, and whose schemes or feuds moved.
- Under 280 words. No preamble, no sign-off.`;

/**
 * Everything the writer needs, computed mechanically. Exported for the test.
 * Arrivals: first line ever (within lookback) falls inside the window.
 * Quiet: spoke before the window, not within it.
 */
export function buildBriefing(lines: Line[], sinceMs: number, nowMs: number) {
  const speakers = new Map<string, { first: number; last: number; inWindow: number }>();
  for (const line of lines) {
    if (!line.from) {
      continue;
    }
    const at = Date.parse(line.at);
    const entry = speakers.get(line.from) ?? { first: at, last: at, inWindow: 0 };
    entry.first = Math.min(entry.first, at);
    entry.last = Math.max(entry.last, at);
    if (at >= sinceMs && at <= nowMs) {
      entry.inWindow += 1;
    }
    speakers.set(line.from, entry);
  }
  const active = [...speakers.entries()].filter(([, s]) => s.inWindow > 0);
  return {
    windowHours: Math.round((nowMs - sinceMs) / 3600_000),
    spoke: active.map(([name, s]) => ({ name, lines: s.inWindow }))
      .sort((a, b) => b.lines - a.lines),
    arrivals: active.filter(([, s]) => s.first >= sinceMs).map(([name]) => name),
    quiet: [...speakers.entries()]
      .filter(([, s]) => s.inWindow === 0 && s.last < sinceMs)
      .map(([name, s]) => ({ name, silentHours: Math.round((nowMs - s.last) / 3600_000) }))
      .sort((a, b) => a.silentHours - b.silentHours)
      .slice(0, 8),
    transcript: lines
      .filter((line) => line.from && Date.parse(line.at) >= sinceMs)
      .map((line) => `[${line.at.slice(11, 16)}][${line.scene}] ${line.from}: ${line.message}`)
      .slice(-350)
      .join('\n')
      .slice(-14_000)
  };
}

async function walkFeed(cutoffMs: number): Promise<Line[]> {
  const lines: Line[] = [];
  let before: number | null = null;
  for (let page = 0; page < 200; page += 1) {
    const url = new URL('/api/chat', FEED);
    url.searchParams.set('limit', '200');
    if (before !== null) {
      url.searchParams.set('before', String(before));
    }
    const response = await fetch(url, { signal: AbortSignal.timeout(20_000) });
    if (!response.ok) {
      throw new Error(`chat feed answered ${response.status}`);
    }
    const data = await response.json() as { lines?: Line[]; oldest?: number; hasMore?: boolean };
    const batch = data.lines ?? [];
    lines.push(...batch.filter((line) => Date.parse(line.at) >= cutoffMs));
    const oldestAt = batch.length ? Math.min(...batch.map((line) => Date.parse(line.at))) : null;
    const next = data.oldest;
    const stuck = next == null || (before !== null && !(next < before));
    if (!data.hasMore || stuck || oldestAt === null || oldestAt < cutoffMs) {
      break;
    }
    before = next;
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  return lines;
}

async function write(briefing: ReturnType<typeof buildBriefing>): Promise<string> {
  const response = await fetch('https://openrouter.ai/api/v1/chat/completions', {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${process.env.OPENROUTER_API_KEY}`,
      'Content-Type': 'application/json'
    },
    body: JSON.stringify({
      model: MODEL,
      messages: [
        { role: 'system', content: VOICE },
        {
          role: 'user',
          content: `The last ${briefing.windowHours} hours of the town's talk.\n`
            + `Spoke: ${briefing.spoke.map((s) => `${s.name}(${s.lines})`).join(', ') || 'nobody'}\n`
            + `New arrivals this window: ${briefing.arrivals.join(', ') || 'none'}\n`
            + `Gone quiet: ${briefing.quiet.map((q) => `${q.name}(${q.silentHours}h)`).join(', ') || 'none'}\n\n`
            + `Transcript:\n${briefing.transcript}\n\nWrite the chronicle.`
        }
      ]
    }),
    signal: AbortSignal.timeout(120_000)
  });
  if (!response.ok) {
    throw new Error(`model answered ${response.status}`);
  }
  const data = await response.json() as { choices?: Array<{ message?: { content?: string } }> };
  const text = data.choices?.[0]?.message?.content?.trim();
  if (!text) {
    throw new Error('model returned an empty chronicle');
  }
  return text;
}

function readCursor(): number | null {
  try {
    return Number(JSON.parse(readFileSync(CURSOR, 'utf8')).lastPostedAt) || null;
  } catch {
    return null;
  }
}

function log(...parts: unknown[]): void {
  console.log(new Date().toISOString().slice(11, 19), ...parts);
}

async function runForever(): Promise<void> {
  if (!WEBHOOK) {
    console.error('Set WORLD_DIGEST_WEBHOOK_URL (or DISCORD_WEBHOOK_URL).');
    process.exit(1);
  }
  for (;;) {
    const now = Date.now();
    const since = readCursor() ?? now - INTERVAL_MS;
    if (now - since >= INTERVAL_MS) {
      try {
        const lines = await walkFeed(now - LOOKBACK_MS);
        const briefing = buildBriefing(lines, since, now);
        if (briefing.spoke.length === 0) {
          log('nobody spoke; skipping this window');
        } else {
          const chronicle = await write(briefing);
          const landed = await postToDiscord(WEBHOOK, chronicle);
          log(landed ? 'chronicle posted' : 'chronicle failed to post; will retry next interval');
          if (!landed) {
            throw new Error('post failed');
          }
        }
        writeFileSync(CURSOR, JSON.stringify({ lastPostedAt: now }));
      } catch (error) {
        log('digest pass failed:', (error as Error)?.message ?? error);
      }
    }
    await new Promise((resolve) => setTimeout(resolve, 60_000));
  }
}

const invokedDirectly = process.argv[1]?.endsWith('world-digest.js');
if (invokedDirectly) {
  await runForever();
}
