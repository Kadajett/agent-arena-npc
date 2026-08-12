/**
 * Character memory on node:sqlite. One file per character on the memory
 * volume, three tools (remember, recall, forget), and the part a memory MCP
 * server cannot do: the strongest memories are injected into the system
 * prompt before every turn, so a restarted character wakes up knowing who it
 * has met rather than hoping it thinks to ask.
 */
import * as fs from "node:fs";
import * as path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { Type } from "typebox";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

const KINDS = ["person", "place", "fact", "event", "feeling", "goal", "promise"] as const;
const PROMPT_BUDGET = 40; // memories carried into every prompt

export default function (pi: ExtensionAPI) {
  const dir = process.env.NPC_MEMORY_DIR || "./var";
  const character = process.env.NPC_CHARACTER || "npc";
  fs.mkdirSync(dir, { recursive: true });
  const db = new DatabaseSync(path.join(dir, `${character}-memory.sqlite`));
  db.exec(`
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

  const ok = (text: string) => ({ content: [{ type: "text" as const, text }], details: undefined });

  // "@Name" is chat routing, not part of anyone's name. Memory stores the
  // person, never the envelope.
  const deAt = (s: string) => s.replace(/@(?=\w)/g, "").trim();

  const rows = (sql: string, ...params: (string | number)[]) =>
    db.prepare(sql).all(...params) as unknown as Array<{
      id: number; kind: string; subject: string; body: string; last_seen: number;
    }>;

  const line = (r: { id: number; kind: string; subject: string; body: string }) =>
    `- [${r.id}] (${r.kind}) ${r.subject}: ${r.body}`;

  pi.registerTool({
    name: "remember",
    label: "Remember",
    description:
      "Keep something for good: a person and how you feel about them, a place, " +
      "a fact, an event, a promise you made, a goal. Repeating a memory " +
      "strengthens it.",
    promptSnippet: "remember: store a lasting memory (person, place, fact, event, feeling, goal, promise)",
    parameters: Type.Object({
      kind: Type.Union(KINDS.map((k) => Type.Literal(k))),
      subject: Type.String({ description: "Who or what this is about, e.g. 'Barnaby'" }),
      body: Type.String({ description: "The thing to remember, one sentence" }),
    }),
    async execute(_id, p) {
      const now = Date.now();
      db.prepare(
        `INSERT INTO memory (kind, subject, body, weight, created_at, last_seen)
         VALUES (?, ?, ?, 1, ?, ?)
         ON CONFLICT(kind, subject, body)
         DO UPDATE SET weight = weight + 1, last_seen = excluded.last_seen`
      ).run(p.kind, deAt(p.subject), deAt(p.body), now, now);
      return ok(`Remembered: (${p.kind}) ${deAt(p.subject)}: ${deAt(p.body)}`);
    },
  });

  pi.registerTool({
    name: "recall",
    label: "Recall",
    description:
      "Search your memory for a person, place, or topic. Use it before " +
      "claiming you do or do not know someone.",
    promptSnippet: "recall: search your long-term memory",
    parameters: Type.Object({
      query: Type.String({ description: "Name or topic to look up" }),
    }),
    async execute(_id, p) {
      const q = `%${deAt(p.query)}%`;
      const found = rows(
        `SELECT id, kind, subject, body, last_seen FROM memory
         WHERE subject LIKE ? OR body LIKE ?
         ORDER BY last_seen DESC LIMIT 25`,
        q, q
      );
      if (found.length === 0) return ok(`Nothing in memory about "${p.query}".`);
      db.prepare(
        `UPDATE memory SET last_seen = ? WHERE id IN (${found.map(() => "?").join(",")})`
      ).run(Date.now(), ...found.map((r) => r.id));
      return ok(found.map(line).join("\n"));
    },
  });

  pi.registerTool({
    name: "forget",
    label: "Forget",
    description: "Delete a memory by its [id] when it turns out to be wrong.",
    parameters: Type.Object({ id: Type.Number() }),
    async execute(_id, p) {
      db.prepare(`DELETE FROM memory WHERE id = ?`).run(p.id);
      return ok(`Forgot memory ${p.id}.`);
    },
  });

  pi.on("before_agent_start", async (event) => {
    const top = rows(
      `SELECT id, kind, subject, body, last_seen FROM memory
       ORDER BY weight DESC, last_seen DESC LIMIT ${PROMPT_BUDGET}`
    );
    if (top.length === 0) return undefined;
    return {
      systemPrompt:
        event.systemPrompt +
        "\n\n## What you remember\n" +
        top.map(line).join("\n") +
        "\nUse recall for anything not listed here before saying you do not know it.",
    };
  });

  pi.registerCommand("memory", {
    description: "Show what the character remembers (/memory [query])",
    handler: async (args, ctx) => {
      const q = args.trim();
      const found = q
        ? rows(
            `SELECT id, kind, subject, body, last_seen FROM memory
             WHERE subject LIKE ? OR body LIKE ? ORDER BY last_seen DESC LIMIT 50`,
            `%${q}%`, `%${q}%`
          )
        : rows(
            `SELECT id, kind, subject, body, last_seen FROM memory
             ORDER BY last_seen DESC LIMIT 50`
          );
      ctx.ui.notify(found.length ? found.map(line).join("\n") : "Memory is empty.", "info");
    },
  });

  pi.on("session_shutdown", async () => {
    db.close();
  });
}
