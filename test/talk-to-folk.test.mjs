/**
 * Talking to somebody standing in the world, not just answering whoever
 * spoke first.
 *
 * talk_to and answer_npc are found by name against whatever notices() was
 * just told - the same observation the harness already fetched - never by
 * the gateway's own object id, which no character would ever think in. A
 * character without the capability cannot start one of these conversations
 * at all, the same way Barnaby cannot walk out of his own inn without the
 * "doors" capability.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Actions } from '../dist/harness/actions.js';

/** A stand-in for the MCP client, queuing one canned reply per call(). */
class FakeArena {
  constructor(replies = []) {
    this.replies = [...replies];
    this.calls = [];
  }

  async call(name, args) {
    this.calls.push({ name, args });
    if (this.replies.length === 0) {
      throw new Error(`FakeArena: no reply queued for ${name}`);
    }
    return this.replies.shift();
  }
}

const ALFRED = {
  objectId: 42,
  objectIndex: 'npcs12',
  label: 'Alfred',
  kind: 'npc',
  interactable: true,
  distanceFromSelf: 10
};

const DIALOG_WITH_CHOICES = {
  opened: true,
  objectId: 42,
  title: 'Alfred',
  content: 'Wolves are back in the north woods.',
  options: { '1': 'Ask about the wolves', '2': 'Never mind' }
};

const DIALOG_NO_CHOICES = {
  opened: true,
  objectId: 42,
  title: 'Alfred',
  content: 'Safe travels.',
  options: null
};

const TOO_FAR = {
  opened: false,
  objectId: 42,
  reason: 'TOO_FAR_AWAY',
  message: 'You are too far away to talk to them. Walk closer and try again.'
};

function actionsWith(capabilities, replies = []) {
  const arena = new FakeArena(replies);
  const actions = new Actions(arena, 'agent-1', new Set(capabilities));
  return { arena, actions };
}

test('a character without talk_to_folk cannot start a conversation', async () => {
  const { actions } = actionsWith([]);
  actions.notices([ALFRED]);
  const result = await actions.talkTo('Alfred');
  assert.equal(result.ok, false);
  assert.match(result.note, /does not strike up conversation/);
});

test('talking to somebody not actually here fails by name, not by crashing', async () => {
  const { actions } = actionsWith(['talk_to_folk']);
  actions.notices([ALFRED]);
  const result = await actions.talkTo('Barnaby');
  assert.equal(result.ok, false);
  assert.match(result.note, /no "Barnaby" here/);
});

test('talking to somebody nearby opens the dialog by their object id', async () => {
  const { arena, actions } = actionsWith(['talk_to_folk'], [DIALOG_WITH_CHOICES]);
  actions.notices([ALFRED]);
  const result = await actions.talkTo('alfred'); // case shouldn't matter
  assert.equal(result.ok, true);
  assert.match(result.note, /Wolves are back in the north woods/);
  assert.match(result.note, /You can answer:/);
  assert.deepEqual(arena.calls[0], {
    name: 'arena_talk_to',
    args: { agent_id: 'agent-1', object_id: 42 }
  });
});

test('what is offered shows up in describe() once a conversation is open', async () => {
  const { actions } = actionsWith(['talk_to_folk'], [DIALOG_WITH_CHOICES]);
  actions.notices([ALFRED]);
  await actions.talkTo('Alfred');
  const described = actions.describe('reldens-town');
  assert.match(described, /"answer_npc": answer Alfred/);
  assert.match(described, /"Ask about the wolves"/);
});

test('before anybody has been spoken to, describe() offers talk_to by name', () => {
  const { actions } = actionsWith(['talk_to_folk']);
  actions.notices([ALFRED]);
  const described = actions.describe('reldens-town');
  assert.match(described, /"talk_to": start a conversation. Needs: target, one of "Alfred"/);
});

test('answering without a conversation open fails plainly', async () => {
  const { actions } = actionsWith(['talk_to_folk']);
  const result = await actions.answerNpc('the wolves');
  assert.equal(result.ok, false);
  assert.match(result.note, /not in the middle of talking to anyone/);
});

test('an answer is matched against what was actually offered, not just its key', async () => {
  const { arena, actions } = actionsWith(
    ['talk_to_folk'],
    [DIALOG_WITH_CHOICES, DIALOG_NO_CHOICES]
  );
  actions.notices([ALFRED]);
  await actions.talkTo('Alfred');
  const result = await actions.answerNpc('ask about the wolves');
  assert.equal(result.ok, true);
  assert.deepEqual(arena.calls[1], {
    name: 'arena_choose',
    args: { agent_id: 'agent-1', object_id: 42, option_key: '1' }
  });
});

test('an answer that matches nothing offered is refused rather than sent blind', async () => {
  const { arena, actions } = actionsWith(['talk_to_folk'], [DIALOG_WITH_CHOICES]);
  actions.notices([ALFRED]);
  await actions.talkTo('Alfred');
  const result = await actions.answerNpc('sing a song');
  assert.equal(result.ok, false);
  assert.match(result.note, /not one of the choices/);
  assert.equal(arena.calls.length, 1, 'never called arena_choose with a guess');
});

test('too far away comes back as a plain message, not a thrown error', async () => {
  const { actions } = actionsWith(['talk_to_folk'], [TOO_FAR]);
  actions.notices([ALFRED]);
  const result = await actions.talkTo('Alfred');
  assert.equal(result.ok, false);
  assert.equal(result.note, TOO_FAR.message);
});

test('a refusal closes the conversation, so answering afterwards fails cleanly', async () => {
  const { actions } = actionsWith(['talk_to_folk'], [TOO_FAR]);
  actions.notices([ALFRED]);
  await actions.talkTo('Alfred');
  const result = await actions.answerNpc('anything');
  assert.equal(result.ok, false);
  assert.match(result.note, /not in the middle of talking to anyone/);
});

test('what the NPC said is handed to the harness once, for memory, then cleared', async () => {
  const { actions } = actionsWith(['talk_to_folk'], [DIALOG_WITH_CHOICES]);
  actions.notices([ALFRED]);
  await actions.talkTo('Alfred');
  const told = actions.takeNpcReply();
  assert.deepEqual(told, { from: 'Alfred', said: 'Alfred: Wolves are back in the north woods.' });
  assert.equal(actions.takeNpcReply(), null, 'reading it once empties it');
});

test('a refusal leaves nothing for the harness to remember', async () => {
  const { actions } = actionsWith(['talk_to_folk'], [TOO_FAR]);
  actions.notices([ALFRED]);
  await actions.talkTo('Alfred');
  assert.equal(actions.takeNpcReply(), null);
});

test('perform() routes talk_to and answer_npc through the same intent shape as everything else', async () => {
  const { actions } = actionsWith(['talk_to_folk'], [DIALOG_WITH_CHOICES]);
  actions.notices([ALFRED]);
  const result = await actions.perform({ action: 'talk_to', target: 'Alfred' }, 'reldens-town');
  assert.equal(result.ok, true);
});
