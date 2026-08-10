/**
 * A character that stops answering in the format has to be told so.
 *
 * Guy went ninety-seven turns replying in prose, narrating himself down a
 * ladder into a room that does not exist while standing motionless outside the
 * inn. Nothing was broken: the prompt still asked for JSON, the salvage still
 * turned his prose into speech, and every turn looked like a success from his
 * side. That is exactly why it never ended. A model follows the format its own
 * history shows it, and his history was a hundred replies of prose.
 *
 * These tests are about the way out: after a run of prose the next prompt says
 * so, in terms of what did not happen.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';

import { Autonomous } from '../dist/harness/behavior.js';

const situation = (extra = {}) => ({
  scene: 'reldens-town',
  where: 'town',
  others: [],
  heard: [],
  actions: '- "wait": stay where you are',
  places: '',
  conversation: [],
  wordiness: 35,
  purpose: '',
  known: '',
  strange: false,
  doors: '',
  view: '',
  harping: '',
  notes: [],
  people: '',
  ...extra
});

const memory = { resource: 'r', thread: 't' };

/**
 * An agent that answers with whatever is handed to it, keeping both halves of
 * every call: the moment (the turn being answered, which Mastra stores) and
 * the brief (per-call instructions, which it does not).
 */
function agentSaying(replies) {
  const moments = [];
  const briefs = [];
  let at = 0;
  return {
    moments,
    briefs,
    async generate(moment, options) {
      moments.push(String(moment ?? ''));
      briefs.push(String(options?.instructions ?? ''));
      return { text: replies[Math.min(at++, replies.length - 1)] };
    }
  };
}

// The real shape of Guy's replies: in character, no action anywhere in them.
const PROSE = 'I finish my ale, set the cup down, and step out of the inn. The street is quiet.';
const ACTION = '{"action": "walk", "place": "the inn door"}';

test('one stray line of prose is not worth interrupting a character over', async () => {
  const agent = agentSaying([PROSE, PROSE]);
  const behavior = new Autonomous(agent, 'persona');
  await behavior.next(situation(), memory);
  await behavior.next(situation(), memory);
  assert.ok(
    !agent.moments[1].includes('had no action in them'),
    'a single prose reply is ordinary and gets no correction'
  );
});

test('a run of prose is answered with what did not happen', async () => {
  const agent = agentSaying([PROSE, PROSE, PROSE]);
  const behavior = new Autonomous(agent, 'persona');
  await behavior.next(situation(), memory);
  await behavior.next(situation(), memory);
  await behavior.next(situation(), memory);

  const third = agent.moments[2];
  assert.ok(third.includes('Your last 2 replies had no action in them'), 'it says how many');
  assert.ok(third.includes('You have not moved'), 'and what that cost');
  // In the moment, not the brief: the brief is where "reply with JSON and
  // nothing else" already lives, and is the place that demonstrably failed.
  assert.ok(
    !agent.briefs[2].includes('had no action in them'),
    'the correction is not buried in the per-call instructions'
  );
});

test('the count keeps climbing while the character stays out of format', async () => {
  const agent = agentSaying([PROSE, PROSE, PROSE, PROSE]);
  const behavior = new Autonomous(agent, 'persona');
  for (let turn = 0; turn < 4; turn++) await behavior.next(situation(), memory);
  assert.ok(agent.moments[3].includes('Your last 3 replies had no action in them'));
});

test('answering in the format again clears it', async () => {
  const agent = agentSaying([PROSE, PROSE, ACTION, PROSE]);
  const behavior = new Autonomous(agent, 'persona');
  for (let turn = 0; turn < 4; turn++) await behavior.next(situation(), memory);
  assert.ok(agent.moments[2].includes('had no action in them'), 'corrected while adrift');
  assert.ok(
    !agent.moments[3].includes('had no action in them'),
    'and left alone once it answers properly again'
  );
});

test('a character answering properly is never corrected', async () => {
  const agent = agentSaying([ACTION, ACTION, ACTION]);
  const behavior = new Autonomous(agent, 'persona');
  for (let turn = 0; turn < 3; turn++) await behavior.next(situation(), memory);
  assert.ok(agent.moments.every((moment) => !moment.includes('had no action in them')));
});

test('thinking aloud counts as out of format, since it moves nobody either', async () => {
  // isThinkingAloud() catches this one and returns wait rather than speech.
  // It is still a reply with no action in it, and a run of them is still a
  // character that has stopped answering.
  const aloud = 'It is about fifteen paces north from here, so let me go and walk over there.';
  const agent = agentSaying([aloud, aloud, aloud]);
  const behavior = new Autonomous(agent, 'persona');
  for (let turn = 0; turn < 3; turn++) await behavior.next(situation(), memory);
  assert.ok(agent.moments[2].includes('Your last 2 replies had no action in them'));
});

test('a stray line of prose is still spoken, because most of it is real dialogue', async () => {
  const agent = agentSaying([PROSE, PROSE, PROSE]);
  const behavior = new Autonomous(agent, 'persona');
  const first = await behavior.next(situation(), memory);
  assert.equal(first.action, 'say', 'the character is heard');
  assert.equal(first.message, PROSE);
});

test('past a run, prose stops being spoken so the correction is true', async () => {
  // Two is where a character is told. Four is where it stops being humoured:
  // until then the harness said "nothing you described happened" and then said
  // the prose out loud anyway, and Guy believed what he could see.
  const agent = agentSaying(Array(6).fill(PROSE));
  const behavior = new Autonomous(agent, 'persona');
  const spoken = [];
  for (let turn = 0; turn < 6; turn++) {
    spoken.push((await behavior.next(situation(), memory)).action);
  }
  assert.deepEqual(
    spoken,
    ['say', 'say', 'say', 'wait', 'wait', 'wait'],
    'humoured while it might be dialogue, silent from the fourth on'
  );
});

test('answering properly again earns the character its voice back', async () => {
  const agent = agentSaying([PROSE, PROSE, PROSE, PROSE, PROSE, ACTION, PROSE]);
  const behavior = new Autonomous(agent, 'persona');
  for (let turn = 0; turn < 5; turn++) await behavior.next(situation(), memory);
  const recovered = await behavior.next(situation(), memory);
  assert.equal(recovered.action, 'walk', 'the action lands');
  const afterwards = await behavior.next(situation(), memory);
  assert.equal(afterwards.action, 'say', 'and one prose reply is humoured again');
});

test('going silent never costs the character what it wrote down', async () => {
  // A promise made in the same breath as unusable prose is still a promise.
  const noting = {
    async generate() {
      return { text: 'I head for the door. {"remember":"Barnaby went quiet about the key"}' };
    }
  };
  const behavior = new Autonomous(noting, 'persona');
  let intent;
  for (let turn = 0; turn < 5; turn++) intent = await behavior.next(situation(), memory);
  assert.equal(intent.action, 'wait', 'silent by now');
});
