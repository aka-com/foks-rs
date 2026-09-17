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
  CompatibilityLease,
  GroupDetailFailure,
  GroupDetailSource,
  Item,
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

export type AvailabilityReason =
  | 'agent-unavailable'
  | 'verification-required'
  | 'verification-failed'
  | 'schema-incompatible'
  | 'import-verification-required'
  | 'server-status-unavailable'
  | 'check-in-unavailable'
  | 'check-in-expired'
  | 'vault-unavailable'
  | 'setup-incomplete';

export type Availability =
  | { available: true }
  | { available: false; reason: AvailabilityReason };

export interface AvailabilityOptions {
  nowSeconds?: number;
  agentReady?: boolean;
  catalogReady?: boolean;
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

/** The server hosting a store. */
export function serverOf(world: World, ref: StoreRef): Server | undefined {
  const store = storeOf(world, ref);
  return store && world.servers.find((s) => s.id === store.server);
}

/** Derives server access from independent facts at the supplied clock instant. */
export function serverAvailability(
  world: World,
  server: Server,
  options: AvailabilityOptions = {},
): Availability {
  if (world.agent.state !== 'ready')
    return { available: false, reason: 'agent-unavailable' };
  return serverFactAvailability(
    server,
    world.observedExpiredLeases,
    options,
  );
}

export function serverFactAvailability(
  server: Server,
  observedExpiredLeases: readonly { profile: string; expiresAt: number }[],
  options: AvailabilityOptions = {},
): Availability {
  if (options.agentReady === false || options.catalogReady === false)
    return { available: false, reason: 'agent-unavailable' };
  return (
    serverSecurityAvailability(server) ??
    serverOperationalAvailability(server, observedExpiredLeases, options)
  );
}

function serverSecurityAvailability(server: Server): Availability | null {
  if (server.trust.status === 'blocked')
    return { available: false, reason: 'verification-failed' };
  if (server.trust.status === 'unprobed')
    return { available: false, reason: 'verification-required' };
  if (server.restrictions.some((entry) => entry.kind === 'schema-incompatible'))
    return { available: false, reason: 'schema-incompatible' };
  if (
    server.restrictions.some(
      (entry) => entry.kind === 'import-verification-required',
    )
  )
    return { available: false, reason: 'import-verification-required' };
  return null;
}

function serverOperationalAvailability(
  server: Server,
  observedExpiredLeases: readonly { profile: string; expiresAt: number }[],
  options: AvailabilityOptions,
): Availability {
  const lease = server.compatibility;
  if (lease.status === 'requirement-unknown')
    return { available: false, reason: 'server-status-unavailable' };
  if (lease.status === 'required-unavailable')
    return { available: false, reason: 'check-in-unavailable' };
  if (lease.status === 'required') {
    const now = options.nowSeconds ?? Math.floor(Date.now() / 1000);
    if (
      now >= lease.expiresAt ||
      observedExpiredLeases.some(
        (entry) =>
          entry.profile === server.id && entry.expiresAt >= lease.expiresAt,
      )
    )
      return { available: false, reason: 'check-in-expired' };
  }
  if (server.passiveStatus.status === 'failed')
    return { available: false, reason: 'server-status-unavailable' };
  return { available: true };
}

/** The single access decision shared by navigation, views, and dispatch. */
export function storeAvailability(
  world: World,
  store: Store,
  options: AvailabilityOptions = {},
): Availability {
  if (
    world.agent.state !== 'ready' ||
    options.agentReady === false ||
    options.catalogReady === false
  )
    return { available: false, reason: 'agent-unavailable' };
  const server = world.servers.find((candidate) => candidate.id === store.server);
  if (!server) return { available: false, reason: 'vault-unavailable' };
  const serverSecurity = serverSecurityAvailability(server);
  if (serverSecurity) return serverSecurity;
  const inventory = world.storeInventory.find((entry) => entry.store === store.id);
  if (inventory?.restrictions.some((entry) => entry.kind === 'schema-incompatible'))
    return { available: false, reason: 'schema-incompatible' };
  if (
    inventory?.restrictions.some(
      (entry) => entry.kind === 'import-verification-required',
    )
  )
    return { available: false, reason: 'import-verification-required' };
  const serverAccess = serverOperationalAvailability(
    server,
    world.observedExpiredLeases,
    options,
  );
  if (!serverAccess.available) return serverAccess;
  if (!inventory || inventory.status === 'unavailable')
    return { available: false, reason: 'vault-unavailable' };
  if (store.kind === 'team' && !store.active)
    return { available: false, reason: 'setup-incomplete' };
  return { available: true };
}

export function profileInventoryComplete(
  world: World,
  kind: 'profiles' | 'accounts' | 'teams',
): boolean {
  if (kind === 'profiles') return world.profileInventoryStatus === 'complete';
  if (world.profileInventoryStatus !== 'complete') return false;
  return world.catalogProfiles.every(
    (profile) =>
      world.profileInventory.find((entry) => entry.profile === profile)?.[
        kind
      ] === 'complete',
  );
}

/** Whether this store's server has a lapsed check-in. */
export function leaseLapsed(world: World, ref: StoreRef): boolean {
  const store = storeOf(world, ref);
  if (!store) return false;
  const availability = storeAvailability(world, store);
  return !availability.available && availability.reason === 'check-in-expired';
}

/** A protocol safety failure blocks a profile without claiming its lease lapsed. */
export function serverBlocked(world: World, ref: StoreRef): boolean {
  return serverOf(world, ref)?.trust.status === 'blocked';
}

/** A server that has never been checked hides its stores until it is. */
export function serverNeverProbed(world: World, ref: StoreRef): boolean {
  return serverOf(world, ref)?.trust.status === 'unprobed';
}

/** No usable signed expiry was available, so access fails closed without claiming lapse. */
export function serverLeaseUnavailable(world: World, ref: StoreRef): boolean {
  const lease = serverOf(world, ref)?.compatibility;
  return lease?.status === 'required-unavailable' || lease?.status === 'requirement-unknown';
}

export type StoreDescriptionState =
  | 'normal'
  | AvailabilityReason;

/** The condition that replaces a store's ordinary sidebar/header description. */
export function storeDescriptionState(
  world: World,
  store: Store,
): StoreDescriptionState {
  const availability = storeAvailability(world, store);
  return availability.available ? 'normal' : availability.reason;
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
  if (state === 'setup-incomplete') return 'Setup incomplete';
  if (state === 'verification-required') return 'Verification required';
  if (state === 'verification-failed') return 'Verification failed';
  if (state === 'schema-incompatible') return 'Schema incompatible';
  if (state === 'import-verification-required') return 'Verification required';
  if (state === 'check-in-expired') return 'Check-in expired';
  if (state === 'check-in-unavailable') return 'Check-in unavailable';
  if (state === 'server-status-unavailable') return 'Server status unavailable';
  if (state === 'vault-unavailable') return 'Vault unavailable';
  if (state === 'agent-unavailable') return 'Service unavailable';
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
export function storeReadable(
  world: World,
  ref: StoreRef,
  options: AvailabilityOptions = {},
): boolean {
  const store = storeOf(world, ref);
  return Boolean(store && storeAvailability(world, store, options).available);
}

export function serverChatAvailable(
  world: World,
  server: Server,
  options: AvailabilityOptions = {},
): boolean {
  return server.capabilities.chat && serverAvailability(world, server, options).available;
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
  return world.notifications.filter((note) => {
    if (note.id !== 'lease-acme') return true;
    const server = world.servers.find((entry) => entry.id === LEASED_SERVER_ID);
    if (!server) return false;
    const availability = serverAvailability(world, server);
    return !availability.available && availability.reason === 'check-in-expired';
  });
}

/**
 * Put a server's compatibility lease into `state`, returning a new world.
 *
 * Returns a cloned World state with the updated lease state and server records.
 */
export function applyLease(
  world: World,
  state: 'fresh' | 'lapsed',
  serverId: string = LEASED_SERVER_ID,
  nowSeconds: number = Math.floor(Date.now() / 1000),
): World {
  const expiresAt =
    state === 'lapsed' ? nowSeconds : nowSeconds + 12 * 86_400;
  const retained = world.observedExpiredLeases.filter(
    (entry) => entry.profile !== serverId || entry.expiresAt >= expiresAt,
  );
  const observedExpiredLeases =
    state === 'lapsed' &&
    !retained.some(
      (entry) => entry.profile === serverId && entry.expiresAt >= expiresAt,
    )
      ? [...retained, { profile: serverId, expiresAt }]
      : retained;
  return {
    ...world,
    observedExpiredLeases,
    servers: world.servers.map((server) =>
      server.id !== serverId
        ? server
        : {
            ...server,
            compatibility: {
              status: 'required',
              expiresAt,
            } satisfies CompatibilityLease,
          },
    ),
  };
}
