import type { Account, AgentSnapshot, CatalogFreshnessEntry } from './model';
import { discoveryAccounts } from './team-discovery';
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
  registry(context: ReconciliationContext): Promise<void>;
  discovery(account: Account, context: ReconciliationContext): Promise<void>;
  metadata(context: ReconciliationContext): Promise<void>;
  nowSeconds(): number;
}

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

export class DesktopReconciliation {
  readonly scheduler: ReconciliationScheduler;
  private accepted = new Map<string, CatalogFreshnessEntry>();
  constructor(
    private reads: DesktopReconciliationReads,
    clock?: ReconciliationClock,
  ) {
    this.scheduler = new ReconciliationScheduler(clock);
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
        interval: 300_000,
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
