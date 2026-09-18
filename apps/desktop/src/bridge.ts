import {
  scheduleProfileWork,
  type BackgroundHistoryWork,
} from './scheduling/profile-work';
import {
  canonicalProbeEndpoint,
  retainProfileConnection,
} from './profile-connectivity';
import type { ProfileReconciliation } from './profile-connectivity';
import {
  catalogItemsComplete,
  catalogStoreComplete,
  mergeProfileSnapshot,
  projectCatalogFreshness,
  sameStoreIdentity,
} from './catalog-state';
import {
  decodeInvitationReply,
  type InvitationAction,
  type InvitationReply,
} from './invitation-contract';
import { decodeBotReply, type BotAction, type BotReply } from './bot-contract';
import {
  decodeRenameProgress,
  type RenameAction,
  type RenameProgress,
} from './rename-contract';
import {
  decodeSsoProgress,
  type SsoAction,
  type SsoProgress,
} from './sso-contract';
import {
  decodeLocalSession,
  type LocalAction,
  type LocalSession,
} from './chat/local-contract';
/**
 * The only seam between the FOKS webview and the local agent.
 *
 * Tauri's generic `invoke<T>` does not validate types at runtime. Native
 * responses pass through runtime decoders before entering the UI state.
 */

import { decodeChatReply } from './chat-contract';
import type { ChatAction, ChatReply } from './chat-contract';
import { Channel, invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  itemKey,
  parseRole,
  serverDisplayName,
  PROTOCOL_CAPABILITIES,
} from './model';
import type {
  AgentStatus,
  CompatibilityLease,
  ProtocolCapability,
  Account,
  AvailabilityReason,
  GroupDetailFailure,
  Item,
  NodeKind,
  Party,
  FederationEntry,
  Notification,
  RoleWire,
  Server,
  ServerRestriction,
  Store,
  StoreRef,
  TeamCreationPhase,
  AgentSnapshot,
} from './model';
import { profileInventoryComplete, serverFactAvailability } from './model';

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
    stateDir?: string;
    reason?: string;
    foundSchema?: number;
    supportedSchema?: number;
  };
}

export type MaintenanceKind =
  'export' | 'import' | 'verify' | 'relocate' | 'restart';
export type MaintenancePhase =
  'selecting' | 'confirming' | 'quiescing' | 'running' | 'restoring';
export type MaintenanceOperationOutcome =
  | { status: 'cancelled' }
  | { status: 'completed' }
  | { status: 'failed'; error: CommandError };
export type MaintenanceDisposition =
  | { status: 'continue-current-root' }
  | { status: 'restart-selected-root'; root: string }
  | { status: 'recovery-required'; root: string }
  | { status: 'restoration-failed'; root: string; error: CommandError };
export type MaintenanceSnapshot =
  | { state: 'idle'; generation: number; revision: number }
  | {
      state: 'active';
      generation: number;
      revision: number;
      kind: MaintenanceKind;
      phase: MaintenancePhase;
    }
  | {
      state: 'complete';
      generation: number;
      revision: number;
      kind: MaintenanceKind;
      operation: MaintenanceOperationOutcome;
      disposition: MaintenanceDisposition;
    };

type AgentReadinessListener = (error: CommandError) => void;
const readinessListeners = new Set<AgentReadinessListener>();
const reportedReadinessErrors = new WeakSet<object>();

export function onAgentReadinessRequired(
  listener: AgentReadinessListener,
): () => void {
  readinessListeners.add(listener);
  return () => readinessListeners.delete(listener);
}

export function isAgentReadinessError(error: CommandError): boolean {
  return (
    error.code === 'bootstrap-required' ||
    error.code === 'agent-lost' ||
    error.code === 'version-mismatch'
  );
}

function reportReadinessError(error: CommandError): void {
  if (!isAgentReadinessError(error)) return;
  if (reportedReadinessErrors.has(error)) return;
  reportedReadinessErrors.add(error);
  for (const listener of readinessListeners) listener(error);
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
  creation_phase?: TeamCreationPhase;
  team_kind?: 'named' | 'adhoc';
  team_id_hex?: string;
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

export interface RemoveFederatedGroupRequest {
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

export interface TrackedAccountAttempt {
  id: string;
  profile: string;
  hostId: string;
  alias: string;
  deviceName: string;
  candidateId?: string;
  kind: 'signup' | 'recovery' | 'copy' | 'pairing';
}

export interface TrackedAccountRequest {
  attempt: TrackedAccountAttempt;
  resume: boolean;
  deviceName: string;
  username?: string;
  email?: string;
  invite?: string;
  phrase?: string;
  candidateId?: string;
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
  compatibility: Exclude<CompatibilityLease, { status: 'requirement-unknown' }>;
  chatSupported: boolean | null;
}

export interface ServerLabelResponse {
  profile: string;
  label: string | null;
  changed: boolean;
}

/** The process answering on the agent socket, as Settings › This Mac shows it. */
export interface AgentProcessInfo {
  pid: number | null;
  executable: string | null;
  /** Seconds since the Unix epoch. */
  startedAt: number | null;
  /** Whether this app launched or adopted it, so it can stop it. */
  owned: boolean;
}
export function decodeAgentProcessInfo(value: unknown): AgentProcessInfo {
  const item = record(value, 'agent_process_info response');
  const pid = item.pid;
  const executable = item.executable;
  const startedAt = item.startedAt;
  if (
    !(pid === null || pid === undefined || typeof pid === 'number') ||
    !(
      executable === null ||
      executable === undefined ||
      typeof executable === 'string'
    ) ||
    !(
      startedAt === null ||
      startedAt === undefined ||
      typeof startedAt === 'number'
    ) ||
    typeof item.owned !== 'boolean'
  )
    throw new Error('Invalid agent process info');
  return {
    pid: pid ?? null,
    executable: executable ?? null,
    startedAt: startedAt ?? null,
    owned: item.owned,
  };
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

/** The server's advertised client-version range and this client's verdict. */
export interface ServerVersionInfo {
  minimum: string | null;
  newest: string | null;
  message: string;
  compatible: boolean;
}

export interface CheckedServer extends StoredHost {
  profile: string;
  acceptance: 'inserted' | 'advanced' | 'unchanged';
  serverVersion: ServerVersionInfo | null;
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
  /** The same authenticated key id carried by AccountDevice.id. */
  deviceId?: string;
  cardSerial?: number;
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

export interface CommandAck {
  ok: true;
}
export function decodeCommandAck(value: unknown): CommandAck {
  if (
    !value ||
    typeof value !== 'object' ||
    Object.keys(value).length !== 1 ||
    !('ok' in value) ||
    value.ok !== true
  )
    throw new Error('Invalid command acknowledgement');
  return { ok: true };
}
export interface Bridge {
  relocateClientState(): Promise<MaintenanceSnapshot>;
  maintainClientState(
    action: 'export' | 'import' | 'verify',
  ): Promise<MaintenanceSnapshot>;
  clientStateMaintenanceStatus(): Promise<MaintenanceSnapshot>;
  /** The process on the agent socket. */
  agentProcessInfo(): Promise<AgentProcessInfo>;
  /**
   * Stops the local agent and starts it again. `takeover` claims an agent
   * this app did not start, once the reader has confirmed that.
   */
  restartAgent(takeover: boolean): Promise<MaintenanceSnapshot>;
  configureWebAdmin(
    profile: string,
    accountAlias: string,
    destination: string,
  ): Promise<CommandAck>;
  openWebAdmin(
    profile: string,
    accountAlias: string,
    pin: string | null,
  ): Promise<CommandAck>;
  botAccount(
    profile: string,
    accountAlias: string,
    action: BotAction,
  ): Promise<BotReply>;
  invitation(
    profile: string,
    accountAlias: string,
    action: InvitationAction,
    pin: string | null,
  ): Promise<InvitationReply>;
  setLocalAccountAlias(
    store: StoreRef,
    label: string,
  ): Promise<{ store: StoreRef; alias: string }>;
  renameAccount(
    profile: string,
    accountAlias: string,
    action: RenameAction | null,
  ): Promise<RenameProgress[]>;
  sso(
    profile: string,
    accountAlias: string,
    action: SsoAction,
  ): Promise<SsoProgress>;
  openSsoBrowser(
    profile: string,
    accountAlias: string,
    operationId: string,
  ): Promise<CopyResponse>;
  chat(
    storeId: StoreRef,
    action: ChatAction,
    viewId?: string,
  ): Promise<ChatReply>;
  cancelChat(viewId: string): Promise<void>;
  readonly native: boolean;
  /** Present only on the development bridge, never on the native bridge. */
  readonly fixtureSnapshot?: AgentSnapshot;
  readonly firstRunFixture?: FirstRunFixture;
  appLockState(): Promise<AppLockState>;
  windowState(): Promise<WindowStateEvent>;
  setTrafficLightsVisible(visible: boolean): Promise<void>;
  lockApp(): Promise<AppLockState>;
  unlockApp(): Promise<AppLockState>;
  restartApp(): Promise<void>;
  quitApp(): Promise<void>;
  agentStatus(): Promise<AgentStatus>;
  probeAgentStatus?(): Promise<AgentStatus>;
  appInfo(): Promise<AppInfo>;
  /** Returns the full catalog including stores. Mutually exclusive with `listStores`. */
  listCatalog(onPartial?: (catalog: CatalogDto) => void): Promise<CatalogDto>;
  listProfileCatalog(profile: string): Promise<CatalogDto>;
  /** Returns store metadata only. Mutually exclusive with `listCatalog`. */
  listStores(): Promise<CatalogDto>;
  /**
   * Server rows drawn from the current native catalog snapshot. Passing the
   * generation of a `listCatalog` response makes the read fail with a
   * retryable catalog-required error when that snapshot has been replaced.
   */
  listServers(generation?: number): Promise<Server[]>;
  /** Account identities for the current snapshot; `generation` as above. */
  listAccounts(generation?: number): Promise<Account[]>;
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
  /**
   * Forgets the local record of a team whose creation never finished. Only a
   * team the catalog reports as inactive can be forgotten this way, and only
   * local state is touched: a creation the server already accepted leaves the
   * team on that server with no key here that can reach it.
   */
  abandonGroupCreation(storeId: StoreRef): Promise<MutationResponse>;
  takeAgentConnectionLoss(): Promise<string | null>;
  retryAgentConnection(): Promise<AgentStatus>;
  autoRecoverAgent?(): Promise<AgentStatus>;
  createGroup(request: CreateGroupRequest): Promise<MutationResponse>;
  addGroupMember(request: GroupRoleRequest): Promise<MutationResponse>;
  resumeGroupMemberAddition(
    request: GroupMemberRequest,
  ): Promise<MutationResponse>;
  demoteGroupMember(request: GroupRoleRequest): Promise<MutationResponse>;
  removeGroupMember(request: GroupMemberRequest): Promise<MutationResponse>;
  resumeGroupMemberEdit(storeId: StoreRef): Promise<MutationResponse>;
  admitGroup(request: AdmitGroupRequest): Promise<MutationResponse>;
  removeFederatedGroup(
    request: RemoveFederatedGroupRequest,
  ): Promise<MutationResponse>;
  rerunGroupAdmission(
    storeId: StoreRef,
    operationId: string,
  ): Promise<MutationResponse>;
  chatLocal(action: LocalAction): Promise<LocalSession>;
  openChatLink(url: string): Promise<CopyResponse>;
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
  runFirstRunAccountOperation?(
    request: TrackedAccountRequest,
  ): Promise<MutationResponse>;
  firstRunOperationStatus?(
    attempt: TrackedAccountAttempt,
  ): Promise<'complete' | 'rejected' | 'unknown' | 'running'>;
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
  describeServerStatus(
    profile: string,
    fresh?: boolean,
  ): Promise<ServerStatusSnapshot>;
  checkServer(profile: string): Promise<CheckedServer>;
  reconcileServer?(profile: string): Promise<ProfileReconciliation>;
  addServer(
    profileName: string,
    probe: string,
  ): Promise<{ profile: string; configuredProbe: string }>;
  setServerLabel(
    profile: string,
    label: string | null,
  ): Promise<ServerLabelResponse>;
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
  onChatNotification(
    listener: (kind: 'activate' | 'error') => void,
  ): Promise<Unlisten>;
  onOpenSettings(listener: () => void): Promise<Unlisten>;
  onMaintenanceStatus(
    listener: (snapshot: MaintenanceSnapshot) => void,
  ): Promise<Unlisten>;
}

const pendingServerStatuses = new WeakMap<
  Bridge,
  Map<string, Promise<ServerStatusSnapshot>>
>();

/**
 * Queues profile-scoped requests sequentially to prevent concurrent
 * requests from failing with `profile-busy`.
 */
export function enqueueProfileWork<T>(
  bridge: Bridge,
  profile: string,
  work: () => Promise<T>,
): Promise<T> {
  return scheduleProfileWork(bridge, profile, work);
}

export function sharedServerStatus(
  bridge: Bridge,
  profile: string,
  fresh = false,
): Promise<ServerStatusSnapshot> {
  let statuses = pendingServerStatuses.get(bridge);
  if (!statuses) {
    statuses = new Map();
    pendingServerStatuses.set(bridge, statuses);
  }
  const key = `${profile}:${fresh ? 'fresh' : 'cached'}`;
  const active = statuses.get(key);
  if (active) return active;
  const pending = enqueueProfileWork(bridge, profile, () =>
    bridge.describeServerStatus(profile, fresh),
  );
  statuses.set(key, pending);
  const clear = (): void => {
    if (statuses.get(key) === pending) statuses.delete(key);
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

function optionalInteger(value: unknown, at: string): number | undefined {
  return value === undefined ? undefined : integer(value, at);
}

function decodeServer(value: unknown, at: string): Server {
  const item = record(value, at);
  if (
    Object.keys(item).some(
      (key) =>
        !['id', 'name', 'label', 'configured_probe', 'accounts'].includes(key),
    )
  )
    throw new Error(`${at} contains unexpected server metadata fields`);
  return {
    id: string(item.id, `${at}.id`),
    name: string(item.name, `${at}.name`),
    label: nullableString(item.label, at + '.label'),
    configuredProbe: string(item.configured_probe, at + '.configured_probe'),
    host_id: null,
    chain: null,
    epoch: null,
    accounts: array(item.accounts, `${at}.accounts`, string),
    trust: { status: 'unknown' },
    compatibility: {
      status: 'requirement-unknown',
      error: {
        code: 'catalog-loading',
        message: 'Server facts have not been loaded.',
        retryable: false,
        ambiguous: false,
        fatal: false,
      },
    },
    passiveStatus: { status: 'loading' },
    connectivity: { status: 'unknown' },
    services: { chat: null },
    restrictions: [],
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
    stateDir: optionalString(item.stateDir, `${at}.stateDir`),
    reason: optionalString(item.reason, `${at}.reason`),
    foundSchema: optionalInteger(item.foundSchema, `${at}.foundSchema`),
    supportedSchema: optionalInteger(
      item.supportedSchema,
      `${at}.supportedSchema`,
    ),
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

export function decodeMaintenanceSnapshot(value: unknown): MaintenanceSnapshot {
  const at = 'maintenance snapshot';
  const item = record(value, at);
  const state = string(item.state, `${at}.state`);
  const generation = integer(item.generation, `${at}.generation`);
  const revision = integer(item.revision, `${at}.revision`);
  if (generation < 0) throw new Error(`${at}.generation must be nonnegative`);
  if (revision < 0) throw new Error(`${at}.revision must be nonnegative`);
  if (state === 'idle') return { state, generation, revision };
  const kind = string(item.kind, `${at}.kind`);
  if (!['export', 'import', 'verify', 'relocate', 'restart'].includes(kind))
    throw new Error(`${at}.kind is invalid`);
  const typedKind = kind as MaintenanceKind;
  if (state === 'active') {
    const phase = string(item.phase, `${at}.phase`);
    if (
      ![
        'selecting',
        'confirming',
        'quiescing',
        'running',
        'restoring',
      ].includes(phase)
    )
      throw new Error(`${at}.phase is invalid`);
    return {
      state,
      generation,
      revision,
      kind: typedKind,
      phase: phase as MaintenancePhase,
    };
  }
  if (state !== 'complete') throw new Error(`${at}.state is invalid`);
  const operationItem = record(item.operation, `${at}.operation`);
  const operationStatus = string(
    operationItem.status,
    `${at}.operation.status`,
  );
  let operation: MaintenanceOperationOutcome;
  if (operationStatus === 'cancelled' || operationStatus === 'completed') {
    operation = { status: operationStatus };
  } else if (operationStatus === 'failed') {
    operation = {
      status: 'failed',
      error: decodeCommandError(operationItem.error, `${at}.operation.error`),
    };
  } else {
    throw new Error(`${at}.operation.status is invalid`);
  }
  const dispositionItem = record(item.disposition, `${at}.disposition`);
  const dispositionStatus = string(
    dispositionItem.status,
    `${at}.disposition.status`,
  );
  let disposition: MaintenanceDisposition;
  if (dispositionStatus === 'continue-current-root') {
    disposition = { status: dispositionStatus };
  } else if (
    dispositionStatus === 'restart-selected-root' ||
    dispositionStatus === 'recovery-required'
  ) {
    disposition = {
      status: dispositionStatus,
      root: string(dispositionItem.root, `${at}.disposition.root`),
    };
  } else if (dispositionStatus === 'restoration-failed') {
    disposition = {
      status: dispositionStatus,
      root: string(dispositionItem.root, `${at}.disposition.root`),
      error: decodeCommandError(
        dispositionItem.error,
        `${at}.disposition.error`,
      ),
    };
  } else {
    throw new Error(`${at}.disposition.status is invalid`);
  }
  return {
    state,
    generation,
    revision,
    kind: typedKind,
    operation,
    disposition,
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

export function decodeCompatibility(
  value: unknown,
): ServerStatusSnapshot['compatibility'] {
  const item = record(value, 'compatibility');
  const status = string(item.status, 'compatibility.status');
  if (status === 'not-required' || status === 'missing') {
    if (Object.keys(item).some((key) => key !== 'status'))
      throw new Error('Unexpected compatibility fields');
    return {
      status: status === 'missing' ? 'required-unavailable' : 'not-required',
    };
  }
  const expiresAt = integer(item.expires_at, 'compatibility.expires_at');
  if (status === 'incompatible') {
    const reason = string(item.reason, 'compatibility.reason');
    if (
      !['drift', 'protocol-mismatch', 'unknown-capability'].includes(reason) ||
      Object.keys(item).some(
        (key) => !['status', 'expires_at', 'reason'].includes(key),
      )
    )
      throw new Error('Invalid compatibility failure');
    return {
      status,
      expiresAt,
      reason: reason as 'drift' | 'protocol-mismatch' | 'unknown-capability',
    };
  }
  if (
    status !== 'validated' ||
    Object.keys(item).some(
      (key) => !['status', 'expires_at', 'capabilities'].includes(key),
    )
  )
    throw new Error('Invalid compatibility status');
  const capabilities = array(
    item.capabilities,
    'compatibility.capabilities',
    string,
  );
  if (
    !capabilities.length ||
    new Set(capabilities).size !== capabilities.length ||
    capabilities.some(
      (capability) =>
        !PROTOCOL_CAPABILITIES.includes(capability as ProtocolCapability),
    )
  )
    throw new Error('Invalid compatibility capabilities');
  return {
    status: 'required',
    expiresAt,
    capabilities: capabilities as ProtocolCapability[],
  };
}

function decodeConnectionResult<S extends string>(
  value: unknown,
  expectedProfile: string,
  allowed: readonly S[],
  fields: readonly string[] = [],
): { status: S } | { status: 'failed'; error: CommandError } {
  const item = record(value, 'connectivity observation');
  const status = string(item.status, 'connectivity observation.status');
  if (status === 'failed') {
    if (Object.keys(item).some((key) => key !== 'status' && key !== 'error'))
      throw new Error('Unexpected connectivity failure fields.');
    const error = decodeCommandError(
      item.error,
      'connectivity observation.error',
    );
    if (error.details?.profile !== expectedProfile)
      throw new Error('Connectivity error belongs to a different profile.');
    return { status: 'failed', error };
  }
  if (
    Object.keys(item).length !== fields.length + 1 ||
    Object.keys(item).some((key) => key !== 'status' && !fields.includes(key))
  )
    throw new Error(
      'Successful connectivity observation contains unexpected fields.',
    );
  for (const candidate of allowed)
    if (candidate === status) return { status: candidate };
  throw new Error('Invalid connectivity observation status.');
}

export function decodeProfileReconciliation(
  value: unknown,
  expectedProfile: string,
): ProfileReconciliation {
  const item = record(value, 'connectivity response');
  if (
    item.profile !== expectedProfile ||
    Object.keys(item).some(
      (key) => !['profile', 'identity', 'compatibility'].includes(key),
    )
  )
    throw new Error(
      'Connectivity response belongs to a different profile or has unexpected fields.',
    );
  const observed = decodeConnectionResult(
    item.identity,
    expectedProfile,
    ['connected'] as const,
    ['hostId', 'configuredProbe'],
  );
  const identity: ProfileReconciliation['identity'] =
    observed.status === 'failed'
      ? observed
      : (() => {
          const value = record(item.identity, 'connectivity identity');
          const hostId = entityId(
            value.hostId,
            '02',
            'connectivity identity.hostId',
          );
          const configuredProbe = string(
            value.configuredProbe,
            'connectivity identity.configuredProbe',
          );
          if (!canonicalProbeEndpoint(configuredProbe))
            throw new Error('Invalid verified probe endpoint.');
          return { status: 'connected' as const, hostId, configuredProbe };
        })();
  return {
    profile: expectedProfile,
    identity,
    compatibility: decodeConnectionResult(item.compatibility, expectedProfile, [
      'not-required',
      'renewed',
      'unchanged',
    ] as const),
  };
}

export function decodeServerStatus(value: unknown): ServerStatusSnapshot {
  const item = record(value, 'describe_server_status response');
  const compatibility = decodeCompatibility(item.compatibility);
  const status = {
    profile: string(item.profile, 'describe_server_status.profile'),
    configuredProbe: string(
      item.configuredProbe,
      'describe_server_status.configuredProbe',
    ),
    host: nullable(item.host, 'describe_server_status.host', decodeStoredHost),
    compatibility,
    leaseRequired: compatibility.status !== 'not-required',
    leaseExpiresAt:
      'expiresAt' in compatibility ? compatibility.expiresAt : null,
    chatSupported: nullable(
      item.chatSupported,
      'describe_server_status.chatSupported',
      bool,
    ),
  };
  if ((status.host !== null) !== (status.chatSupported !== null))
    throw new Error(
      'describe_server_status returned inconsistent host support',
    );
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

function decodeServerVersion(value: unknown): ServerVersionInfo {
  const item = record(value, 'check_server.serverVersion');
  return {
    minimum: nullable(item.minimum, 'serverVersion.minimum', (entry, at) =>
      string(entry, at),
    ),
    newest: nullable(item.newest, 'serverVersion.newest', (entry, at) =>
      string(entry, at),
    ),
    message: string(item.message, 'serverVersion.message'),
    compatible: bool(item.compatible, 'serverVersion.compatible'),
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
  const version = item.serverVersion;
  return {
    profile: string(item.profile, 'check_server.profile'),
    acceptance: acceptance as CheckedServer['acceptance'],
    serverVersion:
      version === undefined || version === null
        ? null
        : decodeServerVersion(version),
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

export const decodeServerLabelResponse = (
  value: unknown,
): ServerLabelResponse => {
  const item = record(value, 'set_server_label response');
  const keys = Object.keys(item).sort();
  if (keys.join(',') !== 'changed,label,profile') {
    throw new Error('set_server_label response has an invalid shape');
  }
  return {
    profile: string(item.profile, 'set_server_label.profile'),
    label: nullable(item.label, 'set_server_label.label', (entry, at) =>
      string(entry, at),
    ),
    changed: bool(item.changed, 'set_server_label.changed'),
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
    const deviceId =
      item.deviceId == null
        ? undefined
        : deviceMemberId(item.deviceId, `${at}.deviceId`);
    if (deviceId && !deviceId.startsWith('08'))
      throw new Error(`${at}.deviceId is not a card key`);
    const cardSerial =
      item.cardSerial == null
        ? undefined
        : integer(item.cardSerial, `${at}.cardSerial`);
    if (cardSerial !== undefined && cardSerial <= 0)
      throw new Error(`${at}.cardSerial is invalid`);
    return {
      alias: string(item.alias, `${at}.alias`),
      state,
      ...(deviceId ? { deviceId } : {}),
      ...(cardSerial ? { cardSerial } : {}),
    };
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
  if (
    typeof value === 'object' &&
    value !== null &&
    reportedReadinessErrors.has(value)
  )
    return value as CommandError;
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

function isHardStateSchemaFailure(error: CommandError): boolean {
  return error.code === 'unsupported-schema';
}

function restrictionFromError(
  error: CommandError,
): ServerRestriction | undefined {
  if (error.code === 'unsupported-schema')
    return { kind: 'schema-incompatible', error };
  if (error.code === 'import-verification-required')
    return { kind: 'import-verification-required', error };
  const capability = error.details?.capability as
    ProtocolCapability | undefined;
  if (
    ['capability-denied', 'capability-unavailable'].includes(error.code) &&
    capability &&
    PROTOCOL_CAPABILITIES.includes(capability)
  )
    return { kind: 'capability-denied', capability, error };
  return undefined;
}

function schemaInstruction(profile: string): string {
  return `Inspect Settings → Servers → ${profile} and use a client that supports this schema. FOKS will not reset the data automatically.`;
}

export function shouldReportPassiveServerStatusError(error: unknown): boolean {
  return !isHardStateSchemaFailure(normalizeCommandError(error));
}

function notificationsOf(
  catalog: CatalogDto,
  servers: readonly Server[],
  nowSeconds: number,
): Notification[] {
  // Generate a notification for any server state that hides stores,
  // including unverified servers.
  const stopped: Partial<
    Record<AvailabilityReason, { detail: string; action: string }>
  > = {
    'check-in-expired': {
      detail: `The signed server check-in has expired. Vaults are unavailable until a newer check-in is verified.`,
      action: 'Check in',
    },
    'check-in-unavailable': {
      detail: `No usable signed check-in is available. Vaults are unavailable until one is verified.`,
      action: 'Inspect',
    },
    'server-status-unavailable': {
      detail: `Server status is unavailable. Vaults remain unavailable until signed status can be read.`,
      action: 'Inspect',
    },
    'verification-failed': {
      detail: `Server verification failed because its history does not match the pinned record. View server details to review the error.`,
      action: 'Inspect',
    },
    'verification-required': {
      detail: `This server has not been verified yet. Check the server to establish a connection and view its contents.`,
      action: 'Verify',
    },
  };
  const notes: Notification[] = servers.flatMap((server) => {
    const availability = serverFactAvailability(server, [], { nowSeconds });
    const copy = availability.available
      ? undefined
      : stopped[availability.reason];
    return copy
      ? [
          {
            id: `${availability.available ? 'available' : availability.reason}-${server.id}`,
            profile: server.id,
            severity: 'crit' as const,
            title: `${serverDisplayName(server)} is locked`,
            ...copy,
          },
        ]
      : [];
  });
  for (const [index, failure] of catalog.failures.entries()) {
    notes.push({
      id: `catalog-${failure.profile}-${failure.scope}-${index}`,
      profile: failure.profile,
      severity: failure.error.fatal ? 'crit' : 'warn',
      title:
        failure.scope === 'store'
          ? `Could not load vault on ${failure.profile}`
          : `Could not load ${failure.source} on ${failure.profile}`,
      detail: isHardStateSchemaFailure(failure.error)
        ? `${failure.error.message} ${schemaInstruction(failure.profile)}`
        : failure.error.message,
      action: failure.error.retryable ? 'Retry' : 'Inspect',
    });
  }
  return notes;
}

let nativeAgentGeneration = 0;

async function checked<T>(
  command: string,
  args: Record<string, unknown> | undefined,
  decode: (value: unknown) => T,
  reportReadiness = true,
): Promise<T> {
  const generation = nativeAgentGeneration;
  try {
    const value = decode(await invoke<unknown>(command, args));
    if (
      command === 'auto_recover_agent' ||
      command === 'retry_agent_connection'
    )
      nativeAgentGeneration++;
    return value;
  } catch (error) {
    const typed = normalizeCommandError(error);
    if (generation !== nativeAgentGeneration && isAgentReadinessError(typed)) {
      throw {
        ...typed,
        code: 'agent-request-retired',
        message: 'An earlier agent request was retired after recovery.',
        retryable: false,
        fatal: false,
      } satisfies CommandError;
    }
    if (reportReadiness) reportReadinessError(typed);
    throw typed;
  }
}

export function decodeAgentStatus(value: unknown): AgentStatus {
  const item = record(value, 'agent_status response');
  const state = string(item.state, 'agent_status.state');
  if (state === 'ready') return { state: 'ready' };
  if (state === 'bootstrap') {
    return {
      state: 'bootstrap',
      step: string(item.step, 'agent_status.step'),
    };
  }
  throw new Error('agent_status.state is not ready or bootstrap');
}

export const tauriBridge: Bridge = {
  runFirstRunAccountOperation: (request) =>
    checked('run_first_run_account_operation', { request }, (value) => {
      const result = decodeMutation(value);
      if (!result.applied)
        throw new Error(
          'Account setup did not acknowledge completion. Check its status.',
        );
      return result;
    }),
  firstRunOperationStatus: (attempt) =>
    checked('first_run_operation_status', { attempt }, (value) => {
      const result = record(value, 'first_run_operation_status');
      const binding = record(
        result.attempt,
        'first_run_operation_status.attempt',
      );
      if (
        Object.entries(attempt).some(
          ([key, value]) => binding[key] !== value,
        ) ||
        !['complete', 'rejected', 'unknown', 'running'].includes(
          String(result.outcome),
        )
      )
        throw new Error(
          'Account setup status belongs to a different operation.',
        );
      return result.outcome as 'complete' | 'rejected' | 'unknown' | 'running';
    }),
  cancelChat: (viewId) => invoke<void>('cancel_chat_requests', { viewId }),
  chat: (storeId, action, viewId) =>
    checked('chat_request', { storeId, action, viewId }, (value) =>
      decodeChatReply(value, storeId, action),
    ),
  native: true,
  appLockState: () => checked('app_lock_state', undefined, decodeAppLockState),
  windowState: () => checked('get_window_state', undefined, decodeWindowState),
  setTrafficLightsVisible: (visible) =>
    checked('set_traffic_lights_visible', { visible }, () => undefined),
  lockApp: () => checked('lock_app', undefined, decodeAppLockState),
  unlockApp: () => checked('unlock_app', undefined, decodeAppLockState),
  restartApp: () => invoke<void>('restart_app'),
  quitApp: () => invoke<void>('quit_app'),
  agentStatus: () => checked('agent_status', undefined, decodeAgentStatus),
  probeAgentStatus: () =>
    checked('agent_status', undefined, decodeAgentStatus, false),
  appInfo: () => checked('app_info', undefined, decodeAppInfo),
  listCatalog: async (onPartial) => {
    if (!onPartial) return checked('list_catalog', undefined, decodeCatalog);
    const channel = new Channel<unknown>();
    let failure: unknown;
    channel.onmessage = (value) => {
      try {
        onPartial(decodeCatalog(value));
      } catch (error) {
        failure = error;
      }
    };
    try {
      const catalog = await checked(
        'list_catalog_progressive',
        { onPartial: channel },
        decodeCatalog,
      );
      if (failure) throw failure;
      return catalog;
    } finally {
      channel.onmessage = () => undefined;
    }
  },
  listProfileCatalog: (profile) =>
    checked('list_profile_catalog', { profile }, decodeCatalog),
  listStores: () => checked('list_stores', undefined, decodeCatalog),
  listServers: (generation) =>
    checked(
      'list_servers',
      generation === undefined ? undefined : { generation },
      decodeServers,
    ),
  listAccounts: (generation) =>
    checked(
      'list_accounts',
      generation === undefined ? undefined : { generation },
      decodeAccounts,
    ),
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
  abandonGroupCreation: (storeId) =>
    checked('abandon_group_creation', { storeId }, decodeMutation),
  takeAgentConnectionLoss: () =>
    checked('take_agent_connection_loss', undefined, decodeConnectionLoss),
  retryAgentConnection: () =>
    checked('retry_agent_connection', undefined, decodeAgentStatus),
  autoRecoverAgent: () =>
    checked('auto_recover_agent', undefined, decodeAgentStatus, false),
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
  removeFederatedGroup: ({ storeId, remoteHostIdHex, remoteTeamIdHex }) =>
    checked(
      'expel_federated_group',
      { storeId, remoteHostIdHex, remoteTeamIdHex },
      decodeMutation,
    ),
  rerunGroupAdmission: (storeId, operationId) =>
    checked('rerun_group_admission', { storeId, operationId }, decodeMutation),
  chatLocal: (action) => checked('chat_local', { action }, decodeLocalSession),
  relocateClientState: () =>
    checked('relocate_client_state', {}, decodeMaintenanceSnapshot),
  maintainClientState: (action) =>
    checked('maintain_client_state', { action }, decodeMaintenanceSnapshot),
  clientStateMaintenanceStatus: () =>
    checked(
      'client_state_maintenance_status',
      undefined,
      decodeMaintenanceSnapshot,
    ),
  agentProcessInfo: () =>
    checked('agent_process_info', undefined, decodeAgentProcessInfo),
  restartAgent: (takeover) =>
    checked('restart_agent', { takeover }, decodeMaintenanceSnapshot),
  configureWebAdmin: (profile, accountAlias, destination) =>
    checked(
      'configure_web_admin',
      { profile, accountAlias, destination },
      decodeCommandAck,
    ),
  openWebAdmin: (profile, accountAlias, pin) =>
    checked('open_web_admin', { profile, accountAlias, pin }, decodeCommandAck),
  botAccount: (profile, accountAlias, action) =>
    checked(
      'bot_account_request',
      { profile, accountAlias, action },
      decodeBotReply,
    ),
  invitation: (profile, accountAlias, action, pin) =>
    enqueueProfileWork(tauriBridge, profile, () =>
      checked(
        'invitation_request',
        { profile, accountAlias, action, pin },
        decodeInvitationReply,
      ),
    ),
  setLocalAccountAlias: (store, label) =>
    checked(
      'set_local_account_alias',
      { accountStoreId: store, label },
      (value) => {
        const item = record(value, 'local alias response');
        if (
          Object.keys(item).sort().join(',') !== 'alias,store' ||
          item.store !== store ||
          item.alias !== label
        )
          throw new Error(
            'Local alias response belongs to a different account or label.',
          );
        return { store, alias: label };
      },
    ),
  renameAccount: (profile, accountAlias, action) =>
    checked(
      'rename_account_request',
      { profile, accountAlias, action },
      decodeRenameProgress,
    ),
  sso: (profile, accountAlias, action) =>
    checked(
      'sso_request',
      { profile, accountAlias, action },
      decodeSsoProgress,
    ),
  openSsoBrowser: (profile, accountAlias, operationId) =>
    checked(
      'open_sso_browser',
      { profile, accountAlias, operationId },
      decodeCopy,
    ),
  openChatLink: (url) => checked('open_chat_link', { url }, decodeCopy),
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
  describeServerStatus: (profile, fresh) =>
    checked(
      'describe_server_status',
      { profile, ...(fresh ? { fresh: true } : {}) },
      decodeServerStatus,
    ),
  checkServer: (profile) =>
    checked('check_server', { profile }, decodeCheckedServer),
  reconcileServer: (profile) =>
    checked('reconcile_server', { profile }, (value) =>
      decodeProfileReconciliation(value, profile),
    ),
  addServer: (profileName, probe) =>
    checked('add_server', { profileName, probe }, decodeAddedServer),
  setServerLabel: (profile, label) =>
    checked('set_server_label', { profile, label }, decodeServerLabelResponse),
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
  onChatNotification: async (listener) => {
    const activation = await listen('foks://chat-notification', () =>
      listener('activate'),
    );
    try {
      const error = await listen('foks://chat-notification-error', () =>
        listener('error'),
      );
      return () => {
        activation();
        error();
      };
    } catch (cause) {
      activation();
      throw cause;
    }
  },
  onOpenSettings: async (listener) => listen('foks://open-settings', listener),
  onMaintenanceStatus: async (listener) =>
    listen<unknown>('foks://maintenance-status', (event) => {
      listener(decodeMaintenanceSnapshot(event.payload));
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
  throw new Error('FOKS desktop host environment is required.');
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
      throw new Error('The agent returned an unrecognized group-detail error.');
    }
    throw typed;
  }
  return {
    store,
    source,
    code: typed.code,
    message: typed.message,
    retryable: typed.retryable,
    ...(typed.details
      ? {
          details: Object.fromEntries(
            Object.entries(typed.details).filter(
              ([, value]) => value !== undefined,
            ),
          ),
        }
      : {}),
  };
}

/**
 * Runs authenticated team discovery for each account store that is not bound to
 * any team yet, so a user who is added to a group after setup sees it on the
 * next ordinary launch. Discovery is best-effort: an unavailable server or a
 * profile that cannot be opened remains unbound until a later discovery attempt.
 *
 * Returns true when any discovery was attempted. The native command invalidates
 * the catalog even if no groups are found or the operation fails, so the
 * catalog must be reloaded after all of those cases.
 */
export async function discoverUnboundTeams(
  bridge: Bridge,
  snapshot: AgentSnapshot,
  isCurrent: () => boolean = () => true,
): Promise<boolean> {
  if (!profileInventoryComplete(snapshot, 'accounts')) return false;
  const bound = new Set<string>();
  for (const store of snapshot.stores) {
    if (store.kind === 'team')
      bound.add(`${store.server}\u0000${store.account}`);
  }
  let attempted = false;
  for (const account of snapshot.accounts) {
    if (!isCurrent()) break;
    if (bound.has(`${account.server}\u0000${account.alias}`)) continue;
    if (
      !snapshot.stores.some(
        (store) =>
          store.kind === 'account' &&
          store.id === account.store &&
          store.server === account.server &&
          store.account === account.alias,
      )
    )
      continue;
    const server = snapshot.servers.find(
      (entry) => entry.id === account.server,
    );
    if (
      !server ||
      !serverFactAvailability(
        server,
        snapshot.observedExpiredLeases,
        { agentReady: snapshot.agent.state === 'ready' },
        ['teams'],
      ).available
    )
      continue;
    attempted = true;
    try {
      await enqueueProfileWork(bridge, account.server, async () => {
        if (isCurrent())
          await bridge.discoverGroups(account.server, account.alias);
      });
    } catch {
      // Best effort: leave the account unbound until a later launch.
    }
  }
  return attempted;
}

/** Load the current snapshot state. */
/**
 * Loads the snapshot from the native catalog. The load is a sequence of reads
 * bound to one catalog snapshot; when a concurrent load or mutation replaces
 * that snapshot mid-sequence the native side reports catalog-required, and
 * one further attempt is made against the new snapshot before giving up.
 */
export async function loadSnapshot(
  bridge: Bridge,
  base = bridge.fixtureSnapshot,
  nowSeconds: number = Math.floor(Date.now() / 1000),
  onPartial?: (snapshot: AgentSnapshot) => void,
  isCurrent: () => boolean = () => true,
): Promise<AgentSnapshot> {
  try {
    return await loadSnapshotOnce(
      bridge,
      base,
      nowSeconds,
      onPartial,
      isCurrent,
    );
  } catch (error) {
    if (
      !isCurrent() ||
      normalizeCommandError(error).code !== 'catalog-required'
    )
      throw error;
    return loadSnapshotOnce(bridge, base, nowSeconds, onPartial, isCurrent);
  }
}

async function loadSnapshotOnce(
  bridge: Bridge,
  base: AgentSnapshot | undefined,
  nowSeconds: number,
  onPartial: ((snapshot: AgentSnapshot) => void) | undefined,
  isCurrent: () => boolean,
): Promise<AgentSnapshot> {
  if (!isCurrent()) throw new Error('Catalog load was retired.');
  const agent = await bridge.agentStatus();
  if (!isCurrent()) throw new Error('Catalog load was retired.');
  if (agent.state !== 'ready') {
    const error: CommandError = {
      code: 'bootstrap-required',
      message:
        'The local agent must finish initialization before loading vaults.',
      retryable: false,
      ambiguous: false,
      fatal: false,
      details: { reason: agent.step },
    };
    reportReadinessError(error);
    throw error;
  }
  let accepting = true;
  let revision = 0;
  let partialFailure: unknown;
  try {
    const response = await bridge.listCatalog(
      onPartial
        ? (partial) => {
            if (!accepting || !isCurrent()) return;
            const current = ++revision;
            void projectCatalog(
              bridge,
              partial,
              base,
              nowSeconds,
              agent,
              true,
            ).then(
              (snapshot) => {
                if (accepting && isCurrent() && current === revision)
                  onPartial(snapshot);
              },
              (error: unknown) => {
                partialFailure = error;
              },
            );
          }
        : undefined,
    );
    if (!isCurrent()) throw new Error('Catalog load was retired.');
    const snapshot = await projectCatalog(
      bridge,
      response,
      base,
      nowSeconds,
      agent,
      false,
    );
    if (partialFailure) throw partialFailure;
    return snapshot;
  } finally {
    accepting = false;
  }
}

export async function loadProfileSnapshot(
  bridge: Bridge,
  profile: string,
  base: AgentSnapshot,
  nowSeconds: number = Math.floor(Date.now() / 1000),
  isCurrent: () => boolean = () => true,
  background?: BackgroundHistoryWork,
): Promise<AgentSnapshot> {
  if (!isCurrent()) throw new Error('Catalog load was retired.');
  const response = await scheduleProfileWork(
    bridge,
    profile,
    () => bridge.listProfileCatalog(profile),
    background,
  );
  if (!isCurrent()) throw new Error('Catalog load was retired.');
  if (
    response.profiles.length !== 1 ||
    response.profiles[0] !== profile ||
    [...response.stores, ...response.knownStores].some(
      (store) => store.server !== profile,
    ) ||
    response.inventory.some((entry) => entry.profile !== profile) ||
    response.failures.some((entry) => entry.profile !== profile) ||
    response.blockedProfiles.some((entry) => entry !== profile) ||
    response.fullItemReads?.some((entry) => entry !== profile) ||
    response.localMetadata?.profiles.some(
      (entry) => entry.profile !== profile,
    ) ||
    response.localMetadata?.accounts.some((entry) => entry.server !== profile)
  )
    throw new Error('list_profile_catalog returned a different scope.');
  if (
    bridge.native &&
    (!response.localMetadata || response.localMetadata.profiles.length !== 1)
  )
    throw new Error('list_profile_catalog omitted scoped metadata.');
  const projected = await projectCatalog(
    bridge,
    response,
    base,
    nowSeconds,
    base.agent,
    false,
    profile,
    background,
  );
  if (!isCurrent()) throw new Error('Catalog load was retired.');
  return mergeProfileSnapshot(base, projected, profile);
}

async function projectCatalog(
  bridge: Bridge,
  response: CatalogDto,
  base: AgentSnapshot | undefined,
  nowSeconds: number,
  agent: AgentStatus,
  partial: boolean,
  profileScope?: string,
  background?: BackgroundHistoryWork,
): Promise<AgentSnapshot> {
  const embeddedMetadata =
    partial || (bridge.native && profileScope !== undefined);
  const globalFailure = response.failures.find((failure) =>
    ['bootstrap-required', 'agent-lost', 'version-mismatch'].includes(
      failure.error.code,
    ),
  );
  if (globalFailure) {
    reportReadinessError(globalFailure.error);
    throw globalFailure.error;
  }
  const liveStores = response.stores as Store[];
  const storesById = new Map<StoreRef, Store>();
  {
    for (const store of base?.stores ?? []) {
      const inventory = response.inventory.find(
        (entry) => entry.profile === store.server,
      );
      if (
        response.profiles.includes(store.server) &&
        !(store.kind === 'account'
          ? inventory?.accountsComplete
          : inventory?.teamsComplete)
      )
        storesById.set(store.id, store);
    }
  }
  for (const store of response.knownStores as Store[])
    storesById.set(store.id, store);
  for (const store of liveStores) storesById.set(store.id, store);
  const stores = [...storesById.values()];
  const liveStoreIds = new Set(liveStores.map((store) => store.id));
  const storeInventory = stores.map((store) => {
    const failures = response.failures.filter((entry) =>
      entry.scope === 'store'
        ? entry.store === store.id
        : entry.profile === store.server && entry.source === 'KV catalog',
    );
    const restrictions = failures.flatMap((failure) => {
      const restriction = restrictionFromError(failure.error);
      return restriction ? [restriction] : [];
    });
    const failure = failures[0];
    const complete = catalogStoreComplete(response, store, partial);
    const previous = base?.storeInventory.find(
      (entry) => entry.store === store.id,
    );
    const previousStore = base?.stores.find((entry) => entry.id === store.id);
    if (
      partial &&
      !complete &&
      !failure &&
      previous &&
      previousStore &&
      sameStoreIdentity(previousStore, store)
    )
      return previous;
    return liveStoreIds.has(store.id) && complete && !failure
      ? {
          store: store.id,
          status: 'available' as const,
          restrictions,
        }
      : {
          store: store.id,
          status:
            partial && !failure
              ? ('loading' as const)
              : ('unavailable' as const),
          restrictions,
          ...(failure ? { error: failure.error } : {}),
        };
  });
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
  const profileInventory = response.profiles.map((profile) => {
    const inventory = response.inventory.find(
      (entry) => entry.profile === profile,
    );
    return {
      profile,
      accounts: inventory?.accountsComplete
        ? ('complete' as const)
        : ('unavailable' as const),
      teams: inventory?.teamsComplete
        ? ('complete' as const)
        : ('unavailable' as const),
    };
  });
  // When a server profile is blocked, skip roster queries for that server.
  const blockedProfiles = new Set(response.blockedProfiles);
  const loadingError = normalizeCommandError({
    code: 'catalog-loading',
    message: 'The profile catalog is still loading.',
    retryable: false,
    ambiguous: false,
    fatal: false,
  });
  const listedServers: Server[] = embeddedMetadata
    ? response.profiles.map((profile) => {
        const metadata = response.localMetadata?.profiles.find(
          (entry) => entry.profile === profile,
        );
        const previous = base?.servers.find((entry) => entry.id === profile);
        return {
          id: profile,
          name: profile,
          label: metadata?.label ?? previous?.label ?? null,
          configuredProbe:
            metadata?.configuredProbe ?? previous?.configuredProbe ?? profile,
          host_id: null,
          chain: null,
          epoch: null,
          accounts: liveStores
            .filter(
              (store) => store.server === profile && store.kind === 'account',
            )
            .map((store) => store.account),
          trust: blockedProfiles.has(profile)
            ? { status: 'blocked', error: loadingError }
            : { status: 'unknown' },
          compatibility: { status: 'requirement-unknown', error: loadingError },
          passiveStatus: {
            status: 'failed',
            source: 'describe-server-status',
            error: loadingError,
          },
          connectivity: { status: 'unknown' },
          services: { chat: null },
          restrictions: previous?.restrictions ?? [],
        };
      })
    : (await bridge.listServers(response.generation)).filter(
        (server) => profileScope === undefined || server.id === profileScope,
      );
  const statusResults =
    bridge.native || partial
      ? await Promise.all(
          listedServers
            .filter(
              (server) =>
                server.trust.status !== 'blocked' &&
                !blockedProfiles.has(server.id),
            )
            .map(async (server) => {
              try {
                const cached = response.localMetadata?.profiles.find(
                  (entry) => entry.profile === server.id,
                );
                if (embeddedMetadata && (cached?.error || !cached?.status))
                  throw (
                    cached?.error ??
                    (partial
                      ? loadingError
                      : new Error('Server status was not returned.'))
                  );
                const status = embeddedMetadata
                  ? cached!.status!
                  : await sharedServerStatus(bridge, server.id);
                if (status.profile !== server.id) {
                  throw new Error(
                    'describe_server_status returned a different profile.',
                  );
                }
                return { profile: server.id, status };
              } catch (error) {
                const typed = normalizeCommandError(error);
                if (
                  typed.code === 'bootstrap-required' ||
                  typed.code === 'agent-lost' ||
                  typed.code === 'version-mismatch'
                ) {
                  reportReadinessError(typed);
                  throw typed;
                }
                return {
                  profile: server.id,
                  error: typed,
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
  const incompatibleSchemaProfiles = new Set(
    response.failures
      .filter((failure) => isHardStateSchemaFailure(failure.error))
      .map((failure) => failure.profile),
  );
  const servers = listedServers.map((server) => {
    if (!bridge.native && !partial) return server;
    const scopedFailures = response.failures.filter(
      (failure) => failure.scope === 'profile' && failure.profile === server.id,
    );
    const restrictions: ServerRestriction[] = scopedFailures.flatMap(
      (failure) => {
        const restriction = restrictionFromError(failure.error);
        return restriction ? [restriction] : [];
      },
    );
    const status = statuses.get(server.id);
    const statusError = statusFailures.get(server.id);
    const statusRestriction = statusError && restrictionFromError(statusError);
    const previous = base?.servers.find((entry) => entry.id === server.id);
    const pendingSameIdentity =
      partial &&
      statusError?.code === 'catalog-loading' &&
      previous?.configuredProbe === server.configuredProbe;
    const allRestrictions = [
      ...(pendingSameIdentity ||
      (partial &&
        !catalogItemsComplete(response, server.id, partial) &&
        previous?.configuredProbe === server.configuredProbe)
        ? (previous?.restrictions ?? [])
        : []),
      ...restrictions,
      ...(statusRestriction ? [statusRestriction] : []),
    ];
    if (server.trust.status === 'blocked' || blockedProfiles.has(server.id)) {
      const trustFailure = scopedFailures.find((failure) =>
        [
          'rollback-detected',
          'checkpoint-reset-required',
          'server-verification-failed',
        ].includes(failure.error.code),
      )?.error;
      return {
        ...server,
        trust: {
          status: 'blocked' as const,
          error:
            trustFailure ??
            (server.trust.status === 'blocked'
              ? server.trust.error
              : normalizeCommandError({
                  code: 'server-verification-failed',
                  message: 'Server verification failed.',
                  retryable: false,
                  ambiguous: false,
                  fatal: false,
                })),
        },
        restrictions: allRestrictions,
        services: { chat: null },
      };
    }
    if (
      pendingSameIdentity &&
      previous?.trust.status === 'verified' &&
      previous.passiveStatus.status === 'available'
    )
      return {
        ...previous,
        label: server.label,
        restrictions: allRestrictions,
      };
    if (!status || statusError)
      return {
        ...server,
        trust: { status: 'unknown' as const },
        passiveStatus:
          statusError?.code === 'catalog-loading'
            ? { status: 'loading' as const }
            : {
                status: 'failed' as const,
                source: 'describe-server-status' as const,
                error:
                  statusError ??
                  normalizeCommandError(
                    new Error('Server status was not returned.'),
                  ),
              },
        compatibility: {
          status: 'requirement-unknown' as const,
          error:
            statusError ??
            normalizeCommandError(
              new Error('Compatibility status is unknown.'),
            ),
        },
        services: { chat: null },
        restrictions: allRestrictions,
      };
    return {
      ...server,
      configuredProbe: status.configuredProbe,
      host_id: status.host?.hostId ?? null,
      chain: status.host?.chain ?? null,
      epoch: status.host?.epoch ?? null,
      trust: status.host
        ? { status: 'verified' as const }
        : { status: 'unprobed' as const },
      compatibility: status.compatibility,
      passiveStatus: {
        status: 'available' as const,
        source: 'signed-server-status' as const,
      },
      services: { chat: status.chatSupported },
      restrictions: allRestrictions,
    };
  });
  const observedExpiredLeases = (base?.observedExpiredLeases ?? []).filter(
    (entry) => {
      const server = servers.find((server) => server.id === entry.profile);
      const previous = base?.servers.find(
        (server) => server.id === entry.profile,
      );
      return (
        server &&
        previous &&
        server.configuredProbe === previous.configuredProbe &&
        (!server.host_id ||
          !previous.host_id ||
          server.host_id === previous.host_id)
      );
    },
  );
  for (const server of servers) {
    const lease = server.compatibility;
    if (
      (lease.status === 'required' || lease.status === 'incompatible') &&
      lease.expiresAt <= nowSeconds &&
      !observedExpiredLeases.some(
        (entry) =>
          entry.profile === server.id && entry.expiresAt === lease.expiresAt,
      )
    )
      observedExpiredLeases.push({
        profile: server.id,
        expiresAt: lease.expiresAt,
      });
  }
  const sameIdentityStores = new Set(
    stores
      .filter((store) => {
        const previous = base?.stores.find((entry) => entry.id === store.id);
        const server = servers.find((entry) => entry.id === store.server);
        const previousServer = base?.servers.find(
          (entry) => entry.id === store.server,
        );
        return (
          previous &&
          sameStoreIdentity(previous, store) &&
          server &&
          previousServer &&
          server.configuredProbe === previousServer.configuredProbe &&
          (!server.host_id ||
            !previousServer.host_id ||
            server.host_id === previousServer.host_id)
        );
      })
      .map((store) => store.id),
  );
  for (const [index, inventory] of storeInventory.entries()) {
    const store = stores.find((store) => store.id === inventory.store)!;
    if (
      partial &&
      !catalogStoreComplete(response, store, partial) &&
      !sameIdentityStores.has(store.id) &&
      inventory.status === 'available'
    )
      storeInventory[index] = {
        store: store.id,
        status: 'loading',
        restrictions: [],
      };
  }
  const rawAccounts = embeddedMetadata
    ? (response.localMetadata?.accounts ?? [])
    : (await bridge.listAccounts(response.generation)).filter(
        (account) =>
          profileScope === undefined || account.server === profileScope,
      );
  const unavailableServers = new Set(
    servers
      .filter(
        (server) =>
          !serverFactAvailability(server, observedExpiredLeases, { nowSeconds })
            .available,
      )
      .map((server) => server.id),
  );
  // Inactive teams cannot query members or federation until setup is complete;
  // skip those reads.
  const teams = liveStores.filter(
    (store) =>
      !partial &&
      store.kind === 'team' &&
      store.active &&
      !blockedProfiles.has(store.server) &&
      !unavailableServers.has(store.server) &&
      servers.some(
        (server) =>
          server.id === store.server &&
          serverFactAvailability(
            server,
            observedExpiredLeases,
            { nowSeconds },
            ['teams'],
          ).available,
      ),
  );
  const rosters = await Promise.all(
    teams.map(async (store) => {
      const { parties, federation, failures } = await scheduleProfileWork(
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
        background
          ? { ...background, key: `${background.key}:roster:${store.id}` }
          : undefined,
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
  if (partial) {
    for (const account of base?.accounts ?? []) {
      if (
        sameIdentityStores.has(account.store) &&
        !accounts.some((entry) => entry.store === account.store)
      )
        accounts.push(account);
    }
  }
  if (!partial && accounts.length !== availableAccountStoreIds.size) {
    throw new Error('list_accounts omitted an available account store.');
  }
  const serverIds = new Set(servers.map((server) => server.id));
  if (stores.some((store) => !serverIds.has(store.server))) {
    throw new Error('list_servers omitted a server used by the catalog.');
  }
  const federation = rosters.flatMap((roster) => roster.federation);
  const displayServerById = (profile: string): string => {
    const server = servers.find((candidate) => candidate.id === profile);
    return server ? serverDisplayName(server) : profile;
  };
  const parties: Party[] = rosters
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
            ? `${admissions[0].remote_team_alias} @ ${displayServerById(admissions[0].remote_profile)}`
            : party.team_name,
      };
    });
  if (partial) {
    parties.push(
      ...(base?.parties ?? []).filter((entry) =>
        sameIdentityStores.has(entry.store),
      ),
    );
    federation.push(
      ...(base?.federation ?? []).filter((entry) =>
        sameIdentityStores.has(entry.store),
      ),
    );
    groupDetailFailures.push(
      ...(base?.groupDetailFailures ?? []).filter((entry) =>
        sameIdentityStores.has(entry.store),
      ),
    );
  }
  const baseItems = new Map(
    (base?.items ?? []).map((item) => [itemKey(item), item]),
  );
  const items: Item[] = response.items
    .filter((item) => {
      const store = liveStores.find((candidate) => candidate.id === item.store);
      return Boolean(
        store &&
        !unavailableServers.has(store.server) &&
        servers.some(
          (server) =>
            server.id === store.server &&
            serverFactAvailability(
              server,
              observedExpiredLeases,
              { nowSeconds },
              store.kind === 'team' ? ['teams', 'kv'] : ['kv'],
            ).available,
        ) &&
        storeInventory.find((entry) => entry.store === store.id)?.status ===
          'available',
      );
    })
    .map((item) => ({
      ...(bridge.native ? {} : baseItems.get(itemKey(item))),
      ...item,
    }));
  const itemKeys = new Set(items.map(itemKey));
  for (const item of base?.items ?? []) {
    const store = stores.find((store) => store.id === item.store);
    if (
      !store ||
      !sameIdentityStores.has(item.store) ||
      itemKeys.has(itemKey(item))
    )
      continue;
    if (
      catalogStoreComplete(response, store, partial) &&
      storeInventory.find((entry) => entry.store === item.store)?.status ===
        'available'
    )
      continue;
    const metadata = { ...item };
    delete metadata.value;
    delete metadata.target;
    items.push(bridge.native ? metadata : item);
  }
  const withFreshness = (snapshot: AgentSnapshot): AgentSnapshot => ({
    ...snapshot,
    servers: snapshot.servers.map((server) => ({
      ...server,
      connectivity: retainProfileConnection(
        server,
        base?.servers.find((previous) => previous.id === server.id)
          ?.connectivity,
      ),
    })),
    observedExpiredLeases,
    catalogFreshness: projectCatalogFreshness(
      snapshot,
      base,
      response,
      partial,
      nowSeconds,
    ),
  });
  if (!bridge.native) {
    if (!base)
      throw new Error('The mock bridge did not supply its fixture snapshot.');
    return withFreshness({
      ...base,
      agent,
      servers,
      accounts,
      stores,
      storeInventory,
      profileInventory,
      catalogProfiles: response.profiles,
      profileInventoryStatus: 'complete',
      items,
      parties,
      federation,
      groupDetailFailures,
    });
  }
  return withFreshness({
    agent,
    servers,
    accounts,
    stores,
    storeInventory,
    profileInventory,
    catalogProfiles: response.profiles,
    profileInventoryStatus: 'complete',
    items,
    parties,
    federation,
    groupDetailFailures,
    devices: [],
    yubiAccounts: [],
    cardsConnected: [],
    notifications: [
      ...notificationsOf(response, servers, nowSeconds).filter(
        (note) =>
          ![...statusFailures.keys()].some(
            (profile) => note.id === `server-status-unavailable-${profile}`,
          ),
      ),
      ...[...statusFailures]
        .filter(([, error]) => error.code !== 'catalog-loading')
        .map(([profile, error]) => ({
          id: `status-unavailable-${profile}`,
          profile,
          severity: 'crit' as const,
          title: `Status for ${profile} is unavailable`,
          detail: `${error.message} Server contents are unavailable until the connection status is verified.${
            incompatibleSchemaProfiles.has(profile)
              ? ` ${schemaInstruction(profile)}`
              : ''
          }`,
          action: 'Inspect',
        })),
      ...groupDetailFailures.map((failure) => ({
        id: `group-${failure.source}-unavailable-${failure.store}`,
        profile: stores.find((store) => store.id === failure.store)?.server,
        severity: 'warn' as const,
        title:
          failure.source === 'roster'
            ? 'Team member list is unavailable'
            : 'Team shared access is unavailable',
        detail: failure.message,
        action: failure.retryable ? 'Refresh' : 'Inspect',
      })),
    ],
    observedExpiredLeases,
    plaintext: {},
  });
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
