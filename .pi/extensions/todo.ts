/**
 * A persistent, prioritized task list. Cheap models drift into whatever the
 * chat log is doing, so the list is injected at the NEWEST edge of context:
 * autonomy.ts appends it to every tick message (via the render hook below),
 * which puts it after the chat noise instead of buried in the system prompt.
 *
 * One sentence per task, ordered by priority (1 = most urgent). The list
 * survives restarts on the memory volume next to the character's memories.
 *
 *   /todo                      show the list
 *   /todo add [1-5] <task>     add a task (default priority 3)
 *   /todo done <id>            clear a task
 *
 * Environment: NPC_TODO_LIMIT caps how many tasks ride along per tick.
 */
import * as fs from "node:fs";
import * as path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { Type } from "typebox";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

const SHOW = Number(process.env.NPC_TODO_LIMIT) || 7;

export default function (pi: ExtensionAPI) {
  if (/^(0|false|off)$/i.test(process.env.NPC_TODO_ENABLED ?? "")) return;
  const dir = process.env.NPC_MEMORY_DIR || "./var";
  const character = process.env.NPC_CHARACTER || "npc";
  fs.mkdirSync(dir, { recursive: true });
  const db = new DatabaseSync(path.join(dir, `${character}-todo.sqlite`));
  db.exec(`
    CREATE TABLE IF NOT EXISTS todo (
      id INTEGER PRIMARY KEY,
      task TEXT NOT NULL UNIQUE,
      priority INTEGER NOT NULL DEFAULT 3,
      created_at INTEGER NOT NULL
    )
  `);

  const ok = (text: string) => ({ content: [{ type: "text" as const, text }], details: undefined });

  // "@Name" is chat routing, never part of a task.
  const deAt = (s: string) => s.replace(/@(?=\w)/g, "").trim();

  const all = () =>
    db.prepare(`SELECT id, task, priority FROM todo ORDER BY priority, created_at`).all() as unknown as Array<{
      id: number; task: string; priority: number;
    }>;

  const line = (t: { id: number; task: string; priority: number }) => `- [${t.id}] (p${t.priority}) ${t.task}`;

  const render = () => {
    const top = all().slice(0, SHOW);
    if (top.length === 0) {
      return (
        "## Task list\nYour task list is empty. Distill what you are up to into " +
        "todo_add tasks, one sentence each, before doing anything else."
      );
    }
    return (
      "## Task list, highest priority first\n" +
      top.map(line).join("\n") +
      "\nWork the top task. Chat that serves no task is a distraction: skip it " +
      "unless someone @-addresses you by name or it changes a task. Clear " +
      "finished tasks with todo_done."
    );
  };
  // Read by autonomy.ts at tick time, so extension load order never matters.
  (globalThis as { __npcTodoRender?: () => string }).__npcTodoRender = render;

  pi.registerTool({
    name: "todo_add",
    label: "Add task",
    description:
      "Put a task on your list, one sentence, with a priority from 1 (most " +
      "urgent) to 5. Adding an existing task again just changes its priority.",
    promptSnippet: "todo_add: add a one-sentence task to your list (priority 1-5)",
    parameters: Type.Object({
      task: Type.String({ description: "The task, one sentence" }),
      priority: Type.Integer({ minimum: 1, maximum: 5 }),
    }),
    async execute(_id, p) {
      const task = deAt(p.task);
      db.prepare(
        `INSERT INTO todo (task, priority, created_at) VALUES (?, ?, ?)
         ON CONFLICT(task) DO UPDATE SET priority = excluded.priority`
      ).run(task, p.priority, Date.now());
      return ok(`On the list (p${p.priority}): ${task}`);
    },
  });

  pi.registerTool({
    name: "todo_done",
    label: "Finish task",
    description: "Clear a task from your list by its [id], done or abandoned.",
    promptSnippet: "todo_done: clear a finished or abandoned task by [id]",
    parameters: Type.Object({ id: Type.Number() }),
    async execute(_id, p) {
      const gone = db.prepare(`DELETE FROM todo WHERE id = ?`).run(p.id);
      return ok(gone.changes ? `Cleared task ${p.id}.` : `No task ${p.id} on the list.`);
    },
  });

  pi.registerCommand("todo", {
    description: "Show or edit the task list (/todo [add [1-5] <task> | done <id>])",
    handler: async (args, ctx) => {
      const arg = args.trim();
      const add = /^add\s+(?:([1-5])\s+)?(.+)$/.exec(arg);
      const done = /^done\s+(\d+)$/.exec(arg);
      if (add) {
        const priority = Number(add[1] || 3);
        db.prepare(
          `INSERT INTO todo (task, priority, created_at) VALUES (?, ?, ?)
           ON CONFLICT(task) DO UPDATE SET priority = excluded.priority`
        ).run(deAt(add[2]), priority, Date.now());
        ctx.ui.notify(`Added (p${priority}): ${deAt(add[2])}`, "info");
      } else if (done) {
        db.prepare(`DELETE FROM todo WHERE id = ?`).run(Number(done[1]));
        ctx.ui.notify(`Cleared task ${done[1]}`, "info");
      } else if (arg === "") {
        const list = all();
        ctx.ui.notify(list.length ? list.map(line).join("\n") : "Task list is empty.", "info");
      } else {
        ctx.ui.notify("Usage: /todo [add [1-5] <task> | done <id>]", "warning");
      }
    },
  });

  pi.on("session_shutdown", async () => {
    db.close();
  });
}
