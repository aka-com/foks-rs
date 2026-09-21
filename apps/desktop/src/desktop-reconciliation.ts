import type { Account, AgentSnapshot, CatalogFreshnessEntry } from './model';
import { accountHasBoundTeam, discoveryAccounts } from './team-discovery';
import {
  ReconciliationScheduler,
  type ReconciliationClock,
  type ReconciliationContext,
  type ReconciliationJob,
  type ReconciliationTrigger,
} from './scheduling/reconciliation';

export interface DesktopReconciliationReads {
  snapshot(): AgentSnapshot;
  profile(profile: string, context: ReconciliationContext): Promise<void>;
  connectivity?(profile: string, context: ReconciliationContext): Promise<void>;
  registry(context: ReconciliationContext): Promise<void>;
  discovery(account: Account, context: ReconciliationContext): Promise<void>;
  metadata(context: ReconciliationContext): Promise<void>;
  nowSeconds(): number;
}

/** How often discovery runs for an account with a team still to bind. */
const DISCOVERY_INTERVAL = 300_000;
/**
 * An account whose teams the catalog already binds has nothing for the next
 * discovery to bind, so it runs one in this many instead. That sweep, the run
 * every account gets shortly after launch, and the one a recovery asks for
 * are what still find a team joined since. Each run the sweep replaces is a
 * `discover_groups` the agent does not serve and a catalog invalidation that
 * does not follow it.
 */
const DISCOVERY_SWEEP_RUNS = 6;

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
  private accepted = new Map<string, CatalogFreshnessEntry>();
  constructor(
    private reads: DesktopReconciliationReads,
    clock?: ReconciliationClock,
  ) {
    this.scheduler = new ReconciliationScheduler(clock);
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
        interval: accountHasBoundTeam(snapshot, account)
          ? DISCOVERY_INTERVAL * DISCOVERY_SWEEP_RUNS
          : DISCOVERY_INTERVAL,
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
        this.scheduler.reconciled(key);
      retained.set(key, facts);
    }
    this.accepted = retained;
  }
  wake(trigger: ReconciliationTrigger): void {
    this.scheduler.requestAll(trigger, ['catalog', 'registry', 'metadata']);
    // A recovery reconnects to the agent. Whatever the catalog says about
    // bound teams was frozen while it was gone, so every account is
    // discovered again instead of waiting for its own sweep.
    if (trigger === 'recovery')
      this.scheduler.requestAll(trigger, ['discovery']);
    if (this.reads.connectivity) {
      for (const server of this.reads.snapshot().servers) {
        const key = profileConnectivityKey(this.reads.snapshot(), server.id);
        const last = this.scheduler.snapshot(key)?.lastAttemptAt;
        const elapsed =
          last === undefined
            ? Infinity
            : this.reads.nowSeconds() * 1_000 - last;
        if (trigger !== 'foreground' || elapsed < 0 || elapsed >= 30_000)
          this.scheduler.request(key, trigger);
      }
    }
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
