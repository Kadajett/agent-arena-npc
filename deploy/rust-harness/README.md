# Rust NPC production deployment

This Compose project runs one generic harness image with one external character sheet and one secret environment file per character. Character policy belongs in the sheet. Runtime and actor code do not contain character-name branches.

The `deploy_npc_var` volume is shared with the legacy deployment so the migrated SQLite databases remain durable. Create `/npc/var/rust` as UID/GID 10003 before startup.

Each `secrets/<character>.env` must be mode 0600 and contain the character's existing Agent Arena key, the shared OpenRouter key, and these explicit production controls:

```dotenv
ARENA_MCP_URL=https://mcp.yougotserved.dev/mcp
ARENA_API_KEY=secret
OPENROUTER_API_KEY=secret
ARENA_PLAYER_NAME=Exact Existing Name

NPC_STRATEGIST_ENABLED=true
NPC_STRATEGIST_MODEL=openai/gpt-oss-120b
NPC_TACTICIAN_MODEL=google/gemini-3.1-flash-lite
NPC_STRATEGIST_MEMORY_MAX_TOKENS=8000
NPC_STRATEGIST_MIN_INTERVAL_MS=30000

NPC_TACTICAL_ROLLOUT_MODE=full
NPC_ALLOW_LIVE_MUTATION=true
NPC_LIVE_ACTION_BUDGET=unlimited
NPC_LIVE_MAX_ACTIONS_PER_PACKET=3
NPC_LIVE_PACKET_MAX_AGE_MS=5000
NPC_LIVE_EXPECTED_CHARACTER_ID=character-id
NPC_LIVE_EXPECTED_PLAYER_NAME=Exact Existing Name
NPC_LIVE_ALLOWED_SCENE=*

NPC_TACTICAL_MAX_HZ=5
NPC_IDLE_TACTICAL_HZ=0
NPC_PERCEPTION_INTERVAL_MS=500
NPC_PERCEPTION_MAP_RADIUS=16
NPC_PERCEPTION_INVENTORY_EVERY_CYCLES=10
RUST_LOG=info
```

Barnaby uses `NPC_LIVE_ALLOWED_SCENE=reldens-house-1`. His sheet also omits `walk`, `doors`, and `fight`, so the generic body validator rejects those mutations even if a model proposes one.

Do not start a Rust service while its legacy service controls the same player. Stop the old service, make an online SQLite backup, run `migrate-mastra-memory`, then start and verify the replacement. Roll back by stopping the Rust service and restarting the untouched legacy service and database.
