/**
 * What this session has learned about each account's keys.
 *
 * The shell maintains recovery-phrase and key-count facts from its shared
 * device metadata queries.
 * Pairing offers remain explicit: the agent does not publish their creation
 * or expiry, so only the Pair sheet reports offers it starts or resumes.
 */

import type { AccountDevice, BackupEnrollment, Bridge } from '../bridge';
import type { AgentSnapshot, StoreRef } from '../model';
import { accountStores } from '../model';
import { deviceIsCard } from './device-model';
import {
  recoveryPhraseTestDue,
  recoveryPhraseTestStatus,
} from '../recovery-phrase-tests';
import type { recoveryPhraseTests } from '../recovery-phrase-tests';

export interface DeviceAlertSnapshot {
  /**
   * Each account's recovery-phrase enrollments, as the keys list answered; an
   * empty list means the account has none.
   */
  phraseLists: ReadonlyMap<StoreRef, readonly BackupEnrollment[]>;
  /**
   * Each account's authenticated keys, as the keys list answered: how many
   * there are, and how many of them are keys on a hardware card. An account
   * missing from the map has not had its keys read this session.
   */
  keyCounts: ReadonlyMap<StoreRef, AccountKeyCount>;
  offers: ReadonlySet<StoreRef>;
}

/** How many authenticated keys an account has, and how many are on a card. */
export interface AccountKeyCount {
  devices: number;
  cards: number;
}

const EMPTY: DeviceAlertSnapshot = {
  phraseLists: new Map(),
  keyCounts: new Map(),
  offers: new Set(),
};

export class DeviceAlertRegistry {
  private snapshot: DeviceAlertSnapshot = EMPTY;
  private listeners = new Set<() => void>();

  getSnapshot = (): DeviceAlertSnapshot => this.snapshot;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  /**
   * Records the recovery-phrase enrollments the keys list answered for an
   * account: whether it has one, and which, so the rail can also report
   * phrases that are due for a test.
   */
  reportRecoveryPhrase(
    store: StoreRef,
    backups: readonly BackupEnrollment[],
  ): void {
    const listed = this.snapshot.phraseLists.get(store);
    if (
      listed !== undefined &&
      listed.length === backups.length &&
      listed.every(
        (entry, index) =>
          entry.backupId === backups[index]?.backupId &&
          entry.backupAlias === backups[index]?.backupAlias,
      )
    )
      return;
    const phraseLists = new Map(this.snapshot.phraseLists);
    phraseLists.set(store, [...backups]);
    this.snapshot = { ...this.snapshot, phraseLists };
    this.publish();
  }

  forgetRecoveryPhrase(store: StoreRef): void {
    if (!this.snapshot.phraseLists.has(store)) return;
    const phraseLists = new Map(this.snapshot.phraseLists);
    phraseLists.delete(store);
    this.snapshot = { ...this.snapshot, phraseLists };
    this.publish();
  }

  /**
   * Records how many authenticated keys the keys list answered for an
   * account, so a single-device account can be told apart from one with a
   * second device or a hardware key.
   */
  reportAccountKeys(store: StoreRef, devices: readonly AccountDevice[]): void {
    const count: AccountKeyCount = {
      devices: devices.length,
      cards: devices.filter(deviceIsCard).length,
    };
    const listed = this.snapshot.keyCounts.get(store);
    if (
      listed !== undefined &&
      listed.devices === count.devices &&
      listed.cards === count.cards
    )
      return;
    const keyCounts = new Map(this.snapshot.keyCounts);
    keyCounts.set(store, count);
    this.snapshot = { ...this.snapshot, keyCounts };
    this.publish();
  }

  forgetAccountKeys(store: StoreRef): void {
    if (!this.snapshot.keyCounts.has(store)) return;
    const keyCounts = new Map(this.snapshot.keyCounts);
    keyCounts.delete(store);
    this.snapshot = { ...this.snapshot, keyCounts };
    this.publish();
  }

  /** Records whether the agent is holding an open pairing offer for an account. */
  reportPairingOffer(store: StoreRef, open: boolean): void {
    if (this.snapshot.offers.has(store) === open) return;
    const offers = new Set(this.snapshot.offers);
    if (open) offers.add(store);
    else offers.delete(store);
    this.snapshot = { ...this.snapshot, offers };
    this.publish();
  }

  private publish(): void {
    for (const listener of this.listeners) listener();
  }
}

const registries = new WeakMap<Bridge, DeviceAlertRegistry>();

/** One registry per bridge, so separate windows and tests never share state. */
export function deviceAlertRegistry(bridge: Bridge): DeviceAlertRegistry {
  let registry = registries.get(bridge);
  if (!registry) {
    registry = new DeviceAlertRegistry();
    registries.set(bridge, registry);
  }
  return registry;
}

/**
 * One reason the Devices tab asks for attention, for one account: an open
 * pairing offer, no recovery phrase, or a recovery phrase that was never
 * tested on this device or is overdue for a test and not snoozed. An account
 * whose keys have not been loaded this session gives no reason either way,
 * the same way a team never visited says nothing about its requests.
 */
export type DeviceAlertReason =
  | { kind: 'pairing-offer'; store: StoreRef }
  | { kind: 'no-recovery-phrase'; store: StoreRef }
  | {
      kind: 'recovery-phrase-test';
      store: StoreRef;
      backup: BackupEnrollment;
      /** True when the phrase was never tested; false when a test is due. */
      never: boolean;
    };

/** Every reason the Devices tab asks for attention, in account order. */
export function deviceAlertReasons(
  snapshot: AgentSnapshot,
  state: DeviceAlertSnapshot,
  tests: ReturnType<typeof recoveryPhraseTests> = {},
  now = Date.now(),
): DeviceAlertReason[] {
  const reasons: DeviceAlertReason[] = [];
  for (const store of accountStores(snapshot)) {
    if (state.offers.has(store.id))
      reasons.push({ kind: 'pairing-offer', store: store.id });
    const backups = state.phraseLists.get(store.id);
    if (backups?.length === 0)
      reasons.push({ kind: 'no-recovery-phrase', store: store.id });
    for (const backup of backups ?? []) {
      const status = recoveryPhraseTestStatus(tests, backup.backupId, now);
      if (!recoveryPhraseTestDue(status)) continue;
      reasons.push({
        kind: 'recovery-phrase-test',
        store: store.id,
        backup,
        never: status.state === 'never',
      });
    }
  }
  return reasons;
}

/**
 * The Devices tab's rail dot: on while `deviceAlertReasons` gives any reason.
 * The description lists each one.
 */
export function devicesAlertSummary(
  snapshot: AgentSnapshot,
  state: DeviceAlertSnapshot,
  tests: ReturnType<typeof recoveryPhraseTests> = {},
  now = Date.now(),
): { description: string } | null {
  const found = deviceAlertReasons(snapshot, state, tests, now);
  const phrases = found.flatMap((reason) =>
    reason.kind !== 'recovery-phrase-test'
      ? []
      : [
          reason.never
            ? `recovery phrase ${reason.backup.backupAlias} has never been tested`
            : `recovery phrase ${reason.backup.backupAlias} is due for a test`,
        ],
  );
  const reasons = [
    ...(found.some((reason) => reason.kind === 'pairing-offer')
      ? ['a pairing offer is open']
      : []),
    ...(found.some((reason) => reason.kind === 'no-recovery-phrase')
      ? ['an account has no recovery phrase']
      : []),
    ...phrases,
  ];
  const last = reasons.pop();
  if (last === undefined) return null;
  const sentence = reasons.length ? `${reasons.join(', ')}, and ${last}` : last;
  return { description: sentence.charAt(0).toUpperCase() + sentence.slice(1) };
}
