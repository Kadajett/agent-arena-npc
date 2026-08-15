// Convert a rust-harness character database into the pi harness memory
// schema. Reads working_memory JSON, semantic_memories, episode summaries,
// and relationships; writes rows the memory extension understands. Safe to
// re-run: rows are keyed UNIQUE(kind, subject, body).
//
//   node scripts/convert-memory.mjs <old.sqlite> <new.sqlite>
import { DatabaseSync } from "node:sqlite";

const [oldPath, newPath] = process.argv.slice(2);
if (!oldPath || !newPath) {
  console.error("usage: node convert-memory.mjs <old.sqlite> <new.sqlite>");
  process.exit(1);
}

const KIND_MAP = {
  person: "person",
  place: "place",
  going_on: "event",
  recent_memory: "event",
  own_business: "fact",
};

const src = new DatabaseSync(oldPath);
const dst = new DatabaseSync(newPath);
dst.exec(`
  CREATE TABLE IF NOT EXISTS memory (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    subject TEXT NOT NULL,
    body TEXT NOT NULL,
    weight REAL NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    last_seen INTEGER NOT NULL,
    UNIQUE(kind, subject, body)
  );
  CREATE INDEX IF NOT EXISTS memory_subject ON memory(subject);
`);

const now = Date.now();
const ts = (s) => {
  const t = Date.parse(s ?? "");
  return Number.isFinite(t) ? t : now;
};
const deAt = (s) => String(s ?? "").replace(/@(?=\w)/g, "").trim();
const insert = dst.prepare(
  `INSERT INTO memory (kind, subject, body, weight, created_at, last_seen)
   VALUES (?, ?, ?, ?, ?, ?)
   ON CONFLICT(kind, subject, body) DO UPDATE SET last_seen = MAX(last_seen, excluded.last_seen)`
);
let count = 0;
const put = (kind, subject, body, weight = 1, at = now) => {
  subject = deAt(subject).slice(0, 120);
  body = deAt(body).slice(0, 500);
  if (!subject || !body) return;
  insert.run(kind, subject, body, weight, at, at);
  count++;
};

// 1. Working memory: the goal, the list, the notes.
try {
  const row = src.prepare(`SELECT memory_json FROM working_memory LIMIT 1`).get();
  if (row) {
    const wm = JSON.parse(row.memory_json);
    if (wm.goal?.aim)
      put("goal", "current goal", wm.goal.aim + (wm.goal.why ? ` (why: ${wm.goal.why})` : ""), 3);
    if (wm.strategic_intent?.objective && wm.strategic_intent.objective !== wm.goal?.aim)
      put("goal", "strategic objective", wm.strategic_intent.objective, 2);
    for (const g of wm.strategic_intent?.subgoals ?? [])
      put("goal", "subgoal", typeof g === "string" ? g : JSON.stringify(g), 2);
    for (const t of wm.todo ?? [])
      put("promise", "todo", typeof t === "string" ? t : t.text ?? JSON.stringify(t), 2);
    for (const n of wm.notes ?? [])
      put("fact", "note", typeof n === "string" ? n : n.text ?? JSON.stringify(n));
    if (wm.progress_summary) put("fact", "progress so far", wm.progress_summary, 2);
  }
} catch (e) {
  console.error("working_memory skipped:", String(e).slice(0, 80));
}

// 2. Semantic memories: the closest cousin of the new schema.
try {
  for (const r of src
    .prepare(`SELECT kind, subject, summary, occurred_at, recorded_at FROM semantic_memories`)
    .all()) {
    const kind = KIND_MAP[r.kind] ?? "fact";
    // Subjects like "person-Barnaby" or "recent_memory-3" carry their kind as
    // a prefix; the remainder (or the summary's lead) is the real subject.
    let subject = String(r.subject ?? "").replace(/^[a-z_]+-/i, "");
    if (!subject || /^\d+$/.test(subject))
      subject = String(r.summary ?? "").split(/\s+/).slice(0, 4).join(" ");
    put(kind, subject, r.summary, kind === "person" ? 2 : 1, ts(r.occurred_at ?? r.recorded_at));
  }
} catch (e) {
  console.error("semantic_memories skipped:", String(e).slice(0, 80));
}

// 3. Episodes: most recent hundred, as events.
try {
  for (const r of src
    .prepare(
      `SELECT scene, summary, ended_at FROM episode_memories ORDER BY ended_at DESC LIMIT 100`
    )
    .all())
    put("event", r.scene || "episode", r.summary, 1, ts(r.ended_at));
} catch (e) {
  console.error("episode_memories skipped:", String(e).slice(0, 80));
}

// 4. Relationships: who they know and how they feel.
try {
  for (const r of src
    .prepare(`SELECT person_id, display_name, trust, opinion, last_updated FROM relationships`)
    .all()) {
    const name = r.display_name || r.person_id;
    const feeling = [
      r.opinion ? `opinion: ${r.opinion}` : "",
      r.trust !== null && r.trust !== undefined ? `trust ${r.trust}` : "",
    ]
      .filter(Boolean)
      .join(", ");
    if (feeling) put("person", name, feeling, 2, ts(r.last_updated));
  }
} catch (e) {
  console.error("relationships skipped:", String(e).slice(0, 80));
}

console.log(`converted ${count} rows -> ${newPath}`);
