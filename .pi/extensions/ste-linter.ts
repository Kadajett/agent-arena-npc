/**
 * The speech gate. Style instructions lose to imitation: a model surrounded
 * by omen-speak will drift into omen-speak no matter what the system prompt
 * says. This extension stops asking and starts enforcing: every speech tool
 * call is linted before it executes, and a violating line is blocked with a
 * reason the model reads and must fix. The world only ever hears lines that
 * passed.
 *
 * Configuration (all optional, all per character via the conf):
 *   NPC_STE_MAX_WORDS      max words per sentence (default 20; 0 disables
 *                          the linter entirely)
 *   NPC_SPEECH_BANNED      regex of banned phrases (case-insensitive);
 *                          set empty to disable the phrase check
 *   NPC_SPEECH_TOOLS       regex of tool references to lint
 *                          (default arena_say|arena_talk_to)
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

const DEFAULT_BANNED =
  "\\bomens?\\b|\\bashes?\\b|\\bcrypt\\b|waits? for (its|the)|what do you (read|hear) in|the \\w+ and I (disagree|share)|that('s| is) not a \\w+, that('s| is)|\\bthe count\\b";

export default function (pi: ExtensionAPI) {
  const maxWords = process.env.NPC_STE_MAX_WORDS === undefined ? 20 : Number(process.env.NPC_STE_MAX_WORDS);
  if (!maxWords) return; // linter off
  const banned =
    process.env.NPC_SPEECH_BANNED === undefined
      ? new RegExp(DEFAULT_BANNED, "i")
      : process.env.NPC_SPEECH_BANNED
        ? new RegExp(process.env.NPC_SPEECH_BANNED, "i")
        : null;
  const speechTools = new RegExp(process.env.NPC_SPEECH_TOOLS || "arena_say|arena_talk_to", "i");

  const collectStrings = (value: unknown, out: string[]): void => {
    if (typeof value === "string") out.push(value);
    else if (Array.isArray(value)) for (const v of value) collectStrings(v, out);
    else if (value && typeof value === "object")
      for (const [k, v] of Object.entries(value)) {
        if (/^(message|text|line|say)$/i.test(k) && typeof v === "string") out.push(v);
        else collectStrings(v, out);
      }
  };

  const lint = (text: string): string | null => {
    const sentences = text
      .split(/(?<=[.!?])\s+/)
      .map((s) => s.trim())
      .filter(Boolean);
    for (const s of sentences) {
      const words = s.split(/\s+/).filter(Boolean).length;
      if (words > maxWords)
        return `sentence "${s.slice(0, 60)}..." has ${words} words (max ${maxWords})`;
    }
    if (banned) {
      const hit = banned.exec(text);
      if (hit) return `contains banned phrasing "${hit[0]}"`;
    }
    return null;
  };

  pi.on("tool_call", async (event) => {
    const raw = JSON.stringify(event.input ?? {});
    if (!speechTools.test(raw) && !speechTools.test(event.toolName)) return undefined;

    // Only lint the fields that carry the spoken line.
    const candidates: string[] = [];
    const input = event.input as Record<string, unknown>;
    for (const [k, v] of Object.entries(input ?? {})) {
      if (/^(message|text|line|say)$/i.test(k) && typeof v === "string") candidates.push(v);
      else if (v && typeof v === "object") {
        const nested: string[] = [];
        collectStrings(v, nested);
        for (const n of nested) candidates.push(n);
      }
    }

    for (const text of candidates) {
      const problem = lint(text);
      if (problem)
        return {
          block: true,
          reason:
            `Speech rejected: ${problem}. Rewrite in Simplified Technical English: ` +
            `short literal sentences, one fact each, no metaphors, then send again.`,
        };
    }
    return undefined;
  });
}
