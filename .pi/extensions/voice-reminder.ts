/**
 * A short reminder appended to the very end of the system prompt on every
 * turn, so it is the last thing the model reads before speaking. The system
 * prompt is rebuilt per call and never stored in the transcript, so this
 * costs a few tokens per turn and accumulates nowhere.
 *
 * Configured by the harness user, not written into stone:
 *   NPC_VOICE_REMINDER        the reminder text itself, or
 *   NPC_VOICE_REMINDER_FILE   a file to read it from (read fresh each turn,
 *                             so a volume-mounted file can be edited live)
 * Set neither and this extension does nothing. /remind shows or replaces the
 * active reminder for the running session.
 *
 * This file is named to sort after the other extensions: systemPrompt edits
 * chain in load order, and the reminder must land last to land closest.
 */
import * as fs from "node:fs";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export default function (pi: ExtensionAPI) {
  let override: string | null = null;

  const reminder = (): string => {
    if (override !== null) return override;
    const file = process.env.NPC_VOICE_REMINDER_FILE;
    if (file) {
      try {
        return fs.readFileSync(file, "utf-8").trim();
      } catch {
        /* fall through to the inline form */
      }
    }
    return (process.env.NPC_VOICE_REMINDER || "").trim();
  };

  pi.on("before_agent_start", async (event) => {
    const text = reminder();
    if (!text) return undefined;
    return {
      systemPrompt: event.systemPrompt + "\n\n## Before you speak\n" + text,
    };
  });

  pi.registerCommand("remind", {
    description: "Show or set the per-turn voice reminder (/remind [new text|off])",
    handler: async (args, ctx) => {
      const arg = args.trim();
      if (arg === "off") {
        override = "";
        ctx.ui.notify("Voice reminder off for this session", "info");
        return;
      }
      if (arg) {
        override = arg;
        ctx.ui.notify(`Voice reminder set: ${arg}`, "info");
        return;
      }
      ctx.ui.notify(reminder() || "No voice reminder configured.", "info");
    },
  });
}
