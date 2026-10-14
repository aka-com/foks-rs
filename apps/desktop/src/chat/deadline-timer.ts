import type { ChatClock } from './clock';

/**
 * One timer armed for an absolute deadline on a `ChatClock`.
 *
 * `armAt` only moves the deadline earlier, so a burst of wake-up requests
 * cannot push work out indefinitely. `reset` replaces the deadline in either
 * direction; the owner calls it once it has recomputed what is due. An
 * infinite deadline arms nothing. The deadline is cleared before `fire`
 * runs, so `fire` may arm the timer again.
 */
export class DeadlineTimer {
  private handle: unknown;
  private due = Infinity;
  constructor(
    private clock: ChatClock,
    private fire: () => void,
  ) {}
  /** When the armed callback runs, or `Infinity` when none is armed. */
  get deadline(): number {
    return this.due;
  }
  armAt(due: number) {
    if (due < this.due) this.reset(due);
  }
  reset(due: number) {
    this.cancel();
    if (!(due < Infinity)) return;
    this.due = due;
    this.handle = this.clock.later(
      () => {
        this.handle = undefined;
        this.due = Infinity;
        this.fire();
      },
      Math.max(0, due - this.clock.now()),
    );
  }
  cancel() {
    if (this.due === Infinity) return;
    this.clock.cancel(this.handle);
    this.handle = undefined;
    this.due = Infinity;
  }
}
