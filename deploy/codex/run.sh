#!/bin/sh
set -u

turn() {
  if find "$CODEX_HOME/sessions" -type f -name '*.jsonl' -print -quit 2>/dev/null | grep -q .; then
    codex exec resume --last --all \
      --dangerously-bypass-approvals-and-sandbox \
      --skip-git-repo-check \
      -m gpt-5.6-terra \
      -c 'model_reasoning_effort="medium"' \
      - < /opt/zella/tick.md
    return
  fi

  codex exec \
    --dangerously-bypass-approvals-and-sandbox \
    --skip-git-repo-check \
    -C /workspace \
    -m gpt-5.6-terra \
    -c 'model_reasoning_effort="medium"' \
    - < /opt/zella/tick.md
}

mkdir -p "$CODEX_HOME/sessions" /workspace
{
  cat /opt/zella/zella.md
  printf '\n'
  cat /opt/zella/zella-world.md
  printf '\n# World connection\n\nYour arena agent id is `%s`. Use it when arena_login asks for agent_id.\n' "$ARENA_AGENT_ID"
} > /workspace/AGENTS.md

while true; do
  turn || printf '%s\n' 'Zella turn failed; retrying after the normal interval.' >&2
  sleep 120
done
