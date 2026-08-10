/**
 * Opinions a character actually holds, rather than reports each time.
 *
 * The point of these is the asymmetry. Something happening again makes a
 * character surer; something happening the other way wears the view down, and
 * only a worn-down view can be replaced. Without that, "opinion" is just the
 * last thing that happened with a longer name on it, and a character who liked
 * the shore for a week would hate it after one bad afternoon.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';

import {
  WorkingMemorySchema,
  describeOpinions,
  formOpinion
} from '../dist/harness/memory.js';

const blank = () => WorkingMemorySchema.parse({});

const about = (state, name) => state.opinions.find((o) => o.about === name);

test('a first impression is recorded, and reads like one', () => {
  const state = formOpinion(blank(), {
    about: 'the crypt',
    kind: 'region',
    stance: 'wary',
    why: 'something moved down there and I never saw what'
  });
  const view = about(state, 'the crypt');
  assert.equal(view.stance, 'wary');
  assert.equal(view.held, 1);
  assert.match(describeOpinions(state), /first impression/);
});

test('the same view again makes it firmer instead of writing it down twice', () => {
  let state = blank();
  for (let time = 0; time < 3; time++) {
    state = formOpinion(state, {
      about: 'the crypt',
      kind: 'region',
      stance: 'wary',
      why: 'it happened again'
    });
  }
  assert.equal(state.opinions.length, 1, 'one opinion, not three');
  assert.equal(about(state, 'the crypt').held, 3);
  assert.match(describeOpinions(state), /proved right more than once/);
});

test('one bad afternoon does not turn a held view around', () => {
  let state = blank();
  for (let time = 0; time < 3; time++) {
    state = formOpinion(state, {
      about: 'the shore',
      kind: 'region',
      stance: 'likes',
      why: 'quiet, and the fisherman talks'
    });
  }
  state = formOpinion(state, {
    about: 'the shore',
    kind: 'region',
    stance: 'hates',
    why: 'a crab took a piece out of me'
  });
  const view = about(state, 'the shore');
  assert.equal(view.stance, 'likes', 'it still likes the shore');
  assert.equal(view.held, 2, 'just less certainly than before');
});

test('but a run of them does', () => {
  // Three good afternoons, so the view is genuinely held.
  let state = blank();
  for (let time = 0; time < 3; time++) {
    state = formOpinion(state, {
      about: 'the shore',
      kind: 'region',
      stance: 'likes',
      why: 'quiet, and the fisherman talks'
    });
  }
  // It takes as many bad ones to wear that down as it took to build it up,
  // and one more on top to actually turn it.
  for (let time = 0; time < 3; time++) {
    state = formOpinion(state, {
      about: 'the shore',
      kind: 'region',
      stance: 'hates',
      why: 'crabs, every time'
    });
  }
  const view = about(state, 'the shore');
  assert.equal(view.stance, 'hates', 'the view finally turns');
  assert.equal(view.held, 1, 'and a freshly changed mind is not a conviction');
  assert.equal(view.why, 'crabs, every time', 'with the reason that turned it');
});

test('a first impression, being untested, turns at the first contrary thing', () => {
  // The other side of the same rule: nothing is being protected here, so there
  // is no reason to make a character defend a view it only just formed.
  let state = formOpinion(blank(), {
    about: 'the shore',
    kind: 'region',
    stance: 'likes',
    why: 'looked quiet'
  });
  state = formOpinion(state, {
    about: 'the shore',
    kind: 'region',
    stance: 'hates',
    why: 'a crab took a piece out of me'
  });
  const view = about(state, 'the shore');
  assert.equal(view.stance, 'hates');
  assert.equal(view.held, 1);
});

test('a character can hold opinions about things that are not people', () => {
  let state = blank();
  const subjects = [
    { about: 'Barnaby', kind: 'person', stance: 'trusts', why: 'he answered straight' },
    { about: 'the volcano', kind: 'region', stance: 'afraid-of', why: 'I was in it for years' },
    { about: "Barnaby's inn", kind: 'building', stance: 'likes', why: 'warm, and the ale is fine' },
    { about: 'Old Fennimore', kind: 'npc', stance: 'unimpressed', why: 'all talk about a rock' },
    { about: 'the Ashling', kind: 'monster', stance: 'wants-another-go', why: 'it had me last time' }
  ];
  for (const subject of subjects) state = formOpinion(state, subject);
  assert.equal(state.opinions.length, 5);
  const described = describeOpinions(state);
  for (const subject of subjects) {
    assert.ok(described.includes(subject.about), `${subject.about} is written out`);
  }
  assert.match(described, /wants another go/, 'stances read as words, not slugs');
});

test('the same name as two different sorts of thing is two opinions', () => {
  // A character can think one thing of the innkeeper and another of his inn.
  let state = formOpinion(blank(), {
    about: 'the inn',
    kind: 'building',
    stance: 'likes',
    why: 'warm'
  });
  state = formOpinion(state, {
    about: 'the inn',
    kind: 'npc',
    stance: 'wary',
    why: 'whoever keeps the ledger writes down more than they say'
  });
  assert.equal(state.opinions.length, 2);
});

test('what it has held longest is what comes to mind first', () => {
  let state = formOpinion(blank(), {
    about: 'the woods',
    kind: 'region',
    stance: 'curious-about',
    why: 'never been'
  });
  for (let time = 0; time < 4; time++) {
    state = formOpinion(state, {
      about: 'the volcano',
      kind: 'region',
      stance: 'afraid-of',
      why: 'years of it'
    });
  }
  const described = describeOpinions(state);
  assert.ok(
    described.indexOf('the volcano') < described.indexOf('the woods'),
    'the settled view leads'
  );
});

test('a character carrying nothing says nothing', () => {
  assert.equal(describeOpinions(blank()), '');
  assert.equal(describeOpinions(null), '');
});

test('an opinion about nothing is not an opinion', () => {
  const state = formOpinion(blank(), {
    about: '   ',
    kind: 'region',
    stance: 'likes',
    why: 'nowhere'
  });
  assert.equal(state.opinions.length, 0);
});

test('when it is carrying too many, the barely-tested one goes', () => {
  let state = blank();
  // One conviction, then enough first impressions to overflow.
  state = formOpinion(state, {
    about: 'the volcano',
    kind: 'region',
    stance: 'afraid-of',
    why: 'years of it'
  });
  for (let time = 0; time < 5; time++) {
    state = formOpinion(state, {
      about: 'the volcano',
      kind: 'region',
      stance: 'afraid-of',
      why: 'still true'
    });
  }
  for (let n = 0; n < 30; n++) {
    state = formOpinion(state, {
      about: `passing thought ${n}`,
      kind: 'person',
      stance: 'neutral',
      why: 'met them once'
    });
  }
  assert.ok(state.opinions.length <= 24, 'it does not carry everything forever');
  assert.ok(about(state, 'the volcano'), 'and what it is sure of is what it keeps');
});
