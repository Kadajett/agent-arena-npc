// Lint-and-spot-fix for lore books. Splits a markdown book into frontmatter
// and body, checks each prose paragraph against a voice profile's ban list,
// and re-renders ONLY the paragraphs that trip a ban. Clean prose passes
// through byte-identical; every fix is cached by content hash so re-runs are
// free. Nothing is written in place: output goes to --out (or stdout diff
// with --check).
//
//   node scripts/render-lore.mjs --profile voices/lore-prose.yaml \
//     --out /tmp/fixed.md [--check] <book.md>
//
// Needs OPENROUTER_API_KEY. Model via LORE_MODEL (default deepseek flash).
import * as fs from "node:fs";
import * as crypto from "node:crypto";
import { DatabaseSync } from "node:sqlite";

const args = process.argv.slice(2);
const opt = (name, dflt) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : dflt;
};
const checkOnly = args.includes("--check");
const profilePath = opt("--profile", "voices/lore-prose.yaml");
const outPath = opt("--out", null);
const file = args.filter((a) => !a.startsWith("--") && a !== opt("--profile", "") && a !== outPath).pop();
if (!file) { console.error("usage: render-lore.mjs [--check] [--profile p.yaml] [--out out.md] book.md"); process.exit(1); }

// Same minimal YAML subset as the extension.
function parseProfile(text) {
  const p = { exemplars: [], ban: [], avoid: [], style: "", max_sentences: 0, max_words: 60 };
  let cur = null;
  for (const raw of text.split("\n")) {
    const line = raw.trimEnd();
    if (!line.trim() || line.trim().startsWith("#")) continue;
    const item = /^\s+-\s+(.*)$/.exec(line);
    if (item && cur) {
      let v = item[1].trim();
      if ((v.startsWith('"') && v.endsWith('"')) || (v.startsWith("'") && v.endsWith("'"))) v = v.slice(1, -1);
      p[cur].push(v);
      continue;
    }
    const kv = /^(\w+):\s*(.*)$/.exec(line);
    if (!kv) continue;
    const [, k, val] = kv;
    if (k === "exemplars" || k === "ban" || k === "avoid") cur = k;
    else if (k === "style") { let s = val.trim(); if (s.startsWith('"') && s.endsWith('"')) s = s.slice(1, -1); p.style = s; cur = null; }
    else if (k === "max_sentences") { p.max_sentences = Number(val) || 0; cur = null; }
    else if (k === "max_words") { p.max_words = Number(val) || 60; cur = null; }
  }
  return p;
}

const profileText = fs.readFileSync(profilePath, "utf-8");
const profile = parseProfile(profileText);
const profileVersion = crypto.createHash("sha256").update(profileText).digest("hex").slice(0, 8);
const bans = profile.ban.map((b) => new RegExp(b, "i"));
const model = process.env.LORE_MODEL || "deepseek/deepseek-v4-flash-0731";

const lint = (text) => {
  const hits = [];
  bans.forEach((re, i) => { const m = re.exec(text); if (m) hits.push({ rule: i, match: m[0] }); });
  const long = text.split(/(?<=[.!?])\s+/).find((s) => s.split(/\s+/).length > profile.max_words);
  if (long) hits.push({ rule: "max_words", match: long.slice(0, 40) });
  return hits;
};

const db = new DatabaseSync(opt("--cache", "var/lore-render-cache.sqlite"));
db.exec(`CREATE TABLE IF NOT EXISTS cache (key TEXT PRIMARY KEY, text TEXT NOT NULL)`);

async function render(paragraph, hits) {
  const key = crypto.createHash("sha256").update(`${profileVersion}:${paragraph}`).digest("hex");
  const hit = db.prepare(`SELECT text FROM cache WHERE key = ?`).get(key);
  if (hit) return hit.text;
  const system =
    "You edit one paragraph of in-world fiction. Rewrite it to remove the flagged mannerisms while keeping every fact, name, number, event, the original voice and tense. " +
    "NEVER invent names, numbers, dates, places, families, or events that are not in the original: this is canon and invention corrupts it. When a flagged sentence carries no concrete fact, delete it or shrink it; shorter is always acceptable. " +
    "The game world is the only world: no real-world references. Output ONLY the rewritten paragraph.\n" +
    profile.style + "\nGood prose in this register:\n" + profile.exemplars.map((e) => `- ${e}`).join("\n") +
    "\nNever write like:\n" + profile.avoid.map((e) => `- ${e}`).join("\n");
  const user = `Flagged mannerisms: ${hits.map((h) => JSON.stringify(h.match)).join(", ")}\n\nParagraph:\n${paragraph}`;
  for (let i = 0; i < 2; i++) {
    try {
      const res = await fetch("https://openrouter.ai/api/v1/chat/completions", {
        method: "POST",
        headers: { authorization: `Bearer ${process.env.OPENROUTER_API_KEY}`, "content-type": "application/json" },
        body: JSON.stringify({
          model, reasoning: { enabled: false }, temperature: 0.3, max_tokens: 700,
          messages: [{ role: "system", content: system }, { role: "user", content: user }],
        }),
        signal: AbortSignal.timeout(20_000),
      });
      const out = (await res.json())?.choices?.[0]?.message?.content?.trim();
      if (out && lint(out).length === 0) {
        db.prepare(`INSERT OR REPLACE INTO cache (key, text) VALUES (?, ?)`).run(key, out);
        return out;
      }
    } catch { /* retry */ }
  }
  return null; // unfixable: leave the original, report it
}

const src = fs.readFileSync(file, "utf-8");
// Frontmatter stays untouched.
const fm = /^---\n[\s\S]*?\n---\n/.exec(src);
const head = fm ? fm[0] : "";
const body = src.slice(head.length);

const blocks = body.split(/\n\n/);
let flagged = 0, fixed = 0, unfixable = 0;
const outBlocks = [];
for (const block of blocks) {
  const trimmed = block.trim();
  // Only prose: skip headings, html comments, tables, lists, short fragments.
  const isProse = trimmed.length > 80 && !/^[#>|\-*<`]/.test(trimmed);
  const hits = isProse ? lint(trimmed) : [];
  if (!hits.length) { outBlocks.push(block); continue; }
  flagged++;
  console.error(`FLAG [${hits.map((h) => h.match).join(" | ")}]  ${trimmed.slice(0, 70)}...`);
  if (checkOnly) { outBlocks.push(block); continue; }
  const replacement = await render(trimmed, hits);
  if (replacement) { fixed++; outBlocks.push(block.replace(trimmed, replacement)); }
  else { unfixable++; outBlocks.push(block); }
}

const result = head + outBlocks.join("\n\n");
if (outPath) fs.writeFileSync(outPath, result);
else if (!checkOnly) process.stdout.write(result);
console.error(`${file}: ${blocks.length} blocks, ${flagged} flagged, ${fixed} fixed, ${unfixable} left as-is`);
