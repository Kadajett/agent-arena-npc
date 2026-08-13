/**
 * The heartbeat. After the agent settles, wait a while and hand it its own
 * next turn, so a character keeps living when nobody is talking to it. Any
 * human keystroke cancels the pending tick: the person steering always wins.
 *
 *   /auto                      toggle
 *   /auto on | off
 *   /auto 45                   set delay seconds
 *   /auto prompt <text>        set the tick prompt
 *
 * Environment: NPC_AUTONOMOUS=0 starts it off (default on), NPC_TICK_SECONDS
 * sets the delay, NPC_TICK_PROMPT replaces the tick text.
 */
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";

const DEFAULT_PROMPT =
  "(Autonomy tick, not a player speaking. Do not reply to this text.) " +
  "Look at the room. If anyone spoke to you since your last turn, answer them " +
  "first. Then take one or two small actions toward whatever you are up to.";

export default function (pi: ExtensionAPI) {
  let enabled = !/^(0|false|off)$/i.test(process.env.NPC_AUTONOMOUS ?? "");
  let delayMs = (Number(process.env.NPC_TICK_SECONDS) || 30) * 1000;
  let tickPrompt = process.env.NPC_TICK_PROMPT || DEFAULT_PROMPT;

  let timer: ReturnType<typeof setTimeout> | undefined;
  let ticker: ReturnType<typeof setInterval> | undefined;
  let fireAt = 0;
  let unsubscribeInput: (() => void) | undefined;
  let lastEditorText = "";
  let editorStalls = 0;

  const clearPending = () => {
    if (timer) clearTimeout(timer);
    if (ticker) clearInterval(ticker);
    timer = undefined;
    ticker = undefined;
    fireAt = 0;
  };

  const status = (ctx: ExtensionContext, text: string | undefined) => {
    if (ctx.hasUI) ctx.ui.setStatus("auto", text);
  };

  const refreshStatus = (ctx: ExtensionContext) => {
    if (!enabled) return status(ctx, undefined);
    if (!fireAt) return status(ctx, "auto: on");
    const left = Math.max(0, Math.ceil((fireAt - Date.now()) / 1000));
    status(ctx, `auto: ${left}s`);
  };

  const schedule = (ctx: ExtensionContext, ms = delayMs) => {
    clearPending();
    if (!enabled) return;
    fireAt = Date.now() + ms;
    refreshStatus(ctx);

    ticker = setInterval(() => refreshStatus(ctx), 1000);
    ticker.unref?.();

    timer = setTimeout(() => {
      clearPending();
      if (!enabled) return;
      // The agent picked work back up, or a message is already queued.
      if (!ctx.isIdle() || ctx.hasPendingMessages()) return;
      // The human is mid-thought in the editor: back off a cycle. But text
      // that sits unchanged for two full cycles is not a human typing, it is
      // stray keystrokes from an attach race, and it would otherwise mute
      // autonomy forever. Clear it and carry on.
      if (ctx.hasUI) {
        const editor = ctx.ui.getEditorText();
        if (editor.trim().length > 0) {
          if (editor === lastEditorText && ++editorStalls >= 2) {
            ctx.ui.setEditorText("");
            ctx.ui.notify(`autonomy: cleared stale editor text "${editor.slice(0, 30)}"`, "info");
            editorStalls = 0;
            lastEditorText = "";
          } else {
            if (editor !== lastEditorText) editorStalls = 0;
            lastEditorText = editor;
            status(ctx, "auto: waiting (editor has text)");
            schedule(ctx);
            return;
          }
        } else {
          lastEditorText = "";
          editorStalls = 0;
        }
      }
      refreshStatus(ctx);
      // todo.ts hangs its list renderer on globalThis; appending it here puts
      // the task list at the newest edge of context, after the chat noise,
      // which is the only place a cheap model reliably reads it.
      const todos = (globalThis as { __npcTodoRender?: () => string }).__npcTodoRender?.();
      pi.sendUserMessage(todos ? `${tickPrompt}\n\n${todos}` : tickPrompt);
    }, ms);
    timer.unref?.();
  };

  pi.on("session_start", async (_event, ctx) => {
    if (ctx.hasUI && !unsubscribeInput) {
      // Any keystroke means the human is steering: push the pending tick
      // back to a full interval. Cancelling outright would strand autonomy,
      // since nothing reschedules until the next agent_settled.
      unsubscribeInput = ctx.ui.onTerminalInput(() => {
        if (timer) schedule(ctx);
        return undefined; // never consume input
      });
    }
    // Wake the character shortly after boot instead of waiting for someone
    // to speak first.
    if (enabled && ctx.isIdle()) schedule(ctx, 3000);
    else refreshStatus(ctx);
  });

  pi.on("agent_start", async (_event, ctx) => {
    clearPending();
    refreshStatus(ctx);
  });

  pi.on("input", async (event, ctx) => {
    // A submitted human message will start a run (agent_start clears, and
    // agent_settled reschedules); a slash command will not, so reset rather
    // than strand the loop.
    if (event.source !== "extension" && timer) schedule(ctx);
    return { action: "continue" as const };
  });

  pi.on("agent_settled", async (_event, ctx) => {
    schedule(ctx);
  });

  pi.on("session_shutdown", async (_event, ctx) => {
    clearPending();
    unsubscribeInput?.();
    unsubscribeInput = undefined;
    status(ctx, undefined);
  });

  pi.registerCommand("auto", {
    description: "Toggle the autonomy loop (/auto [on|off|<seconds>|prompt <text>])",
    handler: async (args, ctx) => {
      const arg = args.trim();

      if (arg.startsWith("prompt ")) {
        tickPrompt = arg.slice(7).trim() || DEFAULT_PROMPT;
        ctx.ui.notify(`auto prompt: ${tickPrompt}`, "info");
        return;
      }
      if (/^\d+$/.test(arg)) {
        delayMs = Number(arg) * 1000;
        enabled = true;
      } else if (arg === "on") {
        enabled = true;
      } else if (arg === "off") {
        enabled = false;
      } else if (arg === "") {
        enabled = !enabled;
      } else {
        ctx.ui.notify("Usage: /auto [on|off|<seconds>|prompt <text>]", "warning");
        return;
      }

      if (!enabled) {
        clearPending();
        status(ctx, undefined);
        ctx.ui.notify("Autonomy off", "info");
        return;
      }

      ctx.ui.notify(`Autonomy on: tick every ${delayMs / 1000}s when idle`, "info");
      if (ctx.isIdle()) schedule(ctx);
      else refreshStatus(ctx);
    },
  });
}
