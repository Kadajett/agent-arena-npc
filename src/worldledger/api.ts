/**
 * The public, read-only surface the world exposes about itself - no API key,
 * no bridge token, nothing this process needs to authenticate as anybody.
 * Everything here is served by the same Worker behind chat.yougotserved.dev,
 * world.yougotserved.dev and agentarena.yougotserved.dev; which hostname is
 * used does not matter, so WORLD_API_URL just picks one.
 *
 * `/api/chat` is the one genuinely global feed: every line said by every
 * character, ours and every other harness talking to this world, paged by a
 * durable row id that survives a restart. `/api/watch/activity` and
 * `/api/watch/thoughts` are per character - passing no name returns only a
 * roster, not data - so watching everybody means asking who is here first
 * (`/api/watchable`) and then fanning out one call per name. Their ids are
 * drawn from one shared counter server-side, so the same `since` cursor
 * works unmodified across every one of those fanned-out calls.
 */

const REQUEST_TIMEOUT_MS = 15_000;

function baseUrl(): string {
  return (process.env.WORLD_API_URL ?? 'https://world.yougotserved.dev').replace(/\/+$/, '');
}

async function getJson<T>(path: string, params: Record<string, string | undefined> = {}): Promise<T> {
  const url = new URL(`${baseUrl()}${path}`);
  for (const [key, value] of Object.entries(params)) {
    if (undefined !== value && '' !== value) {
      url.searchParams.set(key, value);
    }
  }
  const response = await fetch(url, { signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS) });
  if (!response.ok) {
    throw new Error(`${path} -> ${response.status}`);
  }
  return response.json() as Promise<T>;
}

export type ChatLine = {
  id: number;
  message: string;
  at: string;
  type?: string;
  from: string;
  scene: string;
  spoken?: boolean;
};

export type ChatFeed = {
  generatedAt: string;
  cursor: number;
  oldest: number;
  hasMore: boolean;
  lines: ChatLine[];
};

/** Everything said world-wide since `since` (a row id, 0 for the beginning). */
export function fetchChat(since: number): Promise<ChatFeed> {
  return getJson<ChatFeed>('/api/chat', { since: String(since), limit: '200' });
}

export type Watchable = {
  generatedAt: string;
  rooms: Array<{ room: string; present: number }>;
  characters: Array<{ character: string; room: string }>;
};

/** Who is actually in the world right now, so activity/thoughts know who to ask about. */
export function fetchWatchable(): Promise<Watchable> {
  return getJson<Watchable>('/api/watchable');
}

export type ActivityCall = {
  id: number;
  at: string;
  player: string;
  tool: string;
  ok: boolean;
  args: Record<string, unknown>;
  error: string | null;
  /** Present only when the result was small enough to keep whole. */
  value?: unknown;
  truncated?: boolean;
};

export type ActivityFeed = {
  generatedAt: string;
  player: string;
  cursor: number;
  calls: ActivityCall[];
};

/** One character's own tool calls since `since`. Secret-redacted server-side. */
export function fetchActivity(player: string, since: number): Promise<ActivityFeed> {
  return getJson<ActivityFeed>('/api/watch/activity', { player, since: String(since), limit: '100' });
}

export type ThoughtEntry = {
  at: string;
  thought?: string;
  source: 'harness' | 'published';
  [key: string]: unknown;
};

export type ThoughtsFeed = {
  generatedAt: string;
  who: string;
  cursor: string;
  thoughts: ThoughtEntry[];
};

/**
 * One character's own reasoning since `since` - a compound cursor string,
 * not a number, because it carries two upstream sources' positions at once.
 * Public: this is the same feed the spectator viewer's mind panel reads.
 */
export function fetchThoughts(player: string, since: string): Promise<ThoughtsFeed> {
  return getJson<ThoughtsFeed>('/api/watch/thoughts', { who: player, since });
}
