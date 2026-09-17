/** Session-only metadata cache. Reader presence, PIN state and secrets are excluded. */
import { createContext, useContext } from 'react';
import { enqueueProfileWork } from './bridge';
import type {
  AccountDevice,
  BackupEnrollment,
  Bridge,
  YubiEnrollment,
} from './bridge';
import type { StoreRef } from './model';
import type { DeviceLists } from './screens/device-model';

interface Entry<T> {
  value?: T;
  updated: number;
  pending?: Promise<T>;
}
const FRESH_FOR = 60_000;

export class DeviceCache {
  private accounts = new Map<
    string,
    Entry<{ devices: AccountDevice[]; backups: BackupEnrollment[] }>
  >();
  private profiles = new Map<string, Entry<YubiEnrollment[]>>();
  constructor(
    private bridge: Bridge,
    private now: () => number = Date.now,
  ) {}

  /** Drop metadata and detach in-flight results, without cancelling native reads. */
  clear(): void {
    this.accounts.clear();
    this.profiles.clear();
  }

  peek(profile: string, store: StoreRef): DeviceLists | undefined {
    const account = this.accounts.get(JSON.stringify([profile, store]))?.value;
    const yubi = this.profiles.get(profile)?.value;
    return account && yubi ? { ...account, yubi, cards: [] } : undefined;
  }

  private read<T>(
    entries: Map<string, Entry<T>>,
    key: string,
    load: () => Promise<T>,
  ): Promise<T> {
    let entry = entries.get(key);
    if (entry?.pending) return entry.pending;
    if (entry?.value !== undefined && this.now() - entry.updated < FRESH_FOR)
      return Promise.resolve(entry.value);
    if (!entry) {
      entry = { updated: 0 };
      entries.set(key, entry);
    }
    const target = entry;
    const pending = load()
      .then((value) => {
        if (entries.get(key) === target) {
          target.value = value;
          target.updated = this.now();
        }
        return value;
      })
      .finally(() => {
        if (target.pending === pending) target.pending = undefined;
      });
    target.pending = pending;
    return pending;
  }

  async load(profile: string, store: StoreRef): Promise<DeviceLists> {
    const account = this.read(
      this.accounts,
      JSON.stringify([profile, store]),
      () =>
        enqueueProfileWork(this.bridge, profile, async () => ({
          devices: await this.bridge.listAccountDevices(store),
          backups: await this.bridge.listBackupEnrollments(store),
        })),
    );
    const enrollments = this.read(this.profiles, profile, () =>
      enqueueProfileWork(this.bridge, profile, () =>
        this.bridge.listYubiAccounts(profile),
      ),
    );
    const [rows, yubi] = await Promise.all([account, enrollments]);
    return { ...rows, yubi, cards: [] };
  }
}

export const DeviceCacheContext = createContext<DeviceCache | null>(null);
export const useDeviceCache = (): DeviceCache | null =>
  useContext(DeviceCacheContext);
