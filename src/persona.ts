import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));

/** A character's system prompt, kept as prose in personas/<id>.md. */
export function loadPersona(id: string): string {
  return readFileSync(join(here, '..', 'personas', `${id}.md`), 'utf8');
}
