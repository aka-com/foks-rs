import type {
  Account,
  ItemShape,
  LoginFieldName,
  NodeKind,
  StoreRef,
  TeamCreationPhase,
} from '../model';
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
  optionalString,
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
  direntId: string;
  store: StoreRef;
  path: string;
  kind: NodeKind;
  size: number | null;
  version: number;
  read: RoleDto;
  write: RoleDto;
  /** Whether a small file holds a login; absent for other node types. */
  shape?: ItemShape;
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
      configuredEndpoint: string;
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

/** One field of an item for `copy_item_field`; `value` is the whole text. */
export type ItemField = LoginFieldName | 'value';

export interface CopyFieldRequest extends ItemRequest {
  field: ItemField;
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

/**
 * The response to an item copy. The host clears the clipboard
 * `clearAfterSeconds` after the write unless it no longer holds the copied
 * value.
 */
export interface ClipboardCopyResponse {
  ok: true;
  clearAfterSeconds: number;
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

export interface CreateFolderRequest extends CreateRoleRequest {
  storeId: StoreRef;
  path: string;
}

export type CreateFileRequest = CreateFolderRequest;

export interface EditTextRequest {
  direntId: string;
  storeId: StoreRef;
  path: string;
  value: string;
  /** Item version expected by the write to prevent concurrent overwrites. */
  version: number;
}

export type MoveItemRequest = ItemRequest & {
  destination: string;
  direntId: string;
};
export type RemoveItemRequest = ItemRequest & { direntId: string };
/**
 * Copies the selected item into another store at `targetPath`. Roles are
 * given together for a team and omitted for a personal vault, as for a new
 * item.
 */
export type CopyItemToStoreRequest = RemoveItemRequest &
  CreateRoleRequest & {
    targetStoreId: StoreRef;
    targetPath: string;
  };
export type RestoreTrashRequest = RemoveItemRequest & { destination: string };
export type PermanentRemoveRequest = RemoveItemRequest & {
  /**
   * Removes an item outside Trash. The host allows it only for a file in a
   * team that has no `/.trash`, which members other than owners cannot
   * create.
   */
  outsideTrash?: boolean;
};

/** One Trash entry, bound to the dirent and version the catalog listed. */
export interface TrashEntrySelection {
  path: string;
  version: number;
  direntId: string;
}

export interface EmptyTrashRequest {
  storeId: StoreRef;
  entries: TrashEntrySelection[];
}

export interface TrashMutationResponse {
  applied: boolean;
  /** Where the entry is now: its Trash path, or the restored path. */
  path: string;
}

export interface ImportDroppedFileRequest extends CreateRoleRequest {
  storeId: StoreRef;
  path: string;
  /** Path authorized by a native file picker or drop event. */
  sourcePath: string;
}

export interface ReplaceDroppedFileRequest extends ItemRequest {
  direntId: string;
  /** Path authorized by a native file picker or drop event. */
  sourcePath: string;
}

/**
 * One login of a password import: the path component it is saved under and
 * the folders below the destination, `/`-separated and empty at the top.
 * Both are encoded as stored. No field value is part of it.
 */
export interface PasswordImportEntry {
  name: string;
  folder: string;
  /** Longer than a small file holds, so saved as a file, a Document. */
  document: boolean;
}

/** A row of the file that is not imported, counting the header as row 1. */
export interface PasswordImportSkip {
  row: number;
  name?: string;
  reason: string;
}

/**
 * What the host read from a CSV export it holds under `id`. The file's
 * path and every field value stay in the host.
 */
export interface PasswordImportPreview {
  id: string;
  fileName: string;
  entries: PasswordImportEntry[];
  skipped: PasswordImportSkip[];
  /** Headers of the columns that are not imported, such as a TOTP column. */
  ignoredColumns: string[];
}

export type PasswordImportStatus =
  'created' | 'exists' | 'failed' | 'uncertain' | 'not-started';

/** The outcome of the entry at `index` of the preview. */
export interface PasswordImportOutcome {
  index: number;
  status: PasswordImportStatus;
  message?: string;
}

/** What an export of a store's logins wrote. */
export interface LoginExportResult {
  /** False when the save dialog was cancelled. */
  saved: boolean;
  exported: number;
  /** Logins left out because their text is not UTF-8. */
  skipped: number;
}

/**
 * Saves the held import's `entries` (indexes into its preview) under
 * `folder` of a store. Roles are given together for a team and omitted for
 * a personal vault, as for a new item.
 */
export interface CommitPasswordImportRequest extends CreateRoleRequest {
  importId: string;
  entries: readonly number[];
  storeId: StoreRef;
  folder: string;
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

const NODE_KINDS = {
  Secret: true,
  File: true,
  Folder: true,
} as const satisfies Record<NodeKind, true>;

function isNodeKind(kind: string): kind is NodeKind {
  return Object.hasOwn(NODE_KINDS, kind);
}

/**
 * Decodes one catalog item, or `null` for a protocol symlink. The agent
 * reports a symlink as `Link`; its fields are validated like any other node's,
 * and then it is dropped, because the app neither shows nor creates symlinks.
 */
function decodeItem(value: unknown, at: string): ItemDto | null {
  const item = record(value, at);
  const kind = string(item.kind, `${at}.kind`);
  if (kind !== 'Link' && !isNodeKind(kind)) {
    throw new Error(`${at}.kind is not a FOKS node kind`);
  }
  const decoded = {
    store: string(item.store, `${at}.store`),
    path: string(item.path, `${at}.path`),
    direntId: string(item.direntId, `${at}.direntId`),
    size: nullableInteger(item.size, `${at}.size`),
    version: integer(item.version, `${at}.version`),
    read: decodeRole(item.read, `${at}.read`),
    write: decodeRole(item.write, `${at}.write`),
    ...decodeShape(item.shape, `${at}.shape`),
  };
  return isNodeKind(kind) ? { ...decoded, kind } : null;
}

function decodeShape(value: unknown, at: string): { shape?: ItemShape } {
  if (value === undefined) return {};
  if (value !== 'login' && value !== 'other')
    throw new Error(`${at} is not login or other`);
  return { shape: value };
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
                  configuredEndpoint: string(
                    profile.configuredEndpoint,
                    `${at}.configuredEndpoint`,
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
    items: array(item.items, 'items', decodeItem).filter(
      (entry): entry is ItemDto => entry !== null,
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

export function decodeClipboardCopy(value: unknown): ClipboardCopyResponse {
  const item = record(value, 'clipboard copy response');
  if (item.ok !== true)
    throw new Error('clipboard copy response.ok must be true');
  return {
    ok: true,
    clearAfterSeconds: integer(
      item.clearAfterSeconds,
      'clipboard copy response.clearAfterSeconds',
    ),
  };
}

export function decodeDownload(value: unknown): DownloadResponse {
  const item = record(value, 'download_file response');
  return { saved: bool(item.saved, 'download_file.saved') };
}

export function decodeTrashMutation(value: unknown): TrashMutationResponse {
  const item = record(value, 'trash mutation response');
  return {
    applied: bool(item.applied, 'trash mutation response.applied'),
    path: string(item.path, 'trash mutation response.path'),
  };
}

export function decodeMutation(value: unknown): MutationResponse {
  const item = record(value, 'mutation response');
  return { applied: bool(item.applied, 'mutation response.applied') };
}

export function decodePasswordImportPreview(
  value: unknown,
  at = 'password import preview',
): PasswordImportPreview {
  const preview = record(value, at);
  return {
    id: string(preview.id, `${at}.id`),
    fileName: string(preview.fileName, `${at}.fileName`),
    entries: array(preview.entries, `${at}.entries`, (entry, where) => {
      const item = record(entry, where);
      return {
        name: string(item.name, `${where}.name`),
        folder: string(item.folder, `${where}.folder`),
        document: bool(item.document, `${where}.document`),
      };
    }),
    skipped: array(preview.skipped, `${at}.skipped`, (entry, where) => {
      const item = record(entry, where);
      const name = optionalString(item.name, `${where}.name`);
      return {
        row: integer(item.row, `${where}.row`),
        ...(name === undefined ? {} : { name }),
        reason: string(item.reason, `${where}.reason`),
      };
    }),
    ignoredColumns: array(
      preview.ignoredColumns,
      `${at}.ignoredColumns`,
      string,
    ),
  };
}

const IMPORT_STATUSES: readonly PasswordImportStatus[] = [
  'created',
  'exists',
  'failed',
  'uncertain',
  'not-started',
];

export function decodePasswordImportOutcome(
  value: unknown,
  at = 'password import outcome',
): PasswordImportOutcome {
  const item = record(value, at);
  const status = string(item.status, `${at}.status`);
  if (!IMPORT_STATUSES.includes(status as PasswordImportStatus))
    throw new Error(`${at}.status is not a known import status`);
  const message = optionalString(item.message, `${at}.message`);
  return {
    index: integer(item.index, `${at}.index`),
    status: status as PasswordImportStatus,
    ...(message === undefined ? {} : { message }),
  };
}

export function decodeLoginExport(value: unknown): LoginExportResult {
  const item = record(value, 'export_logins response');
  return {
    saved: bool(item.saved, 'export_logins.saved'),
    exported: integer(item.exported, 'export_logins.exported'),
    skipped: integer(item.skipped, 'export_logins.skipped'),
  };
}
