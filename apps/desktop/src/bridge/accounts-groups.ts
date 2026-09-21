import type {
  Account,
  FederationEntry,
  Party,
  RoleWire,
  StoreRef,
} from '../model';
import { parseRole } from '../model/roles';
import { decodeCommandError, type CommandError } from './errors';
import {
  array,
  bool,
  entityId,
  integer,
  optionalString,
  record,
  signed16,
  string,
} from './validation';

export type GroupDetailResult<T> =
  { status: 'success'; value: T } | { status: 'error'; error: CommandError };

export interface GroupDetailsDto {
  parties: GroupDetailResult<Party[]>;
  federation: GroupDetailResult<FederationEntry[]>;
}

export type RoleDto =
  | { role: 'Member'; visibility: number }
  | { role: 'Admin' }
  | { role: 'Owner' };

export interface AccountDto {
  store: StoreRef;
  profile: string;
  alias: string;
  username: string;
}

export interface CreateGroupRequest {
  accountStoreId: StoreRef;
  teamAlias: string;
  name: string;
  kind: 'named' | 'adhoc';
}

export interface GroupMemberRequest {
  storeId: StoreRef;
  username: string;
}

export interface GroupRoleRequest extends GroupMemberRequest {
  destination: RoleDto;
}

export interface AdmitGroupRequest {
  storeId: StoreRef;
  remoteStoreId: StoreRef;
  visibility: number;
}

export interface RemoveFederatedGroupRequest {
  storeId: StoreRef;
  remoteHostIdHex: string;
  remoteTeamIdHex: string;
}

export interface DiscoveredGroup {
  alias: string;
  accountAlias: string;
  teamIdHex: string;
  kind: 'named' | 'adhoc';
  name?: string;
  active: boolean;
}

export interface GroupDiscoveryResponse {
  accountAlias: string;
  groups: DiscoveredGroup[];
  /**
   * The aliases whose local binding this discovery wrote. Empty when it
   * changed nothing, and empty from an agent that predates the field, which
   * a caller must read as "unknown", not as "nothing changed".
   */
  bound?: string[];
}

export function decodeRole(value: unknown, at: string): RoleDto {
  const item = record(value, at);
  const role = string(item.role, `${at}.role`);
  if (role === 'Member') {
    return { role, visibility: signed16(item.visibility, `${at}.visibility`) };
  }
  if (role === 'Admin' || role === 'Owner') return { role };
  throw new Error(`${at}.role is not Member, Admin, or Owner`);
}

function decodeParty(value: unknown, at: string): Party {
  const item = record(value, at);
  const kind = string(item.party_kind, `${at}.party_kind`);
  if (kind !== 'user' && kind !== 'named-team' && kind !== 'ad-hoc-team') {
    throw new Error(`${at}.party_kind is not user, named-team, or ad-hoc-team`);
  }
  const username =
    item.username === null
      ? null
      : optionalString(item.username, `${at}.username`);
  return {
    store: string(item.store, `${at}.store`),
    username,
    party_kind: kind,
    generation: integer(item.generation, `${at}.generation`),
    locally_manageable: bool(
      item.locally_manageable,
      `${at}.locally_manageable`,
    ),
    party_id_hex: string(item.party_id_hex, `${at}.party_id_hex`),
    scoped_host_id_hex: optionalString(
      item.scoped_host_id_hex,
      `${at}.scoped_host_id_hex`,
    ),
    source_role: decodeRole(item.source_role, `${at}.source_role`),
    destination_role: decodeRole(
      item.destination_role,
      `${at}.destination_role`,
    ),
    label: optionalString(item.label, `${at}.label`),
    team_name: optionalString(item.team_name, `${at}.team_name`),
  };
}

function decodeAccount(value: unknown, at: string): Account {
  const item = record(value, at);
  return {
    store: string(item.store, `${at}.store`),
    server: string(item.profile, `${at}.profile`),
    ...(item.local_alias == null
      ? {}
      : { localAlias: string(item.local_alias, `${at}.local_alias`) }),
    alias: string(item.alias, `${at}.alias`),
    username: string(item.username, `${at}.username`),
  };
}

export function decodeAccounts(value: unknown): Account[] {
  return array(value, 'list_accounts response', decodeAccount);
}

function decodeFederationEntry(value: unknown, at: string): FederationEntry {
  const item = record(value, at);
  return {
    store: string(item.store, `${at}.store`),
    remote_profile: string(item.remote_profile, `${at}.remote_profile`),
    remote_team_alias: string(
      item.remote_team_alias,
      `${at}.remote_team_alias`,
    ),
    remote_host_id_hex: string(
      item.remote_host_id_hex,
      `${at}.remote_host_id_hex`,
    ),
    remote_team_id_hex: string(
      item.remote_team_id_hex,
      `${at}.remote_team_id_hex`,
    ),
    destination: decodeRole(item.destination, `${at}.destination`),
    operation_id_hex: optionalString(
      item.operation_id_hex,
      `${at}.operation_id_hex`,
    ),
    active: bool(item.active, `${at}.active`),
  };
}

export const decodeParties = (value: unknown): Party[] =>
  array(value, 'list_parties response', decodeParty);
export const decodeFederation = (value: unknown): FederationEntry[] =>
  array(value, 'list_federation response', decodeFederationEntry);

function decodeGroupDetailResult<T>(
  value: unknown,
  at: string,
  decode: (value: unknown) => T,
): GroupDetailResult<T> {
  const item = record(value, at);
  const status = string(item.status, `${at}.status`);
  if (status === 'success') return { status, value: decode(item.value) };
  if (status === 'error')
    return { status, error: decodeCommandError(item.error, `${at}.error`) };
  throw new Error(`${at}.status is not success or error`);
}

export function decodeGroupDetails(value: unknown): GroupDetailsDto {
  const item = record(value, 'list_group_details response');
  return {
    parties: decodeGroupDetailResult(
      item.parties,
      'list_group_details response.parties',
      decodeParties,
    ),
    federation: decodeGroupDetailResult(
      item.federation,
      'list_group_details response.federation',
      decodeFederation,
    ),
  };
}

function decodeDiscoveredGroup(value: unknown, at: string): DiscoveredGroup {
  const item = record(value, at);
  const kind = string(item.kind, `${at}.kind`);
  if (kind !== 'named' && kind !== 'adhoc')
    throw new Error(`${at}.kind is not named or adhoc`);
  const name = optionalString(item.name, `${at}.name`);
  if ((kind === 'named') !== Boolean(name)) {
    throw new Error(`${at}.name must be present only for a named group`);
  }
  return {
    alias: string(item.alias, `${at}.alias`),
    accountAlias: string(item.accountAlias, `${at}.accountAlias`),
    teamIdHex: entityId(
      item.teamIdHex,
      kind === 'named' ? '03' : '14',
      `${at}.teamIdHex`,
    ),
    kind,
    name,
    active: bool(item.active, `${at}.active`),
  };
}

export function decodeGroupDiscovery(value: unknown): GroupDiscoveryResponse {
  const item = record(value, 'discover_groups response');
  const accountAlias = string(
    item.accountAlias,
    'discover_groups.accountAlias',
  );
  const groups = array(
    item.groups,
    'discover_groups.groups',
    decodeDiscoveredGroup,
  );
  if (groups.some((group) => group.accountAlias !== accountAlias)) {
    throw new Error('discover_groups returned a group for a different account');
  }
  const bound =
    item.bound === undefined
      ? undefined
      : array(item.bound, 'discover_groups.bound', (value, at) =>
          string(value, at),
        );
  if (bound?.some((alias) => !groups.some((group) => group.alias === alias))) {
    throw new Error('discover_groups bound an alias it did not return');
  }
  return { accountAlias, groups, ...(bound ? { bound } : {}) };
}

/** Converts model roles to the command DTO role format. */
export function roleDto(role: RoleWire): RoleDto {
  const parsed = parseRole(role);
  if (!parsed) throw new Error('fixture role is invalid');
  if (parsed.kind === 'member') {
    return { role: 'Member', visibility: parsed.visibility ?? 0 };
  }
  return { role: parsed.kind === 'admin' ? 'Admin' : 'Owner' };
}
