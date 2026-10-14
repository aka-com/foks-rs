/**
 * How long a failed chat request waits before it is tried again.
 *
 * `base` is the delay after the first consecutive failure; each further
 * failure doubles it, up to `cap`. `jitter` adds up to that fraction of the
 * delay, drawn from the caller's random source, so that teams which failed
 * together do not retry together. Jitter never lifts a delay past `cap`.
 */
export interface RetryPolicy {
  base: number;
  cap: number;
  jitter?: number;
}

/**
 * A send-service check or a notification history read: 2 s, 4 s, 8 s, 16 s,
 * then 30 s. The first delay matches the 2 s a successful check waits before
 * it repeats, so a failing request is never asked more often than a working
 * one.
 */
export const REQUEST_RETRY: RetryPolicy = { base: 2_000, cap: 30_000 };

/**
 * An inbox synchronization or account poll: 250 ms doubling to 30 s, plus up
 * to a quarter of the delay. A synchronization is what makes a team's
 * channel list usable, so its first retry is short.
 */
export const SYNC_RETRY: RetryPolicy = {
  base: 250,
  cap: 30_000,
  jitter: 0.25,
};

/**
 * The delay before the next attempt after `failures` consecutive failures,
 * at least one. `random` returns a value in [0, 1) and is read only when the
 * policy has jitter.
 */
export function retryDelay(
  failures: number,
  policy: RetryPolicy,
  random: () => number = Math.random,
): number {
  const delay = Math.min(
    policy.base * 2 ** Math.max(0, failures - 1),
    policy.cap,
  );
  if (!policy.jitter) return delay;
  return Math.min(
    policy.cap,
    delay + Math.floor(random() * delay * policy.jitter),
  );
}

/**
 * The base wait between fallback reads of a channel whose projection cannot
 * report that it moved, and the ceiling that wait doubles toward.
 *
 * A degraded host, or a channel whose baseline is not established yet,
 * cannot say which thread moved, so the only way to notice an arrival is to
 * read the channel again. That is the whole cost of a degraded host: twenty
 * channels at the base interval is four full history reads a second,
 * sustained. A channel whose read finds nothing therefore doubles its wait,
 * and a read that finds it moved puts it straight back to the base.
 *
 * **The presented channel never backs off.** The channel whose thread is
 * open is due at the base interval from its last read, whatever interval is
 * stored. The exemption is applied when the due time is read, not when the
 * interval is stored, so presenting a channel that had backed off makes it
 * due at once if the base interval has passed, and closing it again resumes
 * the interval its quiet reads built up.
 *
 * **This trades alert latency on a degraded host, and it is a product
 * decision.** A message in a quiet channel that is not open is noticed up to
 * `FALLBACK_MAX_MS` after it arrives rather than up to `FALLBACK_BASE_MS`.
 * No message is skipped: the wait decides when the read happens, not
 * whether, and the read that finds the message resets the channel to the
 * base interval.
 */
export const FALLBACK_BASE_MS = 5_000;
export const FALLBACK_MAX_MS = 60_000;

/**
 * One channel's fallback schedule: when it was last read on no evidence,
 * and how long the next such read waits. Immutable; each read replaces it.
 */
export class GeometricInterval {
  private constructor(
    readonly at: number,
    readonly interval: number,
  ) {}
  /** A schedule at the base interval, last read at `at`. */
  static base(at: number): GeometricInterval {
    return new GeometricInterval(at, FALLBACK_BASE_MS);
  }
  /**
   * The schedule after a read at `now`: back to the base interval when the
   * read found the channel `moved`, doubled toward the ceiling otherwise.
   */
  next(moved: boolean, now: number): GeometricInterval {
    return new GeometricInterval(
      now,
      moved ? FALLBACK_BASE_MS : Math.min(this.interval * 2, FALLBACK_MAX_MS),
    );
  }
  /** When the next read is due; a `presented` channel waits the base only. */
  due(presented: boolean): number {
    return this.at + (presented ? FALLBACK_BASE_MS : this.interval);
  }
}
