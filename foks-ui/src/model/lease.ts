/**
 * Server lease evaluation and catalog visibility rules.
 *
 * A lapsed compatibility session lease blocks access to all items and stores
 * hosted on that server.
 */

import { storeNavigationOrder } from './order';
import { admissionActive, partiesOf, peopleGroups, storeOf } from './readers';
import { admits } from './roles';
import type {
  GroupDetailFailure,
  GroupDetailSource,
  Item,
  LeaseState,
  Notification,
  Server,
  Store,
  StoreRef,
  World,
} from './types';

/** Identifier for the sample server subject to session lease requirements. */
export const LEASED_SERVER_ID = 'acme';

export type SignedLeaseState = 'fresh' | 'lapsed' | 'unavailable';

export interface ServerLeaseStatus {
  leaseRequired: boolean;
  leaseExpiresAt: number | null;
}

/** Classify the signed Unix-seconds expiry without treating an absent fact as fresh. */
export function signedLeaseState(
  expiresAt: number | null,
  nowSeconds: number = Math.floor(Date.now() / 1000),
): SignedLeaseState {
  if (expiresAt === null) return 'unavailable';
  return expiresAt > nowSeconds ? 'fresh' : 'lapsed';
}

/** Classify access for a profile whose pinned protocol may not use leases. */
export function serverLeaseState(
  status: ServerLeaseStatus | undefined,
  nowSeconds: number = Math.floor(Date.now() / 1000),
): SignedLeaseState {
  if (!status) return 'unavailable';
  return status.leaseRequired
    ? signedLeaseState(status.leaseExpiresAt, nowSeconds)
    : 'fresh';
}

/** The server a store lives on. */
export function serverOf(world: World, ref: StoreRef): Server | undefined {
  const store = storeOf(world, ref);
  return store && world.servers.find((s) => s.id === store.server);
}

/** Whether this store's server has a lapsed check-in. */
export function leaseLapsed(world: World, ref: StoreRef): boolean {
  return serverOf(world, ref)?.state === 'lease-lapsed';
}

/** A protocol safety failure blocks a profile without claiming its lease lapsed. */
export function serverBlocked(world: World, ref: StoreRef): boolean {
  return serverOf(world, ref)?.state === 'blocked';
}

/** A server that has never been checked hides its stores until it is. */
export function serverNeverProbed(world: World, ref: StoreRef): boolean {
  return serverOf(world, ref)?.state === 'never-probed';
}

/** No usable signed expiry was available, so access fails closed without claiming lapse. */
export function serverLeaseUnavailable(world: World, ref: StoreRef): boolean {
  return serverOf(world, ref)?.state === 'lease-unavailable';
}

export type StoreDescriptionState =
  | 'normal'
  | 'blocked'
  | 'lease-unavailable'
  | 'lease-lapsed'
  | 'never-probed'
  | 'catalog-unavailable'
  | 'inactive';

/** The condition that replaces a store's ordinary sidebar/header description. */
export function storeDescriptionState(
  world: World,
  store: Store,
): StoreDescriptionState {
  if (serverBlocked(world, store.id)) return 'blocked';
  if (serverLeaseUnavailable(world, store.id)) return 'lease-unavailable';
  if (leaseLapsed(world, store.id)) return 'lease-lapsed';
  // Unprobed servers must not report normal status while catalog reads are pending.
  if (serverNeverProbed(world, store.id)) return 'never-probed';
  if (world.unavailableStores.includes(store.id)) return 'catalog-unavailable';
  if (store.kind === 'team' && !store.active) return 'inactive';
  return 'normal';
}

export function groupDetailFailure(
  world: World,
  store: StoreRef,
  source: GroupDetailSource,
): GroupDetailFailure | undefined {
  return world.groupDetailFailures.find(
    (failure) => failure.store === store && failure.source === source,
  );
}

/** The one description used for a store in both navigation and page headers. */
export function storeDescription(world: World, store: Store): string {
  const state = storeDescriptionState(world, store);
  if (state === 'inactive') return 'Setup incomplete';
  if (state !== 'normal') {
    return 'Connection error';
  }
  if (store.kind === 'account') return serverOf(world, store.id)?.name ?? '';
  if (groupDetailFailure(world, store.id, 'roster'))
    return 'Roster unavailable';
  if (groupDetailFailure(world, store.id, 'federation'))
    return 'Federation unavailable';
  return peopleGroups(partiesOf(world, store.id));
}

/**
 * Returns subtitle text for a store page header. Suppresses status error text
 * when errors are already surfaced by full-page notices in the body.
 */
export function storeHeadingDescription(world: World, store: Store): string {
  if (storeDescriptionState(world, store) !== 'normal') return '';
  if (groupDetailFailure(world, store.id, 'roster')) return '';
  if (groupDetailFailure(world, store.id, 'federation')) return '';
  return storeDescription(world, store);
}

/**
 * Returns whether a store is readable: its server lease must be valid,
 * and team stores must report an active status.
 */
export function storeReadable(world: World, ref: StoreRef): boolean {
  const store = storeOf(world, ref);
  if (!store) return false;
  return (
    !world.unavailableStores.includes(store.id) &&
    serverOf(world, ref)?.state === 'ok' &&
    (store.kind !== 'team' || store.active)
  );
}

/** Returns whether the current user has permission to create items in this store. */
export function canCreateInStore(world: World, ref: StoreRef): boolean {
  const store = storeOf(world, ref);
  if (!store || !storeReadable(world, store.id)) return false;
  if (store.kind === 'account') return true;
  const own = partiesOf(world, store.id).filter(
    (party) =>
      party.label === 'you' &&
      party.party_kind === 'user' &&
      party.locally_manageable &&
      admissionActive(world, party, store.id),
  );
  return own.length === 1;
}

/** Returns whether the local authenticated user has permission to edit the specified item. */
export function canChangeItem(world: World, item: Item): boolean {
  const store = storeOf(world, item.store);
  if (!store || !canCreateInStore(world, store.id)) return false;
  if (store.kind === 'account') return true;
  const own = partiesOf(world, store.id).filter(
    (party) =>
      party.label === 'you' &&
      party.party_kind === 'user' &&
      party.locally_manageable &&
      admissionActive(world, party, store.id),
  );
  return own.length === 1 && admits(own[0].destination_role, item.write);
}

/** Returns all accessible items from readable stores, excluding directory entries. */
export function catalog(world: World): Item[] {
  return world.items.filter(
    (item) => item.kind !== 'Folder' && storeReadable(world, item.store),
  );
}

/** Returns all readable stores in navigation display order. */
export function listableStores(world: World): Store[] {
  return world.stores.filter((store) => storeReadable(world, store.id));
}

/**
 * Determines the default store selection for new item creation when no store
 * context is active. Selects the first writable store in navigation order, or
 * falls back to the first available store if all are read-only.
 */
export function defaultCreateStore(world: World): StoreRef | undefined {
  const order = storeNavigationOrder(world);
  const writable = order.find((store) => canCreateInStore(world, store.id));
  return (writable ?? order[0])?.id;
}

/**
 * Filters active system notifications based on current server lease state.
 */
export function notesNow(world: World): Notification[] {
  return world.notifications.filter(
    (note) => note.id !== 'lease-acme' || world.leaseState === 'lapsed',
  );
}

/**
 * Put a server's compatibility lease into `state`, returning a new world.
 *
 * Returns a cloned World state with the updated lease state and server records.
 */
export function applyLease(
  world: World,
  state: Exclude<LeaseState, 'unavailable'>,
  serverId: string = LEASED_SERVER_ID,
): World {
  return {
    ...world,
    leaseState: state,
    servers: world.servers.map((server) =>
      server.id !== serverId
        ? server
        : {
            ...server,
            lease:
              state === 'lapsed'
                ? { state: 'lapsed', expires_in: null }
                : { state: 'fresh', expires_in: '12 d' },
            state: state === 'lapsed' ? 'lease-lapsed' : 'ok',
          },
    ),
  };
}
