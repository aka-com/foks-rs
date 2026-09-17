import type { Server } from '../model';

export interface LeaseExpiryClock {
  now(): number;
  later(callback: () => void, delayMs: number): unknown;
  cancel(timer: unknown): void;
}

export const systemLeaseExpiryClock: LeaseExpiryClock = {
  now: () => Date.now() / 1000,
  later: (callback, delayMs) => setTimeout(callback, delayMs),
  cancel: (timer) => clearTimeout(timer as ReturnType<typeof setTimeout>),
};

export interface ObservedLeaseExpiry {
  profile: string;
  expiresAt: number;
}

/**
 * Reconcile fail-closed observations with a new authenticated server snapshot.
 * A clock rollback or reread of the same/older lease cannot reopen access.
 */
export function reconcileObservedLeaseExpiries(
  servers: readonly Server[],
  previous: readonly ObservedLeaseExpiry[],
  nowSeconds: number,
): ObservedLeaseExpiry[] {
  const old = new Map(previous.map((entry) => [entry.profile, entry.expiresAt]));
  const next: ObservedLeaseExpiry[] = [];
  for (const server of servers) {
    const expiry = server.compatibility;
    const observed = old.get(server.id);
    if (expiry.status === 'not-required') continue;
    if (expiry.status !== 'required') {
      if (observed !== undefined)
        next.push({ profile: server.id, expiresAt: observed });
      continue;
    }
    if (expiry.expiresAt <= nowSeconds) {
      next.push({
        profile: server.id,
        expiresAt: Math.max(observed ?? expiry.expiresAt, expiry.expiresAt),
      });
      continue;
    }
    // Only a newer authenticated lease can clear an observed expiry.
    if (observed !== undefined && expiry.expiresAt <= observed)
      next.push({ profile: server.id, expiresAt: observed });
  }
  return next.sort((left, right) => left.profile.localeCompare(right.profile));
}

export interface LeaseExpiryChange {
  observed: readonly ObservedLeaseExpiry[];
  newlyExpired: readonly ObservedLeaseExpiry[];
}

/** Manages one bounded timer for all known compatibility-lease expirations. */
export class LeaseExpiryCoordinator {
  private servers: readonly Server[] = [];
  private observed: ObservedLeaseExpiry[] = [];
  private timer: unknown;
  private disposed = false;
  private revision = 0;

  constructor(
    private readonly clock: LeaseExpiryClock,
    private readonly onChange: (change: LeaseExpiryChange) => void,
    private readonly maximumDelayMs = 60_000,
  ) {}

  update(
    servers: readonly Server[],
    seeded: readonly ObservedLeaseExpiry[] = [],
  ): void {
    this.servers = servers;
    const combined = new Map<string, number>();
    for (const entry of [...this.observed, ...seeded])
      combined.set(
        entry.profile,
        Math.max(combined.get(entry.profile) ?? -Infinity, entry.expiresAt),
      );
    this.observed = [...combined].map(([profile, expiresAt]) => ({
      profile,
      expiresAt,
    }));
    this.reconcile();
  }

  foreground(): void {
    this.reconcile();
  }

  snapshot(): readonly ObservedLeaseExpiry[] {
    return this.observed;
  }

  dispose(): void {
    this.disposed = true;
    this.revision++;
    this.clock.cancel(this.timer);
  }

  private reconcile(): void {
    if (this.disposed) return;
    const revision = ++this.revision;
    this.clock.cancel(this.timer);
    const before = new Map(
      this.observed.map((entry) => [entry.profile, entry.expiresAt]),
    );
    const next = reconcileObservedLeaseExpiries(
      this.servers,
      this.observed,
      this.clock.now(),
    );
    const newlyExpired = next.filter(
      (entry) =>
        before.get(entry.profile) === undefined ||
        entry.expiresAt > (before.get(entry.profile) ?? -Infinity),
    );
    const changed =
      next.length !== this.observed.length ||
      next.some(
        (entry, index) =>
          entry.profile !== this.observed[index]?.profile ||
          entry.expiresAt !== this.observed[index]?.expiresAt,
      );
    this.observed = next;
    if (changed || newlyExpired.length)
      this.onChange({ observed: next, newlyExpired });
    if (this.disposed || revision !== this.revision) return;

    const now = this.clock.now();
    const nextExpiry = this.servers
      .flatMap((server) => {
        const compatibility = server.compatibility;
        return compatibility.status === 'required' &&
        !next.some(
          (entry) =>
            entry.profile === server.id &&
            entry.expiresAt >= compatibility.expiresAt,
        )
          ? [compatibility.expiresAt]
          : [];
      })
      .sort((left, right) => left - right)[0];
    const untilExpiry =
      nextExpiry === undefined
        ? this.maximumDelayMs
        : Math.max(0, (nextExpiry - now) * 1000);
    this.timer = this.clock.later(
      () => this.reconcile(),
      Math.min(this.maximumDelayMs, untilExpiry),
    );
  }
}
