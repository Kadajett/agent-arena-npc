/**
 * The voice renderer. The agent produces whatever free-form text it likes;
 * this re-renders it into the character's register at the single point where
 * speech leaves the harness (the speech tool call), with a mechanical check,
 * a content-addressed cache, and a deterministic fallback. The turn pipeline
 * is otherwise untouched.
 *
 *   payload -> smallModel(voicePrefix, payload) -> check() -> world
 *                    |  (2 attempts)                 |
 *                 cache hit? return              violation -> retry,
 *                                                then truncate fallback
 *
 * Config (per character, via the conf):
 *   NPC_VOICE_PROFILE   voices/<name>.yaml; unset = renderer off
 *   NPC_VOICE_MODEL     small model for the re-render (openrouter id)
 *   NPC_SPEECH_TOOLS    regex of tool references to wrap
 *
 * The profile is four fields: exemplars, ban, max_sentences, max_words.
 * Cache keys include a hash of the profile file, so editing the YAML
 * invalidates that voice only. /voice shows counts and reject rate.
 */
import * as crypto from "node:crypto";
import * as fs from "node:fs";
import * as path from "node:path";
import { DatabaseSync } from "node:sqlite";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

interface Profile {
  exemplars: string[];
  ban: string[];
  avoid: string[];
  style: string;
  max_sentences: number;
  max_words: number;
}

/** Minimal YAML for exactly this schema: scalars and string lists. */
function parseProfile(text: string): Profile {
  const p: Profile = { exemplars: [], ban: [], avoid: [], style: "", max_sentences: 2, max_words: 30 };
  let current: "exemplars" | "ban" | "avoid" | null = null;
  for (const raw of text.split("\n")) {
    const line = raw.replace(/#.*$/, (m, off) => (raw.slice(0, off).includes('"') ? m : "")).trimEnd();
    if (!line.trim()) continue;
    const item = /^\s+-\s+(.*)$/.exec(line);
    if (item && current) {
      let v = item[1].trim();
      if ((v.startsWith('"') && v.endsWith('"')) || (v.startsWith("'") && v.endsWith("'")))
        v = v.slice(1, -1);
      p[current].push(v);
      continue;
    }
    const kv = /^(\w+):\s*(.*)$/.exec(line);
    if (!kv) continue;
    const [, key, value] = kv;
    if (key === "exemplars" || key === "ban" || key === "avoid") current = key;
    else if (key === "style") {
      let v = value.trim();
      if ((v.startsWith('"') && v.endsWith('"')) || (v.startsWith("'") && v.endsWith("'"))) v = v.slice(1, -1);
      p.style = v; current = null;
    }
    else if (key === "max_sentences") { p.max_sentences = Number(value) || 2; current = null; }
    else if (key === "max_words") { p.max_words = Number(value) || 30; current = null; }
  }
  return p;
}

const sentences = (t: string) => t.split(/(?<=[.!?])\s+/).map((s) => s.trim()).filter(Boolean);

function check(out: string, p: Profile, bans: RegExp[]): string[] {
  const v: string[] = [];
  const ss = sentences(out);
  if (ss.length > p.max_sentences) v.push("max_sentences");
  if (ss.some((s) => s.split(/\s+/).filter(Boolean).length > p.max_words)) v.push("max_words");
  bans.forEach((re, i) => { if (re.test(out)) v.push(`ban[${i}]`); });
  return v;
}

const truncate = (payload: string, maxSentences: number) =>
  sentences(payload).slice(0, maxSentences).join(" ");

export default function (pi: ExtensionAPI) {
  const profilePath = process.env.NPC_VOICE_PROFILE;
  if (!profilePath) return; // renderer off

  const voiceId = path.basename(profilePath).replace(/\.ya?ml$/, "");
  const profileText = fs.readFileSync(profilePath, "utf-8");
  const profileVersion = crypto.createHash("sha256").update(profileText).digest("hex").slice(0, 8);
  const profile = parseProfile(profileText);
  const bans = profile.ban.map((b) => new RegExp(b, "i"));
  const model = (process.env.NPC_VOICE_MODEL || "qwen/qwen3.7-flash").replace(/^openrouter\//, "");
  // The extractor needs better instruction-following than the cheapest tier;
  // it also serves as the render fallback when the primary is rate-limited.
  const extractModel = (process.env.NPC_VOICE_EXTRACT_MODEL || "deepseek/deepseek-v4-flash-0731").replace(/^openrouter\//, "");
  const speechTools = new RegExp(process.env.NPC_SPEECH_TOOLS || "arena_say|arena_talk_to", "i");
  const twoStage = !/^(0|false|off)$/i.test(process.env.NPC_VOICE_TWO_STAGE ?? "");

  const dir = process.env.NPC_MEMORY_DIR || "./var";
  fs.mkdirSync(dir, { recursive: true });
  const db = new DatabaseSync(path.join(dir, `${process.env.NPC_CHARACTER || "npc"}-voice.sqlite`));
  db.exec(`CREATE TABLE IF NOT EXISTS voice_cache (key TEXT PRIMARY KEY, text TEXT NOT NULL);
           CREATE TABLE IF NOT EXISTS voice_log (at INTEGER, turn INTEGER, rule TEXT)`);

  let turn = 0;
  const stats = { rendered: 0, cacheHits: 0, fallbacks: 0, violations: 0 };
  const logViolation = (rule: string) => {
    stats.violations++;
    db.prepare(`INSERT INTO voice_log (at, turn, rule) VALUES (?, ?, ?)`).run(Date.now(), turn, rule);
  };

  const voicePrefix = () =>
    "You rewrite one line of game dialogue into a specific character voice. " +
    "Output ONLY the rewritten line, no quotes, no commentary. Keep every name, number, @mention, and concrete fact from the payload exactly. " +
    "The game world is the only world: no real-world places, people, companies, works, history, or wordplay on a name as if it were a real-world thing. Every name is a person standing in the room. " +
    "If the payload contains metaphor, riddles, or omen-talk, TRANSLATE it into plain literal statements in the voice; never preserve the metaphor itself. " +
    `Hard limits: at most ${profile.max_sentences} sentences, at most ${profile.max_words} words per sentence. ` +
    (profile.style || "Plain literal speech: no archaisms, no omens, no riddles, no metaphor.") + "\n" +
    "The voice, by example:\n" + profile.exemplars.map((e) => `- ${e}`).join("\n") +
    (profile.avoid.length
      ? "\nNever produce lines like these (negative examples):\n" + profile.avoid.map((e) => `- ${e}`).join("\n")
      : "");

  async function smallModel(useModel: string, system: string, user: string): Promise<string | null> {
    try {
      const res = await fetch("https://openrouter.ai/api/v1/chat/completions", {
        method: "POST",
        headers: {
          authorization: `Bearer ${process.env.OPENROUTER_API_KEY}`,
          "content-type": "application/json",
        },
        body: JSON.stringify({
          model: useModel,
          // Reasoning models burn the whole budget thinking and return null
          // content; a voice re-render needs none of it.
          reasoning: { enabled: false },
          messages: [
            { role: "system", content: system },
            { role: "user", content: user },
          ],
          max_tokens: 200,
          temperature: 0.4,
        }),
        signal: AbortSignal.timeout(12_000),
      });
      if (!res.ok) return null;
      const data = (await res.json()) as any;
      const text = data?.choices?.[0]?.message?.content?.trim();
      return text || null;
    } catch {
      return null;
    }
  }

  const EXTRACT_PROMPT =
    "You strip one line of game dialogue down to its verifiable payload. Output exactly three bullet groups:\n" +
    "- addressee: @Name, or none\n" +
    "- facts: concrete, correctly attributed statements a referee could check in the game world (WHO did WHAT, items, places, prices, numbers, proposed actions). " +
    "A sentence about abstract nouns (stories, truth, steel, memory, debts, fate) that names no specific person, object, place, price, or action is an aphorism: omit it silently.\n" +
    "- intent: question / answer / offer / threat / greeting / boast / refusal / acknowledgement\n" +
    "Every name is a person inside the game world and nothing else: never interpret a name as a real-world city, country, brand, book, or person. " +
    "Always output all three groups even when facts is empty. Never explain, never mention omitting.";

  async function renderVoice(payload: string): Promise<string> {
    const key = crypto.createHash("sha256").update(`${voiceId}:${profileVersion}:${payload}`).digest("hex");
    const hit = db.prepare(`SELECT text FROM voice_cache WHERE key = ?`).get(key) as
      | { text: string }
      | undefined;
    if (hit) { stats.cacheHits++; return hit.text; }

    // Stage one: intent plus facts. The agent's prose never reaches the
    // voice model, so the room's register cannot ride through on it.
    let source = payload;
    if (twoStage) {
      const facts =
        (await smallModel(extractModel, EXTRACT_PROMPT, payload)) ??
        (await smallModel(model, EXTRACT_PROMPT, payload));
      if (facts) source = facts;
    }

    const renderSystem =
      voicePrefix() +
      (twoStage
        ? "\nYou receive addressee, facts, and intent as bullets. Write ONE spoken line delivering exactly that. " +
          "If facts is empty, give the addressee a brief in-voice acknowledgement. Never mention bullets, facts, or instructions."
        : "");
    for (let i = 0; i < 2; i++) {
      const out =
        (await smallModel(model, renderSystem, source)) ??
        (await smallModel(extractModel, renderSystem, source));
      if (!out) break;
      const v = check(out, profile, bans);
      if (!v.length) {
        db.prepare(`INSERT OR REPLACE INTO voice_cache (key, text) VALUES (?, ?)`).run(key, out);
        stats.rendered++;
        return out;
      }
      v.forEach(logViolation);
    }
    stats.fallbacks++;
    return truncate(payload, profile.max_sentences); // deterministic fallback
  }

  pi.on("agent_start", async () => { turn++; });

  pi.on("tool_call", async (event) => {
    const input = event.input as Record<string, unknown>;
    const raw = JSON.stringify(input ?? {});
    if (!speechTools.test(raw) && !speechTools.test(event.toolName)) return undefined;

    // The one wrap point: mutate the spoken fields in place, let the call run.
    const rewrite = async (obj: Record<string, unknown>) => {
      for (const [k, v] of Object.entries(obj)) {
        if (/^(message|text|line|say)$/i.test(k) && typeof v === "string" && v.trim())
          obj[k] = await renderVoice(v);
        else if (v && typeof v === "object" && !Array.isArray(v))
          await rewrite(v as Record<string, unknown>);
      }
    };
    await rewrite(input ?? {});
    return undefined;
  });

  pi.registerCommand("voice", {
    description: "Voice renderer stats and recent violations",
    handler: async (_args, ctx) => {
      const recent = db
        .prepare(`SELECT rule, COUNT(*) n FROM voice_log GROUP BY rule ORDER BY n DESC LIMIT 10`)
        .all() as unknown as Array<{ rule: string; n: number }>;
      const total = stats.rendered + stats.fallbacks;
      ctx.ui.notify(
        `voice=${voiceId}@${profileVersion} model=${model}\n` +
          `rendered=${stats.rendered} cacheHits=${stats.cacheHits} fallbacks=${stats.fallbacks} ` +
          `rejectRate=${total ? ((stats.violations / (total * 2)) * 100).toFixed(0) : 0}% turn=${turn}\n` +
          (recent.length ? recent.map((r) => `${r.rule}: ${r.n}`).join("\n") : "no violations logged"),
        "info"
      );
    },
  });

  pi.on("session_shutdown", async () => { db.close(); });
}
