/**
 * The one model call in the whole pipeline: turning raw, numbered chat lines
 * into distinct claims, each carrying the ids of every line that expressed
 * it. Everything downstream of this - the propagation graph, the tier a
 * claim gets - is plain code over that grouping. Clustering paraphrases
 * ("the books don't add up" / "his numbers are off") is the one job here
 * that genuinely needs a model; matching them by room and time does not.
 *
 * Two more things ride along on the same call, because both are judgments
 * about the claim's own content rather than how it moved through the room,
 * and neither is worth a second round trip: whether every line grouped into
 * a claim actually agrees on its specifics (a real fact stays the same
 * telling to telling; an invented one drifts - a name changes, a number
 * changes), and whether the claim directly contradicts something already
 * sitting at a higher tier. Both are content judgments a model has to make;
 * propagation shape, which is the other half of confidence, is deliberately
 * left to graph.ts instead - it needs no judgment at all.
 */

import { Agent } from '@mastra/core/agent';
import { z } from 'zod';
import { withFallback } from '../harness/models.js';
import type { ChatLine } from './api.js';

const EXTRACT_BUDGET = { maxOutputTokens: 2560 };
const DEFAULT_MODEL = 'openrouter/deepseek/deepseek-v4-flash';

const ClaimsSchema = z.object({
  claims: z
    .array(
      z.object({
        text: z.string().min(1).max(300).describe('The claim, restated once, plainly - not a quote.'),
        lineIds: z
          .array(z.number().int())
          .min(1)
          .describe('Every numbered line that expresses this same claim, however differently worded.'),
        consistent: z
          .boolean()
          .describe(
            'True only if every grouped line agrees on the specifics - names, numbers, places. '
              + 'False if any of them differ on a detail the others gave, even if the general shape '
              + 'of the claim is the same. Meaningless with only one line grouped in; answer true in '
              + 'that case, the caller ignores it either way.'
          ),
        contradicts: z
          .string()
          .max(300)
          .nullable()
          .describe(
            'The exact text of one claim from the "already established" list below that this new '
              + 'claim directly conflicts with, or null if it does not conflict with any of them.'
          )
      })
    )
    .max(60)
});

function safeJson(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

export type ExtractedClaim = {
  text: string;
  lineIds: number[];
  consistent: boolean;
  contradicts: string | null;
};

/**
 * Group a batch of world chat into distinct claims, judged against whatever
 * is already trusted. `establishedClaims` should be the current high-tier
 * claims (read / possibly-true) - kept short on purpose, it is reference
 * material for spotting a conflict, not a second transcript to cluster.
 * Returns [] on anything short of a clean parse - an aggregator cycle that
 * found nothing usable is a cycle that tries again next time, not one that
 * invents structure to have something to show.
 */
export async function extractClaims(lines: ChatLine[], establishedClaims: string[] = []): Promise<ExtractedClaim[]> {
  if (0 === lines.length) {
    return [];
  }
  const agent = new Agent({
    id: 'worldledger-extract',
    name: 'World Ledger',
    instructions: [
      'You are sorting a transcript of things said across a shared game world',
      'into distinct claims about the world - not everything said, only lines',
      'that assert or relay something as a fact someone could later check.',
      'Small talk, greetings, and pure action narration are not claims and',
      'should not appear at all.',
      '',
      'Group lines that express the same underlying claim together, even when',
      'worded very differently or hedged ("somebody said...", "I heard...").',
      'A claim restated by the same person twice, or relayed by somebody',
      'else, still belongs to one group. Restate each claim once, plainly, in',
      'your own words - do not quote, and do not add anything the lines',
      'themselves did not say.',
      '',
      'For each claim, say whether every grouped line actually agrees on its',
      'specifics. A retelling that changes a number, a name, or a place from',
      'what an earlier line in the same group said is not consistent, even if',
      'the general shape of the claim matches.',
      '',
      'Also check the claim against what is already established below, and',
      'say if it directly contradicts one of those - not merely different,',
      'actually incompatible with it.',
      '',
      establishedClaims.length > 0
        ? `Already established, from earlier cycles:\n${establishedClaims.map((claim) => `- ${claim}`).join('\n')}`
        : 'Nothing is established yet from earlier cycles.',
      '',
      'Reply with JSON and nothing else:',
      '{"claims": [{"text": "...", "lineIds": [1, 2], "consistent": true, "contradicts": null}]}'
    ].join('\n'),
    model: withFallback(process.env.WORLDLEDGER_MODEL ?? DEFAULT_MODEL)
  });
  const numbered = lines.map((line) => `[${line.id}] ${line.from} (${line.scene}): ${line.message}`).join('\n');
  try {
    const response = await agent.generate(numbered, {
      toolChoice: 'none',
      modelSettings: { ...EXTRACT_BUDGET, temperature: 0.2 },
      abortSignal: AbortSignal.timeout(90_000)
    });
    const text = String((response as { text?: string }).text ?? '');
    const json = text.slice(text.indexOf('{'), text.lastIndexOf('}') + 1);
    const parsed = ClaimsSchema.safeParse(safeJson(json));
    if (!parsed.success) {
      return [];
    }
    const validIds = new Set(lines.map((line) => line.id));
    return parsed.data.claims
      .map((claim) => ({
        text: claim.text.trim(),
        lineIds: claim.lineIds.filter((id) => validIds.has(id)),
        consistent: claim.consistent,
        contradicts: claim.contradicts?.trim() || null
      }))
      .filter((claim) => claim.text.length > 0 && claim.lineIds.length > 0);
  } catch {
    return [];
  }
}
