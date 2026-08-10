/**
 * One failed tool call kills a character's compaction for good.
 *
 * Mastra's token counter handles four tool-invocation states and throws on
 * "output-error". Compaction runs inside a workflow step, so the throw is
 * swallowed: the character carries on, observation fails silently every tick
 * from then on, and history stops being folded down at all.
 *
 * Guy had one such part, an `explore` that failed. The Wanderer had two. Barnaby
 * had none, and was the only one of the three not erroring every turn.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { repairFailedToolCalls, restartWhenCompactionJams } from '../dist/harness/memory.js';

/** A stored message, in the envelope shape Mastra actually writes. */
function message(id, parts) {
  return {
    id,
    content: JSON.stringify({ format: 2, parts, content: '' })
  };
}

const failedCall = {
  type: 'tool-invocation',
  toolInvocation: {
    state: 'output-error',
    toolName: 'explore',
    toolCallId: 'call-1',
    errorText: 'nowhere left to go'
  }
};

const goodCall = {
  type: 'tool-invocation',
  toolInvocation: { state: 'result', toolName: 'look', toolCallId: 'call-2', result: { ok: true } }
};

function dbWith(messages) {
  const dir = mkdtempSync(join(tmpdir(), 'arena-memory-'));
  const path = join(dir, 'character.db');
  const db = new DatabaseSync(path);
  db.exec('CREATE TABLE mastra_messages (id TEXT PRIMARY KEY, content TEXT)');
  const insert = db.prepare('INSERT INTO mastra_messages (id, content) VALUES (?, ?)');
  for (const m of messages) insert.run(m.id, m.content);
  db.close();
  return { path, dir };
}

const partsOf = (path, id) => {
  const db = new DatabaseSync(path, { readOnly: true });
  const row = db.prepare('SELECT content FROM mastra_messages WHERE id = ?').get(id);
  db.close();
  return JSON.parse(row.content).parts;
};

test('a failed tool call becomes a call that completed with an error', () => {
  const { path, dir } = dbWith([message('m1', [failedCall])]);
  try {
    assert.equal(repairFailedToolCalls(path), 1, 'one message repaired');
    const [part] = partsOf(path, 'm1');
    assert.equal(part.toolInvocation.state, 'result', 'a state the token counter accepts');
    assert.deepEqual(
      part.toolInvocation.result,
      { error: 'nowhere left to go' },
      'and the failure is kept rather than pretending the call never happened'
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('tool calls that worked are left exactly as they were', () => {
  const { path, dir } = dbWith([message('m1', [goodCall])]);
  try {
    assert.equal(repairFailedToolCalls(path), 0, 'nothing to repair');
    assert.deepEqual(partsOf(path, 'm1'), [goodCall], 'and nothing touched');
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('only the failed part of a mixed message is rewritten', () => {
  const text = { type: 'text', text: '{"action": "wait"}' };
  const { path, dir } = dbWith([message('m1', [text, failedCall, goodCall])]);
  try {
    assert.equal(repairFailedToolCalls(path), 1);
    const parts = partsOf(path, 'm1');
    assert.deepEqual(parts[0], text, 'the reply itself is untouched');
    assert.equal(parts[1].toolInvocation.state, 'result');
    assert.deepEqual(parts[2], goodCall, 'and the call that worked is untouched');
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('a character on its first boot has no database and that is fine', () => {
  assert.equal(
    repairFailedToolCalls(join(tmpdir(), 'arena-memory-that-does-not-exist', 'nobody.db')),
    0,
    'no throw, no character kept out of the world over it'
  );
});

test('running it twice changes nothing the second time', () => {
  const { path, dir } = dbWith([message('m1', [failedCall])]);
  try {
    assert.equal(repairFailedToolCalls(path), 1);
    assert.equal(repairFailedToolCalls(path), 0, 'idempotent, so booting repeatedly is free');
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

/**
 * Repairing once at startup was not enough, which took a live character to
 * find out. Cutter came up clean, poisoned himself three minutes in, and would
 * have errored on every tick until somebody happened to redeploy him.
 *
 * The obvious answer, repairing on a timer, was worse. The character already
 * holds that SQLite file open through LibSQL in WAL mode, and a second
 * connection opening and closing underneath takes the write-ahead log out from
 * under the first. The Wanderer, who has the largest history, went from working
 * to SQLITE_IOERR on every tick inside ninety seconds of that deploy.
 *
 * So the file is left alone and Mastra's own complaint is listened for instead.
 * A jammed character leaves, and the startup repair, which was always safe
 * because nothing has the database open yet, puts it right on the way back in.
 */
test('a jam is noticed from what Mastra says, without opening the database', () => {
  const wrote = console.error;
  const left = [];
  try {
    restartWhenCompactionJams((why) => left.push(why));
    console.error("Unhandled tool-invocation state 'output-error' in token counting");
  } finally {
    console.error = wrote;
  }
  assert.equal(left.length, 1, 'the jam should be noticed');
  assert.match(left[0], /compaction/);
});

test('the jam still reaches the logs rather than being swallowed by the listener', () => {
  const wrote = console.error;
  const through = [];
  console.error = (...args) => through.push(args.join(' '));
  try {
    restartWhenCompactionJams(() => {});
    console.error('Encountered error during memory observation: something');
  } finally {
    console.error = wrote;
  }
  assert.equal(through.length, 1, 'listening must not mean intercepting');
  assert.match(through[0], /memory observation/);
});

test('leaving happens once, however many ticks go on complaining', () => {
  const wrote = console.error;
  const left = [];
  try {
    restartWhenCompactionJams((why) => left.push(why));
    for (let i = 0; i < 5; i += 1) {
      console.error("Unhandled tool-invocation state 'output-error'");
    }
  } finally {
    console.error = wrote;
  }
  assert.equal(left.length, 1, 'a character only leaves once, and should only say so once');
});

test('ordinary errors are not mistaken for a jam', () => {
  const wrote = console.error;
  const left = [];
  try {
    restartWhenCompactionJams((why) => left.push(why));
    console.error('Error: connection reset by peer');
    console.error('SQLITE_IOERR: disk I/O error');
  } finally {
    console.error = wrote;
  }
  assert.equal(left.length, 0, 'leaving on every error would be a restart loop');
});
