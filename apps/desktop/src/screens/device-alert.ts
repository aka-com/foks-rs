/**
 * What this session has learned about each account's keys.
 *
 * The shell maintains recovery-phrase facts from its shared device metadata queries.
 * Pairing offers remain explicit: the agent does not publish their creation
 * or expiry, so only the Pair sheet reports offers it starts or resumes.
 */

import type { BackupEnrollment, Bridge } from '../bridge';
import type { AgentSnapshot, StoreRef } from '../model';
import { accountStores } from '../model';
import {
  recoveryPhraseTestDue,
  recoveryPhraseTestStatus,
} from '../recovery-phrase-tests';
import type { recoveryPhraseTests } from '../recovery-phrase-tests';

export interface DeviceAlertSnapshot {
  recoveryPhrases: ReadonlyMap<StoreRef, boolean>;
  /** Each account's recovery-phrase enrollments, as the keys list answered. */
  phraseLists: ReadonlyMap<StoreRef, readonly BackupEnrollment[]>;
  offers: ReadonlySet<StoreRef>;
}

const EMPTY: DeviceAlertSnapshot = {
  recoveryPhrases: new Map(),
  phraseLists: new Map(),
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
    const hasRecoveryPhrase = backups.length > 0;
    const listed = this.snapshot.phraseLists.get(store);
    if (
      this.snapshot.recoveryPhrases.get(store) === hasRecoveryPhrase &&
      listed !== undefined &&
      listed.length === backups.length &&
      listed.every(
        (entry, index) =>
          entry.backupId === backups[index]?.backupId &&
          entry.backupAlias === backups[index]?.backupAlias,
      )
    )
      return;
    const recoveryPhrases = new Map(this.snapshot.recoveryPhrases);
    recoveryPhrases.set(store, hasRecoveryPhrase);
    const phraseLists = new Map(this.snapshot.phraseLists);
    phraseLists.set(store, [...backups]);
    this.snapshot = { ...this.snapshot, recoveryPhrases, phraseLists };
    this.publish();
  }

  forgetRecoveryPhrase(store: StoreRef): void {
    if (
      !this.snapshot.recoveryPhrases.has(store) &&
      !this.snapshot.phraseLists.has(store)
    )
      return;
    const recoveryPhrases = new Map(this.snapshot.recoveryPhrases);
    recoveryPhrases.delete(store);
    const phraseLists = new Map(this.snapshot.phraseLists);
    phraseLists.delete(store);
    this.snapshot = { ...this.snapshot, recoveryPhrases, phraseLists };
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
 * The Devices tab's rail dot: on while any account this Mac holds has an open
 * pairing offer, is known to have no recovery phrase, or has a recovery phrase
 * that was never tested on this device or is overdue for a test and not
 * snoozed. The description lists each reason. An account whose keys have not
 * been loaded this session says nothing either way, the same way a team never
 * visited says nothing about its requests.
 */
export function devicesAlertSummary(
  snapshot: AgentSnapshot,
  state: DeviceAlertSnapshot,
  tests: ReturnType<typeof recoveryPhraseTests> = {},
  now = Date.now(),
): { description: string } | null {
  let offer = false;
  let missingKey = false;
  const phrases: string[] = [];
  for (const store of accountStores(snapshot)) {
    if (state.offers.has(store.id)) offer = true;
    if (state.recoveryPhrases.get(store.id) === false) missingKey = true;
    for (const backup of state.phraseLists.get(store.id) ?? []) {
      const status = recoveryPhraseTestStatus(tests, backup.backupId, now);
      if (!recoveryPhraseTestDue(status)) continue;
      phrases.push(
        status.state === 'never'
          ? `recovery phrase ${backup.backupAlias} has never been tested`
          : `recovery phrase ${backup.backupAlias} is due for a test`,
      );
    }
  }
  const reasons = [
    ...(offer ? ['a pairing offer is open'] : []),
    ...(missingKey ? ['an account has no recovery phrase'] : []),
    ...phrases,
  ];
  const last = reasons.pop();
  if (last === undefined) return null;
  const sentence = reasons.length ? `${reasons.join(', ')}, and ${last}` : last;
  return { description: sentence.charAt(0).toUpperCase() + sentence.slice(1) };
}
