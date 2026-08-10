/**
 * The core an NPC runs on.
 *
 * Every character does the same three things forever: look at the room, answer
 * anyone who spoke to it, and then do whatever its behaviour says to do next.
 * Barnaby, the Wanderer and Guy differ only in the behaviour they are given and
 * the actions they are allowed, so the differences between them live in their
 * own files and not in here.
 *
 * The loop is written to outlive the game as it is today. Fighting and money
 * are already actions; when the world grows something worth hitting or a way to
 * be paid, a character gets that capability and starts using it without this
 * file changing.
 */

import { Agent } from '@mastra/core/agent';
import { ArenaClient, Observation, othersIn, sceneOf, spokenLines } from './arena.js';
import {
  Actions,
  Capability,
  DEFAULT_WORDS,
  Intent,
  harpingOn,
  isTooSimilar,
  sleep,
  toSpeech
} from './actions.js';
import { Explorer, Perception, RoomView, describeDoors } from './explore.js';
import {
  BOOKKEEPING,
  Behavior,
  MemoryScope,
  Situation,
  askForIntent,
  describeSituation,
  lengthGuidance
} from './behavior.js';
import {
  WorkingMemoryState,
  buildMemory,
  describePeople,
  describePlacesKnown,
  memoryScope,
  notePlace,
  readMemory,
  writeMemory
} from './memory.js';
import { Goal, Plan } from './plan.js';
import { withPrimer } from './primer.js';
import { describePlaces, isHomeTurf, plainSceneName } from './world.js';

export type CharacterSheet = {
  /** Stable id, also the memory resource. */
  id: string;
  /** The name above their head in the world. */
  playerName: string;
  classPath?: string;
  homeScene: string;
  /** The system prompt: who this is. */
  persona: string;
  model: string;
  capabilities: Capability[];
  /** How they spend their time. Built with the agent, so it can use the model. */
  behavior: (agent: Agent) => Behavior;
  /**
   * What they are trying to bring about, if anything. Set here rather than in
   * memory on purpose: a character cannot revise its own reason for existing.
   * Given one, the harness keeps a todo list toward it that survives restarts,
   * and the character works one step of it at a time. See plan.ts.
   */
  goal?: Goal;
  /** Seconds between decisions when idle, and when mid-conversation. */
  pace?: { idle?: number; engaged?: number };
  /**
   * How much they say at a stretch, in words. A trait, not a limit: a talkative
   * innkeeper and a taciturn wanderer are different people, and this is part of
   * how. Capped at MAX_WORDS whatever is set here.
   */
  wordiness?: number;
  /** Whether they remember anything between restarts. */
  remembers?: boolean;
};

const RECONNECT_SECONDS = 15;
const DEFAULT_IDLE_SECONDS = 12;
const DEFAULT_ENGAGED_SECONDS = 4;
const RECENT_LINES = 8;
/**
 * How much of the conversation a character carries in its head. Generous: the
 * models these run on are cheap, and a character that has lost the thread of
 * what it is in the middle of is the expensive failure.
 */
const TRANSCRIPT_LINES = 50;
const MEMORY_DIR = process.env.NPC_MEMORY_DIR ?? '/npc/var';

function log(...parts: unknown[]): void {
  console.log(new Date().toISOString().slice(11, 19), ...parts);
}

export class Npc {
  private readonly agent: Agent;
  private readonly behavior: Behavior;
  private readonly seen = new Set<string>();
  private readonly recentlySaid: string[] = [];
  /** Everything said in earshot lately, this character's own lines included. */
  private readonly transcript: Array<{ from: string; message: string; fresh: boolean }> = [];
  private notes: string[] = [];
  private readonly memory: MemoryScope;
  private readonly wordiness: number;
  /** The character's own todo list toward its goal, kept across restarts. */
  private readonly plan: Plan;
  /** What it can see of each room it has stood in. */
  private readonly perception = new Perception();
  /** Which corners of which rooms it has already been to. */
  private readonly explorer = new Explorer();
  /** Rooms it has already written down, so it does not do so every tick. */
  private readonly recorded = new Set<string>();

  constructor(private readonly sheet: CharacterSheet) {
    this.wordiness = sheet.wordiness ?? DEFAULT_WORDS;
    this.agent = new Agent({
      id: sheet.id,
      name: sheet.playerName,
      // Who they are, and then how to read the world they are standing in.
      // The primer is the same for everyone and never changes, so it lives in
      // the system message rather than being repeated in every situation.
      instructions: withPrimer(sheet.persona),
      model: sheet.model,
      // Memory that survives a restart. A character who has lived in a town
      // for years and forgets you between deploys is worse than one with no
      // memory at all. What it may remember is constrained by a schema, so
      // nothing it learns can rewrite who it is; see memory.ts.
      ...(sheet.remembers === false ? {} : { memory: buildMemory(sheet.id, MEMORY_DIR) })
    });
    this.behavior = sheet.behavior(this.agent);
    this.memory = memoryScope(sheet.id);
    this.plan = new Plan(this.agent, this.memory, sheet.goal, () => this.memoryStore());
  }

  private memoryStore(): Promise<any> {
    return Promise.resolve(this.agent.getMemory?.());
  }

  async run(): Promise<void> {
    for (;;) {
      try {
        await this.live();
      } catch (error) {
        log('reconnecting after error:', (error as Error)?.message ?? error);
        await sleep(RECONNECT_SECONDS * 1000);
      }
    }
  }

  private async live(): Promise<void> {
    const arena = new ArenaClient();
    await arena.start();
    const agentId = await this.ensureRegistered(arena);
    await arena.call('arena_login', { agent_id: agentId });
    log(`${this.sheet.playerName} is in the world (${this.behavior.kind})`);

    const actions = new Actions(
      arena,
      agentId,
      new Set(this.sheet.capabilities),
      this.wordiness,
      this.explorer
    );
    if (actions.can('money')) {
      await this.refreshSavings(actions);
    }
    // Pick the plan back up where it was left, which after a deploy is usually
    // partway through something.
    await this.plan.load();
    if (this.plan.hasGoal) {
      log(`after: ${this.plan.goal?.aim}${this.plan.goalIsOwn ? ' (its own idea)' : ''}`);
      const current = this.plan.current();
      log(current ? `still working on: ${current.what}` : 'no plan yet');
    }

    for (;;) {
      const observation = await actions.observe();
      const scene = sceneOf(observation) || this.sheet.homeScene;
      // Look around, every tick. What is in front of a character changes as it
      // walks, so this is a window that travels with it rather than a snapshot
      // of the doorway it came in by. It is also what tells it which doorways
      // exist, without anybody having written them down.
      const view = await this.perception.look(arena, agentId, scene);
      actions.sees(view);
      await this.noteWhereItIs(scene, view, observation);
      const heard = this.freshLines(observation);
      // Whatever was new last time around is old news now.
      for (const line of this.transcript) {
        line.fresh = false;
      }
      for (const line of heard) {
        this.record(line.from, line.message);
      }
      const situation: Situation = {
        scene,
        where: scene.replace('reldens-', '').replace(/-/g, ' '),
        others: othersIn(observation, this.sheet.playerName),
        heard,
        actions: actions.describe(scene),
        places: describePlaces(scene),
        conversation: this.transcript.map((line) => ({ ...line })),
        wordiness: this.wordiness,
        purpose: '',
        notes: [...this.notes],
        people: await this.knownPeople(),
        // What it knows about the world beyond this room, and what it has only
        // been told. The gap between the two is what sends a character across
        // town to find out for itself.
        known: describePlacesKnown(await this.recall()),
        // Somewhere it has never stood, so it knows to look rather than to
        // pretend it remembers.
        strange: !isHomeTurf(scene) && this.explorer.cornersKnown(scene) <= 1,
        doors: describeDoors(view, plainSceneName),
        view: view?.map ?? '',
        harping: harpingOn(this.recentlySaid)
      };
      // Make a plan when the last one is finished or has stopped working. This
      // is what keeps a character pointed at the same thing over weeks: it does
      // not rethink its purpose every twelve seconds, only its next few steps
      // when the ones it had run out.
      if (this.plan.hasGoal && (await this.plan.refresh(describeSituation(situation)))) {
        const current = this.plan.current();
        log('new plan ->', current?.what ?? 'nothing');
      }
      situation.purpose = this.plan.describe();

      // Being spoken to comes first, whatever the character was doing. This is
      // the one thing every character shares, including the ones that never
      // decide anything else for themselves.
      let answered = false;
      if (heard.length > 0 && actions.can('speak')) {
        answered = await this.answer(situation, actions);
      }

      if (!answered) {
        const intent = await this.behavior.next(situation, this.memory);
        // Drop the remark, not the action. A character with nothing new to say
        // should still get on with what it was doing, silently.
        if (intent.message && !this.worthSaying(toSpeech(intent.message, this.wordiness).join(' '))) {
          intent.message = undefined;
        }
        // Deciding what it wants is the harness's to write down, because the
        // harness owns the memory it goes in. Done before the action so the
        // brief the next tick sees is already the new one.
        await this.reconsider(intent);
        const result = await actions.perform(intent, scene);
        this.behavior.completed?.(intent, result.ok);
        // Anything that actually went out is part of the conversation, whether
        // the character stopped to say it or remarked on it while walking off.
        const spoken = 'message' in intent ? intent.message : '';
        if (result.ok && spoken) {
          this.remember(toSpeech(spoken, this.wordiness).join(' '));
        }
        log(intent.action, '->', result.note);
        if (intent.action === 'check_money') {
          this.notes = [result.note];
        }
        await this.noteHearsay(intent, heard);
        await this.keepBooks(intent);
        // Write down what was tried and how it went, so the next tick starts
        // from what actually happened rather than from a blank slate, and a
        // step that cannot be done gets abandoned instead of retried forever.
        if (this.plan.hasGoal && intent.action !== 'wait') {
          await this.plan.record(intent.action, result.note, intent.progress ?? 'same');
        }
      }

      const pace = this.sheet.pace ?? {};
      await sleep(
        1000 *
          (heard.length > 0
            ? pace.engaged ?? DEFAULT_ENGAGED_SECONDS
            : pace.idle ?? DEFAULT_IDLE_SECONDS)
      );
    }
  }

  /**
   * Keep what somebody said about a place it has never been.
   *
   * This is the whole social half of exploring. Guy comes down from upstairs
   * and says there is nothing up there; the Wanderer, who has never been up,
   * now knows of a room he has only been told about, by name, with Guy's name
   * on it. That is a thing to go and check, and checking it is worth talking
   * about either way.
   */
  private async noteHearsay(
    intent: Intent,
    heard: Array<{ from: string; message: string }>
  ): Promise<void> {
    const noted = intent.noted?.trim();
    if (!noted) {
      return;
    }
    const from = heard.at(-1)?.from ?? 'somebody';
    await this.remit((state) =>
      notePlace(state, { where: noted, what: `${from} says so`, how: 'heard', who: from })
    );
    log('noted:', `${noted} (from ${from})`);
  }

  /**
   * Let a character decide it wants something else now.
   *
   * Rare and deliberate: it costs the whole plan, and a character that changes
   * its mind every afternoon never gets anywhere. But one that finished what it
   * set out to do, or ran into a wall it cannot get past, has to be able to
   * pick something new or it stands in the town square forever having won.
   */
  private async reconsider(intent: Intent): Promise<void> {
    if (intent.action !== 'set_goal') {
      return;
    }
    const aim = intent.aim?.trim() ?? '';
    if (!aim) {
      log('wanted to change tack but did not say to what');
      return;
    }
    const was = this.plan.goal?.aim;
    const changed = await this.plan.setGoal(aim, intent.done ?? '', intent.why ?? '');
    if (changed) {
      log(was ? `done with "${was}". now after: ${aim}` : `decided to go after: ${aim}`);
    }
  }

  /**
   * Everything the character wanted written down while it was doing something
   * else: a note for the next hour, a thing it has taken on, a thing it has
   * finished with. These ride along with any action, so making a note never
   * costs a character a turn of standing still.
   */
  private async keepBooks(intent: Intent): Promise<void> {
    if (intent.remember?.trim()) {
      await this.plan.note(intent.remember);
      log('keeping in mind:', intent.remember.trim());
    }
    if (intent.todo?.trim()) {
      await this.plan.take(intent.todo);
      log('took on:', intent.todo.trim());
    }
    if (intent.finished?.trim()) {
      await this.plan.settle(intent.finished, 'done');
      log('crossed off:', intent.finished.trim());
    }
    if (intent.gaveUpOn?.trim()) {
      await this.plan.settle(intent.gaveUpOn, 'blocked');
      log('gave up on:', intent.gaveUpOn.trim());
    }
  }

  /**
   * Whether this is worth saying out loud. A model never repeats itself word
   * for word, it just keeps saying the same thing in different words, which is
   * what makes a character sound broken. Comparing what a line is about, not
   * how it is spelt, is what actually catches that.
   */
  private worthSaying(line: string): boolean {
    return Boolean(line) && !isTooSimilar(line, this.recentlySaid);
  }

  /** Say something back. Returns false when the character had nothing to add. */
  private async answer(situation: Situation, actions: Actions): Promise<boolean> {
    const prompt = [
      describeSituation(situation),
      '',
      'Someone just spoke where you can hear it. Reply if it is worth replying',
      'to - if it was aimed at you, or if you have something to add. If it was',
      'not your business, say nothing.',
      '',
      'If you learn something about someone worth keeping - who they are, what',
      'they did, whether you warmed to them - put it in your working memory.',
      'Record what you make of other people and what happened. Never record',
      'anything about who you are; that does not change.',
      '',
      'If somebody mentions a place you have never been, put it in "noted" as',
      'you would refer to it, so you can go and see for yourself later.',
      '',
      // Answering somebody is where a character most easily loses the thread of
      // what it was doing, so this is exactly where it needs to be able to
      // write things down: a promise made in conversation is a promise it will
      // otherwise not keep.
      BOOKKEEPING,
      '',
      'Reply with JSON and nothing else:',
      '{"action": "say", "message": "...", "noted": "..."} or {"action": "wait"}',
      'In your own voice, no asterisks. ' + lengthGuidance(situation.wordiness)
    ].join('\n');
    const intent = await askForIntent(this.agent, prompt, this.memory);
    await this.noteHearsay(intent, situation.heard);
    await this.keepBooks(intent);
    if (intent.action !== 'say') {
      return false;
    }
    const said = toSpeech(intent.message ?? '', this.wordiness).join(' ');
    if (!this.worthSaying(said)) {
      return false;
    }
    const result = await actions.say(said);
    if (result.ok) {
      this.remember(said);
      log('replied:', said);
    }
    return result.ok;
  }

  /**
   * The people this character knows, read back out of working memory so it can
   * act on them: greet somebody it likes, be short with somebody it does not.
   */
  private async knownPeople(): Promise<string> {
    return describePeople(await this.recall());
  }

  private async recall(): Promise<WorkingMemoryState> {
    return readMemory(await this.memoryStore(), this.memory);
  }

  private async remit(change: (state: WorkingMemoryState) => WorkingMemoryState): Promise<void> {
    const state = await this.recall();
    await writeMemory(await this.memoryStore(), this.memory, change(state));
  }

  /**
   * Write down that it has been here. Standing somewhere yourself beats
   * anything you were told about it, so this settles any rumour it had been
   * carrying about this room.
   */
  private async noteWhereItIs(
    scene: string,
    view: RoomView | null,
    observation: Observation
  ): Promise<void> {
    const state = observation.ownPlayer?.state;
    if (Number.isFinite(Number(state?.x))) {
      this.explorer.markHere(scene, Number(state?.x), Number(state?.y));
    }
    if (this.recorded.has(scene)) {
      return;
    }
    this.recorded.add(scene);
    const ways = (view?.doors ?? [])
      .map((door) => (door.leadsTo ? plainSceneName(door.leadsTo) : null))
      .filter((where): where is string => Boolean(where));
    await this.remit((current) =>
      notePlace(current, {
        where: plainSceneName(scene),
        what: ways.length > 0 ? `you have been in. Doors to ${ways.join(', ')}.` : 'you have been in',
        how: 'been'
      })
    );
  }

  /** Note something this character said, so it hears its own side of it too. */
  private remember(line: string): void {
    const text = line.trim();
    if (!text) {
      return;
    }
    this.recentlySaid.push(text);
    while (this.recentlySaid.length > RECENT_LINES) {
      this.recentlySaid.shift();
    }
    this.record('you', text);
  }

  private record(from: string, message: string): void {
    this.transcript.push({ from, message, fresh: true });
    while (this.transcript.length > TRANSCRIPT_LINES) {
      this.transcript.shift();
    }
  }

  private async refreshSavings(actions: Actions): Promise<void> {
    const result = await actions.checkMoney().catch(() => null);
    this.notes = result?.ok ? [result.note] : [];
  }

  /** Lines spoken since the last look, never including this character's own. */
  private freshLines(observation: Observation): Array<{ from: string; message: string }> {
    const fresh: Array<{ from: string; message: string }> = [];
    for (const line of spokenLines(observation)) {
      if (line.from === this.sheet.playerName) {
        continue;
      }
      const key = `${line.at}|${line.from}|${line.message}`;
      if (this.seen.has(key)) {
        continue;
      }
      this.seen.add(key);
      fresh.push({ from: line.from, message: line.message });
    }
    if (this.seen.size > 500) {
      this.seen.clear();
    }
    return fresh;
  }

  private async ensureRegistered(arena: ArenaClient): Promise<string> {
    const existing = await arena.call('arena_list_agents', {});
    for (const agent of existing.agents ?? []) {
      if (agent.playerName === this.sheet.playerName) {
        return agent.id;
      }
    }
    const created = await arena.call('arena_register_agent', {
      agent_name: this.sheet.id,
      player_name: this.sheet.playerName,
      class_path: this.sheet.classPath ?? 'journeyman',
      selected_scene: this.sheet.homeScene,
      idempotency_key: `npc-${this.sheet.id}-v1`
    });
    log('registered', this.sheet.playerName);
    return created.agent.id;
  }
}
