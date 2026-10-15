/** Serializes compound catalog reads and coalesces invalidations into a trailing load. */
export class CatalogReadRetiredError extends Error {
  readonly code = 'catalog-read-retired';
  readonly retryable = false;
  readonly ambiguous = false;
  readonly fatal = false;
  constructor() {
    super('The catalog request belongs to an earlier access generation.');
    this.name = 'CatalogReadRetiredError';
  }
}

export interface CatalogReadTiming {
  outcome: 'published' | 'superseded' | 'failed' | 'retired';
  milliseconds: number;
  /** The read answered a request for fresh facts: a Refresh, or a read back of a write. */
  forced: boolean;
  /**
   * How many profiles the read covered, which is what its duration has to be
   * read against: one catalog read over six servers is not the same read as
   * one over one.
   */
  profiles: number;
}

export class CatalogCoordinator<T> {
  private pending: Promise<T> | null = null;
  private epoch = 0;
  private runningEpoch = 0;
  private dirty = false;
  private active = true;
  private observers = new Set<
    (event: CatalogReadTiming) => void | Promise<void>
  >();

  /** Opt-in aggregate timings; catalog identities and contents are never reported. */
  observe(
    observer: (event: CatalogReadTiming) => void | Promise<void>,
  ): () => void {
    this.observers.add(observer);
    return () => {
      this.observers.delete(observer);
    };
  }

  private report(
    outcome: CatalogReadTiming['outcome'],
    started: number,
    forced: boolean,
  ): void {
    let profiles = 0;
    try {
      profiles = this.observedProfiles();
    } catch {
      /* Diagnostics cannot affect publication. */
    }
    const event = Object.freeze({
      outcome,
      milliseconds: performance.now() - started,
      forced,
      profiles,
    });
    for (const observer of this.observers) {
      try {
        void Promise.resolve(observer(event)).catch(() => undefined);
      } catch {
        /* Diagnostics cannot affect publication. */
      }
    }
  }

  constructor(
    private readonly read: (
      onPartial: (value: T) => void,
      isCurrent: () => boolean,
      /** The read answers a request that asked for fresh facts. */
      forced: boolean,
    ) => Promise<T>,
    private readonly publish: (value: T, forced: boolean) => void,
    /** How many profiles a read covers, for the timings only. */
    private readonly observedProfiles: () => number = () => 0,
  ) {}

  /** Retire replies on lock, maintenance or connection loss without overlapping reads. */
  reset(): void {
    this.epoch++;
    this.dirty = false;
  }

  activate(): void {
    this.active = true;
  }

  deactivate(): void {
    this.active = false;
    this.reset();
  }

  refresh(force = false): Promise<T> {
    if (!this.active) return Promise.reject(new CatalogReadRetiredError());
    if (this.pending) {
      if (force || this.runningEpoch !== this.epoch) this.dirty = true;
      return this.pending;
    }
    this.runningEpoch = this.epoch;
    const pending = Promise.resolve()
      .then(async () => {
        if (this.runningEpoch !== this.epoch && !this.dirty)
          throw new CatalogReadRetiredError();
        let forced = force || this.dirty;
        for (;;) {
          const epoch = this.epoch;
          this.runningEpoch = epoch;
          // A request before this read starts is already included in it.
          this.dirty = false;
          const started = performance.now();
          let value: T;
          let reading = true;
          const isCurrent = (): boolean =>
            reading && this.active && epoch === this.epoch && !this.dirty;
          try {
            // Publish intermediate results as non-forced updates. Marking each
            // partial as forced would make consumers rebuild caches once per
            // profile and again for the completed catalog.
            value = await this.read(
              (partial) => {
                if (isCurrent()) this.publish(partial, false);
              },
              isCurrent,
              forced,
            );
          } catch (error) {
            // A failed read does not publish a forced result, so dependent caches
            // remain unchanged until a later successful refresh.
            if (this.dirty) {
              this.report('superseded', started, forced);
              forced = true;
              continue;
            }
            if (epoch !== this.epoch) {
              this.report('retired', started, forced);
              throw new CatalogReadRetiredError();
            }
            this.report('failed', started, forced);
            throw error;
          } finally {
            reading = false;
          }
          if (this.dirty) {
            this.report('superseded', started, forced);
            forced = true;
            continue;
          }
          if (epoch !== this.epoch) {
            this.report('retired', started, forced);
            throw new CatalogReadRetiredError();
          }
          // Publication can synchronously request a refresh. The completed read
          // no longer owns the pending slot, so that request starts a new drain.
          if (this.pending === pending) this.pending = null;
          this.publish(value, forced);
          this.report('published', started, forced);
          return value;
        }
      })
      .finally(() => {
        if (this.pending === pending) this.pending = null;
      });
    this.pending = pending;
    return pending;
  }
}
