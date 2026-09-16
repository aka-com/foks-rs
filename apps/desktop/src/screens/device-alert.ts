/**
 * What this session has learned about each account's keys.
 *
 * `hasPaperKey` mirrors the backup list the People and Devices pages already
 * load for their own rows — recording it here costs no new request, only a
 * report from a fetch already made. A pairing offer has no such fact to
 * mirror: the agent holds it invisibly and reports neither when it was made
 * nor when it expires (see `PairSheet`'s own note on this), so the only way
 * to know is the one place that ever asks — the Pair sheet's Start or Resume
 * offer, on this device. Both are recorded the same way, from the page that
 * already learned them, rather than by asking the agent again from the rail.
 */

import type { Bridge } from '../bridge';
import type { AgentSnapshot, StoreRef } from '../model';
import { accountStores } from '../model';

export interface DeviceAlertSnapshot {
  paperKeys: ReadonlyMap<StoreRef, boolean>;
  offers: ReadonlySet<StoreRef>;
}

const EMPTY: DeviceAlertSnapshot = {
  paperKeys: new Map(),
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

  /** Records whether an account has a paper key, as the keys list answers it. */
  reportPaperKey(store: StoreRef, hasPaperKey: boolean): void {
    if (this.snapshot.paperKeys.get(store) === hasPaperKey) return;
    const paperKeys = new Map(this.snapshot.paperKeys);
    paperKeys.set(store, hasPaperKey);
    this.snapshot = { ...this.snapshot, paperKeys };
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
 * pairing offer or is known to have no paper key. An account whose keys have
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
    if (state.paperKeys.get(store.id) === false) missingKey = true;
  }
  if (!offer && !missingKey) return null;
  const description =
    offer && missingKey
      ? 'A pairing offer is open, and an account has no paper key'
      : offer
        ? 'A pairing offer is open'
        : 'An account has no paper key';
  return { description };
}
