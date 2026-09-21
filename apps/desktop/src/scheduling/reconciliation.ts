export interface ReconciliationClock {
  now(): number;
  later(callback: () => void, milliseconds: number): unknown;
  cancel(timer: unknown): void;
  random(): number;
}

export const reconciliationClock: ReconciliationClock = {
  now: () => performance.timeOrigin + performance.now(),
  later: (callback, milliseconds) => setTimeout(callback, milliseconds),
  cancel: (timer) => clearTimeout(timer as ReturnType<typeof setTimeout>),
  random: Math.random,
};

export type ReconciliationTrigger =
  'periodic' | 'foreground' | 'network' | 'mutation' | 'recovery' | 'manual';
export type ReconciliationKind =
  'catalog' | 'discovery' | 'metadata' | 'registry' | 'connectivity';
export interface ReconciliationContext {
  signal: AbortSignal;
  trigger: ReconciliationTrigger;
  isCurrent(this: void): boolean;
}
export interface ReconciliationJob {
  key: string;
  scope: string | null;
  kind: ReconciliationKind;
  interval: number;
  initialDelay?: number;
  eligible?(): boolean;
  run(context: ReconciliationContext): Promise<void>;
}
export interface ReconciliationSnapshot {
  refreshing: boolean;
  lastAttemptAt?: number;
  lastSuccessAt?: number;
  error?: unknown;
  /**
   * Whether the scheduler has disabled automatic retries for this job.
   * The UI uses this value instead of inferring retry behavior from the error.
   */
  paused?: boolean;
  /**
   * How long the last run that reached an outcome of its own took. Kept on
   * the snapshot rather than read back out of the timing log, so the job's
   * row still states a duration once that log has been cleared.
   */
  lastMilliseconds?: number;
  /**
   * When the job will next run on its own, while it is idle and scheduled.
   * Read off the entry as the snapshot is taken: the due time moves with
   * every request, retry and completion, so it is never stored in one.
   */
  nextAttemptAt?: number;
}
/**
 * One run of one job, reported when it starts and when it settles. Carries
 * the job's key and scope (a server id, or null for this Mac) so a log can
 * name the job the way the Refresh status popover does; never the error.
 */
export interface ReconciliationDiagnostic {
  key: string;
  scope: string | null;
  kind: ReconciliationKind;
  trigger: ReconciliationTrigger;
  outcome: 'started' | 'success' | 'failed' | 'retired';
  milliseconds: number;
  retry: number;
  /** How long after its due time the run started. */
  late: number;
}
type Waiter = { resolve(): void; reject(error: unknown): void };
type Entry = {
  job: ReconciliationJob;
  due: number;
  retry: number;
  trigger: ReconciliationTrigger;
  trailing: boolean;
  active?: AbortController;
  snapshot: ReconciliationSnapshot;
  /** Callers of `run` waiting for the run that answers their request. */
  waiters: Waiter[];
};
/** What a caller of `run` is told when no run answered its request. */
export type RetiredRun = Error & {
  code: 'catalog-read-retired';
  retryable: false;
  fatal: false;
  ambiguous: false;
};
const retired = (): RetiredRun =>
  Object.assign(new Error('The reconciliation job was retired.'), {
    code: 'catalog-read-retired' as const,
    retryable: false as const,
    fatal: false as const,
    ambiguous: false as const,
  });
type Active = { scope: string | null; controller: AbortController };

export class ReconciliationScheduler {
  private entries = new Map<string, Entry>();
  private active = new Set<Active>();
  private enabled = false;
  private visible = true;
  private timer: unknown;
  private disposed = false;
  private observers = new Set<
    (event: ReconciliationDiagnostic) => void | Promise<void>
  >();
  private listeners = new Set<() => void>();
  private revision = 0;

  constructor(
    private clock: ReconciliationClock = reconciliationClock,
    private capacity = 2,
  ) {}

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  getSnapshot = (): number => this.revision;
  snapshot(key: string): Readonly<ReconciliationSnapshot> | undefined {
    const entry = this.entries.get(key);
    return entry && this.view(entry);
  }
  private view(entry: Entry): Readonly<ReconciliationSnapshot> {
    return entry.active || !Number.isFinite(entry.due)
      ? entry.snapshot
      : Object.freeze({ ...entry.snapshot, nextAttemptAt: entry.due });
  }
  observations(): readonly {
    key: string;
    scope: string | null;
    kind: ReconciliationKind;
    snapshot: Readonly<ReconciliationSnapshot>;
  }[] {
    return [...this.entries.values()].map((entry) => ({
      key: entry.job.key,
      scope: entry.job.scope,
      kind: entry.job.kind,
      snapshot: this.view(entry),
    }));
  }
  observe(
    observer: (event: ReconciliationDiagnostic) => void | Promise<void>,
  ): () => void {
    this.observers.add(observer);
    return () => {
      this.observers.delete(observer);
    };
  }
  update(jobs: readonly ReconciliationJob[]): void {
    if (this.disposed) return;
    const wanted = new Set(jobs.map((job) => job.key));
    for (const [key, entry] of this.entries) {
      if (!wanted.has(key)) {
        entry.active?.abort();
        this.settle(entry, retired());
        this.entries.delete(key);
      }
    }
    for (const job of jobs) {
      const existing = this.entries.get(job.key);
      if (existing && existing.job.scope === job.scope) existing.job = job;
      else {
        existing?.active?.abort();
        if (existing) this.settle(existing, retired());
        this.entries.set(job.key, {
          job,
          due: this.clock.now() + Math.max(0, job.initialDelay ?? job.interval),
          retry: 0,
          trigger: 'periodic',
          trailing: false,
          snapshot: Object.freeze({ refreshing: false }),
          waiters: [],
        });
      }
    }
    this.schedule();
  }
  setEnabled(enabled: boolean): void {
    if (this.disposed || enabled === this.enabled) return;
    this.enabled = enabled;
    if (!enabled) {
      for (const active of this.active) active.controller.abort();
      this.retireIdleWaiters();
    }
    this.schedule();
  }
  setVisible(visible: boolean): void {
    this.visible = visible;
    if (!visible) this.retireIdleWaiters();
    this.schedule();
  }
  /**
   * Reject waiters for idle jobs when the scheduler stops. Waiters attached to
   * an active job are resolved or rejected when that job completes.
   */
  private retireIdleWaiters(): void {
    for (const entry of this.entries.values())
      if (!entry.active) this.settle(entry, retired());
  }
  private settle(entry: Entry, error?: unknown): void {
    if (!entry.waiters.length) return;
    const waiters = entry.waiters;
    entry.waiters = [];
    for (const waiter of waiters)
      if (error === undefined) waiter.resolve();
      else waiter.reject(error);
  }
  /**
   * Requests a job and returns a promise for the first run started after that
   * request. An already active run does not satisfy the promise because it may
   * not include the requested invalidation. Returns `null` when the scheduler
   * cannot run the job.
   */
  run(key: string, trigger: ReconciliationTrigger): Promise<void> | null {
    const entry = this.entries.get(key);
    if (
      !entry ||
      this.disposed ||
      !this.enabled ||
      !this.visible ||
      entry.job.eligible?.() === false ||
      this.declines(entry, trigger)
    )
      return null;
    return new Promise<void>((resolve, reject) => {
      entry.waiters.push({ resolve, reject });
      this.request(key, trigger, true);
    });
  }
  /**
   * Returns whether `request` would ignore this trigger because the job is
   * paused or an idle retry remains in backoff.
   */
  private declines(entry: Entry, trigger: ReconciliationTrigger): boolean {
    if (
      !Number.isFinite(entry.due) &&
      trigger !== 'manual' &&
      trigger !== 'recovery'
    )
      return true;
    return (
      !entry.active &&
      entry.retry > 0 &&
      (trigger === 'foreground' || trigger === 'periodic') &&
      this.clock.now() < entry.due
    );
  }
  request(
    key: string,
    trigger: ReconciliationTrigger,
    invalidate = false,
  ): void {
    const entry = this.entries.get(key);
    if (!entry || this.disposed || this.declines(entry, trigger)) return;
    if (entry.active) {
      if (invalidate) {
        entry.trailing = true;
        entry.trigger = trigger;
      }
      return;
    }
    entry.trigger = trigger;
    entry.due = Math.min(
      entry.due,
      Math.max(
        this.clock.now(),
        (entry.snapshot.lastAttemptAt ?? -Infinity) + 1_000,
      ),
    );
    this.schedule();
  }
  /**
   * Requests every job, or every job of the given kinds; a job `unless`
   * answers true for is left on its own schedule.
   */
  requestAll(
    trigger: ReconciliationTrigger,
    kinds?: readonly ReconciliationKind[],
    unless?: (entry: {
      key: string;
      scope: string | null;
      snapshot: Readonly<ReconciliationSnapshot>;
    }) => boolean,
  ): void {
    for (const [key, entry] of this.entries)
      if (
        (!kinds || kinds.includes(entry.job.kind)) &&
        !unless?.({ key, scope: entry.job.scope, snapshot: this.view(entry) })
      )
        this.request(key, trigger);
  }
  reconciled(key: string, milliseconds?: number): void {
    const entry = this.entries.get(key);
    if (!entry) return;
    entry.retry = 0;
    entry.due = this.clock.now() + Math.max(1_000, entry.job.interval);
    this.publish(entry, {
      ...entry.snapshot,
      error: undefined,
      refreshing: Boolean(entry.active),
      lastSuccessAt: this.clock.now(),
      lastMilliseconds: milliseconds,
      paused: false,
    });
    this.schedule();
  }
  dispose(): void {
    this.setEnabled(false);
    for (const entry of this.entries.values()) this.settle(entry, retired());
    this.disposed = true;
    this.clock.cancel(this.timer);
    this.entries.clear();
    this.listeners.clear();
    this.observers.clear();
  }
  private publish(entry: Entry, snapshot: ReconciliationSnapshot): void {
    entry.snapshot = Object.freeze(snapshot);
    this.revision++;
    for (const listener of this.listeners) listener();
  }
  private report(event: ReconciliationDiagnostic): void {
    for (const observer of this.observers) {
      try {
        void Promise.resolve(observer(Object.freeze(event))).catch(
          () => undefined,
        );
      } catch {
        continue;
      }
    }
  }
  private available(entry: Entry): boolean {
    return (
      !entry.active &&
      entry.job.eligible?.() !== false &&
      ![...this.active].some(
        (active) =>
          active.scope === null ||
          entry.job.scope === null ||
          active.scope === entry.job.scope,
      )
    );
  }
  private schedule(): void {
    this.clock.cancel(this.timer);
    if (
      this.disposed ||
      !this.enabled ||
      !this.visible ||
      this.active.size >= this.capacity
    )
      return;
    const eligible = [...this.entries.values()].filter((entry) =>
      this.available(entry),
    );
    // Reject waiters when their job becomes ineligible and cannot be scheduled.
    for (const entry of this.entries.values())
      if (
        entry.waiters.length &&
        !entry.active &&
        entry.job.eligible?.() === false
      )
        this.settle(entry, retired());
    if (!eligible.length) return;
    const due = Math.min(...eligible.map((entry) => entry.due));
    if (!Number.isFinite(due)) return;
    this.timer = this.clock.later(
      () => this.drain(),
      Math.max(0, due - this.clock.now()),
    );
  }
  private drain(): void {
    if (this.disposed || !this.enabled || !this.visible) return;
    for (const [key, entry] of this.entries) {
      if (this.active.size >= this.capacity) break;
      if (entry.due > this.clock.now() || !this.available(entry)) continue;
      this.entries.delete(key);
      this.entries.set(key, entry);
      void this.execute(entry);
      if (entry.job.scope === null) break;
    }
    this.schedule();
  }
  private async execute(entry: Entry): Promise<void> {
    const controller = new AbortController();
    const active = { scope: entry.job.scope, controller };
    entry.active = controller;
    this.active.add(active);
    const job = entry.job;
    const trigger = entry.trigger;
    const started = this.clock.now();
    const late = Number.isFinite(entry.due)
      ? Math.max(0, started - entry.due)
      : 0;
    const isCurrent = () =>
      !controller.signal.aborted &&
      this.enabled &&
      !this.disposed &&
      this.entries.get(job.key) === entry;
    this.publish(entry, {
      ...entry.snapshot,
      refreshing: true,
      lastAttemptAt: started,
      paused: false,
    });
    this.report({
      key: job.key,
      scope: job.scope,
      kind: job.kind,
      trigger,
      outcome: 'started',
      milliseconds: 0,
      retry: entry.retry,
      late,
    });
    let outcome: ReconciliationDiagnostic['outcome'] = 'retired';
    let failure: unknown;
    try {
      await job.run({ signal: controller.signal, trigger, isCurrent });
      if (isCurrent()) {
        outcome = 'success';
        entry.retry = 0;
        entry.due = this.clock.now() + Math.max(1_000, job.interval);
        this.publish(entry, {
          refreshing: false,
          lastAttemptAt: started,
          lastSuccessAt: this.clock.now(),
          lastMilliseconds: Math.max(0, this.clock.now() - started),
        });
      }
    } catch (error) {
      if (isCurrent()) {
        const typed = error as {
          code?: string;
          fatal?: boolean;
          retryable?: boolean;
        } | null;
        if (
          [
            'cancelled',
            'catalog-read-retired',
            'agent-request-retired',
          ].includes(typed?.code ?? '')
        )
          return;
        outcome = 'failed';
        failure = error;
        entry.retry++;
        const delay =
          typed?.retryable === false
            ? Math.max(60_000, job.interval)
            : Math.min(60_000, 1_000 * 2 ** Math.min(6, entry.retry - 1));
        entry.due =
          typed?.fatal && typed.code !== 'agent-lost' && job.kind !== 'metadata'
            ? Infinity
            : this.clock.now() + delay * (1 + this.clock.random() / 4);
        this.publish(entry, {
          ...entry.snapshot,
          refreshing: false,
          error,
          lastMilliseconds: Math.max(0, this.clock.now() - started),
          paused: entry.due === Infinity,
        });
      }
    } finally {
      this.active.delete(active);
      if (entry.active === controller) entry.active = undefined;
      if (this.entries.get(job.key) !== entry) this.settle(entry, retired());
      else {
        if (outcome === 'retired') {
          entry.due = this.clock.now() + 1_000;
          this.publish(entry, { ...entry.snapshot, refreshing: false });
        }
        if (entry.trailing) {
          entry.trailing = false;
          entry.due = this.clock.now();
          // Waiters require the follow-up run because the active run began before
          // their requests. If the scheduler stopped during the active run, reject
          // those waiters and leave the follow-up run due for the next start.
          if (this.disposed || !this.enabled || !this.visible)
            this.settle(entry, retired());
        } else {
          entry.trigger = 'periodic';
          if (outcome === 'success') this.settle(entry);
          else this.settle(entry, outcome === 'failed' ? failure : retired());
        }
      }
      this.report({
        key: job.key,
        scope: job.scope,
        kind: job.kind,
        trigger,
        outcome,
        milliseconds: Math.max(0, this.clock.now() - started),
        retry: entry.retry,
        late,
      });
      this.schedule();
    }
  }
}
