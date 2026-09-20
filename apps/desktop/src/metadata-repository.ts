/** Session-scoped public metadata. Never store item contents, credentials or reveal phrases here. */
export type QueryKey = readonly string[];
export interface QueryDiagnostic {
  readonly kind:
    | 'cache-hit'
    | 'coalesced'
    | 'load-start'
    | 'load-complete'
    | 'load-error'
    | 'discarded'
    | 'repair-attempt';
  readonly milliseconds?: number;
}
type QueryObserver = (event: QueryDiagnostic) => void | Promise<void>;

export interface QuerySnapshot<T> {
  readonly data: T | undefined;
  readonly error: unknown;
  readonly fetching: boolean;
  readonly lastSuccessAt?: number;
  readonly lastAttemptAt?: number;
  /** Changes only when data is invalidated, so subscribers can request one reload. */
  readonly invalidation: number;
}

export interface QueryLoadContext {
  /** False after invalidation, access reset or retirement. */
  isCurrent(): boolean;
  repairAttempt(): void;
}

export interface QueryLoadOptions {
  /** Eligible read recovery only; commands and ambiguous writes never use this API. */
  recover?: (error: unknown, context: QueryLoadContext) => Promise<boolean>;
}

export class RetiredQueryError extends Error {
  constructor() {
    super(
      'The metadata request belongs to an earlier access or resource generation.',
    );
    this.name = 'RetiredQueryError';
  }
}

/** Copy at publication: callers cannot change other subscribers' accepted metadata. */
function immutableMetadata<T>(value: T): T {
  if (value === null || typeof value !== 'object') return value;
  if (Array.isArray(value))
    return Object.freeze(value.map(immutableMetadata)) as T;
  if (Object.getPrototypeOf(value) !== Object.prototype) {
    throw new TypeError(
      'Query results must contain only plain metadata records.',
    );
  }
  return Object.freeze(
    Object.fromEntries(
      Object.entries(value).map(([key, child]) => [
        key,
        immutableMetadata(child),
      ]),
    ),
  ) as T;
}

export class MetadataQuery<T> {
  private state: QuerySnapshot<T> = Object.freeze({
    data: undefined,
    error: undefined,
    fetching: false,
    invalidation: 0,
  });
  private listeners = new Set<() => void>();
  private generation = 0;
  private updated: number | undefined;
  private pending?: Promise<T>;
  private pendingGeneration?: number;
  private trailing?: { epoch: number; promise: Promise<T> };
  private failures = 0;
  private retryAt = 0;
  private reportedError: unknown;

  constructor(
    readonly key: QueryKey,
    private readonly repository: MetadataRepository,
    private readonly read: () => Promise<T>,
    private readonly freshFor: number,
  ) {}

  readonly getSnapshot = (): QuerySnapshot<T> => this.state;
  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  /** Report a shared failed request once, even when several views observe it. */
  claimError(error: unknown): boolean {
    if (error !== this.state.error || error === this.reportedError)
      return false;
    this.reportedError = error;
    return true;
  }

  private publish(state: QuerySnapshot<T>): void {
    this.state = Object.freeze(state);
    for (const listener of this.listeners) listener();
  }

  invalidate(drop = false): void {
    this.generation++;
    this.updated = undefined;
    this.failures = 0;
    this.retryAt = 0;
    this.reportedError = undefined;
    this.publish({
      data: drop ? undefined : this.state.data,
      error: undefined,
      fetching: this.pending !== undefined,
      lastSuccessAt: drop ? undefined : this.state.lastSuccessAt,
      lastAttemptAt: drop ? undefined : this.state.lastAttemptAt,
      invalidation: this.state.invalidation + 1,
    });
  }

  isDueSubscribed(): boolean {
    const now = this.repository.now();
    return (
      !this.repository.retired &&
      this.listeners.size > 0 &&
      !this.pending &&
      !this.trailing &&
      (now >= this.retryAt || now < (this.state.lastAttemptAt ?? now)) &&
      (this.updated === undefined ||
        now < this.updated ||
        now - this.updated >= this.freshFor)
    );
  }

  load(options: QueryLoadOptions = {}): Promise<T> {
    if (this.repository.retired) return Promise.reject(new RetiredQueryError());
    if (this.pending) {
      this.repository.report({ kind: 'coalesced' });
      if (this.pendingGeneration === this.generation) return this.pending;
      const epoch = this.repository.epoch;
      if (this.trailing?.epoch === epoch) return this.trailing.promise;
      const trailing: { epoch: number; promise: Promise<T> } = {
        epoch,
        promise: this.pending
          .catch(() => undefined)
          .then(() => {
            if (this.trailing === trailing) this.trailing = undefined;
            if (this.repository.retired || epoch !== this.repository.epoch)
              throw new RetiredQueryError();
            return this.load(options);
          }),
      };
      this.trailing = trailing;
      return trailing.promise;
    }
    if (
      this.updated !== undefined &&
      this.repository.now() >= this.updated &&
      this.repository.now() - this.updated < this.freshFor
    ) {
      this.repository.report({ kind: 'cache-hit' });
      return Promise.resolve(this.state.data as T);
    }
    this.reportedError = undefined;
    const recover = options.recover ?? this.repository.readOptions().recover;
    const generation = this.generation;
    const epoch = this.repository.epoch;
    const started = this.repository.now();
    let discarded = false;
    const context = {
      repairAttempt: () => this.repository.report({ kind: 'repair-attempt' }),
      isCurrent: () =>
        !this.repository.retired &&
        epoch === this.repository.epoch &&
        generation === this.generation,
    };
    const requireCurrent = () => {
      if (!context.isCurrent()) {
        if (!discarded) {
          discarded = true;
          this.repository.report({
            kind: 'discarded',
            milliseconds: Math.max(0, this.repository.now() - started),
          });
        }
        throw new RetiredQueryError();
      }
    };
    // Schedule the read after pending is assigned, including loaders that throw
    // synchronously. Reentrant subscribers still see one in-flight request.
    const pending = Promise.resolve()
      .then(async () => {
        requireCurrent();
        try {
          return await this.read();
        } catch (error) {
          requireCurrent();
          if (
            !recover ||
            !this.repository.canRecoverRead() ||
            !(await recover(error, context))
          )
            throw error;
          requireCurrent();
          return await this.read();
        }
      })
      .then((value) => {
        requireCurrent();
        const data = immutableMetadata(value);
        this.updated = this.repository.now();
        this.failures = 0;
        this.retryAt = 0;
        this.publish({
          ...this.state,
          data,
          error: undefined,
          fetching: false,
          lastSuccessAt: this.updated,
        });
        this.repository.report({
          kind: 'load-complete',
          milliseconds: Math.max(0, this.repository.now() - started),
        });
        return data;
      })
      .catch((error: unknown) => {
        requireCurrent();
        this.failures = Math.min(this.failures + 1, 5);
        this.retryAt =
          this.repository.now() +
          Math.min(30_000 * 2 ** (this.failures - 1), 300_000);
        this.publish({ ...this.state, error, fetching: false });
        this.repository.report({
          kind: 'load-error',
          milliseconds: Math.max(0, this.repository.now() - started),
        });
        throw error;
      })
      .finally(() => {
        if (this.pending === pending) {
          this.pending = undefined;
          if (this.state.fetching)
            this.publish({ ...this.state, fetching: false });
        }
      });
    this.pending = pending;
    this.pendingGeneration = generation;
    this.repository.report({ kind: 'load-start' });
    this.publish({
      ...this.state,
      error: undefined,
      fetching: true,
      lastAttemptAt: started,
    });
    return pending;
  }
}

export class MetadataRepository {
  private entries = new Map<string, MetadataQuery<unknown>>();
  private observers = new Set<QueryObserver>();
  private accessEpoch = 0;
  private closed = false;
  private reconciliation?: Promise<void>;
  private recoveryOptions: () => QueryLoadOptions = () => ({});
  private recoveryAllowed: () => boolean = () => true;

  constructor(readonly now: () => number = Date.now) {}
  /** Opt-in aggregate events contain no keys, profiles, values or error messages. */
  observe(observer: QueryObserver): () => void {
    this.observers.add(observer);
    return () => {
      this.observers.delete(observer);
    };
  }
  report(event: QueryDiagnostic): void {
    const immutable = Object.freeze({ ...event });
    for (const observer of this.observers) {
      try {
        void Promise.resolve(observer(immutable)).catch(() => undefined);
      } catch {
        /* Diagnostics cannot change request outcomes. */
      }
    }
  }

  /** The shell supplies one pre-execution catalog repair policy for all metadata reads. */
  setReadRecovery(
    options: () => QueryLoadOptions,
    allowed: () => boolean = () => true,
  ): void {
    this.recoveryOptions = options;
    this.recoveryAllowed = allowed;
  }
  canRecoverRead(): boolean {
    return this.recoveryAllowed();
  }
  readOptions(): QueryLoadOptions {
    return this.recoveryOptions();
  }

  get epoch(): number {
    return this.accessEpoch;
  }
  get retired(): boolean {
    return this.closed;
  }

  query<T>(
    key: QueryKey,
    read: () => Promise<T>,
    freshFor = 60_000,
  ): MetadataQuery<T> {
    const id = JSON.stringify(key);
    let entry = this.entries.get(id);
    if (!entry) {
      entry = new MetadataQuery<unknown>(
        Object.freeze([...key]),
        this,
        read,
        freshFor,
      );
      this.entries.set(id, entry);
    }
    return entry as MetadataQuery<T>;
  }

  reconcileSubscribed(
    allowed: () => boolean = () => true,
    include: (key: QueryKey) => boolean = () => true,
  ): Promise<void> {
    if (this.closed || !allowed()) return Promise.resolve();
    if (this.reconciliation) return this.reconciliation;
    const epoch = this.accessEpoch;
    const due = [...this.entries.values()].filter(
      (query) => include(query.key) && query.isDueSubscribed(),
    );
    let next = 0;
    const errors: unknown[] = [];
    const worker = async () => {
      while (
        !this.closed &&
        epoch === this.accessEpoch &&
        allowed() &&
        next < due.length
      ) {
        const query = due[next++];
        if (!query.isDueSubscribed()) continue;
        try {
          await query.load();
        } catch (error) {
          if (!(error instanceof RetiredQueryError)) errors.push(error);
        }
      }
    };
    const pending = Promise.resolve()
      .then(() => Promise.all([worker(), worker()]))
      .then(() => {
        if (!this.closed && epoch === this.accessEpoch && errors.length > 0)
          throw errors[0];
      })
      .finally(() => {
        if (this.reconciliation === pending) this.reconciliation = undefined;
      });
    this.reconciliation = pending;
    return pending;
  }

  /** Stale display data stays available; only matching resource requests restart. */
  invalidate(prefix: QueryKey): void {
    for (const query of this.entries.values()) {
      if (prefix.every((part, index) => query.key[index] === part))
        query.invalidate();
    }
  }

  /** Drop all data and detach old requests. Reusable for React effect replay. */
  clear(): void {
    this.accessEpoch++;
    for (const query of this.entries.values()) query.invalidate(true);
  }

  /** Permanent retirement for an access scope that will not be reused. */
  retire(): void {
    this.closed = true;
    this.clear();
  }
}
