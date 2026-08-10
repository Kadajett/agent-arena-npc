/**
 * What each thought costs, in tokens and in money, said out loud.
 *
 * This exists because a whole afternoon went into reasoning about this bill
 * from the outside and getting it wrong twice. Once the daily cost was
 * estimated an order of magnitude low. Once a change made specifically to save
 * money raised it from $0.56 an hour to $0.75, and the only reason anybody
 * found out was that the account emptied.
 *
 * From outside there is nothing to reason with. The balance gives one number
 * for everything at once, and OpenRouter refuses per-account activity to an
 * inference key, so an expensive prompt and a frequent one look identical. Both
 * were guessed at, and both guesses were wrong in different directions.
 *
 * So every model call this library makes now says what it cost. A minute of
 * that is worth more than any amount of argument about what ought to be
 * expensive, and it is per character, so the answer to "which one is eating the
 * money" stops being a matter of opinion.
 *
 * Prices are fetched from OpenRouter rather than written down here. A hardcoded
 * price is wrong the first time anybody changes model and wrong silently, which
 * is the failure this whole module exists to stop repeating.
 */

import { appendFileSync } from 'node:fs';

type Price = { prompt: number; completion: number };

const prices = new Map<string, Price>();
let loading: Promise<void> | null = null;

/** The model id as OpenRouter knows it: our strings carry a provider prefix. */
function openRouterId(model: string): string {
  return model.startsWith('openrouter/') ? model.slice('openrouter/'.length) : model;
}

async function loadPrices(): Promise<void> {
  try {
    const response = await fetch('https://openrouter.ai/api/v1/models', {
      signal: AbortSignal.timeout(15_000)
    });
    if (!response.ok) {
      return;
    }
    const body = (await response.json()) as { data?: Array<Record<string, any>> };
    for (const model of body.data ?? []) {
      const prompt = Number(model?.pricing?.prompt);
      const completion = Number(model?.pricing?.completion);
      if (Number.isFinite(prompt) && Number.isFinite(completion)) {
        prices.set(String(model.id), { prompt, completion });
      }
    }
  } catch {
    // No prices means tokens without money, which is still most of the value.
    // Never let the accounting be the thing that stops a character thinking.
  }
}

/** Warm the price list once. Safe to call from anywhere; only the first does work. */
export function learnPrices(): Promise<void> {
  loading ??= loadPrices();
  return loading;
}

const totals = { calls: 0, prompt: 0, completion: 0, dollars: 0 };

export function spentSoFar(): { calls: number; prompt: number; completion: number; dollars: number } {
  return { ...totals };
}

/** Only for tests: puts the running total back to nothing. */
export function forgetSpending(): void {
  totals.calls = 0;
  totals.prompt = 0;
  totals.completion = 0;
  totals.dollars = 0;
}

export function costOf(model: string, prompt: number, completion: number): number | null {
  const price = prices.get(openRouterId(model));
  if (!price) {
    return null;
  }
  return prompt * price.prompt + completion * price.completion;
}

/**
 * Tokens off a Mastra/AI-SDK response, whatever it happens to call them.
 *
 * The field names have moved between versions (promptTokens, inputTokens,
 * prompt_tokens) and a silent zero here reads as a free call, which is exactly
 * the wrong direction for a meter to be wrong in.
 */
export function tokensOf(response: unknown): { prompt: number; completion: number } | null {
  const usage = (response as { usage?: Record<string, unknown> })?.usage;
  if (!usage) {
    return null;
  }
  const pick = (...names: string[]): number => {
    for (const name of names) {
      const value = Number(usage[name]);
      if (Number.isFinite(value) && value > 0) {
        return value;
      }
    }
    return 0;
  };
  const prompt = pick('promptTokens', 'inputTokens', 'prompt_tokens', 'input_tokens');
  const completion = pick('completionTokens', 'outputTokens', 'completion_tokens', 'output_tokens');
  if (0 === prompt && 0 === completion) {
    return null;
  }
  return { prompt, completion };
}

/**
 * Which model actually answered.
 *
 * Worth reading off the response rather than off the agent, because an agent
 * no longer names one model: it names a preferred one with the free router
 * underneath, so the thing that answered may not be the thing that was asked.
 * Costing a fallback reply at the preferred model's price would quietly
 * misreport exactly the situation the fallback exists for.
 */
export function modelOf(response: unknown): string {
  const box = response as Record<string, any>;
  const found =
    box?.response?.modelId
    ?? box?.response?.model
    ?? box?.modelId
    ?? box?.model
    ?? box?.providerMetadata?.openrouter?.model;
  return 'string' === typeof found ? found : '';
}

/**
 * Record one call and say what it cost.
 *
 * `who` is the character, because the useful question is never "what did this
 * world cost" but "which of them is costing it". `what` separates a character
 * deciding what to do from the same character planning, which are different
 * sizes of prompt and were previously indistinguishable in the total.
 */
/**
 * Where the numbers go so something other than a log tail can read them.
 *
 * One line per call, appended, on the volume every character already shares.
 * Nothing here is a secret: a character's name, a model id, two token counts
 * and a price. The key is never in this file and must never be, which is why
 * this writes named fields rather than dumping a request or a response object,
 * since those carry headers.
 */
const LEDGER = process.env.NPC_SPEND_LOG ?? '/npc/var/spend.jsonl';

function jot(line: Record<string, unknown>): void {
  try {
    appendFileSync(LEDGER, `${JSON.stringify(line)}\n`);
  } catch {
    // A character that cannot write its accounts still thinks.
  }
}

export function meter(who: string, what: string, response: unknown, fallbackModel = ''): void {
  const model = modelOf(response) || fallbackModel;
  const tokens = tokensOf(response);
  if (!tokens) {
    return;
  }
  const dollars = costOf(model, tokens.prompt, tokens.completion);
  totals.calls++;
  totals.prompt += tokens.prompt;
  totals.completion += tokens.completion;
  totals.dollars += dollars ?? 0;
  const money = null === dollars ? 'price unknown' : `$${dollars.toFixed(6)}`;
  console.log(
    `spend ${who} ${what}: in ${tokens.prompt} out ${tokens.completion} ${money}`
    + ` | run total ${totals.calls} calls, $${totals.dollars.toFixed(4)}`
  );
  jot({
    at: new Date().toISOString(),
    who,
    what,
    model,
    in: tokens.prompt,
    out: tokens.completion,
    usd: dollars ?? null
  });
}

/**
 * Anything worth watching that is not money: what a character did, and every
 * correction the harness handed it.
 *
 * Same file, same shape, because the question that matters is always both at
 * once. "Guy cost $4 today" is not useful next to "Guy tried the same door
 * thirteen times"; together they are the whole story, and separating them into
 * two places is how you end up reading neither.
 */
export function note(who: string, kind: string, detail: string): void {
  jot({ at: new Date().toISOString(), who, what: kind, detail: detail.slice(0, 240) });
}
