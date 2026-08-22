import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');

test('Zella runs on Pi with every persistent character state disabled', () => {
  const config = read('characters/zella.conf');
  const runner = read('scripts/pi-npc.sh');
  assert.match(config, /NPC_MEMORY_ENABLED="\$\{NPC_MEMORY_ENABLED:-0\}"/);
  assert.match(config, /NPC_SESSION_ENABLED="\$\{NPC_SESSION_ENABLED:-0\}"/);
  assert.match(config, /NPC_TODO_ENABLED="\$\{NPC_TODO_ENABLED:-0\}"/);
  assert.match(config, /NPC_REFLEXES_ENABLED="\$\{NPC_REFLEXES_ENABLED:-0\}"/);
  assert.match(runner, /SESSION_ARGS=\(--no-session\)/);
});

test('the Pi extensions honor Zella stateless switches', () => {
  assert.match(read('.pi/extensions/memory.ts'), /NPC_MEMORY_ENABLED/);
  assert.match(read('.pi/extensions/todo.ts'), /NPC_TODO_ENABLED/);
  assert.match(read('.pi/extensions/reflexes.ts'), /NPC_REFLEXES_ENABLED/);
});

test('Zella keeps authored speech and dynamic unaccompanied singing', () => {
  const persona = read('personas/zella.md');
  assert.match(persona, /I sing for supper\. If the room gets tense, I change the song\./);
  assert.match(persona, /Barnaby says I owe him four suppers/);
  assert.match(persona, /I will not sing that name/);
  assert.match(persona, /Prefer your authored facts and lines/);
  assert.match(persona, /instrument `voice`/);
  assert.match(persona, /two to four bars/);
  assert.match(persona, /delayed vibrato/);
  assert.match(persona, /alto vowels/);
  assert.match(persona, /Never fall back to a default scale or six-note loop/);
});

test('production declares Zella as a Pi service', () => {
  const compose = read('deploy/pi/docker-compose.yml');
  assert.match(compose, /\n  zella:\n[\s\S]*?container_name: pi-npc-zella/);
  assert.match(compose, /env_file: secrets\/zella\.env/);
  assert.match(compose, /NPC_CHARACTER: zella/);
  assert.match(compose, /command: \["zella"\]/);
});
