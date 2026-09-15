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
  AgentSnapshot,
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
  { available: true } | { available: false; reason: AvailabilityReason };

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
export function serverOf(
  snapshot: AgentSnapshot,
  ref: StoreRef,
): Server | undefined {
  const store = storeOf(snapshot, ref);
  return store && snapshot.servers.find((s) => s.id === store.server);
}

/** Derives server access from independent facts at the supplied clock instant. */
export function serverAvailability(
  snapshot: AgentSnapshot,
  server: Server,
  options: AvailabilityOptions = {},
): Availability {
  if (snapshot.agent.state !== 'ready')
    return { available: false, reason: 'agent-unavailable' };
  return serverFactAvailability(
    server,
    snapshot.observedExpiredLeases,
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
  snapshot: AgentSnapshot,
  store: Store,
  options: AvailabilityOptions = {},
): Availability {
  if (
    snapshot.agent.state !== 'ready' ||
    options.agentReady === false ||
    options.catalogReady === false
  )
    return { available: false, reason: 'agent-unavailable' };
  const server = snapshot.servers.find(
    (candidate) => candidate.id === store.server,
  );
  if (!server) return { available: false, reason: 'vault-unavailable' };
  const serverSecurity = serverSecurityAvailability(server);
  if (serverSecurity) return serverSecurity;
  const inventory = snapshot.storeInventory.find(
    (entry) => entry.store === store.id,
  );
  if (
    inventory?.restrictions.some(
      (entry) => entry.kind === 'schema-incompatible',
    )
  )
    return { available: false, reason: 'schema-incompatible' };
  if (
    inventory?.restrictions.some(
      (entry) => entry.kind === 'import-verification-required',
    )
  )
    return { available: false, reason: 'import-verification-required' };
  const serverAccess = serverOperationalAvailability(
    server,
    snapshot.observedExpiredLeases,
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
  snapshot: AgentSnapshot,
  kind: 'profiles' | 'accounts' | 'teams',
): boolean {
  if (kind === 'profiles')
    return snapshot.profileInventoryStatus === 'complete';
  if (snapshot.profileInventoryStatus !== 'complete') return false;
  return snapshot.catalogProfiles.every(
    (profile) =>
      snapshot.profileInventory.find((entry) => entry.profile === profile)?.[
        kind
      ] === 'complete',
  );
}

/** Whether this store's server has a lapsed check-in. */
export function leaseLapsed(snapshot: AgentSnapshot, ref: StoreRef): boolean {
  const store = storeOf(snapshot, ref);
  if (!store) return false;
  const availability = storeAvailability(snapshot, store);
  return !availability.available && availability.reason === 'check-in-expired';
}

/** A protocol safety failure blocks a profile without claiming its lease lapsed. */
export function serverBlocked(snapshot: AgentSnapshot, ref: StoreRef): boolean {
  return serverOf(snapshot, ref)?.trust.status === 'blocked';
}

/** A server that has never been checked hides its stores until it is. */
export function serverNeverProbed(
  snapshot: AgentSnapshot,
  ref: StoreRef,
): boolean {
  return serverOf(snapshot, ref)?.trust.status === 'unprobed';
}

/** No usable signed expiry was available, so access fails closed without claiming lapse. */
export function serverLeaseUnavailable(
  snapshot: AgentSnapshot,
  ref: StoreRef,
): boolean {
  const lease = serverOf(snapshot, ref)?.compatibility;
  return (
    lease?.status === 'required-unavailable' ||
    lease?.status === 'requirement-unknown'
  );
}

export type StoreDescriptionState = 'normal' | AvailabilityReason;

/** The condition that replaces a store's ordinary sidebar/header description. */
export function storeDescriptionState(
  snapshot: AgentSnapshot,
  store: Store,
): StoreDescriptionState {
  const availability = storeAvailability(snapshot, store);
  return availability.available ? 'normal' : availability.reason;
}

export function groupDetailFailure(
  snapshot: AgentSnapshot,
  store: StoreRef,
  source: GroupDetailSource,
): GroupDetailFailure | undefined {
  return snapshot.groupDetailFailures.find(
    (failure) => failure.store === store && failure.source === source,
  );
}

/** The one description used for a store in both navigation and page headers. */
export function storeDescription(
  snapshot: AgentSnapshot,
  store: Store,
): string {
  const state = storeDescriptionState(snapshot, store);
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
  if (store.kind === 'account') return serverOf(snapshot, store.id)?.name ?? '';
  if (groupDetailFailure(snapshot, store.id, 'roster'))
    return 'Roster unavailable';
  if (groupDetailFailure(snapshot, store.id, 'federation'))
    return 'Federation unavailable';
  return peopleGroups(partiesOf(snapshot, store.id));
}

/**
 * Returns subtitle text for a store page header. Suppresses status error text
 * when errors are already surfaced by full-page notices in the body.
 */
export function storeHeadingDescription(
  snapshot: AgentSnapshot,
  store: Store,
): string {
  if (storeDescriptionState(snapshot, store) !== 'normal') return '';
  if (groupDetailFailure(snapshot, store.id, 'roster')) return '';
  if (groupDetailFailure(snapshot, store.id, 'federation')) return '';
  return storeDescription(snapshot, store);
}

/**
 * Returns whether a store is readable: its server lease must be valid,
 * and team stores must report an active status.
 */
export function storeReadable(
  snapshot: AgentSnapshot,
  ref: StoreRef,
  options: AvailabilityOptions = {},
): boolean {
  const store = storeOf(snapshot, ref);
  return Boolean(
    store && storeAvailability(snapshot, store, options).available,
  );
}

export function serverChatAvailable(
  snapshot: AgentSnapshot,
  server: Server,
  options: AvailabilityOptions = {},
): boolean {
  return (
    server.capabilities.chat &&
    serverAvailability(snapshot, server, options).available
  );
}

/** Returns whether the current user has permission to create items in this store. */
export function canCreateInStore(
  snapshot: AgentSnapshot,
  ref: StoreRef,
): boolean {
  const store = storeOf(snapshot, ref);
  if (!store || !storeReadable(snapshot, store.id)) return false;
  if (store.kind === 'account') return true;
  const own = partiesOf(snapshot, store.id).filter(
    (party) =>
      party.label === 'you' &&
      party.party_kind === 'user' &&
      party.locally_manageable &&
      admissionActive(snapshot, party, store.id),
  );
  return own.length === 1;
}

/** Returns whether the local authenticated user has permission to edit the specified item. */
export function canChangeItem(snapshot: AgentSnapshot, item: Item): boolean {
  const store = storeOf(snapshot, item.store);
  if (!store || !canCreateInStore(snapshot, store.id)) return false;
  if (store.kind === 'account') return true;
  const own = partiesOf(snapshot, store.id).filter(
    (party) =>
      party.label === 'you' &&
      party.party_kind === 'user' &&
      party.locally_manageable &&
      admissionActive(snapshot, party, store.id),
  );
  return own.length === 1 && admits(own[0].destination_role, item.write);
}

/** Returns all accessible items from readable stores, excluding directory entries. */
export function catalog(snapshot: AgentSnapshot): Item[] {
  return snapshot.items.filter(
    (item) => item.kind !== 'Folder' && storeReadable(snapshot, item.store),
  );
}

/** Returns all readable stores in navigation display order. */
export function listableStores(snapshot: AgentSnapshot): Store[] {
  return snapshot.stores.filter((store) => storeReadable(snapshot, store.id));
}

/**
 * Determines the default store selection for new item creation when no store
 * context is active. Selects the first writable store in navigation order, or
 * falls back to the first available store if all are read-only.
 */
export function defaultCreateStore(
  snapshot: AgentSnapshot,
): StoreRef | undefined {
  const order = storeNavigationOrder(snapshot);
  const writable = order.find((store) => canCreateInStore(snapshot, store.id));
  return (writable ?? order[0])?.id;
}

/**
 * Filters active system notifications based on current server lease state.
 */
export function notesNow(snapshot: AgentSnapshot): Notification[] {
  return snapshot.notifications.filter((note) => {
    if (note.id !== 'lease-acme') return true;
    const server = snapshot.servers.find(
      (entry) => entry.id === LEASED_SERVER_ID,
    );
    if (!server) return false;
    const availability = serverAvailability(snapshot, server);
    return (
      !availability.available && availability.reason === 'check-in-expired'
    );
  });
}

/**
 * Put a server's compatibility lease into `state`, returning a new snapshot.
 *
 * Returns a cloned AgentSnapshot with the updated lease state and server
 * records.
 */
export function applyLease(
  snapshot: AgentSnapshot,
  state: 'fresh' | 'lapsed',
  serverId: string = LEASED_SERVER_ID,
  nowSeconds: number = Math.floor(Date.now() / 1000),
): AgentSnapshot {
  const expiresAt = state === 'lapsed' ? nowSeconds : nowSeconds + 12 * 86_400;
  const retained = snapshot.observedExpiredLeases.filter(
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
    ...snapshot,
    observedExpiredLeases,
    servers: snapshot.servers.map((server) =>
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
