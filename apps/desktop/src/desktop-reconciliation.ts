import { RefreshActivities } from './refresh-activity';
import type { Account, AgentSnapshot, CatalogFreshnessEntry } from './model';
import { discoveryAccounts } from './team-discovery';
import {
  ReconciliationScheduler,
  type ReconciliationClock,
  type ReconciliationContext,
  type ReconciliationJob,
  type ReconciliationSnapshot,
  type ReconciliationTrigger,
} from './scheduling/reconciliation';

export interface DesktopReconciliationReads {
  activities?: RefreshActivities;
  snapshot(): AgentSnapshot;
  profile(profile: string, context: ReconciliationContext): Promise<void>;
  connectivity?(profile: string, context: ReconciliationContext): Promise<void>;
  registry(context: ReconciliationContext): Promise<void>;
  discovery(account: Account, context: ReconciliationContext): Promise<void>;
  metadata(context: ReconciliationContext): Promise<void>;
  nowSeconds(): number;
}

/** How often each eligible account checks for newly joined teams. */
const DISCOVERY_INTERVAL = 300_000;

export const profileRefreshKey = (
  snapshot: AgentSnapshot,
  profile: string,
): string => {
  const server = snapshot.servers.find((candidate) => candidate.id === profile);
  return JSON.stringify([
    'catalog',
    profile,
    server?.host_id,
    server?.configuredProbe,
  ]);
};

/**
 * The connectivity job's key, derived from the profile's configuration
 * alone. A successful observation binds the server's host id and trust
 * status, so a key carrying those would change with the job's own result:
 * the scheduler would retire the entry and, with no initial delay, run the
 * job again the moment it had first succeeded. The catalog key still carries
 * the host id, where a change of identity is a change of what is being read.
 */
export const profileConnectivityKey = (
  snapshot: AgentSnapshot,
  profile: string,
): string => {
  const server = snapshot.servers.find((candidate) => candidate.id === profile);
  return JSON.stringify(['connectivity', profile, server?.configuredProbe]);
};

export class DesktopReconciliation {
  readonly scheduler: ReconciliationScheduler;
  readonly activities: RefreshActivities;
  private accepted = new Map<string, CatalogFreshnessEntry>();
  constructor(
    private reads: DesktopReconciliationReads,
    clock?: ReconciliationClock,
  ) {
    this.scheduler = new ReconciliationScheduler(clock);
    this.activities = reads.activities ?? new RefreshActivities();
  }
  get supportsConnectivity(): boolean {
    return this.reads.connectivity !== undefined;
  }
  update(snapshot: AgentSnapshot): void {
    const profiles =
      snapshot.profileInventoryStatus === 'complete'
        ? snapshot.catalogProfiles
        : snapshot.servers.map((server) => server.id);
    const jobs: ReconciliationJob[] = profiles.map((profile) => ({
      key: profileRefreshKey(snapshot, profile),
      scope: profile,
      kind: 'catalog',
      interval: 30_000,
      eligible: () => {
        const server = this.reads
          .snapshot()
          .servers.find((candidate) => candidate.id === profile);
        return (
          server?.trust.status !== 'blocked' &&
          !server?.restrictions.some(
            (restriction) =>
              restriction.kind === 'schema-incompatible' ||
              restriction.kind === 'import-verification-required',
          )
        );
      },
      run: (context) => this.reads.profile(profile, context),
    }));
    if (this.reads.connectivity) {
      jobs.unshift(
        ...profiles.map((profile): ReconciliationJob => ({
          key: profileConnectivityKey(snapshot, profile),
          scope: profile,
          kind: 'connectivity',
          interval: 120_000,
          initialDelay: 0,
          eligible: () => {
            const current = this.reads.snapshot();
            const server = current.servers.find(
              (candidate) => candidate.id === profile,
            );
            return (
              server !== undefined &&
              server.trust.status !== 'blocked' &&
              ((server.host_id !== null &&
                server.trust.status !== 'unprobed') ||
                current.stores.some((store) => store.server === profile)) &&
              !server.restrictions.some(
                (restriction) =>
                  restriction.kind === 'schema-incompatible' ||
                  restriction.kind === 'import-verification-required',
              )
            );
          },
          run: (context) => this.reads.connectivity!(profile, context),
        })),
      );
    }
    for (const account of snapshot.accounts) {
      const server = snapshot.servers.find(
        (candidate) => candidate.id === account.server,
      );
      jobs.push({
        key: JSON.stringify([
          'discovery',
          account.server,
          account.store,
          account.alias,
          server?.host_id,
          server?.configuredProbe,
        ]),
        scope: account.server,
        kind: 'discovery',
        interval: DISCOVERY_INTERVAL,
        initialDelay: 5_000,
        eligible: () =>
          discoveryAccounts(
            this.reads.snapshot(),
            this.reads.nowSeconds(),
          ).some(
            (candidate) =>
              candidate.store === account.store &&
              candidate.alias === account.alias &&
              candidate.server === account.server,
          ),
        run: (context) => this.reads.discovery(account, context),
      });
    }
    jobs.push({
      key: 'registry',
      scope: null,
      kind: 'registry',
      interval: 60_000,
      run: (context) => this.reads.registry(context),
    });
    jobs.push({
      key: 'metadata',
      scope: null,
      kind: 'metadata',
      interval: 30_000,
      run: (context) => this.reads.metadata(context),
    });
    this.scheduler.update(jobs);
    const retained = new Map<string, CatalogFreshnessEntry>();
    for (const profile of profiles) {
      const key = profileRefreshKey(snapshot, profile);
      const facts = snapshot.catalogFreshness?.profiles[profile];
      if (!facts) continue;
      if (
        facts !== this.accepted.get(key) &&
        facts.lastSuccessAt !== undefined &&
        !facts.refreshing &&
        !facts.error
      )
        this.scheduler.reconciled(key, facts.lastMilliseconds);
      retained.set(key, facts);
    }
    this.accepted = retained;
  }
  wake(trigger: ReconciliationTrigger): void {
    this.scheduler.requestAll(trigger, ['registry', 'metadata']);
    // A focus asks for the catalog as it is now, and a recovery does so
    // after reading the whole catalog itself, so a profile's catalog job is
    // left alone when a read covered it within the last 30 seconds: a focus
    // event fires on every window switch, and before this every recovery
    // read every profile once more right behind the read that had just
    // covered it. A network event asks for every profile: what failed
    // while the network was gone is exactly what has to be read again.
    const settled = trigger === 'foreground' || trigger === 'recovery';
    this.scheduler.requestAll(
      trigger,
      ['catalog'],
      settled
        ? ({ scope, snapshot }) =>
            this.recent(snapshot) || (scope !== null && this.covered(scope))
        : undefined,
    );
    // A recovery reconnects to the agent. Whatever the catalog says about
    // bound teams was frozen while it was gone, so every account is
    // discovered again instead of waiting for its own sweep.
    if (trigger === 'recovery')
      this.scheduler.requestAll(trigger, ['discovery']);
    if (this.reads.connectivity) {
      for (const server of this.reads.snapshot().servers) {
        const key = profileConnectivityKey(this.reads.snapshot(), server.id);
        if (
          trigger !== 'foreground' ||
          !this.recent(this.scheduler.snapshot(key))
        )
          this.scheduler.request(key, trigger);
      }
    }
  }
  /**
   * Whether the job ran within the last 30 seconds. A clock that moved
   * backwards answers no, so the job runs rather than waits on a time that
   * never comes.
   */
  private recent(
    snapshot: Readonly<ReconciliationSnapshot> | undefined,
  ): boolean {
    const last = Math.max(
      snapshot?.lastAttemptAt ?? -Infinity,
      snapshot?.lastSuccessAt ?? -Infinity,
    );
    const elapsed = this.reads.nowSeconds() * 1_000 - last;
    return elapsed >= 0 && elapsed < 30_000;
  }
  /**
   * Whether the live snapshot records a successful catalog read of `profile`
   * that the job has not yet been told of. A whole-catalog read publishes
   * its snapshot before the next `update` reconciles the job with it, and a
   * recovery wakes the jobs right after such a read; once `update` has run,
   * the job's own record answers through `recent`.
   */
  private covered(profile: string): boolean {
    const current = this.reads.snapshot();
    const facts = current.catalogFreshness?.profiles[profile];
    return (
      facts !== undefined &&
      facts !== this.accepted.get(profileRefreshKey(current, profile)) &&
      facts.lastSuccessAt !== undefined &&
      !facts.refreshing &&
      !facts.error
    );
  }
  reconnect(profile: string, trigger: ReconciliationTrigger = 'manual'): void {
    this.scheduler.request(
      profileConnectivityKey(this.reads.snapshot(), profile),
      trigger,
    );
  }
  /**
   * Requests a profile-scoped catalog refresh and resolves when it completes.
   * Returns `null` when reconciliation is unavailable, allowing the caller to
   * reload the full catalog.
   */
  refreshProfile(profile: string): Promise<void> | null {
    return this.scheduler.run(
      profileRefreshKey(this.reads.snapshot(), profile),
      'mutation',
    );
  }
  invalidate(profile?: string): void {
    if (profile)
      this.scheduler.request(
        profileRefreshKey(this.reads.snapshot(), profile),
        'mutation',
        true,
      );
    else {
      for (const server of this.reads.snapshot().servers)
        this.scheduler.request(
          profileRefreshKey(this.reads.snapshot(), server.id),
          'mutation',
          true,
        );
    }
  }
  dispose(): void {
    this.scheduler.dispose();
  }
}
