/**
 * One chronicle of the whole town, every few hours, to Discord.
 *
 * Spec: docs/world-digest.md. A standalone entrypoint (CMD override in
 * docker-compose), not a character: it reads the public chat feed, asks a
 * model to write the town's last window in-world, and posts one message.
 *
 * Delivery is at-most-once by choice: the state file records the attempt
 * before the webhook call and the completion after it, both written
 * atomically (tmp + rename). A crash between post and record makes the
 * restart treat that window as covered rather than risk posting it twice -
 * a lost chronicle reads as a quiet stretch; a duplicated one reads as a
 * malfunction, in public.
 */

import { readFileSync, writeFileSync, renameSync } from 'node:fs';

const FEED = process.env.CHAT_FEED_URL ?? 'https://chat.yougotserved.dev';
const WEBHOOK = process.env.WORLD_DIGEST_WEBHOOK_URL ?? process.env.DISCORD_WEBHOOK_URL ?? '';
const INTERVAL_HOURS = Number(process.env.DIGEST_INTERVAL_HOURS ?? 6);
const MODEL = process.env.DIGEST_MODEL ?? 'openai/gpt-oss-120b';
const STATE = `${process.env.NPC_MEMORY_DIR ?? '/npc/var'}/world-digest-state.json`;
/**
 * How far past the window the walk reaches for context. Silences are
 * decidable from this; arrivals are not - a resident can outlast any fixed
 * lookback - so arrivals come from the persisted speaker roster instead.
 */
const LOOKBACK_MS = 3 * 24 * 3600_000;

type Line = { at: string; scene: string; from: string | null; message: string };
type State = { lastPostedAt?: number; attemptedAt?: number; knownSpeakers?: string[] };

/** Discord's own ceiling on one message; longer is rejected outright. */
const DISCORD_MESSAGE_LIMIT = 2000;

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
 * Arrivals: spoke in the window and absent from the persisted roster of
 * every name ever seen - never inferred from bounded history, which would
 * call a returning resident new. Quiet: spoke before the window, not in it.
 */
export function buildBriefing(
  lines: Line[],
  sinceMs: number,
  nowMs: number,
  knownSpeakers: ReadonlySet<string> = new Set()
) {
  const speakers = new Map<string, { last: number; inWindow: number }>();
  for (const line of lines) {
    if (!line.from) {
      continue;
    }
    const at = Date.parse(line.at);
    const entry = speakers.get(line.from) ?? { last: at, inWindow: 0 };
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
    arrivals: active.filter(([name]) => !knownSpeakers.has(name)).map(([name]) => name),
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

/**
 * Send one message to a Discord incoming webhook. Never throws; returns
 * whether it landed, so a rejected or timed-out post is not read as done.
 */
async function postToDiscord(webhookUrl: string, content: string): Promise<boolean> {
  try {
    const response = await fetch(webhookUrl, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ content: content.slice(0, DISCORD_MESSAGE_LIMIT) }),
      signal: AbortSignal.timeout(10_000)
    });
    if (!response.ok) {
      log('discord post refused:', response.status, await response.text().catch(() => ''));
      return false;
    }
    return true;
  } catch (error) {
    log('discord post failed:', (error as Error)?.message ?? error);
    return false;
  }
}

function readState(): State {
  try {
    const state = JSON.parse(readFileSync(STATE, 'utf8')) as State;
    return state && typeof state === 'object' ? state : {};
  } catch {
    return {};
  }
}

/** Atomic: a crash mid-write leaves the old state, never a torn file. */
function writeState(state: State): void {
  writeFileSync(`${STATE}.tmp`, JSON.stringify(state));
  renameSync(`${STATE}.tmp`, STATE);
}

function log(...parts: unknown[]): void {
  console.log(new Date().toISOString().slice(11, 19), ...parts);
}

async function runForever(): Promise<void> {
  if (!WEBHOOK) {
    console.error('Set WORLD_DIGEST_WEBHOOK_URL (or DISCORD_WEBHOOK_URL).');
    process.exit(1);
  }
  if (!Number.isFinite(INTERVAL_HOURS) || INTERVAL_HOURS <= 0) {
    console.error(
      `DIGEST_INTERVAL_HOURS must be a positive number, not "${process.env.DIGEST_INTERVAL_HOURS}".`
    );
    process.exit(1);
  }
  const intervalMs = INTERVAL_HOURS * 3600_000;
  for (;;) {
    const state = readState();
    // An attempt with no completion means the process died between the
    // webhook call and the record. The window may have posted; treat it as
    // covered (at-most-once - see the file header).
    if (state.attemptedAt && state.attemptedAt > (state.lastPostedAt ?? 0)) {
      log('recovering: an attempted window is treated as posted');
      writeState({ ...state, lastPostedAt: state.attemptedAt, attemptedAt: undefined });
      continue;
    }
    const now = Date.now();
    const since = state.lastPostedAt ?? now - intervalMs;
    if (now - since >= intervalMs) {
      try {
        // Reach back to the cursor even when failures left it further back
        // than the usual lookback: a late chronicle that covers its whole
        // window beats a punctual one with a hole in it.
        const lines = await walkFeed(Math.min(since, now - LOOKBACK_MS));
        const known = new Set(state.knownSpeakers ?? []);
        const briefing = buildBriefing(lines, since, now, known);
        for (const line of lines) {
          if (line.from) {
            known.add(line.from);
          }
        }
        if (briefing.spoke.length === 0) {
          log('nobody spoke; skipping this window');
        } else {
          const chronicle = await write(briefing);
          writeState({ ...state, attemptedAt: now, knownSpeakers: [...known] });
          const landed = await postToDiscord(WEBHOOK, chronicle);
          if (!landed) {
            // A known failure retries next interval; only a crash between
            // the call and this line leaves attemptedAt to trigger the
            // at-most-once skip. (A timed-out post that secretly landed can
            // therefore duplicate - rare, and preferred over losing the
            // window to every webhook hiccup.)
            writeState({ ...state, attemptedAt: undefined, knownSpeakers: [...known] });
            throw new Error('post failed');
          }
          log('chronicle posted');
        }
        writeState({ lastPostedAt: now, knownSpeakers: [...known] });
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
