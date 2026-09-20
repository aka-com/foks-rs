import { useEffect, useMemo, useRef } from 'react';
import type { RefObject } from 'react';
import type { Bridge } from '../bridge';
import { DeviceCache } from '../device-cache';
import { DeviceMetadata, isDeviceQuery } from '../device-metadata';
import { deviceAlertRegistry } from '../screens/device-alert';
import { accountStopped, type AgentSnapshot } from '../model';
import { readRecoveryFor } from '../query-read-recovery';
import { serverStatusKey } from '../resources/servers';
import { serverBinding } from '../screens/servers/server-workflow';

/**
 * The metadata query kinds whose key names the profile immediately after the
 * kind, so one `invalidate([kind, profile])` reaches every row of that kind
 * for that profile: account devices and paper keys, security-key
 * enrollments, pending operations, and the two invitation counts.
 */
const PROFILE_METADATA_KINDS = [
  'account-devices',
  'profile-enrollments',
  'pending-operations',
  'invitation-recovery',
  'team-requests',
] as const;

export function useMetadataRuntime({
  lifetime,
  bridge,
  shown,
  concealSignal,
  accessGenerations,
  metadataInvalidation,
  metadataReconciliation,
  deviceRefresh,
  foregroundRefreshAllowed,
  refreshSnapshot,
  report,
  initialDeviceCache,
}: {
  lifetime: import('./access-lifetime').AccessLifetime;
  bridge: Bridge;
  shown: AgentSnapshot;
  concealSignal: number;
  accessGenerations: ReadonlyMap<string, number>;
  metadataInvalidation: RefObject<(profiles?: readonly string[]) => void>;
  metadataReconciliation: RefObject<() => Promise<void>>;
  deviceRefresh: RefObject<() => Promise<boolean>>;
  foregroundRefreshAllowed: RefObject<boolean>;
  refreshSnapshot: (force?: boolean) => Promise<AgentSnapshot>;
  report: (error: unknown) => void;
  initialDeviceCache?: DeviceCache;
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
  const initialScope = useRef({ bridge, concealSignal, deviceIdentity });
  const deviceCache = useMemo(
    () =>
      initialDeviceCache &&
      initialScope.current.bridge === bridge &&
      initialScope.current.concealSignal === concealSignal &&
      initialScope.current.deviceIdentity === deviceIdentity
        ? initialDeviceCache
        : new DeviceCache(bridge),
    // These values define the lifetime of the cache, not its read arguments.
    [bridge, concealSignal, deviceIdentity, initialDeviceCache],
  );
  const deviceSnapshot = useRef(shown);
  deviceSnapshot.current = shown;
  deviceCache.snapshot = () => deviceSnapshot.current;
  const devices = useMemo(
    () => new DeviceMetadata(deviceCache, deviceAlertRegistry(bridge)),
    [bridge, deviceCache],
  );
  metadataInvalidation.current = (profiles) => {
    // Without a list of changed profiles the comparison could not be made
    // (boot, unlock, agent restart), and every row is discarded as before.
    if (profiles === undefined) {
      deviceCache.repository.invalidate([]);
      return;
    }
    for (const profile of profiles) {
      for (const kind of PROFILE_METADATA_KINDS)
        deviceCache.repository.invalidate([kind, profile]);
      // A server's signed status is keyed by its binding rather than by the
      // profile name, so it is named through the server row it belongs to.
      const server = deviceSnapshot.current.servers.find(
        (candidate) => candidate.id === profile,
      );
      if (server)
        deviceCache.repository.invalidate([
          ...serverStatusKey(serverBinding(server)),
        ]);
    }
  };
  metadataReconciliation.current = async () => {
    // Devices publish their own progress. Cached background reads must not
    // hold the generic metadata spinner open for their entire duration.
    void devices.reconcile();
    await deviceCache.repository.reconcileSubscribed(
      () => foregroundRefreshAllowed.current && !document.hidden,
      (key) => !isDeviceQuery(key),
    );
  };
  deviceRefresh.current = async () => {
    await devices.settle();
    return (
      devices.isAvailable() &&
      devices
        .getSnapshot()
        .every(
          (status) =>
            !status.failed && !status.refreshing && status.unavailable === 0,
        )
    );
  };
  deviceCache.repository.setReadRecovery(
    () =>
      readRecoveryFor(deviceCache.repository).options({
        refresh: () => refreshSnapshot(),
      }),
    () => foregroundRefreshAllowed.current,
  );
  useEffect(() => {
    const update = () =>
      devices.update(
        shown,
        () => foregroundRefreshAllowed.current && !document.hidden,
        report,
      );
    update();
    document.addEventListener('visibilitychange', update);
    return () => document.removeEventListener('visibilitychange', update);
  }, [devices, shown, foregroundRefreshAllowed, report]);
  useEffect(() => {
    const stop = lifetime.subscribe(() => {
      devices.stop();
      deviceCache.clear();
    });
    return () => {
      stop();
      devices.stop();
      deviceCache.clear();
    };
  }, [deviceCache, devices, lifetime]);
  return { cache: deviceCache, devices };
}
