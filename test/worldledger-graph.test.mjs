/**
 * The propagation graph: whether a claim's own shape - who could have heard
 * it from whom, in what room, at what time - looks like a single seed or
 * independent corroboration. No network, no model; every input here is a
 * plain object standing in for what the world API actually returns.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';

import { buildComponents, classifyAuthenticity, classifyClaim, hasReadEvidence, seededSignal } from '../dist/worldledger/graph.js';

function line(id, from, scene, at) {
  return { id, from, scene, at, message: 'irrelevant here' };
}

function byId(lines) {
  return new Map(lines.map((l) => [l.id, l]));
}

test('the same speaker in two different rooms is one component', () => {
  const lines = [line(1, 'Bolo', 'town', '2026-01-01T10:00:00Z'), line(2, 'Bolo', 'inn', '2026-01-01T11:00:00Z')];
  const components = buildComponents([1, 2], byId(lines));
  assert.equal(components.length, 1);
});

test('two different speakers, same room, close in time, join a component', () => {
  const lines = [
    line(1, 'Aveline', 'inn', '2026-01-01T10:00:00Z'),
    line(2, 'Bolo', 'inn', '2026-01-01T10:05:00Z')
  ];
  const components = buildComponents([1, 2], byId(lines));
  assert.equal(components.length, 1);
});

test('two different speakers, same room, far apart in time, stay separate', () => {
  const lines = [
    line(1, 'Aveline', 'inn', '2026-01-01T10:00:00Z'),
    line(2, 'Bolo', 'inn', '2026-01-01T18:00:00Z')
  ];
  const components = buildComponents([1, 2], byId(lines));
  assert.equal(components.length, 2);
});

test('two different speakers, never in the same room, stay separate', () => {
  const lines = [
    line(1, 'Aveline', 'town', '2026-01-01T10:00:00Z'),
    line(2, 'Bolo', 'inn', '2026-01-01T10:01:00Z')
  ];
  const components = buildComponents([1, 2], byId(lines));
  assert.equal(components.length, 2);
});

test('a chain through an intermediate room-mate merges into one component', () => {
  // Aveline and Bolo share the inn; Bolo later carries it to town where Jack hears it.
  const lines = [
    line(1, 'Aveline', 'inn', '2026-01-01T10:00:00Z'),
    line(2, 'Bolo', 'inn', '2026-01-01T10:05:00Z'),
    line(3, 'Bolo', 'town', '2026-01-01T11:00:00Z'),
    line(4, 'SaneJack', 'town', '2026-01-01T11:02:00Z')
  ];
  const components = buildComponents([1, 2, 3, 4], byId(lines));
  assert.equal(components.length, 1);
});

test('a claim with one component and no supporting evidence reads as overheard', () => {
  const lines = [line(1, 'Bolo', 'inn', '2026-01-01T10:00:00Z')];
  const claim = classifyClaim('Barnaby pays suppliers late', [1], byId(lines), new Map(), new Map(), true, null);
  assert.equal(claim.tier, 'overheard');
  assert.equal(claim.componentCount, 1);
});

test('two independent origins read as possibly true', () => {
  const lines = [
    line(1, 'Bolo', 'inn', '2026-01-01T10:00:00Z'),
    line(2, 'Marren', 'arena-grassland', '2026-01-02T09:00:00Z')
  ];
  const claim = classifyClaim('Barnaby pays suppliers late', [1, 2], byId(lines), new Map(), new Map(), true, null);
  assert.equal(claim.tier, 'possibly-true');
  assert.equal(claim.componentCount, 2);
});

test('activity showing a sign read near the root upgrades the claim to read', () => {
  const lines = [line(1, 'Bolo', 'town', '2026-01-01T10:00:00Z')];
  const activity = new Map([
    [
      'bolo',
      [{ id: 1, at: '2026-01-01T09:58:00Z', player: 'Bolo', tool: 'arena_talk_to', ok: true, args: { name: 'the notice board' }, error: null }]
    ]
  ]);
  assert.equal(hasReadEvidence([1], byId(lines), activity), true);
  const claim = classifyClaim('The east gate is closed for repairs', [1], byId(lines), activity, new Map(), true, null);
  assert.equal(claim.tier, 'read');
});

test('activity against an ordinary person does not count as read evidence', () => {
  const lines = [line(1, 'Bolo', 'town', '2026-01-01T10:00:00Z')];
  const activity = new Map([
    [
      'bolo',
      [{ id: 1, at: '2026-01-01T09:58:00Z', player: 'Bolo', tool: 'arena_talk_to', ok: true, args: { name: 'Aveline' }, error: null }]
    ]
  ]);
  assert.equal(hasReadEvidence([1], byId(lines), activity), false);
});

test('a root speaker planning it in their own thoughts flags as probably seeded', () => {
  const lines = [line(1, 'Bolo', 'inn', '2026-01-01T10:00:00Z')];
  const thoughts = new Map([
    ['bolo', [{ at: '2026-01-01T09:55:00Z', thought: 'Time to plant a doubt about the books and see if he bites.', source: 'harness' }]]
  ]);
  const signal = seededSignal([1], byId(lines), thoughts);
  assert.ok(signal);
  assert.equal(signal.player, 'Bolo');
  const claim = classifyClaim('The books do not add up', [1], byId(lines), new Map(), thoughts, true, null);
  assert.equal(claim.tier, 'probably-seeded');
});

test('a thought with no fabrication language does not flag anything', () => {
  const lines = [line(1, 'Bolo', 'inn', '2026-01-01T10:00:00Z')];
  const thoughts = new Map([['bolo', [{ at: '2026-01-01T09:55:00Z', thought: 'Wondering if the soup is turnip again.', source: 'harness' }]]]);
  assert.equal(seededSignal([1], byId(lines), thoughts), null);
});

test('a thought after the claim was made is too late to be the seed', () => {
  const lines = [line(1, 'Bolo', 'inn', '2026-01-01T10:00:00Z')];
  const thoughts = new Map([
    ['bolo', [{ at: '2026-01-01T10:10:00Z', thought: 'Glad that landed - planted it well.', source: 'harness' }]]
  ]);
  assert.equal(seededSignal([1], byId(lines), thoughts), null);
});

test('read evidence wins even when the same claim also has independent origins', () => {
  const lines = [
    line(1, 'Bolo', 'town', '2026-01-01T10:00:00Z'),
    line(2, 'Marren', 'arena-grassland', '2026-01-02T09:00:00Z')
  ];
  const activity = new Map([
    [
      'bolo',
      [{ id: 1, at: '2026-01-01T09:58:00Z', player: 'Bolo', tool: 'arena_observe', ok: true, args: { name: 'a posted notice' }, error: null }]
    ]
  ]);
  const claim = classifyClaim('The east gate is closed', [1, 2], byId(lines), activity, new Map(), true, null);
  assert.equal(claim.tier, 'read');
  assert.equal(claim.componentCount, 2);
});

test('classifyAuthenticity: a single telling has nothing to have drifted from yet', () => {
  assert.equal(classifyAuthenticity([1], true, null), 'unexamined');
  assert.equal(classifyAuthenticity([1], false, null), 'unexamined');
});

test('classifyAuthenticity: multiple tellings that agree are stable', () => {
  assert.equal(classifyAuthenticity([1, 2], true, null), 'stable');
});

test('classifyAuthenticity: multiple tellings that disagree on specifics are drifting', () => {
  assert.equal(classifyAuthenticity([1, 2], false, null), 'drifting');
});

test('classifyAuthenticity: contradicting an established claim outranks everything else', () => {
  assert.equal(classifyAuthenticity([1, 2], true, 'Barnaby has always kept his own books.'), 'contradicted');
  assert.equal(classifyAuthenticity([1], true, 'Barnaby has always kept his own books.'), 'contradicted');
});

test('classifyClaim carries authenticity and contradicts through to the result', () => {
  const lines = [line(1, 'Bolo', 'inn', '2026-01-01T10:00:00Z'), line(2, 'Bolo', 'town', '2026-01-01T10:20:00Z')];
  const drifting = classifyClaim('The count was seven', [1, 2], byId(lines), new Map(), new Map(), false, null);
  assert.equal(drifting.authenticity, 'drifting');
  assert.equal(drifting.contradicts, null);

  const contradicted = classifyClaim(
    'Barnaby never kept his own books',
    [1],
    byId(lines.slice(0, 1)),
    new Map(),
    new Map(),
    true,
    'Barnaby has always kept his own books.'
  );
  assert.equal(contradicted.authenticity, 'contradicted');
  assert.equal(contradicted.contradicts, 'Barnaby has always kept his own books.');
});

test('an id with no matching line is dropped rather than crashing the graph', () => {
  const lines = [line(1, 'Bolo', 'inn', '2026-01-01T10:00:00Z')];
  const components = buildComponents([1, 999], byId(lines));
  assert.equal(components.length, 1);
  assert.deepEqual(components[0], [1]);
});
