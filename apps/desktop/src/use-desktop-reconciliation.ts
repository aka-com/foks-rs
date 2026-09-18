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
} from './catalog-state';
import { CatalogReadGate } from './catalog-read-gate';
import { DesktopReconciliation } from './desktop-reconciliation';
import type { AgentSnapshot } from './model';
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
    const profile = async (
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
        );
        if (!context.isCurrent()) return;
        const merged = mergeProfileSnapshot(
          live.current.current(),
          next,
          profile,
        );
        live.current.publish(merged);
        observationError = merged.catalogFreshness?.profiles[profile]?.error;
      } catch (error) {
        if (!context.isCurrent()) return;
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
        report(error);
        throw error;
      }
      if (observationError) {
        report(observationError);
        throw observationError;
      }
    };
    return new DesktopReconciliation(
      {
        snapshot: () => live.current.current(),
        nowSeconds: () => live.current.nowSeconds(),
        profile: (name, context) =>
          options.gate.profile(() => profile(name, context)),
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
                () => profile(account.server, context),
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
  }, [options.gate, options.clock]);
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
