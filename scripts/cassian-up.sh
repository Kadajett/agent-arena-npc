#!/usr/bin/env bash
# Bring Cassian up in Docker and attach to his pi session. Waits for the two
# keys to appear in .env first, so it can be started before they are set.
set -euo pipefail
cd "$(dirname "$0")/.."

need() { grep -qE "^$1=.+" .env 2>/dev/null; }

while ! need OPENROUTER_API_KEY || ! need ARENA_API_KEY; do
  missing=""
  need OPENROUTER_API_KEY || missing="OPENROUTER_API_KEY"
  need ARENA_API_KEY || missing="$missing ARENA_API_KEY"
  echo "waiting for$( [ -n "$missing" ] && echo " ${missing// / and }" ) in .env ..."
  sleep 5
done

docker compose -f docker-compose.pi.yml up -d --build
echo "Attaching to cassian. Type to steer him; detach with ctrl-p ctrl-q."
exec docker attach cassian
