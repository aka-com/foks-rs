import type {
  AgentSnapshot,
  AgentStatus,
  Account,
  FederationEntry,
  Party,
  Server,
  StoreRef,
} from '../model';
import type { ProfileReconciliation } from '../profile-connectivity';
import type { BotAction, BotReply } from '../bot-contract';
import type { ChatAction, ChatReply } from '../chat-contract';
import type { LocalAction, LocalSession } from '../chat/local-contract';
import type { InvitationAction, InvitationReply } from '../invitation-contract';
import type { RenameAction, RenameProgress } from '../rename-contract';
import type { SsoAction, SsoProgress } from '../sso-contract';
import type {
  AdmitGroupRequest,
  CreateGroupRequest,
  GroupDetailsDto,
  GroupDiscoveryResponse,
  GroupMemberRequest,
  GroupRoleRequest,
  RemoveFederatedGroupRequest,
} from './accounts-groups';
import type {
  AgentProcessInfo,
  AppInfo,
  AppLockState,
  CommandAck,
  DropHoverEvent,
  MaintenanceSnapshot,
  TimingBatch,
  Unlisten,
  WindowStateEvent,
} from './core';
import type {
  AccountDevice,
  BackupEnrollment,
  BackupPhraseResponse,
  BackupRevocation,
  DeviceProvision,
  FirstRunAccountRequest,
  FirstRunFixture,
  FirstRunPassphraseRequest,
  GoProfileDiscovery,
  PairingOffer,
  PassphraseReport,
  PendingOperation,
  ResetPreview,
  TrackedAccountAttempt,
  TrackedAccountRequest,
} from './enrollment';
import type {
  CheckedProfileResponse,
  CheckedServer,
  ServerLabelResponse,
  ServerStatusSnapshot,
} from './servers';
import type {
  CatalogDto,
  CopyResponse,
  CreateFileRequest,
  CreateFolderRequest,
  CreateLinkRequest,
  CreateTextRequest,
  DownloadResponse,
  EditTextRequest,
  ImportDroppedFileRequest,
  ItemRequest,
  MutationResponse,
  ReadItemResponse,
  RemoveItemRequest,
  ReplaceDroppedFileRequest,
} from './vault-catalog';
import type { YubiCommand, YubiEnrollment } from './yubikey';

export interface Bridge {
  relocateClientState(): Promise<MaintenanceSnapshot>;
  maintainClientState(
    action: 'export' | 'import' | 'verify',
  ): Promise<MaintenanceSnapshot>;
  clientStateMaintenanceStatus(): Promise<MaintenanceSnapshot>;
  /** The process on the agent socket. */
  agentProcessInfo(): Promise<AgentProcessInfo>;
  /**
   * The backend's timing log from a cursor, for Copy diagnostics. Only the
   * native bridge has one.
   */
  diagnosticTimings?(since: number): Promise<TimingBatch>;
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
  /**
   * How many chat submissions the renderer holds that nothing durable would
   * recover. The window asks before discarding them, so this is reported
   * whenever the count changes and cleared when the unlocked shell goes.
   */
  setUnsentMessages(count: number): Promise<void>;
  lockApp(): Promise<AppLockState>;
  unlockApp(): Promise<AppLockState>;
  restartApp(): Promise<void>;
  quitApp(): Promise<void>;
  agentStatus(): Promise<AgentStatus>;
  probeAgentStatus?(): Promise<AgentStatus>;
  appInfo(): Promise<AppInfo>;
  /**
   * Returns the full catalog including stores. Mutually exclusive with
   * `listStores`. `fresh` requires every store's first page to be listed for
   * this read instead of being served from what the agent retained, which the
   * renderer asks for on a refresh the user started and on a read back of a
   * write.
   */
  listCatalog(
    onPartial?: (catalog: CatalogDto) => void,
    fresh?: boolean,
  ): Promise<CatalogDto>;
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
  /** Opens the system pane where desktop alerts are permitted. */
  openNotificationSettings(): Promise<CopyResponse>;
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
