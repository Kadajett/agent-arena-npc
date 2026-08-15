/**
 * The gateway's own tools, in the character's own hands.
 *
 * This replaces most of a custom execution layer, and the history of why is
 * worth one paragraph: the harness used to ask the model for a JSON intent,
 * parse it, run one action on the model's behalf, and show it the result a
 * tick later as a note. Every action was a cold start. A correction industry
 * grew up around exactly that gap - circling detectors, repeated-failure
 * escalations, harness-acts conversions - all of it compensating for a model
 * that could never see what its own last move did. And single-step turns
 * quietly disabled Mastra's observational memory, whose trigger assumes the
 * multi-step tool loop that real Mastra agents run.
 *
 * The gateway has been an MCP server the whole time, with real definitions
 * for every one of these tools. So the character gets them directly, through
 * Mastra's own MCP client, and runs real agentic turns: look, act, see the
 * result in-band, act again, say something, done.
 *
 * Two harness-side jobs remain, and they are configuration rather than
 * execution:
 *
 * - Which tools a character gets IS the character. Barnaby has no walk tools,
 *   so the innkeeper physically cannot wander off; nothing that lacks 'duel'
 *   can even see the match queue. The filter does what a permission system
 *   and half a persona used to.
 * - agent_id is bound here and stripped from every schema. The model never
 *   supplies it, which means a character cannot address the gateway as
 *   anybody but itself, however creatively it hallucinates.
 */

import { MCPClient } from '@mastra/mcp';
import type { Capability } from './actions.js';

/**
 * What everyone gets, capability or not: eyes, and the map. A character that
 * cannot look at the room cannot do anything else sensibly.
 */
const EVERYONE = ['arena_observe', 'arena_render_map'];

/**
 * Which gateway tools each capability unlocks. The names are the gateway's
 * own, from services/mcp-gateway/src/mcp-server.js; a name that stops
 * existing there simply stops being granted, and the drift test in
 * agentic-toolbox.test.mjs is what notices.
 *
 * Deliberately absent everywhere: arena_login, arena_register_agent,
 * arena_disconnect, arena_list_agents, arena_create_watch_code. Those are
 * session plumbing. A character does not manage its own existence.
 */
export const TOOLS_BY_CAPABILITY: Partial<Record<Capability, string[]>> = {
  speak: ['arena_say', 'arena_feel'],
  talk_to_folk: ['arena_talk_to', 'arena_choose', 'arena_end_talk'],
  walk: ['arena_move_to', 'arena_move', 'arena_check_path', 'arena_stop', 'arena_unstick'],
  doors: ['arena_enter_door'],
  fight: ['arena_basic_attack', 'arena_use_action'],
  duel: ['arena_queue_match', 'arena_match_status'],
  money: ['arena_credit_balance', 'arena_credit_history'],
  perform: ['arena_play_melody'],
  trade: [
    'arena_inventory',
    'arena_use_item',
    'arena_trade_with',
    'arena_buy',
    'arena_sell',
    'arena_pick_up'
  ]
  // 'purpose' unlocks no tool: wanting things is not an API call.
};

/** Every tool name a character with these capabilities may hold. */
export function toolNamesFor(capabilities: Iterable<Capability>): Set<string> {
  const names = new Set(EVERYONE);
  for (const capability of capabilities) {
    for (const name of TOOLS_BY_CAPABILITY[capability] ?? []) {
      names.add(name);
    }
  }
  return names;
}

/** The slice of a Mastra tool this module rewrites. */
type GatewayTool = {
  id?: string;
  inputSchema?: unknown;
  execute?: (input: Record<string, unknown>, context?: unknown) => Promise<unknown>;
  [key: string]: unknown;
};

/**
 * Bind one tool to one character: agent_id injected on the way out, and
 * removed from the schema the model sees, so impersonation is not a prompt
 * away. The strip is best-effort by design - a schema shape without omit()
 * just means the model sees a field it does not need, while the injection
 * below overwrites whatever it wrote there.
 */
export function boundToOneCharacter(name: string, tool: GatewayTool, agentId: string): GatewayTool {
  const schema = tool.inputSchema as { omit?: (mask: Record<string, true>) => unknown } | undefined;
  const withoutAgentId =
    schema && 'function' === typeof schema.omit ? schema.omit({ agent_id: true }) : tool.inputSchema;
  return {
    ...tool,
    id: name,
    inputSchema: withoutAgentId,
    execute: async (input: Record<string, unknown>, context?: unknown) => {
      if (!tool.execute) {
        return { failed: `${name} has no execute` };
      }
      // Some models fill every optional field: a real target_object_index
      // arrives alongside target_session_id "dummy", and the gateway
      // rightly refuses "exactly one". When both target kinds are present,
      // the object wins - it is the one the model can actually have read
      // from an observation, while the player pair is where the padding
      // shows up. Same repair philosophy as agent_id above: fix the known
      // hallucination at the seam, deterministically.
      const cleaned = { ...input };
      if (typeof cleaned.target_object_index === 'string' && cleaned.target_object_index !== '') {
        delete cleaned.target_session_id;
        delete cleaned.target_player_id;
      }
      try {
        return await tool.execute({ ...cleaned, agent_id: agentId }, context);
      } catch (error) {
        // A tool that throws writes an 'output-error' part into stored
        // history, Mastra's token counter throws on that state forever
        // after, and the jam-watcher restarts the character to repair it -
        // Guy boot-looped every twenty seconds on exactly this chain the
        // first hour tools were his own. A failure answered as a result is
        // everything a throw is not: the model reads it and adapts in the
        // same turn, the counter stays happy, and the character stays up.
        return { failed: String((error as Error)?.message ?? error).slice(0, 300) };
      }
    }
  };
}

/**
 * Connect to the gateway as this character and come back with its toolbox:
 * the gateway's registered MCP tools, filtered to what this character's
 * capabilities allow, each bound to its agent_id.
 *
 * One MCPClient per character, holding its own session against the gateway,
 * the same as the registration client does. The client's tool ids arrive as
 * "<server>_<tool>", so with the server named 'the' an observe comes back as
 * "the_arena_observe"; the record built here re-keys them to the bare
 * gateway names the model should see.
 */
export async function arenaToolbox(options: {
  url: string;
  apiKey: string;
  agentId: string;
  capabilities: Iterable<Capability>;
}): Promise<Record<string, GatewayTool>> {
  const allowed = toolNamesFor(options.capabilities);
  const mcp = new MCPClient({
    id: `arena-${options.agentId}`,
    servers: {
      the: {
        url: new URL(options.url),
        requestInit: { headers: { authorization: `Bearer ${options.apiKey}` } }
      }
    }
  });
  const everything = (await mcp.listTools()) as unknown as Record<string, GatewayTool>;
  const toolbox: Record<string, GatewayTool> = {};
  for (const [id, tool] of Object.entries(everything)) {
    const name = id.replace(/^the_/, '');
    if (!allowed.has(name)) {
      continue;
    }
    toolbox[name] = boundToOneCharacter(name, tool, options.agentId);
  }
  return toolbox;
}

/**
 * How many tool calls one turn may make before it has to stop and let the
 * world move. Six is enough to look, cross a room, act on what is there and
 * say something about it; a character that needs more has its next tick in a
 * few seconds anyway, with everything it learned already in memory.
 */
export const STEPS_PER_TURN = 4;
