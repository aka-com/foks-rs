/** Session-only metadata queries. Reader presence, PIN state and secrets are excluded. */
import { createContext, useContext, useMemo, useRef } from 'react';
import { requireWorkflow } from './model/workflow-availability';
import type { AgentSnapshot } from './model/types';
import { enqueueProfileWork } from './bridge';
import type {
  AccountDevice,
  BackupEnrollment,
  Bridge,
  YubiEnrollment,
} from './bridge';
import type { StoreRef } from './model';
import type { DeviceLists } from './screens/device-model';
import { NO_DEVICES } from './screens/device-model';
import { QueryRepository } from './query-repository';
import type { QuerySnapshot } from './query-repository';
import { useMetadataQuery, useQueryRepository } from './query-hooks';
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
}

/** Typed resource service; the repository owns data, lifetimes and subscriptions. */
export class DeviceCache {
  snapshot: (() => AgentSnapshot | undefined) | undefined;

  constructor(
    private readonly bridge: Bridge,
    now: () => number = Date.now,
    readonly repository = new QueryRepository(now),
  ) {}

  clear(): void {
    this.repository.clear();
  }
  retire(): void {
    this.repository.retire();
  }

  account(profile: string, store: StoreRef) {
    return this.repository.query<AccountKeys>(
      accountDeviceKey(profile, store),
      () =>
        enqueueProfileWork(this.bridge, profile, async () => {
          const snapshot = this.snapshot?.();
          if (this.snapshot)
            requireWorkflow(snapshot, 'devices-list', {
              profile,
              account: snapshot?.accounts.find((entry) => entry.store === store)
                ?.alias,
            });
          const devices = await this.bridge.listAccountDevices(store);
          if (this.snapshot)
            requireWorkflow(this.snapshot(), 'backup-list', { profile });
          const backups = await this.bridge.listBackupEnrollments(store);
          return { devices, backups };
        }),
    );
  }

  enrollments(profile: string) {
    return this.repository.query<YubiEnrollment[]>(
      profileEnrollmentKey(profile),
      () =>
        enqueueProfileWork(this.bridge, profile, () => {
          if (this.snapshot)
            requireWorkflow(this.snapshot(), 'yubi-list', { profile });
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
  const repository = useQueryRepository(bridge, shared?.repository);
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
  return {
    cache,
    lists,
    loading: available && !complete && !failed,
    failed,
    freshness: metadataFreshness([accountState, enrollmentState]),
  };
}
