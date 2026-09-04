/**
 * The lease world and what it makes listable — ported from
 * `wave6/shell.js:96-101, 243-249`.
 *
 * A lapsed compatibility lease stops **reads as well as writes** on that whole
 * server, so its items are not listed rather than merely
 * uneditable. It outranks everything else on that server and touches no other.
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

/** The fixture's leased server — the one `shell.js`'s `setLease` switches. */
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
  // `loadWorld` drops an unchecked server's items and accounts exactly as it
  // drops a lapsed one's, but this said `normal` for it — so the sidebar row
  // rendered undimmed and the page showed an ordinary empty list with no
  // explanation anywhere.
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
 * The description a page heading may show.
 *
 * A heading names a place. When something has gone wrong the page under it
 * already says so — the access takeover's notice, the roster's own warning —
 * so repeating "Setup incomplete" beside the title says it twice, and says
 * it in the one spot with no room to explain it or act on it. Every failing
 * reading is dropped here; the ordinary one is kept.
 */
export function storeHeadingDescription(world: World, store: Store): string {
  if (storeDescriptionState(world, store) !== 'normal') return '';
  if (groupDetailFailure(world, store.id, 'roster')) return '';
  if (groupDetailFailure(world, store.id, 'federation')) return '';
  return storeDescription(world, store);
}

/**
 * Whether a store lists at all: its server's lease must be fresh, and a team
 * whose summary reports inactive lists nothing.
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

/** Whether the renderer has one authenticated local author for this store. */
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

/** Whether this Mac's authenticated party admits this exact item's write role. */
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

/** Every listable item. Folders are not items — they are path chips. */
export function catalog(world: World): Item[] {
  return world.items.filter(
    (item) => item.kind !== 'Folder' && storeReadable(world, item.store),
  );
}

/** The stores a person picks between, in the order the sidebar shows them. */
export function listableStores(world: World): Store[] {
  return world.stores.filter((store) => storeReadable(world, store.id));
}

/**
 * The vault "Save in" starts on when the list itself does not name one.
 *
 * All Items spans every store, so a new item there has no store to inherit.
 * The answer is the first vault the chooser draws — navigation order, so it
 * is the same first row the sidebar shows — preferring one that can actually
 * take a new item, because starting on a store whose Create button is dead is
 * a worse first impression than starting one row lower. When nothing here can
 * be written to, the first store is still chosen: the sheet then says why in
 * its own words rather than opening on an empty chooser.
 */
export function defaultCreateStore(world: World): StoreRef | undefined {
  const order = storeNavigationOrder(world);
  const writable = order.find((store) => canCreateInStore(world, store.id));
  return (writable ?? order[0])?.id;
}

/**
 * The issue entries that apply to the world as it stands.
 *
 * The lapsed-lease entry is a consequence of the lease world, not a standing
 * fact, so it appears only while that world is lapsed.
 */
export function notesNow(world: World): Notification[] {
  return world.notifications.filter(
    (note) => note.id !== 'lease-acme' || world.leaseState === 'lapsed',
  );
}

/**
 * Put a server's compatibility lease into `state`, returning a new world.
 *
 * `shell.js` mutates a module global (`setLease`); this returns a fresh world
 * so the shell's lease switch is a state transition like any other and two
 * worlds can be held side by side in a test.
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
