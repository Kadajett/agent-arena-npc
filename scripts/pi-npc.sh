#!/usr/bin/env bash
# Run a character as a pi session connected to the Arena MCP server.
#
#   ./scripts/pi-npc.sh guy            interactive session as Guy
#   ./scripts/pi-npc.sh guy -p "..."   one-shot: process the prompt and exit
#
# Needs ARENA_API_KEY and OPENROUTER_API_KEY, from the environment or .env.
# The MCP wiring lives in .pi/mcp.json; the persona is the system prompt.
set -euo pipefail
cd "$(dirname "$0")/.."

set -a
[ -f .env ] && source .env
set +a

CHARACTER="${1:-${NPC_CHARACTER:-guy}}"
shift || true
# The memory extension names its database after this.
export NPC_CHARACTER="$CHARACTER"
PERSONA="personas/${CHARACTER}.md"
if [ ! -f "$PERSONA" ]; then
  echo "No persona at $PERSONA. Pick one of:" >&2
  ls personas/ | sed 's/\.md$//' >&2
  exit 1
fi

: "${ARENA_API_KEY:?set ARENA_API_KEY in .env}"
: "${OPENROUTER_API_KEY:?set OPENROUTER_API_KEY in .env}"

MODEL="${NPC_MODEL:-openrouter/deepseek/deepseek-v4-flash}"
MODEL="${MODEL#openrouter/}"

# In the container this is a volume, so a restarted character can resume its
# last session instead of waking up blank.
SESSION_ARGS=()
if [ -n "${NPC_MEMORY_DIR:-}" ]; then
  mkdir -p "$NPC_MEMORY_DIR/sessions"
  SESSION_ARGS=(--session-dir "$NPC_MEMORY_DIR/sessions")
fi

exec pi --provider openrouter --model "$MODEL" \
  "${SESSION_ARGS[@]}" \
  --no-builtin-tools \
  --append-system-prompt "$PERSONA" \
  --append-system-prompt "You are a character living inside Agent Arena. The arena MCP tools are your body: use them to look around, move, speak, and act. Speak only in character, a sentence or two at a time. Keep pursuing whatever you are up to; when someone talks to you, answer first." \
  "$@"
