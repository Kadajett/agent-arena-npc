/**
 * Cheap models regularly call arena tools without the required agent_id and
 * burn a failed round-trip learning it. The id is pinned in ARENA_AGENT_ID for
 * the character's whole life, so fill it in whenever the model leaves it out.
 *
 * The MCP adapter exposes one pi-level tool named "mcp" whose input carries
 * the real call as {tool, args}, so that is where the id goes. Tool inputs are
 * mutable in the tool_call hook; tools that legitimately take some OTHER
 * agent's id still get whatever the model passed.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export default function (pi: ExtensionAPI) {
  const agentId = process.env.ARENA_AGENT_ID;
  if (!agentId) return;

  pi.on("tool_call", async (event) => {
    const input = event.input as { tool?: unknown; args?: unknown } | undefined;
    if (!input || typeof input.tool !== "string" || !/arena_/.test(input.tool)) return undefined;
    if (input.args === undefined || input.args === null) input.args = {};
    const args = input.args as Record<string, unknown>;
    if (typeof args === "object" && args.agent_id === undefined) {
      args.agent_id = agentId;
    }
    return undefined;
  });
}
