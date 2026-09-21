import { useEffect, useMemo, useRef } from 'react';
import type { Bridge } from './bridge';
import {
  isAgentReadinessError,
  loadProfileSnapshot,
  normalizeCommandError,
} from './bridge';
import {
  failCatalogRefresh,
  markCatalogRefresh,
  mergeProfileSnapshot,
  settleCatalogRefresh,
} from './catalog-state';
import { CatalogReadGate } from './catalog-read-gate';
import {
  DesktopReconciliation,
  profileRefreshKey,
} from './desktop-reconciliation';
import {
  connectionFactsUnchanged,
  observeProfileConnection,
} from './profile-connectivity';
import { scheduleProfileWork } from './scheduling/profile-work';
import type { AgentSnapshot, Server } from './model';
import { discoveryAccounts, reconcileTeamDiscovery } from './team-discovery';
import type {
  ReconciliationClock,
  ReconciliationContext,
} from './scheduling/reconciliation';

interface Options {
  bridge: Bridge;
  snapshot: AgentSnapshot;
  enabled: boolean;
  gate: CatalogReadGate;
  clock?: ReconciliationClock;
  current(): AgentSnapshot;
  retireBoot?(): void;
  publish(snapshot: AgentSnapshot, accepted?: boolean): void;
  refresh(): Promise<unknown>;
  metadata(): Promise<void>;
  report(error: unknown): void;
  nowSeconds(): number;
}

export function useDesktopReconciliation(
  options: Options,
): DesktopReconciliation {
  const live = useRef(options);
  live.current = options;
  const service = useMemo(() => {
    const report = (error: unknown): void => {
      const typed = normalizeCommandError(error);
      if (
        isAgentReadinessError(typed) ||
        [
          'deadline-exceeded',
          'unsafe-socket',
          'protocol',
          'response-binding',
        ].includes(typed.code)
      )
        live.current.report(error);
    };
    // Job callbacks use this binding after the service has been initialized below.
    /**
     * Clear the profile's refreshing flag when a cancelled job exits before
     * publishing. Otherwise the UI would continue to show an active refresh
     * until another job updates the state.
     */
    const settle = (profile: string): void => {
      const current = live.current.current();
      if (current.catalogFreshness?.profiles[profile]?.refreshing)
        live.current.publish(settleCatalogRefresh(current, [profile]), false);
    };
    /**
     * Returns whether the profile's catalog job exists and is eligible to run.
     * Blocked or incompatible profiles cannot service an awaited refresh.
     */
    const catalogJobRuns = (profile: string, key: string): boolean => {
      const server = live.current
        .current()
        .servers.find((candidate) => candidate.id === profile);
      return (
        service.scheduler.snapshot(key) !== undefined &&
        server?.trust.status !== 'blocked' &&
        !server?.restrictions.some(
          (restriction) =>
            restriction.kind === 'schema-incompatible' ||
            restriction.kind === 'import-verification-required',
        )
      );
    };
    /**
     * Defer publishing a connectivity result until the subsequent catalog
     * refresh completes. Publish it only if the server configuration still
     * matches the configuration used for the observation.
     */
    type Observation = {
      before: Server;
      observed: Awaited<ReturnType<NonNullable<Bridge['reconcileServer']>>>;
      observedAt: number;
    };
    const pendingObservations = new Map<string, Observation>();
    const publishObservation = (name: string): void => {
      const pending = pendingObservations.get(name);
      if (!pending) return;
      pendingObservations.delete(name);
      const { before, observed, observedAt } = pending;
      const latest = live.current.current();
      const after = latest.servers.find((server) => server.id === name);
      // Discard an observation if the server configuration changed after it was
      // collected, then schedule another connectivity check using the current
      // configuration.
      if (
        !after ||
        before.configuredProbe !== after.configuredProbe ||
        (before.host_id && after.host_id && before.host_id !== after.host_id) ||
        (before.host_id &&
          after.host_id === null &&
          after.trust.status === 'unprobed' &&
          observed.identity.status === 'connected')
      ) {
        service.reconnect(name, 'recovery');
        return;
      }
      const binding =
        after.host_id === null && after.trust.status === 'unknown'
          ? { ...after, host_id: before.host_id }
          : after;
      let connectivity: Server['connectivity'];
      try {
        connectivity = observeProfileConnection(binding, observed, observedAt);
      } catch {
        // Discard the observation if it no longer matches the current server state.
        // A later connectivity check will collect a current result; this does not
        // change the catalog job's outcome.
        return;
      }
      live.current.publish(
        {
          ...latest,
          servers: latest.servers.map((server) =>
            server.id === name ? { ...server, connectivity } : server,
          ),
        },
        false,
      );
    };
    /**
     * Schedules the profile's catalog job so the refresh uses its normal
     * cancellation, retry, and diagnostic handling. A request received during
     * an active read schedules one follow-up run.
     */
    const requestCatalog = (
      profile: string,
      context: ReconciliationContext,
    ): Promise<void> => {
      const key = profileRefreshKey(live.current.current(), profile);
      if (catalogJobRuns(profile, key)) {
        service.scheduler.request(key, 'recovery', true);
        return Promise.resolve();
      }
      return readProfile(profile, context);
    };
    const readProfile = async (
      profile: string,
      context: ReconciliationContext,
    ): Promise<void> => {
      if (!context.isCurrent()) return;
      const current = live.current;
      current.retireBoot?.();
      current.publish(
        markCatalogRefresh(current.current(), [profile], current.nowSeconds()),
        false,
      );
      let observationError: unknown;
      try {
        const next = await loadProfileSnapshot(
          current.bridge,
          profile,
          current.current(),
          current.nowSeconds(),
          context.isCurrent,
          {
            key: `catalog:${profile}`,
            owner: context,
            generation: 0,
            signal: context.signal,
            current: context.isCurrent,
            cancel: () => undefined,
            preemptible: false,
          },
          // A read back of a write, or a refresh the user asked for, reads
          // this profile's rosters even when no team chain has moved.
          context.trigger === 'mutation' || context.trigger === 'manual',
        );
        if (!context.isCurrent()) {
          settle(profile);
          return;
        }
        const merged = mergeProfileSnapshot(
          live.current.current(),
          next,
          profile,
        );
        live.current.publish(merged);
        observationError = merged.catalogFreshness?.profiles[profile]?.error;
      } catch (error) {
        if (!context.isCurrent()) {
          settle(profile);
          return;
        }
        const current = live.current;
        current.publish(
          failCatalogRefresh(
            current.current(),
            [profile],
            normalizeCommandError(error),
            current.nowSeconds(),
          ),
          false,
        );
        publishObservation(profile);
        report(error);
        throw error;
      }
      publishObservation(profile);
      if (observationError) {
        report(observationError);
        throw observationError;
      }
    };
    const service = new DesktopReconciliation(
      {
        snapshot: () => live.current.current(),
        nowSeconds: () => live.current.nowSeconds(),
        profile: (name, context) =>
          options.gate.profile(() => readProfile(name, context)),
        connectivity: options.bridge.reconcileServer
          ? (name, context) =>
              options.gate.profile(async () => {
                if (!context.isCurrent()) return;
                const current = live.current;
                const before = current
                  .current()
                  .servers.find((server) => server.id === name);
                if (!before || !current.bridge.reconcileServer) return;
                current.retireBoot?.();
                try {
                  const observed = await scheduleProfileWork(
                    current.bridge,
                    name,
                    () => current.bridge.reconcileServer!(name),
                    {
                      key: `connectivity:${name}`,
                      owner: context,
                      generation: 0,
                      signal: context.signal,
                      current: context.isCurrent,
                      cancel: () => undefined,
                      preemptible: false,
                    },
                  );
                  if (!context.isCurrent()) return;
                  const observedAt = live.current.nowSeconds();
                  const errors = [
                    observed.identity,
                    observed.compatibility,
                  ].flatMap((result) =>
                    result.status === 'failed' ? [result.error] : [],
                  );
                  if (
                    errors.some(
                      (error) => error.code === 'profile-configuration-changed',
                    )
                  )
                    throw {
                      code: 'catalog-read-retired',
                      message: 'The profile changed during reconciliation.',
                      retryable: false,
                      fatal: false,
                      ambiguous: false,
                    };
                  pendingObservations.set(name, {
                    before,
                    observed,
                    observedAt,
                  });
                  // Publish unchanged connectivity data immediately without requesting
                  // a catalog refresh. The catalog job retains its normal 30-second schedule.
                  if (connectionFactsUnchanged(before, observed)) {
                    publishObservation(name);
                    return;
                  }
                  await requestCatalog(name, context);
                  if (!context.isCurrent()) return;
                  if (errors[0]) throw errors[0];
                } catch (error) {
                  if (!context.isCurrent()) return;
                  report(error);
                  throw error;
                }
              })
          : undefined,
        registry: async (context) => {
          try {
            const catalog = await live.current.bridge.listStores();
            if (!context.isCurrent()) return;
            const before = [...live.current.current().catalogProfiles].sort();
            const after = [...catalog.profiles].sort();
            if (
              JSON.stringify(before) !== JSON.stringify(after) ||
              live.current.current().profileInventoryStatus !== 'complete'
            )
              await live.current.refresh();
          } catch (error) {
            if (!context.isCurrent()) return;
            report(error);
            throw error;
          }
        },
        discovery: (account, context) =>
          options.gate.profile(async () => {
            if (!context.isCurrent()) return;
            live.current.retireBoot?.();
            try {
              await reconcileTeamDiscovery(
                live.current.bridge,
                account,
                context,
                () =>
                  discoveryAccounts(
                    live.current.current(),
                    live.current.nowSeconds(),
                  ).some(
                    (candidate) =>
                      candidate.store === account.store &&
                      candidate.alias === account.alias &&
                      candidate.server === account.server,
                  ),
                () => requestCatalog(account.server, context),
              );
            } catch (error) {
              if (!context.isCurrent()) return;
              report(error);
              throw error;
            }
          }),
        metadata: async (context) => {
          if (!context.isCurrent()) return;
          try {
            await live.current.metadata();
          } catch (error) {
            if (!context.isCurrent()) return;
            report(error);
            throw error;
          }
        },
      },
      options.clock,
    );
    return service;
  }, [options.bridge, options.gate, options.clock]);
  useEffect(
    () => service.update(options.snapshot),
    [service, options.snapshot],
  );
  useEffect(() => {
    service.scheduler.setEnabled(options.bridge.native && options.enabled);
    return () => service.scheduler.setEnabled(false);
  }, [service, options.bridge, options.enabled]);
  useEffect(() => {
    const foreground = () => {
      service.scheduler.setVisible(!document.hidden);
      if (!document.hidden) service.wake('foreground');
    };
    const online = () => service.wake('network');
    service.scheduler.setVisible(!document.hidden);
    window.addEventListener('focus', foreground);
    window.addEventListener('pageshow', foreground);
    window.addEventListener('online', online);
    document.addEventListener('visibilitychange', foreground);
    return () => {
      window.removeEventListener('focus', foreground);
      window.removeEventListener('pageshow', foreground);
      window.removeEventListener('online', online);
      document.removeEventListener('visibilitychange', foreground);
    };
  }, [service]);
  return service;
}
