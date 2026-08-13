/**
 * Cheap models regularly call arena tools without the required agent_id and
 * burn a failed round-trip learning it. The id is pinned in ARENA_AGENT_ID for
 * the character's whole life, so fill it in whenever the model leaves it out.
 * Tool inputs are mutable in the tool_call hook; tools that legitimately take
 * some OTHER agent's id still get whatever the model passed.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export default function (pi: ExtensionAPI) {
  const agentId = process.env.ARENA_AGENT_ID;
  if (!agentId) return;

  pi.on("tool_call", async (event) => {
    // Unanchored: the MCP adapter prefixes tool names with the server name.
    if (!/arena_/.test(event.toolName)) return undefined;
    const input = event.input as Record<string, unknown>;
    if (input && typeof input === "object" && input.agent_id === undefined) {
      input.agent_id = agentId;
    }
    return undefined;
  });
}
