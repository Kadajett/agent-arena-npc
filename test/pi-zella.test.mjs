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
  assert.match(persona, /Write four bars once through/);
  assert.match(persona, /delayed vibrato/);
  assert.match(persona, /alto vowel/);
  assert.match(persona, /four-bar vocal sentence/);
  assert.match(persona, /at least three\s+quarters/);
  assert.match(persona, /Never default to 100 BPM/);
  assert.match(persona, /OpenScore Lieder Corpus/);
  assert.match(persona, /CC0/);
  assert.match(persona, /Do not quote, transcribe, imitate, or reconstruct/);
  assert.match(persona, /Never copy music or lyrics from a study reference/);
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

test('Zella and Barnaby carry the verified playable quest guide', () => {
  const zella = read('characters/zella.conf');
  const barnaby = read('characters/barnaby.conf');
  const guide = read('personas/zella-world.md');

  assert.match(zella, /NPC_VOICE_FILES="\$\{NPC_VOICE_FILES:-personas\/zella-world\.md\}"/);
  assert.match(barnaby, /NPC_VOICE_FILES="\$\{NPC_VOICE_FILES:-personas\/zella-world\.md\}"/);
  assert.match(zella, /verified quest/);
  assert.match(barnaby, /verified quest/);
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

test('Barnaby is a stateless plain-speech conversation anchor', () => {
  const config = read('characters/barnaby.conf');
  const persona = read('personas/barnaby.md');
  const runner = read('scripts/pi-npc.sh');

  assert.match(config, /NPC_MODEL="\$\{NPC_MODEL:-llmmo\/qwen3\.8-27b\}"/);
  assert.match(config, /NPC_MEMORY_ENABLED="\$\{NPC_MEMORY_ENABLED:-0\}"/);
  assert.match(config, /NPC_SESSION_ENABLED="\$\{NPC_SESSION_ENABLED:-0\}"/);
  assert.match(config, /NPC_TODO_ENABLED="\$\{NPC_TODO_ENABLED:-0\}"/);
  assert.match(config, /NPC_REFLEXES_ENABLED="\$\{NPC_REFLEXES_ENABLED:-0\}"/);
  assert.match(config, /include_recent_messages true/);
  assert.match(config, /Conversation is useful even when it advances no task/);
  assert.match(persona, /normal person having a\s+modern conversation/);
  assert.match(persona, /Do not invent a destination, event, reward, enemy, item/);
  assert.match(persona, /Use no riddles, omens, prophecies, metaphors, aphorisms/);
  assert.doesNotMatch(persona, /existential dread/);
  assert.match(runner, /PROVIDER="\$\{MODEL%%\/\*\}"/);
  assert.match(runner, /exec pi --provider "\$PROVIDER"/);
});

test('production moves Zella from Pi to a persistent Codex session', () => {
  const compose = read('deploy/pi/docker-compose.yml');
  const deployment = read('deploy/codex/kubernetes.yaml');
  const runner = read('deploy/codex/run.sh');
  const config = read('deploy/codex/config.toml');

  assert.doesNotMatch(compose, /\n  zella:\n/);
  assert.match(deployment, /name: zella-codex/);
  assert.match(deployment, /image: zella-codex:0\.1\.3/);
  assert.match(deployment, /name: zella-codex-home-erebor/);
  assert.match(runner, /codex exec resume --last --all/);
  assert.match(runner, /sleep 120/);
  assert.match(config, /model = "gpt-5\.6-terra"/);
  assert.match(config, /model_context_window = 32768/);
  assert.match(config, /model_auto_compact_token_limit = 24576/);
  assert.match(config, /\[mcp_servers\.arena\]/);
  assert.match(config, /enabled_tools = \[/);
  assert.match(config, /shell_tool = false/);
  assert.match(config, /apps = false/);
  assert.match(config, /multi_agent = false/);
});
