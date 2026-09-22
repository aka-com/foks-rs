/** Session-only metadata queries. Reader presence, PIN state and secrets are excluded. */
import { createContext, useCallback, useContext, useMemo, useRef } from 'react';
import {
  requireWorkflow,
  workflowAvailability,
} from './model/workflow-availability';
import type { AgentSnapshot } from './model/types';
import { scheduleProfileWork } from './scheduling/profile-work';
import type {
  AccountDevice,
  BackupEnrollment,
  Bridge,
  YubiEnrollment,
} from './bridge';
import type { StoreRef } from './model';
import type { DeviceLists } from './screens/device-model';
import { NO_DEVICES } from './screens/device-model';
import { MetadataRepository } from './metadata-repository';
import type { QuerySnapshot } from './metadata-repository';
import { useMetadataQuery, useMetadataRepository } from './query-hooks';
import { readRecoveryFor } from './query-read-recovery';
import type { CatalogReadRecovery } from './query-read-recovery';

export function metadataFreshness(states: readonly QuerySnapshot<unknown>[]) {
  const complete =
    states.length > 0 && states.every((state) => state.data !== undefined);
  const times = states.map((state) => state.lastSuccessAt);
  return {
    complete,
    stale: states.some((state) => state.error !== undefined),
    refreshing: states.some((state) => state.fetching),
    lastSuccessAt:
      times.length > 0 &&
      times.every((time): time is number => time !== undefined)
        ? Math.min(...times)
        : undefined,
  };
}

export const accountDeviceKey = (profile: string, store: StoreRef) =>
  ['account-devices', profile, store] as const;
export const profileEnrollmentKey = (profile: string) =>
  ['profile-enrollments', profile] as const;
interface AccountKeys {
  devices: AccountDevice[];
  backups: BackupEnrollment[];
  /**
   * Recovery-phrase availability became unknown because the agent or account state
   * changed after device data loaded. An empty list does not indicate confirmed
   * absence when this flag is set.
   */
  backupsUnavailable?: true;
}

/** Typed resource service; the repository owns data, lifetimes and subscriptions. */
export class DeviceCache {
  snapshot: (() => AgentSnapshot | undefined) | undefined;
  private queued = new AbortController();

  constructor(
    private readonly bridge: Bridge,
    now: () => number = Date.now,
    readonly repository = new MetadataRepository(now),
  ) {}

  clear(): void {
    this.queued.abort();
    this.queued = new AbortController();
    this.repository.clear();
  }
  retire(): void {
    this.queued.abort();
    this.repository.retire();
  }

  private read<T>(
    profile: string,
    key: readonly string[],
    work: () => Promise<T>,
  ) {
    const signal = this.queued.signal;
    const generation = this.repository.epoch;
    return scheduleProfileWork(this.bridge, profile, work, {
      key: JSON.stringify(key),
      owner: this,
      generation,
      signal,
      current: () =>
        !signal.aborted &&
        !this.repository.retired &&
        generation === this.repository.epoch,
      cancel: () => undefined,
      preemptible: false,
    });
  }

  /**
   * An account's devices and recovery phrases. Kept on the default freshness
   * window: the lists are the account's own sigchain, which another of this
   * user's devices can provision from or revoke on, so they move without a
   * local mutation to invalidate them.
   */
  account(profile: string, store: StoreRef) {
    return this.repository.query<AccountKeys>(
      accountDeviceKey(profile, store),
      () =>
        this.read(profile, accountDeviceKey(profile, store), async () => {
          const snapshot = this.snapshot?.();
          if (this.snapshot)
            requireWorkflow(snapshot, 'devices-list', {
              profile,
              account: snapshot?.accounts.find((entry) => entry.store === store)
                ?.alias,
            });
          const devices = await this.bridge.listAccountDevices(store);
          // Agent or account state may change after the initial availability check.
          // Return the loaded device data and mark recovery phrases unavailable instead of
          // failing the entire device query.
          if (
            this.snapshot &&
            !workflowAvailability(this.snapshot(), 'backup-list', { profile })
              .available
          )
            return { devices, backups: [], backupsUnavailable: true as const };
          const backups = await this.bridge.listBackupEnrollments(store);
          return { devices, backups };
        }),
    );
  }

  /** Security-key enrollments, on the default window for the same reason. */
  enrollments(profile: string) {
    return this.repository.query<YubiEnrollment[]>(
      profileEnrollmentKey(profile),
      () =>
        this.read(profile, profileEnrollmentKey(profile), () => {
          // If security-key listing becomes unavailable after the initial check, return
          // no enrollments instead of failing the entire device query.
          if (
            this.snapshot &&
            !workflowAvailability(this.snapshot(), 'yubi-list', { profile })
              .available
          )
            return Promise.resolve([]);
          return this.bridge.listYubiAccounts(profile);
        }),
    );
  }

  recoveryOptions(recovery?: CatalogReadRecovery) {
    return readRecoveryFor(this.repository).options(recovery);
  }

  peek(profile: string, store: StoreRef): DeviceLists | undefined {
    const account = this.account(profile, store).getSnapshot().data;
    const yubi = this.enrollments(profile).getSnapshot().data;
    return account && yubi
      ? Object.freeze({ ...account, yubi, cards: [] })
      : undefined;
  }

  async load(
    profile: string,
    store: StoreRef,
    recovery?: CatalogReadRecovery,
  ): Promise<DeviceLists> {
    const options = this.recoveryOptions(recovery);
    const [account, yubi] = await Promise.all([
      this.account(profile, store).load(options),
      this.enrollments(profile).load(options),
    ]);
    return Object.freeze({ ...account, yubi, cards: [] });
  }

  invalidateAccount(profile: string, store: StoreRef): void {
    this.repository.invalidate(accountDeviceKey(profile, store));
  }
  invalidateEnrollments(profile: string): void {
    this.repository.invalidate(profileEnrollmentKey(profile));
  }
}

export const DeviceCacheContext = createContext<DeviceCache | null>(null);
export const useDeviceCache = (): DeviceCache | null =>
  useContext(DeviceCacheContext);

/** Use one resource owner for profile pages and account pages in this access scope. */
export function useDeviceQueries(
  bridge: Bridge,
  snapshot?: AgentSnapshot,
): DeviceCache {
  const shared = useDeviceCache();
  const repository = useMetadataRepository(bridge, shared?.repository);
  const latest = useRef(snapshot);
  latest.current = snapshot;
  const cache = useMemo(
    () => shared ?? new DeviceCache(bridge, Date.now, repository),
    [bridge, repository, shared],
  );
  if (snapshot && (!shared || !cache.snapshot))
    cache.snapshot = () => latest.current;
  return cache;
}

/** Both account pages subscribe to these exact resources instead of copying their results. */
export function useDeviceMetadata({
  bridge,
  snapshot,
  profile,
  store,
  enabled,
  recovery,
  onError,
}: {
  bridge: Bridge;
  snapshot: AgentSnapshot;
  profile?: string;
  store?: StoreRef;
  enabled: boolean;
  recovery: CatalogReadRecovery;
  onError: (error: unknown) => void;
}) {
  const cache = useDeviceQueries(bridge, snapshot);
  const available = enabled && Boolean(profile && store);
  const account = available ? cache.account(profile!, store!) : null;
  const enrollments = available ? cache.enrollments(profile!) : null;
  const options = { ...cache.recoveryOptions(recovery), onError };
  const accountState = useMetadataQuery(account, options);
  const enrollmentState = useMetadataQuery(enrollments, options);
  const complete =
    accountState.data !== undefined && enrollmentState.data !== undefined;
  const failed =
    available &&
    !complete &&
    (accountState.error !== undefined || enrollmentState.error !== undefined);
  const lists = useMemo<DeviceLists>(
    () => ({
      devices: accountState.data?.devices ?? NO_DEVICES.devices,
      backups: accountState.data?.backups ?? NO_DEVICES.backups,
      yubi: enrollmentState.data ?? NO_DEVICES.yubi,
      cards: NO_DEVICES.cards,
    }),
    [accountState.data, enrollmentState.data],
  );
  // Retry only the account-device and enrollment queries represented by this
  // status. A forced catalog refresh would also invalidate unrelated queries
  // and probe hardware.
  const retry = useCallback((): void => {
    if (!available) return;
    cache.invalidateAccount(profile!, store!);
    cache.invalidateEnrollments(profile!);
  }, [available, cache, profile, store]);
  return {
    cache,
    lists,
    loading: available && !complete && !failed,
    failed,
    /** Whether recovery-phrase data could not be queried for the current account state. */
    backupsUnknown: accountState.data?.backupsUnavailable === true,
    freshness: metadataFreshness([accountState, enrollmentState]),
    retry,
  };
}
