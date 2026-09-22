/**
 * What this session has learned about each account's keys.
 *
 * The shell maintains recovery-phrase facts from its shared device metadata queries.
 * Pairing offers remain explicit: the agent does not publish their creation
 * or expiry, so only the Pair sheet reports offers it starts or resumes.
 */

import type { Bridge } from '../bridge';
import type { AgentSnapshot, StoreRef } from '../model';
import { accountStores } from '../model';

export interface DeviceAlertSnapshot {
  recoveryPhrases: ReadonlyMap<StoreRef, boolean>;
  offers: ReadonlySet<StoreRef>;
}

const EMPTY: DeviceAlertSnapshot = {
  recoveryPhrases: new Map(),
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

  /** Records whether an account has a recovery phrase, as the keys list answers it. */
  reportRecoveryPhrase(store: StoreRef, hasRecoveryPhrase: boolean): void {
    if (this.snapshot.recoveryPhrases.get(store) === hasRecoveryPhrase) return;
    const recoveryPhrases = new Map(this.snapshot.recoveryPhrases);
    recoveryPhrases.set(store, hasRecoveryPhrase);
    this.snapshot = { ...this.snapshot, recoveryPhrases };
    this.publish();
  }

  forgetRecoveryPhrase(store: StoreRef): void {
    if (!this.snapshot.recoveryPhrases.has(store)) return;
    const recoveryPhrases = new Map(this.snapshot.recoveryPhrases);
    recoveryPhrases.delete(store);
    this.snapshot = { ...this.snapshot, recoveryPhrases };
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
 * pairing offer or is known to have no recovery phrase. An account whose keys have
 * not been loaded this session says nothing either way, the same way a team
 * never visited says nothing about its requests.
 */
export function devicesAlertSummary(
  snapshot: AgentSnapshot,
  state: DeviceAlertSnapshot,
): { description: string } | null {
  let offer = false;
  let missingKey = false;
  for (const store of accountStores(snapshot)) {
    if (state.offers.has(store.id)) offer = true;
    if (state.recoveryPhrases.get(store.id) === false) missingKey = true;
  }
  if (!offer && !missingKey) return null;
  const description =
    offer && missingKey
      ? 'A pairing offer is open, and an account has no recovery phrase'
      : offer
        ? 'A pairing offer is open'
        : 'An account has no recovery phrase';
  return { description };
}
