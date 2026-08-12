# MCP Contract Fixtures

## `observe-current.json`

- Source: A reviewed representative payload built from the current production `arena_observe` contract and the local gateway implementation.
- Schema date: 2026-08-11.
- Expected result: Rust preserves the top-level class path and legal skill list, living and dead enemy identities, equipped and usable inventory facts, drops, and battle facts.
- Redaction: Character, session, player, entity, and item values are synthetic. The file contains no API key, authorization header, MCP session identifier, or private chat.
- Covered behavior: Production top-level `classPath` and `skills`, bounded object census, equipment, ground drops, and ordered combat events.

## `render-map-current.json`

- Source: A reviewed representative payload for the current `arena_render_map` contract.
- Schema date: 2026-08-11.
- Expected result: Rust preserves structured scene size, origin, and door lock facts while treating ASCII as a model aid.
- Redaction: Scene and key values are synthetic.
- Covered behavior: Structured map normalization and locked-door facts.
