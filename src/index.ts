/**
 * One image, one character per container: NPC_CHARACTER picks who to be.
 */

import { Npc, CharacterSheet } from './harness/npc.js';
import { guy } from './characters/guy.js';
import { barnaby } from './characters/barnaby.js';
import { wanderer } from './characters/wanderer.js';

const CAST: Record<string, CharacterSheet> = { guy, barnaby, wanderer };

const wanted = String(process.env.NPC_CHARACTER ?? 'guy').toLowerCase();
const sheet = CAST[wanted];
if (!sheet) {
  console.error(
    `No character called "${wanted}". Available: ${Object.keys(CAST).join(', ')}.`
  );
  process.exit(1);
}

await new Npc(sheet).run();
