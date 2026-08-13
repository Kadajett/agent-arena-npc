/**
 * The read-only view onto what the aggregator has found: GET /claims,
 * filterable by tier and authenticity, and GET /health. A real HTTP server
 * on a free port, closed after each test.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { startServer } from '../dist/worldledger/server.js';
import { mergeClaims } from '../dist/worldledger/store.js';

function claim(overrides) {
  return {
    text: 'a claim',
    tier: 'overheard',
    componentCount: 1,
    lineIds: [1],
    seededBy: null,
    authenticity: 'unexamined',
    contradicts: null,
    ...overrides
  };
}

async function withServer(dir, run) {
  const server = startServer(dir, 0);
  await new Promise((resolve) => server.once('listening', resolve));
  const port = server.address().port;
  try {
    await run(`http://127.0.0.1:${port}`);
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
}

test('GET /health reports how many claims are stored', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'worldledger-server-test-'));
  mergeClaims(dir, [claim({ text: 'a' }), claim({ text: 'b' })], '2026-01-01T00:00:00Z');
  await withServer(dir, async (base) => {
    const response = await fetch(`${base}/health`);
    assert.equal(response.status, 200);
    assert.deepEqual(await response.json(), { ok: true, claims: 2 });
  });
});

test('GET /claims with nothing stored yet returns an empty list, not an error', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'worldledger-server-test-'));
  await withServer(dir, async (base) => {
    const response = await fetch(`${base}/claims`);
    assert.equal(response.status, 200);
    const body = await response.json();
    assert.equal(body.total, 0);
    assert.deepEqual(body.claims, []);
  });
});

test('GET /claims filters by tier', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'worldledger-server-test-'));
  mergeClaims(
    dir,
    [claim({ text: 'a read claim', tier: 'read' }), claim({ text: 'an overheard claim', tier: 'overheard' })],
    '2026-01-01T00:00:00Z'
  );
  await withServer(dir, async (base) => {
    const response = await fetch(`${base}/claims?tier=read`);
    const body = await response.json();
    assert.equal(body.count, 1);
    assert.equal(body.claims[0].text, 'a read claim');
  });
});

test('GET /claims filters by authenticity', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'worldledger-server-test-'));
  mergeClaims(
    dir,
    [claim({ text: 'stable one', authenticity: 'stable' }), claim({ text: 'drifting one', authenticity: 'drifting' })],
    '2026-01-01T00:00:00Z'
  );
  await withServer(dir, async (base) => {
    const response = await fetch(`${base}/claims?authenticity=drifting`);
    const body = await response.json();
    assert.equal(body.count, 1);
    assert.equal(body.claims[0].text, 'drifting one');
  });
});

test('GET /claims rejects an unknown tier rather than silently returning nothing', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'worldledger-server-test-'));
  await withServer(dir, async (base) => {
    const response = await fetch(`${base}/claims?tier=definitely-true`);
    assert.equal(response.status, 400);
    assert.equal((await response.json()).error, 'UNKNOWN_TIER');
  });
});

test('GET /claims most-recently-seen first, by default', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'worldledger-server-test-'));
  mergeClaims(dir, [claim({ text: 'older' })], '2026-01-01T00:00:00Z');
  mergeClaims(dir, [claim({ text: 'newer' })], '2026-01-02T00:00:00Z');
  await withServer(dir, async (base) => {
    const body = await (await fetch(`${base}/claims`)).json();
    assert.deepEqual(body.claims.map((c) => c.text), ['newer', 'older']);
  });
});

test('GET /claims respects and caps limit', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'worldledger-server-test-'));
  mergeClaims(
    dir,
    [claim({ text: 'a' }), claim({ text: 'b' }), claim({ text: 'c' })],
    '2026-01-01T00:00:00Z'
  );
  await withServer(dir, async (base) => {
    const body = await (await fetch(`${base}/claims?limit=1`)).json();
    assert.equal(body.count, 1);
    assert.equal(body.total, 3, 'total still reports how many matched before the limit');
  });
});

test('an unknown route is a plain 404, not a crash', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'worldledger-server-test-'));
  await withServer(dir, async (base) => {
    const response = await fetch(`${base}/nope`);
    assert.equal(response.status, 404);
  });
});

test('a non-GET request is rejected rather than treated as a query', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'worldledger-server-test-'));
  await withServer(dir, async (base) => {
    const response = await fetch(`${base}/claims`, { method: 'POST' });
    assert.equal(response.status, 405);
  });
});
