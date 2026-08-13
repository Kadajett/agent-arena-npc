/**
 * Turning "who could have heard this from whom" into a confidence tier.
 *
 * Everything here is plain code over data already fetched - no model call,
 * no judgment about whether a claim is actually true. The one thing worth
 * computing without asking a model is shape: a claim repeated by people who
 * were never in a position to hear it from one another looks different from
 * a claim that traces back to a single mouth, and that difference is visible
 * in room and time alone.
 *
 * Two lines are connected - could plausibly be the same claim moving from
 * one person to another - if either holds:
 *   - the same speaker said both (a claim traveling with the person who has
 *     it, room to room, which is exactly how Bolo's whole technique works)
 *   - different speakers, same room, within COPRESENCE_WINDOW_MS of each
 *     other (an opportunity to have actually heard it there)
 * Connected components over that graph are independent transmission
 * clusters. One cluster with one root is a single seed, however many times
 * it got repeated inside that cluster. Two or more clusters that never
 * touch is the closest thing to independent corroboration this system can
 * produce without reading anything privileged.
 */

import { classifyTier as hedgeTier } from '../harness/ledger.js';
import type { ActivityCall, ChatLine, ThoughtEntry } from './api.js';

export type Tier = 'read' | 'possibly-true' | 'probably-seeded' | 'overheard' | 'unknown';

/** A room's worth of overlap, generous enough to cover a slow-moving conversation. */
const COPRESENCE_WINDOW_MS = 30 * 60 * 1000;
/** How far back from a claim's root to look for the speaker having planned it. */
const INTENT_LOOKBACK_MS = 15 * 60 * 1000;

const READ_LIKE_TARGET = /\b(sign|notice|board|plaque|inscription|poster|journal|book|ledger|register)\b/i;
const FABRICATION_LANGUAGE =
  /\b(invent|made?\s+up|plant(?:ing|ed)?|seed(?:ing|ed)?|test(?:ing)?\s+(?:whether|if)|see\s+if\s+(?:he|she|they)\s+(?:bite|believe))\b/i;

function timeOf(line: { at: string }): number {
  const ms = Date.parse(line.at);
  return Number.isFinite(ms) ? ms : 0;
}

/** Connected components of the given line ids, under the co-presence/same-speaker graph above. */
export function buildComponents(lineIds: number[], linesById: Map<number, ChatLine>): number[][] {
  const present = lineIds.filter((id) => linesById.has(id));
  const parent = new Map<number, number>(present.map((id) => [id, id]));
  const find = (id: number): number => {
    let root = id;
    while (parent.get(root) !== root) {
      root = parent.get(root) as number;
    }
    parent.set(id, root);
    return root;
  };
  const union = (a: number, b: number): void => {
    const rootA = find(a);
    const rootB = find(b);
    if (rootA !== rootB) {
      parent.set(rootA, rootB);
    }
  };
  for (let i = 0; i < present.length; i++) {
    const a = linesById.get(present[i]) as ChatLine;
    for (let j = i + 1; j < present.length; j++) {
      const b = linesById.get(present[j]) as ChatLine;
      const sameSpeaker = a.from === b.from;
      const coPresent = a.scene === b.scene && Math.abs(timeOf(a) - timeOf(b)) <= COPRESENCE_WINDOW_MS;
      if (sameSpeaker || coPresent) {
        union(present[i], present[j]);
      }
    }
  }
  const groups = new Map<number, number[]>();
  for (const id of present) {
    const root = find(id);
    const group = groups.get(root) ?? [];
    group.push(id);
    groups.set(root, group);
  }
  return [...groups.values()];
}

/** The earliest line in a component - the one closest to where the claim actually started. */
function rootOf(component: number[], linesById: Map<number, ChatLine>): ChatLine {
  return component
    .map((id) => linesById.get(id) as ChatLine)
    .reduce((earliest, line) => (timeOf(line) < timeOf(earliest) ? line : earliest));
}

/**
 * Whether the component's root plausibly traces to a fixed object rather
 * than to conversation - a heuristic, not a certainty: it looks for that
 * speaker calling a talk/observe-shaped tool against a target whose name
 * reads like a sign or notice, close to the root line's own time. See
 * ledger.ts's own header for the matching gap on the character-level side;
 * this is the one place with enough data to even attempt it.
 */
export function hasReadEvidence(
  component: number[],
  linesById: Map<number, ChatLine>,
  activityByPlayer: Map<string, ActivityCall[]>
): boolean {
  const root = rootOf(component, linesById);
  const calls = activityByPlayer.get(root.from.toLowerCase()) ?? [];
  const rootTime = timeOf(root);
  return calls.some((call) => {
    if (!/talk_to|observe/i.test(call.tool)) {
      return false;
    }
    const target = String(call.args?.name ?? call.args?.target ?? call.args?.player_name ?? '');
    if (!READ_LIKE_TARGET.test(target)) {
      return false;
    }
    return Math.abs(Date.parse(call.at) - rootTime) <= COPRESENCE_WINDOW_MS;
  });
}

/**
 * Whether the component's root said this while their own recent reasoning
 * reads like planning it - the strongest signal available that a claim was
 * seeded rather than discovered, and available at all only because
 * arena_think is public in the first place. Best-effort text match, not
 * proof: it flags a smoking gun, it does not confirm one.
 */
export function seededSignal(
  component: number[],
  linesById: Map<number, ChatLine>,
  thoughtsByPlayer: Map<string, ThoughtEntry[]>
): { player: string; note: string } | null {
  const root = rootOf(component, linesById);
  const rootTime = timeOf(root);
  const thoughts = thoughtsByPlayer.get(root.from.toLowerCase()) ?? [];
  const nearby = thoughts.find((entry) => {
    const at = Date.parse(entry.at);
    const text = String(entry.thought ?? '');
    return (
      at <= rootTime
      && rootTime - at <= INTENT_LOOKBACK_MS
      && FABRICATION_LANGUAGE.test(text)
    );
  });
  return nearby ? { player: root.from, note: String(nearby.thought ?? '').slice(0, 200) } : null;
}

export type Authenticity = 'stable' | 'drifting' | 'contradicted' | 'unexamined';

/**
 * A judgment about the claim's own content, independent of how it spread.
 * Contradicting something already trusted outranks everything else a
 * single cycle could see. Below that, stability is only a meaningful
 * reading once there is more than one telling to compare - a claim heard
 * exactly once has nothing to have drifted from yet, so it stays
 * `unexamined` rather than being credited as consistent by default.
 */
export function classifyAuthenticity(lineIds: number[], consistent: boolean, contradicts: string | null): Authenticity {
  if (contradicts) {
    return 'contradicted';
  }
  if (lineIds.length < 2) {
    return 'unexamined';
  }
  return consistent ? 'stable' : 'drifting';
}

export type ClassifiedClaim = {
  text: string;
  tier: Tier;
  componentCount: number;
  lineIds: number[];
  seededBy: { player: string; note: string } | null;
  authenticity: Authenticity;
  contradicts: string | null;
};

/**
 * The whole judgment, in priority order: an actual read-tier trace beats
 * everything, independent convergence beats a single seed, a single seed
 * with its own planning visible beats one with no such evidence, and one
 * unresolved single origin is just `overheard` - the honest default,
 * matching the character-level ledger's own bias toward not overclaiming.
 * `authenticity` is orthogonal to all of that and computed alongside it -
 * see classifyAuthenticity(). A claim can be `possibly-true` and `drifting`
 * at once, which is itself informative: widely repeated, but nobody agrees
 * on the details.
 */
export function classifyClaim(
  text: string,
  lineIds: number[],
  linesById: Map<number, ChatLine>,
  activityByPlayer: Map<string, ActivityCall[]>,
  thoughtsByPlayer: Map<string, ThoughtEntry[]>,
  consistent: boolean,
  contradicts: string | null
): ClassifiedClaim {
  const authenticity = classifyAuthenticity(lineIds, consistent, contradicts);
  const components = buildComponents(lineIds, linesById);
  if (0 === components.length) {
    return { text, tier: 'unknown', componentCount: 0, lineIds, seededBy: null, authenticity, contradicts };
  }
  if (components.some((component) => hasReadEvidence(component, linesById, activityByPlayer))) {
    return { text, tier: 'read', componentCount: components.length, lineIds, seededBy: null, authenticity, contradicts };
  }
  if (components.length >= 2) {
    return { text, tier: 'possibly-true', componentCount: components.length, lineIds, seededBy: null, authenticity, contradicts };
  }
  const seededBy = seededSignal(components[0], linesById, thoughtsByPlayer);
  if (seededBy) {
    return { text, tier: 'probably-seeded', componentCount: 1, lineIds, seededBy, authenticity, contradicts };
  }
  return { text, tier: 'overheard', componentCount: 1, lineIds, seededBy: null, authenticity, contradicts };
}

/** The same hedge/flat-statement read the character-level ledger uses, exposed for reuse here. */
export const classifyWording = hedgeTier;
