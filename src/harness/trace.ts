/**
 * Every prompt that leaves this process, written down before it goes.
 *
 * This exists because three separate failures in one day were invisible from
 * the outside: compaction died on the free tier and nothing said so, the
 * unobserved backlog rode along in every prompt and nothing showed it, and
 * whether observation was running at all could not be answered from any log
 * we had. The spend ledger meters what the harness itself sends; it cannot
 * see what Mastra sends on its own behalf - the observation and reflection
 * calls that are precisely the ones in question.
 *
 * So this wraps fetch, at the bottom, where every call to OpenRouter has to
 * pass whoever composed it. One JSON line per request into the character's
 * own trace file: when, what model, how many messages, how big, and the full
 * request body. The response is logged by status only, plus the error text
 * when there is one, because error text is where "your daily cap is spent"
 * and "temporarily rate-limited upstream" live.
 *
 * What is deliberately never written: headers. The Authorization header IS
 * the API key, and the reason the body is safe to log wholesale is that the
 * key never appears in a body. The request is reconstructed field by field
 * from (url, init) and the headers are not among the fields.
 *
 * The file is capped and rotated once, trace -> trace.old, so a week of
 * sixty-thousand-token prompts cannot fill the volume.
 */

import { appendFileSync, mkdirSync, renameSync, statSync } from 'node:fs';

/** Per-file cap before rotation. Two files per character at most. */
const ROTATE_AT = 50 * 1024 * 1024;
/** The largest request body kept whole. Beyond this, head and tail. */
const BODY_CAP = 400_000;

let traceFile: string | null = null;

function rotateIfHuge(file: string): void {
  try {
    if (statSync(file).size > ROTATE_AT) {
      renameSync(file, `${file}.old`);
    }
  } catch {
    // No file yet is the usual case and not a problem.
  }
}

function clipBody(body: string): string {
  if (body.length <= BODY_CAP) {
    return body;
  }
  return `${body.slice(0, BODY_CAP / 2)}\n...[${body.length} chars, middle cut]...\n${body.slice(-BODY_CAP / 2)}`;
}

/** What kind of call this is, judged from the request itself. */
function purposeOf(parsed: { messages?: Array<{ role?: string; content?: unknown }> }): string {
  const first = parsed.messages?.[0];
  const text = typeof first?.content === 'string' ? first.content : JSON.stringify(first?.content ?? '');
  if (text.includes('observations block') || text.includes('<observations>')) {
    return 'observation';
  }
  if (text.includes('reflect') && text.includes('observation')) {
    return 'reflection';
  }
  return 'agent';
}

function jot(entry: Record<string, unknown>): void {
  if (!traceFile) {
    return;
  }
  try {
    rotateIfHuge(traceFile);
    appendFileSync(traceFile, `${JSON.stringify(entry)}\n`);
  } catch {
    // A trace that cannot be written must never take the character with it.
  }
}

/**
 * Start tracing. Idempotent, and does nothing unless NPC_TRACE is set, so a
 * character without the flag pays one boolean per fetch and nothing else.
 */
export function installTrace(who: string, directory: string): void {
  if (!process.env.NPC_TRACE || traceFile) {
    return;
  }
  try {
    mkdirSync(`${directory}/trace`, { recursive: true });
  } catch {
    return;
  }
  traceFile = `${directory}/trace/${who}.jsonl`;
  const underneath = globalThis.fetch;
  globalThis.fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    if (!url.includes('openrouter.ai')) {
      return underneath(input, init);
    }
    const body = typeof init?.body === 'string' ? init.body : '';
    let parsed: { model?: string; messages?: Array<{ role?: string; content?: unknown }>; stream?: boolean } = {};
    try {
      parsed = JSON.parse(body);
    } catch {
      // Not JSON; the raw body still gets logged below.
    }
    const at = new Date().toISOString();
    const started = Date.now();
    let response: Response;
    try {
      response = await underneath(input, init);
    } catch (error) {
      jot({
        at, who, url,
        model: parsed.model ?? '',
        purpose: purposeOf(parsed),
        messages: parsed.messages?.length ?? 0,
        ms: Date.now() - started,
        failed: String(error),
        body: clipBody(body)
      });
      throw error;
    }
    const entry: Record<string, unknown> = {
      at, who, url,
      model: parsed.model ?? '',
      purpose: purposeOf(parsed),
      messages: parsed.messages?.length ?? 0,
      bodyChars: body.length,
      status: response.status,
      ms: Date.now() - started,
      body: clipBody(body)
    };
    if (!response.ok) {
      // The error text is the diagnosis: cap-spent, rate-limited upstream,
      // context length exceeded. Reading it off a clone leaves the original
      // stream untouched for whoever asked.
      entry.error = await response.clone().text().catch(() => '(unreadable)');
    }
    jot(entry);
    return response;
  };
  console.log(`trace: every OpenRouter prompt from ${who} -> ${traceFile}`);
}
