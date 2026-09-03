/**
 * The shapes the FOKS shell reads.
 *
 * Mirrors the desktop fixture and, through it, what the agent can actually
 * return. The product constraint is that item
 * metadata is exactly path, kind, version, size, read role and write role —
 * no modified time, no owner name, no per-item sync state, no tags, no
 * history. Do not add a field here that the agent cannot answer.
 */

/* ---------------------------------------------------------------- roles -- */

/**
 * A role, normalised.
 *
 * The protocol's three roles are Member { visibility }, Admin and Owner
 * Never "Reader", "Manager", "Viewer" or "Editor". `visibility`
 * is an ordered band inside Member — 0 is the default; lower bands see less —
 * and is meaningless on Admin and Owner, so it is absent there.
 */
export interface Role {
  kind: 'member' | 'admin' | 'owner';
  visibility?: number;
}

/**
 * A role as the fixture (and the agent) writes it: either the display string
 * `"Owner"` / `"Member · visibility 0"`, or the structured
 * `{ role, visibility }`. `parseRole` normalises both.
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

export interface TeamStore {
  id: StoreRef;
  kind: 'team';
  name: string;
  alias: string;
  server: string;
  account: string;
  /** A team summary carries one state — active or not. */
  active: boolean;
  team_kind: 'named' | 'adhoc';
  /** 66 hex characters. */
  team_id_hex: string;
}

export type Store = AccountStore | TeamStore;

/* ---------------------------------------------------------------- items -- */

/** The KV node type, as the agent reports it. */
export type NodeKind = 'Secret' | 'File' | 'Folder' | 'Link';

/** The kind rule's reading of a node — client-side, no protocol meaning. */
export type ItemKind = 'Password' | 'Resource' | 'File' | 'Link' | 'Folder';

/** The node type a `NodeKind` is a reading of. */
export type NodeType = 'small_file' | 'file' | 'symlink' | 'directory';

export interface Item {
  store: StoreRef;
  path: string;
  kind: NodeKind;
  size: number;
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

/** The compatibility lease's state for one server. */
export type LeaseState = 'fresh' | 'lapsed' | 'unavailable';

export interface Lease {
  state: LeaseState;
  expires_in: string | null;
}

export type ServerState =
  'ok' | 'lease-lapsed' | 'lease-unavailable' | 'never-probed' | 'blocked';

/** The last `ProbeReport` for a server, plus its lease. */
export interface Server {
  id: string;
  name: string;
  label: string | null;
  host_id: string | null;
  chain: number | null;
  epoch: number | null;
  lease: Lease | null;
  accounts: string[];
  state: ServerState;
}

export interface Account {
  /**
   * Exact identity: the `id` of this account's `AccountStore`. Required, not
   * optional — the live projection has always carried it (`list_accounts`
   * fails closed without it) and every fixture now does too, so nothing has to
   * fall back to matching on an alias that two profiles can share.
   */
  store: StoreRef;
  /** The profile-local display label. Never an identity. */
  alias: string;
  username: string;
  server: string;
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
}

export interface Card {
  serial: number;
}

export type Severity = 'info' | 'warn' | 'crit';

export interface Notification {
  id: string;
  severity: Severity;
  title: string;
  detail: string;
  action: string;
}

export interface AgentStatus {
  phase: string;
}

/* ---------------------------------------------------------------- world -- */

/**
 * Everything the shell reads, in one immutable value.
 *
 * The mock reaches for module globals; every model function here takes the
 * world it is asked about instead, so a fixture, a mocked bridge and a live
 * bridge are the same code path and the lease switch is a pure function
 * (`applyLease`) rather than a mutation.
 */
export type GroupDetailSource = 'roster' | 'federation';

export interface GroupDetailFailure {
  store: StoreRef;
  source: GroupDetailSource;
  code: string;
  message: string;
  retryable: boolean;
}

export interface World {
  agent: AgentStatus;
  servers: readonly Server[];
  accounts: readonly Account[];
  stores: readonly Store[];
  unavailableStores: readonly StoreRef[];
  accountInventoryComplete: boolean;
  items: readonly Item[];
  parties: readonly Party[];
  federation: readonly FederationEntry[];
  groupDetailFailures: readonly GroupDetailFailure[];
  devices: readonly Device[];
  yubiAccounts: readonly YubiAccount[];
  cardsConnected: readonly Card[];
  notifications: readonly Notification[];
  /** The lease world the shell is currently showing. */
  leaseState: LeaseState;
  /** Illustrative plaintext, keyed `<store>|<path>` — a fixture extension. */
  plaintext: Readonly<Record<string, string>>;
}

/** A `<store>|<path>` item key, as the fixture's `plaintext` map uses. */
export function itemKey(item: Pick<Item, 'store' | 'path'>): string {
  return `${item.store}|${item.path}`;
}
