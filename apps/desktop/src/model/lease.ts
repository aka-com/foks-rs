/**
 * Server lease evaluation and catalog visibility rules.
 *
 * A lapsed compatibility session lease blocks access to all items and stores
 * hosted on that server.
 */

import { storeNavigationOrder } from './order';
import { admissionActive, partiesOf, peopleGroups, storeOf } from './readers';
import { admits } from './roles';
import { serverName } from './server-name';
import { serverDisplayName, PROTOCOL_CAPABILITIES } from './types';
import type {
  AccountStore,
  CompatibilityLease,
  GroupDetailFailure,
  GroupDetailSource,
  Item,
  Notification,
  ProtocolCapability,
  Server,
  Store,
  StoreRef,
  TeamStore,
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
  | 'loading'
  | 'compatibility-incompatible'
  | 'capability-unavailable'
  | 'chat-unsupported'
  | 'store-metadata-unavailable'
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
  | {
      available: false;
      reason: AvailabilityReason;
      capability?: ProtocolCapability;
    };

export type StoreOperation =
  'vault' | 'chat' | 'teams' | 'federation' | 'devices' | 'metadata';

export interface AvailabilityOptions {
  operation?: StoreOperation;
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
  capabilities: readonly ProtocolCapability[] = [],
): Availability {
  if (options.agentReady === false)
    return { available: false, reason: 'agent-unavailable' };
  if (options.catalogReady === false)
    return { available: false, reason: 'store-metadata-unavailable' };
  const common =
    serverSecurityAvailability(server) ??
    serverOperationalAvailability(server, observedExpiredLeases, options);
  if (!common.available) return common;
  for (const capability of capabilities) {
    if (
      server.restrictions.some(
        (entry) =>
          entry.kind === 'capability-denied' && entry.capability === capability,
      ) ||
      (server.compatibility.status === 'required' &&
        !server.compatibility.capabilities.includes(capability))
    )
      return { available: false, reason: 'capability-unavailable', capability };
  }
  return { available: true };
}

function serverSecurityAvailability(server: Server): Availability | null {
  if (server.trust.status === 'blocked')
    return { available: false, reason: 'verification-failed' };
  if (server.restrictions.some((entry) => entry.kind === 'schema-incompatible'))
    return { available: false, reason: 'schema-incompatible' };
  if (
    server.restrictions.some(
      (entry) => entry.kind === 'import-verification-required',
    )
  )
    return { available: false, reason: 'import-verification-required' };
  if (server.trust.status === 'unknown')
    return {
      available: false,
      reason:
        server.passiveStatus.status === 'loading'
          ? 'loading'
          : 'server-status-unavailable',
    };
  if (server.trust.status === 'unprobed')
    return { available: false, reason: 'verification-required' };
  return null;
}

function serverOperationalAvailability(
  server: Server,
  observedExpiredLeases: readonly { profile: string; expiresAt: number }[],
  options: AvailabilityOptions,
): Availability {
  const lease = server.compatibility;
  if (lease.status === 'incompatible')
    return { available: false, reason: 'compatibility-incompatible' };
  if (server.passiveStatus.status === 'loading')
    return { available: false, reason: 'loading' };
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

function requiredCapabilities(
  store: Store,
  operation: StoreOperation,
): readonly ProtocolCapability[] {
  switch (operation) {
    case 'vault':
      return store.kind === 'team' ? ['teams', 'kv'] : ['kv'];
    case 'federation':
      return ['teams', 'federation'];
    case 'devices':
      return ['device-administration'];
    case 'metadata':
      return [];
    default:
      return [operation];
  }
}

export function serverCapabilityAvailability(
  snapshot: AgentSnapshot,
  server: Server,
  capabilities: readonly ProtocolCapability[],
  options: AvailabilityOptions = {},
): Availability {
  if (snapshot.agent.state !== 'ready')
    return { available: false, reason: 'agent-unavailable' };
  return serverFactAvailability(
    server,
    snapshot.observedExpiredLeases,
    options,
    capabilities,
  );
}

/** The single access decision shared by navigation, views, and dispatch. */
export function storeAvailability(
  snapshot: AgentSnapshot,
  store: Store,
  options: AvailabilityOptions = {},
): Availability {
  return storeOperationAvailability(snapshot, store, 'vault', options);
}

export function storeOperationAvailability(
  snapshot: AgentSnapshot,
  store: Store,
  operation: StoreOperation,
  options: AvailabilityOptions = {},
): Availability {
  if (snapshot.agent.state !== 'ready' || options.agentReady === false)
    return { available: false, reason: 'agent-unavailable' };
  const server = snapshot.servers.find(
    (candidate) => candidate.id === store.server,
  );
  if (!server) return { available: false, reason: 'vault-unavailable' };
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
  const capabilities = requiredCapabilities(store, operation);
  const serverAccess = serverCapabilityAvailability(
    snapshot,
    server,
    capabilities,
    options,
  );
  if (!serverAccess.available) return serverAccess;
  for (const capability of capabilities) {
    if (
      inventory?.restrictions.some(
        (entry) =>
          entry.kind === 'capability-denied' && entry.capability === capability,
      )
    )
      return { available: false, reason: 'capability-unavailable', capability };
  }
  if (store.kind === 'team' && !store.active)
    return { available: false, reason: 'setup-incomplete' };
  if (operation === 'chat') {
    if (
      store.kind !== 'team' ||
      store.team_kind !== 'named' ||
      server.services.chat === false
    )
      return { available: false, reason: 'chat-unsupported' };
    if (server.services.chat === null)
      return { available: false, reason: 'server-status-unavailable' };
  }
  if (operation === 'vault') {
    if (inventory?.status === 'loading')
      return { available: false, reason: 'loading' };
    if (inventory?.status !== 'available')
      return { available: false, reason: 'vault-unavailable' };
  } else {
    const metadata = snapshot.profileInventory.find(
      (entry) => entry.profile === store.server,
    );
    if (
      metadata?.[store.kind === 'team' ? 'teams' : 'accounts'] !== 'complete' &&
      inventory?.status !== 'available'
    )
      return {
        available: false,
        reason:
          inventory?.status === 'loading'
            ? 'loading'
            : 'store-metadata-unavailable',
      };
  }
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
  options: AvailabilityOptions = {},
): StoreDescriptionState {
  const availability = storeOperationAvailability(
    snapshot,
    store,
    options.operation ?? 'vault',
    options,
  );
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

/** Why a store's row says something other than its ordinary description. */
export type StoreAttentionState =
  StoreDescriptionState | 'roster-unavailable' | 'federation-unavailable';

/**
 * Whether anything is wrong with a store, as every list that draws a chip at
 * the end of a row decides it: the store is unavailable, or a detail the row
 * would otherwise summarize could not be read. A list that consulted only
 * `storeDescriptionState` would print "Roster unavailable" as if it were the
 * roster summary.
 */
export function storeAttentionState(
  snapshot: AgentSnapshot,
  store: Store,
  options: AvailabilityOptions = {},
): StoreAttentionState {
  const state = storeDescriptionState(snapshot, store, options);
  if (state !== 'normal') return state;
  if (groupDetailFailure(snapshot, store.id, 'roster'))
    return 'roster-unavailable';
  if (groupDetailFailure(snapshot, store.id, 'federation'))
    return 'federation-unavailable';
  return 'normal';
}

/**
 * The caption under a group's name, wherever one is listed: what it is, then
 * the server it lives on. The account it is held through is added only when
 * this Mac holds two accounts on that server, where the server alone would not
 * say which one this row belongs to; the server itself is dropped on a page
 * that is already about one server, and the kind on a list whose own section
 * label already says it.
 */
export function teamCaption(
  snapshot: AgentSnapshot,
  store: TeamStore,
  options: { shared?: boolean; server?: boolean; kind?: boolean } = {},
): string {
  const parts =
    options.kind === false
      ? []
      : [store.team_kind === 'named' ? 'Named team' : 'Ad-hoc share'];
  if (options.server !== false) {
    parts.push(serverName(snapshot, store));
  }
  if (
    options.shared ??
    snapshot.stores.filter(
      (entry) => entry.kind === 'account' && entry.server === store.server,
    ).length > 1
  )
    parts.push(
      `as ${
        snapshot.accounts.find(
          (account) =>
            account.server === store.server && account.alias === store.account,
        )?.username ?? store.account
      }`,
    );
  return parts.join(' · ');
}

/** The one description used for a store in both navigation and page headers. */
export function storeDescription(
  snapshot: AgentSnapshot,
  store: Store,
  options: AvailabilityOptions = {},
): string {
  const state = storeDescriptionState(snapshot, store, options);
  if (state === 'chat-unsupported') return 'Chat not supported';
  if (state === 'store-metadata-unavailable')
    return 'Store information unavailable';
  if (state === 'loading') return 'Loading';
  if (state === 'compatibility-incompatible') return 'Protocol incompatible';
  if (state === 'capability-unavailable') return 'Operation not permitted';
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
  if (store.kind === 'account') {
    return serverName(snapshot, store);
  }
  if (groupDetailFailure(snapshot, store.id, 'roster'))
    return 'Roster unavailable';
  if (groupDetailFailure(snapshot, store.id, 'federation'))
    return 'Federation unavailable';
  return peopleGroups(partiesOf(snapshot, store.id));
}

/**
 * Returns subtitle text for a store page header. Suppresses status error text
 * when errors are already surfaced by full-page notices in the body, except
 * that a store hosted on a server other than `activeAccount`'s still names
 * that server alongside the problem, since the rail header — which is what
 * ordinarily says the server — is showing a different one.
 *
 * A store on the same server as `activeAccount` never repeats a server the
 * rail header already states; `activeAccount` absent (no account known) is
 * treated the same way, rather than guessing which server to name.
 */
export function storeHeadingDescription(
  snapshot: AgentSnapshot,
  store: Store,
  activeAccount?: Pick<AccountStore, 'server'>,
): string {
  const foreign =
    Boolean(activeAccount) && store.server !== activeAccount!.server;
  const server = foreign ? serverName(snapshot, store) : undefined;
  if (storeDescriptionState(snapshot, store) !== 'normal') {
    if (!server) return '';
    return `${storeDescription(snapshot, store)} · ${server}`;
  }
  if (groupDetailFailure(snapshot, store.id, 'roster'))
    return server ? `Roster unavailable · ${server}` : '';
  if (groupDetailFailure(snapshot, store.id, 'federation'))
    return server ? `Federation unavailable · ${server}` : '';
  // An account store's ordinary description is already just its server name;
  // naming it again after itself would repeat the same word twice.
  if (store.kind === 'account') return server ?? '';
  const description = storeDescription(snapshot, store);
  return server ? `${description} · ${server}` : description;
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
    server.services.chat === true &&
    serverCapabilityAvailability(snapshot, server, ['chat'], options).available
  );
}

/** Whether a store is a team whose server offers chat this account can read. */
export function chatAvailable(
  snapshot: AgentSnapshot,
  store: Store,
  options: AvailabilityOptions = {},
): boolean {
  return storeOperationAvailability(snapshot, store, 'chat', options).available;
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

/** A server whose check-in has lapsed or whose trust has never been verified: the two reasons the Settings tab's amber dot answers for. */
function serverFlaggedForSettings(availability: Availability): boolean {
  return (
    !availability.available &&
    (availability.reason === 'check-in-expired' ||
      availability.reason === 'verification-required')
  );
}

/**
 * The Settings tab's amber dot: on when a server's check-in has lapsed or it
 * has never been verified. Every other access problem — blocked trust, an
 * incompatible schema, an unreadable status — already has its own row in the
 * Servers section's "Needs attention" list, which is the dot's other half:
 * the band that resolves it.
 */
export function settingsAlertSummary(
  snapshot: AgentSnapshot,
): { description: string } | null {
  const flagged = snapshot.servers.filter((server) =>
    serverFlaggedForSettings(serverAvailability(snapshot, server)),
  );
  if (!flagged.length) return null;
  if (flagged.length > 1)
    return { description: `${flagged.length} servers need attention` };
  const server = flagged[0];
  const availability = serverAvailability(snapshot, server);
  const reason =
    !availability.available && availability.reason === 'check-in-expired'
      ? 'check-in expired'
      : 'not verified';
  return { description: `${serverDisplayName(server)}: ${reason}` };
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
              capabilities:
                server.compatibility.status === 'required'
                  ? server.compatibility.capabilities
                  : PROTOCOL_CAPABILITIES,
            } satisfies CompatibilityLease,
          },
    ),
  };
}
