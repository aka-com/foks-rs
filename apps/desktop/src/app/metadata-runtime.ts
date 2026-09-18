import { useEffect, useMemo, useRef } from 'react';
import type { RefObject } from 'react';
import type { Bridge } from '../bridge';
import { DeviceCache } from '../device-cache';
import { accountStopped, type AgentSnapshot } from '../model';
import { readRecoveryFor } from '../query-read-recovery';

export function useMetadataRuntime({
  lifetime,
  bridge,
  shown,
  concealSignal,
  accessGenerations,
  metadataInvalidation,
  metadataReconciliation,
  foregroundRefreshAllowed,
  refreshSnapshot,
}: {
  lifetime: import('./access-lifetime').AccessLifetime;
  bridge: Bridge;
  shown: AgentSnapshot;
  concealSignal: number;
  accessGenerations: ReadonlyMap<string, number>;
  metadataInvalidation: RefObject<() => void>;
  metadataReconciliation: RefObject<() => Promise<void>>;
  foregroundRefreshAllowed: RefObject<boolean>;
  refreshSnapshot: (force?: boolean) => Promise<AgentSnapshot>;
}) {
  // Display labels do not identify accounts. Replacing identities or access
  // retires all cached metadata. Ordinary publication keeps resource data; a
  // forced full refresh invalidates it after the new catalog is installed.
  const deviceIdentity = JSON.stringify({
    accounts: shown.accounts.map(({ store, alias, server }) => [
      store,
      alias,
      server,
    ]),
    servers: shown.servers.map(({ id, host_id, configuredProbe }) => [
      id,
      host_id,
      configuredProbe,
    ]),
    access: shown.stores
      .filter((store) => store.kind === 'account')
      .map((store) => [store.id, accountStopped(shown, store).stopped]),
    generations: [...accessGenerations],
  });
  const deviceCache = useMemo(
    () => new DeviceCache(bridge),
    // These values define the lifetime of the cache, not its read arguments.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [bridge, concealSignal, deviceIdentity],
  );
  const deviceSnapshot = useRef(shown);
  deviceSnapshot.current = shown;
  deviceCache.snapshot = () => deviceSnapshot.current;
  metadataInvalidation.current = () => deviceCache.repository.invalidate([]);
  metadataReconciliation.current = () =>
    deviceCache.repository.reconcileSubscribed(
      () => foregroundRefreshAllowed.current && !document.hidden,
    );
  deviceCache.repository.setReadRecovery(
    () =>
      readRecoveryFor(deviceCache.repository).options({
        refresh: () => refreshSnapshot(),
      }),
    () => foregroundRefreshAllowed.current,
  );
  useEffect(() => {
    const stop = lifetime.subscribe((event) => {
      if (event.profile === undefined) deviceCache.clear();
    });
    return () => {
      stop();
      deviceCache.clear();
    };
  }, [deviceCache, lifetime]);
  return deviceCache;
}
