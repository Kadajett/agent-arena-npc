/**
 * Everything an NPC is able to do, and nothing else.
 *
 * The gateway exposes a lot of tools. A character in a town needs a handful of
 * them, expressed the way a person would think about them: go somewhere, go
 * through that door, say something, stand still. Each action here wraps the
 * gateway call and constrains its inputs, so a character asks for "the east
 * gate" rather than a pixel coordinate and a badly chosen action sends someone
 * to the wrong end of town instead of into a wall.
 *
 * A character declares which of these it may use. Barnaby cannot walk out of
 * his own inn because he was never given the door.
 */

import { z } from 'zod';
import { ArenaClient, ArenaObject, CarriedItem, Observation, SeenDrop, sceneOf } from './arena.js';
import { Explorer, RoomView, SeenDoor, TILE } from './explore.js';
import { placesIn, plainSceneName, rawSceneName, roomOf } from './world.js';
import { Feeling, FEELINGS, emojiFor } from './feeling.js';

export const CAPABILITIES = [
  'speak',
  'talk_to_folk',
  'walk',
  'doors',
  'fight',
  /**
   * May agree to fight another character, which is not the same permission as
   * fighting wildlife. A monster-culler with no interest in duelling people
   * and a duellist with no business swinging at boars are both writable.
   * Requires 'fight' in code: you must be able to swing at all before you can
   * challenge somebody to.
   */
  'duel',
  'money',
  /**
   * May stand at a merchant's counter and haggle. Separate from 'money',
   * which is only the ability to count what it has, because the two are
   * genuinely different characters: a monster-culler carries coins and never
   * shops, and a trader shops without ever swinging at anything. Nothing that
   * fights automatically learns to bargain.
   */
  'trade',
  /**
   * May perform music the whole scene hears. Separate from 'speak' because
   * a performance is a public act with an audience, and most characters who
   * can talk have no business holding an instrument.
   */
  'perform',
  'purpose'
] as const;
export type Capability = (typeof CAPABILITIES)[number];

/**
 * How the step a character is working on stands after this action. It is the
 * character that says, because only it knows what it was trying to do; the
 * harness holds the list and does the writing down.
 */
export type Progress = 'same' | 'done' | 'blocked';

/**
 * How long a character has to wait before it can give up on a room again.
 *
 * Long on purpose. This is the one action that does not have to obey the map,
 * so the cost of reaching for it has to be higher than the cost of walking.
 */
const GIVING_UP_AGAIN_MS = 60 * 60 * 1000;

const ACTIONS = [
  'say',
  'walk',
  'explore',
  'use_door',
  'talk_to',
  'answer_npc',
  'attack',
  'use_skill',
  'use_item',
  'pick_up',
  'buy',
  'sell',
  'check_money',
  'set_goal',
  'duel_queue',
  'give_up_and_walk_back',
  'wait'
] as const;

/**
 * What a character decided to do this tick.
 *
 * One action, plus anything it wants written down while doing it. The
 * bookkeeping fields are separate from the action on purpose: a character
 * should be able to walk out of a room and remember why on the same tick,
 * rather than spending a turn standing still to make a note.
 */
export type Intent = {
  action: (typeof ACTIONS)[number];
  place?: string;
  /** With talk_to, attack or use_skill: who or what it means, by name. */
  target?: string;
  /**
   * With use_skill: which of its own skills, by name. A character only has the
   * ones its class path grants it, so this is checked against what it actually
   * knows rather than taken on trust. See Actions.useSkill().
   */
  skill?: string;
  /**
   * With use_item, buy, sell or pick_up: which thing, by the name it is
   * carried or sold under. Kept apart from `target`, which is a person: a
   * character selling a pelt to Gimly names both, and collapsing them into
   * one field means it can only ever say one.
   */
  item?: string;
  /** With buy or sell: how many. One when it does not say. */
  quantity?: number;
  message?: string;
  /**
   * With answer_npc: which of the choices it was just given, by the words of
   * the choice or its number. Only meaningful right after a talk_to that
   * offered any.
   */
  option?: string;
  progress?: Progress;
  /**
   * Something the character just learned about a place, in its own words. This
   * is how hearsay gets into memory: somebody mentions a room upstairs, the
   * character notes that it heard it and from whom, and can go and check later.
   */
  noted?: string;
  /**
   * A place it was told about and has now looked for and not found. The other
   * half of hearsay: a rumour that can only ever be confirmed is an
   * announcement.
   */
  notThere?: string;
  /**
   * Set by the harness, never by the model: this intent was recovered from a
   * reply that arrived as prose with no action in it. It is the only way to
   * tell a character that chose to speak from one that has stopped answering
   * in the format at all, since both arrive as a say.
   */
  salvagedFromProse?: boolean;
  /** Something to hold in mind for the next hour, then forget. */
  remember?: string;
  /** Something it is taking on, added to its own list. */
  todo?: string;
  /** With todo: who asked for it, if this is a favour rather than a chore it set itself. */
  askedBy?: string;
  /** An item on that list it has just finished, by number or by roughly what it was. */
  finished?: string;
  /** An item it is giving up on, same way of referring to it. */
  gaveUpOn?: string;
  /**
   * Somebody or somewhere it wants to think back on. What it knows comes back
   * in the next brief rather than immediately, which costs a tick and is about
   * how remembering works anyway. Its purpose is reaching past the summary the
   * brief can afford to carry: see recallAbout() in memory.ts.
   */
  recall?: string;
  /** An item on its list it has got somewhere with, by number or by what it was. */
  progressOn?: string;
  /** What it found out about that item, kept against it. */
  learned?: string;
  /** With set_goal: what it has decided it is after now. */
  aim?: string;
  /** With set_goal: how it would know it had got there. */
  done?: string;
  /** With set_goal: why it settled on that, in its own words. */
  why?: string;
  /**
   * How it feels right now, from the closed set in feeling.ts. Optional and
   * additive, exactly like the bookkeeping fields above: it rides along with
   * whatever action this is, and never replaces one. See emojiFor() and
   * Actions.showFeeling() for what becomes of it.
   */
  feeling?: Feeling;
};

export const IntentSchema = z.object({
  // Lenient for the same reason `progress` below is lenient, and it costs more
  // here. A model that means "look around" writes {"action": "look"}, which is
  // not on the list, so the whole reply was thrown away - the action, and the
  // note it was going to make, and the thing it had just decided to take on.
  // The character then stood still for a tick having apparently decided
  // nothing. Reading the near-misses is cheaper than losing the turn.
  action: z.preprocess((value) => {
    if ('string' !== typeof value) {
      return value;
    }
    const said = value.trim().toLowerCase().replace(/[\s-]+/g, '_');
    if ((ACTIONS as readonly string[]).includes(said)) {
      return said;
    }
    const meant: Record<string, (typeof ACTIONS)[number]> = {
      look: 'explore',
      look_around: 'explore',
      observe: 'explore',
      examine: 'explore',
      search: 'explore',
      wander: 'explore',
      move: 'walk',
      go: 'walk',
      go_to: 'walk',
      travel: 'walk',
      enter: 'use_door',
      exit: 'use_door',
      leave: 'use_door',
      door: 'use_door',
      // A sign is an NPC that cannot walk, so reading one is talking to it.
      read: 'talk_to',
      talk: 'talk_to',
      speak: 'say',
      speak_to: 'talk_to',
      ask: 'talk_to',
      answer: 'answer_npc',
      reply: 'say',
      fight: 'attack',
      hit: 'attack',
      duel: 'duel_queue',
      challenge: 'duel_queue',
      queue: 'duel_queue',
      cast: 'use_skill',
      // "use" stays a skill because that is what it has always meant to these
      // characters and to every prompt they have been written against. The
      // things a person does to a potion get their own readings below.
      use: 'use_skill',
      skill: 'use_skill',
      drink: 'use_item',
      eat: 'use_item',
      consume: 'use_item',
      apply: 'use_item',
      take: 'pick_up',
      grab: 'pick_up',
      pick: 'pick_up',
      collect: 'pick_up',
      loot: 'pick_up',
      purchase: 'buy',
      shop: 'buy',
      trade: 'buy',
      barter: 'buy',
      vend: 'sell',
      hawk: 'sell',
      stuck: 'give_up_and_walk_back',
      unstick: 'give_up_and_walk_back',
      give_up: 'give_up_and_walk_back',
      idle: 'wait',
      stay: 'wait',
      nothing: 'wait',
      rest: 'wait'
    };
    return meant[said] ?? value;
  }, z.enum(ACTIONS)),
  place: z.string().optional(),
  target: z.string().optional(),
  skill: z.string().optional(),
  item: z.string().optional(),
  // Coerced rather than strict: a model asked how many writes "2" about as
  // often as 2, and losing the whole intent over the quotes would cost a tick.
  // Anything that is not a number at all falls back to one in the actions.
  quantity: z.coerce.number().int().min(1).max(99).optional().catch(undefined),
  message: z.string().optional(),
  option: z.string().optional(),
  // Lenient on purpose. A model asked where a step stands will answer "doing",
  // "in progress", "ongoing" - all of which mean "same" - and a strict enum
  // threw the entire intent away over one word, costing the character a whole
  // tick of standing still for a reply that was otherwise perfectly good.
  progress: z
    .preprocess((value) => {
      if ('string' !== typeof value) {
        return value;
      }
      const said = value.trim().toLowerCase();
      if (['done', 'finished', 'complete', 'completed'].includes(said)) {
        return 'done';
      }
      if (['blocked', 'stuck', 'impossible', 'failed'].includes(said)) {
        return 'blocked';
      }
      return 'same';
    }, z.enum(['same', 'done', 'blocked']))
    .optional(),
  noted: z.string().optional(),
  notThere: z.string().optional(),
  remember: z.string().optional(),
  todo: z.string().optional(),
  askedBy: z.string().optional(),
  finished: z.string().optional(),
  gaveUpOn: z.string().optional(),
  /** Somebody or somewhere to bring to mind; answered into the next brief. */
  recall: z.string().optional(),
  /** What it has found out about something already on its list. */
  progressOn: z.string().optional(),
  learned: z.string().optional(),
  aim: z.string().optional(),
  done: z.string().optional(),
  why: z.string().optional(),
  // Closed, unlike progress: a feeling is not worth guessing a meaning for.
  // Anything outside the known set is dropped rather than failing the whole
  // intent, same reasoning as bookkeepingOf() in behavior.ts - losing the
  // decoration is a shrug, losing the action over it would not be.
  feeling: z.preprocess((value) => {
    if ('string' !== typeof value) {
      return undefined;
    }
    const said = value.trim().toLowerCase();
    return (FEELINGS as readonly string[]).includes(said) ? said : undefined;
  }, z.enum(FEELINGS).optional())
});

const ARRIVAL_PIXELS = 40;
const WALK_POLL_MS = 1500;
const STILL_POLLS = 3;
const LEG_TIMEOUT_MS = 45_000;
/**
 * Whether a name is really a heading. The same eight the room description
 * uses (see explore.ts's WAYS), plus the spellings a model reaches for when
 * it is not copying them back exactly.
 */
const BEARINGS = new Set([
  'north', 'south', 'east', 'west',
  'north-east', 'north-west', 'south-east', 'south-west',
  'northeast', 'northwest', 'southeast', 'southwest',
  'up', 'down', 'left', 'right', 'back', 'onwards', 'ahead'
]);

function isBearing(name: string): boolean {
  return BEARINGS.has(name.trim().toLowerCase().replace(/^(the|to the|towards?)\s+/, ''));
}

/**
 * What the gateway says came of walking into a door. `entered` is the only
 * field always present; `reason` and `message` come back together when it did
 * not open, and `reason` is what tells a retry-worth-having (DOOR_TOO_FAR,
 * the door is fine and simply far off) from one that is not.
 */
type DoorAttempt = {
  entered: boolean;
  scene?: string;
  reason?: string;
  message?: string;
};

/**
 * Tiles beside a given one, nearest and truest "beside" first. Standing on
 * top of somebody is not standing next to them, and `talk_to` has its own
 * range check on top of that - see walkToSomebody() - so a spot has to be
 * picked, not just the target's own tile.
 */
const ADJACENT_TILES: Array<[number, number]> = [
  [0, 1],
  [0, -1],
  [1, 0],
  [-1, 0],
  [1, 1],
  [1, -1],
  [-1, 1],
  [-1, -1]
];
/** Let a body finish sliding before anything reads its position. */
const SETTLE_MS = 600;
/** Reldens clips chat at 100 characters (config chat/messages/characterLimit). */
export const CHAT_LINE_LIMIT = 100;
/**
 * How much anyone may say at a stretch. A character's own wordiness sets what
 * it usually says; this is the ceiling none of them may pass, because a wall of
 * text arrives as a stack of chat bubbles nobody reads.
 */
export const MAX_WORDS = 120;
export const DEFAULT_WORDS = 35;
/** Long enough to read as one person talking, short enough not to be a speech. */
const MAX_LINES = 6;
const BETWEEN_LINES_MS = 1400;

export type ActionResult = { ok: boolean; note: string };

/**
 * Whether the character has lost its body, as opposed to merely failed at
 * something. Only this is worth throwing away a session and a plan over.
 */
export function isDisconnected(error: unknown): boolean {
  const said = String((error as Error)?.message ?? error);
  return said.includes('AGENT_NOT_CONNECTED')
    || said.includes('NOT_CONNECTED')
    || said.includes('MCP session');
}

/** What went wrong, said the way a person would notice it rather than as a code. */
export function whatWentWrong(error: unknown): string {
  const said = String((error as Error)?.message ?? error);
  const lower = said.toLowerCase();
  // A request that never got an answer, whichever plumbing failed to deliver
  // one: the gateway's own RELDENS_TIMEOUT, the transport aborting the call
  // once REQUEST_TIMEOUT_MS ran out (arena.ts's AbortSignal.timeout(), which
  // surfaces as "The operation was aborted due to timeout" - Guy hit this
  // three times in six minutes in the volcano with the raw text showing
  // through), or the stream closing with nothing in it. All three read the
  // same to a character standing there waiting, so they get the same words
  // back instead of whichever one happened to fail today.
  if (
    said.includes('RELDENS_TIMEOUT')
    || lower.includes('timed out')
    || lower.includes('aborted due to timeout')
    || lower.includes('mcp endpoint')
  ) {
    return 'waited, and nothing came of it';
  }
  if (said.includes('NO_DOORS_HERE')) {
    return 'there is no way out of here that it can see';
  }
  if (said.includes('too far') || said.includes('INTERACTION')) {
    return 'is not close enough for that';
  }
  // Everything else, trimmed of the transport noise in front of it.
  const plain = said.replace(/^[a-z_]+:\s*/i, '').replace(/^[A-Z_]+:\s*/, '');
  return plain.slice(0, 120) || 'did not work';
}

export class Actions {
  /** What the character can see of the room it is in. Set each tick. */
  private view: RoomView | null = null;
  /** NPCs, traders, and enemies visible right now. Set each tick. */
  private nearby: ArenaObject[] = [];
  /** Everyone standing in the room, with the ids a player target needs. */
  private people: NonNullable<Observation['players']> = [];
  /** What is in the satchel, as of this tick. */
  private carried: CarriedItem[] = [];
  /** What is lying on the floor here, as of this tick. */
  private loot: SeenDrop[] = [];
  /**
   * The duel this character is actually in, once the coordinator has paired
   * it. Same lifecycle as this.talking: per-run, set by the harness, never by
   * the model. Its presence is the gate that lets attack and use_skill aim at
   * a person, and only at this person: without it a character with the duel
   * capability could hit any bystander it can see by asking to.
   */
  private matched: { matchId: string; opponentName: string; scene: string } | null = null;
  /**
   * When this character last gave up and walked back, so it cannot do it
   * again straight away. Starts at zero rather than "now", because a character
   * that comes up already walled in should not have to serve an hour it never
   * earned.
   */
  private gaveUpAt = 0;
  /** Rooms this character has actually stood in, and how long ago. */
  private visited = new Map<string, string>();
  /**
   * How many times in a row this character has chosen to explore each scene.
   * The count that matters is per scene and it survives leaving, because the
   * thing it guards against - wandering a room that has nothing left to show
   * - is true of the room, not of the visit.
   */
  private roamed = new Map<string, number>();
  /** The dialog box currently open, if any, and what it last offered. */
  private talking: { objectId: number; label: string; options: Record<string, string> | null } | null = null;
  /**
   * What an NPC just told this character, waiting to be written into memory
   * as a first-hand finding. Set by talkTo()/answerNpc(), read and cleared by
   * takeNpcReply(); see noteToldByNpc() in npc.ts.
   */
  private lastReply: { from: string; said: string } | null = null;
  /**
   * The feeling last shown to the gateway, so it is only sent again once it
   * actually changes - see showFeeling(). Carrying the same emoji on every
   * tick would be one more call for nothing every few seconds, forever.
   */
  private lastShownFeeling: Feeling | null = null;

  constructor(
    private readonly arena: ArenaClient,
    private readonly agentId: string,
    private readonly capabilities: Set<Capability>,
    /** How much this character says at a stretch, in words. */
    private readonly wordiness: number = DEFAULT_WORDS,
    private readonly explorer: Explorer = new Explorer(),
    /**
     * The named skills this character's class path grants it. Empty for
     * somebody who only ever swings. See useSkill() for why this is declared
     * rather than discovered: the skills are rows in the world, they differ by
     * class path, and a character asking for one it does not have should hear
     * so rather than watch nothing happen.
     */
    private readonly skills: readonly string[] = []
  ) {}

  can(capability: Capability): boolean {
    return this.capabilities.has(capability);
  }

  sees(view: RoomView | null): void {
    this.view = view;
  }

  /**
   * What is standing nearby right now: NPCs, traders, enemies. Set once a
   * tick from the same observation the harness already fetched, so naming
   * somebody to talk to or hit does not cost a second round trip.
   */
  /** Who is standing here this tick, ids included. See the players type. */
  meets(players: Observation['players']): void {
    this.people = players ?? [];
  }

  notices(objects: ArenaObject[] | undefined): void {
    this.nearby = objects ?? [];
  }

  /**
   * What this character is carrying and what is on the floor around it, from
   * the same observation everything else this tick came from. Handed in
   * rather than fetched so that knowing what is in your own pockets costs
   * nothing: a character that has to make a call to find out will not.
   */
  holds(carrying: CarriedItem[] | undefined, drops: SeenDrop[] | undefined): void {
    this.carried = carrying ?? [];
    this.loot = drops ?? [];
  }

  /** What it is carrying, said in one line. Empty when it has nothing. */
  carryingLine(): string {
    if (0 === this.carried.length) {
      return '';
    }
    const said = this.carried.map((item) => {
      const many = 1 < item.quantity ? ` x${item.quantity}` : '';
      return `${item.label}${many}${item.equipped ? ' (worn)' : ''}`;
    });
    return `You are carrying: ${said.join(', ')}.`;
  }

  /** The merchant standing here, if one is. */
  private merchantHere(): ArenaObject | null {
    return this.nearby.find((object) => object.isMerchant && object.objectId != null) ?? null;
  }

  /**
   * What could be offered to a merchant: everything carried that is not
   * currently being worn. Whether the merchant actually wants any of it is
   * the merchant's to say - the harness has no price list and should never
   * pretend to one.
   */
  private sellable(): CarriedItem[] {
    return this.carried.filter((item) => !item.equipped);
  }

  /** Match a name against something carried, the way a person refers to it. */
  private carriedNamed(name: string): CarriedItem | null {
    const wanted = name.trim().toLowerCase();
    if (!wanted) {
      return null;
    }
    return (
      this.carried.find((item) => item.key.toLowerCase() === wanted)
      ?? this.carried.find((item) => item.label.toLowerCase() === wanted)
      ?? this.carried.find((item) => item.label.toLowerCase().includes(wanted))
      ?? this.carried.find((item) => wanted.includes(item.label.toLowerCase()))
      ?? null
    );
  }

  /** The doorways out of here, as the character can see them. */
  doors(): SeenDoor[] {
    return this.view?.doors ?? [];
  }

  /** Match a name against what is nearby, the way a person would refer to it. */
  private findNearby(name: string, kind: ArenaObject['kind']): ArenaObject | null {
    const wanted = name.trim().toLowerCase();
    if (!wanted) {
      return null;
    }
    const candidates = this.nearby.filter((object) => object.kind === kind);
    return (
      candidates.find((object) => object.label.trim().toLowerCase() === wanted)
      ?? candidates.find((object) => object.label.toLowerCase().includes(wanted))
      ?? candidates.find((object) => wanted.includes(object.label.trim().toLowerCase()))
      ?? null
    );
  }

  /** The list of actions to offer a character, given where it is standing. */
  describe(scene: string): string {
    const lines: string[] = [];
    if (this.can('speak')) {
      lines.push('- "say": say something out loud. Needs: message');
    }
    if (this.can('walk')) {
      const names = Object.keys(placesIn(scene));
      if (names.length > 0) {
        lines.push(`- "walk": go somewhere in this room. Needs: place, one of ${names
          .map((name) => `"${name}"`)
          .join(', ')}`);
      }
      // Anywhere that is not home is found out by walking around it. This is
      // the only way to see a room nobody has written down.
      lines.push('- "explore": wander to a part of this room you have not seen');
    }
    if (this.can('doors')) {
      const doors = this.doors();
      if (doors.length === 1) {
        lines.push(`- "use_door": go through to ${this.doorWithHistory(doors[0])}`);
      } else if (doors.length > 1) {
        lines.push(
          '- "use_door": go through a door. Needs: place, one of '
            + doors.map((door) => this.doorWithHistory(door)).join(', ')
        );
        // Only the doors are places you can walk through. Said here because a
        // character that wants somewhere in the next room keeps naming that
        // place as a door: Guy asked for "the east gate" six times running from
        // inside a house, and the east gate is a spot in town, two steps past a
        // door he could see the whole time. He was told each time that no such
        // door existed, which is true and no help at all.
        lines.push('  (a door goes to a whole room. To reach a spot inside one, go through, then walk.)');
      }
    }
    if (this.can('talk_to_folk')) {
      if (this.talking) {
        const offered = this.talking.options ? Object.values(this.talking.options) : [];
        lines.push(
          offered.length > 0
            ? `- "answer_npc": answer ${this.talking.label}. Needs: option, one of `
              + offered.map((choice) => `"${choice}"`).join(', ')
            : `- "talk_to": speak to ${this.talking.label} again, or somebody else. Needs: target`
        );
      } else {
        const names = this.nearby.filter((object) => object.kind === 'npc').map((object) => object.label);
        if (names.length > 0) {
          lines.push(
            `- "talk_to": start a conversation. Needs: target, one of ${names
              .map((name) => `"${name}"`)
              .join(', ')}`
          );
        }
      }
    }
    if (this.can('fight')) {
      const names = this.nearby.filter((object) => object.kind === 'enemy').map((object) => object.label);
      if (names.length > 0) {
        lines.push(
          `- "attack": attack something here. Needs: target, one of ${names
            .map((name) => `"${name}"`)
            .join(', ')}`
        );
        // Only offered with something to aim at, same as attack. A character
        // told it can cast fireball in an empty room will try, and be told no
        // by the only part of this that can see there is nobody there.
        if (this.skills.length > 0) {
          lines.push(
            `- "use_skill": use one of your own skills on something. Needs: skill, one of ${this.skills
              .map((skill) => `"${skill}"`)
              .join(', ')}; and target`
          );
        }
      }
    }
    if (this.can('duel') && this.can('fight')) {
      lines.push(
        this.matched
          ? `- you are in a duel with ${this.matched.opponentName}. Attack them by name when they are here.`
          : '- "duel_queue": stand for a duel here, against whoever answers. Needs nothing;'
            + ' name a target if you have somebody in mind.'
      );
    }
    // Anybody can drink what they are carrying; it is not a trade and it is
    // not a fight. Only offered when there is actually something to drink,
    // the same rule attack follows: an action with nothing to point it at is
    // an invitation to waste a turn.
    const usable = this.carried.filter((item) => item.usable);
    if (0 < usable.length) {
      lines.push(
        `- "use_item": use something you are carrying. Needs: item, one of ${usable
          .map((item) => `"${item.label}"`)
          .join(', ')}`
      );
    }
    if (0 < this.loot.length) {
      const named = this.loot.filter((drop) => drop.itemKey).map((drop) => drop.itemKey as string);
      lines.push(
        0 < named.length
          ? `- "pick_up": pick up what has been dropped here: ${named.join(', ')}. Needs nothing for the nearest.`
          : '- "pick_up": pick up what has been dropped here. Needs nothing for the nearest.'
      );
    }
    if (this.can('trade')) {
      const merchant = this.merchantHere();
      if (merchant) {
        lines.push(
          `- "buy": buy from ${merchant.label}. Needs: item, and quantity if more than one.`
            + ' Leave out the item to ask what is for sale.'
        );
        const offerable = this.sellable();
        lines.push(
          0 < offerable.length
            ? `- "sell": sell to ${merchant.label}. Needs: item, one of ${offerable
                .map((item) => `"${item.label}"`)
                .join(', ')}`
            : `- "sell": you have nothing loose to sell ${merchant.label}.`
        );
      }
    }
    if (this.can('money')) {
      lines.push('- "check_money": count your arena credits, which are not the coins in your purse.');
    }
    if (this.can('purpose')) {
      lines.push(
        '- "set_goal": decide what you are after from now on. Needs: aim, done, why.'
          + ' Only when what you wanted is finished or plainly hopeless.'
      );
    }
    // Deliberately last, after everything that involves actually walking, and
    // worded so it reads as the admission it is. A character offered this next
    // to "walk" will use it as a shortcut home.
    if (this.can('doors')) {
      lines.push(
        '- "give_up_and_walk_back": only if you have genuinely tried and cannot get out of'
          + ' where you are. You end up back at the inn and cannot do it again for an hour.'
      );
    }
    lines.push('- "wait": stay where you are');
    return lines.join('\n');
  }


  /**
   * A door, and whether this character has already been through it.
   *
   * Without this a door is just a name, and every unexplored room and every
   * room somebody has walked in and out of nine times read exactly alike. A
   * character deciding where to go next had nothing to go on and so kept
   * picking the nearest one, which is how a loop between two rooms starts and
   * why it never stops.
   *
   * The knowledge is the harness's own record of where this character has
   * actually stood, not anything the model wrote down, so it cannot talk itself
   * into having explored somewhere it has not.
   */
  private doorWithHistory(door: SeenDoor): string {
    const name = this.doorLabel(door);
    const been = door.leadsTo ? this.visited.get(door.leadsTo) : undefined;
    if (!door.leadsTo) {
      return `"${name}"`;
    }
    if (!been) {
      return `"${name}" (never been)`;
    }
    return `"${name}" (been there, ${been})`;
  }

  /**
   * What the character remembers about rooms it has stood in, keyed by scene.
   * Handed in each tick rather than kept here, because the harness owns it and
   * a stale copy would tell somebody they had been somewhere they had not.
   */
  remembersRooms(visited: Map<string, string>): void {
    this.visited = visited;
  }

  private doorLabel(door: SeenDoor): string {
    return door.leadsTo ? plainSceneName(door.leadsTo) : 'somewhere else';
  }

  /**
   * Do the thing, and treat a refusal as a refusal rather than a catastrophe.
   *
   * Every gateway call throws when it fails, and nothing used to catch them, so
   * an NPC that did not answer in time came all the way up through the tick
   * loop to the reconnect handler. The character then tore down its session,
   * logged back in, and came back with no plan - having lost, over one
   * unanswered greeting, everything it had worked out about what it was doing.
   *
   * Almost nothing that goes wrong in a single action is fatal. A door that
   * will not open, a monster that died before the swing landed, an NPC too far
   * away to hear: all of those are things that happen to people, and the
   * honest response is to say so and carry on. Only losing the body itself is
   * worth reconnecting for, so only that is allowed past.
   */
  async perform(intent: Intent, scene: string): Promise<ActionResult> {
    // Carried to the gateway alongside the action, never in place of it, and
    // never allowed to affect the result below - see showFeeling().
    if (intent.feeling) {
      await this.showFeeling(intent.feeling);
    }
    try {
      return await this.attempt(intent, scene);
    } catch (error) {
      if (isDisconnected(error)) {
        throw error;
      }
      return { ok: false, note: whatWentWrong(error) };
    }
  }

  /**
   * Tell the gateway how this character is doing, so the spectator viewer can
   * show it over its head. This is decoration, not an action: it costs no
   * turn, and nothing it does can fail the tick it rides along on. Only sent
   * when it actually changed, so a character sitting in one mood for a while
   * is not re-announcing it every few seconds.
   */
  private async showFeeling(feeling: Feeling): Promise<void> {
    if (feeling === this.lastShownFeeling || !emojiFor(feeling)) {
      return;
    }
    try {
      await this.arena.call('arena_feel', { agent_id: this.agentId, feeling });
      this.lastShownFeeling = feeling;
    } catch {
      // Never worth losing the turn over. The next tick tries again if the
      // feeling still holds, same as any other best-effort side channel.
    }
  }

  private async attempt(intent: Intent, scene: string): Promise<ActionResult> {
    switch (intent.action) {
      case 'say':
        return this.say(intent.message);
      case 'walk':
        return this.walk(intent.place, scene, intent.message);
      case 'explore':
        return this.explore(scene, intent.message);
      case 'use_door':
        return this.useDoor(scene, intent.place, intent.message);
      case 'talk_to':
        return this.talkTo(intent.target);
      case 'answer_npc':
        return this.answerNpc(intent.option);
      case 'attack':
        return this.attack(intent.target);
      case 'use_skill':
        return this.useSkill(intent.skill, intent.target);
      case 'use_item':
        return this.useItem(intent.item ?? intent.target);
      case 'pick_up':
        return this.pickUp(intent.item ?? intent.target);
      case 'buy':
        return this.buy(intent.item, intent.quantity);
      case 'sell':
        return this.sell(intent.item, intent.quantity);
      case 'duel_queue':
        return this.duelQueue(scene, intent.target);
      case 'give_up_and_walk_back':
        return this.giveUpAndWalkBack();
      case 'check_money':
        return this.checkMoney();
      case 'set_goal':
        // The harness owns the goal, because it owns the memory it is written
        // to. It applies this before anything gets here; see npc.ts.
        return { ok: true, note: 'thought about what you are doing with yourself' };
      default:
        return { ok: true, note: 'stayed put' };
    }
  }

  /**
   * Wander somewhere in this room the character has not been. The harness picks
   * the spot off the real collision grid and confirms the route first, so this
   * is exploring rather than walking hopefully into a wall.
   */
  async explore(scene: string, message?: string): Promise<ActionResult> {
    if (!this.can('walk')) {
      return { ok: false, note: 'this character stays where it is' };
    }
    const here = await this.where();
    if (!here) {
      return { ok: false, note: 'could not tell where it was standing' };
    }
    this.explorer.markHere(scene, here.x, here.y);
    // The third explore of the same room is where wandering stops teaching.
    // Guy spent an afternoon proving this: explore picks a fresh bearing every
    // time, so each look "succeeds", the model copies its own success, and the
    // circling detector - which keys on action plus place - never sees two
    // moves alike. If there is a door here this character has never been
    // through, the harness takes it, because a room it has never seen beats
    // any corner of one it has. Advice was tried first and lost to pattern,
    // the same as it did at the pickDoor fallthrough.
    const wandered = (this.roamed.get(scene) ?? 0) + 1;
    this.roamed.set(scene, wandered);
    if (wandered > 3 && this.can('doors')) {
      const somewhereUnseen = this.doors().find(
        (door) => door.leadsTo && !door.locked && !this.visited.get(door.leadsTo)
      );
      if (somewhereUnseen) {
        this.roamed.set(scene, 0);
        const through = await this.useDoor(scene, this.doorLabel(somewhereUnseen), message);
        if (through.ok) {
          return {
            ok: true,
            note:
              `this room had nothing left it had not seen, so instead of another look around it `
              + `${through.note}, somewhere it had never been`
          };
        }
        // The door refused; fall through to an honest wander rather than
        // failing an explore the character never asked to convert.
      }
    }
    const spot = await this.explorer.somewhereNew(this.arena, this.agentId, scene, here);
    if (!spot) {
      return { ok: false, note: 'there was nowhere new to go from here' };
    }
    const talking = this.alsoSay(message);
    await this.arena.call('arena_move_to', { agent_id: this.agentId, x: spot.x, y: spot.y });
    const arrived = await this.waitForArrival(spot.x, spot.y);
    await talking;
    // Mark where it actually ended up, not where it meant to go. A character
    // that stalls against a corner has still moved, and recording the target
    // would tell it it had seen a patch it never reached.
    const landed = await this.where();
    if (landed) {
      this.explorer.markHere(scene, landed.x, landed.y);
    }
    return {
      ok: arrived,
      note: arrived
        ? `had a look around to the ${spot.bearing}`
        : `set off ${spot.bearing} and did not get there`
    };
  }

  private async where(): Promise<{ x: number; y: number } | null> {
    const observation = await this.observe().catch(() => null);
    const state = observation?.ownPlayer?.state;
    if (!state || !Number.isFinite(Number(state.x))) {
      return null;
    }
    return { x: Number(state.x), y: Number(state.y) };
  }

  /**
   * Speak. A long thought goes out as several chat lines in a row, paced like
   * someone actually saying it, because the chat field takes 100 characters and
   * a character who talks in paragraphs would otherwise arrive cut mid-word.
   */
  async say(message: string | undefined): Promise<ActionResult> {
    if (!this.can('speak')) {
      return { ok: false, note: 'this character does not speak' };
    }
    const lines = toSpeech(message ?? '', this.wordiness);
    if (lines.length === 0) {
      return { ok: false, note: 'nothing worth saying' };
    }
    for (const [index, line] of lines.entries()) {
      if (index > 0) {
        await sleep(BETWEEN_LINES_MS);
      }
      await this.arena.call('arena_say', { agent_id: this.agentId, message: line });
    }
    return { ok: true, note: `said: ${lines.join(' ')}` };
  }

  async walk(place: string | undefined, scene: string, message?: string): Promise<ActionResult> {
    if (!this.can('walk')) {
      return { ok: false, note: 'this character stays where it is' };
    }
    const places = placesIn(scene);
    const key = Object.keys(places).find(
      (name) => name.toLowerCase() === String(place ?? '').trim().toLowerCase()
    );
    if (!key) {
      // The place may simply be in the next room, and the way to another room
      // is through the door. A character heading for the bar from the street
      // should walk in rather than report that the bar does not exist.
      const elsewhere = roomOf(String(place ?? ''));
      if (elsewhere && elsewhere !== scene && this.can('doors')) {
        const through = this.doors().find((door) => door.leadsTo === elsewhere);
        if (through) {
          return this.useDoor(scene, place, message);
        }
      }
      // Somebody standing right here is also somewhere to walk to - the only
      // named destination that exists at all outside home turf, where
      // placesIn() is empty by design (see world.ts). Without this, "walk
      // over to the sellsword" had nothing to resolve to anywhere but town
      // and fell straight through to exploring at random, while talk_to on
      // its own kept failing as too far away. See walkToSomebody().
      const toSomebody = await this.walkToSomebody(place, message);
      if (toSomebody) {
        return toSomebody;
      }
      // Somewhere it has heard of but cannot place: look around for it rather
      // than announcing that it does not exist.
      if (this.can('walk') && place) {
        return this.explore(scene, message);
      }
      return { ok: false, note: `there is no "${place}" here` };
    }
    // Talk on the way. Waiting for a three-line remark to finish before taking
    // a step means standing in the street reciting, and it reads as a stall.
    const talking = this.alsoSay(message);
    const target = places[key];
    await this.arena.call('arena_move_to', {
      agent_id: this.agentId,
      x: target.x,
      y: target.y
    });
    const arrived = await this.waitForArrival(target.x, target.y);
    await talking;
    return { ok: arrived, note: arrived ? `walked to ${key}` : `set off for ${key} but stopped short` };
  }

  /** Start saying something without waiting for it to finish. */
  private alsoSay(message: string | undefined): Promise<unknown> {
    return message ? this.say(message).catch(() => undefined) : Promise.resolve();
  }

  /**
   * Reach somebody standing nearby by name, matched the same forgiving way a
   * door is (see matchDoors()) rather than needing the exact string the
   * gateway calls them. Only NPCs: an enemy is what `attack` is for, and
   * walking up to one on purpose is a different thing to mean.
   *
   * Returns null, not a failure, when the name matches nobody here at all and
   * nobody is standing here to report either - the caller then falls through
   * to explore() exactly as it always did for a place only ever heard of.
   * That is still the right guess for an unfamiliar name in an empty room.
   * But once somebody actually is standing here, guessing wrong and wandering
   * off is worse than saying so: a real failure names who is actually here,
   * the same as a door with no matching name lists what doors there are.
   */
  private async walkToSomebody(place: string | undefined, message?: string): Promise<ActionResult | null> {
    const wanted = String(place ?? '').trim();
    if (!wanted || isBearing(wanted)) {
      // "south", "south-east": a heading, not somebody's name. Reporting who
      // is standing here in answer to it is worse than useless - Guy asked to
      // walk south in the volcano and was told twice, in consecutive turns,
      // that there was nobody here called "south", with two people listed
      // back at him. explore() knows what to do with a bearing; this does not.
      return null;
    }
    const people = this.nearby.filter((object) => object.kind === 'npc');
    // Only the one name each: unlike a door, a person has no second, raw form
    // hiding behind a pretty override for a memory to have recorded instead.
    const matches = this.matchByLabel(wanted, people, (person) => [person.label]);
    if (matches.length === 0) {
      if (people.length === 0) {
        return null;
      }
      return {
        ok: false,
        note: `there is nobody here it would call "${place}". It can see: `
          + people.map((person) => `"${person.label}"`).join(', ')
      };
    }
    if (matches.length > 1) {
      return {
        ok: false,
        note: `it could not tell which of them "${place}" meant: `
          + matches.map((person) => `"${person.label}"`).join(' or ')
      };
    }
    const [person] = matches;
    if (!Number.isFinite(person.tileX) || !Number.isFinite(person.tileY)) {
      return { ok: false, note: `cannot tell where ${person.label} actually is` };
    }
    const spot = await this.adjacentTile(person.tileX, person.tileY);
    if (!spot) {
      return { ok: false, note: `could not find a way to stand next to ${person.label}` };
    }
    const talking = this.alsoSay(message);
    const arrived = await this.approach(spot.x, spot.y);
    await talking;
    return {
      ok: arrived,
      note: arrived
        ? `walked over to ${person.label}`
        : `set off for ${person.label} but stopped short`
    };
  }

  /**
   * Walk to a point and wait for the body to actually get there. The one
   * move every one of these actions needs once it has worked out where to
   * go - a person's tile, a spot to explore, a door too far off to reach in
   * one try - so it is written once here rather than three times over.
   */
  private async approach(x: number, y: number): Promise<boolean> {
    await this.arena.call('arena_move_to', { agent_id: this.agentId, x, y });
    return this.waitForArrival(x, y);
  }

  /**
   * A tile beside the given one that the character can actually reach,
   * checked against the real collision grid the same way
   * Explorer.somewhereNew() confirms a spot before setting off - standing
   * next to somebody, not on top of them, is the whole point, and next door
   * is exactly where a wall might be.
   */
  private async adjacentTile(
    tileX: number,
    tileY: number,
    rings = 1
  ): Promise<{ x: number; y: number } | null> {
    for (let ring = 1; ring <= rings; ring++) {
      for (const [dx, dy] of ADJACENT_TILES) {
        const x = (tileX + dx * ring) * TILE + TILE / 2;
        const y = (tileY + dy * ring) * TILE + TILE / 2;
        if (x < TILE || y < TILE) {
          continue;
        }
        try {
          const path = await this.arena.call('arena_check_path', { agent_id: this.agentId, x, y });
          if (path?.reachable) {
            return { x, y };
          }
        } catch {
          // Treat an unanswerable probe as unreachable and try the next side.
        }
      }
    }
    return null;
  }

  /**
   * Go through a doorway the character can see. Which doors exist comes from
   * looking at the room, not from a table somebody wrote out in advance, so
   * this works the same in a room nobody has ever surveyed.
   */
  async useDoor(scene: string, which?: string, message?: string): Promise<ActionResult> {
    if (!this.can('doors')) {
      return { ok: false, note: 'this character does not leave this room' };
    }
    const doors = this.doors();
    if (doors.length === 0) {
      return { ok: false, note: 'there is no way out of here that it can see' };
    }
    const picked = this.pickDoor(which, doors, scene);
    if ('walkTo' in picked) {
      return this.walk(picked.walkTo, scene, message);
    }
    if ('note' in picked) {
      // Never just "could not tell": a character stuck on a door with no
      // idea what its actual choices are will try the same unreadable name
      // again next tick. Naming what is really there is what gets it moving.
      return { ok: false, note: picked.note };
    }
    const door = picked;
    if (door.locked) {
      return { ok: false, note: `the door to ${this.doorLabel(door)} is locked` };
    }
    if (message) {
      await this.say(message).catch(() => undefined);
    }
    // The gateway routes to the door, steps through, and retries: door tiles
    // are excluded from path-finding on purpose, so they can only be walked
    // into. See arena_enter_door.
    const result = await this.enterDoor(door);
    if (result.entered) {
      return { ok: true, note: `went through into ${this.arrivedIn(result, door)}` };
    }
    if (result.reason === 'DOOR_TOO_FAR') {
      return this.crossToDoor(door);
    }
    return { ok: false, note: `the door did not open: ${result.message ?? result.reason}` };
  }

  /**
   * A door the gateway could not reach inside its own budget: cross the room
   * and, if that gets us there, step through.
   *
   * Not a refusal. The gateway is saying the door is fine and simply a long
   * way off, so the useful move is the one it suggests rather than a sentence
   * the character can do nothing with.
   *
   * Two details, both learned by watching this run in the volcano and taking
   * three minutes over it. The walk aims at a tile BESIDE the door, not the
   * door: a change point is walked into, not stood on, so aiming at it either
   * fails or trips the transition halfway through a leg and leaves the retry
   * running in the wrong room. And when the walk still has not arrived, that
   * is the end of the turn. Chaining another door attempt onto the end of a
   * timed-out crossing stacks forty five seconds of walking onto two forty
   * second door budgets, and for those three minutes the character cannot
   * hear anybody, look at anything, or be talked to. Getting most of the way
   * across a room is real progress and is reported as such, so the next tick
   * picks the door up from close enough for the ordinary path to work.
   */
  private async crossToDoor(door: SeenDoor): Promise<ActionResult> {
    const where = this.doorLabel(door);
    // Two rings, not one. The eight tiles touching a doorway are the obvious
    // place to stand and often the worst: half of them are the wall the door
    // is set into, and in a narrow passage the rest can be a change point
    // itself. Widening the search is cheaper than the alternative, which is a
    // character standing across the room from a door it can see and being told
    // there is no way to it.
    const spot = await this.adjacentTile(door.column, door.row, 2);
    if (!spot) {
      // Nothing beside the door answered. That is a fact about the probe, not
      // about the world, and the old wording said the opposite: "there is no
      // way through to the door" is a claim the character will believe and act
      // on, and it was wrong often enough to strand somebody. Say what was
      // actually established, and leave the door worth trying again.
      return {
        ok: false,
        note:
          `could not find anywhere to stand beside the door to ${where} from here, `
          + 'so it may be walled off from this side or simply too far to work out yet'
      };
    }
    if (!(await this.approach(spot.x, spot.y))) {
      return { ok: true, note: `set off for the door to ${where} and got part of the way; it is still ahead` };
    }
    const retried = await this.enterDoor(door);
    return retried.entered
      ? { ok: true, note: `went through into ${this.arrivedIn(retried, door)}` }
      : { ok: false, note: `the door did not open: ${retried.message ?? retried.reason}` };
  }

  /**
   * Where the character ended up, named the way it would name it.
   *
   * The gateway reports the scene it arrived in, and that is the truth worth
   * having, because a door can land somebody somewhere other than the room its
   * label advertised. But `scene` is only documented as present alongside
   * `entered`, not guaranteed by anything, and typing this properly is what
   * turned that up: it used to be read straight into plainSceneName(), so a
   * reply without it would have told the character it "went through into
   * undefined" and written that into its memory as a place it had been. Fall
   * back to what the door said it led to, which is at worst a label the
   * character already had.
   */
  private arrivedIn(result: DoorAttempt, door: SeenDoor): string {
    return result.scene ? plainSceneName(result.scene) : this.doorLabel(door);
  }

  /** Ask the gateway to walk a character to a door already resolved to a tile. */
  private async enterDoor(door: SeenDoor): Promise<DoorAttempt> {
    return this.arena.call('arena_enter_door', { agent_id: this.agentId, x: door.x, y: door.y });
  }

  /**
   * Match what the character asked for against the doorways it can see, or
   * say plainly why nothing was picked. Returns the door itself, or a note
   * to hand straight back as the result - never a bare failure with nothing
   * for the character to go on, because that is what left it stuck on "the
   * inn door" with no way to know that was not close enough.
   */
  private pickDoor(
    which: string | undefined,
    doors: SeenDoor[],
    scene = ''
  ): SeenDoor | { note: string } | { walkTo: string } {
    if (doors.length === 1) {
      // Only one way out, whatever they called it: "the door out", "outside",
      // "back" when there is somewhere it came from. Nothing to disambiguate.
      return doors[0];
    }
    const wanted = String(which ?? '').trim();
    if (!wanted) {
      return { note: `it was not clear which door. It can see: ${this.listDoors(doors)}` };
    }
    const matches = this.matchDoors(wanted, doors);
    if (matches.length === 1) {
      return matches[0];
    }
    if (matches.length > 1) {
      // A doorway two tiles wide is two change points, so it arrives here as
      // two doors with the same label. Asking a character to choose between
      // "town" and "town" is asking it to answer a question with no answer,
      // and it did: the Wanderer spent a turn on exactly that. If every
      // candidate leads to the same room then the name was never ambiguous.
      const first = matches[0];
      if (matches.every((door) => door.leadsTo === first.leadsTo)) {
        return first;
      }
      return {
        note: `it could not tell which door "${which}" meant: `
          + matches.map((door) => `"${this.doorLabel(door)}"`).join(' or ')
      };
    }
    // Before giving up: the thing it asked for may be a real place, just not a
    // door. Guy asked for "the east gate" from inside a house on the far side
    // of town, thirteen times, and each time was told no such door exists.
    // Which is true, and useless: the east gate is a spot in town, through a
    // door he could see the whole time. A character that knows where somewhere
    // is and cannot get there is worse off than one that has never heard of it.
    const elsewhere = roomOf(String(which ?? ""));
    if (elsewhere === scene) {
      // The place is in this very room. Guy stood in town asking for a door to
      // the south field, which is forty tiles away across the grass he was
      // looking at, and was told there was no door to there from here. True,
      // in the way that only useless things are true. Walking is the action he
      // meant, so it becomes a walk.
      return { walkTo: String(which) };
    }
    if (elsewhere) {
      const wayThrough = doors.find((door) => door.leadsTo === elsewhere);
      if (wayThrough) {
        // Take it, rather than explaining it. Guy was handed this exact
        // sentence as advice - "go through the door to town first, then walk
        // to it" - and asked for the same non-existent door five more times,
        // because a model copies the pattern its own history demonstrates and
        // his history was thirteen attempts at that door. Advice loses to the
        // pattern. Doing the obvious thing breaks it.
        //
        // There is no guesswork in the choice: the name is a place, the place
        // is in a room, and exactly one door here goes to that room. Somebody
        // asking for the east gate from a house across town wants to head that
        // way, and this is the first step of the only route there is.
        return wayThrough;
      }
      return {
        note:
          `"${which}" is not a door, it is a spot in ${plainSceneName(elsewhere)}, `
          + `and there is no door to there from here. The doors here go to: ${this.listDoors(doors)}`
      };
    }
    return { note: `there is no door here it would call "${which}". It can see: ${this.listDoors(doors)}` };
  }

  /**
   * Every door a name could plausibly mean. Delegates to matchByLabel(): a
   * door is just a candidate with names, the same as a person is - see
   * walkToSomebody(), which names somebody nearby the identical way, just
   * with only the one name to offer instead of a door's two.
   */
  private matchDoors(which: string, doors: SeenDoor[]): SeenDoor[] {
    return this.matchByLabel(which, doors, (door) => this.doorNames(door));
  }

  /** The doorways here, named the way a character would name them. */
  private listDoors(doors: SeenDoor[]): string {
    return doors.map((door) => `"${this.doorLabel(door)}"`).join(', ');
  }

  /**
   * Every name a door could reasonably be asked for by, not only the pretty
   * one describe() and listDoors() show.
   *
   * plainSceneName() - what doorLabel() is built from - overrides a handful
   * of rooms with a name a person would actually say: reldens-forest reads
   * as "the woods". But a character's own memory of standing in that same
   * room is written with rawSceneName() instead (see notePlace() in npc.ts),
   * which never applies that override and calls it "forest". A production
   * memory fragment showed exactly this: a character had "bots forest" and
   * "bots forest house 01 n0" written down, and a door that only answered to
   * its pretty name could never be reached again by the name the character
   * actually had for it. Deduped, since most doors have no override at all
   * and the two forms are the same string.
   */
  private doorNames(door: SeenDoor): string[] {
    const pretty = this.doorLabel(door);
    const raw = door.leadsTo ? rawSceneName(door.leadsTo) : pretty;
    return raw === pretty ? [pretty] : [pretty, raw];
  }

  /**
   * Every candidate a name could plausibly mean, out of anything with one or
   * more names: an exact match against any of them, a substring either way,
   * or - failing both - any word the name and one of theirs actually share
   * once filler words are out of it, using the same word-overlap
   * contentWords() already does for catching a character repeating itself.
   * "the inn door" reaches "Barnaby's inn" on the shared word "inn"; "the
   * sellsword" reaches "Old Ferro the sellsword" the same way. One matcher
   * for both doors and people, because it is one problem: a name given
   * loosely against a short list of real names - a person only ever has the
   * one, a door can have two.
   */
  private matchByLabel<T>(which: string, candidates: T[], namesOf: (item: T) => string[]): T[] {
    const wanted = which.toLowerCase();
    const exact = candidates.filter((item) =>
      namesOf(item).some((name) => name.toLowerCase() === wanted)
    );
    if (exact.length > 0) {
      return exact;
    }
    const substring = candidates.filter((item) =>
      namesOf(item).some((name) => {
        const label = name.toLowerCase();
        return label.includes(wanted) || wanted.includes(label);
      })
    );
    if (substring.length > 0) {
      return substring;
    }
    const wantedWords = contentWords(which);
    if (wantedWords.size === 0) {
      return [];
    }
    return candidates.filter((item) =>
      namesOf(item).some((name) => {
        const labelWords = contentWords(name);
        return [...wantedWords].some((word) => labelWords.has(word));
      })
    );
  }

  /**
   * Put this character up for a duel where it is standing.
   *
   * The scene is the harness's, never the model's. arena_queue_match defaults
   * a missing scene to the agent's home, so a model that forgot the field
   * would queue Nerys against her own bedroom rather than the plateau she just
   * walked to. Supplying it from the actual current scene is the same shape of
   * decision as walk supplying coordinates from a place name.
   *
   * The named opponent is intent, not enforcement. The coordinator pairs
   * whoever queued on the same scene, first come first served; there is no way
   * to queue against a specific person and no way to leave a queue once in it.
   * So the honest thing is to say who turned up, and refuse to pretend a
   * stranger is the person this character meant: the pairing still stands,
   * because two people who both walked to the same flat and asked for a fight
   * have agreed to one, whoever they hoped would be there.
   */
  async duelQueue(scene: string, opponent: string | undefined): Promise<ActionResult> {
    if (!this.can('duel') || !this.can('fight')) {
      return { ok: false, note: 'this character does not duel' };
    }
    if (this.matched) {
      // The one standing duel may have finished: the world decides that, not
      // this slot, so ask before refusing. This is also the only place the
      // slot is cleared, which keeps its lifecycle in one method.
      const standing = await this.arena
        .call('arena_match_status', { match_id: this.matched.matchId })
        .catch(() => null);
      if ('completed' === standing?.status) {
        const beaten = this.matched.opponentName;
        this.matched = null;
        return {
          ok: true,
          note: `the duel with ${beaten} is over and decided. Free to queue for another.`
        };
      }
      return {
        ok: false,
        note: `already in a duel with ${this.matched.opponentName}; that has to finish first`
      };
    }
    const match = await this.arena.call('arena_queue_match', {
      agent_id: this.agentId,
      scene_name: scene
    });
    if ('queued' === match?.status) {
      return {
        ok: true,
        note:
          `waiting at ${plainSceneName(scene)} for somebody to answer the challenge`
          + (opponent ? `, hoping for ${opponent}` : '')
          + '. Keep doing other things; the fight starts when somebody turns up.'
      };
    }
    const participants: Array<{ agentId?: string; playerName?: string }> = match?.participants ?? [];
    const other = participants.find((one) => one.agentId !== this.agentId);
    if (!match?.id || !other?.playerName) {
      return { ok: false, note: 'the queue did not answer sensibly; try again in a moment' };
    }
    this.matched = { matchId: String(match.id), opponentName: other.playerName, scene };
    const hoped = opponent?.trim().toLowerCase();
    const got = other.playerName.trim().toLowerCase();
    return {
      ok: true,
      note:
        `matched: a duel with ${other.playerName} at ${plainSceneName(scene)}`
        + (hoped && hoped !== got ? ` (you hoped for ${opponent}, but it is ${other.playerName} who answered)` : '')
        + '. Attack them by name when you are ready.'
    };
  }

  /**
   * The registered opponent as a live player target, if that is who was named.
   *
   * Gated on the match, not the capability. The capability says this character
   * may duel; the match says it is in one, with this person, and only somebody
   * both named by the match and actually standing here resolves. A duellist
   * cannot hit a bystander by asking, and cannot hit its opponent from another
   * room.
   */
  private opponentNamed(target: string): { sessionId: string; playerId: number; label: string } | null {
    if (!this.matched) {
      return null;
    }
    const wanted = target.trim().toLowerCase();
    const opponent = this.matched.opponentName.trim().toLowerCase();
    if (wanted !== opponent && !opponent.includes(wanted) && !wanted.includes(opponent)) {
      return null;
    }
    for (const person of this.people) {
      const name = (person.playerName ?? person.name ?? person.label ?? '').trim().toLowerCase();
      if (name === opponent && person.sessionId && person.playerId) {
        return {
          sessionId: String(person.sessionId),
          playerId: Number(person.playerId),
          label: this.matched.opponentName
        };
      }
    }
    return null;
  }

  async attack(target: string | undefined): Promise<ActionResult> {
    if (!this.can('fight')) {
      return { ok: false, note: 'this character does not fight' };
    }
    if (!target) {
      return { ok: false, note: 'nothing named to hit' };
    }
    const enemy = this.findNearby(target, 'enemy');
    if (!enemy) {
      // Not a monster: perhaps the person this character has agreed to fight.
      // Enemies resolve first so a duel never shadows the room's real
      // dangers, and the opponent path is gated on the match itself.
      const opponent = this.opponentNamed(target);
      if (opponent) {
        await this.arena.call('arena_basic_attack', {
          agent_id: this.agentId,
          target_session_id: opponent.sessionId,
          target_player_id: opponent.playerId
        });
        return { ok: true, note: `swung at ${opponent.label}` };
      }
      return { ok: false, note: `there is no "${target}" here to hit` };
    }
    // Attacking targets the objectIndex (layer_name+tile_index), a different
    // value from the objectId dialogue uses - see the comment on ArenaObject
    // in arena.ts and on target_object_index in the gateway's own tools.
    await this.arena.call('arena_basic_attack', {
      agent_id: this.agentId,
      target_object_index: enemy.objectIndex
    });
    return { ok: true, note: `swung at ${enemy.label}` };
  }

  /**
   * Use a named skill on something, which is the only way a fight looks like
   * anything in particular.
   *
   * Worth setting down, because everyone including me assumed otherwise: a
   * character's costume has nothing to do with how its attacks look. Every
   * class-path spritesheet in this world produces exactly four animations,
   * all of them walking, and there is no attack frame on any of them. The
   * swings and casts come from a separate set of effects keyed by SKILL, so
   * a mage in mage robes swinging the default attack is visually identical
   * to a swordsman doing the same thing. Dressing somebody as a mage does not
   * make them cast; casting makes them cast.
   *
   * The skills are real rows in the world and differ by class path: fireball
   * belongs to sorcerers, warlocks and journeymen, and a swordsman genuinely
   * does not have it. So what a character may use comes from its sheet rather
   * than from what it fancies, and asking for one it does not have is a plain
   * no rather than a silent fizzle.
   */
  /**
   * The way out for a character that has genuinely run out of ways to walk.
   *
   * It is the only thing in here that does not move a character by asking the
   * engine to move it, and it is deliberately unpleasant to reach for. Guy
   * spent real days in the volcano because every route out was refused and
   * nothing in his hands could tell the difference between "try again" and
   * "there is no way". This is that difference, made available once.
   *
   * The world does the actual moving, and even there it sets no position: the
   * character arrives at the inn on the inn's own return point, the same tile
   * anybody walking in off the street lands on. So the worst this can do is
   * put somebody at the bar who did not need to be.
   *
   * The wait afterwards is the point. Without one this stops being a last
   * resort and becomes a fast way across the map, and every locked door in the
   * world turns into a free trip home. An hour is long enough that a character
   * has to actually try the room it is in.
   */
  async giveUpAndWalkBack(): Promise<ActionResult> {
    if (!this.can('doors')) {
      return { ok: false, note: 'this character does not leave where it is' };
    }
    const waited = Date.now() - this.gaveUpAt;
    if (waited < GIVING_UP_AGAIN_MS) {
      const minutes = Math.ceil((GIVING_UP_AGAIN_MS - waited) / 60_000);
      return {
        ok: false,
        note:
          `already gave up and walked back once, and it is too soon to do it again: `
          + `${minutes} more minute${1 === minutes ? '' : 's'}. Whatever is wrong with this room `
          + `has to be walked out of. Try a door, or a different way across to one.`
      };
    }
    const result = await this.arena.call('arena_unstick', { agent_id: this.agentId });
    if (false === result?.moved) {
      // Not a failure worth spending the hour on: nothing moved, so nothing
      // was used up. ALREADY_AT_THE_INN is the common one and reads as a
      // character having lost track of where it is, which is worth saying.
      return {
        ok: false,
        note:
          'ALREADY_AT_THE_INN' === result?.reason
            ? 'already at the inn, so there is nowhere to be walked back to'
            : 'could not walk back just now; try again in a moment'
      };
    }
    this.gaveUpAt = Date.now();
    // Everything it thought it knew about where it was standing is now wrong.
    this.nearby = [];
    this.talking = null;
    return {
      ok: true,
      note:
        'gave up on getting out of there under your own steam and walked back to the inn, '
        + 'arriving at the door off the street. It cannot be done again for an hour.'
    };
  }

  async useSkill(skill: string | undefined, target: string | undefined): Promise<ActionResult> {
    if (!this.can('fight')) {
      return { ok: false, note: 'this character does not fight' };
    }
    if (!skill) {
      return { ok: false, note: 'no skill named to use' };
    }
    const known = this.skills.find((name) => name.toLowerCase() === skill.trim().toLowerCase());
    if (!known) {
      return {
        ok: false,
        note: this.skills.length
          ? `${skill} is not something this character can do; it knows ${this.skills.join(', ')}`
          : 'this character has no skills of its own to use'
      };
    }
    if (!target) {
      return { ok: false, note: `nothing named to use ${known} on` };
    }
    const enemy = this.findNearby(target, 'enemy');
    if (!enemy) {
      // Same second look attack takes: the registered opponent, and only the
      // registered opponent, standing in this room. See opponentNamed().
      const opponent = this.opponentNamed(target);
      if (opponent) {
        await this.arena.call('arena_use_action', {
          agent_id: this.agentId,
          action_type: known,
          target_session_id: opponent.sessionId,
          target_player_id: opponent.playerId
        });
        return { ok: true, note: `used ${known} on ${opponent.label}` };
      }
      return { ok: false, note: `there is no "${target}" here to use ${known} on` };
    }
    await this.arena.call('arena_use_action', {
      agent_id: this.agentId,
      action_type: known,
      target_object_index: enemy.objectIndex
    });
    return { ok: true, note: `used ${known} on ${enemy.label}` };
  }

  /**
   * Start a conversation with an NPC or trader standing nearby, found by
   * name against what notices() was just told, not by an object index no
   * character would ever think in.
   */
  async talkTo(target: string | undefined): Promise<ActionResult> {
    if (!this.can('talk_to_folk')) {
      return { ok: false, note: 'this character does not strike up conversation like that' };
    }
    if (!target) {
      return { ok: false, note: 'nobody named to talk to' };
    }
    const npc = this.findNearby(target, 'npc');
    if (!npc) {
      return { ok: false, note: `there is no "${target}" here to talk to` };
    }
    if (npc.objectId == null) {
      return { ok: false, note: `there is no way to open a conversation with ${npc.label}` };
    }
    const reply = await this.arena.call('arena_talk_to', {
      agent_id: this.agentId,
      object_id: npc.objectId
    });
    return this.describeNpcReply(npc.label, reply);
  }

  /** Pick one of the choices a conversation just offered. */
  async answerNpc(option: string | undefined): Promise<ActionResult> {
    if (!this.can('talk_to_folk')) {
      return { ok: false, note: 'this character does not strike up conversation like that' };
    }
    if (!this.talking) {
      return { ok: false, note: 'is not in the middle of talking to anyone' };
    }
    if (!option) {
      return { ok: false, note: 'did not say which answer to give' };
    }
    const key = this.matchOption(option);
    if (!key) {
      return { ok: false, note: `"${option}" was not one of the choices on offer` };
    }
    const label = this.talking.label;
    const reply = await this.arena.call('arena_choose', {
      agent_id: this.agentId,
      object_id: this.talking.objectId,
      option_key: key
    });
    return this.describeNpcReply(label, reply);
  }

  /**
   * Turn the gateway's reply from arena_talk_to/arena_choose into what the
   * character heard, and remember it for the next call so answer_npc knows
   * who it is still talking to and takeNpcReply() can hand the harness what
   * was actually said.
   */
  private describeNpcReply(npcLabel: string, reply: {
    opened: boolean;
    objectId: number;
    title?: string | null;
    content?: string | null;
    options?: Record<string, string> | null;
    message?: string;
  }): ActionResult {
    if (!reply.opened) {
      this.talking = null;
      this.lastReply = null;
      return { ok: false, note: reply.message ?? `${npcLabel} is too far away to talk to` };
    }
    const said = [reply.title, reply.content].filter(Boolean).join(': ').trim();
    this.talking = { objectId: reply.objectId, label: npcLabel, options: reply.options ?? null };
    this.lastReply = said ? { from: npcLabel, said } : null;
    const offered = this.talking.options ? Object.values(this.talking.options) : [];
    const heard = said || `${npcLabel} has nothing more to say`;
    const note = offered.length > 0
      ? `${heard}${/[.!?]$/.test(heard) ? '' : '.'} You can answer: ${offered.join(', ')}`
      : heard;
    return { ok: true, note };
  }

  /** Match what the character said back against the choices actually on offer. */
  private matchOption(said: string): string | null {
    const options = this.talking?.options;
    if (!options) {
      return null;
    }
    const wanted = said.trim().toLowerCase();
    const entries = Object.entries(options);
    return (
      entries.find(([key]) => key.toLowerCase() === wanted)?.[0]
      ?? entries.find(([, value]) => value.trim().toLowerCase() === wanted)?.[0]
      ?? entries.find(([, value]) => value.toLowerCase().includes(wanted))?.[0]
      ?? entries.find(([, value]) => wanted.includes(value.trim().toLowerCase()))?.[0]
      ?? null
    );
  }

  /**
   * What an NPC just told this character, for the harness to write into
   * memory as a first-hand finding - see noteToldByNpc() in npc.ts. Read
   * once and cleared, so the same line is not remembered twice.
   */
  takeNpcReply(): { from: string; said: string } | null {
    const reply = this.lastReply;
    this.lastReply = null;
    return reply;
  }

  /**
   * Count what is in the purse, and say what it is actually good for.
   *
   * The number on its own is a fortune with nothing behind it: every agent is
   * granted a large opening balance, nothing in the world sells anything, and a
   * character handed a bare figure decides it is rich and goes looking for a
   * land office to spend it at. Guy spent an evening walking to a council
   * building the Wanderer had invented, on the strength of ten thousand
   * coppers he cannot spend on anything. Saying so is not flavour, it is the
   * true state of the economy.
   */
  async checkMoney(): Promise<ActionResult> {
    if (!this.can('money')) {
      return { ok: false, note: 'this character has no purse' };
    }
    const balance = await this.arena.call('arena_credit_balance', { agent_id: this.agentId });
    return {
      ok: true,
      note: `has ${balance.balance} arena credits, which nothing here sells anything for yet`
    };
  }

  /**
   * Drink, eat, or otherwise use something out of the satchel.
   *
   * Not gated behind anything: using what you are already carrying is not a
   * trade and it is not a fight, and a character who has been handed a potion
   * should be able to drink it. What it is gated on is the item being real
   * and being usable, both of which the harness can see from the observation
   * it already has, so a wrong guess costs a plain sentence rather than a
   * round trip and a shrug from the world.
   */
  async useItem(item: string | undefined): Promise<ActionResult> {
    const usable = this.carried.filter((carried) => carried.usable);
    if (0 === usable.length) {
      return { ok: false, note: 'is carrying nothing that can be used' };
    }
    // Nothing named and only one thing it could possibly mean is not
    // ambiguous, it is a person saying "drink it".
    const meant = item ? this.carriedNamed(item) : (1 === usable.length ? usable[0] : null);
    if (!meant) {
      return {
        ok: false,
        note: item
          ? `is not carrying anything called "${item}"`
          : `did not say which to use: ${usable.map((carried) => carried.label).join(', ')}`
      };
    }
    if (!meant.usable) {
      return {
        ok: false,
        note: meant.equipped || meant.equipment
          ? `${meant.label} is worn, not drunk`
          : `${meant.label} is not something you use`
      };
    }
    const result = await this.arena.call('arena_use_item', {
      agent_id: this.agentId,
      item: meant.key
    });
    if (!result.used) {
      return { ok: false, note: result.message ?? `nothing came of using ${meant.label}` };
    }
    const left = Number(result.remaining ?? 0);
    return {
      ok: true,
      note: `used ${meant.label}${0 < left ? `, ${left} left` : ', the last one'}`
    };
  }

  /**
   * Take something off the floor. Walking over loot does nothing on this
   * world, so this is the only way anything a monster dropped ever reaches a
   * character's hands.
   */
  async pickUp(item: string | undefined): Promise<ActionResult> {
    if (0 === this.loot.length) {
      return { ok: false, note: 'there is nothing lying here to pick up' };
    }
    const wanted = item?.trim().toLowerCase();
    const drop = wanted
      ? this.loot.find((lying) => (lying.itemKey ?? '').toLowerCase().includes(wanted))
      : this.loot[0];
    if (!drop) {
      return { ok: false, note: `there is no "${item}" lying here` };
    }
    const result = await this.arena.call('arena_pick_up', {
      agent_id: this.agentId,
      drop_id: drop.dropId
    });
    if (!result.pickedUp) {
      return { ok: false, note: result.message ?? 'could not reach it' };
    }
    return { ok: true, note: `picked up ${result.item ?? 'what was lying there'}` };
  }

  /**
   * Buy something from the merchant standing here.
   *
   * With no item named this asks what is for sale instead of failing, because
   * that is what a person does on walking into a shop, and because a character
   * cannot name a thing it has never been shown. The answer comes back as the
   * note, which the harness puts in front of it on the very next tick.
   */
  async buy(item: string | undefined, quantity: number | undefined): Promise<ActionResult> {
    const counter = this.tradingWith();
    if (counter.refusal) {
      return counter.refusal;
    }
    const shop = counter.merchant as ArenaObject;
    if (!item) {
      return this.listOffers(shop, 'buy');
    }
    const result = await this.arena.call('arena_buy', {
      agent_id: this.agentId,
      object_id: shop.objectId,
      item,
      quantity: Math.max(1, Math.trunc(Number(quantity) || 1))
    });
    return this.tradeResult(shop, result, 'bought');
  }

  /** Sell something to the merchant standing here. Mirrors buy(). */
  async sell(item: string | undefined, quantity: number | undefined): Promise<ActionResult> {
    const counter = this.tradingWith();
    if (counter.refusal) {
      return counter.refusal;
    }
    const shop = counter.merchant as ArenaObject;
    if (!item) {
      return this.listOffers(shop, 'sell');
    }
    const result = await this.arena.call('arena_sell', {
      agent_id: this.agentId,
      object_id: shop.objectId,
      item,
      quantity: Math.max(1, Math.trunc(Number(quantity) || 1))
    });
    return this.tradeResult(shop, result, 'sold');
  }

  /**
   * The merchant this character is allowed to deal with, or the reason it is
   * not. Returns one or the other so buy() and sell() can share every refusal
   * without either of them repeating it.
   */
  private tradingWith(): { merchant?: ArenaObject; refusal?: ActionResult } {
    if (!this.can('trade')) {
      return { refusal: { ok: false, note: 'this character does not haggle' } };
    }
    const merchant = this.merchantHere();
    if (!merchant) {
      return { refusal: { ok: false, note: 'there is nobody here who keeps a shop' } };
    }
    return { merchant };
  }

  /** What is on the counter, said the way somebody would read it back. */
  private async listOffers(merchant: ArenaObject, side: 'buy' | 'sell'): Promise<ActionResult> {
    const listing = await this.arena.call('arena_trade_with', {
      agent_id: this.agentId,
      object_id: merchant.objectId,
      side
    });
    if (!listing.opened) {
      return { ok: false, note: listing.message ?? `${merchant.label} is too far off to trade with` };
    }
    const offers = (listing.offers ?? []) as Array<{
      label: string;
      price?: { itemKey: string; quantity: number } | null;
      payout?: { itemKey: string; quantity: number } | null;
    }>;
    if (0 === offers.length) {
      return {
        ok: true,
        note: 'buy' === side
          ? `${merchant.label} has nothing for sale`
          : `${merchant.label} does not want anything you are carrying`
      };
    }
    const said = offers.map((offer) => {
      const cost = 'buy' === side ? offer.price : offer.payout;
      return cost ? `${offer.label} for ${cost.quantity} ${cost.itemKey}` : `${offer.label} (no price)`;
    });
    return {
      ok: true,
      note: 'buy' === side
        ? `${merchant.label} sells: ${said.join(', ')}`
        : `${merchant.label} will pay for: ${said.join(', ')}`
    };
  }

  /** One completed - or refused - transaction, in a sentence. */
  private tradeResult(
    merchant: ArenaObject,
    result: {
      traded?: boolean;
      message?: string;
      item?: { label?: string };
      quantity?: number;
      price?: { itemKey: string; quantity: number } | null;
      payout?: { itemKey: string; quantity: number } | null;
    },
    verb: 'bought' | 'sold'
  ): ActionResult {
    if (!result.traded) {
      return { ok: false, note: result.message ?? `${merchant.label} would not do it` };
    }
    const what = result.item?.label ?? 'it';
    const many = 1 < Number(result.quantity ?? 1) ? ` x${result.quantity}` : '';
    const money = 'bought' === verb ? result.price : result.payout;
    const price = money ? ` for ${money.quantity} ${money.itemKey}` : '';
    return { ok: true, note: `${verb} ${what}${many}${price} from ${merchant.label}` };
  }

  async observe(): Promise<Observation> {
    return this.arena.call('arena_observe', { agent_id: this.agentId });
  }

  /**
   * Watch until the body arrives or stops moving, then let it settle.
   *
   * The settle matters: a body still sliding when the next decision is made
   * reports a position it is about to leave, and the character decides where
   * to go next from where it briefly was. Waiting for it to actually stop
   * costs half a second and makes every following observation true.
   */
  private async waitForArrival(x: number, y: number): Promise<boolean> {
    const deadline = Date.now() + LEG_TIMEOUT_MS;
    let last: string | null = null;
    let still = 0;
    while (Date.now() < deadline) {
      await sleep(WALK_POLL_MS);
      const observation = await this.observe();
      const state = observation.ownPlayer?.state ?? {};
      const here = { x: Number(state.x ?? 0), y: Number(state.y ?? 0) };
      if (Math.abs(here.x - x) <= ARRIVAL_PIXELS && Math.abs(here.y - y) <= ARRIVAL_PIXELS) {
        await sleep(SETTLE_MS);
        return true;
      }
      // Leaving the room mid-walk counts as done; something else moved us.
      if (sceneOf(observation) === '') {
        return false;
      }
      const key = `${here.x},${here.y}`;
      if (key === last) {
        still += 1;
        if (still >= STILL_POLLS) {
          return false;
        }
      } else {
        still = 0;
      }
      last = key;
    }
    return false;
  }
}

export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * Turn what a character meant to say into the chat lines it actually sends.
 *
 * Two separate limits are at work. How much a character says is a trait: an
 * innkeeper who has told the same story for twenty years runs on, and a
 * wanderer who has spoken to nobody for a week does not. That is `maxWords`,
 * and it is enforced by dropping whole sentences, never by cutting one short.
 *
 * How much fits in one chat line is Reldens': 100 characters. So the kept
 * sentences are packed into lines under that limit and sent in sequence, which
 * is what a person typing in a chat box does anyway.
 */
export function toSpeech(raw: string, maxWords: number = DEFAULT_WORDS): string[] {
  // Models narrate themselves in asterisks however firmly they are told not
  // to. Cut what is between them, not just the markers: stripping only the
  // asterisks turns "*Guy shrugs.* Fine." into a character announcing that he
  // shrugs, which is worse than the stage direction was.
  const text = raw
    .replace(/\*[^*]*\*/g, ' ')
    .replace(/\*/g, '')
    .replace(/\s+/g, ' ')
    .trim()
    .replace(/^["“”]|["“”]$/g, '');
  if (!text) {
    return [];
  }
  const budget = Math.max(1, Math.min(Math.round(maxWords), MAX_WORDS));
  const kept: string[] = [];
  let spent = 0;
  for (const sentence of splitSentences(text)) {
    const words = countWords(sentence);
    // The first sentence always goes, however long: a character cut off
    // before it has said anything is worse than one that ran over.
    if (kept.length > 0 && spent + words > budget) {
      break;
    }
    kept.push(sentence);
    spent += words;
    if (spent >= budget) {
      break;
    }
  }
  return packIntoLines(kept).slice(0, MAX_LINES);
}

/** Words too common to say anything about what a line is about. */
const FILLER = new Set([
  'the', 'a', 'an', 'and', 'but', 'or', 'so', 'is', 'it', 'its', 'was', 'be',
  'been', 'to', 'of', 'in', 'on', 'at', 'for', 'with', 'that', 'this', 'there',
  'here', 'you', 'your', 'i', 'im', 'me', 'my', 'we', 'they', 'he', 'she',
  'not', 'no', 'yes', 'do', 'does', 'did', 'have', 'has', 'had', 'will',
  'would', 'can', 'could', 'if', 'as', 'up', 'out', 'about', 'just', 'like',
  'what', 'who', 'how', 'why', 'when', 'where', 'still', 'got', 'get', 'one'
]);

function contentWords(line: string): Set<string> {
  return new Set(
    line
      .toLowerCase()
      .replace(/[^a-z\s]/g, ' ')
      .split(/\s+/)
      .filter((word) => word.length > 2 && !FILLER.has(word))
  );
}

/**
 * Whether a character is about to say something it has effectively just said.
 *
 * Exact-match checks catch nothing, because a model never repeats itself
 * word for word: it says "the road's the same as ever" and then "same road as
 * always" and sounds like a broken toy. Comparing what a line is *about*
 * catches that.
 */
export function isTooSimilar(line: string, recent: string[], threshold = 0.6): boolean {
  const words = contentWords(line);
  if (words.size === 0) {
    return recent.some((said) => said.toLowerCase() === line.toLowerCase());
  }
  for (const said of recent) {
    const before = contentWords(said);
    if (before.size === 0) {
      continue;
    }
    let shared = 0;
    for (const word of words) {
      if (before.has(word)) {
        shared++;
      }
    }
    // Against the shorter line, so a long rambling repeat of a short remark
    // still counts as the same remark.
    if (shared / Math.min(words.size, before.size) >= threshold) {
      return true;
    }
  }
  return false;
}

/**
 * A subject a character keeps coming back to. Naming it in the prompt works
 * far better than a general plea not to repeat itself, which models agree to
 * and then ignore.
 */
export function harpingOn(recent: string[], minLines = 3): string {
  if (recent.length < minLines) {
    return '';
  }
  const counts = new Map<string, number>();
  for (const line of recent.slice(-6)) {
    for (const word of contentWords(line)) {
      counts.set(word, (counts.get(word) ?? 0) + 1);
    }
  }
  const stuck = [...counts.entries()]
    .filter(([, count]) => count >= minLines)
    .sort((left, right) => right[1] - left[1])
    .slice(0, 2)
    .map(([word]) => word);
  return stuck.length === 0
    ? ''
    : `You have brought up ${stuck.map((word) => `"${word}"`).join(' and ')} in most of `
      + 'your last few lines. Talk about something else, or say nothing.';
}

function splitSentences(text: string): string[] {
  const sentences: string[] = [];
  let current = '';
  for (const character of text) {
    current += character;
    if ('.!?'.includes(character)) {
      sentences.push(current.trim());
      current = '';
    }
  }
  if (current.trim()) {
    sentences.push(current.trim());
  }
  return sentences;
}

function countWords(text: string): number {
  return text.split(/\s+/).filter(Boolean).length;
}

/** Fill each chat line as full as it will go, breaking only between words. */
function packIntoLines(sentences: string[]): string[] {
  const lines: string[] = [];
  let line = '';
  const push = () => {
    if (line) {
      lines.push(line);
      line = '';
    }
  };
  for (const sentence of sentences) {
    for (const piece of sentence.length > CHAT_LINE_LIMIT ? breakUp(sentence) : [sentence]) {
      const candidate = line ? `${line} ${piece}` : piece;
      if (candidate.length > CHAT_LINE_LIMIT) {
        push();
        line = piece;
        continue;
      }
      line = candidate;
    }
  }
  push();
  return lines;
}

/** A sentence too long for one line, split on words as late as it can be. */
function breakUp(sentence: string): string[] {
  const pieces: string[] = [];
  let rest = sentence;
  while (rest.length > CHAT_LINE_LIMIT) {
    let cut = rest.lastIndexOf(' ', CHAT_LINE_LIMIT);
    if (cut <= 0) {
      cut = CHAT_LINE_LIMIT;
    }
    pieces.push(rest.slice(0, cut).trim());
    rest = rest.slice(cut).trim();
  }
  if (rest) {
    pieces.push(rest);
  }
  return pieces;
}
