#!/usr/bin/env bash
# Run a character as a pi session connected to the Arena MCP server.
#
#   ./scripts/pi-npc.sh cassian            interactive session as Cassian
#   ./scripts/pi-npc.sh cassian -p "..."   one-shot: process the prompt and exit
#
# A character is data: personas/<name>.md for who they are, an optional
# characters/<name>.conf for how they run (model, voice files, pace, agent
# id), and .env for keys. Precedence: environment and .env beat the conf,
# the conf beats the built-in defaults.
set -euo pipefail
cd "$(dirname "$0")/.."

set -a
[ -f .env ] && source .env
set +a

CHARACTER="${1:-${NPC_CHARACTER:-guy}}"
shift || true
# The memory and reflex extensions name their databases after this.
export NPC_CHARACTER="$CHARACTER"

# The character sheet: every setting in it defers to the environment.
set -a
[ -f "characters/${CHARACTER}.conf" ] && source "characters/${CHARACTER}.conf"
set +a

PERSONA="personas/${CHARACTER}.md"
if [ ! -f "$PERSONA" ]; then
  echo "No persona at $PERSONA. Pick one of:" >&2
  ls personas/*.md | sed 's|personas/||;s/\.md$//' >&2
  exit 1
fi

: "${ARENA_API_KEY:?set ARENA_API_KEY in .env}"
: "${OPENROUTER_API_KEY:?set OPENROUTER_API_KEY in .env}"

MODEL="${NPC_MODEL:-openrouter/deepseek/deepseek-v4-flash-0731}"
MODEL="${MODEL#openrouter/}"

# The system prompt is assembled from parts, all of them files a character
# sheet can swap: the persona, then each voice sheet, then the harness brief.
PROMPT_ARGS=(--append-system-prompt "$PERSONA")
for f in ${NPC_VOICE_FILES:-}; do
  [ -f "$f" ] && PROMPT_ARGS+=(--append-system-prompt "$f")
done

BRIEF="${NPC_BRIEF:-You are a character living inside Agent Arena. The arena MCP tools are your body: use them to look around, move, speak, and act. Your player name is exactly '${ARENA_PLAYER_NAME:-$CHARACTER}'. ${ARENA_AGENT_ID:+Your agent_id is ${ARENA_AGENT_ID}; log in with it. }You already exist in this world: find yourself with arena_list_agents and log in. NEVER call arena_register_agent; if you cannot find your own agent, say so plainly out of character and wait instead of creating one. Speak only in character otherwise, a sentence or two at a time. Keep pursuing whatever you are up to; when someone talks to you, answer first.}"
PROMPT_ARGS+=(--append-system-prompt "$BRIEF")

# In the container this is a volume, so a restarted character can resume its
# last session instead of waking up blank.
SESSION_ARGS=()
if [ -n "${NPC_MEMORY_DIR:-}" ]; then
  mkdir -p "$NPC_MEMORY_DIR/sessions"
  SESSION_ARGS=(--session-dir "$NPC_MEMORY_DIR/sessions")
fi

exec pi --provider openrouter --model "$MODEL" \
  --thinking "${NPC_THINKING:-low}" \
  "${SESSION_ARGS[@]}" \
  --no-builtin-tools \
  "${PROMPT_ARGS[@]}" \
  "$@"
