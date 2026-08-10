# Known advisories in the NPC harness dependencies

`npm audit` reports four advisories, two of them high. All four are unfixable
today: the harness is already on the newest `@mastra/core` (1.57.0) and
`@mastra/memory` (1.26.0), and the advisories are against transitive
dependencies those packages pin. `npm audit fix` changes nothing, and the
`image-size` advisory covers every published version (`range: *`), so there is
no version to move to.

What matters is that neither is a code-execution or credential problem. The
container holds a live Arena API key, and nothing here puts it at risk.

## image-size (high, GHSA-w3rx-r6r6-pgpr, GHSA-5p2g-fcmc-qvqq)

Infinite loops in the ICNS, JXL and HEIF parsers: a malformed image hangs the
parser.

**Not reachable.** `@mastra/memory` pulls this in for multi-modal messages.
NPCs store and recall text. No image ever enters the loop, so no parser runs.

## @ai-sdk/provider-utils (low, GHSA-866g-f22w-33x8)

Uncontrolled resource consumption while handling a model provider's response.

**Reachable**, since characters talk to OpenRouter on every decision. The worst
case is one container burning CPU or memory on a malformed response, and the
sandbox already bounds that: `cpus: 0.5`, `mem_limit: 512m`, `pids_limit: 64`,
read-only root filesystem, all capabilities dropped, and `restart: always` so a
wedged character comes back by itself.

## When to look again

Re-run `npm audit` after any Mastra upgrade. Both advisories resolve upstream,
not here; the only local action worth taking would be dropping `@mastra/memory`
if the image-size dependency ever becomes reachable.
