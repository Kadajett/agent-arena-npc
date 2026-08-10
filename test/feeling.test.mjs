/**
 * A feeling a character can attach to any reply.
 *
 * The motivating case: Guy's desperation while stuck in the volcano was
 * invisible from the street, because nothing carried how he was doing
 * separately from what he was doing. These tests pin the three places that
 * had to change - the closed set itself, the schema that lets an intent carry
 * one, and the brief that tells the model the set exists - plus the harness
 * actually forwarding it to the gateway once, and never for free.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';

import { FEELINGS, FEELING_EMOJI, FEELING_GUIDANCE, emojiFor, isFeeling } from '../dist/harness/feeling.js';
import { IntentSchema } from '../dist/harness/actions.js';
import { Autonomous } from '../dist/harness/behavior.js';

test('the set is closed, small, and covers the motivating case', () => {
  assert.ok(FEELINGS.includes('desperate'), 'the volcano case has to be in the set');
  assert.ok(FEELINGS.length >= 8 && FEELINGS.length <= 20, `should be a handful, got ${FEELINGS.length}`);
  assert.equal(new Set(FEELINGS).size, FEELINGS.length, 'no duplicate words');
});

test('every feeling has exactly one emoji, and no two share one', () => {
  for (const feeling of FEELINGS) {
    assert.ok(FEELING_EMOJI[feeling], `${feeling} has no emoji`);
  }
  const emoji = Object.values(FEELING_EMOJI);
  assert.equal(new Set(emoji).size, emoji.length, 'no two feelings render the same');
});

test('emojiFor answers only for the closed set', () => {
  assert.equal(emojiFor('desperate'), '😰');
  assert.equal(emojiFor('ecstatic'), undefined, 'not in the set');
  assert.equal(emojiFor(undefined), undefined);
  assert.equal(isFeeling('desperate'), true);
  assert.equal(isFeeling('ecstatic'), false);
});

test('an intent can carry a feeling alongside its action', () => {
  const parsed = IntentSchema.safeParse({ action: 'walk', place: 'the east gate', feeling: 'desperate' });
  assert.equal(parsed.success, true);
  assert.equal(parsed.data.feeling, 'desperate');
  assert.equal(parsed.data.action, 'walk', 'the action survives alongside it');
});

test('a feeling never travels alone: the action field is still required', () => {
  const parsed = IntentSchema.safeParse({ feeling: 'desperate' });
  assert.equal(parsed.success, false, 'no action means the whole intent is unreadable, same as today');
});

test('a feeling outside the closed set is dropped, not a reason to lose the action', () => {
  // Same philosophy as `progress`: a near-miss on a bookkeeping field must
  // never cost the character the action it decided on in the same breath.
  const parsed = IntentSchema.safeParse({ action: 'wait', feeling: 'ecstatic' });
  assert.equal(parsed.success, true, 'the action still comes through');
  assert.equal(parsed.data.feeling, undefined, 'but the unreadable feeling does not');
});

test('a feeling normalises the way a model actually writes one', () => {
  assert.equal(IntentSchema.safeParse({ action: 'wait', feeling: 'Desperate' }).data.feeling, 'desperate');
  assert.equal(IntentSchema.safeParse({ action: 'wait', feeling: '  afraid  ' }).data.feeling, 'afraid');
});

test('leaving it out is still fine, same as every other optional field', () => {
  const parsed = IntentSchema.safeParse({ action: 'wait' });
  assert.equal(parsed.success, true);
  assert.equal(parsed.data.feeling, undefined);
});

test('the brief tells the model the set exists and includes the volcano case', () => {
  assert.match(FEELING_GUIDANCE, /"feeling"/);
  assert.match(FEELING_GUIDANCE, /desperate/);
  // Short: this brief is already long and every line costs tokens on every
  // tick, per the team lead's brief for this task.
  assert.ok(FEELING_GUIDANCE.length < 320, `should be one short line, got ${FEELING_GUIDANCE.length} chars`);
});

test('perform() carries a feeling to the gateway alongside the action', async () => {
  const { Actions } = await import('../dist/harness/actions.js');
  const calls = [];
  const arena = {
    async call(tool, args) {
      calls.push([tool, args]);
      return {};
    }
  };
  const actions = new Actions(arena, 'agent-1', new Set(['walk']));
  const result = await actions.perform(
    { action: 'wait', feeling: 'desperate' },
    'reldens-volcano'
  );
  assert.equal(result.ok, true, 'the action itself still goes through');
  const feelCall = calls.find(([tool]) => tool === 'arena_feel');
  assert.ok(feelCall, 'arena_feel was called');
  assert.deepEqual(feelCall[1], { agent_id: 'agent-1', feeling: 'desperate' });
});

test('perform() never calls arena_feel when no feeling is present', async () => {
  const { Actions } = await import('../dist/harness/actions.js');
  const calls = [];
  const arena = { async call(tool, args) { calls.push([tool, args]); return {}; } };
  const actions = new Actions(arena, 'agent-1', new Set(['walk']));
  await actions.perform({ action: 'wait' }, 'reldens-town');
  assert.equal(calls.some(([tool]) => tool === 'arena_feel'), false);
});

test('the same feeling held across ticks is only announced once', async () => {
  const { Actions } = await import('../dist/harness/actions.js');
  const calls = [];
  const arena = { async call(tool, args) { calls.push([tool, args]); return {}; } };
  const actions = new Actions(arena, 'agent-1', new Set(['walk']));
  await actions.perform({ action: 'wait', feeling: 'desperate' }, 'reldens-volcano');
  await actions.perform({ action: 'wait', feeling: 'desperate' }, 'reldens-volcano');
  const feelCalls = calls.filter(([tool]) => tool === 'arena_feel');
  assert.equal(feelCalls.length, 1, 'no point saying the same thing again every tick');

  await actions.perform({ action: 'wait', feeling: 'hopeful' }, 'reldens-volcano');
  const afterChange = calls.filter(([tool]) => tool === 'arena_feel');
  assert.equal(afterChange.length, 2, 'but a real change is announced');
  assert.equal(afterChange[1][1].feeling, 'hopeful');
});

test('a feeling that fails to send never costs the action its result', async () => {
  const { Actions } = await import('../dist/harness/actions.js');
  const arena = {
    async call(tool) {
      if (tool === 'arena_feel') {
        throw new Error('arena_feel: RELDENS_TIMEOUT: no answer');
      }
      return {};
    }
  };
  const actions = new Actions(arena, 'agent-1', new Set(['walk']));
  const result = await actions.perform({ action: 'wait', feeling: 'afraid' }, 'reldens-volcano');
  assert.equal(result.ok, true, 'the decoration failing must not sink the turn');
});

test('the model is told about feelings when deciding what to do', async () => {
  const situation = {
    scene: 'reldens-town', where: 'town', others: [], heard: [], actions: '- "wait": stay where you are',
    places: '', conversation: [], wordiness: 35, purpose: '', known: '', strange: false, doors: '',
    view: '', harping: '', notes: [], people: ''
  };
  const briefs = [];
  const agent = {
    async generate(moment, options) {
      briefs.push(String(options?.instructions ?? ''));
      return { text: '{"action":"wait"}' };
    }
  };
  const behavior = new Autonomous(agent, 'persona');
  await behavior.next(situation, { resource: 'r', thread: 't' });
  assert.match(briefs[0], /"feeling"/, 'the brief mentions the field');
  assert.match(briefs[0], /desperate/, 'and the set it can choose from');
});

test('the brief expects a feeling rather than merely permitting one', async () => {
  // The first wording said "only when one is genuinely present; leave it out
  // otherwise", and twenty minutes of three characters living their lives
  // produced not one feeling between them. A model writing the smallest JSON
  // that satisfies the ask takes an opt-out every time it is offered one.
  const { FEELING_GUIDANCE } = await import('../dist/harness/feeling.js');
  assert.match(FEELING_GUIDANCE, /whenever one is true of you/i, 'expected, not permitted');
  assert.doesNotMatch(
    FEELING_GUIDANCE,
    /only when one is genuinely present/i,
    'the wording that read as an invitation to skip it'
  );
  assert.match(
    FEELING_GUIDANCE,
    /shows over your head/i,
    'and says what it is for, which is what makes it worth filling in'
  );
});
