import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ToastController } from '/kit/toasts';
import {
  isAgentReadinessError,
  loadSnapshot,
  normalizeCommandError,
} from '../bridge';
import type { Bridge } from '../bridge';
import { CatalogCoordinator } from '../catalog-coordinator';
import { CatalogReadGate } from '../catalog-read-gate';
import { failCatalogRefresh, markCatalogRefresh } from '../catalog-state';
import type { AgentSnapshot, Item } from '../model';
import {
  reconcileMutationFailure,
  type MutationFailureHandler,
} from '../mutation-recovery';
import { synchronizeApplied } from '../operation-outcome';

export type CommandErrorHandler = (
  error: unknown,
  item?: Item,
  draft?: string,
) => void;

export function useCatalogRuntime({
  bridge,
  agentSnapshot,
  retireBoot,
  currentBootSnapshot,
  toasts,
}: {
  bridge: Bridge;
  agentSnapshot: AgentSnapshot;
  retireBoot: () => void;
  currentBootSnapshot: () => boolean;
  toasts: ToastController;
}) {
  const [latest, setLatest] = useState(agentSnapshot);
  const latestRef = useRef(latest);
  latestRef.current = latest;
  const [agentCatalogReady, setAgentCatalogReady] = useState(true);
  const [refreshingSnapshot, setRefreshingSnapshot] = useState(false);
  const metadataInvalidation = useRef<() => void>(() => undefined);
  const metadataReconciliation = useRef<() => Promise<void>>(async () => {});
  const [hardwareRefresh, setHardwareRefresh] = useState(0);
  const commandErrorRef = useRef<CommandErrorHandler>(() => undefined);

  useEffect(() => {
    if (currentBootSnapshot()) setLatest(agentSnapshot);
  }, [agentSnapshot, currentBootSnapshot]);

  const [catalogGate] = useState(() => new CatalogReadGate());
  const publishSnapshot = useCallback(
    (next: AgentSnapshot, accepted = true): void => {
      latestRef.current = next;
      setLatest(next);
      if (accepted) setAgentCatalogReady(true);
    },
    [],
  );
  const catalogCoordinator = useMemo(
    () =>
      new CatalogCoordinator<AgentSnapshot>(
        (onPartial, isCurrent) =>
          catalogGate.exclusive(async () => {
            if (!isCurrent())
              throw Object.assign(new Error('Catalog load was retired.'), {
                code: 'catalog-read-retired',
              });
            const base = markCatalogRefresh(latestRef.current);
            latestRef.current = base;
            setLatest(base);
            try {
              return await loadSnapshot(
                bridge,
                base,
                undefined,
                onPartial,
                isCurrent,
              );
            } catch (error) {
              if (isCurrent()) {
                latestRef.current = failCatalogRefresh(
                  latestRef.current,
                  base.catalogProfiles,
                  normalizeCommandError(error),
                );
                setLatest(latestRef.current);
              }
              throw error;
            }
          }),
        (next, forced) => {
          publishSnapshot(next);
          if (forced) {
            metadataInvalidation.current();
            setHardwareRefresh((generation) => generation + 1);
          }
        },
      ),
    [bridge, catalogGate, publishSnapshot],
  );
  useEffect(() => {
    catalogCoordinator.activate();
    return () => catalogCoordinator.deactivate();
  }, [catalogCoordinator]);
  const refreshSnapshot = useCallback(
    (force = false): Promise<AgentSnapshot> => {
      retireBoot();
      return catalogCoordinator.refresh(force);
    },
    [catalogCoordinator, retireBoot],
  );

  const refresh = useCallback(
    async (message: string): Promise<void> => {
      const result = await synchronizeApplied(async () => {
        const snapshot = await refreshSnapshot(true);
        const failure = Object.values(
          snapshot.catalogFreshness?.profiles ?? {},
        ).find((entry) => entry.error)?.error;
        if (failure) throw failure;
        return snapshot;
      });
      if (result.synchronization === 'pending') {
        if (result.error.code === 'catalog-read-retired') return;
        if (isAgentReadinessError(result.error))
          commandErrorRef.current(result.error);
        else
          toasts.show(
            'Change completed. Updated data could not be loaded. Use Refresh to reload it.',
            { tone: 'warning' },
          );
        return;
      }
      toasts.show(message);
    },
    [refreshSnapshot, toasts],
  );

  const refreshAll = (
    reconcile: () => void,
    commandError: CommandErrorHandler,
  ): void => {
    if (refreshingSnapshot) return;
    setRefreshingSnapshot(true);
    void refreshSnapshot(true)
      .then((next) => {
        reconcile();
        const incomplete = Object.values(
          next.catalogFreshness?.profiles ?? {},
        ).some((entry) => entry.error || entry.refreshing);
        toasts.show(
          incomplete
            ? 'Refresh completed with unavailable data. See Refresh status.'
            : 'Vaults and teams refreshed',
          incomplete ? { tone: 'warning' } : undefined,
        );
      })
      .catch(commandError)
      .finally(() => setRefreshingSnapshot(false));
  };

  return {
    latest,
    latestRef,
    setLatest,
    agentCatalogReady,
    setAgentCatalogReady,
    refreshingSnapshot,
    hardwareRefresh,
    metadataInvalidation,
    metadataReconciliation,
    commandErrorRef,
    catalogGate,
    catalogCoordinator,
    publishSnapshot,
    refreshSnapshot,
    refresh,
    refreshAll,
  };
}

export function useMutationError(
  commandError: CommandErrorHandler,
  refreshSnapshot: (force?: boolean) => Promise<AgentSnapshot>,
): MutationFailureHandler {
  return useCallback<MutationFailureHandler>(
    async (error, options = {}) => {
      const typed = normalizeCommandError(error);
      if (options.report !== false || typed.code === 'agent-lost')
        commandError(error, options.item, options.draft);
      await reconcileMutationFailure(
        error,
        async () => {
          await refreshSnapshot(true);
        },
        commandError,
      );
    },
    [commandError, refreshSnapshot],
  );
}

export type CatalogRuntime = ReturnType<typeof useCatalogRuntime>;
