//! Product-level orchestration shared by standalone FOKS frontends.
//!
//! This crate owns profiles, capability policy, credential serialization, and
//! one-shot synchronization. It deliberately owns neither a UI nor a resident
//! runtime, and it has no dependency on any AKA crate.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use foks_client::{
    AdHocTeamSecrets, DeviceCredential, EncryptedFileMutationStore, FoksClient, FoksScheduler,
    KvWriteOptions, NamedTeamSecrets, NewSoftwareDeviceSecrets, ProbeTarget,
    ScheduledJobRegistration, SchedulerConfig, SoftwareAccountRequest, SoftwareAccountSecrets,
    SoftwareDeviceProvisionRequest,
};
use foks_client_db::{
    HardStateStore, KvDirectoryProjection, MutationKind, ScheduledJobKind, SoftStateStore,
};
use foks_compat_artifact::{Outcome as CanaryOutcome, SignedCanaryArtifact};
use foks_crypto::{derive_device_public, derive_shared_verify_key, prefixed_hash, BackupKey};
use foks_keystore::SecretStore;
use foks_proto::{
    EntityId, InviteCode, KvNodeId, KvNodeType, Role, SecretSeed, ENTITY_PUK_VERIFY, ENTITY_USER,
};
use fs2::FileExt as _;
use rustls::pki_types::CertificateDer;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::{Zeroize as _, Zeroizing};

const CONFIG_VERSION: u32 = 1;
const CREDENTIAL_VERSION: u32 = 1;
const VAULT_KEY_TYPE_ID: u64 = 0x43cc_5eca_5249_22a1;
const MUTATION_KEY_TYPE_ID: u64 = 0x5e4b_52ca_d668_dd1d;
const USER_REFRESH_JOB_TYPE_ID: u64 = 0xb1a8_c09a_d2b9_4de7;
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_CERTIFICATES: usize = 8;
const MAX_CERTIFICATE_BYTES: usize = 1024 * 1024;
const STATE_CONFIG_VERSION: u32 = 1;
const STATE_CONFIG_FILE: &str = "client-state.toml";
const MASTER_KEY_RECORD: &str = "master-key-v1";

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid FOKS profile: {0}")]
    InvalidProfile(&'static str),
    #[error("FOKS profile already exists")]
    ProfileExists,
    #[error("FOKS profile is missing")]
    ProfileMissing,
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
    #[error("FOKS backup phrase failed: {0}")]
    Backup(#[from] foks_crypto::BackupPhraseError),
    #[error("FOKS application I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("FOKS client state failed: {0}")]
    ClientDatabase(#[from] foks_client_db::Error),
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
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CredentialBackend {
    /// Native macOS Keychain or Linux Secret Service storage.
    Native,
    /// Explicit development/test backend with no external rollback boundary.
    PrivateFile,
}

#[derive(Debug, Deserialize, Serialize)]
struct ClientStateFile {
    version: u32,
    state_id: String,
    credential_backend: CredentialBackend,
}

/// Resolves the vault wrapping key and security checkpoint independently from
/// profiles and databases. Native records remain outside the state root.
pub struct ClientCredentials {
    root: PathBuf,
    state_id: String,
    backend: CredentialBackend,
}

impl ClientCredentials {
    pub fn initialize(root: impl AsRef<Path>, backend: CredentialBackend) -> Result<Self> {
        let root = prepare_private_directory(root.as_ref())?;
        let config_path = root.join(STATE_CONFIG_FILE);
        if config_path.exists() {
            return Err(Error::InvalidConfig("client state is already initialized"));
        }
        let mut random_id = [0u8; 16];
        getrandom::fill(&mut random_id).map_err(|_| Error::Randomness)?;
        let state_id = hex(&random_id);

        match backend {
            CredentialBackend::Native => {
                let mut master = Zeroizing::new([0u8; 32]);
                getrandom::fill(&mut *master).map_err(|_| Error::Randomness)?;
                let mut native = foks_keystore::NativeCredentialStore::open(&state_id)?;
                native.put(MASTER_KEY_RECORD, &*master)?;
            }
            CredentialBackend::PrivateFile => {
                foks_keystore::create_master_key_file(root.join("master.key"))?;
            }
        }

        let bytes = toml::to_string_pretty(&ClientStateFile {
            version: STATE_CONFIG_VERSION,
            state_id: state_id.clone(),
            credential_backend: backend,
        })?;
        if let Err(error) = create_private_config(&config_path, bytes.as_bytes()) {
            match backend {
                CredentialBackend::Native => {
                    if let Ok(mut native) = foks_keystore::NativeCredentialStore::open(&state_id) {
                        let _ = native.remove(MASTER_KEY_RECORD);
                    }
                }
                CredentialBackend::PrivateFile => {
                    let _ = fs::remove_file(root.join("master.key"));
                }
            }
            return Err(error);
        }
        Ok(Self {
            root,
            state_id,
            backend,
        })
    }

    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = prepare_private_directory(root.as_ref())?;
        let bytes = read_private_file_optional(&root.join(STATE_CONFIG_FILE), MAX_CONFIG_BYTES)?
            .ok_or(Error::InvalidConfig("client state is not initialized"))?;
        let state: ClientStateFile = toml::from_slice(&bytes)?;
        if state.version != STATE_CONFIG_VERSION {
            return Err(Error::InvalidConfig("unsupported client state version"));
        }
        validate_name(&state.state_id)?;
        Ok(Self {
            root,
            state_id: state.state_id,
            backend: state.credential_backend,
        })
    }

    pub fn backend(&self) -> CredentialBackend {
        self.backend
    }

    pub fn master_key(&self) -> Result<Zeroizing<[u8; 32]>> {
        match self.backend {
            CredentialBackend::Native => {
                let mut native = foks_keystore::NativeCredentialStore::open(&self.state_id)?;
                let bytes = native.get(MASTER_KEY_RECORD)?;
                if bytes.len() != 32 {
                    return Err(foks_keystore::Error::InvalidMasterKey.into());
                }
                let mut key = Zeroizing::new([0u8; 32]);
                key.copy_from_slice(&bytes);
                Ok(key)
            }
            CredentialBackend::PrivateFile => {
                foks_keystore::load_master_key_file(self.root.join("master.key"))
                    .map_err(Into::into)
            }
        }
    }

    /// Serializes security-sensitive use across CLI and agent processes, checks
    /// the external watermark before the operation, and only advances it after
    /// the operation succeeds.
    pub fn with_checkpoint<T, E>(
        &self,
        session: &ProfileSession,
        operation: impl FnOnce() -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E>
    where
        E: From<Error>,
    {
        let lock = self.checkpoint_lock().map_err(E::from)?;
        self.verify_checkpoint(session).map_err(E::from)?;
        let result = operation()?;
        self.advance_checkpoint(session).map_err(E::from)?;
        lock.unlock().map_err(Error::from).map_err(E::from)?;
        Ok(result)
    }

    fn verify_checkpoint(&self, session: &ProfileSession) -> Result<()> {
        if self.backend != CredentialBackend::Native {
            return Ok(());
        }
        let key = rollback_record_key(&session.profile.name)?;
        let mut native = foks_keystore::NativeCredentialStore::open(&self.state_id)?;
        match native.get(&key) {
            Ok(bytes) => {
                let previous: RollbackCheckpoint = serde_json::from_slice(&bytes)?;
                let current = session.rollback_checkpoint()?;
                current.verify_descends_from(&previous)?;
            }
            Err(foks_keystore::Error::Missing) => {}
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    fn advance_checkpoint(&self, session: &ProfileSession) -> Result<()> {
        if self.backend != CredentialBackend::Native {
            return Ok(());
        }
        let current = session.rollback_checkpoint()?;
        let key = rollback_record_key(&session.profile.name)?;
        let mut native = foks_keystore::NativeCredentialStore::open(&self.state_id)?;
        native.put(&key, &serde_json::to_vec(&current)?)?;
        Ok(())
    }

    fn checkpoint_lock(&self) -> Result<File> {
        let path = self.root.join(".rollback-checkpoint.lock");
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(path)?;
        file.lock_exclusive()?;
        Ok(file)
    }
}

fn rollback_record_key(profile: &str) -> Result<String> {
    validate_name(profile)?;
    Ok(format!("rollback.{profile}"))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RollbackCheckpoint {
    profile: String,
    host_id: Vec<u8>,
    host_chain_sequence: u64,
    host_chain_tail: [u8; 32],
    #[serde(skip, default)]
    host_chain_bytes: Vec<u8>,
    merkle_epoch: u64,
    merkle_root_hash: [u8; 32],
    #[serde(skip, default)]
    authenticated_roots: BTreeMap<u64, [u8; 32]>,
}

impl RollbackCheckpoint {
    pub fn verify_descends_from(&self, previous: &Self) -> Result<()> {
        if self.profile != previous.profile || self.host_id != previous.host_id {
            return Err(Error::RollbackDetected("host identity changed"));
        }
        if self.host_chain_sequence < previous.host_chain_sequence
            || !foks_verify::hostchain_contains_tail(
                &self.host_chain_bytes,
                previous.host_chain_sequence,
                previous.host_chain_tail,
            )?
        {
            return Err(Error::RollbackDetected("hostchain checkpoint is absent"));
        }
        if self.merkle_epoch < previous.merkle_epoch
            || self.authenticated_roots.get(&previous.merkle_epoch)
                != Some(&previous.merkle_root_hash)
        {
            return Err(Error::RollbackDetected("Merkle checkpoint is absent"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    Probe,
    Signup,
    UserSync,
    Kv,
    DeviceAdministration,
    Recovery,
    Teams,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "generation", rename_all = "kebab-case")]
pub enum ProtocolPolicy {
    /// The checked and fixture-pinned upstream v0.1.9 surface.
    V019,
    /// Current mainline or hosted service, public probing only.
    CurrentProbeOnly { canary_public_key: String },
    /// Current service after an external authenticated compatibility run.
    CurrentValidated {
        canary_public_key: String,
        artifact: Box<SignedCanaryArtifact>,
    },
}

impl ProtocolPolicy {
    fn permits_at(&self, capability: Capability, now: u64) -> bool {
        capability == Capability::Probe
            || match self {
                Self::V019 => true,
                Self::CurrentProbeOnly { .. } => false,
                Self::CurrentValidated { artifact, .. } => {
                    now < artifact.artifact.expires_at
                        && artifact
                            .artifact
                            .capabilities
                            .contains(capability_canary_name(capability))
                }
            }
    }

    fn canary_public_key(&self) -> Option<&str> {
        match self {
            Self::V019 => None,
            Self::CurrentProbeOnly { canary_public_key }
            | Self::CurrentValidated {
                canary_public_key, ..
            } => Some(canary_public_key),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum TrustRoot {
    WebPki,
    CertificateDer { path: PathBuf },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Profile {
    pub name: String,
    pub probe: String,
    pub protocol: ProtocolPolicy,
    pub trust: TrustRoot,
}

impl Profile {
    pub fn validate(&self) -> Result<()> {
        validate_name(&self.name)?;
        ProbeTarget::parse(&self.probe)?;
        if let Some(key) = self.protocol.canary_public_key() {
            let decoded = foks_compat_artifact::decode_public_key(key)
                .map_err(|_| Error::InvalidProfile("canary public key is invalid"))?;
            if let ProtocolPolicy::CurrentValidated { artifact, .. } = &self.protocol {
                artifact
                    .verify(&decoded)
                    .map_err(|_| Error::InvalidProfile("canary signature is invalid"))?;
                if artifact.artifact.target != self.probe
                    || artifact.artifact.outcome != CanaryOutcome::Compatible
                    || artifact
                        .artifact
                        .capabilities
                        .iter()
                        .any(|capability| capability_from_canary(capability).is_err())
                {
                    return Err(Error::InvalidProfile(
                        "persisted canary lease is not a compatible grant for this target",
                    ));
                }
            }
        }
        if let TrustRoot::CertificateDer { path } = &self.trust {
            if path.as_os_str().is_empty() {
                return Err(Error::InvalidProfile("certificate path is empty"));
            }
        }
        Ok(())
    }

    pub fn require(&self, capability: Capability) -> Result<()> {
        self.require_at(capability, unix_seconds()?)
    }

    pub fn require_at(&self, capability: Capability, now: u64) -> Result<()> {
        self.validate()?;
        if self.protocol.permits_at(capability, now) {
            Ok(())
        } else {
            Err(Error::CapabilityDenied(capability))
        }
    }

    pub fn apply_canary(&self, signed: &SignedCanaryArtifact, now: u64) -> Result<Self> {
        let public_key = self
            .protocol
            .canary_public_key()
            .ok_or(Error::InvalidProfile(
                "v0.1.9 profiles do not accept canaries",
            ))?;
        let decoded_key = foks_compat_artifact::decode_public_key(public_key)
            .map_err(|_| Error::InvalidProfile("canary public key is invalid"))?;
        signed
            .verify(&decoded_key)
            .map_err(|_| Error::InvalidProfile("canary signature is invalid"))?;
        if signed.artifact.target != self.probe
            || signed.artifact.generated_at > now.saturating_add(300)
            || signed.artifact.expires_at <= now
        {
            return Err(Error::InvalidProfile(
                "canary target or validity interval is invalid",
            ));
        }
        let canary_public_key = public_key.to_owned();
        let protocol = match signed.artifact.outcome {
            CanaryOutcome::Drift => ProtocolPolicy::CurrentProbeOnly { canary_public_key },
            CanaryOutcome::Compatible => ProtocolPolicy::CurrentValidated {
                canary_public_key,
                artifact: Box::new(signed.clone()),
            },
        };
        let updated = Self {
            protocol,
            ..self.clone()
        };
        updated.validate()?;
        Ok(updated)
    }
}

fn capability_from_canary(value: &str) -> Result<Capability> {
    match value {
        "signup" => Ok(Capability::Signup),
        "user-sync" => Ok(Capability::UserSync),
        "kv" => Ok(Capability::Kv),
        "device-administration" => Ok(Capability::DeviceAdministration),
        "recovery" => Ok(Capability::Recovery),
        "teams" => Ok(Capability::Teams),
        _ => Err(Error::InvalidProfile("canary grants an unknown capability")),
    }
}

fn capability_canary_name(capability: Capability) -> &'static str {
    match capability {
        Capability::Probe => "probe",
        Capability::Signup => "signup",
        Capability::UserSync => "user-sync",
        Capability::Kv => "kv",
        Capability::DeviceAdministration => "device-administration",
        Capability::Recovery => "recovery",
        Capability::Teams => "teams",
    }
}

fn unix_seconds() -> Result<u64> {
    use std::time::{SystemTime, UNIX_EPOCH};
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::InvalidConfig("system clock predates Unix epoch"))?
        .as_secs())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfilePaths {
    pub directory: PathBuf,
    pub hard_database: PathBuf,
    pub soft_database: PathBuf,
    pub protected_mutations: PathBuf,
    pub credential_store: PathBuf,
}

#[derive(Debug, Deserialize, Serialize)]
struct RegistryFile {
    version: u32,
    profiles: BTreeMap<String, Profile>,
}

pub struct ProfileRegistry {
    root: PathBuf,
    profiles: BTreeMap<String, Profile>,
}

impl ProfileRegistry {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = prepare_private_directory(root.as_ref())?;
        let path = root.join("profiles.toml");
        let profiles = match read_private_file_optional(&path, MAX_CONFIG_BYTES)? {
            Some(bytes) => {
                if bytes.len() as u64 > MAX_CONFIG_BYTES {
                    return Err(Error::InvalidConfig("profile registry is too large"));
                }
                let file: RegistryFile = toml::from_slice(&bytes)?;
                if file.version != CONFIG_VERSION {
                    return Err(Error::InvalidConfig("unsupported profile registry version"));
                }
                for (name, profile) in &file.profiles {
                    if name != &profile.name {
                        return Err(Error::InvalidConfig("profile map key does not match name"));
                    }
                    profile.validate()?;
                }
                file.profiles
            }
            None => BTreeMap::new(),
        };
        Ok(Self { root, profiles })
    }

    pub fn profiles(&self) -> impl ExactSizeIterator<Item = &Profile> {
        self.profiles.values()
    }

    pub fn profile(&self, name: &str) -> Result<&Profile> {
        self.profiles.get(name).ok_or(Error::ProfileMissing)
    }

    pub fn add(&mut self, profile: Profile) -> Result<()> {
        profile.validate()?;
        if self.profiles.contains_key(&profile.name) {
            return Err(Error::ProfileExists);
        }
        let mut next = self.profiles.clone();
        next.insert(profile.name.clone(), profile);
        self.save(&next)?;
        self.profiles = next;
        Ok(())
    }

    pub fn replace(&mut self, profile: Profile) -> Result<()> {
        profile.validate()?;
        if !self.profiles.contains_key(&profile.name) {
            return Err(Error::ProfileMissing);
        }
        let mut next = self.profiles.clone();
        next.insert(profile.name.clone(), profile);
        self.save(&next)?;
        self.profiles = next;
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> Result<bool> {
        validate_name(name)?;
        let mut next = self.profiles.clone();
        let removed = next.remove(name).is_some();
        if removed {
            self.save(&next)?;
            self.profiles = next;
        }
        Ok(removed)
    }

    pub fn paths(&self, name: &str) -> Result<ProfilePaths> {
        validate_name(name)?;
        let directory = self.root.join("profiles").join(name);
        Ok(ProfilePaths {
            hard_database: directory.join("hard.sqlite3"),
            soft_database: directory.join("soft.sqlite3"),
            protected_mutations: directory.join("mutations"),
            credential_store: directory.join("credentials"),
            directory,
        })
    }

    pub fn prepare_profile_directory(&self, name: &str) -> Result<ProfilePaths> {
        let paths = self.paths(name)?;
        prepare_private_directory(&paths.directory)?;
        Ok(paths)
    }

    fn save(&self, profiles: &BTreeMap<String, Profile>) -> Result<()> {
        let bytes = toml::to_string_pretty(&RegistryFile {
            version: CONFIG_VERSION,
            profiles: profiles.clone(),
        })?;
        atomic_private_write(&self.root.join("profiles.toml"), bytes.as_bytes())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProbeReport {
    pub lookup_name: String,
    pub canonical_name: String,
    pub host_id_hex: String,
    pub host_chain_sequence: u64,
    pub merkle_epoch: u64,
}

pub struct ProfileSession {
    profile: Profile,
    paths: ProfilePaths,
    client: FoksClient,
}

impl ProfileSession {
    pub fn open(registry: &ProfileRegistry, name: &str) -> Result<Self> {
        let profile = registry.profile(name)?.clone();
        let paths = registry.prepare_profile_directory(name)?;
        let client = client_for_trust(&profile.trust)?;
        Ok(Self {
            profile,
            paths,
            client,
        })
    }

    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    pub fn paths(&self) -> &ProfilePaths {
        &self.paths
    }

    pub fn probe_and_pin(&self) -> Result<ProbeReport> {
        self.profile.require(Capability::Probe)?;
        let target = ProbeTarget::parse(&self.profile.probe)?;
        let outcome = self
            .client
            .probe_and_pin(&target, &self.paths.hard_database)?;
        Ok(ProbeReport {
            lookup_name: target.hostname().to_owned(),
            canonical_name: outcome.verified.snapshot.canonical_name().to_owned(),
            host_id_hex: hex(outcome.pinned.host_id().as_bytes()),
            host_chain_sequence: outcome.verified.snapshot.chain_seqno(),
            merkle_epoch: outcome.verified.snapshot.merkle_root().epoch(),
        })
    }

    pub fn pinned_host(&self) -> Result<foks_client::PinnedHost> {
        let target = ProbeTarget::parse(&self.profile.probe)?;
        self.client
            .pinned_host(target.hostname(), &self.paths.hard_database)
            .map_err(Into::into)
    }

    pub fn rollback_checkpoint(&self) -> Result<RollbackCheckpoint> {
        let target = ProbeTarget::parse(&self.profile.probe)?;
        let store = HardStateStore::open(&self.paths.hard_database)?;
        let snapshot = store
            .host_for_lookup(target.hostname())?
            .ok_or(Error::RollbackDetected("pinned host is absent"))?;
        let authenticated_roots = snapshot
            .merkle_root
            .authenticated_roots
            .iter()
            .map(|root| (root.epoch(), root.root_hash()))
            .collect();
        Ok(RollbackCheckpoint {
            profile: self.profile.name.clone(),
            host_id: snapshot.host_id,
            host_chain_sequence: snapshot.chain_seqno,
            host_chain_tail: snapshot.chain_tail_hash,
            host_chain_bytes: snapshot.chain_bytes,
            merkle_epoch: snapshot.merkle_root.epoch,
            merkle_root_hash: snapshot.merkle_root.root_hash,
            authenticated_roots,
        })
    }

    pub fn create_account(
        &self,
        alias: &str,
        username: &str,
        device_name: &str,
        email: &str,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<SyncReport> {
        self.profile.require(Capability::Signup)?;
        validate_name(alias)?;
        if vault.contains(alias)? {
            return Err(Error::AccountExists);
        }
        let pending = PendingSignup::random(alias, username)?;
        vault.put_pending(&pending)?;
        let host = self.pinned_host()?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let created = self.client.create_software_account(
            &host,
            SoftwareAccountRequest {
                username_utf8: username.to_owned(),
                device_name: device_name.to_owned(),
                invite_code: InviteCode::Empty,
                email: email.to_owned(),
            },
            pending.secrets()?,
            &self.paths.soft_database,
            &mut mutations,
        )?;
        vault.commit_created(alias, username, &created.credential)?;
        Ok(SyncReport::from_created(&created))
    }

    pub fn resume_account(
        &self,
        alias: &str,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<SyncReport> {
        self.profile.require(Capability::Signup)?;
        let pending = vault.pending(alias)?;
        let host = self.pinned_host()?;
        let mut uid =
            derive_shared_verify_key(&SecretSeed::new(pending.puk_seed), ENTITY_PUK_VERIFY)?
                .into_bytes();
        uid[0] = ENTITY_USER;
        let uid = EntityId::from_bytes(uid)?;
        let device = derive_device_public(&SecretSeed::new(pending.device_seed))?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let created = self.client.resume_software_account_for_credential(
            &host,
            &uid,
            &device.id,
            &self.paths.soft_database,
            &mut mutations,
        )?;
        vault.commit_created(alias, &pending.username, &created.credential)?;
        Ok(SyncReport::from_created(&created))
    }

    pub fn list_devices(
        &self,
        alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<Vec<DeviceSummary>> {
        self.profile.require(Capability::DeviceAdministration)?;
        let loaded = vault.account(alias)?;
        let host = self.pinned_host()?;
        let authenticated = self
            .client
            .authenticate_and_pin(&host, &loaded.credential)?;
        Ok(authenticated
            .verified
            .devices()
            .iter()
            .map(|device| DeviceSummary {
                id_hex: hex(device.id.as_bytes()),
                role: format!("{:?}", device.role.kind()).to_ascii_lowercase(),
                current: derive_device_public(&loaded.credential.seed)
                    .is_ok_and(|current| current.id == device.id),
            })
            .collect())
    }

    pub fn provision_owner_device(
        &self,
        source_alias: &str,
        target_alias: &str,
        device_name: &str,
        serial: u64,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<DeviceProvisionReport> {
        self.profile.require(Capability::DeviceAdministration)?;
        validate_name(target_alias)?;
        if vault.contains(target_alias)? {
            return Err(Error::AccountExists);
        }
        let source = vault.account(source_alias)?;
        let pending = PendingDevice::random(source_alias, target_alias, &source.username, serial)?;
        vault.put_pending_device(&pending)?;
        let host = self.pinned_host()?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let provisioned = self.client.provision_software_device(
            &host,
            &source.credential,
            SoftwareDeviceProvisionRequest {
                role: Role::OWNER,
                device_name: device_name.to_owned(),
                serial,
            },
            pending.secrets(),
            &mut mutations,
        )?;
        vault.commit_created(target_alias, &source.username, &provisioned.credential)?;
        vault.remove_pending_device(target_alias)?;
        Ok(DeviceProvisionReport {
            alias: target_alias.to_owned(),
            device_id_hex: hex(derive_device_public(&provisioned.credential.seed)?
                .id
                .as_bytes()),
            user_chain_sequence: provisioned.authenticated.verified.chain_seqno(),
        })
    }

    pub fn resume_owner_device_provision(
        &self,
        target_alias: &str,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<DeviceProvisionReport> {
        self.profile.require(Capability::DeviceAdministration)?;
        let pending = vault.pending_device(target_alias)?;
        let source = vault.account(&pending.source_alias)?;
        let host = self.pinned_host()?;
        let device_seed = SecretSeed::new(pending.device_seed);
        let device = derive_device_public(&device_seed)?;
        let operation = HardStateStore::open(&self.paths.hard_database)?
            .pending_mutations(host.host_id().as_bytes())?
            .into_iter()
            .find(|operation| {
                operation.kind == MutationKind::DeviceProvision
                    && operation.scope_id == source.credential.uid.as_bytes()
                    && operation.subject_id == device.id.as_bytes()
            })
            .ok_or(Error::InvalidAccount(
                "pending device provision has no matching journal operation",
            ))?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let provisioned = self.client.resume_software_device_provision(
            &host,
            &source.credential,
            operation.operation_id,
            device_seed,
            Role::OWNER,
            &mut mutations,
        )?;
        vault.commit_created(target_alias, &pending.username, &provisioned.credential)?;
        vault.remove_pending_device(target_alias)?;
        Ok(DeviceProvisionReport {
            alias: target_alias.to_owned(),
            device_id_hex: hex(device.id.as_bytes()),
            user_chain_sequence: provisioned.authenticated.verified.chain_seqno(),
        })
    }

    /// Generates and durably stores a backup key before submitting enrollment.
    /// The returned phrase should additionally be copied to offline storage.
    pub fn enroll_owner_backup(
        &self,
        account_alias: &str,
        backup_alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<Zeroizing<String>> {
        self.profile.require(Capability::Recovery)?;
        validate_name(backup_alias)?;
        if vault
            .store
            .keys()?
            .iter()
            .any(|key| key == &backup_key(backup_alias))
        {
            return Err(Error::AccountExists);
        }
        let loaded = vault.account(account_alias)?;
        let host = self.pinned_host()?;
        let backup = BackupKey::generate()?;
        let phrase = backup.phrase().expose_joined();
        vault.put_backup(backup_alias, account_alias, &phrase)?;
        self.client
            .enroll_backup_key(&host, &loaded.credential, Role::OWNER, &backup)?;
        Ok(phrase)
    }

    pub fn recover_owner_account(
        &self,
        target_alias: &str,
        phrase: Zeroizing<String>,
        device_name: &str,
        serial: u64,
        vault: &mut AccountVault<'_>,
    ) -> Result<DeviceProvisionReport> {
        self.profile.require(Capability::Recovery)?;
        validate_name(target_alias)?;
        if vault.contains(target_alias)? {
            return Err(Error::AccountExists);
        }
        let pending = PendingRecovery::random(target_alias, serial)?;
        vault.put_pending_recovery(&pending)?;
        self.finish_recovery(pending, phrase, device_name, vault)
    }

    pub fn resume_owner_recovery(
        &self,
        target_alias: &str,
        phrase: Zeroizing<String>,
        device_name: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<DeviceProvisionReport> {
        self.profile.require(Capability::Recovery)?;
        let pending = vault.pending_recovery(target_alias)?;
        self.finish_recovery(pending, phrase, device_name, vault)
    }

    fn finish_recovery(
        &self,
        pending: PendingRecovery,
        phrase: Zeroizing<String>,
        device_name: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<DeviceProvisionReport> {
        let host = self.pinned_host()?;
        let backup = BackupKey::from_phrase(&phrase)?;
        let located = self.client.load_backup_key(&host, backup)?;
        let recovered = self.client.recover_software_device(
            &host,
            located,
            SoftwareDeviceProvisionRequest {
                role: Role::OWNER,
                device_name: device_name.to_owned(),
                serial: pending.serial,
            },
            pending.secrets(),
        )?;
        let username =
            String::from_utf8_lossy(recovered.authenticated.verified.username()).into_owned();
        vault.commit_created(&pending.target_alias, &username, &recovered.credential)?;
        vault.remove_pending_recovery(&pending.target_alias)?;
        Ok(DeviceProvisionReport {
            alias: pending.target_alias.clone(),
            device_id_hex: hex(derive_device_public(&recovered.credential.seed)?
                .id
                .as_bytes()),
            user_chain_sequence: recovered.authenticated.verified.chain_seqno(),
        })
    }

    pub fn create_named_team(
        &self,
        account_alias: &str,
        team_alias: &str,
        team_name: &str,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<TeamSyncReport> {
        self.profile.require(Capability::Teams)?;
        if vault.contains_team(team_alias)? {
            return Err(Error::AccountExists);
        }
        let account = vault.account(account_alias)?;
        let mut stored = StoredTeam::random_named(team_alias, account_alias, team_name)?;
        vault.put_team(&stored)?;
        let host = self.pinned_host()?;
        let secrets = stored.named_secrets()?;
        let created = self.client.create_single_owner_named_team(
            &host,
            &account.credential,
            team_name,
            &secrets,
        )?;
        let report =
            self.ensure_team_root(team_alias, &account, created.authenticated, master_key)?;
        stored.active = true;
        vault.put_team(&stored)?;
        Ok(report)
    }

    pub fn create_adhoc_team(
        &self,
        account_alias: &str,
        team_alias: &str,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<TeamSyncReport> {
        self.profile.require(Capability::Teams)?;
        if vault.contains_team(team_alias)? {
            return Err(Error::AccountExists);
        }
        let account = vault.account(account_alias)?;
        let mut stored = StoredTeam::random_adhoc(team_alias, account_alias)?;
        vault.put_team(&stored)?;
        let host = self.pinned_host()?;
        let secrets = stored.adhoc_secrets()?;
        let created =
            self.client
                .create_single_owner_adhoc_team(&host, &account.credential, &secrets)?;
        let report =
            self.ensure_team_root(team_alias, &account, created.authenticated, master_key)?;
        stored.active = true;
        vault.put_team(&stored)?;
        Ok(report)
    }

    pub fn resume_team_creation(
        &self,
        team_alias: &str,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<TeamSyncReport> {
        self.profile.require(Capability::Teams)?;
        let mut stored = vault.team(team_alias)?;
        let account = vault.account(&stored.account_alias)?;
        let host = self.pinned_host()?;
        let authenticated = match stored.kind {
            StoredTeamKind::Named => {
                self.client
                    .resume_single_owner_named_team(
                        &host,
                        &account.credential,
                        stored
                            .name
                            .as_deref()
                            .ok_or(Error::InvalidAccount("named team has no stored name"))?,
                        &stored.named_secrets()?,
                    )?
                    .authenticated
            }
            StoredTeamKind::AdHoc => {
                self.client
                    .resume_single_owner_adhoc_team(
                        &host,
                        &account.credential,
                        &stored.adhoc_secrets()?,
                    )?
                    .authenticated
            }
        };
        let report = self.ensure_team_root(team_alias, &account, authenticated, master_key)?;
        stored.active = true;
        vault.put_team(&stored)?;
        Ok(report)
    }

    pub fn list_teams(&self, vault: &mut AccountVault<'_>) -> Result<Vec<TeamSummary>> {
        self.profile.require(Capability::Teams)?;
        vault
            .team_aliases()?
            .into_iter()
            .map(|alias| {
                let team = vault.team(&alias)?;
                Ok(TeamSummary {
                    alias,
                    account_alias: team.account_alias.clone(),
                    team_id_hex: hex(&team.team_id),
                    kind: match team.kind {
                        StoredTeamKind::Named => "named",
                        StoredTeamKind::AdHoc => "ad-hoc",
                    }
                    .to_owned(),
                    name: team.name.clone(),
                    active: team.active,
                })
            })
            .collect()
    }

    pub fn sync_team(
        &self,
        team_alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<TeamSyncReport> {
        self.profile.require(Capability::Teams)?;
        self.profile.require(Capability::Kv)?;
        let stored = vault.team(team_alias)?;
        if !stored.active {
            return Err(Error::InvalidAccount("team creation is still pending"));
        }
        let account = vault.account(&stored.account_alias)?;
        let host = self.pinned_host()?;
        let user = self
            .client
            .authenticate_and_pin(&host, &account.credential)?;
        let team_id = EntityId::from_bytes(stored.team_id.clone())?;
        let team = self.client.load_and_pin_team(
            &host,
            &account.credential,
            &user.verified,
            &user.puks,
            &team_id,
        )?;
        let tree = self.client.sync_team_kv(
            &host,
            &account.credential,
            &team,
            &self.paths.soft_database,
        )?;
        Ok(TeamSyncReport::new(team_alias, &team, &tree))
    }

    fn ensure_team_root(
        &self,
        team_alias: &str,
        account: &LoadedAccount,
        authenticated: foks_client::AuthenticatedTeamOutcome,
        master_key: &[u8; 32],
    ) -> Result<TeamSyncReport> {
        let host = self.pinned_host()?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let mut session = self.client.team_kv_write_session(
            &host,
            &account.credential,
            &authenticated,
            &self.paths.soft_database,
            &mut mutations,
        )?;
        let tree = session.ensure_root(Role::OWNER, Role::OWNER)?;
        Ok(TeamSyncReport::new(team_alias, &authenticated, &tree))
    }

    pub fn sync_account(&self, alias: &str, vault: &mut AccountVault<'_>) -> Result<SyncReport> {
        self.profile.require(Capability::UserSync)?;
        self.profile.require(Capability::Kv)?;
        let (_, authenticated, directories) = self.authenticated_tree(alias, vault)?;
        Ok(SyncReport::from_tree(
            authenticated.verified.username(),
            authenticated.verified.chain_seqno(),
            &directories,
        ))
    }

    pub fn list_kv(&self, alias: &str, vault: &mut AccountVault<'_>) -> Result<KvListReport> {
        self.profile.require(Capability::Kv)?;
        let (_, authenticated, directories) = self.authenticated_tree(alias, vault)?;
        Ok(KvListReport {
            sync: SyncReport::from_tree(
                authenticated.verified.username(),
                authenticated.verified.chain_seqno(),
                &directories,
            ),
            entries: flatten_tree(&directories)?,
        })
    }

    pub fn read_kv_file(
        &self,
        alias: &str,
        path: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<Vec<u8>> {
        let mut output = Vec::new();
        self.write_kv_file(alias, path, vault, &mut output)?;
        Ok(output)
    }

    /// Streams one authenticated KV file to the caller without buffering a
    /// large-file projection in application memory.
    pub fn write_kv_file<W: Write>(
        &self,
        alias: &str,
        path: &str,
        vault: &mut AccountVault<'_>,
        writer: &mut W,
    ) -> Result<u64> {
        self.profile.require(Capability::Kv)?;
        let (loaded, _, directories) = self.authenticated_tree(alias, vault)?;
        let entry = resolve_entry(&directories, path)?;
        match KvNodeId(entry.node_id).node_type()? {
            KvNodeType::SmallFile => {
                let content = entry.content.as_ref().ok_or(Error::InvalidAccount(
                    "small-file projection has no content",
                ))?;
                writer.write_all(content)?;
                u64::try_from(content.len())
                    .map_err(|_| Error::InvalidAccount("small-file size overflow"))
            }
            KvNodeType::File => {
                let host = self.pinned_host()?;
                let store = SoftStateStore::open(&self.paths.soft_database)?;
                store
                    .write_large_file(
                        host.host_id().as_bytes(),
                        loaded.credential.uid.as_bytes(),
                        &entry.node_id,
                        writer,
                    )?
                    .ok_or(Error::InvalidAccount("large-file projection is incomplete"))
            }
            _ => Err(Error::InvalidAccount("KV path is not a file")),
        }
    }

    pub fn put_kv_file<R: Read>(
        &self,
        alias: &str,
        path: &str,
        reader: &mut R,
        overwrite: bool,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<KvWriteReport> {
        self.profile.require(Capability::Kv)?;
        let (parent_path, name) = split_parent(path)?;
        let loaded = vault.account(alias)?;
        let host = self.pinned_host()?;
        let authenticated = self
            .client
            .authenticate_and_pin(&host, &loaded.credential)?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let mut session = self.client.user_kv_write_session(
            &host,
            &loaded.credential,
            &authenticated.verified,
            &authenticated.puks,
            &self.paths.soft_database,
            &mut mutations,
        )?;
        let tree = session.sync()?;
        let parent = resolve_directory(&tree, &parent_path)?;
        let result = session.put_file(
            parent,
            &name,
            reader,
            KvWriteOptions {
                read_role: Role::OWNER,
                write_role: Role::OWNER,
                overwrite,
                expected_version: None,
            },
        )?;
        Ok(KvWriteReport::from_tree(
            path,
            result.dirent_version,
            &result.tree,
        ))
    }

    pub fn mkdir_kv(
        &self,
        alias: &str,
        path: &str,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<KvWriteReport> {
        self.profile.require(Capability::Kv)?;
        let (parent_path, name) = split_parent(path)?;
        let loaded = vault.account(alias)?;
        let host = self.pinned_host()?;
        let authenticated = self
            .client
            .authenticate_and_pin(&host, &loaded.credential)?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let mut session = self.client.user_kv_write_session(
            &host,
            &loaded.credential,
            &authenticated.verified,
            &authenticated.puks,
            &self.paths.soft_database,
            &mut mutations,
        )?;
        let tree = session.sync()?;
        let parent = resolve_directory(&tree, &parent_path)?;
        let result = session.mkdir(
            parent,
            &name,
            KvWriteOptions {
                read_role: Role::OWNER,
                write_role: Role::OWNER,
                overwrite: false,
                expected_version: None,
            },
        )?;
        Ok(KvWriteReport::from_tree(
            path,
            result.dirent_version,
            &result.tree,
        ))
    }

    pub fn remove_kv(
        &self,
        alias: &str,
        path: &str,
        recursive: bool,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<KvWriteReport> {
        self.profile.require(Capability::Kv)?;
        let (parent_path, name) = split_parent(path)?;
        let loaded = vault.account(alias)?;
        let host = self.pinned_host()?;
        let authenticated = self
            .client
            .authenticate_and_pin(&host, &loaded.credential)?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let mut session = self.client.user_kv_write_session(
            &host,
            &loaded.credential,
            &authenticated.verified,
            &authenticated.puks,
            &self.paths.soft_database,
            &mut mutations,
        )?;
        let tree = session.sync()?;
        let parent = resolve_directory(&tree, &parent_path)?;
        let existing = resolve_entry(&tree, path)?;
        let version = existing
            .version
            .checked_add(1)
            .ok_or(Error::InvalidAccount("KV version overflow"))?;
        let tree = session.unlink(
            parent,
            &name,
            Some(existing.version),
            Role::OWNER,
            recursive,
        )?;
        Ok(KvWriteReport::from_tree(path, version, &tree))
    }

    pub fn schedule_user_refresh(
        &self,
        alias: &str,
        interval_micros: u64,
        first_run_at: u64,
        vault: &mut AccountVault<'_>,
    ) -> Result<[u8; 16]> {
        self.profile.require(Capability::UserSync)?;
        if interval_micros == 0 {
            return Err(Error::InvalidConfig("job interval is zero"));
        }
        let loaded = vault.account(alias)?;
        let host = self.pinned_host()?;
        let mut binding = Vec::with_capacity(66);
        binding.extend_from_slice(host.host_id().as_bytes());
        binding.extend_from_slice(loaded.credential.uid.as_bytes());
        let digest = prefixed_hash(USER_REFRESH_JOB_TYPE_ID, &binding);
        let job_id = digest[..16]
            .try_into()
            .expect("hash prefix has fixed length");
        FoksScheduler::new(&self.paths.hard_database, SchedulerConfig::default())?.register(
            ScheduledJobRegistration {
                job_id,
                kind: ScheduledJobKind::UserRefresh,
                host_id: host.host_id().as_bytes().to_vec(),
                scope_id: loaded.credential.uid.as_bytes().to_vec(),
                interval_micros,
                first_run_at,
                registered_at: now_microseconds()?,
            },
        )?;
        Ok(job_id)
    }

    pub fn run_due_jobs(&self, now: u64, vault: &mut AccountVault<'_>) -> Result<JobRunReport> {
        self.profile.require(Capability::UserSync)?;
        let scheduler = FoksScheduler::new(&self.paths.hard_database, SchedulerConfig::default())?;
        let report = scheduler.run_due(now, |job| {
            if job.kind != ScheduledJobKind::UserRefresh {
                return Err("unsupported application job kind".to_owned());
            }
            let aliases = vault.aliases().map_err(|error| error.to_string())?;
            let alias = aliases
                .into_iter()
                .find(|alias| {
                    vault
                        .account(alias)
                        .is_ok_and(|account| account.credential.uid.as_bytes() == job.scope_id)
                })
                .ok_or_else(|| "scheduled account credential is unavailable".to_owned())?;
            self.sync_account(&alias, vault)
                .map(|_| ())
                .map_err(|error| error.to_string())
        })?;
        Ok(JobRunReport {
            runs: report
                .runs
                .into_iter()
                .map(|run| {
                    let (completed, error) = match run.status {
                        foks_client::ScheduledRunStatus::Completed => (true, None),
                        foks_client::ScheduledRunStatus::Failed { error } => (false, Some(error)),
                    };
                    JobRun {
                        job_id_hex: hex(&run.job_id),
                        completed,
                        error,
                        next_run_at: run.next_run_at,
                    }
                })
                .collect(),
        })
    }

    fn authenticated_tree(
        &self,
        alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<(
        LoadedAccount,
        foks_client::AuthenticatedUserOutcome,
        Vec<KvDirectoryProjection>,
    )> {
        let loaded = vault.account(alias)?;
        let host = self.pinned_host()?;
        let authenticated = self
            .client
            .authenticate_and_pin(&host, &loaded.credential)?;
        let directories = self.client.sync_user_kv(
            &host,
            &loaded.credential,
            &authenticated.verified,
            &authenticated.puks,
            &self.paths.soft_database,
        )?;
        Ok((loaded, authenticated, directories))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SyncReport {
    pub username: String,
    pub user_chain_sequence: u64,
    pub directories: usize,
    pub entries: usize,
}

impl SyncReport {
    fn from_created(created: &foks_client::CreatedSoftwareAccount) -> Self {
        Self::from_tree(
            created.authenticated.verified.username(),
            created.authenticated.verified.chain_seqno(),
            &created.kv_projection,
        )
    }

    fn from_tree(username: &[u8], chain: u64, directories: &[KvDirectoryProjection]) -> Self {
        Self {
            username: String::from_utf8_lossy(username).into_owned(),
            user_chain_sequence: chain,
            directories: directories.len(),
            entries: directories
                .iter()
                .map(|directory| directory.entries.len())
                .sum(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct KvEntrySummary {
    pub path: String,
    pub node_type: String,
    pub version: u64,
    pub size: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct KvListReport {
    pub sync: SyncReport,
    pub entries: Vec<KvEntrySummary>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct KvWriteReport {
    pub path: String,
    pub version: u64,
    pub directories: usize,
    pub entries: usize,
}

impl KvWriteReport {
    fn from_tree(path: &str, version: u64, tree: &[KvDirectoryProjection]) -> Self {
        Self {
            path: path.to_owned(),
            version,
            directories: tree.len(),
            entries: tree.iter().map(|directory| directory.entries.len()).sum(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct JobRunReport {
    pub runs: Vec<JobRun>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct JobRun {
    pub job_id_hex: String,
    pub completed: bool,
    pub error: Option<String>,
    pub next_run_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeviceSummary {
    pub id_hex: String,
    pub role: String,
    pub current: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeviceProvisionReport {
    pub alias: String,
    pub device_id_hex: String,
    pub user_chain_sequence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TeamSummary {
    pub alias: String,
    pub account_alias: String,
    pub team_id_hex: String,
    pub kind: String,
    pub name: Option<String>,
    pub active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TeamSyncReport {
    pub alias: String,
    pub team_id_hex: String,
    pub team_chain_sequence: u64,
    pub directories: usize,
    pub entries: usize,
}

impl TeamSyncReport {
    fn new(
        alias: &str,
        team: &foks_client::AuthenticatedTeamOutcome,
        tree: &[KvDirectoryProjection],
    ) -> Self {
        Self {
            alias: alias.to_owned(),
            team_id_hex: hex(team.verified.team().as_bytes()),
            team_chain_sequence: team.verified.chain_seqno(),
            directories: tree.len(),
            entries: tree.iter().map(|directory| directory.entries.len()).sum(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct StoredAccount {
    version: u32,
    alias: String,
    username: String,
    uid: Vec<u8>,
    device_seed: [u8; 32],
    certificate_chain: Vec<Vec<u8>>,
}

impl Drop for StoredAccount {
    fn drop(&mut self) {
        self.device_seed.zeroize();
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct PendingSignup {
    version: u32,
    alias: String,
    username: String,
    device_seed: [u8; 32],
    puk_seed: [u8; 32],
    self_token: [u8; 17],
}

impl Drop for PendingSignup {
    fn drop(&mut self) {
        self.device_seed.zeroize();
        self.puk_seed.zeroize();
        self.self_token.zeroize();
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct PendingDevice {
    version: u32,
    source_alias: String,
    target_alias: String,
    username: String,
    serial: u64,
    device_seed: [u8; 32],
    self_token: [u8; 17],
}

impl PendingDevice {
    fn random(source_alias: &str, target_alias: &str, username: &str, serial: u64) -> Result<Self> {
        if serial == 0 {
            return Err(Error::InvalidAccount("device serial is zero"));
        }
        let mut device_seed = [0u8; 32];
        let mut self_token = [0u8; 17];
        getrandom::fill(&mut device_seed).map_err(|_| Error::Randomness)?;
        getrandom::fill(&mut self_token).map_err(|_| Error::Randomness)?;
        self_token[0] = 54;
        Ok(Self {
            version: CREDENTIAL_VERSION,
            source_alias: source_alias.to_owned(),
            target_alias: target_alias.to_owned(),
            username: username.to_owned(),
            serial,
            device_seed,
            self_token,
        })
    }

    fn secrets(&self) -> NewSoftwareDeviceSecrets {
        NewSoftwareDeviceSecrets::new(SecretSeed::new(self.device_seed), None, self.self_token)
    }
}

impl Drop for PendingDevice {
    fn drop(&mut self) {
        self.device_seed.zeroize();
        self.self_token.zeroize();
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct PendingRecovery {
    version: u32,
    target_alias: String,
    serial: u64,
    device_seed: [u8; 32],
    self_token: [u8; 17],
}

impl PendingRecovery {
    fn random(target_alias: &str, serial: u64) -> Result<Self> {
        if serial == 0 {
            return Err(Error::InvalidAccount("recovery device serial is zero"));
        }
        let mut device_seed = [0u8; 32];
        let mut self_token = [0u8; 17];
        getrandom::fill(&mut device_seed).map_err(|_| Error::Randomness)?;
        getrandom::fill(&mut self_token).map_err(|_| Error::Randomness)?;
        self_token[0] = 54;
        Ok(Self {
            version: CREDENTIAL_VERSION,
            target_alias: target_alias.to_owned(),
            serial,
            device_seed,
            self_token,
        })
    }

    fn secrets(&self) -> NewSoftwareDeviceSecrets {
        NewSoftwareDeviceSecrets::new(SecretSeed::new(self.device_seed), None, self.self_token)
    }
}

impl Drop for PendingRecovery {
    fn drop(&mut self) {
        self.device_seed.zeroize();
        self.self_token.zeroize();
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct StoredBackup {
    version: u32,
    backup_alias: String,
    account_alias: String,
    phrase: String,
}

impl Drop for StoredBackup {
    fn drop(&mut self) {
        self.phrase.zeroize();
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum StoredTeamKind {
    Named,
    AdHoc,
}

#[derive(Debug, Deserialize, Serialize)]
struct StoredTeam {
    version: u32,
    alias: String,
    account_alias: String,
    kind: StoredTeamKind,
    name: Option<String>,
    team_id: Vec<u8>,
    member_min: [u8; 32],
    member: [u8; 32],
    admin: [u8; 32],
    owner: [u8; 32],
    removal_key: Option<[u8; 32]>,
    name_commitment: Option<[u8; 16]>,
    active: bool,
}

impl StoredTeam {
    fn random_named(alias: &str, account_alias: &str, name: &str) -> Result<Self> {
        if name.trim().is_empty() || name.len() > 256 {
            return Err(Error::InvalidAccount("team name is missing or excessive"));
        }
        let mut stored = Self::random(alias, account_alias, StoredTeamKind::Named)?;
        stored.name = Some(name.to_owned());
        stored.removal_key = Some(random_array()?);
        stored.name_commitment = Some(random_array()?);
        stored.team_id = stored.named_secrets()?.team_id()?.into_bytes();
        Ok(stored)
    }

    fn random_adhoc(alias: &str, account_alias: &str) -> Result<Self> {
        let mut stored = Self::random(alias, account_alias, StoredTeamKind::AdHoc)?;
        stored.team_id = stored.adhoc_secrets()?.team_id()?.into_bytes();
        Ok(stored)
    }

    fn random(alias: &str, account_alias: &str, kind: StoredTeamKind) -> Result<Self> {
        validate_name(alias)?;
        validate_name(account_alias)?;
        Ok(Self {
            version: CREDENTIAL_VERSION,
            alias: alias.to_owned(),
            account_alias: account_alias.to_owned(),
            kind,
            name: None,
            team_id: Vec::new(),
            member_min: random_array()?,
            member: random_array()?,
            admin: random_array()?,
            owner: random_array()?,
            removal_key: None,
            name_commitment: None,
            active: false,
        })
    }

    fn named_secrets(&self) -> Result<NamedTeamSecrets> {
        if self.kind != StoredTeamKind::Named {
            return Err(Error::InvalidAccount("team is not named"));
        }
        Ok(NamedTeamSecrets {
            member_min: SecretSeed::new(self.member_min),
            member: SecretSeed::new(self.member),
            admin: SecretSeed::new(self.admin),
            owner: SecretSeed::new(self.owner),
            removal_key: SecretSeed::new(
                self.removal_key
                    .ok_or(Error::InvalidAccount("named team has no removal key"))?,
            ),
            team_name_commitment_key: self.name_commitment.ok_or(Error::InvalidAccount(
                "named team has no name commitment key",
            ))?,
        })
    }

    fn adhoc_secrets(&self) -> Result<AdHocTeamSecrets> {
        if self.kind != StoredTeamKind::AdHoc {
            return Err(Error::InvalidAccount("team is not ad-hoc"));
        }
        Ok(AdHocTeamSecrets {
            member_min: SecretSeed::new(self.member_min),
            member: SecretSeed::new(self.member),
            admin: SecretSeed::new(self.admin),
            owner: SecretSeed::new(self.owner),
        })
    }
}

impl Drop for StoredTeam {
    fn drop(&mut self) {
        self.member_min.zeroize();
        self.member.zeroize();
        self.admin.zeroize();
        self.owner.zeroize();
        self.removal_key.zeroize();
        self.name_commitment.zeroize();
    }
}

impl PendingSignup {
    fn random(alias: &str, username: &str) -> Result<Self> {
        let mut device_seed = [0u8; 32];
        let mut puk_seed = [0u8; 32];
        let mut self_token = [0u8; 17];
        getrandom::fill(&mut device_seed).map_err(|_| Error::Randomness)?;
        getrandom::fill(&mut puk_seed).map_err(|_| Error::Randomness)?;
        getrandom::fill(&mut self_token).map_err(|_| Error::Randomness)?;
        self_token[0] = 54;
        Ok(Self {
            version: CREDENTIAL_VERSION,
            alias: alias.to_owned(),
            username: username.to_owned(),
            device_seed,
            puk_seed,
            self_token,
        })
    }

    fn secrets(&self) -> Result<SoftwareAccountSecrets> {
        validate_pending(self)?;
        Ok(SoftwareAccountSecrets::new(
            SecretSeed::new(self.device_seed),
            SecretSeed::new(self.puk_seed),
            self.self_token,
        ))
    }
}

pub struct LoadedAccount {
    pub alias: String,
    pub username: String,
    pub credential: DeviceCredential,
}

pub struct AccountVault<'a> {
    store: &'a mut dyn SecretStore,
}

impl<'a> AccountVault<'a> {
    pub fn new(store: &'a mut dyn SecretStore) -> Self {
        Self { store }
    }

    pub fn aliases(&mut self) -> Result<Vec<String>> {
        Ok(self
            .store
            .keys()?
            .into_iter()
            .filter_map(|key| key.strip_prefix("account.").map(str::to_owned))
            .collect())
    }

    pub fn team_aliases(&mut self) -> Result<Vec<String>> {
        Ok(self
            .store
            .keys()?
            .into_iter()
            .filter_map(|key| key.strip_prefix("team.").map(str::to_owned))
            .collect())
    }

    pub fn contains(&mut self, alias: &str) -> Result<bool> {
        validate_name(alias)?;
        Ok(self
            .store
            .keys()?
            .iter()
            .any(|key| key == &account_key(alias)))
    }

    fn contains_team(&mut self, alias: &str) -> Result<bool> {
        validate_name(alias)?;
        Ok(self.store.keys()?.iter().any(|key| key == &team_key(alias)))
    }

    fn team(&mut self, alias: &str) -> Result<StoredTeam> {
        validate_name(alias)?;
        let bytes = self
            .store
            .get(&team_key(alias))
            .map_err(|error| match error {
                foks_keystore::Error::Missing => Error::AccountMissing,
                other => Error::Keystore(other),
            })?;
        let team: StoredTeam = serde_json::from_slice(&bytes)?;
        validate_stored_team(&team, alias)?;
        Ok(team)
    }

    fn put_team(&mut self, team: &StoredTeam) -> Result<()> {
        validate_stored_team(team, &team.alias)?;
        let encoded = Zeroizing::new(serde_json::to_vec(team)?);
        self.store.put(&team_key(&team.alias), &encoded)?;
        Ok(())
    }

    pub fn account(&mut self, alias: &str) -> Result<LoadedAccount> {
        validate_name(alias)?;
        let bytes = self
            .store
            .get(&account_key(alias))
            .map_err(|error| match error {
                foks_keystore::Error::Missing => Error::AccountMissing,
                other => Error::Keystore(other),
            })?;
        let stored: StoredAccount = serde_json::from_slice(&bytes)?;
        validate_account(&stored, alias)?;
        Ok(LoadedAccount {
            alias: stored.alias.clone(),
            username: stored.username.clone(),
            credential: DeviceCredential {
                uid: EntityId::from_bytes(stored.uid.clone())?,
                seed: SecretSeed::new(stored.device_seed),
                certificate_chain: stored.certificate_chain.clone(),
            },
        })
    }

    fn pending(&mut self, alias: &str) -> Result<PendingSignup> {
        validate_name(alias)?;
        let bytes = self
            .store
            .get(&pending_key(alias))
            .map_err(|error| match error {
                foks_keystore::Error::Missing => Error::AccountMissing,
                other => Error::Keystore(other),
            })?;
        let pending: PendingSignup = serde_json::from_slice(&bytes)?;
        validate_pending(&pending)?;
        if pending.alias != alias {
            return Err(Error::InvalidAccount("pending alias binding changed"));
        }
        Ok(pending)
    }

    fn put_pending(&mut self, pending: &PendingSignup) -> Result<()> {
        validate_pending(pending)?;
        let encoded = Zeroizing::new(serde_json::to_vec(pending)?);
        self.store.put(&pending_key(&pending.alias), &encoded)?;
        Ok(())
    }

    fn pending_device(&mut self, target_alias: &str) -> Result<PendingDevice> {
        validate_name(target_alias)?;
        let bytes =
            self.store
                .get(&pending_device_key(target_alias))
                .map_err(|error| match error {
                    foks_keystore::Error::Missing => Error::AccountMissing,
                    other => Error::Keystore(other),
                })?;
        let pending: PendingDevice = serde_json::from_slice(&bytes)?;
        validate_pending_device(&pending)?;
        if pending.target_alias != target_alias {
            return Err(Error::InvalidAccount(
                "pending device alias binding changed",
            ));
        }
        Ok(pending)
    }

    fn put_pending_device(&mut self, pending: &PendingDevice) -> Result<()> {
        validate_pending_device(pending)?;
        let encoded = Zeroizing::new(serde_json::to_vec(pending)?);
        self.store
            .put(&pending_device_key(&pending.target_alias), &encoded)?;
        Ok(())
    }

    fn remove_pending_device(&mut self, target_alias: &str) -> Result<()> {
        self.store.remove(&pending_device_key(target_alias))?;
        Ok(())
    }

    fn pending_recovery(&mut self, target_alias: &str) -> Result<PendingRecovery> {
        validate_name(target_alias)?;
        let bytes = self
            .store
            .get(&pending_recovery_key(target_alias))
            .map_err(|error| match error {
                foks_keystore::Error::Missing => Error::AccountMissing,
                other => Error::Keystore(other),
            })?;
        let pending: PendingRecovery = serde_json::from_slice(&bytes)?;
        validate_pending_recovery(&pending)?;
        if pending.target_alias != target_alias {
            return Err(Error::InvalidAccount(
                "pending recovery alias binding changed",
            ));
        }
        Ok(pending)
    }

    fn put_pending_recovery(&mut self, pending: &PendingRecovery) -> Result<()> {
        validate_pending_recovery(pending)?;
        let encoded = Zeroizing::new(serde_json::to_vec(pending)?);
        self.store
            .put(&pending_recovery_key(&pending.target_alias), &encoded)?;
        Ok(())
    }

    fn remove_pending_recovery(&mut self, target_alias: &str) -> Result<()> {
        self.store.remove(&pending_recovery_key(target_alias))?;
        Ok(())
    }

    fn put_backup(&mut self, backup_alias: &str, account_alias: &str, phrase: &str) -> Result<()> {
        validate_name(backup_alias)?;
        validate_name(account_alias)?;
        if phrase.len() > 1024 || phrase.split_whitespace().count() != 17 {
            return Err(Error::InvalidAccount("backup phrase is malformed"));
        }
        let backup = StoredBackup {
            version: CREDENTIAL_VERSION,
            backup_alias: backup_alias.to_owned(),
            account_alias: account_alias.to_owned(),
            phrase: phrase.to_owned(),
        };
        let encoded = Zeroizing::new(serde_json::to_vec(&backup)?);
        self.store.put(&backup_key(backup_alias), &encoded)?;
        Ok(())
    }

    fn commit_created(
        &mut self,
        alias: &str,
        username: &str,
        credential: &DeviceCredential,
    ) -> Result<()> {
        let stored = StoredAccount {
            version: CREDENTIAL_VERSION,
            alias: alias.to_owned(),
            username: username.to_owned(),
            uid: credential.uid.as_bytes().to_vec(),
            device_seed: *credential.seed.as_bytes(),
            certificate_chain: credential.certificate_chain.clone(),
        };
        validate_account(&stored, alias)?;
        let encoded = Zeroizing::new(serde_json::to_vec(&stored)?);
        self.store.put(&account_key(alias), &encoded)?;
        self.store.remove(&pending_key(alias))?;
        Ok(())
    }
}

pub fn derive_vault_key(master_key: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    Zeroizing::new(prefixed_hash(VAULT_KEY_TYPE_ID, master_key))
}

pub fn derive_mutation_key(master_key: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    Zeroizing::new(prefixed_hash(MUTATION_KEY_TYPE_ID, master_key))
}

fn validate_account(account: &StoredAccount, expected_alias: &str) -> Result<()> {
    if account.version != CREDENTIAL_VERSION || account.alias != expected_alias {
        return Err(Error::InvalidAccount("version or alias binding changed"));
    }
    validate_name(&account.alias)?;
    if account.username.is_empty() || account.username.len() > 256 {
        return Err(Error::InvalidAccount("username is missing or excessive"));
    }
    EntityId::from_bytes(account.uid.clone())?.require_type(ENTITY_USER)?;
    validate_certificates(&account.certificate_chain)
}

fn validate_pending(pending: &PendingSignup) -> Result<()> {
    if pending.version != CREDENTIAL_VERSION || pending.self_token[0] != 54 {
        return Err(Error::InvalidAccount(
            "pending signup version or token is invalid",
        ));
    }
    validate_name(&pending.alias)?;
    if pending.username.is_empty() || pending.username.len() > 256 {
        return Err(Error::InvalidAccount(
            "pending username is missing or excessive",
        ));
    }
    Ok(())
}

fn validate_pending_device(pending: &PendingDevice) -> Result<()> {
    if pending.version != CREDENTIAL_VERSION || pending.self_token[0] != 54 || pending.serial == 0 {
        return Err(Error::InvalidAccount(
            "pending device version, token, or serial is invalid",
        ));
    }
    validate_name(&pending.source_alias)?;
    validate_name(&pending.target_alias)?;
    if pending.username.is_empty() || pending.username.len() > 256 {
        return Err(Error::InvalidAccount(
            "pending device username is missing or excessive",
        ));
    }
    Ok(())
}

fn validate_pending_recovery(pending: &PendingRecovery) -> Result<()> {
    if pending.version != CREDENTIAL_VERSION || pending.self_token[0] != 54 || pending.serial == 0 {
        return Err(Error::InvalidAccount(
            "pending recovery version, token, or serial is invalid",
        ));
    }
    validate_name(&pending.target_alias)
}

fn validate_certificates(certificates: &[Vec<u8>]) -> Result<()> {
    if certificates.is_empty()
        || certificates.len() > MAX_CERTIFICATES
        || certificates
            .iter()
            .any(|certificate| certificate.is_empty() || certificate.len() > MAX_CERTIFICATE_BYTES)
    {
        return Err(Error::InvalidAccount(
            "certificate chain is invalid or excessive",
        ));
    }
    Ok(())
}

fn validate_stored_team(team: &StoredTeam, expected_alias: &str) -> Result<()> {
    if team.version != CREDENTIAL_VERSION || team.alias != expected_alias {
        return Err(Error::InvalidAccount(
            "team version or alias binding changed",
        ));
    }
    validate_name(&team.alias)?;
    validate_name(&team.account_alias)?;
    let id = EntityId::from_bytes(team.team_id.clone())?;
    let derived = match team.kind {
        StoredTeamKind::Named => {
            if team.name.as_deref().is_none_or(str::is_empty)
                || team.removal_key.is_none()
                || team.name_commitment.is_none()
            {
                return Err(Error::InvalidAccount("named team material is incomplete"));
            }
            team.named_secrets()?.team_id()?
        }
        StoredTeamKind::AdHoc => {
            if team.name.is_some() || team.removal_key.is_some() || team.name_commitment.is_some() {
                return Err(Error::InvalidAccount("ad-hoc team has named-team material"));
            }
            team.adhoc_secrets()?.team_id()?
        }
    };
    if id != derived {
        return Err(Error::InvalidAccount(
            "team ID does not match protected PTKs",
        ));
    }
    Ok(())
}

fn account_key(alias: &str) -> String {
    format!("account.{alias}")
}

fn pending_key(alias: &str) -> String {
    format!("pending.{alias}")
}

fn pending_device_key(alias: &str) -> String {
    format!("pending-device.{alias}")
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

fn path_components(path: &str) -> Result<Vec<&str>> {
    if !path.starts_with('/') || path.len() > 4096 {
        return Err(Error::InvalidKvPath(
            "path must be absolute and at most 4096 bytes",
        ));
    }
    if path == "/" {
        return Ok(Vec::new());
    }
    let components = path[1..].split('/').collect::<Vec<_>>();
    if components.iter().any(|component| {
        component.is_empty() || *component == "." || *component == ".." || component.len() > 255
    }) {
        return Err(Error::InvalidKvPath(
            "path contains an empty, relative, or excessive component",
        ));
    }
    Ok(components)
}

fn split_parent(path: &str) -> Result<(String, String)> {
    let components = path_components(path)?;
    let (name, parents) = components.split_last().ok_or(Error::InvalidKvPath(
        "the KV root cannot be mutated as an entry",
    ))?;
    let parent = if parents.is_empty() {
        "/".to_owned()
    } else {
        format!("/{}", parents.join("/"))
    };
    Ok((parent, (*name).to_owned()))
}

fn resolve_directory(tree: &[KvDirectoryProjection], path: &str) -> Result<[u8; 16]> {
    let components = path_components(path)?;
    let root = tree
        .first()
        .ok_or(Error::InvalidKvPath("KV projection has no root"))?
        .root_directory_id;
    let mut current = root;
    for component in components {
        let directory = tree
            .iter()
            .find(|directory| directory.directory_id == current)
            .ok_or(Error::InvalidKvPath(
                "parent directory is absent from the projection",
            ))?;
        let entry = directory
            .entries
            .iter()
            .find(|entry| entry.name == component.as_bytes())
            .ok_or(Error::InvalidKvPath("directory component does not exist"))?;
        let node = KvNodeId(entry.node_id);
        if node.node_type()? != KvNodeType::Directory {
            return Err(Error::InvalidKvPath("path component is not a directory"));
        }
        current = node.object_id();
    }
    Ok(current)
}

fn resolve_entry<'a>(
    tree: &'a [KvDirectoryProjection],
    path: &str,
) -> Result<&'a foks_client_db::KvProjectedEntry> {
    let (parent_path, name) = split_parent(path)?;
    let parent = resolve_directory(tree, &parent_path)?;
    tree.iter()
        .find(|directory| directory.directory_id == parent)
        .and_then(|directory| {
            directory
                .entries
                .iter()
                .find(|entry| entry.name == name.as_bytes())
        })
        .ok_or(Error::InvalidKvPath("KV entry does not exist"))
}

fn flatten_tree(tree: &[KvDirectoryProjection]) -> Result<Vec<KvEntrySummary>> {
    use std::collections::{BTreeSet, VecDeque};

    let root = tree
        .first()
        .ok_or(Error::InvalidKvPath("KV projection has no root"))?
        .root_directory_id;
    let mut pending = VecDeque::from([(root, String::new())]);
    let mut visited = BTreeSet::new();
    let mut output = Vec::new();
    while let Some((directory_id, parent_path)) = pending.pop_front() {
        if !visited.insert(directory_id) {
            return Err(Error::InvalidKvPath(
                "KV projection contains a directory cycle",
            ));
        }
        let directory = tree
            .iter()
            .find(|directory| directory.directory_id == directory_id)
            .ok_or(Error::InvalidKvPath(
                "KV projection omits a reachable directory",
            ))?;
        let mut entries = directory.entries.iter().collect::<Vec<_>>();
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        for entry in entries {
            let name = display_component(&entry.name);
            let path = format!("{parent_path}/{name}");
            let node_type = KvNodeId(entry.node_id).node_type()?;
            let size = match node_type {
                KvNodeType::SmallFile => entry.content.as_ref().map(|content| content.len() as u64),
                KvNodeType::File => entry.large_file_size,
                KvNodeType::Symlink => entry.symlink.as_ref().map(|target| target.len() as u64),
                _ => None,
            };
            output.push(KvEntrySummary {
                path: path.clone(),
                node_type: match node_type {
                    KvNodeType::None => "none",
                    KvNodeType::Directory => "directory",
                    KvNodeType::File => "file",
                    KvNodeType::SmallFile => "small-file",
                    KvNodeType::Symlink => "symlink",
                }
                .to_owned(),
                version: entry.version,
                size,
            });
            if node_type == KvNodeType::Directory {
                pending.push_back((KvNodeId(entry.node_id).object_id(), path));
            }
        }
    }
    Ok(output)
}

fn display_component(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut output, byte| {
        use std::fmt::Write as _;
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
            output.push(char::from(*byte));
        } else {
            write!(&mut output, "%{byte:02X}").expect("writing to a String cannot fail");
        }
        output
    })
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

fn client_for_trust(trust: &TrustRoot) -> Result<FoksClient> {
    match trust {
        TrustRoot::WebPki => Ok(FoksClient::webpki()),
        TrustRoot::CertificateDer { path } => {
            let certificate = read_bounded_regular_file(path, MAX_CERTIFICATE_BYTES as u64)?;
            if certificate.is_empty() || certificate.len() > MAX_CERTIFICATE_BYTES {
                return Err(Error::TrustRoot);
            }
            let mut roots = rustls::RootCertStore::empty();
            roots
                .add(CertificateDer::from(certificate))
                .map_err(|_| Error::TrustRoot)?;
            Ok(FoksClient::with_roots(roots))
        }
    }
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
        return Err(Error::InvalidConfig("file path is unsafe or excessive"));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(maximum + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum {
        return Err(Error::InvalidConfig("file grew beyond its size limit"));
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
    use foks_keystore::MemorySecretStore;
    use std::collections::BTreeSet;

    const CANARY_KEY: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

    fn probe_only() -> ProtocolPolicy {
        ProtocolPolicy::CurrentProbeOnly {
            canary_public_key: CANARY_KEY.to_owned(),
        }
    }

    fn profile(name: &str, protocol: ProtocolPolicy) -> Profile {
        Profile {
            name: name.to_owned(),
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
    fn explicit_private_file_credentials_round_trip_without_native_services() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let initialized =
            ClientCredentials::initialize(&root, CredentialBackend::PrivateFile).unwrap();
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
        };
        assert!(checkpoint.verify_descends_from(&checkpoint).is_ok());
        let external = serde_json::to_vec(&checkpoint).unwrap();
        assert!(external.len() < 1024);
        assert!(!String::from_utf8_lossy(&external).contains("host_chain_bytes"));
        assert!(!String::from_utf8_lossy(&external).contains("authenticated_roots"));

        let mut missing_merkle_history = checkpoint.clone();
        missing_merkle_history.authenticated_roots.clear();
        assert!(matches!(
            missing_merkle_history.verify_descends_from(&checkpoint),
            Err(Error::RollbackDetected("Merkle checkpoint is absent"))
        ));

        let mut forked_hostchain = checkpoint.clone();
        forked_hostchain.host_chain_tail[0] ^= 1;
        assert!(matches!(
            checkpoint.verify_descends_from(&forked_hostchain),
            Err(Error::RollbackDetected("hostchain checkpoint is absent"))
        ));
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
            target: "foks.app".to_owned(),
            run_id: "run-1".to_owned(),
            generated_at: 100,
            expires_at: 200,
            protocol_metadata_sha256: "11".repeat(32),
            mutation_digest: "22".repeat(32),
            read_digest: "33".repeat(32),
            outcome: CanaryOutcome::Compatible,
            capabilities: BTreeSet::from(["kv".to_owned(), "user-sync".to_owned()]),
            drift_reason: String::new(),
        };
        let initial = profile("hosted", probe_only());
        let signed = SignedCanaryArtifact::sign(artifact.clone(), &seed).unwrap();
        let granted = initial.apply_canary(&signed, 101).unwrap();
        assert!(granted.require_at(Capability::Kv, 199).is_ok());
        assert!(matches!(
            granted.require_at(Capability::Kv, 200),
            Err(Error::CapabilityDenied(Capability::Kv))
        ));

        let mut tampered = granted.clone();
        if let ProtocolPolicy::CurrentValidated { artifact, .. } = &mut tampered.protocol {
            artifact.artifact.capabilities.insert("teams".to_owned());
        }
        assert!(matches!(
            tampered.validate(),
            Err(Error::InvalidProfile("canary signature is invalid"))
        ));

        artifact.outcome = CanaryOutcome::Drift;
        artifact.capabilities.clear();
        artifact.drift_reason = "read-back mismatch".to_owned();
        let drift = SignedCanaryArtifact::sign(artifact, &seed).unwrap();
        let revoked = granted.apply_canary(&drift, 102).unwrap();
        assert!(matches!(
            revoked.require_at(Capability::Kv, 102),
            Err(Error::CapabilityDenied(Capability::Kv))
        ));
        assert!(matches!(
            revoked.protocol,
            ProtocolPolicy::CurrentProbeOnly { .. }
        ));
    }

    #[test]
    fn account_vault_validates_binding_and_lists_aliases() {
        let mut store = MemorySecretStore::default();
        let credential = DeviceCredential {
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
        vault
            .commit_created("personal", "alice", &credential)
            .unwrap();
        assert_eq!(vault.aliases().unwrap(), vec!["personal"]);
        let loaded = vault.account("personal").unwrap();
        assert_eq!(loaded.username, "alice");
        assert_eq!(loaded.credential.seed.as_slice(), &[9; 32]);
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
        for invalid in ["relative", "/", "/one//two", "/one/../two", "/one/."] {
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
    }
}
