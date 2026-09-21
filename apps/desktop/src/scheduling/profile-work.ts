/** Serialization shared by every caller using the same bridge and profile. */
export interface WorkTiming {
  /** The queue's profile: a server id, as the Refresh status popover names it. */
  profile: string;
  /**
   * The background entry's key, or the label a foreground caller gave its
   * work. Foreground work has no identity of its own, so without a label the
   * queue cannot say what a later request was waiting behind.
   */
  key?: string;
  /** The lane that admitted the work: chat work reports its lane, not its rank. */
  priority: 'foreground' | 'background' | 'chat';
  queueMilliseconds: number;
  executionMilliseconds: number;
  outcome: 'success' | 'error' | 'cancelled';
  busy: boolean;
}
export interface BackgroundHistoryWork {
  key: string;
  owner: object;
  generation: number;
  signal: AbortSignal;
  current(): boolean;
  cancel(this: void): void;
  // Native cancellation currently has no agent-lock-release acknowledgment.
  preemptible: false;
}
type Observer = (event: WorkTiming) => void;
type Work = {
  background?: BackgroundHistoryWork;
  promise: Promise<unknown>;
  run(): Promise<void>;
  cancel(this: void): void;
};
type Lane = {
  active?: Work;
  foreground: Work[];
  background: Work[];
  last?: string;
  draining: boolean;
};
/**
 * Lanes are admitted independently of each other. Chat has its own because
 * the agent already admits a chat request against its store and admits one
 * inbox poll per account: waiting here behind a catalog walk or an
 * invitation read for the same profile is a second wait that buys nothing.
 * Work within a lane stays serialized.
 */
export type WorkLane = 'default' | 'chat';
type Queue = Record<WorkLane, Lane>;
const newLane = (): Lane => ({
  foreground: [],
  background: [],
  draining: false,
});
const idle = (entries: Lane): boolean =>
  !entries.draining && !entries.foreground.length && !entries.background.length;
/**
 * The agent answers a request within its own budget: the managed agent is
 * started with a sixty-second request timeout (`--request-timeout-seconds`
 * in `src-tauri/src/agent.rs`).
 */
const NATIVE_REQUEST_TIMEOUT = 60_000;
/**
 * How long queued work waits for admission. Work queued behind a request
 * that uses its whole budget has to outlast that budget plus the scheduling
 * and IPC slack around it; a wait equal to the budget expired such work at
 * the moment the queue freed.
 */
const MAX_QUEUE_WAIT = NATIVE_REQUEST_TIMEOUT + 15_000;
const MAX_QUEUED = 256;
const queueBusy = (message: string) =>
  Object.assign(new Error(message), {
    code: 'profile-busy',
    retryable: true,
    fatal: false,
    ambiguous: false,
  });
const owners = new WeakMap<object, Map<string, Queue>>();
const observers = new WeakMap<object, Set<Observer>>();
const cancellation = () =>
  Object.assign(new Error('Queued request cancelled.'), {
    code: 'cancelled',
    retryable: false,
    fatal: false,
    ambiguous: false,
  });

/** Opt-in timings: the profile and entry key, never request values or results. */
export function observeProfileWork(
  owner: object,
  observer: Observer,
): () => void {
  let listeners = observers.get(owner);
  if (!listeners) observers.set(owner, (listeners = new Set()));
  listeners.add(observer);
  return () => {
    listeners.delete(observer);
    if (!listeners.size) observers.delete(owner);
  };
}
export function scheduleProfileWork<T>(
  owner: object,
  profile: string,
  work: () => Promise<T>,
  background?: BackgroundHistoryWork,
  /** An operation name for the timings; never an argument or a result. */
  label?: string,
  lane: WorkLane = 'default',
): Promise<T> {
  if (background && (background.signal.aborted || !background.current()))
    return Promise.reject(cancellation());
  let profiles = owners.get(owner);
  if (!profiles) owners.set(owner, (profiles = new Map<string, Queue>()));
  let queue = profiles.get(profile);
  if (!queue)
    profiles.set(profile, (queue = { default: newLane(), chat: newLane() }));
  const track = queue[lane];
  if (background) {
    const duplicate = track.background.find(
      (w) =>
        w.background?.key === background.key &&
        w.background.owner === background.owner &&
        w.background.generation === background.generation &&
        w.background.signal === background.signal,
    );
    // Only the same request owner may share its result and cancellation lifetime.
    if (duplicate) return duplicate.promise as Promise<T>;
    if (track.background.length >= 64)
      return Promise.reject(
        Object.assign(new Error('Background queue full.'), {
          code: 'profile-busy',
          retryable: true,
          fatal: false,
          ambiguous: false,
        }),
      );
  }
  if (track.foreground.length + track.background.length >= MAX_QUEUED)
    return Promise.reject(
      queueBusy('Profile queue full; request did not start.'),
    );
  const queued = performance.now();
  let resolve!: (value: T | PromiseLike<T>) => void,
    reject!: (cause: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  let settled = false,
    cancelled = false;
  const report = (
    started: number,
    outcome: WorkTiming['outcome'],
    busy = false,
  ) => {
    const event: WorkTiming = {
      profile,
      ...(background
        ? { key: background.key }
        : label !== undefined
          ? { key: label }
          : {}),
      priority:
        lane === 'chat' ? 'chat' : background ? 'background' : 'foreground',
      queueMilliseconds: started - queued,
      executionMilliseconds:
        outcome === 'cancelled' && track.active !== entry
          ? 0
          : performance.now() - started,
      outcome,
      busy,
    };
    for (const observer of observers.get(owner) ?? []) {
      try {
        observer(event);
      } catch {
        /* diagnostics cannot change admission */
      }
    }
  };
  const finish = () => {
    settled = true;
    clearTimeout(admissionTimer);
    background?.signal.removeEventListener('abort', entry.cancel);
  };
  const entry: Work = {
    background,
    promise,
    cancel() {
      if (settled || cancelled) return;
      cancelled = true;
      if (track.active === entry) {
        try {
          background?.cancel();
        } catch {
          /* await actual settlement regardless */
        }
      } else {
        const index = track.background.indexOf(entry);
        if (index >= 0) track.background.splice(index, 1);
        finish();
        reject(cancellation());
        report(performance.now(), 'cancelled');
      }
    },
    async run() {
      clearTimeout(admissionTimer);
      const started = performance.now();
      let outcome: WorkTiming['outcome'] = 'success',
        busy = false;
      try {
        if (
          cancelled ||
          (background && (background.signal.aborted || !background.current()))
        )
          throw cancellation();
        const value = await work();
        if (
          cancelled ||
          (background && (background.signal.aborted || !background.current()))
        )
          throw cancellation();
        resolve(value);
      } catch (cause) {
        const code = (cause as { code?: string } | null)?.code;
        outcome = code === 'cancelled' ? 'cancelled' : 'error';
        busy = code === 'profile-busy';
        reject(cause);
      } finally {
        finish();
        report(started, outcome, busy);
      }
    },
  };
  (background ? track.background : track.foreground).push(entry);
  const admissionTimer = setTimeout(() => {
    if (settled || track.active === entry) return;
    const waiting = background ? track.background : track.foreground;
    const index = waiting.indexOf(entry);
    if (index >= 0) waiting.splice(index, 1);
    finish();
    reject(
      queueBusy('Profile queue deadline exceeded; request did not start.'),
    );
    report(performance.now(), 'error', true);
  }, MAX_QUEUE_WAIT);
  background?.signal.addEventListener('abort', entry.cancel, { once: true });
  const drain = async () => {
    if (track.draining) return;
    track.draining = true;
    try {
      while (track.foreground.length || track.background.length) {
        let next = track.foreground.shift();
        if (!next) {
          const index = track.background.findIndex(
            (w) => w.background!.key !== track.last,
          );
          next = track.background.splice(Math.max(0, index), 1)[0];
          track.last = next.background!.key;
        }
        track.active = next;
        await next.run();
        track.active = undefined;
      }
    } finally {
      track.draining = false;
      // The queue is shared by both lanes; the other may still be draining.
      if (
        profiles.get(profile) === queue &&
        idle(queue.default) &&
        idle(queue.chat)
      )
        profiles.delete(profile);
      if (!profiles.size && owners.get(owner) === profiles)
        owners.delete(owner);
    }
  };
  queueMicrotask(() => {
    void drain();
  });
  return promise;
}
