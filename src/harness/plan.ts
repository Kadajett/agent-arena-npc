/**
 * How a character gets anywhere over weeks rather than seconds.
 *
 * A model asked "what do you do next?" every twelve seconds, with nothing but
 * the room in front of it, does not pursue anything. It reacts. It will walk to
 * the east gate, forget why, and walk back. Long-run purpose needs three things
 * that a single prompt cannot supply:
 *
 *   1. A goal that is in front of it every single time it is asked anything,
 *      not once at startup. A goal mentioned in the first prompt of the day and
 *      never again is a goal the character has effectively forgotten by lunch.
 *   2. A list it did not have to remember making. That lives in working memory,
 *      is written by this file rather than by the model's goodwill, and survives
 *      restarts.
 *   3. Knowing how the last few attempts went, so a step that cannot be done is
 *      abandoned rather than retried forever.
 *
 * The cycle is: plan, do one step, record what happened, and when the list runs
 * out or goes nowhere, plan again. The model chooses the steps and judges the
 * progress; the harness holds the list and the honesty.
 *
 * The goal itself is the character's. It can set one, and it can decide an old
 * one is finished or hopeless and pick another, because a person who completes
 * what they set out to do and then wants nothing is not a person. What it
 * cannot do is change who it is: the persona is a system message that memory
 * never touches. Wanting something new is not becoming someone else.
 *
 * A character sheet may still hand over a starting goal. That seeds memory the
 * first time and can be changed between deploys, but it does not overwrite a
 * goal the character chose for itself; see load().
 */

import { Agent } from '@mastra/core/agent';
import { z } from 'zod';
import { MemoryScope, REPLY_BUDGET } from './behavior.js';
import {
  StepState,
  WorkingMemoryState,
  addTodo,
  describeNotes,
  describeTodo,
  noteLately,
  noteToSelf,
  readMemory,
  settleTodo,
  writeMemory
} from './memory.js';

/**
 * What a character is after. A sheet may supply a starting one; from then on it
 * is the character's, kept in memory and revisable by it.
 */
export type Goal = {
  /** What they are trying to bring about, in a line. */
  aim: string;
  /** How they would know they had got there, if they ever can. */
  done?: string;
  /** Why they settled on it, in their own words. Empty for one they were given. */
  why?: string;
};

export type Step = { what: string; status: StepState; note: string };

/** What the model reports about the step it was working on. */
export type Progress = 'same' | 'done' | 'blocked';

const MAX_STEPS = 6;
/** After this many blocked steps in a row, the plan is not working. Start over. */
const BLOCKED_BEFORE_REPLAN = 3;

const StepsSchema = z.object({
  steps: z
    .array(z.string().min(1))
    .min(1)
    .max(MAX_STEPS)
    .describe('what to do next, in order, each one concrete enough to just do')
});

/**
 * A character's plan: read from memory at startup, written back on every
 * change, and re-made by the model when it runs out.
 */
export class Plan {
  private state: WorkingMemoryState | null = null;
  private replanning = false;

  constructor(
    private readonly agent: Agent,
    private readonly scope: MemoryScope,
    /** The goal the character sheet handed over, if any. Only ever a seed. */
    private readonly seed: Goal | undefined,
    private readonly memoryOf: () => Promise<any>
  ) {}

  /** What the character is actually after: its own, or the one it was given. */
  get goal(): Goal | null {
    const own = this.state?.goal;
    if (own?.aim) {
      return { aim: own.aim, done: own.done || undefined, why: own.why || undefined };
    }
    return this.seed?.aim ? this.seed : null;
  }

  get hasGoal(): boolean {
    return Boolean(this.goal?.aim);
  }

  /** Whether the character chose this itself rather than being handed it. */
  get goalIsOwn(): boolean {
    return Boolean(this.state?.goal?.aim);
  }

  /**
   * Pick everything back up: the goal, the plan toward it, the list, the notes.
   *
   * Two goals can be in play, and which wins matters. A goal the character
   * chose stands, because taking it away every restart makes choosing one
   * pointless. A goal on the sheet seeds an empty memory, and if the operator
   * edits the sheet to something new, that new one is adopted: that is the
   * only way to redirect a character that has settled on something.
   *
   * Whichever wins, a plan made for a different goal is dropped. Reload one
   * against the other and the character works a list toward something nobody
   * wants any more, which is what Guy did: told to find out what was upstairs,
   * he went on asking the price of a field.
   */
  async load(): Promise<void> {
    const state = await readMemory(await this.memoryOf(), this.scope);
    this.state = state;
    const sheetAim = this.seed?.aim ?? '';
    const changedOnTheSheet = Boolean(sheetAim) && state.goalSeed !== sheetAim;
    if (changedOnTheSheet) {
      if (state.goal.aim && state.goal.aim !== sheetAim) {
        console.log(`told to want something else; setting aside "${state.goal.aim}"`);
      }
      this.state = {
        ...state,
        goal: {
          aim: sheetAim,
          done: this.seed?.done ?? '',
          why: '',
          setAt: new Date().toISOString()
        },
        goalSeed: sheetAim
      };
    }
    const aim = this.goal?.aim ?? '';
    if (this.state.planFor !== aim) {
      if (this.state.plan.length > 0) {
        // Say so. A plan quietly vanishing looks identical to a plan that was
        // never saved, and only one of those is a bug.
        console.log(
          `the goal changed; dropping ${this.state.plan.length} step(s) planned for the old one`
        );
      }
      this.state = { ...this.state, plan: [], planFor: aim };
    }
    await this.save();
  }

  /**
   * Settle on something new to want.
   *
   * The old plan goes with it. Keeping steps written toward the last goal is
   * how a character ends up walking somewhere it no longer has a reason to be.
   */
  async setGoal(aim: string, done: string, why: string): Promise<boolean> {
    const wanted = aim.trim();
    if (!this.state || !wanted || wanted.toLowerCase() === (this.goal?.aim ?? '').toLowerCase()) {
      return false;
    }
    const previous = this.goal?.aim;
    this.state = {
      ...this.state,
      goal: { aim: wanted, done: done.trim(), why: why.trim(), setAt: new Date().toISOString() },
      plan: [],
      planFor: wanted
    };
    if (previous) {
      this.state = noteLately(this.state, `gave up on "${previous}" and decided on "${wanted}"`);
    }
    await this.save();
    return true;
  }

  /** Take something on that is nothing to do with the goal. */
  async take(what: string): Promise<void> {
    if (!this.state) {
      return;
    }
    this.state = addTodo(this.state, what);
    await this.save();
  }

  /** Cross something off the list, or give up on it. */
  async settle(which: string, status: StepState): Promise<void> {
    if (!this.state) {
      return;
    }
    this.state = settleTodo(this.state, which, status);
    await this.save();
  }

  /** Hold something in mind for the next hour. */
  async note(text: string): Promise<void> {
    if (!this.state) {
      return;
    }
    this.state = noteToSelf(this.state, text);
    await this.save();
  }

  get steps(): Step[] {
    return (this.state?.plan ?? []) as Step[];
  }

  /** The step being worked on: the first one not finished with. */
  current(): Step | null {
    return this.steps.find((step) => step.status === 'next' || step.status === 'doing') ?? null;
  }

  /**
   * The standing brief: what it wants, where it has got to, what it owes, and
   * what it is holding in mind. This goes into every single prompt, including
   * the ones that are only about answering somebody. That is the whole point.
   *
   * A goal stated once at startup is gone by the second conversation, because
   * the only thing in front of the model after that is a room and a line of
   * dialogue, and it will answer the dialogue. Repeating the brief every time
   * costs a few hundred tokens and is the difference between a character that
   * is up to something and one that is making small talk forever.
   */
  describe(now: number = Date.now()): string {
    const goal = this.goal;
    const blocks: string[] = [];
    if (goal?.aim) {
      const lines = [`What you are after: ${goal.aim}`];
      if (goal.done) {
        lines.push(`You will have got there when: ${goal.done}`);
      }
      if (this.goalIsOwn && goal.why) {
        // In its own words, so it reads as something it decided rather than
        // something it was handed. A character argues itself out of an
        // instruction more easily than out of its own reason.
        lines.push(`You settled on this yourself because: ${goal.why}`);
      }
      blocks.push(lines.join('\n'));
    }
    const steps = this.steps;
    if (steps.length > 0) {
      const lines = ['Your plan:'];
      for (const step of steps) {
        const mark =
          step.status === 'done'
            ? 'done'
            : step.status === 'blocked'
              ? 'no good'
              : step.status === 'doing'
                ? 'on it now'
                : 'to do';
        lines.push(`  [${mark}] ${step.what}${step.note ? ` - ${step.note}` : ''}`);
      }
      const current = this.current();
      if (current) {
        lines.push(`Work on this now: ${current.what}`);
      }
      blocks.push(lines.join('\n'));
    }
    const todo = describeTodo(this.state);
    if (todo) {
      blocks.push(todo);
    }
    const notes = describeNotes(this.state, now);
    if (notes) {
      blocks.push(notes);
    }
    const lately = this.state?.lately ?? [];
    if (lately.length > 0) {
      blocks.push(['Lately you have:', ...lately.slice(-6).map((line) => `  ${line}`)].join('\n'));
    }
    if (blocks.length === 0) {
      return '';
    }
    if (goal?.aim) {
      blocks.push(
        'This is still what you are doing. A conversation, a new room or somebody '
          + 'else\'s business does not replace it. Come back to it.'
      );
    }
    return blocks.join('\n\n');
  }

  /**
   * Record what just happened, and move the plan on if the model says the step
   * is finished with. Everything is written through to memory, so a restart
   * picks up mid-plan rather than at the beginning.
   */
  async record(what: string, outcome: string, progress: Progress): Promise<void> {
    if (!this.state) {
      return;
    }
    this.state = noteLately(this.state, `${what}: ${outcome}`);
    if (progress !== 'same') {
      const current = this.current();
      if (current) {
        this.state = {
          ...this.state,
          plan: this.steps.map((step) =>
            step === current
              ? { ...step, status: progress === 'done' ? 'done' : 'blocked', note: outcome }
              : step
          )
        };
      }
    } else {
      const current = this.current();
      if (current && current.status === 'next') {
        this.state = {
          ...this.state,
          plan: this.steps.map((step) =>
            step === current ? { ...step, status: 'doing' as StepState } : step
          )
        };
      }
    }
    await this.save();
  }

  /**
   * Make a plan when there is nothing left to do, or when the last one has
   * plainly stopped working. Doing nothing when the list is already good is the
   * common case, and costs nothing.
   */
  async refresh(context: string): Promise<boolean> {
    const goal = this.goal;
    if (!goal?.aim || !this.state || this.replanning) {
      return false;
    }
    if (!this.spent()) {
      return false;
    }
    this.replanning = true;
    try {
      const done = this.steps.filter((step) => step.status === 'done');
      const blocked = this.steps.filter((step) => step.status === 'blocked');
      const prompt = [
        context,
        '',
        `What you are after: ${goal.aim}`,
        goal.done ? `You would be there when: ${goal.done}` : '',
        done.length > 0 ? `Already done: ${done.map((step) => step.what).join('; ')}` : '',
        blocked.length > 0
          ? `Tried and got nowhere, so do not plan these again: ${blocked
              .map((step) => `${step.what} (${step.note || 'no good'})`)
              .join('; ')}`
          : '',
        '',
        `Write your next few steps. Up to ${MAX_STEPS}, in order, each one small`,
        'enough to actually do with what you can do here, and each one a step',
        'toward what you are after rather than a restatement of it.',
        '',
        'Reply with JSON and nothing else:',
        '{"steps": ["...", "..."]}'
      ]
        .filter((line) => line !== '')
        .join('\n');
      const response = await this.agent.generate(prompt, {
        memory: this.scope,
        modelSettings: REPLY_BUDGET
      });
      const text = String(response.text ?? '');
      const json = text.slice(text.indexOf('{'), text.lastIndexOf('}') + 1);
      const parsed = json ? StepsSchema.safeParse(safeJson(json)) : { success: false as const };
      if (!parsed.success) {
        // Keep the old plan rather than wiping it for an unreadable reply.
        return false;
      }
      this.state = {
        ...this.state,
        // Finished and failed steps are dropped: what they taught went into the
        // prompt above, and carrying them forever turns the plan into a diary.
        plan: parsed.data.steps.map((what) => ({ what, status: 'next' as StepState, note: '' })),
        // Stamped with the goal it was made for, so changing the goal - by the
        // character or on its sheet - retires it rather than leaving it to run
        // on toward something nobody wants.
        planFor: goal.aim
      };
      await this.save();
      return true;
    } catch {
      return false;
    } finally {
      this.replanning = false;
    }
  }

  /** Whether the plan has nothing useful left in it. */
  private spent(): boolean {
    const steps = this.steps;
    if (steps.length === 0) {
      return true;
    }
    if (!this.current()) {
      return true;
    }
    return steps.filter((step) => step.status === 'blocked').length >= BLOCKED_BEFORE_REPLAN;
  }

  /**
   * Write back only the fields this file owns, over whatever is in memory now.
   *
   * Working memory is one record and two things write to it: this, and the
   * harness noting people and places. Saving the whole record from a copy read
   * at startup silently undoes everything the other one wrote in between - a
   * character would discover a room, note it, and lose the note the moment it
   * ticked a step off its plan. Re-reading first is what keeps both.
   */
  private async save(): Promise<void> {
    if (!this.state) {
      return;
    }
    const memory = await this.memoryOf();
    const current = await readMemory(memory, this.scope);
    const merged: WorkingMemoryState = {
      ...current,
      goal: this.state.goal,
      goalSeed: this.state.goalSeed,
      plan: this.state.plan,
      planFor: this.state.planFor,
      todo: this.state.todo,
      notes: this.state.notes,
      lately: this.state.lately
    };
    this.state = merged;
    await writeMemory(memory, this.scope, merged);
  }
}

function safeJson(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}
