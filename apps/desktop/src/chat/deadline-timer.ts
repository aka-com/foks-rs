import type { ChatClock } from './clock';

/**
 * The longest delay `setTimeout` accepts, 2^31 - 1 ms (about 24.8 days). A
 * longer delay overflows and the callback runs almost at once.
 */
const MAXIMUM_DELAY_MS = 2_147_483_647;

/**
 * One timer armed for an absolute deadline on a `ChatClock`.
 *
 * `armAt` only moves the deadline earlier, so a burst of wake-up requests
 * cannot push work out indefinitely. `reset` replaces the deadline in either
 * direction; the owner calls it once it has recomputed what is due. An
 * infinite deadline arms nothing. The deadline is cleared before `fire`
 * runs, so `fire` may arm the timer again. A deadline further away than the
 * longest host timer delay is reached in steps of that delay, and `fire` runs
 * only at the deadline itself.
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
    const wait = Math.max(0, due - this.clock.now());
    this.handle = this.clock.later(
      () => {
        this.handle = undefined;
        this.due = Infinity;
        if (wait > MAXIMUM_DELAY_MS) this.reset(due);
        else this.fire();
      },
      Math.min(wait, MAXIMUM_DELAY_MS),
    );
  }
  cancel() {
    if (this.due === Infinity) return;
    this.clock.cancel(this.handle);
    this.handle = undefined;
    this.due = Infinity;
  }
}
