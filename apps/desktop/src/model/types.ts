/**
 * Domain data types for the FOKS desktop shell.
 *
 * Defines models for accounts, stores, parties, federation, and items
 * corresponding to the backend agent command schema.
 */

/* ---------------------------------------------------------------- roles -- */

/**
 * A role, normalized.
 *
 * Protocol roles are Member (with visibility band), Admin, and Owner.
 * The optional visibility level applies only to Member roles.
 */
export interface Role {
  kind: 'member' | 'admin' | 'owner';
  visibility?: number;
}

/**
 * A role as the fixture (and the agent) writes it: either the display string
 * `"Owner"` / `"Member · visibility 0"`, or the structured
 * `{ role, visibility }`. `parseRole` normalizes both.
 */
export type RoleWire = string | { role: string; visibility?: number };

/* --------------------------------------------------------------- stores -- */

/** A store's identity: `acct:<alias>` or `team:<alias>` in the fixture. */
export type StoreRef = string;

export interface AccountStore {
  id: StoreRef;
  kind: 'account';
  name: string;
  server: string;
  account: string;
}

export type TeamCreationPhase =
  | 'preparing'
  | 'prepared'
  | 'submission-unknown'
  | 'submitted'
  | 'remote-verified'
  | 'complete'
  | 'rejected'
  | 'legacy-unknown';

export interface TeamStore {
  id: StoreRef;
  kind: 'team';
  name: string;
  alias: string;
  server: string;
  account: string;
  active: boolean;
  /** Durable local preparation and submission evidence; absent for discovered teams. */
  creation_phase?: TeamCreationPhase;
  team_kind: 'named' | 'adhoc';
  /** 66 hex characters. */
  team_id_hex: string;
  /**
   * The team chain sequence the agent has pinned locally. A roster read is
   * skipped while this is unchanged; absent means unknown, which always
   * reads the roster.
   */
  chain_seqno?: number;
}

export type Store = AccountStore | TeamStore;

/* ---------------------------------------------------------------- items -- */

/** The KV node type, as the agent reports it. */
export type NodeKind = 'Secret' | 'File' | 'Folder' | 'Link';

/** The kind rule's reading of a node — client-side, no protocol meaning. */
export type ItemKind = 'Password' | 'Document' | 'Folder';

/** The node type a `NodeKind` is a reading of. */
export type NodeType = 'small_file' | 'file' | 'symlink' | 'directory';

export interface Item {
  store: StoreRef;
  path: string;
  kind: NodeKind;
  size: number | null;
  version: number;
  read: RoleWire;
  write: RoleWire;
  /** Masked placeholder shown before Show; absent on File and Folder nodes. */
  value?: string;
  /** Symlink target, on a Link. */
  target?: string;
}

/* -------------------------------------------------------------- parties -- */

export interface Party {
  store: StoreRef;
  /** Optional on the wire; a team party has none. */
  username?: string | null;
  /** `"you"` on the local account's own party, in the fixture. */
  label?: string;
  party_kind: 'user' | 'named-team' | 'ad-hoc-team';
  /** Present when `party_kind` is `named-team`. */
  team_name?: string;
  generation: number;
  /** Whether this party is a local user of this server. */
  locally_manageable: boolean;
  party_id_hex: string;
  scoped_host_id_hex?: string;
  /** Source and destination differ for a federated party. */
  source_role: RoleWire;
  destination_role: RoleWire;
  /** Fixture-only explanatory note; not a protocol field. */
  note?: string;
}

/** One admission of a remote team into a local team. */
export interface FederationEntry {
  store: StoreRef;
  remote_profile: string;
  remote_team_alias: string;
  remote_host_id_hex: string;
  remote_team_id_hex: string;
  destination: RoleWire;
  operation_id_hex?: string;
  /** False when the admission is inactive; no extra payload in that case. */
  active: boolean;
}

/* -------------------------------------------------- servers and leases -- */

/** Fixture URL control; production access derives from signed lease facts. */
export type LeaseState = 'fresh' | 'lapsed';

/** A structured scoped failure retained as data instead of collapsed prose. */
export interface ServerFailure {
  code: string;
  message: string;
  retryable: boolean;
  ambiguous: boolean;
  fatal: boolean;
  details?: {
    kind?: string;
    operation?: string;
    capability?: string;
    profile?: string;
    stateDir?: string;
    reason?: string;
    foundSchema?: number;
    supportedSchema?: number;
  };
}

export type ServerTrust =
  | { status: 'unknown' }
  | { status: 'verified' }
  | { status: 'unprobed' }
  | { status: 'blocked'; error: ServerFailure };

export const PROTOCOL_CAPABILITIES = [
  'signup',
  'user-sync',
  'kv',
  'device-administration',
  'recovery',
  'passphrases',
  'teams',
  'chat',
  'federation',
] as const;
export type ProtocolCapability = (typeof PROTOCOL_CAPABILITIES)[number];
export type CompatibilityFailure =
  'drift' | 'protocol-mismatch' | 'unknown-capability';

export type CompatibilityLease =
  | { status: 'not-required' }
  | {
      status: 'required';
      expiresAt: number;
      capabilities: readonly ProtocolCapability[];
    }
  | { status: 'incompatible'; expiresAt: number; reason: CompatibilityFailure }
  | { status: 'required-unavailable' }
  | { status: 'requirement-unknown'; error: ServerFailure };

export type PassiveServerStatus =
  | { status: 'loading' }
  | { status: 'available'; source: 'signed-server-status' }
  | {
      status: 'failed';
      source: 'describe-server-status';
      error: ServerFailure;
    };

/** Passive signed status is not evidence of a live connection. */
export type ConnectivityObservation =
  import('../profile-connectivity').ProfileConnectivity;

export interface ServerServices {
  chat: boolean | null;
}

export type ServerRestriction =
  | { kind: 'schema-incompatible'; error: ServerFailure }
  | { kind: 'import-verification-required'; error: ServerFailure }
  | {
      kind: 'capability-denied';
      capability: ProtocolCapability;
      error: ServerFailure;
    };

/** Independent facts about one configured server. */
export interface Server {
  /** Durable local profile identity; never a display label or network address. */
  id: string;
  /** The immutable profile name; currently identical to `id`. */
  name: string;
  label: string | null;
  /** Configured network probe/address, separate from profile identity and label. */
  configuredProbe: string;
  host_id: string | null;
  chain: number | null;
  epoch: number | null;
  accounts: string[];
  trust: ServerTrust;
  compatibility: CompatibilityLease;
  passiveStatus: PassiveServerStatus;
  connectivity: ConnectivityObservation;
  services: ServerServices;
  restrictions: readonly ServerRestriction[];
}

/** The local display label, falling back to the durable profile name. */
export function serverDisplayName(
  server: Pick<Server, 'name' | 'label'>,
): string {
  return server.label ?? server.name;
}

export function serverLocalAlias(
  server: Pick<Server, 'label'> | undefined,
): string {
  return server?.label?.trim() || 'Loading...';
}

export interface Account {
  /** Canonical identifier of the account store. */
  store: StoreRef;
  /** Stable profile-local command selector. */
  alias: string;
  /** Editable local display alias; absent until changed. */
  localAlias?: string;
  username: string;
  server: string;
}

/** A name resolved from the Devices page's account-scoped bridge lists. */
export interface DeviceLabel {
  store: StoreRef;
  address: string;
  name: string;
}

export interface Device {
  alias: string;
  name: string;
  role: string;
  current: boolean;
  id_hex: string;
}

export interface YubiAccount {
  alias: string;
  server: string;
  serial: number;
  /** As `list_yubi_accounts` reports it: an enrollment may be unfinished. */
  state: 'pending' | 'complete';
}

export interface Card {
  serial: number;
}

export type Severity = 'info' | 'warn' | 'crit';

export interface Notification {
  id: string;
  profile?: string;
  severity: Severity;
  title: string;
  detail: string;
  action: string;
}

export type AgentStatus =
  | { readonly state: 'ready'; readonly historyAfter?: boolean }
  | { readonly state: 'bootstrap'; readonly step: string };

/* ------------------------------------------------------------- snapshot -- */

/**
 * Complete immutable snapshot of the application state, including servers,
 * stores, items, parties, and active leases.
 */
export type GroupDetailSource = 'roster' | 'federation';

export interface GroupDetailFailure {
  store: StoreRef;
  source: GroupDetailSource;
  details?: ServerFailure['details'];
  code: string;
  message: string;
  retryable: boolean;
}

export interface CatalogFreshnessEntry {
  /** Elapsed time of the catalog load that last successfully covered this entry. */
  lastMilliseconds?: number;
  lastSuccessAt?: number;
  lastAttemptAt?: number;
  refreshing: boolean;
  error?: ServerFailure;
}

export interface CatalogFreshness {
  attempt?: CatalogFreshnessEntry;
  profiles: Readonly<Record<string, CatalogFreshnessEntry>>;
  stores: Readonly<Record<StoreRef, CatalogFreshnessEntry>>;
}

export interface AgentSnapshot {
  catalogFreshness?: CatalogFreshness;
  agent: AgentStatus;
  servers: readonly Server[];
  accounts: readonly Account[];
  stores: readonly Store[];
  storeInventory: readonly {
    store: StoreRef;
    status: 'available' | 'unavailable' | 'loading';
    error?: ServerFailure;
    restrictions: readonly ServerRestriction[];
  }[];
  profileInventory: readonly {
    profile: string;
    accounts: 'complete' | 'unavailable';
    teams: 'complete' | 'unavailable';
  }[];
  /** Profiles named by the authoritative catalog inventory response. */
  catalogProfiles: readonly string[];
  profileInventoryStatus: 'complete' | 'unavailable';
  items: readonly Item[];
  parties: readonly Party[];
  federation: readonly FederationEntry[];
  groupDetailFailures: readonly GroupDetailFailure[];
  devices: readonly Device[];
  yubiAccounts: readonly YubiAccount[];
  cardsConnected: readonly Card[];
  notifications: readonly Notification[];
  /** Signed expirations already observed locally, keyed to their exact fact. */
  observedExpiredLeases: readonly { profile: string; expiresAt: number }[];
  /** Illustrative plaintext, keyed `<store>|<path>` — a fixture extension. */
  plaintext: Readonly<Record<string, string>>;
}

/** A `<store>|<path>` item key, as the fixture's `plaintext` map uses. */
export function itemKey(item: Pick<Item, 'store' | 'path'>): string {
  return `${item.store}|${item.path}`;
}
