/**
 * One image, one character per container: NPC_CHARACTER picks who to be.
 */

import { Npc, CharacterSheet } from './harness/npc.js';
import { installTrace } from './harness/trace.js';
import { guy } from './characters/guy.js';
import { barnaby } from './characters/barnaby.js';
import { wanderer } from './characters/wanderer.js';
import { tansy } from './characters/tansy.js';
import { hollis } from './characters/hollis.js';
import { marren } from './characters/marren.js';
import { cutter } from './characters/cutter.js';
import { nerys } from './characters/nerys.js';
import { ash } from './characters/ash.js';
import { doran } from './characters/doran.js';
import { aveline } from './characters/aveline.js';
import { bolo } from './characters/bolo.js';
import { sanejack } from './characters/sanejack.js';
import { kest } from './characters/kest.js';

const CAST: Record<string, CharacterSheet> = {
  guy,
  barnaby,
  wanderer,
  tansy,
  hollis,
  marren,
  cutter,
  nerys,
  ash,
  doran,
  aveline,
  bolo,
  sanejack,
  kest
};

const wanted = String(process.env.NPC_CHARACTER ?? 'guy').toLowerCase();
const sheet = CAST[wanted];
if (!sheet) {
  console.error(
    `No character called "${wanted}". Available: ${Object.keys(CAST).join(', ')}.`
  );
  process.exit(1);
}

// Before the Npc exists, so the very first call - and every one Mastra makes
// on its own behalf, observation and reflection included - passes through the
// trace. That coverage is the point: the calls we could not see are the ones
// that have been failing. See trace.ts.
installTrace(sheet.id, process.env.NPC_MEMORY_DIR ?? '/npc/var');

await new Npc(sheet).run();
