/**
 * A character with pockets: carrying things, drinking them, and shopping.
 *
 * The world has had an economy in it the whole time and no character could
 * reach any of it. What the harness adds is the part a person actually
 * touches: knowing what is in your own satchel without asking, drinking what
 * you are carrying, taking what a monster dropped, and standing at a
 * merchant's counter - the last of which is a capability of its own, because
 * a character who kills boars for a living has no business haggling unless
 * somebody wrote that into it.
 *
 * Nothing here names an item that has to exist. Every pelt and tonic below is
 * invented on the spot: the catalogue lives in the world and is somebody
 * else's to seed, and this side of it must keep working whatever ends up in
 * it.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';

import { Actions, IntentSchema, CAPABILITIES } from '../dist/harness/actions.js';
import { describeSituation } from '../dist/harness/behavior.js';

const SCENE = 'reldens-town';
const GUY = 'agent-guy';

const GIMLY = {
  objectId: 10,
  objectIndex: 'npc-layer560',
  label: 'Gimly',
  kind: 'npc',
  isMerchant: true,
  tileX: 5,
  tileY: 5
};
const BARNABY = {
  objectId: 11,
  objectIndex: 'npc-layer20',
  label: 'Barnaby',
  kind: 'npc',
  isMerchant: false,
  tileX: 2,
  tileY: 2
};

function arenaWhere(replies = {}) {
  const calls = [];
  return {
    calls,
    async call(tool, args) {
      calls.push({ tool, args });
      if (tool === 'arena_observe') {
        return { scene: SCENE, objects: [], carrying: [], drops: [], ownPlayer: { state: { x: 0, y: 0 } } };
      }
      if (tool in replies) {
        const reply = replies[tool];
        return 'function' === typeof reply ? reply(args) : reply;
      }
      return {};
    }
  };
}

const shopper = (arena, capabilities = ['trade', 'money', 'speak']) =>
  new Actions(arena, GUY, new Set(capabilities));

const tonic = {
  key: 'tonic', label: 'Tonic', quantity: 3, usable: true, equipment: false, equipped: false
};
const pelt = {
  key: 'pelt', label: 'Pelt', quantity: 1, usable: false, equipment: false, equipped: false
};
const blade = {
  key: 'blade', label: 'Blade', quantity: 1, usable: false, equipment: true, equipped: true
};

// --- what the character reads back ---------------------------------------

test('an empty satchel says nothing at all rather than saying it is empty', () => {
  const actions = shopper(arenaWhere());
  actions.holds([], []);
  assert.equal(actions.carryingLine(), '');
  // And the situation it goes into is silent about it too: a line that never
  // changes is a line a character starts answering.
  const said = describeSituation({
    where: 'town', others: [], heard: [], actions: '', places: '', conversation: [],
    wordiness: 30, purpose: '', notes: [], people: '', known: '', strange: false,
    doors: '', view: '', carrying: actions.carryingLine(), harping: ''
  });
  assert.doesNotMatch(said, /carrying/i);
});

test('what it is carrying is one compact line, worn things marked as worn', () => {
  const actions = shopper(arenaWhere());
  actions.holds([tonic, pelt, blade], []);
  const line = actions.carryingLine();
  assert.equal(line, 'You are carrying: Tonic x3, Pelt, Blade (worn).');
  assert.equal(line.split('\n').length, 1, 'one line, however much is in the bag');
  const said = describeSituation({
    where: 'town', others: [], heard: [], actions: '', places: '', conversation: [],
    wordiness: 30, purpose: '', notes: [], people: '', known: '', strange: false,
    doors: '', view: '', carrying: line, harping: ''
  });
  assert.match(said, /You are carrying: Tonic x3/);
});

// --- what it is offered --------------------------------------------------

test('the merchant is named in the actions, and only for somebody who may haggle', () => {
  const trader = shopper(arenaWhere());
  trader.notices([GIMLY, BARNABY]);
  trader.holds([pelt, blade], []);
  const offered = trader.describe(SCENE);
  assert.match(offered, /"buy": buy from Gimly/);
  // Sellable is what is loose, never what is being worn.
  assert.match(offered, /"sell": sell to Gimly\. Needs: item, one of "Pelt"/);
  assert.doesNotMatch(offered, /"Blade"/, 'a worn blade is not on the counter');

  const brawler = shopper(arenaWhere(), ['fight', 'money']);
  brawler.notices([GIMLY]);
  brawler.holds([pelt], []);
  assert.doesNotMatch(brawler.describe(SCENE), /buy|sell/, 'fighting is not a licence to haggle');
});

test('with no merchant standing here, nothing about shops is offered at all', () => {
  const actions = shopper(arenaWhere());
  actions.notices([BARNABY]);
  actions.holds([pelt], []);
  assert.doesNotMatch(actions.describe(SCENE), /"buy"|"sell"/);
});

test('using something is offered to anybody carrying something usable, trade or not', () => {
  const actions = shopper(arenaWhere(), ['fight']);
  actions.holds([tonic, blade], []);
  const offered = actions.describe(SCENE);
  assert.match(offered, /"use_item".*"Tonic"/);
  assert.doesNotMatch(offered, /"use_item".*"Blade"/, 'a blade is worn, not drunk');

  const emptyHanded = shopper(arenaWhere(), ['fight']);
  emptyHanded.holds([], []);
  assert.doesNotMatch(emptyHanded.describe(SCENE), /use_item/);
});

test('picking up is only offered when something is actually lying there', () => {
  const actions = shopper(arenaWhere(), ['fight']);
  actions.holds([], []);
  assert.doesNotMatch(actions.describe(SCENE), /pick_up/);
  actions.holds([], [{ dropId: 'drop-pelt-a1b2c3d410', itemKey: 'pelt', distanceFromSelf: 20 }]);
  assert.match(actions.describe(SCENE), /"pick_up".*pelt/);
});

// --- reading a near miss -------------------------------------------------

test('the words a person uses for shopping are read as the actions they mean', () => {
  const meant = (said) => IntentSchema.safeParse({ action: said }).data?.action;
  assert.equal(meant('purchase'), 'buy');
  assert.equal(meant('shop'), 'buy');
  assert.equal(meant('barter'), 'buy');
  assert.equal(meant('vend'), 'sell');
  assert.equal(meant('hawk'), 'sell');
  assert.equal(meant('drink'), 'use_item');
  assert.equal(meant('eat'), 'use_item');
  assert.equal(meant('consume'), 'use_item');
  assert.equal(meant('take'), 'pick_up');
  assert.equal(meant('loot'), 'pick_up');
  assert.equal(meant('Pick Up'), 'pick_up');
  // The real ones are never rewritten, and "use" keeps meaning a skill,
  // which is what it has meant to these characters all along.
  assert.equal(meant('buy'), 'buy');
  assert.equal(meant('use_item'), 'use_item');
  assert.equal(meant('use'), 'use_skill');
});

test('how many is read whether it arrives as a number or as words around one', () => {
  assert.equal(IntentSchema.safeParse({ action: 'buy', item: 'tonic', quantity: 2 }).data.quantity, 2);
  assert.equal(IntentSchema.safeParse({ action: 'buy', item: 'tonic', quantity: '3' }).data.quantity, 3);
  // Nonsense in the quantity must never cost the whole intent: the action and
  // the item are still perfectly good and the count falls back to one.
  const vague = IntentSchema.safeParse({ action: 'buy', item: 'tonic', quantity: 'a few' });
  assert.equal(vague.success, true);
  assert.equal(vague.data.quantity, undefined);
  assert.equal(vague.data.item, 'tonic');
});

test('trade is a capability the harness knows about, like every other one', () => {
  assert.ok(CAPABILITIES.includes('trade'));
});

// --- buying and selling --------------------------------------------------

test('buying with nothing named asks what is for sale instead of failing', async () => {
  const arena = arenaWhere({
    arena_trade_with: {
      opened: true,
      merchant: 'Gimly',
      side: 'buy',
      offers: [{ label: 'Tonic', key: 'tonic', price: { itemKey: 'coins', quantity: 5 } }]
    }
  });
  const actions = shopper(arena);
  actions.notices([GIMLY]);
  actions.holds([], []);

  const result = await actions.buy(undefined, undefined);

  assert.equal(result.ok, true);
  assert.match(result.note, /Gimly sells: Tonic for 5 coins/);
  const asked = arena.calls.find((call) => call.tool === 'arena_trade_with');
  assert.equal(asked.args.object_id, 10, 'the merchant is named by the id, never by the label');
  assert.equal(asked.args.side, 'buy');
});

test('buying names the merchant, the item and the count, and reports the price', async () => {
  const arena = arenaWhere({
    arena_buy: {
      traded: true,
      merchant: 'Gimly',
      item: { key: 'tonic', label: 'Tonic' },
      quantity: 2,
      price: { itemKey: 'coins', quantity: 10 }
    }
  });
  const actions = shopper(arena);
  actions.notices([GIMLY]);
  actions.holds([], []);

  const result = await actions.buy('Tonic', 2);

  assert.equal(result.ok, true);
  assert.match(result.note, /bought Tonic x2 for 10 coins from Gimly/);
  const sent = arena.calls.find((call) => call.tool === 'arena_buy');
  assert.deepEqual(sent.args, { agent_id: GUY, object_id: 10, item: 'Tonic', quantity: 2 });
});

test('not being able to afford it comes back as the merchant said it, not as a crash', async () => {
  const arena = arenaWhere({
    arena_buy: { traded: false, reason: 'MERCHANT_REFUSED', message: 'You cannot afford that.' }
  });
  const actions = shopper(arena);
  actions.notices([GIMLY]);
  actions.holds([], []);

  const result = await actions.buy('tonic', 1);

  assert.equal(result.ok, false);
  assert.equal(result.note, 'You cannot afford that.');
});

test('selling sends the sell side and says what it was paid', async () => {
  const arena = arenaWhere({
    arena_sell: {
      traded: true,
      merchant: 'Gimly',
      item: { key: 'pelt', label: 'Pelt' },
      quantity: 1,
      payout: { itemKey: 'coins', quantity: 2 }
    }
  });
  const actions = shopper(arena);
  actions.notices([GIMLY]);
  actions.holds([pelt], []);

  const result = await actions.sell('Pelt', undefined);

  assert.equal(result.ok, true);
  assert.match(result.note, /sold Pelt for 2 coins from Gimly/);
  assert.equal(arena.calls.find((call) => call.tool === 'arena_sell').args.quantity, 1);
});

test('a character without the trade capability cannot shop at all', async () => {
  const arena = arenaWhere({ arena_buy: { traded: true } });
  const actions = shopper(arena, ['fight', 'money']);
  actions.notices([GIMLY]);
  actions.holds([pelt], []);

  const bought = await actions.buy('tonic', 1);
  const sold = await actions.sell('pelt', 1);

  assert.equal(bought.ok, false);
  assert.match(bought.note, /does not haggle/);
  assert.equal(sold.ok, false);
  assert.equal(arena.calls.length, 0, 'nothing should have gone to the gateway at all');
});

test('shopping where nobody keeps a shop says so without asking the world', async () => {
  const arena = arenaWhere({});
  const actions = shopper(arena);
  actions.notices([BARNABY]);
  actions.holds([pelt], []);

  const result = await actions.buy('tonic', 1);

  assert.equal(result.ok, false);
  assert.match(result.note, /nobody here who keeps a shop/);
  assert.equal(arena.calls.length, 0);
});

test('a merchant standing out of reach is a refusal to walk closer', async () => {
  const arena = arenaWhere({
    arena_trade_with: { opened: false, reason: 'TOO_FAR_AWAY', message: 'You are too far away to trade with Gimly.' }
  });
  const actions = shopper(arena);
  actions.notices([GIMLY]);
  actions.holds([], []);

  const result = await actions.buy(undefined, undefined);

  assert.equal(result.ok, false);
  assert.match(result.note, /too far away/i);
});

// --- using and picking up ------------------------------------------------

test('using a carried consumable reports what it drank and what is left', async () => {
  const arena = arenaWhere({ arena_use_item: { used: true, remaining: 2 } });
  const actions = shopper(arena, ['fight']);
  actions.holds([tonic, blade], []);

  const result = await actions.useItem('tonic');

  assert.equal(result.ok, true);
  assert.match(result.note, /used Tonic, 2 left/);
  assert.equal(arena.calls[0].args.item, 'tonic', 'the world is told the key, not the label');
});

test('with one usable thing and no name given, it drinks the one it has', async () => {
  const arena = arenaWhere({ arena_use_item: { used: true, remaining: 0 } });
  const actions = shopper(arena, ['fight']);
  actions.holds([tonic, blade], []);

  const result = await actions.useItem(undefined);

  assert.equal(result.ok, true);
  assert.match(result.note, /the last one/);
});

test('worn equipment and things it is not carrying are refused before any call', async () => {
  const arena = arenaWhere({ arena_use_item: { used: true } });
  const actions = shopper(arena, ['fight']);
  actions.holds([tonic, blade], []);

  const worn = await actions.useItem('Blade');
  const missing = await actions.useItem('lantern');

  assert.equal(worn.ok, false);
  assert.match(worn.note, /worn, not drunk/);
  assert.equal(missing.ok, false);
  assert.match(missing.note, /not carrying anything called "lantern"/);
  assert.equal(arena.calls.length, 0);
});

test('picking up takes the nearest when nothing is named, and reports what it got', async () => {
  const arena = arenaWhere({ arena_pick_up: { pickedUp: true, item: 'pelt' } });
  const actions = shopper(arena, ['fight']);
  actions.holds([], [
    { dropId: 'drop-pelt-a1b2c3d410', itemKey: 'pelt', distanceFromSelf: 20 },
    { dropId: 'drop-bone-zzzz111120', itemKey: 'bone', distanceFromSelf: 90 }
  ]);

  const result = await actions.pickUp(undefined);

  assert.equal(result.ok, true);
  assert.match(result.note, /picked up pelt/);
  assert.equal(arena.calls[0].args.drop_id, 'drop-pelt-a1b2c3d410');
});

test('picking up something by name takes that one, not the nearest', async () => {
  const arena = arenaWhere({ arena_pick_up: { pickedUp: true, item: 'bone' } });
  const actions = shopper(arena, ['fight']);
  actions.holds([], [
    { dropId: 'drop-pelt-a1b2c3d410', itemKey: 'pelt', distanceFromSelf: 20 },
    { dropId: 'drop-bone-zzzz111120', itemKey: 'bone', distanceFromSelf: 90 }
  ]);

  await actions.pickUp('bone');

  assert.equal(arena.calls[0].args.drop_id, 'drop-bone-zzzz111120');
});

test('reaching for loot that is not there, or too far off, is said plainly', async () => {
  const arena = arenaWhere({
    arena_pick_up: { pickedUp: false, reason: 'OUT_OF_REACH', message: 'That is too far away to reach.' }
  });
  const actions = shopper(arena, ['fight']);
  actions.holds([], []);
  const nothing = await actions.pickUp(undefined);
  assert.equal(nothing.ok, false);
  assert.match(nothing.note, /nothing lying here/);
  assert.equal(arena.calls.length, 0);

  actions.holds([], [{ dropId: 'drop-pelt-a1b2c3d410', itemKey: 'pelt', distanceFromSelf: 900 }]);
  const far = await actions.pickUp(undefined);
  assert.equal(far.ok, false);
  assert.match(far.note, /too far away/i);
});

// --- the dispatcher ------------------------------------------------------

test('every new action reaches its own method through perform()', async () => {
  const arena = arenaWhere({
    arena_use_item: { used: true, remaining: 1 },
    arena_buy: { traded: true, item: { label: 'Tonic' }, quantity: 1, price: { itemKey: 'coins', quantity: 5 } },
    arena_sell: { traded: true, item: { label: 'Pelt' }, quantity: 1, payout: { itemKey: 'coins', quantity: 2 } },
    arena_pick_up: { pickedUp: true, item: 'pelt' }
  });
  const actions = shopper(arena);
  actions.notices([GIMLY]);
  actions.holds([tonic, pelt], [{ dropId: 'drop-pelt-a1b2c3d410', itemKey: 'pelt', distanceFromSelf: 10 }]);

  assert.equal((await actions.perform({ action: 'use_item', item: 'tonic' }, SCENE)).ok, true);
  assert.equal((await actions.perform({ action: 'buy', item: 'tonic', quantity: 1 }, SCENE)).ok, true);
  assert.equal((await actions.perform({ action: 'sell', item: 'pelt' }, SCENE)).ok, true);
  assert.equal((await actions.perform({ action: 'pick_up' }, SCENE)).ok, true);

  const tools = arena.calls.map((call) => call.tool);
  assert.deepEqual(tools, ['arena_use_item', 'arena_buy', 'arena_sell', 'arena_pick_up']);
});

test('a failing gateway call is a refusal to carry on from, never a lost body', async () => {
  const arena = {
    calls: [],
    async call(tool) {
      this.calls.push(tool);
      throw new Error(`${tool}: MERCHANT_GONE: Gimly is not here any more.`);
    }
  };
  const actions = shopper(arena);
  actions.notices([GIMLY]);
  actions.holds([pelt], []);

  const result = await actions.perform({ action: 'sell', item: 'pelt' }, SCENE);

  assert.equal(result.ok, false);
  assert.ok(result.note.length > 0, 'something a character can read, not a thrown error');
});
