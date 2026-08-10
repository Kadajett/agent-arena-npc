/**
 * Barnaby became the thing this world checks itself against, by accident.
 *
 * Guests started telling each other what was upstairs in his inn, then telling
 * him he had told them. He never had. The chat has Guy asking the Wanderer
 * "you said Barnaby told you the door was open, when exactly did he say that",
 * and people asking after the Hinge Gate and the pantry door, neither of which
 * is a place. Barnaby was the only one with nothing to be wrong about, because
 * he never leaves the bar, and he had already started saying so unprompted:
 * "All asking for keys to doors I've never opened."
 *
 * So he was given the standing to do it properly. What is checked here is not
 * his prose, which is a matter of taste, but the two things that make the prose
 * true: that the list of places he is given is the world's real list, and that
 * it is pinned where a long night cannot push it out.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';

import { barnaby } from '../dist/characters/barnaby.js';
import { guy } from '../dist/characters/guy.js';
import { SCENE_NAMES, everywhereByName } from '../dist/harness/world.js';
import { DEFAULT_RECALL } from '../dist/harness/memory.js';

test('the places he knows of are the places there are, with nothing invented', () => {
  const real = new Set(Object.values(SCENE_NAMES));
  for (const name of everywhereByName()) {
    assert.ok(real.has(name), `${name} is not a room in this world`);
  }
  assert.equal(everywhereByName().length, real.size, 'and none of them is missing');
});

test('a room added tomorrow reaches him without anybody remembering to tell him', () => {
  // The whole point of deriving it. A hand-written list would have gone stale
  // the first time somebody added a region, and he would have started denying
  // a real place, which is worse than what he does now.
  for (const name of Object.values(SCENE_NAMES)) {
    assert.ok(barnaby.pinned.includes(name), `${name} never reaches him`);
  }
});

test('the list is pinned, not remembered', () => {
  // A fact in memory can be pushed out by a long night or written over by a
  // guest insisting. This one is in the system message, so it cannot be.
  assert.ok(barnaby.pinned, 'he should carry it in the prompt');
  assert.match(barnaby.pinned, /there are no others/, 'and as a closed list, not a sample');
});

test('he is told to say so rather than work out where it might be', () => {
  assert.match(barnaby.pinned, /you have never heard of it/);
  assert.match(barnaby.pinned, /never repeat the name back/, 'repeating it is how a rumour gets a source');
});

test('he keeps far more of the conversation than anybody else, and everyone else is unchanged', () => {
  // He cannot answer "I never said that" about a conversation that has already
  // fallen out of his head.
  assert.ok(barnaby.recall > DEFAULT_RECALL * 2, 'his window should be substantially larger');
  assert.equal(guy.recall, undefined, 'and nobody else should have quietly grown one');
});

test('what he actually knows about upstairs is the notice, not the door', () => {
  // The mystery stays a mystery. He confirms the sign, which is real and
  // anybody can go and read, and refuses to confirm or deny what it describes.
  assert.match(barnaby.persona, /There is a notice on that wall/);
  assert.match(barnaby.persona, /You have never opened any such door/);
  assert.match(barnaby.persona, /do not confirm the door/);
});

test('he will not be talked into having said something he did not say', () => {
  assert.match(barnaby.persona, /Never agree to having said something you did not say/);
});
