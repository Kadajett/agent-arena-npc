/**
 * Self-improvement: standing orders and watchdogs.
 *
 * Standing orders are rules the character writes for itself ("if my health
 * drops below a third, break off and run for the town gate") . They persist in
 * SQLite on the memory volume and are injected into every prompt above the
 * conversation, so they are obeyed even when the model is mid-errand. The
 * model manages them with standing_order / drop_standing_order; a human can
 * do the same with /reflex.
 *
 * Watchdogs are the hard-wired layer underneath: regexes run against every
 * arena tool result. When one fires, the extension does not wait for the next
 * autonomy tick; it hands the agent an urgent turn immediately. Defaults
 * cover dying and low health; NPC_WATCHDOGS can replace them with a JSON
 * array of { name, pattern, whenBelow?, prompt } (whenBelow compares the
 * pattern's first numeric capture group).
 */
import * as fs from "node:fs";
import * as path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { Type } from "typebox";
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";

interface Watchdog {
  name: string;
  pattern: string;
  whenBelow?: number;
  prompt: string;
}

const DEFAULT_WATCHDOGS: Watchdog[] = [
  {
    name: "died",
    pattern: '"recentlyDied"\\s*:\\s*true',
    prompt:
      "(Reflex, not a player speaking.) You have just died and returned. Collect yourself: observe, note where you fell and to what, and act on your standing orders before anything else.",
  },
  {
    name: "low-health",
    pattern: '"(?:hp|health|currentLife|life)"\\s*:\\s*(\\d+)',
    whenBelow: Number(process.env.NPC_LOW_HEALTH_BELOW) || 30,
    prompt:
      "(Reflex, not a player speaking.) Your health is dangerously low. Follow your standing orders about danger NOW; if you have none, disengage and get somewhere safe before doing anything else.",
  },
];

function loadWatchdogs(): Watchdog[] {
  const raw = process.env.NPC_WATCHDOGS;
  if (!raw) return DEFAULT_WATCHDOGS;
  try {
    const parsed = JSON.parse(raw) as Watchdog[];
    return parsed.filter((w) => w?.name && w?.pattern && w?.prompt);
  } catch {
    return DEFAULT_WATCHDOGS;
  }
}

export default function (pi: ExtensionAPI) {
  const dir = process.env.NPC_MEMORY_DIR || "./var";
  const character = process.env.NPC_CHARACTER || "npc";
  fs.mkdirSync(dir, { recursive: true });
  const db = new DatabaseSync(path.join(dir, `${character}-reflexes.sqlite`));
  db.exec(`CREATE TABLE IF NOT EXISTS standing_order (
    id INTEGER PRIMARY KEY,
    orders TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL
  )`);

  const watchdogs = loadWatchdogs();
  const cooldownMs = (Number(process.env.NPC_WATCHDOG_COOLDOWN_SECONDS) || 60) * 1000;
  const lastFired = new Map<string, number>();
  let urgent: string | null = null;

  const list = () =>
    db.prepare(`SELECT id, orders FROM standing_order ORDER BY id`).all() as unknown as Array<{
      id: number;
      orders: string;
    }>;

  const ok = (text: string) => ({ content: [{ type: "text" as const, text }], details: undefined });

  // "@Name" is chat routing, never stored language.
  const deAt = (s: string) => s.replace(/@(?=\w)/g, "").trim();

  pi.registerTool({
    name: "standing_order",
    label: "Standing order",
    description:
      "Give yourself a permanent rule to follow from now on, e.g. 'if my health drops below a third, break off and run to the inn'. Written as a complete instruction to your future self.",
    promptSnippet: "standing_order: adopt a permanent rule for yourself",
    parameters: Type.Object({
      order: Type.String({ description: "The rule, one sentence, imperative" }),
    }),
    async execute(_id, p) {
      db.prepare(
        `INSERT OR IGNORE INTO standing_order (orders, created_at) VALUES (?, ?)`
      ).run(deAt(p.order), Date.now());
      return ok(`Standing order adopted: ${deAt(p.order)}`);
    },
  });

  pi.registerTool({
    name: "drop_standing_order",
    label: "Drop standing order",
    description: "Retire one of your standing orders by its [id] when it no longer serves you.",
    parameters: Type.Object({ id: Type.Number() }),
    async execute(_id, p) {
      db.prepare(`DELETE FROM standing_order WHERE id = ?`).run(p.id);
      return ok(`Standing order ${p.id} retired.`);
    },
  });

  pi.on("before_agent_start", async (event) => {
    const orders = list();
    if (orders.length === 0) return undefined;
    return {
      systemPrompt:
        event.systemPrompt +
        "\n\n## Standing orders\nWhen a standing order's condition is met, it outranks everything below it. Obey first, explain later.\n" +
        orders.map((o) => `- [${o.id}] ${o.orders}`).join("\n"),
    };
  });

  pi.on("tool_execution_end", async (event) => {
    if (event.isError) return;
    let text: string;
    try {
      text = typeof event.result === "string" ? event.result : JSON.stringify(event.result);
    } catch {
      return;
    }
    for (const w of watchdogs) {
      const m = new RegExp(w.pattern).exec(text);
      if (!m) continue;
      if (w.whenBelow !== undefined) {
        const value = Number(m[1]);
        if (!Number.isFinite(value) || value >= w.whenBelow) continue;
      }
      const last = lastFired.get(w.name) ?? 0;
      if (Date.now() - last < cooldownMs) continue;
      lastFired.set(w.name, Date.now());
      urgent = w.prompt;
    }
  });

  // Deliver the urgent turn the instant the current run settles, ahead of any
  // autonomy timer.
  pi.on("agent_settled", async (_event, ctx: ExtensionContext) => {
    if (!urgent) return;
    const prompt = urgent;
    urgent = null;
    if (ctx.isIdle() && !ctx.hasPendingMessages()) pi.sendUserMessage(prompt);
  });

  pi.registerCommand("reflex", {
    description: "Standing orders (/reflex, /reflex add <rule>, /reflex rm <id>)",
    handler: async (args, ctx) => {
      const arg = args.trim();
      if (arg.startsWith("add ")) {
        const order = arg.slice(4).trim();
        db.prepare(`INSERT OR IGNORE INTO standing_order (orders, created_at) VALUES (?, ?)`).run(
          order,
          Date.now()
        );
        ctx.ui.notify(`Standing order added: ${order}`, "info");
        return;
      }
      if (arg.startsWith("rm ")) {
        db.prepare(`DELETE FROM standing_order WHERE id = ?`).run(Number(arg.slice(3).trim()));
        ctx.ui.notify("Standing order removed", "info");
        return;
      }
      const orders = list();
      const dogs = watchdogs.map((w) => `~ ${w.name} (${w.whenBelow !== undefined ? `<${w.whenBelow}` : "match"})`);
      ctx.ui.notify(
        (orders.length
          ? orders.map((o) => `[${o.id}] ${o.orders}`).join("\n")
          : "No standing orders.") + "\nWatchdogs:\n" + dogs.join("\n"),
        "info"
      );
    },
  });

  pi.on("session_shutdown", async () => {
    db.close();
  });
}
