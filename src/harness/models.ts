/**
 * What a character thinks with, and what it thinks with when the money runs out.
 *
 * This world has gone quiet before because an account emptied. Every character
 * stops at once, which is the worst possible failure: not one person behaving
 * oddly, but a town where nobody moves and nothing is obviously broken. The
 * logs say the provider refused and that is all.
 *
 * So nobody names a single model any more. A character names the one it would
 * rather use, and the free router is put underneath it, and Mastra walks the
 * list until something answers. A character on a paid model that has run dry
 * carries on thinking, more cheaply and probably less well, which is a great
 * deal better than standing still.
 *
 * The free router goes last for everyone and is never removed, because it is
 * the floor. If it is also the character's first choice there is nothing to
 * fall back to and the list is one entry: falling back from free to free would
 * just double the wait before giving up.
 */

import type { ModelWithRetries } from '@mastra/core/agent';

/**
 * OpenRouter's own free router, doubled on purpose.
 *
 * The first word is the provider and the rest is that provider's name for the
 * model. OpenRouter calls this one "openrouter/free", so asking for
 * "openrouter/free" asks the openrouter provider for a model called "free",
 * which does not exist and answers 502. Checked against the live endpoint.
 */
export const FREE_ROUTER = 'openrouter/openrouter/free';

/**
 * The rung between the preferred model and the floor.
 *
 * Falling straight from a paid model to the free router sounds fine until the
 * free router is out of requests for the day, which is its normal state: it is
 * capped at 1,000 for the whole account. Then a provider-side 429 on the
 * preferred model - which is a hiccup lasting seconds, not an outage - drops a
 * character onto something that cannot answer at all, and it stands still.
 * That is exactly what happened to Hollis, Cutter and Ash.
 *
 * So there is a second paid model in between, from a different provider, picked
 * from the same fourteen-run test as the first: 14/14 clean intents, 291ms,
 * which was the fastest of everything measured. It costs about three and a half
 * times the preferred one and is still a quarter of what this world used to run
 * on, and it is only reached when the first is refusing.
 */
export const SECOND_CHOICE = 'openrouter/amazon/nova-micro-v1';

/**
 * Two tries at the preferred model before dropping to the free one.
 *
 * One is too few: a single blip on a paid model would move a character onto
 * the free router for that turn for no good reason. Many is too many, because
 * every retry is time a character spends standing in a room not answering
 * somebody, and the whole point is that it keeps talking.
 */
const TRIES = 2;

export function withFallback(preferred: string): ModelWithRetries[] {
  const wanted = preferred.trim();
  if (wanted === FREE_ROUTER || '' === wanted) {
    return [{ id: 'free', model: FREE_ROUTER, maxRetries: TRIES }];
  }
  const chain = [{ id: 'preferred', model: wanted, maxRetries: TRIES }];
  if (wanted !== SECOND_CHOICE) {
    chain.push({ id: 'second', model: SECOND_CHOICE, maxRetries: TRIES });
  }
  chain.push({ id: 'free', model: FREE_ROUTER, maxRetries: TRIES });
  return chain;
}
