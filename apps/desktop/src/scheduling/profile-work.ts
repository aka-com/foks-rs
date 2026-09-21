/** Serialization shared by every caller using the same bridge and profile. */
export interface WorkTiming {
  priority: 'foreground' | 'background';
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
type Queue = {
  active?: Work;
  foreground: Work[];
  background: Work[];
  last?: string;
  draining: boolean;
};
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

/** Opt-in aggregate timings only: no profile names, request values or results. */
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
): Promise<T> {
  if (background && (background.signal.aborted || !background.current()))
    return Promise.reject(cancellation());
  let profiles = owners.get(owner);
  if (!profiles) owners.set(owner, (profiles = new Map<string, Queue>()));
  let queue = profiles.get(profile);
  if (!queue)
    profiles.set(
      profile,
      (queue = { foreground: [], background: [], draining: false }),
    );
  if (background) {
    const duplicate = queue.background.find(
      (w) =>
        w.background?.key === background.key &&
        w.background.owner === background.owner &&
        w.background.generation === background.generation &&
        w.background.signal === background.signal,
    );
    // Only the same request owner may share its result and cancellation lifetime.
    if (duplicate) return duplicate.promise as Promise<T>;
    if (queue.background.length >= 64)
      return Promise.reject(
        Object.assign(new Error('Background queue full.'), {
          code: 'profile-busy',
          retryable: true,
          fatal: false,
          ambiguous: false,
        }),
      );
  }
  if (queue.foreground.length + queue.background.length >= MAX_QUEUED)
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
      priority: background ? 'background' : 'foreground',
      queueMilliseconds: started - queued,
      executionMilliseconds:
        outcome === 'cancelled' && queue.active !== entry
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
      if (queue.active === entry) {
        try {
          background?.cancel();
        } catch {
          /* await actual settlement regardless */
        }
      } else {
        const index = queue.background.indexOf(entry);
        if (index >= 0) queue.background.splice(index, 1);
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
  (background ? queue.background : queue.foreground).push(entry);
  const admissionTimer = setTimeout(() => {
    if (settled || queue.active === entry) return;
    const waiting = background ? queue.background : queue.foreground;
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
    if (queue.draining) return;
    queue.draining = true;
    try {
      while (queue.foreground.length || queue.background.length) {
        let next = queue.foreground.shift();
        if (!next) {
          const index = queue.background.findIndex(
            (w) => w.background!.key !== queue.last,
          );
          next = queue.background.splice(Math.max(0, index), 1)[0];
          queue.last = next.background!.key;
        }
        queue.active = next;
        await next.run();
        queue.active = undefined;
      }
    } finally {
      queue.draining = false;
      if (profiles.get(profile) === queue) profiles.delete(profile);
      if (!profiles.size && owners.get(owner) === profiles)
        owners.delete(owner);
    }
  };
  queueMicrotask(() => {
    void drain();
  });
  return promise;
}
