/** Shell-owned subscriptions to the same device queries used by account pages. */
import { createContext, useContext } from 'react';
import { isAgentReadinessError, normalizeCommandError } from './bridge';
import type { DeviceCache } from './device-cache';
import type { MetadataQuery } from './metadata-repository';
import { accountStores, type AgentSnapshot } from './model';
import { workflowAvailability } from './model/workflow-availability';
import type { DeviceAlertRegistry } from './screens/device-alert';

type Query = Pick<
  MetadataQuery<unknown>,
  'getSnapshot' | 'subscribe' | 'load' | 'isDueSubscribed' | 'invalidate'
>;
interface Entry {
  profile: string;
  store?: string;
  query: Query;
  invalidation: number;
  requested: boolean;
  release(): void;
}
export interface DeviceMetadataStatus {
  profile: string;
  total: number;
  ready: number;
  unavailable: number;
  refreshing: boolean;
  failed: boolean;
  error?: string;
  lastSuccessAt?: number;
}

export class DeviceMetadata {
  private entries = new Map<Query, Entry>();
  private statuses: readonly DeviceMetadataStatus[] = [];
  private listeners = new Set<() => void>();
  private snapshot?: AgentSnapshot;
  private running?: Promise<void>;
  private queued = false;
  private active = false;
  private allowed: () => boolean = () => false;
  private report: (error: unknown) => void = () => undefined;

  constructor(
    readonly cache: DeviceCache,
    private alerts: DeviceAlertRegistry,
  ) {}

  getSnapshot = (): readonly DeviceMetadataStatus[] => this.statuses;
  isAvailable = (): boolean => this.active && this.allowed();
  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  update(
    snapshot: AgentSnapshot,
    allowed: () => boolean,
    report: (error: unknown) => void = () => undefined,
  ): void {
    this.snapshot = snapshot;
    this.allowed = allowed;
    this.report = report;
    this.active = true;
    const desired = new Map<Query, { profile: string; store?: string }>();
    if (allowed()) {
      for (const store of accountStores(snapshot)) {
        if (
          !workflowAvailability(snapshot, 'devices-list', {
            profile: store.server,
            account: store.account,
          }).available
        )
          continue;
        desired.set(this.cache.account(store.server, store.id), {
          profile: store.server,
          store: store.id,
        });
        desired.set(this.cache.enrollments(store.server), {
          profile: store.server,
        });
      }
    }
    for (const [query, entry] of this.entries) {
      if (desired.has(query)) continue;
      entry.release();
      if (entry.store) this.alerts.forgetPaperKey(entry.store);
      this.entries.delete(query);
    }
    for (const [query, scope] of desired) {
      if (this.entries.has(query)) continue;
      const entry: Entry = {
        ...scope,
        query,
        invalidation: query.getSnapshot().invalidation,
        requested: false,
        release: () => undefined,
      };
      entry.release = query.subscribe(() => {
        const invalidation = query.getSnapshot().invalidation;
        if (invalidation !== entry.invalidation) {
          entry.invalidation = invalidation;
          entry.requested = true;
          this.schedule();
        }
        this.publish();
      });
      this.entries.set(query, entry);
    }
    this.publish();
    this.schedule();
  }

  private schedule(): void {
    if (this.queued || !this.active) return;
    this.queued = true;
    queueMicrotask(() => {
      this.queued = false;
      void this.reconcile();
    });
  }

  /** Two workers bound startup and refresh work across all profiles. */
  reconcile = (): Promise<void> => {
    if (this.running) return this.running;
    if (!this.active || !this.allowed()) return Promise.resolve();
    const worker = async () => {
      while (this.active && this.allowed()) {
        const entry = [...this.entries.values()].find(
          (candidate) =>
            !claimed.has(candidate) &&
            (candidate.requested || candidate.query.isDueSubscribed()),
        );
        if (!entry) return;
        claimed.add(entry);
        const invalidation = entry.invalidation;
        try {
          await entry.query.load();
        } catch (error) {
          if (
            this.active &&
            this.allowed() &&
            this.entries.get(entry.query) === entry &&
            isAgentReadinessError(normalizeCommandError(error))
          )
            this.report(error);
        } finally {
          if (entry.invalidation === invalidation) entry.requested = false;
          this.publish();
        }
      }
    };
    const claimed = new Set<Entry>();
    const pending = Promise.resolve()
      .then(() => Promise.all([worker(), worker()]))
      .then(() => undefined)
      .finally(() => {
        if (this.running === pending) this.running = undefined;
        if (
          this.active &&
          this.allowed() &&
          [...this.entries.values()].some(
            (entry) => entry.requested || entry.query.isDueSubscribed(),
          )
        )
          this.schedule();
      });
    this.running = pending;
    return pending;
  };

  retry = (profile: string): void => {
    for (const entry of this.entries.values())
      if (entry.profile === profile) entry.query.invalidate();
    this.schedule();
  };

  /** Includes trailing reads requested while an older generation was running. */
  async settle(): Promise<void> {
    do {
      await this.reconcile();
    } while (
      this.active &&
      this.allowed() &&
      [...this.entries.values()].some(
        (entry) => entry.requested || entry.query.isDueSubscribed(),
      )
    );
  }

  stop(): void {
    this.active = false;
    for (const entry of this.entries.values()) {
      entry.release();
      if (entry.store) this.alerts.forgetPaperKey(entry.store);
    }
    this.entries.clear();
    this.snapshot = undefined;
    this.publish();
  }

  private publish(): void {
    const snapshot = this.snapshot;
    this.statuses = snapshot
      ? snapshot.servers.map((server) => {
          const accounts = accountStores(snapshot).filter(
            (store) => store.server === server.profileName,
          );
          const entries = [...this.entries.values()].filter(
            (entry) => entry.profile === server.profileName,
          );
          let ready = 0;
          let unavailable = 0;
          const enrollment = entries
            .find((entry) => !entry.store)
            ?.query.getSnapshot();
          for (const store of accounts) {
            const entry = entries.find((entry) => entry.store === store.id);
            const state = entry
              ? this.cache.account(server.profileName, store.id).getSnapshot()
              : undefined;
            if (!entry || state?.data?.backupsUnavailable) unavailable++;
            if (
              state?.data &&
              !state.data.backupsUnavailable &&
              enrollment?.data !== undefined
            )
              ready++;
            if (state?.data && !state.error && !state.data.backupsUnavailable)
              this.alerts.reportPaperKey(
                store.id,
                state.data.backups.length > 0,
              );
            else this.alerts.forgetPaperKey(store.id);
          }
          const states = entries.map((entry) => entry.query.getSnapshot());
          const error = states.find(
            (state) => state.error !== undefined,
          )?.error;
          const times = states.map((state) => state.lastSuccessAt);
          return {
            profile: server.profileName,
            total: accounts.length,
            ready,
            unavailable,
            refreshing: entries.some(
              (entry) =>
                entry.requested ||
                (entry.query.getSnapshot().data === undefined &&
                  !entry.query.getSnapshot().error),
            ),
            failed:
              error !== undefined ||
              states.some((state) =>
                Boolean(
                  (state.data as { backupsUnavailable?: boolean } | undefined)
                    ?.backupsUnavailable,
                ),
              ),
            ...(error === undefined
              ? {}
              : { error: normalizeCommandError(error).message }),
            lastSuccessAt:
              times.length > 0 &&
              times.every((time): time is number => time !== undefined)
                ? Math.min(...times)
                : undefined,
          };
        })
      : [];
    for (const listener of this.listeners) listener();
  }
}

export const DeviceMetadataContext = createContext<DeviceMetadata | null>(null);
export const useShellDeviceMetadata = () => useContext(DeviceMetadataContext);
export const EMPTY_DEVICE_METADATA: readonly DeviceMetadataStatus[] = [];
export const emptyDeviceMetadata = () => EMPTY_DEVICE_METADATA;
export const noDeviceSubscription = () => () => undefined;
export const isDeviceQuery = (key: readonly string[]) =>
  key[0] === 'account-devices' || key[0] === 'profile-enrollments';
