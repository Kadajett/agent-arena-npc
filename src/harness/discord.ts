/**
 * Posting a character's own digest of itself out to Discord.
 *
 * A side channel, not a capability: never worth losing the character's own
 * turn over, and never allowed to hang the tick loop that awaits it - see
 * the timeout below. It does, however, tell its caller whether the post
 * actually landed, because npc.ts needs that to decide whether this episode
 * number was really spent or is free to try again. "Best effort" describes
 * how failure is handled, not that failure is invisible to the caller.
 */

import { readFileSync, writeFileSync } from 'node:fs';

function log(...parts: unknown[]): void {
  console.log(new Date().toISOString().slice(11, 19), ...parts);
}

/** Discord's own ceiling on one message; longer is rejected outright. */
const DISCORD_MESSAGE_LIMIT = 2000;

/**
 * Send one message to a Discord incoming webhook. Never throws; returns
 * whether it actually landed, so a rejected or timed-out post can be told
 * apart from a delivered one rather than both reading as "done".
 */
export async function postToDiscord(webhookUrl: string, content: string): Promise<boolean> {
  try {
    const response = await fetch(webhookUrl, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ content: content.slice(0, DISCORD_MESSAGE_LIMIT) }),
      // Discord hanging, or never answering at all, must not be able to
      // wedge the character's own tick loop, which awaits this call.
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

function episodeFile(memoryDir: string, characterId: string): string {
  return `${memoryDir}/${characterId}-episode.json`;
}

/**
 * The episode number this character's next digest should carry. Starts at
 * one and is kept in its own small file on the same volume memory already
 * lives on, so a redeploy resumes the count instead of starting the season
 * over. Not folded into the character's own working memory - that is the
 * model's own record of the world, written by the model; this is the
 * harness's bookkeeping about how many times it has posted, and the two
 * should not be able to overwrite each other.
 */
export function nextEpisode(memoryDir: string, characterId: string): number {
  try {
    const stored = JSON.parse(readFileSync(episodeFile(memoryDir, characterId), 'utf8'));
    const next = Number(stored?.next);
    return Number.isInteger(next) && next > 0 ? next : 1;
  } catch {
    return 1;
  }
}

/** Record that this episode number has gone out, so the next one moves past it. */
export function markEpisodeUsed(memoryDir: string, characterId: string, episode: number): void {
  try {
    writeFileSync(episodeFile(memoryDir, characterId), JSON.stringify({ next: episode + 1 }));
  } catch (error) {
    // Worst case the next digest reuses this number instead of moving past
    // it - a cosmetic repeat, not a reason to lose the digest itself.
    log('could not save episode number:', (error as Error)?.message ?? error);
  }
}
