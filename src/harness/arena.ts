/**
 * The Arena's MCP endpoint, spoken to the way any other agent would.
 *
 * This is deliberately not a general MCP client. Guy is an NPC, and the tools
 * he is given are a small, safe subset of what the gateway exposes, wrapped so
 * a model can only ask for things that make sense for a person in a town. The
 * gateway is the boundary that enforces that anyway, but there is no reason to
 * hand a character sheet the whole surface.
 */

import { decode as toonDecode } from '@toon-format/toon';

const MCP_URL = process.env.ARENA_MCP_URL ?? 'https://mcp.yougotserved.dev/mcp';
const API_KEY = process.env.ARENA_API_KEY ?? '';
const REQUEST_TIMEOUT_MS = 60_000;

/**
 * An NPC, trader, or enemy standing in the scene, as arena_observe reports it.
 * `label` is its name, the way a person would refer to it. `objectIndex` is
 * what targets an attack; it is not the same value as an NPC dialogue box's
 * id, which the gateway keeps to itself and this harness never needs to see
 * directly - talking to someone is done by name, the way a character actually
 * thinks about who it is talking to.
 */
export type ArenaObject = {
  objectId: number | null;
  objectIndex: string;
  label: string;
  kind: 'npc' | 'enemy';
  /**
   * Whether this one keeps a shop. A trader and a townsperson are both people
   * you walk up to and talk to, so they arrive with the same `kind`; only this
   * says which of them will sell you anything.
   */
  isMerchant?: boolean;
  interactable?: boolean;
  distanceFromSelf?: number;
  /**
   * Where it is standing, in tiles rather than pixels - the gateway never
   * hands the harness raw pixel coordinates for another object, only its own.
   * Required rather than optional: headless-client.js's visibleEntities()
   * computes these for every entity it reports and drops anything it could
   * not (see the filter right after roomObjectEntities() in that file), so
   * an object that reaches here always has them. See walkToSomebody() in
   * actions.ts for what they are for: reaching somebody outside home turf,
   * where there is no other way to say where to walk.
   */
  tileX: number;
  tileY: number;
};

/**
 * One thing in the satchel, as the gateway reports it. `key` is what the
 * world knows it by and what the trading tools want; `label` is what a person
 * would call it. Nothing in this harness has a list of what any of these
 * might be - the catalogue lives in the world, and a character only ever
 * knows what it is actually holding.
 */
export type CarriedItem = {
  key: string;
  label: string;
  description?: string | null;
  quantity: number;
  usable: boolean;
  equipment: boolean;
  equipped: boolean;
};

/**
 * Loot on the ground. `itemKey` is read back out of the drop's id by the
 * gateway and is null when it could not be, because the world announces a
 * drop as a sprite and a position and never says what it is.
 */
export type SeenDrop = {
  dropId: string;
  itemKey: string | null;
  distanceFromSelf?: number;
};

export type Observation = {
  ownPlayer?: { state?: { scene?: string; x?: number; y?: number } };
  sceneName?: string;
  /**
   * Everyone standing in the room. The gateway has always sent sessionId,
   * playerId and full state for each of them; this type used to keep only the
   * name and throw the rest away at the boundary, which is why no character
   * could ever aim at a person. Duelling needs the ids: a player target goes
   * out as target_session_id/target_player_id where an enemy goes out as an
   * object index.
   */
  players?: Array<{
    name?: string;
    label?: string;
    playerName?: string;
    sessionId?: string;
    playerId?: number;
    state?: { x?: number; y?: number; scene?: string };
  }>;
  chat?: Array<{ from?: string; message?: string; receivedAt?: string }>;
  recentChat?: Array<{ from?: string; message?: string; receivedAt?: string }>;
  /** Nearby NPCs, traders, and enemies. See ArenaObject. */
  objects?: ArenaObject[];
  /**
   * What this character is holding. It rides along with the observation
   * rather than being fetched on its own, because the gateway keeps a running
   * copy of it and reading that costs nothing - and a character that has to
   * make a second call to find out what is in its own pockets will not bother.
   */
  carrying?: CarriedItem[];
  /** What is lying on the floor of this room. See SeenDrop. */
  drops?: SeenDrop[];
};

export class ArenaClient {
  private sessionId: string | null = null;
  private nextId = 1;

  async rpc(method: string, params: unknown): Promise<any> {
    const headers: Record<string, string> = {
      'content-type': 'application/json',
      accept: 'application/json, text/event-stream',
      authorization: `Bearer ${API_KEY}`,
      'mcp-protocol-version': '2025-06-18',
      // Cloudflare rejects unrecognised agents at the edge.
      'user-agent': 'AgentArena-Guy/1.0'
    };
    if (this.sessionId) {
      headers['mcp-session-id'] = this.sessionId;
    }
    const response = await fetch(MCP_URL, {
      method: 'POST',
      headers,
      body: JSON.stringify({ jsonrpc: '2.0', id: this.nextId++, method, params }),
      signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS)
    });
    this.sessionId = response.headers.get('mcp-session-id') ?? this.sessionId;
    const contentType = response.headers.get('content-type') ?? '';
    const payload = contentType.includes('text/event-stream')
      ? await readSseEvent(response)
      : await response.json();
    if (payload?.error) {
      throw new Error(`${method}: ${JSON.stringify(payload.error)}`);
    }
    return payload?.result;
  }

  async start(): Promise<void> {
    this.sessionId = null;
    await this.rpc('initialize', {
      protocolVersion: '2025-06-18',
      capabilities: {},
      clientInfo: { name: 'guy', version: '1.0' }
    });
    await this.rpc('notifications/initialized', {}).catch(() => undefined);
  }

  async call(name: string, args: Record<string, unknown>): Promise<any> {
    const result = await this.rpc('tools/call', { name, arguments: args });
    const body = decodeResult(result.content[0].text);
    if (result.isError) {
      throw new Error(`${name}: ${body?.error}: ${body?.message}`);
    }
    return body;
  }
}

/**
 * A tool result, whichever encoding the gateway chose for it.
 *
 * Since agentArena#127 the gateway ships each result as TOON whenever that is
 * meaningfully smaller than JSON, and as JSON otherwise - per payload, with no
 * marker saying which. Parsing JSON first and falling back keeps both working;
 * without the fallback, one TOON-shaped reply (arena_list_agents is reliably
 * one) wedges the reconnect loop forever, which is how Bolo spent a night
 * retrying every fifteen seconds.
 */
function decodeResult(text: string): any {
  try {
    return JSON.parse(text);
  } catch {
    return toonDecode(text);
  }
}

/**
 * Take one complete event off an SSE response.
 *
 * The stream stays open after the reply is delivered, so this stops as soon as
 * what has arrived parses as JSON rather than reading to the end, which would
 * never come. Reading through response.body keeps the chunked transfer decoded
 * for us; reading a raw socket does not, and hands back payloads cut at chunk
 * boundaries.
 */
async function readSseEvent(response: Response): Promise<any> {
  const body = response.body;
  if (!body) {
    throw new Error('The MCP endpoint returned an event stream with no body.');
  }
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffered = '';
  let data: string[] = [];
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (done) {
        break;
      }
      buffered += decoder.decode(value, { stream: true });
      let newline: number;
      while ((newline = buffered.indexOf('\n')) !== -1) {
        const line = buffered.slice(0, newline).replace(/\r$/, '');
        buffered = buffered.slice(newline + 1);
        if (line.startsWith('data:')) {
          data.push(line.slice(5).trimStart());
          try {
            return JSON.parse(data.join('\n'));
          } catch {
            continue; // a value split across several data: lines
          }
        }
        if (line === '') {
          data = []; // an event we could not read; start the next one clean
        }
      }
    }
  } finally {
    await reader.cancel().catch(() => undefined);
  }
  throw new Error('The MCP endpoint closed the stream without a reply.');
}

export function sceneOf(observation: Observation): string {
  return observation.ownPlayer?.state?.scene ?? observation.sceneName ?? '';
}

export function othersIn(observation: Observation, self: string): string[] {
  return (observation.players ?? [])
    .map((player) => nameOf(player))
    .filter((name) => name && name !== self);
}

export function nameOf(player: { name?: string; label?: string; playerName?: string }): string {
  return player.playerName ?? player.name ?? player.label ?? '';
}

/**
 * The engine's own chatter, which is not anybody speaking.
 *
 * Reldens announces joins, leaves and the like down the same channel people
 * talk on, as a bare key: "chat.joinedRoom". Nothing was filtering those, so
 * every one arrived as a line of dialogue from "someone", went into the
 * transcript, and was written to memory as a thing a character had heard said
 * to it. Guy's memory was largely this. It is the cheapest possible thing to
 * be paying to re-read on every tick for the rest of his life.
 *
 * Matched on the shape rather than a list of keys, so whatever the engine adds
 * next is caught too: these are all dotted identifiers with no spaces, and no
 * person says "chat.joinedRoom".
 */
const ENGINE_CHATTER = /^[a-z]+\.[a-zA-Z]+$/;

export function isEngineChatter(message: string): boolean {
  return ENGINE_CHATTER.test(String(message ?? '').trim().replace(/^"|"$/g, ''));
}

export function spokenLines(observation: Observation): Array<{ from: string; message: string; at: string }> {
  const entries = observation.chat ?? observation.recentChat ?? [];
  return entries
    .filter((entry) => entry.message)
    .filter((entry) => !isEngineChatter(entry.message as string))
    .map((entry) => ({
      from: entry.from ?? 'someone',
      message: entry.message as string,
      at: entry.receivedAt ?? ''
    }));
}
