/**
 * The only seam between the FOKS webview and the local agent.
 *
 * Tauri's generic `invoke<T>` does not validate types at runtime. Native
 * responses pass through runtime decoders before entering the UI state.
 */

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { itemKey, parseRole } from './model';
import type {
  AgentStatus,
  Account,
  GroupDetailFailure,
  Item,
  NodeKind,
  Party,
  FederationEntry,
  Notification,
  RoleWire,
  Server,
  Store,
  StoreRef,
  World,
} from './model';
import { serverLeaseState } from './model';

declare global {
  interface Window {
    /** Injected by the Tauri runtime even with `withGlobalTauri: false`. */
    __TAURI_INTERNALS__?: unknown;
  }
}

export interface CommandError {
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
    reason?: string;
  };
}

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

export interface StoreDto {
  id: StoreRef;
  kind: 'account' | 'team';
  name: string;
  /** Agent profile name; this is the configured, displayable server name. */
  server: string;
  account: string;
  alias?: string;
  active?: boolean;
  team_kind?: 'named' | 'adhoc';
  team_id_hex?: string;
}

export interface ItemDto {
  store: StoreRef;
  path: string;
  kind: NodeKind;
  size: number;
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
  profiles: string[];
  stores: StoreDto[];
  knownStores: StoreDto[];
  inventory: CatalogInventoryDto[];
  items: ItemDto[];
  failures: CatalogFailureDto[];
  blockedProfiles: string[];
}

export interface AccountDto {
  store: StoreRef;
  profile: string;
  alias: string;
  username: string;
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
  /** Path registered from a native file drop event. */
  sourcePath: string;
}

export interface ReplaceDroppedFileRequest extends ItemRequest {
  /** Path registered from a native file drop event. */
  sourcePath: string;
}

export interface DropHoverEvent {
  hovering: boolean;
}

export interface WindowStateEvent {
  maximized: boolean;
  fullscreen: boolean;
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

export interface ExpelFederatedGroupRequest {
  storeId: StoreRef;
  remoteHostIdHex: string;
  remoteTeamIdHex: string;
}

export interface CheckedProfileResponse {
  profile: string;
  /** Indicates whether the server identity was newly added, updated, or unchanged. */
  acceptance: 'inserted' | 'advanced' | 'unchanged';
  lookupName: string;
  canonicalName: string;
  hostId: string;
  chain: number;
  epoch: number;
}

export type PendingOperationKind =
  | 'account-signup'
  | 'device-provision'
  | 'pairing-offer'
  | 'pairing-acceptance'
  | 'account-recovery'
  | 'yubi-enrollment'
  | 'team-creation'
  | 'team-member-addition'
  | 'team-member-edit'
  | 'federation-expulsion'
  | 'team-rekey';

export interface PendingOperation {
  kind: PendingOperationKind;
  alias: string;
  target?: string;
}

export interface FirstRunAccountRequest {
  profile: string;
  alias: string;
  username: string;
  deviceName: string;
  email: string;
  invite: string;
  passphrase?: string;
  passphraseConfirmation?: string;
}

export interface FirstRunPassphraseRequest {
  profile: string;
  alias: string;
  passphrase: string;
  confirmation: string;
}

export interface BackupPhraseResponse {
  backupAlias: string;
  /** Ephemeral recovery phrase; should not be cached or persisted. */
  phrase: string;
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
}

export interface StoredHost {
  lookupName: string;
  canonicalName: string;
  hostId: string;
  chain: number;
  epoch: number;
}

export interface ServerStatusSnapshot {
  profile: string;
  configuredProbe: string;
  host: StoredHost | null;
  /** Whether this server requires a compatibility session lease. */
  leaseRequired: boolean;
  /** Expiration timestamp of the signed lease in Unix seconds, or null if unleased. */
  leaseExpiresAt: number | null;
}

export interface AppInfo {
  version: string;
  agentSocket: string;
  /** Profile prepared and authenticated by the launcher before opening the app. */
  managedProfile?: string;
  /** macOS Computer Name configured in System Settings. */
  computerName?: string;
}

export interface GoProfileCandidate {
  candidateId: string;
  username?: string;
  serverHint?: string;
  hostId: string;
  userId: string;
  deviceId: string;
  role: string;
  storageKind:
    | 'plaintext'
    | 'passphrase'
    | 'macos-keychain'
    | 'noise-file'
    | 'generic-keychain';
  hidden: boolean;
  provisional: boolean;
  pairable: boolean;
  copyable: boolean;
}

export interface GoProfileDiscovery {
  installed: boolean;
  candidates: GoProfileCandidate[];
}

export interface AppLockState {
  locked: boolean;
  available: boolean;
  mechanism: 'biometry' | 'password' | 'none';
  unavailableReason?: string;
}

export interface CheckedServer extends StoredHost {
  profile: string;
  acceptance: 'inserted' | 'advanced' | 'unchanged';
}

export interface AccountDevice {
  id: string;
  /** Authenticated device name reported by the agent, if available. */
  name?: string;
  role: 'owner' | 'admin' | 'member';
  current: boolean;
}

export interface BackupEnrollment {
  backupAlias: string;
  accountAlias: string;
  backupId: string;
}

export interface BackupRevocation extends BackupEnrollment {
  userChainSequence: number;
  alreadyAbsent: boolean;
  removedLocalEnrollment: true;
}

export interface PairingOffer {
  accountAlias: string;
  /** Ephemeral pairing phrase; not persisted. */
  phrase: string;
}

export interface DeviceProvision {
  alias: string;
  deviceId: string;
  userChainSequence: number;
}

export interface PassphraseReport {
  generation: number;
  stretchVersion: 'v1';
  verified: true;
}

export interface ResetPreview {
  profile: string;
  resumables: PendingOperation[];
  artifacts: { kind: string; entries: number; bytes: number }[];
  /** Single-use confirmation token valid only for this reset preview. */
  token: string;
  expiresInSeconds: number;
}

export interface YubiEnrollment {
  alias: string;
  state: 'pending' | 'complete';
}

export interface CreateYubiAccountRequest {
  profile: string;
  alias: string;
  username: string;
  deviceName: string;
  email: string;
  invite: string;
  passphrase?: string;
  passphraseConfirmation?: string;
  cardSerial: number;
  signingSlot: number;
  pqSlot: number;
  pin: string;
  puk: string;
  pinAttempts: number;
  pukAttempts: number;
}

export interface ProvisionYubiDeviceRequest {
  accountStoreId: StoreRef;
  targetAlias: string;
  deviceName: string;
  cardSerial: number;
  signingSlot: number;
  pqSlot: number;
  pin: string;
  puk: string;
  pinAttempts: number;
  pukAttempts: number;
}

export type YubiCommand =
  | { command: 'create_yubi_account'; args: CreateYubiAccountRequest }
  | {
      command: 'resume_yubi_account';
      args: { profile: string; alias: string; pin: string };
    }
  | { command: 'provision_yubi_device'; args: ProvisionYubiDeviceRequest }
  | {
      command: 'sync_yubi_account';
      args: {
        profile: string;
        alias: string;
        pin: string;
        withFederation: boolean;
      };
    }
  | { command: 'yubi_pin_status'; args: { profile: string; alias: string } }
  | {
      command: 'change_yubi_pin';
      args: { profile: string; alias: string; oldPin: string; newPin: string };
    }
  | {
      command: 'set_yubi_passphrase' | 'change_yubi_passphrase';
      args: {
        profile: string;
        alias: string;
        pin: string;
        passphrase: string;
        confirmation: string;
      };
    }
  | {
      command: 'verify_yubi_passphrase';
      args: { profile: string; alias: string; pin: string; passphrase: string };
    }
  | {
      command: 'change_yubi_puk';
      args: { profile: string; alias: string; oldPuk: string; newPuk: string };
    }
  | {
      command: 'unblock_yubi_pin';
      args: { profile: string; alias: string; puk: string; newPin: string };
    }
  | {
      command: 'rotate_yubi_management_key';
      args: { profile: string; alias: string; pin: string };
    }
  | {
      command: 'resume_yubi_management_key';
      args: { profile: string; alias: string; pin?: string };
    }
  | {
      command: 'recover_yubi_management_key';
      args: { accountStoreId: StoreRef; yubiAlias: string };
    }
  | {
      command: 'recover_yubi_subkey';
      args: { profile: string; alias: string; pin: string };
    }
  | {
      command: 'revoke_yubi_device';
      args: {
        accountStoreId: StoreRef;
        yubiAlias: string;
        confirmation: string;
      };
    };

/** Mock fixtures used for local development and testing. */
export interface FirstRunFixturePath {
  profile: string;
  accountAlias: string;
  server: string;
  typo: string;
  username: string;
  deviceName: string;
  admin?: string;
  groupName?: string;
  groupAlias?: string;
  groupTeamIdHex?: string;
  report: CheckedProfileResponse;
}

export interface FirstRunFixture {
  invited: FirstRunFixturePath;
  own: FirstRunFixturePath;
  backupPhrase: string;
}

export type Unlisten = () => void;

export interface Bridge {
  readonly native: boolean;
  /** Present only on the development bridge, never on the native bridge. */
  readonly fixtureWorld?: World;
  readonly firstRunFixture?: FirstRunFixture;
  appLockState(): Promise<AppLockState>;
  windowState(): Promise<WindowStateEvent>;
  lockApp(): Promise<AppLockState>;
  unlockApp(): Promise<AppLockState>;
  agentStatus(): Promise<AgentStatus>;
  appInfo(): Promise<AppInfo>;
  /** Returns the full catalog including stores. Mutually exclusive with `listStores`. */
  listCatalog(): Promise<CatalogDto>;
  /** Returns store metadata only. Mutually exclusive with `listCatalog`. */
  listStores(): Promise<CatalogDto>;
  listServers(): Promise<Server[]>;
  listAccounts(): Promise<Account[]>;
  listGroupDetails(storeId: StoreRef): Promise<GroupDetailsDto>;
  listParties(storeId: StoreRef): Promise<Party[]>;
  listFederation(storeId: StoreRef): Promise<FederationEntry[]>;
  readItem(request: ItemRequest): Promise<ReadItemResponse>;
  copyItemValue(request: ItemRequest): Promise<CopyResponse>;
  copyItemPath(request: ItemRequest): Promise<CopyResponse>;
  downloadFile(request: ItemRequest): Promise<DownloadResponse>;
  createTextItem(request: CreateTextRequest): Promise<MutationResponse>;
  createLink(request: CreateLinkRequest): Promise<MutationResponse>;
  createFolder(request: CreateFolderRequest): Promise<MutationResponse>;
  editTextItem(request: EditTextRequest): Promise<MutationResponse>;
  removeItem(request: RemoveItemRequest): Promise<MutationResponse>;
  importDroppedFile(
    request: ImportDroppedFileRequest,
  ): Promise<MutationResponse>;
  pickAndImportFile(request: CreateFileRequest): Promise<MutationResponse>;
  replaceDroppedFile(
    request: ReplaceDroppedFileRequest,
  ): Promise<MutationResponse>;
  pickAndReplaceFile(request: ItemRequest): Promise<MutationResponse>;
  resumeGroupCreation(storeId: StoreRef): Promise<MutationResponse>;
  takeAgentConnectionLoss(): Promise<string | null>;
  retryAgentConnection(): Promise<AgentStatus>;
  createGroup(request: CreateGroupRequest): Promise<MutationResponse>;
  addGroupMember(request: GroupRoleRequest): Promise<MutationResponse>;
  resumeGroupMemberAddition(
    request: GroupMemberRequest,
  ): Promise<MutationResponse>;
  demoteGroupMember(request: GroupRoleRequest): Promise<MutationResponse>;
  removeGroupMember(request: GroupMemberRequest): Promise<MutationResponse>;
  resumeGroupMemberEdit(storeId: StoreRef): Promise<MutationResponse>;
  admitGroup(request: AdmitGroupRequest): Promise<MutationResponse>;
  expelFederatedGroup(
    request: ExpelFederatedGroupRequest,
  ): Promise<MutationResponse>;
  rerunGroupAdmission(
    storeId: StoreRef,
    operationId: string,
  ): Promise<MutationResponse>;
  copyText(text: string): Promise<CopyResponse>;
  initializeClientState(): Promise<AgentStatus>;
  discoverGoProfiles(): Promise<GoProfileDiscovery>;
  checkAndAddProfile(
    profileName: string,
    probe: string,
  ): Promise<CheckedProfileResponse>;
  checkAndAddGoProfile(
    candidateId: string,
    hostId: string,
    profileName: string,
    probe: string,
  ): Promise<CheckedProfileResponse>;
  listPendingOperations(profile: string): Promise<PendingOperation[]>;
  createFirstRunAccount(
    request: FirstRunAccountRequest,
  ): Promise<MutationResponse>;
  resumeFirstRunAccount(
    profile: string,
    alias: string,
  ): Promise<MutationResponse>;
  setFirstRunPassphrase(
    request: FirstRunPassphraseRequest,
  ): Promise<MutationResponse>;
  prepareOwnerBackup(
    profile: string,
    accountAlias: string,
    backupAlias: string,
  ): Promise<BackupPhraseResponse>;
  commitOwnerBackup(
    profile: string,
    accountAlias: string,
    backupAlias: string,
    phrase: string,
  ): Promise<MutationResponse>;
  recoverOwnerAccount(
    profile: string,
    targetAlias: string,
    phrase: string,
    deviceName: string,
  ): Promise<MutationResponse>;
  resumeOwnerRecovery(
    profile: string,
    targetAlias: string,
    phrase: string,
    deviceName: string,
  ): Promise<MutationResponse>;
  discoverGroups(
    profile: string,
    accountAlias: string,
  ): Promise<GroupDiscoveryResponse>;
  describeServerStatus(profile: string): Promise<ServerStatusSnapshot>;
  checkServer(profile: string): Promise<CheckedServer>;
  addServer(
    profileName: string,
    probe: string,
  ): Promise<{ profile: string; configuredProbe: string }>;
  forgetServer(
    profile: string,
    confirmation: string,
  ): Promise<{ profile: string; removed: true }>;
  listAccountDevices(accountStoreId: StoreRef): Promise<AccountDevice[]>;
  removeAccountDevice(
    accountStoreId: StoreRef,
    deviceId: string,
  ): Promise<{
    deviceId: string;
    userChainSequence: number;
    alreadyAbsent: boolean;
  }>;
  listBackupEnrollments(accountStoreId: StoreRef): Promise<BackupEnrollment[]>;
  revokeOwnerBackup(
    accountStoreId: StoreRef,
    backup: BackupEnrollment,
    confirmation: string,
  ): Promise<BackupRevocation>;
  startDevicePairing(accountStoreId: StoreRef): Promise<PairingOffer>;
  resumeDevicePairingOffer(accountStoreId: StoreRef): Promise<PairingOffer>;
  finishDevicePairing(accountStoreId: StoreRef): Promise<DeviceProvision>;
  acceptDevicePairing(
    profile: string,
    targetAlias: string,
    deviceName: string,
    phrase: string,
  ): Promise<DeviceProvision>;
  acceptGoProfilePairing(
    candidateId: string,
    profile: string,
    targetAlias: string,
    deviceName: string,
    phrase: string,
  ): Promise<DeviceProvision>;
  resumeDevicePairingAcceptance(
    profile: string,
    targetAlias: string,
  ): Promise<DeviceProvision>;
  resumeGoProfilePairing(
    candidateId: string,
    profile: string,
    targetAlias: string,
  ): Promise<DeviceProvision>;
  copyGoProfileDevice(
    candidateId: string,
    profile: string,
    targetAlias: string,
  ): Promise<DeviceProvision>;
  setAccountPassphrase(
    accountStoreId: StoreRef,
    passphrase: string,
    confirmation: string,
  ): Promise<PassphraseReport>;
  changeAccountPassphrase(
    accountStoreId: StoreRef,
    passphrase: string,
    confirmation: string,
  ): Promise<PassphraseReport>;
  verifyAccountPassphrase(
    accountStoreId: StoreRef,
    passphrase: string,
  ): Promise<PassphraseReport>;
  describeReset(profile: string): Promise<ResetPreview>;
  resetServer(
    profile: string,
    confirmation: string,
    token: string,
  ): Promise<MutationResponse>;
  listYubiCards(profile: string): Promise<{ serial: number }[]>;
  listYubiAccounts(profile: string): Promise<YubiEnrollment[]>;
  runYubi(command: YubiCommand): Promise<Record<string, unknown>>;
  onDropHover(listener: (event: DropHoverEvent) => void): Promise<Unlisten>;
  onDropPaths(listener: (paths: string[]) => void): Promise<Unlisten>;
  onWindowState(listener: (event: WindowStateEvent) => void): Promise<Unlisten>;
}

const pendingServerStatuses = new WeakMap<
  Bridge,
  Map<string, Promise<ServerStatusSnapshot>>
>();
const profileWork = new WeakMap<Bridge, Map<string, Promise<void>>>();

/**
 * Queues profile-scoped requests sequentially to prevent concurrent
 * requests from failing with `profile-busy`.
 */
export function enqueueProfileWork<T>(
  bridge: Bridge,
  profile: string,
  work: () => Promise<T>,
): Promise<T> {
  let queues = profileWork.get(bridge);
  if (!queues) {
    queues = new Map();
    profileWork.set(bridge, queues);
  }
  const previous = queues.get(profile) ?? Promise.resolve();
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  queues.set(profile, gate);
  return previous
    .catch(() => undefined)
    .then(async () => {
      try {
        return await work();
      } finally {
        release();
      }
    });
}

export function sharedServerStatus(
  bridge: Bridge,
  profile: string,
): Promise<ServerStatusSnapshot> {
  let statuses = pendingServerStatuses.get(bridge);
  if (!statuses) {
    statuses = new Map();
    pendingServerStatuses.set(bridge, statuses);
  }
  const active = statuses.get(profile);
  if (active) return active;
  const pending = enqueueProfileWork(bridge, profile, () =>
    bridge.describeServerStatus(profile),
  );
  statuses.set(profile, pending);
  const clear = (): void => {
    if (statuses.get(profile) === pending) statuses.delete(profile);
  };
  void pending.then(clear, clear);
  return pending;
}

function record(value: unknown, at: string): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error(`${at} must be an object`);
  }
  return value as Record<string, unknown>;
}

function string(value: unknown, at: string): string {
  if (typeof value !== 'string') throw new Error(`${at} must be a string`);
  return value;
}

function entityId(
  value: unknown,
  prefix: '01' | '02' | '03' | '04' | '0d' | '10' | '14',
  at: string,
): string {
  const result = string(value, at);
  if (!new RegExp(`^${prefix}[0-9a-f]{64}$`).test(result)) {
    throw new Error(`${at} must be a canonical ${prefix} entity id`);
  }
  return result;
}

function deviceMemberId(value: unknown, at: string): string {
  const result = string(value, at);
  if (!/^04[0-9a-f]{64}$/.test(result) && !/^08[0-9a-f]{66}$/.test(result)) {
    throw new Error(`${at} must be a canonical software-device or YubiKey id`);
  }
  return result;
}

function bool(value: unknown, at: string): boolean {
  if (typeof value !== 'boolean') throw new Error(`${at} must be a boolean`);
  return value;
}

function integer(value: unknown, at: string): number {
  if (!Number.isSafeInteger(value) || (value as number) < 0) {
    throw new Error(`${at} must be a non-negative safe integer`);
  }
  return value as number;
}

function signed16(value: unknown, at: string): number {
  if (
    !Number.isSafeInteger(value) ||
    (value as number) < -32768 ||
    (value as number) > 32767
  ) {
    throw new Error(`${at} must be a signed 16-bit integer`);
  }
  return value as number;
}

function array<T>(
  value: unknown,
  at: string,
  decode: (entry: unknown, at: string) => T,
): T[] {
  if (!Array.isArray(value)) throw new Error(`${at} must be an array`);
  return value.map((entry, index) => decode(entry, `${at}[${index}]`));
}

function decodeRole(value: unknown, at: string): RoleDto {
  const item = record(value, at);
  const role = string(item.role, `${at}.role`);
  if (role === 'Member') {
    return { role, visibility: signed16(item.visibility, `${at}.visibility`) };
  }
  if (role === 'Admin' || role === 'Owner') return { role };
  throw new Error(`${at}.role is not Member, Admin, or Owner`);
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
    team_kind: teamKind,
    team_id_hex: string(item.team_id_hex, `${at}.team_id_hex`),
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
    size: integer(item.size, `${at}.size`),
    version: integer(item.version, `${at}.version`),
    read: decodeRole(item.read, `${at}.read`),
    write: decodeRole(item.write, `${at}.write`),
  };
}

function nullableString(value: unknown, at: string): string | null {
  return value === null ? null : string(value, at);
}

function decodeConnectionLoss(value: unknown): string | null {
  return nullableString(value, 'take_agent_connection_loss response');
}

function nullableInteger(value: unknown, at: string): number | null {
  return value === null ? null : integer(value, at);
}

function optionalString(value: unknown, at: string): string | undefined {
  return value === undefined ? undefined : string(value, at);
}

function decodeServer(value: unknown, at: string): Server {
  const item = record(value, at);
  const state = string(item.state, `${at}.state`);
  if (!['ok', 'lease-lapsed', 'never-probed', 'blocked'].includes(state)) {
    throw new Error(`${at}.state is not a server state`);
  }
  if (item.lease !== null)
    throw new Error(`${at}.lease must be null when no lease is active`);
  return {
    id: string(item.id, `${at}.id`),
    name: string(item.name, `${at}.name`),
    label: nullableString(item.label, `${at}.label`),
    host_id: nullableString(item.host_id, `${at}.host_id`),
    chain: nullableInteger(item.chain, `${at}.chain`),
    epoch: nullableInteger(item.epoch, `${at}.epoch`),
    lease: null,
    accounts: array(item.accounts, `${at}.accounts`, string),
    state: state as Server['state'],
  };
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

function decodeErrorDetails(
  value: unknown,
  at: string,
): CommandError['details'] {
  if (value === undefined) return undefined;
  const item = record(value, at);
  return {
    kind: optionalString(item.kind, `${at}.kind`),
    operation: optionalString(item.operation, `${at}.operation`),
    capability: optionalString(item.capability, `${at}.capability`),
    profile: optionalString(item.profile, `${at}.profile`),
    reason: optionalString(item.reason, `${at}.reason`),
  };
}

function decodeCommandError(value: unknown, at: string): CommandError {
  const item = record(value, at);
  const details = decodeErrorDetails(item.details, `${at}.details`);
  return {
    code: string(item.code, `${at}.code`),
    message: string(item.message, `${at}.message`),
    retryable: bool(item.retryable, `${at}.retryable`),
    ambiguous: bool(item.ambiguous, `${at}.ambiguous`),
    fatal: bool(item.fatal, `${at}.fatal`),
    ...(details ? { details } : {}),
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

export const decodeServers = (value: unknown): Server[] =>
  array(value, 'list_servers response', decodeServer);
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
  return {
    profiles: array(item.profiles, 'profiles', string),
    stores: array(item.stores, 'stores', decodeStore),
    knownStores: array(item.knownStores, 'knownStores', decodeStore),
    inventory: array(item.inventory, 'inventory', decodeInventory),
    items: array(item.items, 'items', decodeItem),
    failures: array(item.failures, 'failures', decodeFailure),
    blockedProfiles: array(item.blockedProfiles, 'blockedProfiles', string),
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

export function decodeCheckedProfile(value: unknown): CheckedProfileResponse {
  const item = record(value, 'check_and_add_profile response');
  const acceptance = string(
    item.acceptance,
    'check_and_add_profile.acceptance',
  );
  if (
    acceptance !== 'inserted' &&
    acceptance !== 'advanced' &&
    acceptance !== 'unchanged'
  ) {
    throw new Error('check_and_add_profile.acceptance is invalid');
  }
  return {
    profile: string(item.profile, 'check_and_add_profile.profile'),
    acceptance,
    lookupName: string(item.lookupName, 'check_and_add_profile.lookupName'),
    canonicalName: string(
      item.canonicalName,
      'check_and_add_profile.canonicalName',
    ),
    hostId: entityId(item.hostId, '02', 'check_and_add_profile.hostId'),
    chain: integer(item.chain, 'check_and_add_profile.chain'),
    epoch: integer(item.epoch, 'check_and_add_profile.epoch'),
  };
}

const PENDING_KINDS: readonly PendingOperationKind[] = [
  'account-signup',
  'device-provision',
  'pairing-offer',
  'pairing-acceptance',
  'account-recovery',
  'yubi-enrollment',
  'team-creation',
  'team-member-addition',
  'team-member-edit',
  'federation-expulsion',
  'team-rekey',
];

function decodePendingOperation(value: unknown, at: string): PendingOperation {
  const item = record(value, at);
  const kind = string(item.kind, `${at}.kind`);
  if (!(PENDING_KINDS as readonly string[]).includes(kind)) {
    throw new Error(`${at}.kind is not a pending-operation kind`);
  }
  return {
    kind: kind as PendingOperationKind,
    alias: string(item.alias, `${at}.alias`),
    target: optionalString(item.target, `${at}.target`),
  };
}

export const decodePendingOperations = (value: unknown): PendingOperation[] =>
  array(value, 'list_pending_operations response', decodePendingOperation);

export function decodeBackupPhrase(value: unknown): BackupPhraseResponse {
  const item = record(value, 'prepare_owner_backup response');
  return {
    backupAlias: string(item.backupAlias, 'prepare_owner_backup.backupAlias'),
    phrase: string(item.phrase, 'prepare_owner_backup.phrase'),
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
  return { accountAlias, groups };
}

function nullable<T>(
  value: unknown,
  at: string,
  decode: (value: unknown, at: string) => T,
): T | null {
  return value === null ? null : decode(value, at);
}

function decodeStoredHost(value: unknown, at: string): StoredHost {
  const item = record(value, at);
  return {
    lookupName: string(item.lookupName, `${at}.lookupName`),
    canonicalName: string(item.canonicalName, `${at}.canonicalName`),
    hostId: entityId(item.hostId, '02', `${at}.hostId`),
    chain: integer(item.chain, `${at}.chain`),
    epoch: integer(item.epoch, `${at}.epoch`),
  };
}

export function decodeServerStatus(value: unknown): ServerStatusSnapshot {
  const item = record(value, 'describe_server_status response');
  const status = {
    profile: string(item.profile, 'describe_server_status.profile'),
    configuredProbe: string(
      item.configuredProbe,
      'describe_server_status.configuredProbe',
    ),
    host: nullable(item.host, 'describe_server_status.host', decodeStoredHost),
    leaseRequired: bool(
      item.leaseRequired,
      'describe_server_status.leaseRequired',
    ),
    leaseExpiresAt: nullable(
      item.leaseExpiresAt,
      'describe_server_status.leaseExpiresAt',
      integer,
    ),
  };
  if (!status.leaseRequired && status.leaseExpiresAt !== null) {
    throw new Error(
      'describe_server_status returned an expiration timestamp for a protocol that does not use leases',
    );
  }
  return status;
}

export function decodeAppInfo(value: unknown): AppInfo {
  const item = record(value, 'app_info response');
  const managedProfile = optionalString(
    item.managedProfile,
    'app_info.managedProfile',
  );
  const computerName = optionalString(
    item.computerName,
    'app_info.computerName',
  );
  return {
    version: string(item.version, 'app_info.version'),
    agentSocket: string(item.agentSocket, 'app_info.agentSocket'),
    ...(managedProfile ? { managedProfile } : {}),
    ...(computerName ? { computerName } : {}),
  };
}

function decodeGoProfileCandidate(
  value: unknown,
  at: string,
): GoProfileCandidate {
  const item = record(value, at);
  const candidateId = string(item.candidateId, `${at}.candidateId`);
  if (!/^[0-9a-f]{64}$/.test(candidateId))
    throw new Error(`${at}.candidateId is not canonical hex`);
  const username = optionalString(item.username, `${at}.username`);
  const serverHint = optionalString(item.serverHint, `${at}.serverHint`);
  const hostId = entityId(item.hostId, '02', `${at}.hostId`);
  const userId = entityId(item.userId, '01', `${at}.userId`);
  const deviceId = string(item.deviceId, `${at}.deviceId`);
  if (
    !/^(04|10|13)[0-9a-f]{64}$/.test(deviceId) &&
    !/^08[0-9a-f]{66}$/.test(deviceId)
  )
    throw new Error(`${at}.deviceId is not a supported FOKS key id`);
  const role = string(item.role, `${at}.role`);
  const member = /^member\((-?(?:0|[1-9][0-9]{0,4}))\)$/.exec(role);
  if (
    !['none', 'admin', 'owner'].includes(role) &&
    (!member || Math.abs(Number(member[1])) > 16384)
  )
    throw new Error(`${at}.role is invalid`);
  const storageKind = string(item.storageKind, `${at}.storageKind`);
  if (
    ![
      'plaintext',
      'passphrase',
      'macos-keychain',
      'noise-file',
      'generic-keychain',
    ].includes(storageKind)
  )
    throw new Error(`${at}.storageKind is unsupported`);
  const hidden = bool(item.hidden, `${at}.hidden`);
  const provisional = bool(item.provisional, `${at}.provisional`);
  const pairable = bool(item.pairable, `${at}.pairable`);
  const copyable = bool(item.copyable, `${at}.copyable`);
  if ((hidden || provisional) && (pairable || copyable))
    throw new Error(`${at} exposes an incomplete Go profile`);
  if (
    copyable &&
    (!pairable ||
      !deviceId.startsWith('04') ||
      storageKind !== 'macos-keychain')
  )
    throw new Error(`${at} exposes an unsupported device copy`);
  return {
    candidateId,
    ...(username ? { username } : {}),
    ...(serverHint ? { serverHint } : {}),
    hostId,
    userId,
    deviceId,
    role,
    storageKind: storageKind as GoProfileCandidate['storageKind'],
    hidden,
    provisional,
    pairable,
    copyable,
  };
}

export function decodeGoProfileDiscovery(value: unknown): GoProfileDiscovery {
  const item = record(value, 'discover_go_profiles response');
  const installed = bool(item.installed, 'discover_go_profiles.installed');
  const candidates = array(
    item.candidates,
    'discover_go_profiles.candidates',
    decodeGoProfileCandidate,
  );
  if (candidates.length > 128)
    throw new Error('discover_go_profiles returned too many candidates');
  if (!installed && candidates.length)
    throw new Error(
      'discover_go_profiles returned candidates without an installation',
    );
  if (
    new Set(candidates.map((candidate) => candidate.candidateId)).size !==
    candidates.length
  )
    throw new Error('discover_go_profiles returned duplicate candidates');
  return { installed, candidates };
}

export function decodeAppLockState(value: unknown): AppLockState {
  const item = record(value, 'app_lock_state response');
  const locked = bool(item.locked, 'app_lock_state.locked');
  const available = bool(item.available, 'app_lock_state.available');
  const mechanism = string(item.mechanism, 'app_lock_state.mechanism');
  const unavailableReason = optionalString(
    item.unavailableReason,
    'app_lock_state.unavailableReason',
  );
  if (!['biometry', 'password', 'none'].includes(mechanism)) {
    throw new Error('app_lock_state.mechanism is not supported');
  }
  if (locked && !available) {
    throw new Error('app_lock_state cannot be locked when lock is unavailable');
  }
  if (
    (available && mechanism === 'none') ||
    (!available && mechanism !== 'none')
  ) {
    throw new Error('app_lock_state capability and mechanism are incompatible');
  }
  if ((!available && !unavailableReason) || (available && unavailableReason)) {
    throw new Error('app_lock_state availability reason is inconsistent');
  }
  return {
    locked,
    available,
    mechanism: mechanism as AppLockState['mechanism'],
    ...(unavailableReason === undefined ? {} : { unavailableReason }),
  };
}

export function decodeCheckedServer(value: unknown): CheckedServer {
  const item = record(value, 'check_server response');
  const acceptance = string(item.acceptance, 'check_server.acceptance');
  if (!['inserted', 'advanced', 'unchanged'].includes(acceptance)) {
    throw new Error(
      'check_server.acceptance must be inserted, advanced, or unchanged',
    );
  }
  return {
    profile: string(item.profile, 'check_server.profile'),
    acceptance: acceptance as CheckedServer['acceptance'],
    ...decodeStoredHost(item, 'check_server response'),
  };
}

const decodeAddedServer = (
  value: unknown,
): { profile: string; configuredProbe: string } => {
  const item = record(value, 'add_server response');
  return {
    profile: string(item.profile, 'add_server.profile'),
    configuredProbe: string(item.configuredProbe, 'add_server.configuredProbe'),
  };
};

const decodeForgottenServer = (
  value: unknown,
): { profile: string; removed: true } => {
  const item = record(value, 'forget_server response');
  if (item.removed !== true)
    throw new Error('forget_server.removed must be true');
  return {
    profile: string(item.profile, 'forget_server.profile'),
    removed: true,
  };
};

function decodeAccountDevice(value: unknown, at: string): AccountDevice {
  const item = record(value, at);
  const role = string(item.role, `${at}.role`);
  if (role !== 'owner' && role !== 'admin' && role !== 'member')
    throw new Error(`${at}.role is invalid`);
  return {
    id: deviceMemberId(item.id, `${at}.id`),
    name: optionalString(item.name, `${at}.name`),
    role,
    current: bool(item.current, `${at}.current`),
  };
}
export const decodeAccountDevices = (value: unknown): AccountDevice[] =>
  array(value, 'list_account_devices response', decodeAccountDevice);

function decodeBackupEnrollment(value: unknown, at: string): BackupEnrollment {
  const item = record(value, at);
  return {
    backupAlias: string(item.backupAlias, `${at}.backupAlias`),
    accountAlias: string(item.accountAlias, `${at}.accountAlias`),
    backupId: entityId(item.backupId, '10', `${at}.backupId`),
  };
}
export const decodeBackupEnrollments = (value: unknown): BackupEnrollment[] =>
  array(value, 'list_backup_enrollments response', decodeBackupEnrollment);

export function decodeBackupRevocation(value: unknown): BackupRevocation {
  const item = record(value, 'revoke_owner_backup response');
  if (item.removedLocalEnrollment !== true)
    throw new Error('revoke_owner_backup.removedLocalEnrollment must be true');
  return {
    backupAlias: string(item.backupAlias, 'revoke_owner_backup.backupAlias'),
    accountAlias: string(item.accountAlias, 'revoke_owner_backup.accountAlias'),
    backupId: entityId(item.backupId, '10', 'revoke_owner_backup.backupId'),
    userChainSequence: integer(
      item.userChainSequence,
      'revoke_owner_backup.userChainSequence',
    ),
    alreadyAbsent: bool(
      item.alreadyAbsent,
      'revoke_owner_backup.alreadyAbsent',
    ),
    removedLocalEnrollment: true,
  };
}

export function decodePairingOffer(value: unknown): PairingOffer {
  const item = record(value, 'pairing offer response');
  return {
    accountAlias: string(item.accountAlias, 'pairing offer.accountAlias'),
    phrase: string(item.phrase, 'pairing offer.phrase'),
  };
}
export function decodeDeviceProvision(value: unknown): DeviceProvision {
  const item = record(value, 'device provision response');
  return {
    alias: string(item.alias, 'device provision.alias'),
    deviceId: entityId(item.deviceId, '04', 'device provision.deviceId'),
    userChainSequence: integer(
      item.userChainSequence,
      'device provision.userChainSequence',
    ),
  };
}

export function decodeDeviceRemoval(value: unknown): {
  deviceId: string;
  userChainSequence: number;
  alreadyAbsent: boolean;
} {
  const item = record(value, 'remove_account_device response');
  return {
    deviceId: entityId(item.deviceId, '04', 'remove_account_device.deviceId'),
    userChainSequence: integer(
      item.userChainSequence,
      'remove_account_device.userChainSequence',
    ),
    alreadyAbsent: bool(
      item.alreadyAbsent,
      'remove_account_device.alreadyAbsent',
    ),
  };
}
export function decodePassphraseReport(value: unknown): PassphraseReport {
  const item = record(value, 'passphrase response');
  if (item.stretchVersion !== 'v1' || item.verified !== true)
    throw new Error('passphrase response is invalid');
  return {
    generation: integer(item.generation, 'passphrase.generation'),
    stretchVersion: 'v1',
    verified: true,
  };
}

function decodeResetArtifact(
  value: unknown,
  at: string,
): ResetPreview['artifacts'][number] {
  const item = record(value, at);
  return {
    kind: string(item.kind, `${at}.kind`),
    entries: integer(item.entries, `${at}.entries`),
    bytes: integer(item.bytes, `${at}.bytes`),
  };
}
export function decodeResetPreview(value: unknown): ResetPreview {
  const item = record(value, 'describe_reset response');
  return {
    profile: string(item.profile, 'describe_reset.profile'),
    resumables: array(
      item.resumables,
      'describe_reset.resumables',
      decodePendingOperation,
    ),
    artifacts: array(
      item.artifacts,
      'describe_reset.artifacts',
      decodeResetArtifact,
    ),
    token: string(item.token, 'describe_reset.token'),
    expiresInSeconds: integer(
      item.expiresInSeconds,
      'describe_reset.expiresInSeconds',
    ),
  };
}

export const decodeYubiCards = (value: unknown): { serial: number }[] =>
  array(value, 'list_yubi_cards response', (entry, at) => ({
    serial: integer(record(entry, at).serial, `${at}.serial`),
  }));
export const decodeYubiAccounts = (value: unknown): YubiEnrollment[] =>
  array(value, 'list_yubi_accounts response', (entry, at) => {
    const item = record(entry, at);
    const state = string(item.state, `${at}.state`);
    if (state !== 'pending' && state !== 'complete')
      throw new Error(`${at}.state is invalid`);
    return { alias: string(item.alias, `${at}.alias`), state };
  });

export function decodeYubiResult(
  command: YubiCommand['command'],
  value: unknown,
): Record<string, unknown> {
  const item = record(value, `${command} response`);
  if (
    command === 'yubi_pin_status' ||
    command === 'change_yubi_pin' ||
    command === 'unblock_yubi_pin'
  ) {
    return {
      remaining: integer(item.remaining, `${command}.remaining`),
      blocked: bool(item.blocked, `${command}.blocked`),
    };
  } else if (command.includes('passphrase')) {
    return { ...decodePassphraseReport(item) };
  } else if (
    command === 'create_yubi_account' ||
    command === 'resume_yubi_account' ||
    command === 'provision_yubi_device'
  ) {
    const yubiId = string(item.yubiId, `${command}.yubiId`);
    if (!/^08[0-9a-f]{66}$/.test(yubiId))
      throw new Error(`${command}.yubiId must be a canonical YubiKey id`);
    const subkeyId = entityId(item.subkeyId, '0d', `${command}.subkeyId`);
    return {
      alias: string(item.alias, `${command}.alias`),
      username: string(item.username, `${command}.username`),
      yubiId,
      subkeyId,
      userChainSequence: integer(
        item.userChainSequence,
        `${command}.userChainSequence`,
      ),
      managementEnrolled: bool(
        item.managementEnrolled,
        `${command}.managementEnrolled`,
      ),
    };
  } else if (command === 'sync_yubi_account') {
    return {
      username: string(item.username, `${command}.username`),
      userChainSequence: integer(
        item.userChainSequence,
        `${command}.userChainSequence`,
      ),
      directories: integer(item.directories, `${command}.directories`),
      entries: integer(item.entries, `${command}.entries`),
      federation: array(
        item.federation,
        `${command}.federation`,
        (value, at) => {
          const row = record(value, at);
          return {
            localProfile: string(row.localProfile, `${at}.localProfile`),
            localTeamAlias: string(row.localTeamAlias, `${at}.localTeamAlias`),
            refreshed: bool(row.refreshed, `${at}.refreshed`),
            deferred: nullable(row.deferred, `${at}.deferred`, string),
          };
        },
      ),
    };
  } else if (command === 'change_yubi_puk') {
    if (item.changed !== true)
      throw new Error(`${command}.changed must be true`);
    return { alias: string(item.alias, `${command}.alias`), changed: true };
  } else if (command === 'recover_yubi_subkey') {
    const subkeyId = entityId(item.subkeyId, '0d', `${command}.subkeyId`);
    return {
      alias: string(item.alias, `${command}.alias`),
      subkeyId,
      certificateCount: integer(
        item.certificateCount,
        `${command}.certificateCount`,
      ),
    };
  } else if (command === 'revoke_yubi_device') {
    return {
      alias: string(item.alias, `${command}.alias`),
      userChainSequence: integer(
        item.userChainSequence,
        `${command}.userChainSequence`,
      ),
      removedLocalCredential: bool(
        item.removedLocalCredential,
        `${command}.removedLocalCredential`,
      ),
    };
  }
  return {
    alias: string(item.alias, `${command}.alias`),
    managementEnrolled: bool(
      item.managementEnrolled,
      `${command}.managementEnrolled`,
    ),
    managementGeneration: nullable(
      item.managementGeneration,
      `${command}.managementGeneration`,
      integer,
    ),
  };
}

function decodeDropHover(value: unknown): DropHoverEvent {
  const item = record(value, 'foks://drop-hover payload');
  return { hovering: bool(item.hovering, 'foks://drop-hover.hovering') };
}

function decodeDropPaths(value: unknown): string[] {
  return array(value, 'foks://drop-paths payload', string);
}

function decodeWindowState(value: unknown): WindowStateEvent {
  const item = record(value, 'foks://window-state payload');
  return {
    maximized: bool(item.maximized, 'foks://window-state.maximized'),
    fullscreen: bool(item.fullscreen, 'foks://window-state.fullscreen'),
  };
}

export function normalizeCommandError(value: unknown): CommandError {
  try {
    return decodeCommandError(value, 'command error');
  } catch {
    return {
      code: 'invalid-command-error',
      message:
        value instanceof Error
          ? value.message
          : 'The local agent returned an invalid error.',
      retryable: false,
      ambiguous: true,
      fatal: true,
    };
  }
}

function notificationsOf(
  catalog: CatalogDto,
  servers: readonly Server[],
): Notification[] {
  // Generate a notification for any server state that hides stores,
  // including unverified servers.
  const stopped: Partial<
    Record<Server['state'], { detail: string; action: string }>
  > = {
    'lease-lapsed': {
      detail: `The server session has expired. Vaults are unavailable until reconnected.`,
      action: 'Reconnect',
    },
    'lease-unavailable': {
      detail: `Unable to verify connection with this server. Vaults are unavailable until verified.`,
      action: 'Inspect',
    },
    blocked: {
      detail: `Server verification failed because its history does not match the pinned record. View server details to review the error.`,
      action: 'Inspect',
    },
    'never-probed': {
      detail: `This server has not been verified yet. Check the server to establish a connection and view its contents.`,
      action: 'Verify',
    },
  };
  const notes: Notification[] = servers.flatMap((server) => {
    const reason = stopped[server.state];
    return reason
      ? [
          {
            id: `${server.state}-${server.id}`,
            severity: 'crit' as const,
            title: `${server.name} is locked`,
            ...reason,
          },
        ]
      : [];
  });
  for (const [index, failure] of catalog.failures.entries()) {
    notes.push({
      id: `catalog-${failure.scope}-${index}`,
      severity: failure.error.fatal ? 'crit' : 'warn',
      title:
        failure.scope === 'store'
          ? `Could not load vault on ${failure.profile}`
          : `Could not load ${failure.source} on ${failure.profile}`,
      detail: failure.error.message,
      action: failure.error.retryable ? 'Retry' : 'Inspect',
    });
  }
  return notes;
}

async function checked<T>(
  command: string,
  args: Record<string, unknown> | undefined,
  decode: (value: unknown) => T,
): Promise<T> {
  return decode(await invoke<unknown>(command, args));
}

function decodeAgentStatus(value: unknown): AgentStatus {
  const item = record(value, 'agent_status response');
  const state = string(item.state, 'agent_status.state');
  if (state === 'ready') return { phase: 'Ready' };
  if (state === 'bootstrap') {
    return { phase: `Bootstrap · ${string(item.step, 'agent_status.step')}` };
  }
  throw new Error('agent_status.state is not ready or bootstrap');
}

export const tauriBridge: Bridge = {
  native: true,
  appLockState: () => checked('app_lock_state', undefined, decodeAppLockState),
  windowState: () => checked('get_window_state', undefined, decodeWindowState),
  lockApp: () => checked('lock_app', undefined, decodeAppLockState),
  unlockApp: () => checked('unlock_app', undefined, decodeAppLockState),
  agentStatus: () => checked('agent_status', undefined, decodeAgentStatus),
  appInfo: () => checked('app_info', undefined, decodeAppInfo),
  listCatalog: () => checked('list_catalog', undefined, decodeCatalog),
  listStores: () => checked('list_stores', undefined, decodeCatalog),
  listServers: () => checked('list_servers', undefined, decodeServers),
  listAccounts: () => checked('list_accounts', undefined, decodeAccounts),
  listGroupDetails: (storeId) =>
    checked('list_group_details', { storeId }, decodeGroupDetails),
  listParties: (storeId) => checked('list_parties', { storeId }, decodeParties),
  listFederation: (storeId) =>
    checked('list_federation', { storeId }, decodeFederation),
  readItem: ({ storeId, path, version }) =>
    checked('read_item', { storeId, path, version }, decodeReadItem),
  copyItemValue: ({ storeId, path, version }) =>
    checked('copy_item_value', { storeId, path, version }, decodeCopy),
  copyItemPath: ({ storeId, path, version }) =>
    checked('copy_item_path', { storeId, path, version }, decodeCopy),
  downloadFile: ({ storeId, path, version }) =>
    checked('download_file', { storeId, path, version }, decodeDownload),
  createTextItem: ({ storeId, path, value, readRole, writeRole }) =>
    checked(
      'create_text_item',
      {
        storeId,
        path,
        value,
        ...(readRole === undefined ? {} : { readRole }),
        ...(writeRole === undefined ? {} : { writeRole }),
      },
      decodeMutation,
    ),
  createLink: ({ storeId, path, target, readRole, writeRole }) =>
    checked(
      'create_link',
      {
        storeId,
        path,
        target,
        ...(readRole === undefined ? {} : { readRole }),
        ...(writeRole === undefined ? {} : { writeRole }),
      },
      decodeMutation,
    ),
  createFolder: ({ storeId, path, readRole, writeRole }) =>
    checked(
      'create_folder',
      {
        storeId,
        path,
        ...(readRole === undefined ? {} : { readRole }),
        ...(writeRole === undefined ? {} : { writeRole }),
      },
      decodeMutation,
    ),
  editTextItem: ({ storeId, path, version, value }) =>
    checked(
      'edit_text_item',
      { storeId, path, version, value },
      decodeMutation,
    ),
  removeItem: ({ storeId, path, version }) =>
    checked('remove_item', { storeId, path, version }, decodeMutation),
  importDroppedFile: ({ storeId, path, sourcePath, readRole, writeRole }) =>
    checked(
      'import_dropped_file',
      {
        storeId,
        path,
        sourcePath,
        ...(readRole === undefined ? {} : { readRole }),
        ...(writeRole === undefined ? {} : { writeRole }),
      },
      decodeMutation,
    ),
  pickAndImportFile: ({ storeId, path, readRole, writeRole }) =>
    checked(
      'pick_and_import_file',
      {
        storeId,
        path,
        ...(readRole === undefined ? {} : { readRole }),
        ...(writeRole === undefined ? {} : { writeRole }),
      },
      decodeMutation,
    ),
  replaceDroppedFile: ({ storeId, path, version, sourcePath }) =>
    checked(
      'replace_dropped_file',
      { storeId, path, version, sourcePath },
      decodeMutation,
    ),
  pickAndReplaceFile: ({ storeId, path, version }) =>
    checked(
      'pick_and_replace_file',
      { storeId, path, version },
      decodeMutation,
    ),
  resumeGroupCreation: (storeId) =>
    checked('resume_group_creation', { storeId }, decodeMutation),
  takeAgentConnectionLoss: () =>
    checked('take_agent_connection_loss', undefined, decodeConnectionLoss),
  retryAgentConnection: () =>
    checked('retry_agent_connection', undefined, decodeAgentStatus),
  createGroup: ({ accountStoreId, teamAlias, name, kind }) =>
    checked(
      'create_group',
      { accountStoreId, teamAlias, name, kind },
      decodeMutation,
    ),
  addGroupMember: ({ storeId, username, destination }) =>
    checked(
      'add_group_member',
      { storeId, username, destination },
      decodeMutation,
    ),
  resumeGroupMemberAddition: ({ storeId, username }) =>
    checked(
      'resume_group_member_addition',
      { storeId, username },
      decodeMutation,
    ),
  demoteGroupMember: ({ storeId, username, destination }) =>
    checked(
      'demote_group_member',
      { storeId, username, destination },
      decodeMutation,
    ),
  removeGroupMember: ({ storeId, username }) =>
    checked('remove_group_member', { storeId, username }, decodeMutation),
  resumeGroupMemberEdit: (storeId) =>
    checked('resume_group_member_edit', { storeId }, decodeMutation),
  admitGroup: ({ storeId, remoteStoreId, visibility }) =>
    checked(
      'admit_group',
      { storeId, remoteStoreId, visibility },
      decodeMutation,
    ),
  expelFederatedGroup: ({ storeId, remoteHostIdHex, remoteTeamIdHex }) =>
    checked(
      'expel_federated_group',
      { storeId, remoteHostIdHex, remoteTeamIdHex },
      decodeMutation,
    ),
  rerunGroupAdmission: (storeId, operationId) =>
    checked('rerun_group_admission', { storeId, operationId }, decodeMutation),
  copyText: (text) => checked('copy_text', { text }, decodeCopy),
  initializeClientState: () =>
    checked('initialize_client_state', undefined, decodeAgentStatus),
  discoverGoProfiles: () =>
    checked('discover_go_profiles', undefined, decodeGoProfileDiscovery),
  checkAndAddProfile: (profileName, probe) =>
    checked(
      'check_and_add_profile',
      { profileName, probe },
      decodeCheckedProfile,
    ),
  checkAndAddGoProfile: (candidateId, hostId, profileName, probe) =>
    checked(
      'check_and_add_go_profile',
      { candidateId, hostId, profileName, probe },
      decodeCheckedProfile,
    ),
  listPendingOperations: (profile) =>
    checked('list_pending_operations', { profile }, decodePendingOperations),
  createFirstRunAccount: ({
    profile,
    alias,
    username,
    deviceName,
    email,
    invite,
    passphrase,
    passphraseConfirmation,
  }) =>
    checked(
      'create_first_run_account',
      {
        profile,
        alias,
        username,
        deviceName,
        email,
        invite,
        passphrase,
        passphraseConfirmation,
      },
      decodeMutation,
    ),
  resumeFirstRunAccount: (profile, alias) =>
    checked('resume_first_run_account', { profile, alias }, decodeMutation),
  setFirstRunPassphrase: ({ profile, alias, passphrase, confirmation }) =>
    checked(
      'set_first_run_passphrase',
      { profile, alias, passphrase, confirmation },
      decodeMutation,
    ),
  prepareOwnerBackup: (profile, accountAlias, backupAlias) =>
    checked(
      'prepare_owner_backup',
      { profile, accountAlias, backupAlias },
      decodeBackupPhrase,
    ),
  commitOwnerBackup: (profile, accountAlias, backupAlias, phrase) =>
    checked(
      'commit_owner_backup',
      { profile, accountAlias, backupAlias, phrase },
      decodeMutation,
    ),
  recoverOwnerAccount: (profile, targetAlias, phrase, deviceName) =>
    checked(
      'recover_owner_account',
      { profile, targetAlias, phrase, deviceName },
      decodeMutation,
    ),
  resumeOwnerRecovery: (profile, targetAlias, phrase, deviceName) =>
    checked(
      'resume_owner_recovery',
      { profile, targetAlias, phrase, deviceName },
      decodeMutation,
    ),
  discoverGroups: (profile, accountAlias) =>
    checked('discover_groups', { profile, accountAlias }, decodeGroupDiscovery),
  describeServerStatus: (profile) =>
    checked('describe_server_status', { profile }, decodeServerStatus),
  checkServer: (profile) =>
    checked('check_server', { profile }, decodeCheckedServer),
  addServer: (profileName, probe) =>
    checked('add_server', { profileName, probe }, decodeAddedServer),
  forgetServer: (profile, confirmation) =>
    checked('forget_server', { profile, confirmation }, decodeForgottenServer),
  listAccountDevices: (accountStoreId) =>
    checked('list_account_devices', { accountStoreId }, decodeAccountDevices),
  removeAccountDevice: (accountStoreId, deviceId) =>
    checked(
      'remove_account_device',
      { accountStoreId, deviceId },
      decodeDeviceRemoval,
    ),
  listBackupEnrollments: (accountStoreId) =>
    checked(
      'list_backup_enrollments',
      { accountStoreId },
      decodeBackupEnrollments,
    ),
  revokeOwnerBackup: (accountStoreId, backup, confirmation) =>
    checked(
      'revoke_owner_backup',
      {
        accountStoreId,
        backupAlias: backup.backupAlias,
        backupId: backup.backupId,
        confirmation,
      },
      (value) => {
        const revoked = decodeBackupRevocation(value);
        if (
          revoked.backupAlias !== backup.backupAlias ||
          revoked.accountAlias !== backup.accountAlias ||
          revoked.backupId !== backup.backupId
        )
          throw new Error(
            'revoke_owner_backup returned a different backup enrollment.',
          );
        return revoked;
      },
    ),
  startDevicePairing: (accountStoreId) =>
    checked('start_device_pairing', { accountStoreId }, decodePairingOffer),
  resumeDevicePairingOffer: (accountStoreId) =>
    checked(
      'resume_device_pairing_offer',
      { accountStoreId },
      decodePairingOffer,
    ),
  finishDevicePairing: (accountStoreId) =>
    checked('finish_device_pairing', { accountStoreId }, decodeDeviceProvision),
  acceptDevicePairing: (profile, targetAlias, deviceName, phrase) =>
    checked(
      'accept_device_pairing',
      { profile, targetAlias, deviceName, phrase },
      decodeDeviceProvision,
    ),
  acceptGoProfilePairing: (
    candidateId,
    profile,
    targetAlias,
    deviceName,
    phrase,
  ) =>
    checked(
      'accept_go_profile_pairing',
      { request: { candidateId, profile, targetAlias, deviceName, phrase } },
      decodeDeviceProvision,
    ),
  resumeDevicePairingAcceptance: (profile, targetAlias) =>
    checked(
      'resume_device_pairing_acceptance',
      { profile, targetAlias },
      decodeDeviceProvision,
    ),
  resumeGoProfilePairing: (candidateId, profile, targetAlias) =>
    checked(
      'resume_go_profile_pairing',
      { candidateId, profile, targetAlias },
      decodeDeviceProvision,
    ),
  copyGoProfileDevice: (candidateId, profile, targetAlias) =>
    checked(
      'copy_go_profile_device',
      { candidateId, profile, targetAlias },
      decodeDeviceProvision,
    ),
  setAccountPassphrase: (accountStoreId, passphrase, confirmation) =>
    checked(
      'set_account_passphrase',
      { accountStoreId, passphrase, confirmation },
      decodePassphraseReport,
    ),
  changeAccountPassphrase: (accountStoreId, passphrase, confirmation) =>
    checked(
      'change_account_passphrase',
      { accountStoreId, passphrase, confirmation },
      decodePassphraseReport,
    ),
  verifyAccountPassphrase: (accountStoreId, passphrase) =>
    checked(
      'verify_account_passphrase',
      { accountStoreId, passphrase },
      decodePassphraseReport,
    ),
  describeReset: (profile) =>
    checked('describe_reset', { profile }, decodeResetPreview),
  resetServer: (profile, confirmation, token) =>
    checked('reset_server', { profile, confirmation, token }, decodeMutation),
  listYubiCards: (profile) =>
    checked('list_yubi_cards', { profile }, decodeYubiCards),
  listYubiAccounts: (profile) =>
    checked('list_yubi_accounts', { profile }, decodeYubiAccounts),
  runYubi: ({ command, args }) =>
    checked(command, { ...args }, (value) => decodeYubiResult(command, value)),
  onDropHover: async (listener) =>
    listen<unknown>('foks://drop-hover', (event) => {
      listener(decodeDropHover(event.payload));
    }),
  onDropPaths: async (listener) =>
    listen<unknown>('foks://drop-paths', (event) => {
      listener(decodeDropPaths(event.payload));
    }),
  onWindowState: async (listener) =>
    listen<unknown>('foks://window-state', (event) => {
      listener(decodeWindowState(event.payload));
    }),
};

export function isNativeHost(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

export function mockRequested(): boolean {
  return import.meta.env?.VITE_FOKS_MOCK === '1';
}

export async function selectBridge(): Promise<Bridge> {
  // A development Tauri window still exercises the real command boundary.
  if (isNativeHost()) return tauriBridge;
  // Both conditions are compile-time constants in Vite. Ordinary production
  // builds eliminate this branch and its fixture chunk completely.
  if (
    import.meta.env?.VITE_FOKS_MOCK === '1' ||
    import.meta.env?.DEV === true
  ) {
    const { mockBridge } = await import('./mock-bridge');
    return mockBridge();
  }
  throw new Error(
    'FOKS desktop host environment is required.',
  );
}

function recoverableGroupDetailFailure(
  error: unknown,
  store: StoreRef,
  source: GroupDetailFailure['source'],
): GroupDetailFailure {
  const typed = normalizeCommandError(error);
  if (
    ![
      'rate-limited',
      'quota-exceeded',
      'capability-denied',
      'operation-failed',
      'io',
      'deadline-exceeded',
      'profile-busy',
      'busy',
      'cancelled',
    ].includes(typed.code)
  ) {
    if (typed.code === 'invalid-command-error') {
      throw new Error(
        'The agent returned an unrecognized group-detail error.',
      );
    }
    throw typed;
  }
  return {
    store,
    source,
    code: typed.code,
    message: typed.message,
    retryable: typed.retryable,
  };
}

/** Load the current world state. */
export async function loadWorld(
  bridge: Bridge,
  base = bridge.fixtureWorld,
  nowSeconds: number = Math.floor(Date.now() / 1000),
): Promise<World> {
  const [agent, response] = await Promise.all([
    bridge.agentStatus(),
    bridge.listCatalog(),
  ]);
  const liveStores = response.stores as Store[];
  const storesById = new Map<StoreRef, Store>();
  for (const store of response.knownStores as Store[])
    storesById.set(store.id, store);
  for (const store of liveStores) storesById.set(store.id, store);
  const stores = [...storesById.values()];
  const liveStoreIds = new Set(liveStores.map((store) => store.id));
  const unavailableStores = stores
    .filter((store) => !liveStoreIds.has(store.id))
    .map((store) => store.id);
  const inventoryProfiles = new Set(
    response.inventory.map((state) => state.profile),
  );
  if (
    inventoryProfiles.size !== response.inventory.length ||
    response.inventory.some(
      (state) => !response.profiles.includes(state.profile),
    )
  ) {
    throw new Error(
      'list_catalog returned duplicate or unknown inventory profiles.',
    );
  }
  const accountInventoryComplete = response.profiles.every((profile) =>
    response.inventory.some(
      (state) => state.profile === profile && state.accountsComplete,
    ),
  );
  // When a server profile is blocked, skip roster queries for that server.
  const blockedProfiles = new Set(response.blockedProfiles);
  const listedServers = await bridge.listServers();
  const statusResults = bridge.native
    ? await Promise.all(
        listedServers
          .filter(
            (server) =>
              server.state !== 'blocked' && !blockedProfiles.has(server.id),
          )
          .map(async (server) => {
            try {
              const status = await sharedServerStatus(bridge, server.id);
              if (status.profile !== server.id) {
                throw new Error(
                  'describe_server_status returned a different profile.',
                );
              }
              return { profile: server.id, status };
            } catch (error) {
              return {
                profile: server.id,
                error:
                  error instanceof Error
                    ? error.message
                    : 'Server status was unavailable.',
              };
            }
          }),
      )
    : [];
  const statuses = new Map(
    statusResults.flatMap((result) =>
      result.status ? [[result.profile, result.status] as const] : [],
    ),
  );
  const statusFailures = new Map(
    statusResults.flatMap((result) =>
      result.error ? [[result.profile, result.error] as const] : [],
    ),
  );
  const servers = listedServers.map((server) => {
    if (!bridge.native) return server;
    if (server.state === 'blocked' || blockedProfiles.has(server.id))
      return { ...server, state: 'blocked' as const };
    const status = statuses.get(server.id);
    if (!status || statusFailures.has(server.id))
      return { ...server, state: 'lease-unavailable' as const };
    if (!status.host) return { ...server, state: 'never-probed' as const };
    const lease = serverLeaseState(status, nowSeconds);
    if (lease === 'lapsed')
      return { ...server, state: 'lease-lapsed' as const };
    if (lease === 'fresh') return { ...server, state: 'ok' as const };
    return { ...server, state: 'lease-unavailable' as const };
  });
  const rawAccounts = await bridge.listAccounts();
  const unavailableServers = new Set(
    servers
      .filter((server) => server.state !== 'ok')
      .map((server) => server.id),
  );
  // Inactive teams cannot query members or federation until setup is complete;
  // skip those reads.
  const teams = liveStores.filter(
    (store) =>
      store.kind === 'team' &&
      store.active &&
      !blockedProfiles.has(store.server) &&
      !unavailableServers.has(store.server),
  );
  const rosters = await Promise.all(
    teams.map(async (store) => {
      const { parties, federation, failures } = await enqueueProfileWork(
        bridge,
        store.server,
        async () => {
          let parties: Party[] = [];
          let federation: FederationEntry[] = [];
          const failures: GroupDetailFailure[] = [];
          try {
            const details = await bridge.listGroupDetails(store.id);
            if (details.parties.status === 'success')
              parties = details.parties.value;
            else
              failures.push(
                recoverableGroupDetailFailure(
                  details.parties.error,
                  store.id,
                  'roster',
                ),
              );
            if (details.federation.status === 'success')
              federation = details.federation.value;
            else
              failures.push(
                recoverableGroupDetailFailure(
                  details.federation.error,
                  store.id,
                  'federation',
                ),
              );
          } catch (error) {
            failures.push(
              recoverableGroupDetailFailure(error, store.id, 'roster'),
            );
            failures.push(
              recoverableGroupDetailFailure(error, store.id, 'federation'),
            );
          }
          return { parties, federation, failures };
        },
      );
      if (parties.some((party) => party.store !== store.id)) {
        throw new Error(
          'list_parties returned a roster for a different store.',
        );
      }
      if (federation.some((entry) => entry.store !== store.id)) {
        throw new Error(
          'list_federation returned group memberships for a different store.',
        );
      }
      return { parties, federation, failures };
    }),
  );
  const groupDetailFailures = rosters.flatMap((roster) => roster.failures);
  const storeIds = new Set(liveStores.map((store) => store.id));
  if (response.items.some((item) => !storeIds.has(item.store))) {
    throw new Error(
      'list_catalog returned an item for a store it did not include.',
    );
  }
  const availableAccountStoreIds = new Set(
    liveStores
      .filter(
        (store) =>
          store.kind === 'account' &&
          !blockedProfiles.has(store.server) &&
          !unavailableServers.has(store.server),
      )
      .map((store) => store.id),
  );
  const accounts = rawAccounts.filter(
    (account) =>
      account.store !== undefined &&
      availableAccountStoreIds.has(account.store),
  );
  if (
    rawAccounts.some(
      (account) => !account.store || !storeIds.has(account.store),
    ) ||
    rawAccounts.some((account) => {
      const store = liveStores.find(
        (candidate) => candidate.id === account.store,
      );
      return store?.kind !== 'account';
    }) ||
    new Set(rawAccounts.map((account) => account.store)).size !==
      rawAccounts.length
  ) {
    throw new Error(
      'list_accounts returned an unknown, non-account, or duplicate store.',
    );
  }
  if (accounts.length !== availableAccountStoreIds.size) {
    throw new Error('list_accounts omitted an available account store.');
  }
  const serverIds = new Set(servers.map((server) => server.id));
  if (stores.some((store) => !serverIds.has(store.server))) {
    throw new Error('list_servers omitted a server used by the catalog.');
  }
  const federation = rosters.flatMap((roster) => roster.federation);
  const parties = rosters
    .flatMap((roster) => roster.parties)
    .map((party) => {
      const team = liveStores.find(
        (store) => store.id === party.store && store.kind === 'team',
      );
      const ownerStore = team
        ? liveStores.find(
            (store) =>
              store.kind === 'account' &&
              store.account === team.account &&
              store.server === team.server,
          )
        : undefined;
      const ownerAccount = ownerStore
        ? accounts.find((account) => account.store === ownerStore.id)
        : undefined;
      const admissions =
        party.party_kind === 'named-team'
          ? federation.filter(
              (entry) =>
                entry.store === party.store &&
                entry.remote_team_id_hex === party.party_id_hex &&
                (!party.scoped_host_id_hex ||
                  entry.remote_host_id_hex === party.scoped_host_id_hex),
            )
          : [];
      return {
        ...party,
        label:
          party.party_kind === 'user' &&
          ownerAccount &&
          party.locally_manageable &&
          !party.scoped_host_id_hex &&
          party.username === ownerAccount.username
            ? 'you'
            : bridge.native
              ? undefined
              : party.label,
        // Resolve the remote server's display name rather than its internal profile identifier.
        team_name:
          admissions.length === 1
            ? `${admissions[0].remote_team_alias} @ ${servers.find((server) => server.id === admissions[0].remote_profile)?.name ?? admissions[0].remote_profile}`
            : party.team_name,
      };
    });
  const baseItems = new Map(
    (base?.items ?? []).map((item) => [itemKey(item), item]),
  );
  const items: Item[] = response.items
    .filter((item) => {
      const store = liveStores.find((candidate) => candidate.id === item.store);
      return Boolean(store && !unavailableServers.has(store.server));
    })
    .map((item) => ({
      ...(bridge.native ? {} : baseItems.get(itemKey(item))),
      ...item,
    }));
  if (!bridge.native) {
    if (!base)
      throw new Error('The mock bridge did not supply its fixture world.');
    return {
      ...base,
      agent,
      servers,
      accounts,
      stores,
      unavailableStores,
      accountInventoryComplete,
      items,
      parties,
      federation,
      groupDetailFailures,
    };
  }
  return {
    agent,
    servers,
    accounts,
    stores,
    unavailableStores,
    accountInventoryComplete,
    items,
    parties,
    federation,
    groupDetailFailures,
    devices: [],
    yubiAccounts: [],
    cardsConnected: [],
    notifications: [
      ...notificationsOf(response, servers).filter(
        (note) =>
          ![...statusFailures.keys()].some(
            (profile) => note.id === `lease-unavailable-${profile}`,
          ),
      ),
      ...[...statusFailures].map(([profile, message]) => ({
        id: `status-unavailable-${profile}`,
        severity: 'crit' as const,
        title: `Status for ${profile} is unavailable`,
        detail: `${message} Server contents are unavailable until the connection status is verified.`,
        action: 'Inspect',
      })),
      ...groupDetailFailures.map((failure) => ({
        id: `group-${failure.source}-unavailable-${failure.store}`,
        severity: 'warn' as const,
        title: failure.source === 'roster'
          ? 'Group member list is unavailable'
          : 'Group shared access is unavailable',
        detail: failure.message,
        action: failure.retryable ? 'Refresh' : 'Inspect',
      })),
    ],
    leaseState: servers.some((server) => server.state === 'lease-lapsed')
      ? 'lapsed'
      : servers.some((server) => server.state === 'lease-unavailable')
        ? 'unavailable'
        : 'fresh',
    plaintext: {},
  };
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
