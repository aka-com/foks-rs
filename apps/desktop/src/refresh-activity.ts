/** Live work, separate from the freshness of the data it last published. */
export interface RefreshActivity {
  id: number;
  startedAt: number;
  phases: readonly string[];
  isCurrent: () => boolean;
}

export interface RefreshOperation {
  update(phase: string): void;
  child(phase: string): RefreshOperation;
  finish(): void;
}

export class RefreshActivities {
  private nextId = 0;
  private active = new Map<
    number,
    {
      startedAt: number;
      isCurrent: () => boolean;
      phases: Map<symbol, string>;
    }
  >();
  private snapshot: readonly RefreshActivity[] = [];
  private listeners = new Set<() => void>();

  getSnapshot = (): readonly RefreshActivity[] => this.snapshot;
  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  private publish(): void {
    this.snapshot = [...this.active].map(([id, entry]) => ({
      id,
      startedAt: entry.startedAt,
      isCurrent: entry.isCurrent,
      phases: [...new Set(entry.phases.values())],
    }));
    for (const listener of this.listeners) listener();
  }

  begin(phase: string, isCurrent = () => true): RefreshOperation {
    const id = ++this.nextId;
    this.active.set(id, {
      startedAt: Date.now(),
      isCurrent,
      phases: new Map(),
    });
    return this.token(id, phase);
  }

  async run<T>(
    phase: string,
    read: () => Promise<T>,
    isCurrent = () => true,
  ): Promise<T> {
    const operation = this.begin(phase, isCurrent);
    try {
      return await read();
    } finally {
      operation.finish();
    }
  }

  private token(id: number, phase: string): RefreshOperation {
    const key = Symbol();
    const entry = this.active.get(id);
    entry?.phases.set(key, phase);
    this.publish();
    let finished = false;
    return {
      update: (next) => {
        if (finished || entry?.phases.get(key) === next) return;
        entry?.phases.set(key, next);
        this.publish();
      },
      child: (next) => {
        if (finished) throw new Error('Cannot extend settled refresh work.');
        return this.token(id, next);
      },
      finish: () => {
        if (finished) return;
        finished = true;
        entry?.phases.delete(key);
        if (!entry?.phases.size) this.active.delete(id);
        this.publish();
      },
    };
  }
}

// Startup and the shell share a bridge, but have different React lifetimes.
// A retired startup read remains visible until its outstanding work settles.
const byBridge = new WeakMap<object, RefreshActivities>();
export function refreshActivitiesFor(bridge: object): RefreshActivities {
  let activities = byBridge.get(bridge);
  if (!activities) {
    activities = new RefreshActivities();
    byBridge.set(bridge, activities);
  }
  return activities;
}
