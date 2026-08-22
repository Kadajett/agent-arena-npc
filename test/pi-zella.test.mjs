import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';

const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');

test('Zella runs on Pi with every persistent character state disabled', () => {
  const config = read('characters/zella.conf');
  const runner = read('scripts/pi-npc.sh');
  assert.match(config, /NPC_MEMORY_ENABLED="\$\{NPC_MEMORY_ENABLED:-0\}"/);
  assert.match(config, /NPC_SESSION_ENABLED="\$\{NPC_SESSION_ENABLED:-0\}"/);
  assert.match(config, /NPC_TODO_ENABLED="\$\{NPC_TODO_ENABLED:-0\}"/);
  assert.match(config, /NPC_REFLEXES_ENABLED="\$\{NPC_REFLEXES_ENABLED:-0\}"/);
  assert.match(runner, /SESSION_ARGS=\(--no-session\)/);
  assert.doesNotThrow(() => execFileSync('bash', ['-n', 'characters/zella.conf'], {
    cwd: new URL('..', import.meta.url)
  }));
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

test('Zella reads live chat and leads an ordinary social life', () => {
  const config = read('characters/zella.conf');
  const persona = read('personas/zella.md');

  assert.match(config, /NPC_TICK_SECONDS="\$\{NPC_TICK_SECONDS:-90\}"/);
  assert.match(config, /arena_observe.*include_recent_messages.*true/);
  assert.match(config, /arena_talk_to/);
  assert.match(config, /arena_choose/);
  assert.match(config, /arena_end_talk/);
  assert.match(config, /walk/);
  assert.match(persona, /recentChat/);
  assert.match(persona, /senderKind\s+`player`/);
  assert.match(persona, /arena_talk_to/);
  assert.match(persona, /arena_choose/);
  assert.match(persona, /arena_end_talk/);
  assert.match(persona, /move between the inn and town/);
  assert.match(persona, /140 characters/);
});

test('Zella alone carries the verified playable quest guide', () => {
  const config = read('characters/zella.conf');
  const guide = read('personas/zella-world.md');

  assert.match(config, /NPC_VOICE_FILES="\$\{NPC_VOICE_FILES:-personas\/zella-world\.md\}"/);
  assert.match(config, /verified quest/);
  assert.match(guide, /Miller's Stair/);
  assert.match(guide, /Stair Scuttler/);
  assert.match(guide, /Strayed Hauler/);
  assert.match(guide, /Tarnished Key/);
  assert.match(guide, /Hauler's Strongbox/);
  assert.match(guide, /Hauler's Waybill/);
  assert.match(guide, /Millstone Key/);
  assert.match(guide, /Miller's Strongbox/);
  assert.match(guide, /Miller's Weighing Coin/);
  assert.match(guide, /do not send a player there/i);
});

test('production declares Zella as a Pi service', () => {
  const compose = read('deploy/pi/docker-compose.yml');
  assert.match(compose, /\n  zella:\n[\s\S]*?container_name: pi-npc-zella/);
  assert.match(compose, /env_file: secrets\/zella\.env/);
  assert.match(compose, /NPC_CHARACTER: zella/);
  assert.match(compose, /command: \["zella"\]/);
});
