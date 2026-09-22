//! Product-level orchestration shared by standalone FOKS frontends.
//!
//! This crate owns profiles, capability policy, credential serialization, and
//! one-shot synchronization. It deliberately owns neither a UI nor a resident
//! runtime, and it has no dependency on any AKA crate.

#![forbid(unsafe_code)]

mod chat_intent;
mod pending_chat;
pub use chat_intent::{LegacyChatIntent, LocalChatIntent, LocalChatIntentStore};
pub use pending_chat::{PendingChatBinding, PendingChatStore};
mod adapter_maintenance;
mod merkle_maintenance;
pub use adapter_maintenance::{AdapterMaintenanceCursor, AdapterMaintenanceReport};
mod auth_cache;
pub use auth_cache::{
    AuthCacheKey, AuthenticatedUserCache, KvNodeMemo, KvNodeMemoEntry, KvNodeMemoKey, ReadCaches,
    TeamViewCacheKey, TeamViewTokenCache,
};
mod adapter_clock;
pub use adapter_clock::{
    AdapterClock, AdapterClockPreview, AdapterClockRepair, SystemAdapterClock,
};
pub use foks_proto::SubmissionHandle;
mod federation;
pub mod portability;
mod runtime;
pub use portability::{ClientStateLease, ClientStateMaintenanceGuard};
mod yubi;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub use foks_client::CancellationToken;
use foks_client::{
    AdHocTeamSecrets, AuthenticatedTeamOutcome, AuthenticatedUserOutcome, DeviceCredential,
    EncryptedFileMutationStore, FoksClient, KvWriteOptions, MutationCoordinator, NamedTeamSecrets,
    NewSoftwareDeviceSecrets, ProbeTarget, SoftwareAccountRequest, SoftwareAccountSecrets,
    SoftwareDeviceProvisionRequest,
};
use foks_client_db::ScheduledJobKind;
use foks_client_db::{HardStateStore, KvDirectoryProjection, MutationKind, MutationState};
use foks_compat_artifact::{Outcome as CanaryOutcome, SignedCanaryArtifact};
use foks_crypto::{derive_device_public, derive_shared_verify_key, prefixed_hash, BackupKey};
pub use foks_crypto::{BackupPhrase, Passphrase};
use foks_keystore::SecretStore;
use foks_proto::{
    EntityId, InviteCode, KvNodeId, KvNodeType, Role, SecretSeed, ENTITY_PUK_VERIFY, ENTITY_USER,
};
use rustls::pki_types::CertificateDer;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::{Zeroize as _, Zeroizing};

const CONFIG_VERSION: u32 = 1;
const CREDENTIAL_VERSION: u32 = 1;
const VAULT_KEY_TYPE_ID: u64 = 0x43cc_5eca_5249_22a1;
const MUTATION_KEY_TYPE_ID: u64 = 0x5e4b_52ca_d668_dd1d;
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_CERTIFICATES: usize = 8;
const MAX_CERTIFICATE_BYTES: usize = 1024 * 1024;
const STATE_CONFIG_VERSION: u32 = 3;
const STATE_CONFIG_FILE: &str = "client-state.toml";
const MASTER_KEY_RECORD: &str = "master-key-v1";
const STATE_ROOT_RECORD: &str = "state-root-v1";
const STATE_ROOT_BINDING_TYPE_ID: u64 = 0xf8d8_c42e_96e0_4734;
const REGISTRY_LOCK_FILE: &str = ".profiles.lock";
pub use foks_protocol_metadata::PINNED_PROTOCOL_METADATA_SHA256;

#[derive(Debug, Error)]
pub enum Error {
    #[error("saved host identity is missing; explicit server verification is required")]
    SavedTrustMissing,
    #[error("imported profile requires online verification before ordinary use; run state verify-online")]
    ImportVerificationRequired,
    #[error("client state is busy; stop its agent and close active operations before maintenance")]
    StateBusy,
    #[error("client state path identity changed; reopen the verified state root")]
    StatePathChanged,
    #[error("client state maintenance is incomplete; run state recovery before opening or initializing it")]
    StateRecoveryRequired,
    #[error("state portability requires native credentials on Linux or macOS")]
    PortabilityUnsupported,
    #[error(transparent)]
    WebAdmin(#[from] foks_client::WebAdminError),
    #[error("bot token is locked; load the original token into this agent session")]
    BotTokenLocked,
    #[error("invalid bot token")]
    BotToken,
    #[error("invalid FOKS profile: {0}")]
    InvalidProfile(&'static str),
    #[error("FOKS profile already exists")]
    ProfileExists,
    #[error("FOKS profile is missing")]
    ProfileMissing,
    #[error("FOKS profile registry changed concurrently; reload and retry")]
    ProfileRegistryChanged,
    #[error("FOKS reset preview no longer matches this profile's local state")]
    ResetPreviewChanged,
    #[error("FOKS profile does not permit {0:?}")]
    CapabilityDenied(Capability),
    #[error("FOKS account already exists")]
    AccountExists,
    #[error("FOKS account is missing")]
    AccountMissing,
    #[error("FOKS account record is invalid: {0}")]
    InvalidAccount(&'static str),
    #[error("FOKS KV path is invalid: {0}")]
    InvalidKvPath(&'static str),
    #[error("FOKS KV item changed since it was read")]
    KvConflict,
    /// The passphrase offered as the account's current one did not match, so
    /// the rotation it was guarding was never submitted.
    #[error("FOKS current passphrase is not correct")]
    CurrentPassphraseRejected,
    #[error("FOKS application configuration is invalid: {0}")]
    InvalidConfig(&'static str),
    #[error("FOKS client failed: {0}")]
    Client(#[from] foks_client::Error),
    #[error("FOKS protected mutation store failed: {0}")]
    ProtectedStore(#[from] foks_client::ProtectedStoreError),
    #[error("FOKS keystore failed: {0}")]
    Keystore(#[from] foks_keystore::Error),
    #[error("FOKS protocol value failed: {0}")]
    Protocol(#[from] foks_proto::Error),
    #[error("FOKS cryptography failed: {0}")]
    Crypto(#[from] foks_crypto::Error),
    #[error("FOKS hardware key operation failed: {0}")]
    Yubi(#[from] foks_yubi::Error),
    #[error("FOKS backup phrase failed: {0}")]
    Backup(#[from] foks_crypto::BackupPhraseError),
    #[error("FOKS KEX phrase failed: {0}")]
    KexPhrase(#[from] foks_crypto::KexPhraseError),
    #[error("FOKS application I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("FOKS client state failed: {0}")]
    ClientDatabase(#[from] foks_client_db::Error),
    #[error("FOKS background security refresh failed: {0}")]
    BackgroundRefresh(String),
    #[error("FOKS security refresh is deferred until a hardware key is unlocked: {0}")]
    YubiUnlockRequired(String),
    #[error("FOKS application TOML failed: {0}")]
    TomlDecode(#[from] toml::de::Error),
    #[error("FOKS application TOML encoding failed: {0}")]
    TomlEncode(#[from] toml::ser::Error),
    #[error("FOKS credential encoding failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("OS randomness is unavailable")]
    Randomness,
    #[error("FOKS trust root is invalid")]
    TrustRoot,
    #[error("FOKS verification failed: {0}")]
    Verify(#[from] foks_verify::Error),
    #[error("FOKS hard-state rollback or fork detected: {0}")]
    RollbackDetected(&'static str),
    #[error(
        "FOKS external rollback checkpoint rejected profile '{profile}': {reason}. To discard the checkpoint and reset local hard state, run: foks-rs --state-dir {state_dir:?} profile reset-hard-state {profile} --confirm-delete"
    )]
    CheckpointResetRequired {
        profile: String,
        state_dir: PathBuf,
        reason: &'static str,
    },
}

fn entity_id_from_hex(value: &str) -> Result<EntityId> {
    if !value.len().is_multiple_of(2) || value.len() > 128 {
        return Err(Error::InvalidAccount("entity ID is not valid hexadecimal"));
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().chunks_exact(2) {
        let digit = |byte| match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        };
        let high =
            digit(pair[0]).ok_or(Error::InvalidAccount("entity ID is not valid hexadecimal"))?;
        let low =
            digit(pair[1]).ok_or(Error::InvalidAccount("entity ID is not valid hexadecimal"))?;
        bytes.push((high << 4) | low);
    }
    EntityId::from_bytes(bytes).map_err(Into::into)
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PendingOperationSummary {
    pub kind: PendingOperationKind,
    pub alias: String,
    pub target: Option<String>,
}

pub use federation::{
    AddFederatedTeamMemberReport, FederatedMembershipSummary, FederationDestinationRole,
    FederationExpulsionReport, FederationRefreshReport, UnlockedYubiActor,
};
pub use runtime::{JobRun, JobRunReport};
pub use yubi::{
    LoadedYubiAccount, YubiAccountReport, YubiAccountSummary, YubiCardSummary, YubiEnrollmentState,
    YubiFederationSyncReport, YubiLifecycleReport, YubiPinStatus, YubiProvisionInput,
    YubiRevocationReport, YubiSignupInput, YubiSubkeyRecoveryReport,
};

mod bot_token;
#[cfg(test)]
mod test_support;
mod web_admin;
pub use bot_token::{
    BotEnrollmentAction, BotEnrollmentOutcome, BotEnrollmentReport, BotRevocationReport,
    BotSelection,
};
pub use web_admin::AdminHandoff;
mod account;
mod account_conveniences;
mod local_alias;
pub use local_alias::validate_local_alias;
mod invitations;
pub use account_conveniences::{RenameAction, RenameReport};
pub use invitations::InvitationAction;
mod sso;
pub use sso::{SsoAction, SsoReport, SsoSignupInput};
mod chat;
pub use chat::{chat_submission, ChatChannelInput, ChatMessageInput};
mod checkpoint;
mod kv;
mod registry;
mod team;

pub use account::{
    derive_mutation_key, derive_vault_key, AccountVault, BackupEnrollmentReport,
    BackupEnrollmentSummary, BackupRevocationReport, DeviceProvisionReport, DeviceRevocationReport,
    DeviceSummary, KexAcceptanceInput, KexOfferReport, LoadedAccount, PassphraseReport,
    PassphraseStatus, SyncReport,
};
use checkpoint::RollbackHostCheckpoint;
#[cfg(test)]
use checkpoint::{
    database_claim_record_key, hard_state_artifact_paths, rollback_record_key,
    CheckpointReconciliation,
};
pub use checkpoint::{
    ClientCredentials, CredentialBackend, ResetArtifactKind, ResetArtifactSummary,
    ResetStatePreview, RollbackCheckpoint, SharedSessionOutcome,
};
#[cfg(test)]
use kv::{display_component, split_parent};
pub use kv::{
    DataCatalogEntryReport, DataCatalogReport, DataMemberReport, DataMembershipReport,
    DataMembershipsReport, DataStatReport, DataWriteKind, DataWriteOutcome, DataWriteSpec,
    DataWriteStatus, KvCatalogEntry, KvCatalogReport, KvChunkReport, KvEntrySummary, KvListReport,
    KvMutationPrecondition, KvReadReport, KvRoleSummary, KvWriteReport,
};
pub use registry::{
    normalize_profile_label, Capability, CapabilityDenial, CheckedProfileSession,
    CompatibilityFailure, CompatibilityStatus, HostedLeaseRenewal, ProbeAcceptance, ProbeReport,
    Profile, ProfilePaths, ProfilePublicationReport, ProfileRegistry, ProfileSession,
    ProtocolPolicy, ServerStatusSnapshot, ServerVersionReport, StoredHostStatus, TrustRoot,
    PROFILE_LABEL_MAX_BYTES,
};
#[cfg(test)]
use team::StoredTeam;
pub use team::{
    TeamDiscoveryReport, TeamMemberMutationReport, TeamMemberRole, TeamMemberSummary, TeamSummary,
    TeamSyncReport,
};
fn account_key(alias: &str) -> String {
    format!("account.{alias}")
}

fn pending_key(alias: &str) -> String {
    format!("pending.{alias}")
}

fn pending_device_key(alias: &str) -> String {
    format!("pending-device.{alias}")
}

fn kex_offer_key(alias: &str) -> String {
    format!("kex-offer.{alias}")
}

fn pending_kex_key(alias: &str) -> String {
    format!("pending-kex.{alias}")
}

fn pending_recovery_key(alias: &str) -> String {
    format!("pending-recovery.{alias}")
}

fn backup_key(alias: &str) -> String {
    format!("backup.{alias}")
}

fn team_key(alias: &str) -> String {
    format!("team.{alias}")
}

fn team_rekey_key(alias: &str) -> String {
    format!("team-rekey.{alias}")
}

fn team_member_edit_key(alias: &str) -> String {
    format!("team-member-edit.{alias}")
}

fn federation_expulsion_key(alias: &str) -> String {
    format!("federation-expulsion.{alias}")
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(Error::InvalidProfile("name is invalid"));
    }
    Ok(())
}
fn now_microseconds() -> Result<u64> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::InvalidConfig("system clock precedes Unix epoch"))?;
    u64::try_from(duration.as_micros())
        .map_err(|_| Error::InvalidConfig("system timestamp overflow"))
}

fn random_array<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|_| Error::Randomness)?;
    Ok(bytes)
}

const TRANSPORT_FINGERPRINT_TYPE_ID: u64 = 0x2b9f_4d17_a60c_7e35;

/// Builds a client for a profile's trust configuration together with a digest
/// of that configuration. An embedder that retains one base client per profile
/// compares the digest before reusing it, so a changed or replaced certificate
/// cannot be served from an already-built transport.
pub fn base_client_for_profile(
    registry: &ProfileRegistry,
    name: &str,
) -> Result<(FoksClient, [u8; 32])> {
    let profile = registry.profile(name)?;
    let fingerprint = trust_fingerprint(&profile.trust, registry.root())?;
    Ok((
        client_for_trust(&profile.trust, registry.root())?,
        fingerprint,
    ))
}

/// The digest [`base_client_for_profile`] returns, without building a client.
pub fn profile_transport_fingerprint(registry: &ProfileRegistry, name: &str) -> Result<[u8; 32]> {
    let profile = registry.profile(name)?;
    trust_fingerprint(&profile.trust, registry.root())
}

fn trust_fingerprint(trust: &TrustRoot, root: &Path) -> Result<[u8; 32]> {
    let mut input = Vec::new();
    match portability::trust::read_certificate(root, trust)? {
        Some(certificate) => {
            input.push(1);
            input.extend_from_slice(&certificate);
        }
        None => input.push(0),
    }
    Ok(prefixed_hash(TRANSPORT_FINGERPRINT_TYPE_ID, &input))
}

fn client_for_trust(trust: &TrustRoot, root: &Path) -> Result<FoksClient> {
    let Some(certificate) = portability::trust::read_certificate(root, trust)? else {
        return Ok(FoksClient::webpki());
    };
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from(certificate))
        .map_err(|_| Error::TrustRoot)?;
    Ok(FoksClient::with_roots(roots))
}

fn prepare_private_directory(path: &Path) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
        if !path.exists() {
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true).mode(0o700);
            builder.create(path)?;
        }
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(Error::InvalidConfig(
                "state directory is not a real directory",
            ));
        }
        if metadata.permissions().mode() & 0o077 != 0 {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(path)?;
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(Error::InvalidConfig(
                "state directory is not a real directory",
            ));
        }
    }
    path.canonicalize().map_err(Error::from)
}

fn state_root_binding(path: &Path) -> [u8; 32] {
    #[cfg(unix)]
    let bytes = {
        use std::os::unix::ffi::OsStrExt as _;
        path.as_os_str().as_bytes().to_vec()
    };
    #[cfg(windows)]
    let bytes = {
        use std::os::windows::ffi::OsStrExt as _;
        path.as_os_str()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>()
    };
    #[cfg(not(any(unix, windows)))]
    let bytes = path.to_string_lossy().as_bytes().to_vec();
    prefixed_hash(STATE_ROOT_BINDING_TYPE_ID, &bytes)
}

fn atomic_private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or(Error::InvalidConfig("path has no parent"))?;
    let mut suffix = [0u8; 16];
    getrandom::fill(&mut suffix).map_err(|_| Error::Randomness)?;
    let temporary = parent.join(format!(".profiles-{}.tmp", hex(&suffix)));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = match options.open(&temporary) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(Error::InvalidConfig("stale profile registry transaction"));
        }
        Err(error) => return Err(error.into()),
    };
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn create_private_config(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or(Error::InvalidConfig("path has no parent"))?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            Error::InvalidConfig("client state is already initialized")
        } else {
            Error::Io(error)
        }
    })?;
    if let Err(error) = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        File::open(parent)?.sync_all()?;
        Ok::<_, std::io::Error>(())
    })() {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error.into());
    }
    Ok(())
}

fn read_private_file_optional(path: &Path, maximum: u64) -> Result<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > maximum {
        return Err(Error::InvalidConfig("profile registry path is unsafe"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(Error::InvalidConfig(
                "profile registry permissions are unsafe",
            ));
        }
    }
    Ok(Some(read_bounded_regular_file(path, maximum)?))
}

fn read_bounded_regular_file(path: &Path, maximum: u64) -> Result<Vec<u8>> {
    use std::io::Read as _;

    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > maximum {
        return Err(Error::InvalidConfig(
            "file path is invalid, symlinked, or exceeds maximum allowed size",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    let opened = file.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.nlink() != 1
            || opened.nlink() != 1
            || metadata.dev() != opened.dev()
            || metadata.ino() != opened.ino()
        {
            return Err(Error::InvalidConfig(
                "file identity or hard-link count changed",
            ));
        }
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    (&file)
        .take(maximum.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum
        || bytes.len() as u64 != metadata.len()
        || portability::files::changed(&metadata, &file.metadata()?)
        || portability::files::changed(&metadata, &fs::symlink_metadata(path)?)
    {
        return Err(Error::InvalidConfig("file changed during bounded read"));
    }

    Ok(bytes)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut encoded, byte| {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
        encoded
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::PendingSignup;
    use foks_keystore::MemorySecretStore;
    use std::collections::BTreeSet;

    const CANARY_KEY: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

    fn probe_only() -> ProtocolPolicy {
        ProtocolPolicy::CurrentProbeOnly {
            canary_public_key: CANARY_KEY.to_owned(),
            lease_url: "https://updates.example.test/foks/canary.json".to_owned(),
            last_artifact: None,
        }
    }

    fn profile(name: &str, protocol: ProtocolPolicy) -> Profile {
        Profile {
            name: name.to_owned(),
            label: None,
            probe: "foks.app".to_owned(),
            protocol,
            trust: TrustRoot::WebPki,
        }
    }

    #[test]
    fn profile_registry_round_trips_only_under_explicit_root() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let mut registry = ProfileRegistry::open(&root).unwrap();
        registry.add(profile("hosted", probe_only())).unwrap();
        drop(registry);
        let registry = ProfileRegistry::open(&root).unwrap();
        assert_eq!(registry.profile("hosted").unwrap().probe, "foks.app");
        let paths = registry.prepare_profile_directory("hosted").unwrap();
        assert!(paths.directory.starts_with(root.canonicalize().unwrap()));
    }

    #[test]
    fn registry_mutations_merge_across_processes() {
        const ROOT_ENV: &str = "FOKS_REGISTRY_TEST_ROOT";
        const NAME_ENV: &str = "FOKS_REGISTRY_TEST_NAME";
        const START_ENV: &str = "FOKS_REGISTRY_TEST_START";

        if let (Ok(root), Ok(name), Ok(start)) = (
            std::env::var(ROOT_ENV),
            std::env::var(NAME_ENV),
            std::env::var(START_ENV),
        ) {
            let mut registry = ProfileRegistry::open(root).unwrap();
            std::fs::write(format!("{start}.{name}.ready"), b"ready").unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !Path::new(&start).exists() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "parent did not release registry child"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            registry.add(profile(&name, ProtocolPolicy::V019)).unwrap();
            return;
        }

        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        ProfileRegistry::open(&root).unwrap();
        let start = temporary.path().join("start");
        let executable = std::env::current_exe().unwrap();
        let spawn = |name: &str| {
            std::process::Command::new(&executable)
                .args([
                    "--exact",
                    "tests::registry_mutations_merge_across_processes",
                    "--nocapture",
                ])
                .env(ROOT_ENV, &root)
                .env(NAME_ENV, name)
                .env(START_ENV, &start)
                .spawn()
                .unwrap()
        };
        let mut first = spawn("child_a");
        let mut second = spawn("child_b");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        for name in ["child_a", "child_b"] {
            let ready = PathBuf::from(format!("{}.{name}.ready", start.display()));
            while !ready.exists() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "registry child did not become ready"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        std::fs::write(&start, b"start").unwrap();
        assert!(first.wait().unwrap().success());
        assert!(second.wait().unwrap().success());

        let registry = ProfileRegistry::open(&root).unwrap();
        let names = registry
            .profiles()
            .map(|profile| profile.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["child_a", "child_b"]);
    }

    #[test]
    fn stale_profile_replacement_is_rejected_instead_of_losing_an_update() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let mut initial = ProfileRegistry::open(&root).unwrap();
        initial.add(profile("local", ProtocolPolicy::V019)).unwrap();
        let mut first = ProfileRegistry::open(&root).unwrap();
        let mut stale = ProfileRegistry::open(&root).unwrap();

        let mut updated = first.profile("local").unwrap().clone();
        updated.probe = "first.example".to_owned();
        first.replace(updated).unwrap();

        let mut conflicting = stale.profile("local").unwrap().clone();
        conflicting.probe = "stale.example".to_owned();
        assert!(matches!(
            stale.replace(conflicting),
            Err(Error::ProfileRegistryChanged)
        ));
        assert_eq!(
            ProfileRegistry::open(&root)
                .unwrap()
                .profile("local")
                .unwrap()
                .probe,
            "first.example"
        );
    }

    #[test]
    fn explicit_private_file_credentials_round_trip_without_native_services() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        assert!(!ClientCredentials::is_initialized(&root).unwrap());
        let initialized =
            ClientCredentials::initialize(&root, CredentialBackend::PrivateFile).unwrap();
        assert!(ClientCredentials::is_initialized(&root).unwrap());
        assert!(fs::read_to_string(root.join(STATE_CONFIG_FILE))
            .unwrap()
            .contains("version = 3"));
        let expected = initialized.master_key().unwrap();
        drop(initialized);
        let reopened = ClientCredentials::open(&root).unwrap();
        assert_eq!(reopened.backend(), CredentialBackend::PrivateFile);
        assert_eq!(&*reopened.master_key().unwrap(), &*expected);
        assert!(matches!(
            ClientCredentials::initialize(&root, CredentialBackend::PrivateFile),
            Err(Error::InvalidConfig("client state is already initialized"))
        ));
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    #[ignore = "requires an unlocked macOS Keychain or Linux Secret Service session"]
    fn native_initialization_uses_one_physical_manifest_record() {
        let temporary = tempfile::tempdir().unwrap();
        let credentials =
            ClientCredentials::initialize(temporary.path(), CredentialBackend::Native).unwrap();
        let mut native = foks_keystore::NativeCredentialStore::open(&credentials.state_id).unwrap();
        let manifest = native.get(checkpoint::NATIVE_MANIFEST_RECORD);
        let legacy_master = native.get(MASTER_KEY_RECORD);
        let legacy_root = native.get(STATE_ROOT_RECORD);
        let cleanup = native.remove(checkpoint::NATIVE_MANIFEST_RECORD);

        assert!(manifest.is_ok());
        assert!(matches!(legacy_master, Err(foks_keystore::Error::Missing)));
        assert!(matches!(legacy_root, Err(foks_keystore::Error::Missing)));
        assert!(matches!(cleanup, Ok(true)));
    }

    #[test]
    fn legacy_native_record_storage_is_rejected_before_keychain_access() {
        let temporary = tempfile::tempdir().unwrap();
        let root = prepare_private_directory(&temporary.path().join("state")).unwrap();
        create_private_config(
            &root.join(STATE_CONFIG_FILE),
            b"version = 2\nstate_id = \"legacy\"\ncredential_backend = \"native\"\n",
        )
        .unwrap();

        assert!(matches!(
            ClientCredentials::open(&root),
            Err(Error::InvalidConfig(
                "native client state uses unsupported per-record credential storage"
            ))
        ));
    }

    #[test]
    fn native_checkpoint_enrolls_only_before_hard_state_exists() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let mut registry = ProfileRegistry::open(&root).unwrap();
        registry
            .add(profile("local", ProtocolPolicy::V019))
            .unwrap();
        let session = ProfileSession::open(&registry, "local").unwrap();
        let credentials = ClientCredentials {
            lease: ClientStateLease::acquire(root.canonicalize().unwrap()).unwrap(),
            root: root.canonicalize().unwrap(),
            state_id: "test-state".to_owned(),
            backend: CredentialBackend::Native,
        };
        let key = rollback_record_key("local").unwrap();
        let mut external = MemorySecretStore::default();

        let current = session.rollback_checkpoint().unwrap();
        credentials
            .verify_native_checkpoint_with_store(&session, &current, true, &mut external)
            .unwrap();
        assert!(session.paths().hard_database.is_file());
        assert!(SecretStore::get(&mut external, &key).is_ok());

        SecretStore::remove(&mut external, &key).unwrap();
        let error = credentials
            .verify_checkpoint_with_store(&session, &key, &mut external)
            .unwrap_err();
        assert!(matches!(
            &error,
            Error::CheckpointResetRequired {
                reason: "external checkpoint is missing",
                ..
            }
        ));
        let message = error.to_string();
        assert!(message.contains("reset-hard-state local --confirm-delete"));
        assert!(message.contains(&root.display().to_string()));

        SecretStore::put(&mut external, &key, b"not a checkpoint").unwrap();
        assert!(matches!(
            credentials.verify_checkpoint_with_store(&session, &key, &mut external),
            Err(Error::CheckpointResetRequired {
                reason: "external checkpoint is invalid",
                ..
            })
        ));
    }

    #[test]
    fn native_database_identity_cannot_be_claimed_by_two_profiles() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let mut registry = ProfileRegistry::open(&root).unwrap();
        registry
            .add(profile("first", ProtocolPolicy::V019))
            .unwrap();
        registry
            .add(profile("second", ProtocolPolicy::V019))
            .unwrap();
        let first = ProfileSession::open(&registry, "first").unwrap();
        let second = ProfileSession::open(&registry, "second").unwrap();
        let first_checkpoint = first.rollback_checkpoint().unwrap();
        std::fs::copy(&first.paths().hard_database, &second.paths().hard_database).unwrap();
        let second_checkpoint = second.rollback_checkpoint().unwrap();
        assert_eq!(first_checkpoint.database_id, second_checkpoint.database_id);
        assert_eq!(first_checkpoint.write_token, second_checkpoint.write_token);

        let credentials = ClientCredentials {
            lease: ClientStateLease::acquire(root.canonicalize().unwrap()).unwrap(),
            root: root.canonicalize().unwrap(),
            state_id: "test-state".to_owned(),
            backend: CredentialBackend::Native,
        };
        let mut external = MemorySecretStore::default();
        assert!(matches!(
            credentials.verify_native_checkpoint_with_store(
                &second,
                &second_checkpoint,
                false,
                &mut external
            ),
            Err(Error::CheckpointResetRequired {
                reason: "external checkpoint is missing",
                ..
            })
        ));
        assert!(matches!(
            SecretStore::get(
                &mut external,
                &database_claim_record_key(&first_checkpoint.database_id)
            ),
            Err(foks_keystore::Error::Missing)
        ));
        SecretStore::put(
            &mut external,
            &rollback_record_key("first").unwrap(),
            &serde_json::to_vec(&first_checkpoint).unwrap(),
        )
        .unwrap();
        credentials
            .verify_native_checkpoint_with_store(&first, &first_checkpoint, false, &mut external)
            .unwrap();
        SecretStore::put(
            &mut external,
            &rollback_record_key("second").unwrap(),
            &serde_json::to_vec(&second_checkpoint).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            credentials.verify_native_checkpoint_with_store(
                &second,
                &second_checkpoint,
                false,
                &mut external
            ),
            Err(Error::CheckpointResetRequired {
                reason: "hard-state database identity is already claimed by another profile",
                ..
            })
        ));
    }

    #[test]
    fn native_state_root_binding_rejects_a_copied_root() {
        let temporary = tempfile::tempdir().unwrap();
        let original = temporary.path().join("original");
        let copied = temporary.path().join("copied");
        std::fs::create_dir_all(&original).unwrap();
        std::fs::create_dir_all(&copied).unwrap();
        let original = original.canonicalize().unwrap();
        let copied = copied.canonicalize().unwrap();
        let mut external = MemorySecretStore::default();
        SecretStore::put(
            &mut external,
            STATE_ROOT_RECORD,
            &state_root_binding(&original),
        )
        .unwrap();

        let original_credentials = ClientCredentials {
            lease: ClientStateLease::acquire(&original).unwrap(),
            root: original,
            state_id: "shared-state".to_owned(),
            backend: CredentialBackend::Native,
        };
        original_credentials
            .verify_root_binding_with_store(&mut external)
            .unwrap();

        let copied_credentials = ClientCredentials {
            lease: ClientStateLease::acquire(&copied).unwrap(),
            root: copied,
            state_id: "shared-state".to_owned(),
            backend: CredentialBackend::Native,
        };
        assert!(matches!(
            copied_credentials.verify_root_binding_with_store(&mut external),
            Err(Error::InvalidConfig(
                "native client state belongs to a different root path"
            ))
        ));
    }

    #[test]
    fn checked_session_rejects_credentials_from_another_state_root() {
        let temporary = tempfile::tempdir().unwrap();
        let first_root = temporary.path().join("first");
        let second_root = temporary.path().join("second");
        let credentials =
            ClientCredentials::initialize(&first_root, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&second_root).unwrap();
        registry
            .add(profile("local", ProtocolPolicy::V019))
            .unwrap();
        let session = ProfileSession::open(&registry, "local").unwrap();

        assert!(matches!(
            credentials.with_checked_session(&session, |_| Ok::<(), Error>(())),
            Err(Error::InvalidConfig(
                "profile session belongs to a different client state"
            ))
        ));
    }

    /// A recursive federation graph revisits a profile that is already on
    /// this thread's stack. Without reentry that visit blocks on the very
    /// operation lock this thread holds; with it, the nested visit runs and
    /// the outermost frame still owns the real lock and the checkpoint.
    #[test]
    fn checked_session_reenters_an_ancestor_profile_on_the_same_thread() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let credentials =
            ClientCredentials::initialize(&root, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&root).unwrap();
        registry
            .add(profile("local", ProtocolPolicy::V019))
            .unwrap();
        let outer = ProfileSession::open(&registry, "local").unwrap();

        credentials
            .with_checked_session(&outer, |_| {
                let nested = ProfileSession::open(&registry, "local")?;
                let result = credentials.try_with_checked_session(&nested, |checked| {
                    assert_eq!(checked.profile().name, "local");
                    Ok::<_, Error>(7)
                })?;
                assert_eq!(result, Some(7));
                credentials.with_checked_session(&nested, |checked| {
                    assert_eq!(checked.profile().name, "local");
                    Ok::<_, Error>(())
                })
            })
            .unwrap();

        // The outermost operation still owns and releases the real file lock.
        assert_eq!(
            credentials
                .try_with_checked_session(&outer, |_| Ok::<_, Error>(9))
                .unwrap(),
            Some(9)
        );
    }

    /// A paired operation takes both locks in a canonical order, so it cannot
    /// safely reuse a hold this thread already owns. It must say so rather
    /// than block on itself.
    #[test]
    fn paired_checked_sessions_refuse_to_nest_inside_a_held_profile() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let credentials =
            ClientCredentials::initialize(&root, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&root).unwrap();
        registry
            .add(profile("local", ProtocolPolicy::V019))
            .unwrap();
        registry
            .add(profile("remote", ProtocolPolicy::V019))
            .unwrap();
        let local = ProfileSession::open(&registry, "local").unwrap();
        let remote = ProfileSession::open(&registry, "remote").unwrap();

        let outcome = credentials.with_checked_session(&local, |_| {
            credentials.with_checked_sessions(&local, &remote, |_, _| Ok::<_, Error>(()))
        });
        assert!(
            matches!(outcome, Err(Error::InvalidConfig(_))),
            "{outcome:?}"
        );
    }

    #[test]
    fn dual_checked_sessions_preserve_call_order_and_release_both_locks_after_error() {
        use foks_client_db::ScheduledJob;

        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let credentials =
            ClientCredentials::initialize(&root, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&root).unwrap();
        registry
            .add(profile("first", ProtocolPolicy::V019))
            .unwrap();
        registry
            .add(profile("second", ProtocolPolicy::V019))
            .unwrap();
        let first = ProfileSession::open(&registry, "first").unwrap();
        let second = ProfileSession::open(&registry, "second").unwrap();
        let verified = foks_verify::verify_public_host(
            "foks.app",
            include_bytes!(
                "../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
            ),
        )
        .unwrap();
        let host_id = verified.snapshot.host_id().to_vec();

        let result = credentials.with_checked_sessions(&second, &first, |left, right| {
            assert_eq!(left.profile().name, "second");
            assert_eq!(right.profile().name, "first");
            for (index, session) in [left, right].into_iter().enumerate() {
                let mut store = HardStateStore::open(&session.paths().hard_database)?;
                store.accept_verified_host(&verified.snapshot)?;
                store.register_scheduled_job(&ScheduledJob {
                    job_id: [u8::try_from(index + 1).unwrap(); 16],
                    kind: ScheduledJobKind::FederationRefresh,
                    host_id: host_id.clone(),
                    scope_id: br#"{"remote_profile":"other"}"#.to_vec(),
                    interval_micros: 1_000,
                    next_run_at: 100,
                    failure_count: 0,
                    lease_until: None,
                    last_completed_at: None,
                    last_error: None,
                    updated_at: 90,
                })?;
            }
            Err::<(), Error>(Error::InvalidConfig("injected dual-profile failure"))
        });
        assert!(matches!(
            result,
            Err(Error::InvalidConfig("injected dual-profile failure"))
        ));

        credentials
            .with_checked_sessions(&first, &second, |left, right| {
                assert_eq!(left.profile().name, "first");
                assert_eq!(right.profile().name, "second");
                for session in [left, right] {
                    let store = HardStateStore::open(&session.paths().hard_database)?;
                    assert!(store.metadata()?.revision > 0);
                }
                Ok::<(), Error>(())
            })
            .unwrap();
    }

    #[test]
    fn reset_preview_binds_validated_resumables_and_all_local_artifacts() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let credentials =
            ClientCredentials::initialize(&root, CredentialBackend::PrivateFile).unwrap();
        let master = credentials.master_key().unwrap();
        let mut registry = ProfileRegistry::open(&root).unwrap();
        registry
            .add(profile("local", ProtocolPolicy::V019))
            .unwrap();
        let session = ProfileSession::open(&registry, "local").unwrap();
        HardStateStore::open(&session.paths().hard_database).unwrap();
        HardStateStore::open(&session.paths().soft_database).unwrap();
        for database in [
            &session.paths().hard_database,
            &session.paths().soft_database,
        ] {
            for sidecar in hard_state_artifact_paths(database)[1..].iter() {
                std::fs::write(sidecar, b"test sidecar").unwrap();
            }
        }
        std::fs::create_dir_all(&session.paths().protected_mutations).unwrap();
        std::fs::write(
            session.paths().protected_mutations.join("queued-write"),
            b"protected mutation",
        )
        .unwrap();
        let mut store = foks_keystore::EncryptedFileSecretStore::open(
            &session.paths().credential_store,
            derive_vault_key(&master),
        )
        .unwrap();
        {
            let mut vault = AccountVault::new(&mut store);
            vault
                .put_pending(&PendingSignup::random("personal", "alice").unwrap())
                .unwrap();
        }
        drop(store);

        let preview = credentials.describe_reset_state(&session).unwrap();
        assert_eq!(preview.profile, "local");
        assert_eq!(preview.resumables.len(), 1);
        assert_eq!(
            preview.resumables[0].kind,
            PendingOperationKind::AccountSignup
        );
        assert_eq!(preview.resumables[0].alias, "personal");
        for kind in [
            ResetArtifactKind::HardState,
            ResetArtifactKind::SoftState,
            ResetArtifactKind::ProtectedMutations,
            ResetArtifactKind::CredentialsAndResumables,
        ] {
            assert!(preview
                .artifacts
                .iter()
                .any(|artifact| artifact.kind == kind));
        }

        // Any post-preview change, even in soft state, invalidates the exact
        // deletion authorization and leaves every artifact intact.
        std::fs::write(
            session.paths().protected_mutations.join("late-write"),
            b"arrived after preview",
        )
        .unwrap();
        assert!(matches!(
            credentials.reset_hard_state_if_matches(&session, preview.state_digest()),
            Err(Error::ResetPreviewChanged)
        ));
        assert!(session.paths().hard_database.exists());
        assert!(session.paths().credential_store.exists());

        // An interruption after atomic directory staging is safe to resume.
        // The next preview still validates and names the quarantined resumable.
        let preview = credentials.describe_reset_state(&session).unwrap();
        checkpoint::TEST_FAIL_AFTER_RESET_STAGING.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(matches!(
            credentials.reset_hard_state_if_matches(&session, preview.state_digest()),
            Err(Error::InvalidConfig("injected reset interruption"))
        ));
        let resumed = credentials.describe_reset_state(&session).unwrap();
        assert_eq!(resumed.resumables.len(), 1);
        assert_eq!(resumed.resumables[0].alias, "personal");
        credentials
            .reset_hard_state_if_matches(&session, resumed.state_digest())
            .unwrap();

        for database in [
            &session.paths().hard_database,
            &session.paths().soft_database,
        ] {
            assert!(hard_state_artifact_paths(database)
                .iter()
                .all(|path| !path.exists()));
        }
        assert!(!session.paths().protected_mutations.exists());
        assert!(!session.paths().credential_store.exists());
        assert!(!session.paths().directory.join(".reset-mutations").exists());
        assert!(!session
            .paths()
            .directory
            .join(".reset-credentials")
            .exists());
    }

    #[test]
    fn best_effort_reset_goes_ahead_when_the_credentials_cannot_be_read() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let credentials =
            ClientCredentials::initialize(&root, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&root).unwrap();
        registry
            .add(profile("local", ProtocolPolicy::V019))
            .unwrap();
        let session = ProfileSession::open(&registry, "local").unwrap();
        HardStateStore::open(&session.paths().hard_database).unwrap();
        std::fs::create_dir_all(&session.paths().credential_store).unwrap();
        std::fs::write(
            session.paths().credential_store.join("account.personal"),
            b"sealed",
        )
        .unwrap();

        // The master key is unreadable. The exact preview refuses; the
        // best-effort one describes what is on disk and says why the
        // resumables inside the credential store could not be listed.
        std::fs::write(root.join("master.key"), b"short").unwrap();
        assert!(matches!(
            credentials.describe_reset_state(&session),
            Err(Error::Keystore(foks_keystore::Error::InvalidMasterKey))
        ));
        let preview = credentials
            .describe_reset_state_best_effort(&session)
            .unwrap();
        assert!(preview.credentials_unavailable.is_some());
        assert!(preview.resumables.is_empty());
        for kind in [
            ResetArtifactKind::HardState,
            ResetArtifactKind::CredentialsAndResumables,
        ] {
            assert!(preview
                .artifacts
                .iter()
                .any(|artifact| artifact.kind == kind));
        }

        // The preview still binds the reset to the state it described.
        std::fs::write(session.paths().credential_store.join("late"), b"x").unwrap();
        assert!(matches!(
            credentials.reset_hard_state_best_effort_if_matches(&session, preview.state_digest()),
            Err(Error::ResetPreviewChanged)
        ));
        assert!(session.paths().hard_database.exists());

        let preview = credentials
            .describe_reset_state_best_effort(&session)
            .unwrap();
        let outcome = credentials
            .reset_hard_state_best_effort_if_matches(&session, preview.state_digest())
            .unwrap();
        // A private-file state keeps no records outside the profile directory,
        // so nothing was left behind.
        assert_eq!(outcome.credential_records_retained, None);
        assert!(!session.paths().hard_database.exists());
        assert!(!session.paths().credential_store.exists());
        assert!(!session
            .paths()
            .directory
            .join(".reset-credentials")
            .exists());
    }

    #[test]
    fn rollback_checkpoint_requires_both_authenticated_histories() {
        let public = foks_verify::verify_public_host(
            "foks.app",
            include_bytes!(
                "../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
            ),
        )
        .unwrap();
        let snapshot = public.snapshot;
        let checkpoint = RollbackCheckpoint {
            profile: "hosted".to_owned(),
            database_id: [7; 16],
            hard_state_revision: 12,
            write_token: [8; 16],
            host: Some(RollbackHostCheckpoint {
                host_id: snapshot.host_id().to_vec(),
                host_chain_sequence: snapshot.chain_seqno(),
                host_chain_tail: snapshot.chain_tail_hash(),
                host_chain_bytes: snapshot.chain_bytes().to_vec(),
                merkle_epoch: snapshot.merkle_root().epoch(),
                merkle_root_hash: snapshot.merkle_root().root_hash(),
                authenticated_roots: snapshot
                    .merkle_root()
                    .authenticated_roots()
                    .iter()
                    .map(|root| (root.epoch(), root.root_hash()))
                    .collect(),
            }),
        };
        assert!(checkpoint.verify_descends_from(&checkpoint).is_ok());
        let external = serde_json::to_vec(&checkpoint).unwrap();
        assert!(external.len() < 1024);
        assert!(!String::from_utf8_lossy(&external).contains("host_chain_bytes"));
        assert!(!String::from_utf8_lossy(&external).contains("authenticated_roots"));

        let mut missing_merkle_history = checkpoint.clone();
        missing_merkle_history
            .host
            .as_mut()
            .unwrap()
            .authenticated_roots
            .clear();
        assert!(matches!(
            missing_merkle_history.verify_descends_from(&checkpoint),
            Err(Error::RollbackDetected("Merkle checkpoint is absent"))
        ));

        let mut forked_hostchain = checkpoint.clone();
        forked_hostchain.host.as_mut().unwrap().host_chain_tail[0] ^= 1;
        assert!(matches!(
            checkpoint.verify_descends_from(&forked_hostchain),
            Err(Error::RollbackDetected("hostchain checkpoint is absent"))
        ));

        let mut restored_database = checkpoint.clone();
        restored_database.hard_state_revision -= 1;
        assert!(matches!(
            restored_database.verify_descends_from(&checkpoint),
            Err(Error::RollbackDetected("hard-state revision rolled back"))
        ));

        let mut substituted_database = checkpoint.clone();
        substituted_database.database_id[0] ^= 1;
        assert!(matches!(
            substituted_database.verify_descends_from(&checkpoint),
            Err(Error::RollbackDetected(
                "hard-state database identity changed"
            ))
        ));

        let mut committed_before_external_update = checkpoint.clone();
        committed_before_external_update.hard_state_revision += 1;
        committed_before_external_update.write_token[0] ^= 1;
        assert_eq!(
            committed_before_external_update
                .reconciliation(&checkpoint)
                .unwrap(),
            CheckpointReconciliation::AdvanceExternal
        );

        let mut equal_revision_fork = checkpoint.clone();
        equal_revision_fork.write_token[0] ^= 1;
        assert!(matches!(
            equal_revision_fork.reconciliation(&checkpoint),
            Err(Error::RollbackDetected(
                "hard-state write token changed at the same revision"
            ))
        ));

        let mut unchanged_write_token = checkpoint.clone();
        unchanged_write_token.hard_state_revision += 1;
        assert!(matches!(
            unchanged_write_token.reconciliation(&checkpoint),
            Err(Error::RollbackDetected(
                "hard-state write token did not advance"
            ))
        ));
    }

    #[test]
    fn database_checkpoint_rejects_restores_before_jobs_and_mutation_journals() {
        use foks_client_db::{MutationOperation, MutationState, ScheduledJob};

        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let mut registry = ProfileRegistry::open(&root).unwrap();
        registry
            .add(profile("local", ProtocolPolicy::V019))
            .unwrap();
        let session = ProfileSession::open(&registry, "local").unwrap();
        let verified = foks_verify::verify_public_host(
            "foks.app",
            include_bytes!(
                "../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
            ),
        )
        .unwrap();
        let host_id = verified.snapshot.host_id().to_vec();
        let mut store = HardStateStore::open(&session.paths().hard_database).unwrap();
        store.accept_verified_host(&verified.snapshot).unwrap();
        drop(store);
        let before_job = session.rollback_checkpoint().unwrap();

        let mut store = HardStateStore::open(&session.paths().hard_database).unwrap();
        store
            .register_scheduled_job(&ScheduledJob {
                job_id: [1; 16],
                kind: ScheduledJobKind::UserRefresh,
                host_id: host_id.clone(),
                scope_id: vec![2; 33],
                interval_micros: 1_000,
                next_run_at: 100,
                failure_count: 0,
                lease_until: None,
                last_completed_at: None,
                last_error: None,
                updated_at: 90,
            })
            .unwrap();
        drop(store);
        let after_job = session.rollback_checkpoint().unwrap();
        assert!(matches!(
            before_job.reconciliation(&after_job),
            Err(Error::RollbackDetected("hard-state revision rolled back"))
        ));

        let mut store = HardStateStore::open(&session.paths().hard_database).unwrap();
        store
            .record_mutation(&MutationOperation {
                operation_id: [3; 16],
                kind: MutationKind::DeviceProvision,
                host_id,
                scope_id: vec![4; 33],
                subject_id: vec![5; 33],
                expected_version: Some(1),
                request_hash: [6; 32],
                material_ref: b"mutation-material".to_vec(),
                material_hash: [7; 32],
                state: MutationState::Prepared,
                attempt_count: 0,
                created_at: 100,
                updated_at: 100,
            })
            .unwrap();
        drop(store);
        let after_mutation = session.rollback_checkpoint().unwrap();
        assert!(matches!(
            after_job.reconciliation(&after_mutation),
            Err(Error::RollbackDetected("hard-state revision rolled back"))
        ));
        assert_eq!(
            after_mutation.reconciliation(&after_job).unwrap(),
            CheckpointReconciliation::AdvanceExternal
        );
    }

    #[test]
    fn failed_registry_publication_does_not_change_live_state() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let mut registry = ProfileRegistry::open(&root).unwrap();
        std::fs::create_dir(root.join("profiles.toml")).unwrap();

        assert!(registry.add(profile("hosted", probe_only())).is_err());
        assert!(matches!(
            registry.profile("hosted"),
            Err(Error::ProfileMissing)
        ));
    }

    #[test]
    fn local_team_inventory_is_independent_of_remote_team_permission() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let credentials =
            ClientCredentials::initialize(&root, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&root).unwrap();
        registry.add(profile("hosted", probe_only())).unwrap();
        let session = ProfileSession::open(&registry, "hosted").unwrap();
        credentials
            .with_checked_session(&session, |checked| {
                let mut secrets = MemorySecretStore::default();
                let mut vault = AccountVault::new(&mut secrets);
                assert!(checked.list_local_teams(&mut vault)?.is_empty());
                assert!(matches!(
                    checked.list_teams(&mut vault),
                    Err(Error::CapabilityDenied(Capability::Teams))
                ));
                Ok::<_, Error>(())
            })
            .unwrap();
    }

    #[test]
    fn current_hosted_policy_fails_closed_until_validated() {
        let probe_only = profile("hosted", probe_only());
        assert!(probe_only.require(Capability::Probe).is_ok());
        assert!(matches!(
            probe_only.require(Capability::Kv),
            Err(Error::CapabilityDenied(Capability::Kv))
        ));
    }

    #[test]
    fn signed_canaries_grant_temporarily_and_drift_revokes_everything() {
        let seed = [
            0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec,
            0x2c, 0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03,
            0x1c, 0xae, 0x7f, 0x60,
        ];
        let mut artifact = foks_compat_artifact::CanaryArtifact {
            schema_version: foks_compat_artifact::SCHEMA_VERSION,
            generation: 1,
            target: "foks.app".to_owned(),
            run_id: "run-1".to_owned(),
            generated_at: 100,
            expires_at: 200,
            protocol_metadata_sha256: PINNED_PROTOCOL_METADATA_SHA256.to_owned(),
            mutation_digest: "22".repeat(32),
            read_digest: "33".repeat(32),
            outcome: CanaryOutcome::Compatible,
            capabilities: BTreeSet::from([
                "kv".to_owned(),
                "passphrases".to_owned(),
                "teams".to_owned(),
                "user-sync".to_owned(),
            ]),
            drift_reason: String::new(),
        };
        let initial = profile("hosted", probe_only());
        let signed = SignedCanaryArtifact::sign(artifact.clone(), &seed).unwrap();
        let granted = initial.apply_canary(&signed, 101).unwrap();
        let temporary = tempfile::tempdir().unwrap();
        let mut registry = ProfileRegistry::open(temporary.path()).unwrap();
        registry.add(initial.clone()).unwrap();
        assert_eq!(
            registry.apply_canary("hosted", &signed, 101).unwrap(),
            granted
        );
        drop(registry);
        let mut registry = ProfileRegistry::open(temporary.path()).unwrap();
        assert_eq!(registry.profile("hosted").unwrap(), &granted);
        assert_eq!(granted.apply_canary(&signed, 101).unwrap(), granted);
        let grants = granted.protocol.compatibility_status();
        assert_eq!(grants.denial_at(Capability::Kv, 199), None);
        assert_eq!(
            grants.denial_at(Capability::Federation, 199),
            Some(CapabilityDenial::NotGranted)
        );
        assert_eq!(
            grants.denial_at(Capability::Kv, 200),
            Some(CapabilityDenial::Expired)
        );
        assert!(granted.require_at(Capability::Kv, 199).is_ok());
        assert!(granted.require_at(Capability::Passphrases, 199).is_ok());
        assert!(granted.require_at(Capability::Teams, 199).is_ok());
        assert!(matches!(
            granted.require_at(Capability::Federation, 199),
            Err(Error::CapabilityDenied(Capability::Federation))
        ));
        assert!(matches!(
            granted.require_at(Capability::Kv, 200),
            Err(Error::CapabilityDenied(Capability::Kv))
        ));

        assert!(granted.require_at(Capability::Chat, 199).is_err());
        let mut chat_artifact = artifact.clone();
        chat_artifact.capabilities.insert("chat".to_owned());
        let signed_chat = SignedCanaryArtifact::sign(chat_artifact, &seed).unwrap();
        let chat = initial.apply_canary(&signed_chat, 101).unwrap();
        assert!(chat.require_at(Capability::Chat, 199).is_ok());
        assert!(chat.require_at(Capability::Chat, 200).is_err());

        let mut tampered = granted.clone();
        if let ProtocolPolicy::CurrentValidated { artifact, .. } = &mut tampered.protocol {
            artifact.artifact.capabilities.insert("chat".to_owned());
        }
        assert!(matches!(
            tampered.validate(),
            Err(Error::InvalidProfile("canary signature is invalid"))
        ));

        artifact.outcome = CanaryOutcome::Drift;
        artifact.generation = 2;
        artifact.capabilities.clear();
        artifact.drift_reason = "read-back mismatch".to_owned();
        let drift = SignedCanaryArtifact::sign(artifact, &seed).unwrap();
        let revoked = granted.apply_canary(&drift, 102).unwrap();
        assert_eq!(
            revoked.protocol.compatibility_status(),
            CompatibilityStatus::Incompatible {
                reason: CompatibilityFailure::Drift,
                expires_at: 200,
            }
        );
        assert_eq!(
            registry.apply_canary("hosted", &drift, 102).unwrap(),
            revoked
        );
        assert!(registry.apply_canary("hosted", &signed, 102).is_err());
        drop(registry);
        let registry = ProfileRegistry::open(temporary.path()).unwrap();
        assert_eq!(registry.profile("hosted").unwrap(), &revoked);
        assert!(matches!(
            revoked.require_at(Capability::Kv, 102),
            Err(Error::CapabilityDenied(Capability::Kv))
        ));
        assert!(matches!(
            revoked.protocol,
            ProtocolPolicy::CurrentProbeOnly { .. }
        ));
        assert!(matches!(
            revoked.apply_canary(&signed, 102),
            Err(Error::InvalidProfile("canary generation rolled back"))
        ));

        artifact = drift.artifact.clone();
        artifact.generation = 3;
        artifact.outcome = CanaryOutcome::Compatible;
        artifact.capabilities = BTreeSet::from(["kv".to_owned()]);
        artifact.drift_reason.clear();
        artifact.protocol_metadata_sha256 = "44".repeat(32);
        let mismatched = SignedCanaryArtifact::sign(artifact, &seed).unwrap();
        let still_revoked = revoked.apply_canary(&mismatched, 103).unwrap();
        assert_eq!(
            still_revoked.protocol.compatibility_status(),
            CompatibilityStatus::Incompatible {
                reason: CompatibilityFailure::ProtocolMismatch,
                expires_at: 200,
            }
        );
        assert!(matches!(
            still_revoked.protocol,
            ProtocolPolicy::CurrentProbeOnly { .. }
        ));
        assert!(matches!(
            still_revoked.require_at(Capability::Kv, 103),
            Err(Error::CapabilityDenied(Capability::Kv))
        ));
    }

    #[test]
    fn pinned_canary_digest_matches_the_embedded_v019_metadata() {
        use sha2::{Digest as _, Sha256};

        let digest = Sha256::digest(include_bytes!(
            "../../foks-server/protocol/upstream-v0.1.9.json"
        ));
        assert_eq!(hex(&digest), PINNED_PROTOCOL_METADATA_SHA256);
    }

    #[test]
    fn hosted_lease_urls_are_https_and_do_not_carry_credentials() {
        for invalid in [
            "http://updates.example.test/lease.json",
            "https://user@updates.example.test/lease.json",
            "https://updates.example.test/lease.json?token=secret",
            "https://updates.example.test/lease.json#current",
        ] {
            let mut invalid_profile = profile("hosted", probe_only());
            if let ProtocolPolicy::CurrentProbeOnly { lease_url, .. } =
                &mut invalid_profile.protocol
            {
                *lease_url = invalid.to_owned();
            }
            assert!(invalid_profile.validate().is_err());
        }
    }

    #[test]
    fn probe_reports_preserve_acceptance_outcomes() {
        assert_eq!(
            ProbeAcceptance::from(foks_client_db::Acceptance::Inserted),
            ProbeAcceptance::Inserted
        );
        assert_eq!(
            ProbeAcceptance::from(foks_client_db::Acceptance::Advanced),
            ProbeAcceptance::Advanced
        );
        assert_eq!(
            ProbeAcceptance::from(foks_client_db::Acceptance::Unchanged),
            ProbeAcceptance::Unchanged
        );
    }

    #[test]
    fn passive_server_status_reports_only_configured_and_authenticated_facts() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let credentials =
            ClientCredentials::initialize(&root, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&root).unwrap();
        registry
            .add(profile("hosted", ProtocolPolicy::V019))
            .unwrap();
        let session = ProfileSession::open(&registry, "hosted").unwrap();

        let empty = session.server_status_without_pinned_host().unwrap();
        assert_eq!(empty.profile, "hosted");
        assert_eq!(empty.configured_probe, "foks.app");
        assert!(empty.host.is_none());
        assert_eq!(empty.compatibility, CompatibilityStatus::NotRequired);
        assert_eq!(empty.chat_supported, None);

        let verified = foks_verify::verify_public_host(
            "foks.app",
            include_bytes!(
                "../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
            ),
        )
        .unwrap();
        HardStateStore::open(&session.paths().hard_database)
            .unwrap()
            .accept_verified_host(&verified.snapshot)
            .unwrap();
        assert!(session.server_status_without_pinned_host().is_err());

        let status = credentials
            .with_checked_session(&session, |checked| checked.server_status())
            .unwrap();
        let host = status.host.as_ref().unwrap();
        assert_eq!(host.lookup_name, "foks.app");
        assert_eq!(host.canonical_name, verified.snapshot.canonical_name());
        assert_eq!(host.host_id_hex, hex(verified.snapshot.host_id()));
        assert_eq!(host.host_chain_sequence, verified.snapshot.chain_seqno());
        assert_eq!(host.merkle_epoch, verified.snapshot.merkle_root().epoch());
        let value = serde_json::to_value(status).unwrap();
        assert!(value.get("checked_at").is_none());
        assert!(value.get("trusted_since").is_none());
    }

    #[test]
    fn account_vault_validates_binding_and_lists_aliases() {
        let mut store = MemorySecretStore::default();
        let credential = DeviceCredential {
            key_kind: foks_client::SoftwareKeyKind::Device,
            uid: EntityId::from_bytes({
                let mut bytes = vec![0; 33];
                bytes[0] = ENTITY_USER;
                bytes
            })
            .unwrap(),
            seed: SecretSeed::new([9; 32]),
            certificate_chain: vec![vec![1, 2, 3]],
        };
        let mut vault = AccountVault::new(&mut store);
        let pending = PendingSignup::random("personal", "alice").unwrap();
        vault.put_pending(&pending).unwrap();
        vault
            .commit_created("personal", "alice", &credential)
            .unwrap();
        assert!(vault.pending("personal").is_ok());
        vault.remove_pending_signup("personal").unwrap();
        assert!(matches!(
            vault.pending("personal"),
            Err(Error::AccountMissing)
        ));
        assert_eq!(vault.aliases().unwrap(), vec!["personal"]);
        let loaded = vault.account("personal").unwrap();
        assert_eq!(loaded.username, "alice");
        assert_eq!(loaded.credential.seed.as_slice(), &[9; 32]);
    }

    #[test]
    fn credential_aliases_are_type_specific_and_pending_records_reserve_names() {
        let mut store = MemorySecretStore::default();
        for key in [
            pending_key("software-pending"),
            pending_device_key("device-pending"),
            pending_recovery_key("recovery-pending"),
            yubi::pending_yubi_key("yubi-pending"),
            yubi::yubi_account_key("yubi-ready"),
        ] {
            store.put(&key, b"reserved").unwrap();
        }
        let mut vault = AccountVault::new(&mut store);
        assert!(vault.aliases().unwrap().is_empty());
        assert_eq!(
            vault.yubi_aliases().unwrap(),
            vec!["yubi-pending", "yubi-ready"]
        );
        assert!(vault.yubi_accounts().is_err());
        for alias in [
            "software-pending",
            "device-pending",
            "recovery-pending",
            "yubi-pending",
            "yubi-ready",
        ] {
            assert!(vault.contains(alias).unwrap(), "{alias} was not reserved");
        }
    }

    #[test]
    fn pending_operations_are_enumerated_by_resumable_kind() {
        let mut store = MemorySecretStore::default();
        let signup = PendingSignup::random("signup", "alice").unwrap();
        let mut vault = AccountVault::new(&mut store);
        vault.put_pending(&signup).unwrap();
        let pending = vault.pending_operations().unwrap();
        assert_eq!(pending.len(), 1);
        assert!(pending.iter().any(|operation| {
            operation.kind == PendingOperationKind::AccountSignup
                && operation.alias == "signup"
                && operation.target.is_none()
        }));
    }

    #[test]
    fn malformed_pending_record_is_not_advertised_as_resumable() {
        let mut store = MemorySecretStore::default();
        store
            .put(
                &pending_device_key("new-laptop"),
                b"authenticated but malformed",
            )
            .unwrap();
        assert!(AccountVault::new(&mut store).pending_operations().is_err());
    }

    #[test]
    fn key_domains_are_separate_and_deterministic() {
        let master = [7; 32];
        assert_eq!(derive_vault_key(&master), derive_vault_key(&master));
        assert_eq!(derive_mutation_key(&master), derive_mutation_key(&master));
        assert_ne!(derive_vault_key(&master), derive_mutation_key(&master));
    }

    #[test]
    fn kv_paths_are_absolute_and_canonical() {
        assert_eq!(
            split_parent("/one/two").unwrap(),
            ("/one".into(), "two".into())
        );
        assert_eq!(split_parent("/one").unwrap(), ("/".into(), "one".into()));
        assert_eq!(
            split_parent("/space%20parent/hello%20world").unwrap(),
            ("/space%20parent".into(), "hello world".into())
        );
        for invalid in [
            "relative",
            "/",
            "/one//two",
            "/one/../two",
            "/one/.",
            "/one/%2E%2E",
            "/one/%",
            "/one/%GG",
            "/one/%FF",
        ] {
            assert!(split_parent(invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn nonportable_kv_names_are_unambiguously_displayed() {
        assert_eq!(display_component(b"plain-name.txt"), "plain-name.txt");
        assert_eq!(
            display_component(b"space and / slash"),
            "space%20and%20%2F%20slash"
        );
        assert_eq!(display_component(&[0xff, 0]), "%FF%00");
    }

    #[test]
    fn protected_team_records_bind_ids_to_ptk_material() {
        let mut store = MemorySecretStore::default();
        let named = StoredTeam::random_named("engineering", "personal", "Engineering").unwrap();
        let expected_id = named.team_id.clone();
        let mut vault = AccountVault::new(&mut store);
        vault.put_team(&named).unwrap();
        let restored = vault.team("engineering").unwrap();
        assert_eq!(restored.team_id, expected_id);
        assert_eq!(vault.team_aliases().unwrap(), vec!["engineering"]);
        assert!(vault.pending_operations().unwrap().iter().any(|operation| {
            operation.kind == PendingOperationKind::TeamCreation
                && operation.alias == "engineering"
                && operation.target.is_none()
        }));
    }
}
