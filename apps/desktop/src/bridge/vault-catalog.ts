import type { Account, NodeKind, StoreRef, TeamCreationPhase } from '../model';
import { decodeAccounts, decodeRole, type RoleDto } from './accounts-groups';
import { decodeCommandError, type CommandError } from './errors';
import { decodeServerStatus, type ServerStatusSnapshot } from './servers';
import {
  array,
  bool,
  integer,
  nullableInteger,
  nullableString,
  optionalInteger,
  record,
  string,
} from './validation';

export interface StoreDto {
  id: StoreRef;
  kind: 'account' | 'team';
  name: string;
  /** Agent profile name; this is the configured, displayable server name. */
  server: string;
  account: string;
  alias?: string;
  active?: boolean;
  creation_phase?: TeamCreationPhase;
  team_kind?: 'named' | 'adhoc';
  team_id_hex?: string;
  /** The agent's pinned team chain sequence, absent when it reported none. */
  chain_seqno?: number;
}

export interface ItemDto {
  store: StoreRef;
  path: string;
  kind: NodeKind;
  size: number | null;
  version: number;
  read: RoleDto;
  write: RoleDto;
}

export interface CatalogInventoryDto {
  profile: string;
  accountsComplete: boolean;
  teamsComplete: boolean;
}

export interface CatalogDto {
  storeReads?: { store: string; state: 'not-loaded' | 'complete' | 'failed' }[];
  fullItemReads?: string[];
  profiles: string[];
  stores: StoreDto[];
  knownStores: StoreDto[];
  inventory: CatalogInventoryDto[];
  items: ItemDto[];
  failures: CatalogFailureDto[];
  blockedProfiles: string[];
  localMetadata?: {
    accounts: Account[];
    profiles: {
      profile: string;
      label: string | null;
      configuredProbe: string;
      status: ServerStatusSnapshot | null;
      error: CommandError | null;
    }[];
  };
  /**
   * Identifies the native snapshot this catalog came from. Later reads that
   * pass it back fail when a newer load or mutation replaced the snapshot.
   * Absent from mock responses, which have no native snapshot.
   */
  generation?: number;
}

export type CatalogFailureDto =
  | { scope: 'profile'; profile: string; source: string; error: CommandError }
  | { scope: 'store'; profile: string; store: string; error: CommandError };

export interface ItemRequest {
  /** Canonical store identifier managed by the native agent. */
  storeId: StoreRef;
  path: string;
  version: number;
}

export interface ReadItemResponse {
  store: StoreRef;
  path: string;
  version: number;
  value: string;
}

export interface CopyResponse {
  ok: true;
}

export interface DownloadResponse {
  saved: boolean;
}

export interface MutationResponse {
  applied: boolean;
}

/** Valid role formats accepted when writing group items. */
export type KvRoleInput = 'Owner' | 'Admin' | `Member:${number}`;

export interface CreateRoleRequest {
  /** Omitted together for account stores, whose native default is Owner. */
  readRole?: KvRoleInput;
  /** Omitted together for account stores, whose native default is Owner. */
  writeRole?: KvRoleInput;
}

export interface CreateTextRequest extends CreateRoleRequest {
  storeId: StoreRef;
  path: string;
  value: string;
}

export interface CreateLinkRequest extends CreateRoleRequest {
  storeId: StoreRef;
  path: string;
  target: string;
}

export interface CreateFolderRequest extends CreateRoleRequest {
  storeId: StoreRef;
  path: string;
}

export type CreateFileRequest = CreateFolderRequest;

export interface EditTextRequest {
  storeId: StoreRef;
  path: string;
  value: string;
  /** Item version expected by the write to prevent concurrent overwrites. */
  version: number;
}

export type RemoveItemRequest = ItemRequest;

export interface ImportDroppedFileRequest extends CreateRoleRequest {
  storeId: StoreRef;
  path: string;
  /** Path authorized by a native file picker or drop event. */
  sourcePath: string;
}

export interface ReplaceDroppedFileRequest extends ItemRequest {
  /** Path authorized by a native file picker or drop event. */
  sourcePath: string;
}

function decodeCreationPhase(
  value: unknown,
  at: string,
): TeamCreationPhase | undefined {
  if (value === undefined) return undefined;
  const phase = string(value, at);
  if (
    ![
      'preparing',
      'prepared',
      'submission-unknown',
      'submitted',
      'remote-verified',
      'complete',
      'rejected',
      'legacy-unknown',
    ].includes(phase)
  ) {
    throw new Error(`${at} is not a known creation phase`);
  }
  return phase as TeamCreationPhase;
}

function decodeStore(value: unknown, at: string): StoreDto {
  const item = record(value, at);
  const kind = string(item.kind, `${at}.kind`);
  if (kind !== 'account' && kind !== 'team') {
    throw new Error(`${at}.kind is not account or team`);
  }
  const common: Omit<StoreDto, 'kind'> = {
    id: string(item.id, `${at}.id`),
    name: string(item.name, `${at}.name`),
    server: string(item.server, `${at}.server`),
    account: string(item.account, `${at}.account`),
  };
  if (kind === 'account') return { ...common, kind };
  const teamKind = string(item.team_kind, `${at}.team_kind`);
  if (teamKind !== 'named' && teamKind !== 'adhoc') {
    throw new Error(`${at}.team_kind is not named or adhoc`);
  }
  return {
    ...common,
    kind,
    alias: string(item.alias, `${at}.alias`),
    active: bool(item.active, `${at}.active`),
    creation_phase: decodeCreationPhase(
      item.creation_phase,
      `${at}.creation_phase`,
    ),
    team_kind: teamKind,
    team_id_hex: string(item.team_id_hex, `${at}.team_id_hex`),
    chain_seqno: optionalInteger(item.chain_seqno, `${at}.chain_seqno`),
  };
}

function decodeItem(value: unknown, at: string): ItemDto {
  const item = record(value, at);
  const kind = string(item.kind, `${at}.kind`);
  if (!['Secret', 'File', 'Folder', 'Link'].includes(kind)) {
    throw new Error(`${at}.kind is not a FOKS node kind`);
  }
  return {
    store: string(item.store, `${at}.store`),
    path: string(item.path, `${at}.path`),
    kind: kind as NodeKind,
    size: nullableInteger(item.size, `${at}.size`),
    version: integer(item.version, `${at}.version`),
    read: decodeRole(item.read, `${at}.read`),
    write: decodeRole(item.write, `${at}.write`),
  };
}

function decodeFailure(value: unknown, at: string): CatalogFailureDto {
  const item = record(value, at);
  const scope = string(item.scope, `${at}.scope`);
  const profile = string(item.profile, `${at}.profile`);
  const error = decodeCommandError(item.error, `${at}.error`);
  if (scope === 'profile') {
    return {
      scope,
      profile,
      source: string(item.source, `${at}.source`),
      error,
    };
  }
  if (scope === 'store') {
    return { scope, profile, store: string(item.store, `${at}.store`), error };
  }
  throw new Error(`${at}.scope is not profile or store`);
}

function decodeInventory(value: unknown, at: string): CatalogInventoryDto {
  const item = record(value, at);
  return {
    profile: string(item.profile, `${at}.profile`),
    accountsComplete: bool(item.accountsComplete, `${at}.accountsComplete`),
    teamsComplete: bool(item.teamsComplete, `${at}.teamsComplete`),
  };
}

export function decodeCatalog(value: unknown): CatalogDto {
  const item = record(value, 'list_catalog response');
  const metadata =
    item.localMetadata === undefined
      ? undefined
      : record(item.localMetadata, 'catalog metadata');
  return {
    ...(metadata
      ? {
          localMetadata: {
            accounts: decodeAccounts(metadata.accounts),
            profiles: array(
              metadata.profiles,
              'catalog metadata profiles',
              (value, at) => {
                const profile = record(value, at);
                const id = string(profile.profile, `${at}.profile`);
                const status =
                  profile.status === null
                    ? null
                    : decodeServerStatus(profile.status);
                if (status && status.profile !== id)
                  throw new Error(
                    'Catalog status belongs to a different profile.',
                  );
                return {
                  profile: id,
                  label: nullableString(profile.label, `${at}.label`),
                  configuredProbe: string(
                    profile.configuredProbe,
                    `${at}.configuredProbe`,
                  ),
                  status,
                  error:
                    profile.error === null
                      ? null
                      : decodeCommandError(profile.error, `${at}.error`),
                };
              },
            ),
          },
        }
      : {}),
    ...(item.storeReads === undefined
      ? {}
      : {
          storeReads: array(item.storeReads, 'storeReads', (value, at) => {
            const read = record(value, at);
            const state = string(read.state, `${at}.state`);
            if (
              state !== 'not-loaded' &&
              state !== 'complete' &&
              state !== 'failed'
            )
              throw new Error(`${at}.state is invalid`);
            return { store: string(read.store, `${at}.store`), state };
          }),
        }),
    ...(item.fullItemReads === undefined
      ? {}
      : { fullItemReads: array(item.fullItemReads, 'fullItemReads', string) }),
    profiles: array(item.profiles, 'profiles', string),
    stores: array(item.stores, 'stores', decodeStore),
    knownStores: array(item.knownStores, 'knownStores', decodeStore),
    inventory: array(item.inventory, 'inventory', decodeInventory),
    // Symlinks are a protocol node the app no longer shows or creates.
    items: array(item.items, 'items', decodeItem).filter(
      (entry) => entry.kind !== 'Link',
    ),
    failures: array(item.failures, 'failures', decodeFailure),
    blockedProfiles: array(item.blockedProfiles, 'blockedProfiles', string),
    ...(item.generation === undefined
      ? {}
      : { generation: integer(item.generation, 'generation') }),
  };
}

export function decodeReadItem(value: unknown): ReadItemResponse {
  const item = record(value, 'read_item response');
  return {
    store: string(item.store, 'read_item.store'),
    path: string(item.path, 'read_item.path'),
    version: integer(item.version, 'read_item.version'),
    value: string(item.value, 'read_item.value'),
  };
}

export function decodeCopy(value: unknown): CopyResponse {
  const item = record(value, 'copy response');
  if (item.ok !== true) throw new Error('copy response.ok must be true');
  return { ok: true };
}

export function decodeDownload(value: unknown): DownloadResponse {
  const item = record(value, 'download_file response');
  return { saved: bool(item.saved, 'download_file.saved') };
}

export function decodeMutation(value: unknown): MutationResponse {
  const item = record(value, 'mutation response');
  return { applied: bool(item.applied, 'mutation response.applied') };
}
