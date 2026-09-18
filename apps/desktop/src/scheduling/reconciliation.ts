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
  'catalog' | 'discovery' | 'metadata' | 'registry';
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
}
export interface ReconciliationDiagnostic {
  kind: ReconciliationKind;
  trigger: ReconciliationTrigger;
  outcome: 'started' | 'success' | 'failed' | 'retired';
  milliseconds: number;
  retry: number;
}
type Entry = {
  job: ReconciliationJob;
  due: number;
  retry: number;
  trigger: ReconciliationTrigger;
  trailing: boolean;
  active?: AbortController;
  snapshot: ReconciliationSnapshot;
};
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
    return this.entries.get(key)?.snapshot;
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
      snapshot: entry.snapshot,
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
        this.entries.delete(key);
      }
    }
    for (const job of jobs) {
      const existing = this.entries.get(job.key);
      if (existing && existing.job.scope === job.scope) existing.job = job;
      else {
        existing?.active?.abort();
        this.entries.set(job.key, {
          job,
          due: this.clock.now() + Math.max(0, job.initialDelay ?? job.interval),
          retry: 0,
          trigger: 'periodic',
          trailing: false,
          snapshot: Object.freeze({ refreshing: false }),
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
    }
    this.schedule();
  }
  setVisible(visible: boolean): void {
    this.visible = visible;
    this.schedule();
  }
  request(
    key: string,
    trigger: ReconciliationTrigger,
    invalidate = false,
  ): void {
    const entry = this.entries.get(key);
    if (!entry || this.disposed) return;
    if (
      !Number.isFinite(entry.due) &&
      trigger !== 'manual' &&
      trigger !== 'recovery'
    )
      return;
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
  requestAll(
    trigger: ReconciliationTrigger,
    kinds?: readonly ReconciliationKind[],
  ): void {
    for (const [key, entry] of this.entries)
      if (!kinds || kinds.includes(entry.job.kind)) this.request(key, trigger);
  }
  reconciled(key: string): void {
    const entry = this.entries.get(key);
    if (!entry) return;
    entry.retry = 0;
    entry.due = this.clock.now() + Math.max(1_000, entry.job.interval);
    this.publish(entry, {
      ...entry.snapshot,
      error: undefined,
      refreshing: Boolean(entry.active),
      lastSuccessAt: this.clock.now(),
    });
    this.schedule();
  }
  dispose(): void {
    this.setEnabled(false);
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
      void this.run(entry);
      if (entry.job.scope === null) break;
    }
    this.schedule();
  }
  private async run(entry: Entry): Promise<void> {
    const controller = new AbortController();
    const active = { scope: entry.job.scope, controller };
    entry.active = controller;
    this.active.add(active);
    const job = entry.job;
    const trigger = entry.trigger;
    const started = this.clock.now();
    const isCurrent = () =>
      !controller.signal.aborted &&
      this.enabled &&
      !this.disposed &&
      this.entries.get(job.key) === entry;
    this.publish(entry, {
      ...entry.snapshot,
      refreshing: true,
      lastAttemptAt: started,
    });
    this.report({
      kind: job.kind,
      trigger,
      outcome: 'started',
      milliseconds: 0,
      retry: entry.retry,
    });
    let outcome: ReconciliationDiagnostic['outcome'] = 'retired';
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
        entry.retry++;
        const delay =
          typed?.retryable === false
            ? Math.max(60_000, job.interval)
            : Math.min(60_000, 1_000 * 2 ** Math.min(6, entry.retry - 1));
        entry.due =
          typed?.fatal && typed.code !== 'agent-lost' && job.kind !== 'metadata'
            ? Infinity
            : this.clock.now() + delay * (1 + this.clock.random() / 4);
        this.publish(entry, { ...entry.snapshot, refreshing: false, error });
      }
    } finally {
      this.active.delete(active);
      if (entry.active === controller) entry.active = undefined;
      if (this.entries.get(job.key) === entry) {
        if (outcome === 'retired') {
          entry.due = this.clock.now() + 1_000;
          this.publish(entry, { ...entry.snapshot, refreshing: false });
        }
        if (entry.trailing) {
          entry.trailing = false;
          entry.due = this.clock.now();
        } else entry.trigger = 'periodic';
      }
      this.report({
        kind: job.kind,
        trigger,
        outcome,
        milliseconds: Math.max(0, this.clock.now() - started),
        retry: entry.retry,
      });
      this.schedule();
    }
  }
}
