import { refreshActivitiesFor } from '../refresh-activity';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { RefObject } from 'react';
import type { ToastController } from '/kit/toasts';
import {
  isAgentReadinessError,
  loadSnapshot,
  normalizeCommandError,
} from '../bridge';
import type { Bridge } from '../bridge';
import {
  CatalogCoordinator,
  CatalogReadRetiredError,
} from '../catalog-coordinator';
import { CatalogReadGate } from '../catalog-read-gate';
import {
  changedCatalogProfiles,
  failWholeCatalogRefresh,
  markCatalogRefresh,
} from '../catalog-state';
import { storeOf, type AgentSnapshot, type Item } from '../model';
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

/**
 * Attempts a profile-scoped refresh after a mutation. Returns `false` if no
 * profile was supplied, scoped refresh is unavailable, or the request was
 * retired before execution, allowing the caller to reload the full catalog.
 * Other refresh failures propagate.
 */
async function reloadProfile(
  profileRefresh: RefObject<(profile: string) => Promise<void> | null>,
  profile: string | undefined,
): Promise<boolean> {
  const scoped = profile ? profileRefresh.current(profile) : null;
  if (!scoped) return false;
  try {
    await scoped;
    return true;
  } catch (error) {
    if (normalizeCommandError(error).code !== 'catalog-read-retired')
      throw error;
    return false;
  }
}

export function useCatalogRuntime({
  lifetime,
  bridge,
  agentSnapshot,
  retireBoot,
  currentBootSnapshot,
  toasts,
}: {
  lifetime: import('./access-lifetime').AccessLifetime;
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
  /**
   * Discards the metadata read beside the catalog. The argument names the
   * profiles whose catalog changed; `undefined` means the comparison could
   * not be made and every profile's metadata is discarded.
   */
  const metadataInvalidation = useRef<(profiles?: readonly string[]) => void>(
    () => undefined,
  );
  /**
   * Starts a profile-scoped post-mutation refresh through the reconciliation
   * service. Returns `null` when scoped refresh is unavailable so the caller
   * can reload the full catalog.
   */
  const profileRefresh = useRef<(profile: string) => Promise<void> | null>(
    () => null,
  );
  const metadataReconciliation = useRef<() => Promise<void>>(async () => {});
  const deviceRefresh = useRef<() => Promise<boolean>>(async () => true);
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
        (onPartial, catalogCurrent, forced) => {
          const ticket = lifetime.capture();
          const isCurrent = () => ticket.isCurrent() && catalogCurrent();
          const activity = refreshActivitiesFor(bridge).begin(
            'Waiting for current catalog reads',
            isCurrent,
          );
          return catalogGate
            .exclusive(async () => {
              if (!isCurrent()) throw new CatalogReadRetiredError();
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
                  // A refresh the user asked for reads every roster, whether
                  // or not the team's chain has moved.
                  forced,
                  undefined,
                  activity,
                );
              } catch (error) {
                if (isCurrent()) {
                  latestRef.current = failWholeCatalogRefresh(
                    latestRef.current,
                    normalizeCommandError(error),
                  );
                  setLatest(latestRef.current);
                }
                throw error;
              }
            })
            .finally(() => activity.finish());
        },
        (next, forced) => {
          // The comparison is made against the snapshot this publication
          // replaces, so it must run before the new one is installed.
          const changed = forced
            ? changedCatalogProfiles(latestRef.current, next)
            : undefined;
          publishSnapshot(next);
          if (forced) {
            metadataInvalidation.current(changed);
            setHardwareRefresh((generation) => generation + 1);
          }
        },
        () => latestRef.current.catalogProfiles.length,
      ),
    [bridge, catalogGate, publishSnapshot, lifetime],
  );
  useEffect(() => {
    catalogCoordinator.activate();
    const stop = lifetime.subscribe((event) => {
      if (event.profile === undefined) catalogCoordinator.reset();
    });
    return () => {
      stop();
      catalogCoordinator.deactivate();
    };
  }, [catalogCoordinator, lifetime]);
  const refreshSnapshot = useCallback(
    (force = false): Promise<AgentSnapshot> => {
      retireBoot();
      return catalogCoordinator.refresh(force);
    },
    [catalogCoordinator, retireBoot],
  );

  /**
   * Refreshes the affected profile after a mutation when possible; otherwise
   * reloads the full catalog. Both paths report catalog freshness failures.
   */
  const reloadApplied = useCallback(
    async (profile?: string): Promise<void> => {
      if (await reloadProfile(profileRefresh, profile)) return;
      const snapshot = await refreshSnapshot(true);
      const failure = Object.values(
        snapshot.catalogFreshness?.profiles ?? {},
      ).find((entry) => entry.error)?.error;
      if (failure) throw failure;
    },
    [refreshSnapshot],
  );

  const refresh = useCallback(
    async (message: string, profile?: string): Promise<void> => {
      const result = await synchronizeApplied(() => reloadApplied(profile));
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
    [reloadApplied, toasts],
  );

  const refreshAll = (
    reconcile: () => void,
    commandError: CommandErrorHandler,
  ): void => {
    if (refreshingSnapshot) return;
    setRefreshingSnapshot(true);
    void refreshSnapshot(true)
      .then(async (next) => {
        // The Refresh the user asked for is a request for current data, not
        // only for what the catalog says changed: device lists, enrollments
        // and invitation counts are server state the catalog does not
        // describe, so every row is discarded before the reconciliation that
        // reads the visible ones again.
        metadataInvalidation.current();
        reconcile();
        // Account identity changes can replace the metadata owner while its
        // previous reads settle. Wait for the current owner's reads as well.
        let readDevices = deviceRefresh.current;
        let devicesReady = await readDevices();
        while (readDevices !== deviceRefresh.current) {
          readDevices = deviceRefresh.current;
          devicesReady = await readDevices();
        }
        const incomplete = Object.values(
          next.catalogFreshness?.profiles ?? {},
        ).some((entry) => entry.error || entry.refreshing);
        toasts.show(
          incomplete || !devicesReady
            ? 'Refresh completed with unavailable data. See Refresh status.'
            : 'Vaults, teams, and devices refreshed',
          incomplete || !devicesReady ? { tone: 'warning' } : undefined,
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
    deviceRefresh,
    profileRefresh,
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
  catalog: Pick<
    CatalogRuntime,
    'latestRef' | 'profileRefresh' | 'refreshSnapshot'
  >,
): MutationFailureHandler {
  const { latestRef, profileRefresh, refreshSnapshot } = catalog;
  return useCallback<MutationFailureHandler>(
    async (error, options = {}) => {
      const typed = normalizeCommandError(error);
      if (options.report !== false || typed.code === 'agent-lost')
        commandError(error, options.item, options.draft);
      await reconcileMutationFailure(
        error,
        async () => {
          const profile = options.item
            ? storeOf(latestRef.current, options.item.store)?.server
            : undefined;
          if (await reloadProfile(profileRefresh, profile)) return;
          await refreshSnapshot(true);
        },
        commandError,
      );
    },
    [commandError, latestRef, profileRefresh, refreshSnapshot],
  );
}

export type CatalogRuntime = ReturnType<typeof useCatalogRuntime>;
