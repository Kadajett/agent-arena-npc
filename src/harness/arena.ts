/**
 * The Arena's MCP endpoint, spoken to the way any other agent would.
 *
 * This is deliberately not a general MCP client. Guy is an NPC, and the tools
 * he is given are a small, safe subset of what the gateway exposes, wrapped so
 * a model can only ask for things that make sense for a person in a town. The
 * gateway is the boundary that enforces that anyway, but there is no reason to
 * hand a character sheet the whole surface.
 */

const MCP_URL = process.env.ARENA_MCP_URL ?? 'https://mcp.yougotserved.dev/mcp';
const API_KEY = process.env.ARENA_API_KEY ?? '';
const REQUEST_TIMEOUT_MS = 60_000;

export type Observation = {
  ownPlayer?: { state?: { scene?: string; x?: number; y?: number } };
  sceneName?: string;
  players?: Array<{ name?: string; label?: string }>;
  chat?: Array<{ from?: string; message?: string; receivedAt?: string }>;
  recentChat?: Array<{ from?: string; message?: string; receivedAt?: string }>;
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
    const body = JSON.parse(result.content[0].text);
    if (result.isError) {
      throw new Error(`${name}: ${body?.error}: ${body?.message}`);
    }
    return body;
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
    .map((player) => player.name ?? player.label ?? '')
    .filter((name) => name && name !== self);
}

export function spokenLines(observation: Observation): Array<{ from: string; message: string; at: string }> {
  const entries = observation.chat ?? observation.recentChat ?? [];
  return entries
    .filter((entry) => entry.message)
    .map((entry) => ({
      from: entry.from ?? 'someone',
      message: entry.message as string,
      at: entry.receivedAt ?? ''
    }));
}
