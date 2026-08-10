/**
 * How a character gets anywhere over weeks rather than seconds.
 *
 * A model asked "what do you do next?" every twelve seconds, with nothing but
 * the room in front of it, does not pursue anything. It reacts. It will walk to
 * the east gate, forget why, and walk back. Long-run purpose needs three things
 * that a single prompt cannot supply:
 *
 *   1. A goal it cannot talk itself out of. That lives on the character sheet,
 *      next to the persona, and memory can never write to it. A character that
 *      can revise its own reason for existing has no reason for existing.
 *   2. A todo list it did not have to remember making. That lives in working
 *      memory, is written by this file rather than by the model's goodwill, and
 *      survives restarts.
 *   3. Knowing how the last few attempts went, so a step that cannot be done is
 *      abandoned rather than retried forever.
 *
 * The cycle is: plan, do one step, record what happened, and when the list runs
 * out or goes nowhere, plan again. The model chooses the steps and judges the
 * progress; the harness holds the list and the honesty.
 */

import { Agent } from '@mastra/core/agent';
import { z } from 'zod';
import { MemoryScope } from './behavior.js';
import {
  StepState,
  WorkingMemoryState,
  noteLately,
  readMemory,
  writeMemory
} from './memory.js';

/**
 * What a character is for. Set once on the character sheet, alongside the
 * persona, and never written by anything the character learns.
 */
export type Goal = {
  /** What they are trying to bring about, in a line. */
  aim: string;
  /** How they would know they had got there, if they ever can. */
  done?: string;
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
    private readonly goal: Goal | undefined,
    private readonly memoryOf: () => Promise<any>
  ) {}

  get hasGoal(): boolean {
    return Boolean(this.goal?.aim);
  }

  /**
   * Pick the plan back up, unless it was made for a different goal.
   *
   * Goals live on the character sheet and get edited between deploys; plans
   * live in memory and survive them. Reload one against the other and the
   * character carries on working through a list toward something nobody wants
   * any more, which is what Guy did: told to find out what was upstairs, he
   * went on asking the price of a field.
   */
  async load(): Promise<void> {
    const state = await readMemory(await this.memoryOf(), this.scope);
    const aim = this.goal?.aim ?? '';
    if (state.planFor === aim) {
      this.state = state;
      return;
    }
    this.state = { ...state, plan: [], planFor: aim };
    if (state.plan.length > 0) {
      // Say so. A plan quietly vanishing looks identical to a plan that was
      // never saved, and only one of those is a bug.
      console.log(`the goal changed; dropping ${state.plan.length} step(s) planned for the old one`);
    }
    await this.save();
  }

  get steps(): Step[] {
    return (this.state?.plan ?? []) as Step[];
  }

  /** The step being worked on: the first one not finished with. */
  current(): Step | null {
    return this.steps.find((step) => step.status === 'next' || step.status === 'doing') ?? null;
  }

  /** Everything the character knows about its own purpose, ready for a prompt. */
  describe(): string {
    if (!this.goal?.aim) {
      return '';
    }
    const lines = [`What you are after: ${this.goal.aim}`];
    if (this.goal.done) {
      lines.push(`You will have got there when: ${this.goal.done}`);
    }
    const steps = this.steps;
    if (steps.length > 0) {
      lines.push('', 'Your plan:');
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
        lines.push('', `Work on this now: ${current.what}`);
      }
    }
    const lately = this.state?.lately ?? [];
    if (lately.length > 0) {
      lines.push('', 'Lately you have:', ...lately.slice(-6).map((line) => `  ${line}`));
    }
    return lines.join('\n');
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
    if (!this.goal?.aim || !this.state || this.replanning) {
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
        `What you are after: ${this.goal.aim}`,
        this.goal.done ? `You would be there when: ${this.goal.done}` : '',
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
      const response = await this.agent.generate(prompt, { memory: this.scope });
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
        // Stamped with the goal it was made for, so editing the goal on the
        // character sheet retires it rather than leaving it to run on.
        planFor: this.goal.aim
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

  private async save(): Promise<void> {
    if (this.state) {
      await writeMemory(await this.memoryOf(), this.scope, this.state);
    }
  }
}

function safeJson(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}
