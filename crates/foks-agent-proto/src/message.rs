use serde::{Deserialize, Serialize};
use zeroize::Zeroize as _;

pub const PROTOCOL_VERSION: u32 = 9;

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SecretString([REDACTED])")
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct YubiFederationUnlockInput {
    pub profile: String,
    pub alias: String,
    pub pin: SecretString,
}

impl std::fmt::Debug for YubiFederationUnlockInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("YubiFederationUnlockInput")
            .field("profile", &self.profile)
            .field("alias", &self.alias)
            .field("pin", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct YubiRetryConfiguration {
    pub puk: SecretString,
    pub pin_attempts: u8,
    pub puk_attempts: u8,
}

impl std::fmt::Debug for YubiRetryConfiguration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("YubiRetryConfiguration")
            .field("puk", &"<redacted>")
            .field("pin_attempts", &self.pin_attempts)
            .field("puk_attempts", &self.puk_attempts)
            .finish()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Request {
    pub version: u32,
    pub id: u64,
    pub operation: Operation,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FederationRole {
    Member,
    Admin,
    Owner,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TeamRole {
    Member,
    Admin,
    Owner,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TeamKind {
    Named,
    AdHoc,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct AccountStoreRef {
    pub profile: String,
    pub account_alias: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AccountSummary {
    pub profile: String,
    pub alias: String,
    pub username: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TeamSummary {
    pub alias: String,
    pub account_alias: String,
    pub team_id_hex: String,
    pub kind: String,
    pub name: Option<String>,
    pub active: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProfileOverview {
    pub profile: String,
    pub accounts: ResponseResult,
    pub teams: ResponseResult,
    pub server_status: ResponseResult,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TeamDetailsSummary {
    pub members: ResponseResult,
    pub federation: ResponseResult,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "store_kind", rename_all = "kebab-case")]
pub enum KnownStoreSummary {
    Account {
        account_alias: String,
        last_seen_at: u64,
    },
    Team {
        account_alias: String,
        team_alias: String,
        team_id_hex: String,
        team_kind: TeamKind,
        name: Option<String>,
        active: bool,
        last_seen_at: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeviceSummary {
    pub id_hex: String,
    pub name: Option<String>,
    pub role: String,
    pub current: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GoProfileCandidate {
    pub candidate_id: String,
    pub username: Option<String>,
    pub server_hint: Option<String>,
    pub host_id_hex: String,
    pub user_id_hex: String,
    pub device_id_hex: String,
    pub role: String,
    pub storage_kind: String,
    pub hidden: bool,
    pub provisional: bool,
    pub pairable: bool,
    pub copyable: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GoProfileDiscovery {
    pub installed: bool,
    pub candidates: Vec<GoProfileCandidate>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PendingOperationKind {
    AccountSignup,
    DeviceProvision,
    PairingOffer,
    PairingAcceptance,
    AccountRecovery,
    YubiEnrollment,
    TeamCreation,
    TeamMemberAddition,
    TeamMemberEdit,
    FederationExpulsion,
    TeamRekey,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PendingOperationSummary {
    pub kind: PendingOperationKind,
    pub alias: String,
    pub target: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResetArtifactKind {
    HardState,
    SoftState,
    ProtectedMutations,
    CredentialsAndResumables,
    ExternalRollbackCheckpoint,
    ExternalDatabaseClaim,
    ExternalPublicationAuthorization,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResetArtifactSummary {
    pub kind: ResetArtifactKind,
    pub entries: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResetStatePreview {
    pub profile: String,
    pub resumables: Vec<PendingOperationSummary>,
    pub artifacts: Vec<ResetArtifactSummary>,
    pub token: SecretString,
    pub expires_in_seconds: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BackupEnrollmentSummary {
    pub backup_alias: String,
    pub account_alias: String,
    pub backup_id_hex: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServerStatusSnapshot {
    pub profile: String,
    pub configured_probe: String,
    pub host: Option<StoredHostStatus>,
    pub lease_required: bool,
    pub lease_expires_at: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StoredHostStatus {
    pub lookup_name: String,
    pub canonical_name: String,
    pub host_id_hex: String,
    pub host_chain_sequence: u64,
    pub merkle_epoch: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct TeamStoreRef {
    pub profile: String,
    pub account_alias: String,
    pub team_alias: String,
    pub team_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "store", rename_all = "kebab-case")]
pub enum KvStoreRef {
    Account(AccountStoreRef),
    Team(TeamStoreRef),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum KvRole {
    Member { visibility: i16 },
    Admin,
    Owner,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KvEntryMetadata {
    pub path: String,
    pub node_type: String,
    pub version: u64,
    pub size: Option<u64>,
    pub read_role: KvRole,
    pub write_role: KvRole,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KvPage {
    pub snapshot_version: u64,
    pub entries: Vec<KvEntryMetadata>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum KvPrecondition {
    Create,
    ExactVersion { version: u64 },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KvReadResult {
    pub store: KvStoreRef,
    pub path: String,
    pub version: u64,
    pub node_type: String,
    pub size: Option<u64>,
    pub read_role: KvRole,
    pub write_role: KvRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symlink_target: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KvChunkResult {
    pub store: KvStoreRef,
    pub path: String,
    pub version: u64,
    pub offset: u64,
    pub content: Vec<u8>,
    pub eof: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KvUploadHeader {
    pub store: KvStoreRef,
    pub path: String,
    pub total_length: u64,
    pub read_role: KvRole,
    pub write_role: KvRole,
    pub precondition: KvPrecondition,
    /// Creates intermediate parent directories along the path if they do not exist.
    #[serde(default)]
    pub mkdir_p: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KvUploadFrame {
    pub version: u32,
    pub id: u64,
    #[serde(flatten)]
    pub payload: KvUploadPayload,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "frame", rename_all = "kebab-case")]
pub enum KvUploadPayload {
    Chunk { offset: u64, content: Vec<u8> },
    Commit,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum AgentStatus {
    Bootstrap { step: String },
    Ready,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CredentialBackend {
    Native,
    PrivateFile,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "generation", rename_all = "kebab-case")]
pub enum ProfileProtocol {
    V019,
    CurrentProbeOnly {
        canary_public_key: String,
        lease_url: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ProfileTrust {
    WebPki,
    CertificateDer { path: String },
}

impl Request {
    pub fn new(id: u64, operation: Operation) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id,
            operation,
        }
    }
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "operation", rename_all = "kebab-case")]
pub enum Operation {
    Chat {
        store: TeamStoreRef,
        action: crate::chat::ChatAction,
    },
    Ping,
    AgentStatus,
    DiscoverGoProfiles,
    InitializeState {
        backend: CredentialBackend,
    },
    AddProfile {
        name: String,
        probe: String,
        protocol: ProfileProtocol,
        trust: ProfileTrust,
    },
    CheckAndAddProfile {
        name: String,
        probe: String,
        protocol: ProfileProtocol,
        trust: ProfileTrust,
    },
    CheckAndAddGoProfile {
        candidate_id: String,
        name: String,
        probe: String,
        protocol: ProfileProtocol,
        trust: ProfileTrust,
    },
    RemoveProfile {
        name: String,
    },
    DescribeResetHardState {
        profile: String,
    },
    ResetHardState {
        profile: String,
        token: SecretString,
    },
    ListProfiles,
    Probe {
        profile: String,
    },
    RefreshLease {
        profile: String,
    },
    ListKnownStores {
        profile: String,
    },
    ListProfileOverview {
        profile: String,
    },
    ListAccounts {
        profile: String,
    },
    ListPendingOperations {
        profile: String,
    },
    CreateAccount {
        profile: String,
        alias: String,
        username: String,
        device_name: String,
        email: String,
        invite: SecretString,
        passphrase: Option<SecretString>,
    },
    ResumeAccount {
        profile: String,
        alias: String,
    },
    ListDevices {
        profile: String,
        alias: String,
    },
    ListBackupEnrollments {
        profile: String,
        account_alias: String,
    },
    DescribeServerStatus {
        profile: String,
    },
    RemoveDevice {
        profile: String,
        signer_alias: String,
        device_id: String,
    },
    ProvisionOwnerDevice {
        profile: String,
        source_alias: String,
        target_alias: String,
        device_name: String,
        serial: u64,
    },
    ResumeOwnerDeviceProvision {
        profile: String,
        target_alias: String,
    },
    PrepareOwnerBackup {
        profile: String,
        account_alias: String,
        backup_alias: String,
    },
    CommitOwnerBackup {
        profile: String,
        account_alias: String,
        backup_alias: String,
        phrase: SecretString,
    },
    RevokeOwnerBackup {
        profile: String,
        account_alias: String,
        backup_alias: String,
        backup_id: String,
    },
    RecoverOwnerAccount {
        profile: String,
        target_alias: String,
        phrase: SecretString,
        device_name: String,
        serial: u64,
    },
    ResumeOwnerRecovery {
        profile: String,
        target_alias: String,
        phrase: SecretString,
        device_name: String,
    },
    SetPassphrase {
        profile: String,
        alias: String,
        passphrase: SecretString,
    },
    ChangePassphrase {
        profile: String,
        alias: String,
        passphrase: SecretString,
    },
    VerifyPassphrase {
        profile: String,
        alias: String,
        passphrase: SecretString,
    },
    SetYubiPassphrase {
        profile: String,
        alias: String,
        pin: SecretString,
        passphrase: SecretString,
    },
    ChangeYubiPassphrase {
        profile: String,
        alias: String,
        pin: SecretString,
        passphrase: SecretString,
    },
    VerifyYubiPassphrase {
        profile: String,
        alias: String,
        pin: SecretString,
        passphrase: SecretString,
    },
    SyncAccount {
        profile: String,
        alias: String,
    },
    StartDevicePairing {
        profile: String,
        account_alias: String,
    },
    RepublishDevicePairing {
        profile: String,
        account_alias: String,
    },
    FinishDevicePairing {
        profile: String,
        account_alias: String,
    },
    AcceptDevicePairing {
        profile: String,
        target_alias: String,
        device_name: String,
        serial: u64,
        phrase: SecretString,
    },
    AcceptGoProfilePairing {
        candidate_id: String,
        profile: String,
        target_alias: String,
        device_name: String,
        serial: u64,
        phrase: SecretString,
    },
    ResumeDevicePairingAcceptance {
        profile: String,
        target_alias: String,
    },
    ResumeGoProfilePairing {
        candidate_id: String,
        profile: String,
        target_alias: String,
    },
    CopyGoProfileDevice {
        candidate_id: String,
        profile: String,
        target_alias: String,
    },
    ListYubiCards {
        profile: String,
    },
    ListYubiAccounts {
        profile: String,
    },
    CreateYubiAccount {
        profile: String,
        alias: String,
        username: String,
        device_name: String,
        email: String,
        invite: SecretString,
        passphrase: Option<SecretString>,
        card_serial: u32,
        signing_slot: u8,
        pq_slot: u8,
        pin: SecretString,
        retry_configuration: Option<YubiRetryConfiguration>,
    },
    ResumeYubiAccount {
        profile: String,
        alias: String,
        pin: SecretString,
    },
    ProvisionYubiDevice {
        profile: String,
        source_alias: String,
        target_alias: String,
        device_name: String,
        serial: u64,
        card_serial: u32,
        signing_slot: u8,
        pq_slot: u8,
        pin: SecretString,
        retry_configuration: Option<YubiRetryConfiguration>,
    },
    SyncYubiAccount {
        profile: String,
        alias: String,
        pin: SecretString,
        /// Also run every federated security responder this unlocked key can
        /// drive on the named profile.
        #[serde(default)]
        with_federation: bool,
    },
    YubiPinStatus {
        profile: String,
        alias: String,
    },
    ChangeYubiPin {
        profile: String,
        alias: String,
        old_pin: SecretString,
        new_pin: SecretString,
    },
    ChangeYubiPuk {
        profile: String,
        alias: String,
        old_puk: SecretString,
        new_puk: SecretString,
    },
    UnblockYubiPin {
        profile: String,
        alias: String,
        puk: SecretString,
        new_pin: SecretString,
    },
    RotateYubiManagementKey {
        profile: String,
        alias: String,
        pin: SecretString,
    },
    ResumeYubiManagementKey {
        profile: String,
        alias: String,
        pin: Option<SecretString>,
    },
    RecoverYubiManagementKey {
        profile: String,
        yubi_alias: String,
        software_alias: String,
    },
    RecoverYubiSubkey {
        profile: String,
        alias: String,
        pin: SecretString,
    },
    RevokeYubiDevice {
        profile: String,
        yubi_alias: String,
        software_alias: String,
    },
    ListKv {
        store: AccountStoreRef,
        #[serde(default)]
        cursor: Option<String>,
        limit: u32,
    },
    ListTeamKv {
        store: TeamStoreRef,
        #[serde(default)]
        cursor: Option<String>,
        limit: u32,
    },
    ReadKv {
        store: KvStoreRef,
        path: String,
        version: u64,
    },
    ReadKvChunk {
        store: KvStoreRef,
        path: String,
        version: u64,
        offset: u64,
        length: u32,
    },
    PutKv {
        store: KvStoreRef,
        path: String,
        content: Vec<u8>,
        read_role: KvRole,
        write_role: KvRole,
        precondition: KvPrecondition,
        #[serde(default)]
        mkdir_p: bool,
    },
    PutKvStream {
        header: KvUploadHeader,
    },
    PutKvSymlink {
        store: KvStoreRef,
        path: String,
        target: String,
        read_role: KvRole,
        write_role: KvRole,
        precondition: KvPrecondition,
        #[serde(default)]
        mkdir_p: bool,
    },
    MkdirKv {
        store: KvStoreRef,
        path: String,
        read_role: KvRole,
        write_role: KvRole,
        precondition: KvPrecondition,
        #[serde(default)]
        mkdir_p: bool,
    },
    RemoveKv {
        store: KvStoreRef,
        path: String,
        recursive: bool,
        precondition: KvPrecondition,
    },
    CreateTeam {
        profile: String,
        account_alias: String,
        team_alias: String,
        name: String,
        kind: TeamKind,
    },
    ResumeTeamCreation {
        profile: String,
        team_alias: String,
    },
    ListTeams {
        profile: String,
    },
    DiscoverTeams {
        profile: String,
        account_alias: String,
    },
    SyncTeam {
        profile: String,
        team_alias: String,
    },
    ListTeamDetails {
        profile: String,
        team_alias: String,
    },
    ListTeamMembers {
        profile: String,
        team_alias: String,
    },
    AddTeamMember {
        profile: String,
        team_alias: String,
        username: String,
        role: TeamRole,
        visibility: i16,
    },
    ResumeTeamMemberAddition {
        profile: String,
        team_alias: String,
        username: String,
    },
    DemoteTeamMember {
        profile: String,
        team_alias: String,
        party_id_hex: String,
        role: TeamRole,
        visibility: i16,
    },
    RemoveTeamMember {
        profile: String,
        team_alias: String,
        party_id_hex: String,
    },
    ResumeTeamMemberEdit {
        profile: String,
        team_alias: String,
    },
    AdmitFederatedTeam {
        local_profile: String,
        local_team_alias: String,
        remote_profile: String,
        remote_team_alias: String,
        role: FederationRole,
        visibility: i16,
    },
    ListFederatedTeams {
        profile: String,
        team_alias: String,
    },
    ExpelFederatedTeam {
        profile: String,
        team_alias: String,
        remote_host_id_hex: String,
        remote_team_id_hex: String,
    },
    /// Runs the federated security responder for one local team. Either side
    /// may supply an already-enrolled Yubi alias plus its PIN; supplying both
    /// is the explicit two-key workflow that no unattended schedule performs.
    RefreshFederatedSecurity {
        profile: String,
        team_alias: String,
        #[serde(default)]
        local_yubi_alias: Option<String>,
        #[serde(default)]
        local_pin: Option<SecretString>,
        #[serde(default)]
        remote_profile: Option<String>,
        #[serde(default)]
        remote_yubi_alias: Option<String>,
        #[serde(default)]
        remote_pin: Option<SecretString>,
        /// Further hardware unlocks for the profiles a cascade reaches
        /// beyond the immediate pair.
        #[serde(default)]
        unlocks: Vec<YubiFederationUnlockInput>,
    },
    RunDueJobs {
        profile: String,
    },
}

impl Operation {
    /// Clears request-owned plaintext once a client has serialized it. Secret
    /// strings already zeroize on drop; KV byte/string fields use ordinary
    /// wire types for serde compatibility and therefore need this explicit
    /// handoff hook.
    pub fn zeroize_plaintext(&mut self) {
        match self {
            Self::PutKv { content, .. } => content.zeroize(),
            Self::PutKvSymlink { target, .. } => target.zeroize(),
            _ => {}
        }
    }

    /// Interactive KEX can contain two sequential relay polls. Frontends and
    /// the resident agent use a longer bounded deadline only for these calls.
    pub fn is_device_pairing_wait(&self) -> bool {
        matches!(
            self,
            Self::FinishDevicePairing { .. }
                | Self::AcceptDevicePairing { .. }
                | Self::AcceptGoProfilePairing { .. }
                | Self::ResumeDevicePairingAcceptance { .. }
                | Self::ResumeGoProfilePairing { .. }
                | Self::CopyGoProfileDevice { .. }
        )
    }

    pub fn is_chat_poll_wait(&self) -> bool {
        matches!(
            self,
            Self::Chat {
                action: crate::chat::ChatAction::PollInbox { .. },
                ..
            }
        )
    }

    /// Calls in the allow-list may share the bounded worker pool; every other
    /// call additionally takes the agent's global single-flight gate. Some
    /// allow-listed calls still advance authenticated local checkpoints or
    /// host pins under the per-profile lock.
    pub fn is_mutation(&self) -> bool {
        if let Self::Chat { action, .. } = self {
            return action.is_mutation();
        }
        !matches!(
            self,
            Self::Ping
                | Self::AgentStatus
                | Self::DiscoverGoProfiles
                | Self::ListProfiles
                | Self::DescribeResetHardState { .. }
                | Self::Probe { .. }
                | Self::PrepareOwnerBackup { .. }
                | Self::ListKnownStores { .. }
                | Self::ListProfileOverview { .. }
                | Self::ListAccounts { .. }
                | Self::ListPendingOperations { .. }
                | Self::ListDevices { .. }
                | Self::ListBackupEnrollments { .. }
                | Self::DescribeServerStatus { .. }
                | Self::VerifyPassphrase { .. }
                | Self::ListYubiCards { .. }
                | Self::ListYubiAccounts { .. }
                | Self::YubiPinStatus { .. }
                | Self::ListKv { .. }
                | Self::ListTeamKv { .. }
                | Self::ReadKv { .. }
                | Self::ReadKvChunk { .. }
                | Self::ListTeams { .. }
                | Self::ListTeamDetails { .. }
                | Self::ListTeamMembers { .. }
                | Self::ListFederatedTeams { .. }
        )
    }
}

impl std::fmt::Debug for Operation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Chat { store, action } => formatter
                .debug_struct("Chat")
                .field("store", store)
                .field("action", action)
                .finish(),
            Self::Ping => formatter.write_str("Ping"),
            Self::AgentStatus => formatter.write_str("AgentStatus"),
            Self::DiscoverGoProfiles => formatter.write_str("DiscoverGoProfiles"),
            Self::InitializeState { backend } => formatter
                .debug_struct("InitializeState")
                .field("backend", backend)
                .finish(),
            Self::AddProfile {
                name,
                probe,
                protocol,
                trust,
            } => formatter
                .debug_struct("AddProfile")
                .field("name", name)
                .field("probe", probe)
                .field("protocol", protocol)
                .field("trust", trust)
                .finish(),
            Self::CheckAndAddProfile {
                name,
                probe,
                protocol,
                trust,
            } => formatter
                .debug_struct("CheckAndAddProfile")
                .field("name", name)
                .field("probe", probe)
                .field("protocol", protocol)
                .field("trust", trust)
                .finish(),
            Self::CheckAndAddGoProfile {
                candidate_id,
                name,
                probe,
                protocol,
                trust,
            } => formatter
                .debug_struct("CheckAndAddGoProfile")
                .field("candidate_id", candidate_id)
                .field("name", name)
                .field("probe", probe)
                .field("protocol", protocol)
                .field("trust", trust)
                .finish(),
            Self::RemoveProfile { name } => formatter
                .debug_struct("RemoveProfile")
                .field("name", name)
                .finish(),
            Self::DescribeResetHardState { profile } => formatter
                .debug_struct("DescribeResetHardState")
                .field("profile", profile)
                .finish(),
            Self::ResetHardState { profile, token: _ } => formatter
                .debug_struct("ResetHardState")
                .field("profile", profile)
                .field("token", &"<redacted>")
                .finish(),
            Self::ListProfiles => formatter.write_str("ListProfiles"),
            Self::Probe { profile } => formatter
                .debug_struct("Probe")
                .field("profile", profile)
                .finish(),
            Self::RefreshLease { profile } => formatter
                .debug_struct("RefreshLease")
                .field("profile", profile)
                .finish(),
            Self::ListKnownStores { profile } => formatter
                .debug_struct("ListKnownStores")
                .field("profile", profile)
                .finish(),
            Self::ListProfileOverview { profile } => formatter
                .debug_struct("ListProfileOverview")
                .field("profile", profile)
                .finish(),
            Self::ListAccounts { profile } => formatter
                .debug_struct("ListAccounts")
                .field("profile", profile)
                .finish(),
            Self::ListPendingOperations { profile } => formatter
                .debug_struct("ListPendingOperations")
                .field("profile", profile)
                .finish(),
            Self::CreateAccount {
                profile,
                alias,
                username,
                device_name,
                email,
                invite: _,
                passphrase: _,
            } => formatter
                .debug_struct("CreateAccount")
                .field("profile", profile)
                .field("alias", alias)
                .field("username", username)
                .field("device_name", device_name)
                .field("email", email)
                .field("invite", &"<redacted>")
                .field("passphrase", &"<redacted>")
                .finish(),
            Self::ResumeAccount { profile, alias } => formatter
                .debug_struct("ResumeAccount")
                .field("profile", profile)
                .field("alias", alias)
                .finish(),
            Self::ListDevices { profile, alias } => formatter
                .debug_struct("ListDevices")
                .field("profile", profile)
                .field("alias", alias)
                .finish(),
            Self::ListBackupEnrollments {
                profile,
                account_alias,
            } => formatter
                .debug_struct("ListBackupEnrollments")
                .field("profile", profile)
                .field("account_alias", account_alias)
                .finish(),
            Self::DescribeServerStatus { profile } => formatter
                .debug_struct("DescribeServerStatus")
                .field("profile", profile)
                .finish(),
            Self::RemoveDevice {
                profile,
                signer_alias,
                device_id,
            } => formatter
                .debug_struct("RemoveDevice")
                .field("profile", profile)
                .field("signer_alias", signer_alias)
                .field("device_id", device_id)
                .finish(),
            Self::ProvisionOwnerDevice {
                profile,
                source_alias,
                target_alias,
                device_name,
                serial,
            } => formatter
                .debug_struct("ProvisionOwnerDevice")
                .field("profile", profile)
                .field("source_alias", source_alias)
                .field("target_alias", target_alias)
                .field("device_name", device_name)
                .field("serial", serial)
                .finish(),
            Self::ResumeOwnerDeviceProvision {
                profile,
                target_alias,
            } => formatter
                .debug_struct("ResumeOwnerDeviceProvision")
                .field("profile", profile)
                .field("target_alias", target_alias)
                .finish(),
            Self::PrepareOwnerBackup {
                profile,
                account_alias,
                backup_alias,
            } => formatter
                .debug_struct("PrepareOwnerBackup")
                .field("profile", profile)
                .field("account_alias", account_alias)
                .field("backup_alias", backup_alias)
                .finish(),
            Self::CommitOwnerBackup {
                profile,
                account_alias,
                backup_alias,
                phrase: _,
            } => formatter
                .debug_struct("CommitOwnerBackup")
                .field("profile", profile)
                .field("account_alias", account_alias)
                .field("backup_alias", backup_alias)
                .field("phrase", &"<redacted>")
                .finish(),
            Self::RevokeOwnerBackup {
                profile,
                account_alias,
                backup_alias,
                backup_id,
            } => formatter
                .debug_struct("RevokeOwnerBackup")
                .field("profile", profile)
                .field("account_alias", account_alias)
                .field("backup_alias", backup_alias)
                .field("backup_id", backup_id)
                .finish(),
            Self::RecoverOwnerAccount {
                profile,
                target_alias,
                phrase: _,
                device_name,
                serial,
            } => formatter
                .debug_struct("RecoverOwnerAccount")
                .field("profile", profile)
                .field("target_alias", target_alias)
                .field("phrase", &"<redacted>")
                .field("device_name", device_name)
                .field("serial", serial)
                .finish(),
            Self::ResumeOwnerRecovery {
                profile,
                target_alias,
                phrase: _,
                device_name,
            } => formatter
                .debug_struct("ResumeOwnerRecovery")
                .field("profile", profile)
                .field("target_alias", target_alias)
                .field("phrase", &"<redacted>")
                .field("device_name", device_name)
                .finish(),
            Self::SetPassphrase { profile, alias, .. } => formatter
                .debug_struct("SetPassphrase")
                .field("profile", profile)
                .field("alias", alias)
                .field("passphrase", &"<redacted>")
                .finish(),
            Self::ChangePassphrase { profile, alias, .. } => formatter
                .debug_struct("ChangePassphrase")
                .field("profile", profile)
                .field("alias", alias)
                .field("passphrase", &"<redacted>")
                .finish(),
            Self::VerifyPassphrase { profile, alias, .. } => formatter
                .debug_struct("VerifyPassphrase")
                .field("profile", profile)
                .field("alias", alias)
                .field("passphrase", &"<redacted>")
                .finish(),
            Self::SetYubiPassphrase {
                profile,
                alias,
                pin: _,
                passphrase: _,
            } => formatter
                .debug_struct("SetYubiPassphrase")
                .field("profile", profile)
                .field("alias", alias)
                .field("pin", &"<redacted>")
                .field("passphrase", &"<redacted>")
                .finish(),
            Self::ChangeYubiPassphrase {
                profile,
                alias,
                pin: _,
                passphrase: _,
            } => formatter
                .debug_struct("ChangeYubiPassphrase")
                .field("profile", profile)
                .field("alias", alias)
                .field("pin", &"<redacted>")
                .field("passphrase", &"<redacted>")
                .finish(),
            Self::VerifyYubiPassphrase {
                profile,
                alias,
                pin: _,
                passphrase: _,
            } => formatter
                .debug_struct("VerifyYubiPassphrase")
                .field("profile", profile)
                .field("alias", alias)
                .field("pin", &"<redacted>")
                .field("passphrase", &"<redacted>")
                .finish(),
            Self::SyncAccount { profile, alias } => formatter
                .debug_struct("SyncAccount")
                .field("profile", profile)
                .field("alias", alias)
                .finish(),
            Self::StartDevicePairing {
                profile,
                account_alias,
            } => formatter
                .debug_struct("StartDevicePairing")
                .field("profile", profile)
                .field("account_alias", account_alias)
                .finish(),
            Self::RepublishDevicePairing {
                profile,
                account_alias,
            } => formatter
                .debug_struct("RepublishDevicePairing")
                .field("profile", profile)
                .field("account_alias", account_alias)
                .finish(),
            Self::FinishDevicePairing {
                profile,
                account_alias,
            } => formatter
                .debug_struct("FinishDevicePairing")
                .field("profile", profile)
                .field("account_alias", account_alias)
                .finish(),
            Self::AcceptDevicePairing {
                profile,
                target_alias,
                device_name,
                serial,
                phrase: _,
            } => formatter
                .debug_struct("AcceptDevicePairing")
                .field("profile", profile)
                .field("target_alias", target_alias)
                .field("device_name", device_name)
                .field("serial", serial)
                .field("phrase", &"<redacted>")
                .finish(),
            Self::AcceptGoProfilePairing {
                candidate_id,
                profile,
                target_alias,
                device_name,
                serial,
                phrase: _,
            } => formatter
                .debug_struct("AcceptGoProfilePairing")
                .field("candidate_id", candidate_id)
                .field("profile", profile)
                .field("target_alias", target_alias)
                .field("device_name", device_name)
                .field("serial", serial)
                .field("phrase", &"<redacted>")
                .finish(),
            Self::ResumeDevicePairingAcceptance {
                profile,
                target_alias,
            } => formatter
                .debug_struct("ResumeDevicePairingAcceptance")
                .field("profile", profile)
                .field("target_alias", target_alias)
                .finish(),
            Self::ResumeGoProfilePairing {
                candidate_id,
                profile,
                target_alias,
            } => formatter
                .debug_struct("ResumeGoProfilePairing")
                .field("candidate_id", candidate_id)
                .field("profile", profile)
                .field("target_alias", target_alias)
                .finish(),
            Self::CopyGoProfileDevice {
                candidate_id,
                profile,
                target_alias,
            } => formatter
                .debug_struct("CopyGoProfileDevice")
                .field("candidate_id", candidate_id)
                .field("profile", profile)
                .field("target_alias", target_alias)
                .finish(),
            Self::ListYubiCards { profile } => formatter
                .debug_struct("ListYubiCards")
                .field("profile", profile)
                .finish(),
            Self::ListYubiAccounts { profile } => formatter
                .debug_struct("ListYubiAccounts")
                .field("profile", profile)
                .finish(),
            Self::CreateYubiAccount {
                profile,
                alias,
                username,
                device_name,
                email,
                invite: _,
                passphrase: _,
                card_serial,
                signing_slot,
                pq_slot,
                pin: _,
                retry_configuration,
            } => formatter
                .debug_struct("CreateYubiAccount")
                .field("profile", profile)
                .field("alias", alias)
                .field("username", username)
                .field("device_name", device_name)
                .field("email", email)
                .field("invite", &"<redacted>")
                .field("passphrase", &"<redacted>")
                .field("card_serial", card_serial)
                .field("signing_slot", signing_slot)
                .field("pq_slot", pq_slot)
                .field("pin", &"<redacted>")
                .field("retry_configuration", retry_configuration)
                .finish(),
            Self::ResumeYubiAccount {
                profile,
                alias,
                pin: _,
            } => formatter
                .debug_struct("ResumeYubiAccount")
                .field("profile", profile)
                .field("alias", alias)
                .field("pin", &"<redacted>")
                .finish(),
            Self::ProvisionYubiDevice {
                profile,
                source_alias,
                target_alias,
                device_name,
                serial,
                card_serial,
                signing_slot,
                pq_slot,
                pin: _,
                retry_configuration,
            } => formatter
                .debug_struct("ProvisionYubiDevice")
                .field("profile", profile)
                .field("source_alias", source_alias)
                .field("target_alias", target_alias)
                .field("device_name", device_name)
                .field("serial", serial)
                .field("card_serial", card_serial)
                .field("signing_slot", signing_slot)
                .field("pq_slot", pq_slot)
                .field("pin", &"<redacted>")
                .field("retry_configuration", retry_configuration)
                .finish(),
            Self::SyncYubiAccount {
                profile,
                alias,
                pin: _,
                with_federation,
            } => formatter
                .debug_struct("SyncYubiAccount")
                .field("profile", profile)
                .field("alias", alias)
                .field("pin", &"<redacted>")
                .field("with_federation", with_federation)
                .finish(),
            Self::YubiPinStatus { profile, alias } => formatter
                .debug_struct("YubiPinStatus")
                .field("profile", profile)
                .field("alias", alias)
                .finish(),
            Self::ChangeYubiPin {
                profile,
                alias,
                old_pin: _,
                new_pin: _,
            } => formatter
                .debug_struct("ChangeYubiPin")
                .field("profile", profile)
                .field("alias", alias)
                .field("old_pin", &"<redacted>")
                .field("new_pin", &"<redacted>")
                .finish(),
            Self::ChangeYubiPuk {
                profile,
                alias,
                old_puk: _,
                new_puk: _,
            } => formatter
                .debug_struct("ChangeYubiPuk")
                .field("profile", profile)
                .field("alias", alias)
                .field("old_puk", &"<redacted>")
                .field("new_puk", &"<redacted>")
                .finish(),
            Self::UnblockYubiPin {
                profile,
                alias,
                puk: _,
                new_pin: _,
            } => formatter
                .debug_struct("UnblockYubiPin")
                .field("profile", profile)
                .field("alias", alias)
                .field("puk", &"<redacted>")
                .field("new_pin", &"<redacted>")
                .finish(),
            Self::RotateYubiManagementKey {
                profile,
                alias,
                pin: _,
            } => formatter
                .debug_struct("RotateYubiManagementKey")
                .field("profile", profile)
                .field("alias", alias)
                .field("pin", &"<redacted>")
                .finish(),
            Self::ResumeYubiManagementKey {
                profile,
                alias,
                pin: _,
            } => formatter
                .debug_struct("ResumeYubiManagementKey")
                .field("profile", profile)
                .field("alias", alias)
                .field("pin", &"<redacted>")
                .finish(),
            Self::RecoverYubiManagementKey {
                profile,
                yubi_alias,
                software_alias,
            } => formatter
                .debug_struct("RecoverYubiManagementKey")
                .field("profile", profile)
                .field("yubi_alias", yubi_alias)
                .field("software_alias", software_alias)
                .finish(),
            Self::RecoverYubiSubkey {
                profile,
                alias,
                pin: _,
            } => formatter
                .debug_struct("RecoverYubiSubkey")
                .field("profile", profile)
                .field("alias", alias)
                .field("pin", &"<redacted>")
                .finish(),
            Self::RevokeYubiDevice {
                profile,
                yubi_alias,
                software_alias,
            } => formatter
                .debug_struct("RevokeYubiDevice")
                .field("profile", profile)
                .field("yubi_alias", yubi_alias)
                .field("software_alias", software_alias)
                .finish(),
            Self::ListKv {
                store,
                cursor,
                limit,
            } => formatter
                .debug_struct("ListKv")
                .field("store", store)
                .field("cursor", cursor)
                .field("limit", limit)
                .finish(),
            Self::ListTeamKv {
                store,
                cursor,
                limit,
            } => formatter
                .debug_struct("ListTeamKv")
                .field("store", store)
                .field("cursor", cursor)
                .field("limit", limit)
                .finish(),
            Self::ReadKv {
                store,
                path,
                version,
            } => formatter
                .debug_struct("ReadKv")
                .field("store", store)
                .field("path", path)
                .field("version", version)
                .finish(),
            Self::ReadKvChunk {
                store,
                path,
                version,
                offset,
                length,
            } => formatter
                .debug_struct("ReadKvChunk")
                .field("store", store)
                .field("path", path)
                .field("version", version)
                .field("offset", offset)
                .field("length", length)
                .finish(),
            Self::PutKv {
                store,
                path,
                content: _,
                read_role,
                write_role,
                precondition,
                mkdir_p,
            } => formatter
                .debug_struct("PutKv")
                .field("store", store)
                .field("path", path)
                .field("content", &"<redacted>")
                .field("read_role", read_role)
                .field("write_role", write_role)
                .field("precondition", precondition)
                .field("mkdir_p", mkdir_p)
                .finish(),
            Self::PutKvStream { header } => formatter
                .debug_struct("PutKvStream")
                .field("header", header)
                .finish(),
            Self::PutKvSymlink {
                store,
                path,
                target: _,
                read_role,
                write_role,
                precondition,
                mkdir_p,
            } => formatter
                .debug_struct("PutKvSymlink")
                .field("store", store)
                .field("path", path)
                .field("target", &"<redacted>")
                .field("read_role", read_role)
                .field("write_role", write_role)
                .field("precondition", precondition)
                .field("mkdir_p", mkdir_p)
                .finish(),
            Self::MkdirKv {
                store,
                path,
                read_role,
                write_role,
                precondition,
                mkdir_p,
            } => formatter
                .debug_struct("MkdirKv")
                .field("store", store)
                .field("path", path)
                .field("read_role", read_role)
                .field("write_role", write_role)
                .field("precondition", precondition)
                .field("mkdir_p", mkdir_p)
                .finish(),
            Self::RemoveKv {
                store,
                path,
                recursive,
                precondition,
            } => formatter
                .debug_struct("RemoveKv")
                .field("store", store)
                .field("path", path)
                .field("recursive", recursive)
                .field("precondition", precondition)
                .finish(),
            Self::CreateTeam {
                profile,
                account_alias,
                team_alias,
                name,
                kind,
            } => formatter
                .debug_struct("CreateTeam")
                .field("profile", profile)
                .field("account_alias", account_alias)
                .field("team_alias", team_alias)
                .field("name", name)
                .field("kind", kind)
                .finish(),
            Self::ResumeTeamCreation {
                profile,
                team_alias,
            } => formatter
                .debug_struct("ResumeTeamCreation")
                .field("profile", profile)
                .field("team_alias", team_alias)
                .finish(),
            Self::ListTeams { profile } => formatter
                .debug_struct("ListTeams")
                .field("profile", profile)
                .finish(),
            Self::DiscoverTeams {
                profile,
                account_alias,
            } => formatter
                .debug_struct("DiscoverTeams")
                .field("profile", profile)
                .field("account_alias", account_alias)
                .finish(),
            Self::SyncTeam {
                profile,
                team_alias,
            } => formatter
                .debug_struct("SyncTeam")
                .field("profile", profile)
                .field("team_alias", team_alias)
                .finish(),
            Self::ListTeamDetails {
                profile,
                team_alias,
            } => formatter
                .debug_struct("ListTeamDetails")
                .field("profile", profile)
                .field("team_alias", team_alias)
                .finish(),
            Self::ListTeamMembers {
                profile,
                team_alias,
            } => formatter
                .debug_struct("ListTeamMembers")
                .field("profile", profile)
                .field("team_alias", team_alias)
                .finish(),
            Self::AddTeamMember {
                profile,
                team_alias,
                username,
                role,
                visibility,
            } => formatter
                .debug_struct("AddTeamMember")
                .field("profile", profile)
                .field("team_alias", team_alias)
                .field("username", username)
                .field("role", role)
                .field("visibility", visibility)
                .finish(),
            Self::ResumeTeamMemberAddition {
                profile,
                team_alias,
                username,
            } => formatter
                .debug_struct("ResumeTeamMemberAddition")
                .field("profile", profile)
                .field("team_alias", team_alias)
                .field("username", username)
                .finish(),
            Self::DemoteTeamMember {
                profile,
                team_alias,
                party_id_hex,
                role,
                visibility,
            } => formatter
                .debug_struct("DemoteTeamMember")
                .field("profile", profile)
                .field("team_alias", team_alias)
                .field("party_id_hex", party_id_hex)
                .field("role", role)
                .field("visibility", visibility)
                .finish(),
            Self::RemoveTeamMember {
                profile,
                team_alias,
                party_id_hex,
            } => formatter
                .debug_struct("RemoveTeamMember")
                .field("profile", profile)
                .field("team_alias", team_alias)
                .field("party_id_hex", party_id_hex)
                .finish(),
            Self::ResumeTeamMemberEdit {
                profile,
                team_alias,
            } => formatter
                .debug_struct("ResumeTeamMemberEdit")
                .field("profile", profile)
                .field("team_alias", team_alias)
                .finish(),
            Self::AdmitFederatedTeam {
                local_profile,
                local_team_alias,
                remote_profile,
                remote_team_alias,
                role,
                visibility,
            } => formatter
                .debug_struct("AdmitFederatedTeam")
                .field("local_profile", local_profile)
                .field("local_team_alias", local_team_alias)
                .field("remote_profile", remote_profile)
                .field("remote_team_alias", remote_team_alias)
                .field("role", role)
                .field("visibility", visibility)
                .finish(),
            Self::ListFederatedTeams {
                profile,
                team_alias,
            } => formatter
                .debug_struct("ListFederatedTeams")
                .field("profile", profile)
                .field("team_alias", team_alias)
                .finish(),
            Self::ExpelFederatedTeam {
                profile,
                team_alias,
                remote_host_id_hex,
                remote_team_id_hex,
            } => formatter
                .debug_struct("ExpelFederatedTeam")
                .field("profile", profile)
                .field("team_alias", team_alias)
                .field("remote_host_id_hex", remote_host_id_hex)
                .field("remote_team_id_hex", remote_team_id_hex)
                .finish(),
            Self::RefreshFederatedSecurity {
                profile,
                team_alias,
                local_yubi_alias,
                local_pin: _,
                remote_profile,
                remote_yubi_alias,
                remote_pin: _,
                unlocks,
            } => formatter
                .debug_struct("RefreshFederatedSecurity")
                .field("profile", profile)
                .field("team_alias", team_alias)
                .field("local_yubi_alias", local_yubi_alias)
                .field("local_pin", &"<redacted>")
                .field("remote_profile", remote_profile)
                .field("remote_yubi_alias", remote_yubi_alias)
                .field("remote_pin", &"<redacted>")
                .field("unlocks", unlocks)
                .finish(),
            Self::RunDueJobs { profile } => formatter
                .debug_struct("RunDueJobs")
                .field("profile", profile)
                .finish(),
        }
    }
}

#[derive(Clone, Deserialize, PartialEq, Serialize)]
pub struct Response {
    pub version: u32,
    pub id: Option<u64>,
    #[serde(flatten)]
    pub result: ResponseResult,
}

impl std::fmt::Debug for Response {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Response")
            .field("version", &self.version)
            .field("id", &self.id)
            .field("result", &self.result)
            .finish()
    }
}

impl Response {
    pub fn success(id: u64, value: serde_json::Value) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id: Some(id),
            result: ResponseResult::Success { value },
        }
    }

    pub fn error(id: u64, code: ErrorCode, message: impl Into<String>) -> Self {
        Self::error_with_fields(Some(id), code, message, ErrorFields::default())
    }

    pub fn error_without_id(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::error_with_fields(None, code, message, ErrorFields::default())
    }

    pub fn error_with_fields(
        id: Option<u64>,
        code: ErrorCode,
        message: impl Into<String>,
        fields: ErrorFields,
    ) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id,
            result: ResponseResult::Error {
                code,
                message: bounded_field(message.into()),
                fields: fields.bounded(),
            },
        }
    }
}

#[derive(Clone, Deserialize, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum ResponseResult {
    Success {
        value: serde_json::Value,
    },
    Error {
        code: ErrorCode,
        message: String,
        #[serde(default, skip_serializing_if = "ErrorFields::is_empty")]
        fields: ErrorFields,
    },
}

impl std::fmt::Debug for ResponseResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Success { .. } => formatter
                .debug_struct("Success")
                .field("value", &"<redacted>")
                .finish(),
            Self::Error {
                code,
                message,
                fields,
            } => formatter
                .debug_struct("Error")
                .field("code", code)
                .field("message", message)
                .field("fields", fields)
                .finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorCode {
    ChatInvalidInput,
    ChatUnsupported,
    ChatAccessDenied,
    ChatRefreshRequired,
    ChatReprepareRequired,
    ChatNotFound,
    ChatKeyUnavailable,
    ChatLimit,
    ChatOperationState,
    ChatNameConflict,
    ChatRandomness,
    ChatIntegrity,

    InvalidRequest,
    VersionMismatch,
    BootstrapRequired,
    Conflict,
    Busy,
    DeadlineExceeded,
    CapabilityDenied,
    RollbackDetected,
    CheckpointResetRequired,
    ProfileBusy,
    RateLimited,
    QuotaExceeded,
    OperationFailed,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ErrorFields {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl ErrorFields {
    pub fn is_empty(&self) -> bool {
        self.capability.is_none()
            && self.profile.is_none()
            && self.state_dir.is_none()
            && self.reason.is_none()
    }

    fn bounded(mut self) -> Self {
        self.capability = self.capability.map(bounded_field);
        self.profile = self.profile.map(bounded_field);
        self.state_dir = self.state_dir.map(bounded_field);
        self.reason = self.reason.map(bounded_field);
        self
    }
}

fn bounded_field(mut value: String) -> String {
    const MAXIMUM_FIELD_BYTES: usize = 4096;
    if value.len() <= MAXIMUM_FIELD_BYTES {
        return value;
    }
    let mut end = MAXIMUM_FIELD_BYTES;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_teams_operation_has_stable_wire_and_debug_shapes() {
        let operation = Operation::DiscoverTeams {
            profile: "work".to_owned(),
            account_alias: "personal".to_owned(),
        };
        assert_eq!(
            serde_json::to_value(&operation).unwrap(),
            serde_json::json!({
                "operation": "discover-teams",
                "profile": "work",
                "account_alias": "personal"
            })
        );
        assert_eq!(
            serde_json::from_value::<Operation>(serde_json::to_value(&operation).unwrap()).unwrap(),
            operation
        );
        assert_eq!(
            format!("{operation:?}"),
            "DiscoverTeams { profile: \"work\", account_alias: \"personal\" }"
        );
        assert!(operation.is_mutation());
    }

    #[test]
    fn first_run_publication_and_backup_operations_have_stable_safe_shapes() {
        let publication = Operation::CheckAndAddProfile {
            name: "work".to_owned(),
            probe: "foks.example.test".to_owned(),
            protocol: ProfileProtocol::V019,
            trust: ProfileTrust::WebPki,
        };
        assert_eq!(
            serde_json::to_value(&publication).unwrap(),
            serde_json::json!({
                "operation": "check-and-add-profile",
                "name": "work",
                "probe": "foks.example.test",
                "protocol": { "generation": "v019" },
                "trust": { "kind": "web-pki" }
            })
        );
        assert!(publication.is_mutation());

        let prepare = Operation::PrepareOwnerBackup {
            profile: "work".to_owned(),
            account_alias: "personal".to_owned(),
            backup_alias: "paper".to_owned(),
        };
        assert_eq!(
            serde_json::to_value(&prepare).unwrap(),
            serde_json::json!({
                "operation": "prepare-owner-backup",
                "profile": "work",
                "account_alias": "personal",
                "backup_alias": "paper"
            })
        );
        assert!(!prepare.is_mutation());

        let commit = Operation::CommitOwnerBackup {
            profile: "work".to_owned(),
            account_alias: "personal".to_owned(),
            backup_alias: "paper".to_owned(),
            phrase: SecretString::new("never print this phrase"),
        };
        let encoded = serde_json::to_value(&commit).unwrap();
        assert_eq!(
            serde_json::from_value::<Operation>(encoded).unwrap(),
            commit
        );
        assert!(commit.is_mutation());
        assert!(!format!("{commit:?}").contains("never print this phrase"));

        let revoke = Operation::RevokeOwnerBackup {
            profile: "work".to_owned(),
            account_alias: "personal".to_owned(),
            backup_alias: "paper".to_owned(),
            backup_id: format!("10{}", "44".repeat(32)),
        };
        assert_eq!(
            serde_json::to_value(&revoke).unwrap(),
            serde_json::json!({
                "operation": "revoke-owner-backup",
                "profile": "work",
                "account_alias": "personal",
                "backup_alias": "paper",
                "backup_id": format!("10{}", "44".repeat(32))
            })
        );
        assert!(revoke.is_mutation());

        let response = Response::success(
            17,
            serde_json::json!({ "phrase": "never print this response phrase" }),
        );
        let debug = format!("{response:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("never print this response phrase"));
    }

    #[test]
    fn team_details_operation_and_nested_results_are_explicit() {
        let operation = Operation::ListTeamDetails {
            profile: "work".to_owned(),
            team_alias: "engineering".to_owned(),
        };
        assert_eq!(
            serde_json::to_value(&operation).unwrap(),
            serde_json::json!({
                "operation": "list-team-details",
                "profile": "work",
                "team_alias": "engineering"
            })
        );
        assert_eq!(
            format!("{operation:?}"),
            "ListTeamDetails { profile: \"work\", team_alias: \"engineering\" }"
        );
        let details = TeamDetailsSummary {
            members: ResponseResult::Error {
                code: ErrorCode::RateLimited,
                message: "wait".to_owned(),
                fields: ErrorFields::default(),
            },
            federation: ResponseResult::Success {
                value: serde_json::json!([]),
            },
        };
        assert_eq!(
            serde_json::from_value::<TeamDetailsSummary>(serde_json::to_value(&details).unwrap())
                .unwrap(),
            details
        );
    }

    #[test]
    fn reset_and_settings_operations_have_stable_safe_shapes() {
        let describe = Operation::DescribeResetHardState {
            profile: "work".to_owned(),
        };
        assert_eq!(
            serde_json::to_value(&describe).unwrap(),
            serde_json::json!({
                "operation": "describe-reset-hard-state",
                "profile": "work"
            })
        );
        assert!(!describe.is_mutation());

        let reset = Operation::ResetHardState {
            profile: "work".to_owned(),
            token: SecretString::new("one-use-secret-token"),
        };
        let reset_value = serde_json::to_value(&reset).unwrap();
        assert_eq!(
            reset_value,
            serde_json::json!({
                "operation": "reset-hard-state",
                "profile": "work",
                "token": "one-use-secret-token"
            })
        );
        assert_eq!(
            serde_json::from_value::<Operation>(reset_value).unwrap(),
            reset
        );
        assert!(reset.is_mutation());
        assert!(!format!("{reset:?}").contains("one-use-secret-token"));

        let preview = ResetStatePreview {
            profile: "work".to_owned(),
            resumables: vec![PendingOperationSummary {
                kind: PendingOperationKind::AccountRecovery,
                alias: "personal".to_owned(),
                target: None,
            }],
            artifacts: vec![ResetArtifactSummary {
                kind: ResetArtifactKind::CredentialsAndResumables,
                entries: 2,
                bytes: 144,
            }],
            token: SecretString::new("opaque"),
            expires_in_seconds: 60,
        };
        assert!(!format!("{preview:?}").contains("opaque"));
        assert_eq!(
            serde_json::to_value(preview).unwrap(),
            serde_json::json!({
                "profile": "work",
                "resumables": [{
                    "kind": "account-recovery",
                    "alias": "personal",
                    "target": null
                }],
                "artifacts": [{
                    "kind": "credentials-and-resumables",
                    "entries": 2,
                    "bytes": 144
                }],
                "token": "opaque",
                "expires_in_seconds": 60
            })
        );

        let backups = Operation::ListBackupEnrollments {
            profile: "work".to_owned(),
            account_alias: "personal".to_owned(),
        };
        assert!(!backups.is_mutation());
        assert_eq!(
            serde_json::to_value(backups).unwrap(),
            serde_json::json!({
                "operation": "list-backup-enrollments",
                "profile": "work",
                "account_alias": "personal"
            })
        );

        let status = Operation::DescribeServerStatus {
            profile: "work".to_owned(),
        };
        assert!(!status.is_mutation());
        assert_eq!(
            serde_json::to_value(status).unwrap(),
            serde_json::json!({
                "operation": "describe-server-status",
                "profile": "work"
            })
        );

        assert_eq!(
            serde_json::to_value(ServerStatusSnapshot {
                profile: "work".to_owned(),
                configured_probe: "foks.example.test:443".to_owned(),
                host: Some(StoredHostStatus {
                    lookup_name: "foks.example.test".to_owned(),
                    canonical_name: "foks.example.test".to_owned(),
                    host_id_hex: "abcd".to_owned(),
                    host_chain_sequence: 9,
                    merkle_epoch: 4,
                }),
                lease_required: true,
                lease_expires_at: Some(1_800_000_000),
            })
            .unwrap(),
            serde_json::json!({
                "profile": "work",
                "configured_probe": "foks.example.test:443",
                "host": {
                    "lookup_name": "foks.example.test",
                    "canonical_name": "foks.example.test",
                    "host_id_hex": "abcd",
                    "host_chain_sequence": 9,
                    "merkle_epoch": 4
                },
                "lease_required": true,
                "lease_expires_at": 1_800_000_000_u64
            })
        );
        assert_eq!(
            serde_json::to_value(BackupEnrollmentSummary {
                backup_alias: "paper".to_owned(),
                account_alias: "personal".to_owned(),
                backup_id_hex: "1234".to_owned(),
            })
            .unwrap(),
            serde_json::json!({
                "backup_alias": "paper",
                "account_alias": "personal",
                "backup_id_hex": "1234"
            })
        );
    }
}
