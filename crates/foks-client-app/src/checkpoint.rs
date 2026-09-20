use super::*;
use sha2::{Digest as _, Sha256};

#[cfg(test)]
pub(super) static TEST_FAIL_AFTER_CHECKPOINT_PUBLICATION: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(test)]
pub(super) static TEST_FAIL_AFTER_RESET_STAGING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

const PROFILE_PUBLICATION_AUTHORIZATION_PREFIX: &str = "profile-publication.";
pub(super) const NATIVE_MANIFEST_RECORD: &str = "native-state-v1";
const NATIVE_MANIFEST_VERSION: u32 = 2;
const MAXIMUM_NATIVE_MANIFEST_BYTES: usize = 1024 * 1024;
const MAXIMUM_NATIVE_MANIFEST_RECORDS: usize = 1024;
const MAXIMUM_NATIVE_MANIFEST_VALUE_BYTES: usize = 64 * 1024;
const RESET_CREDENTIAL_QUARANTINE: &str = ".reset-credentials";
const RESET_MUTATION_QUARANTINE: &str = ".reset-mutations";

pub(super) struct ProfilePublicationAuthorization {
    pub(super) authorized: bool,
    pub(super) nonce: [u8; 32],
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeManifestStore {
    version: u32,
    pub(crate) generation: u64,
    pub(crate) records: BTreeMap<String, Vec<u8>>,
    #[serde(skip)]
    dirty: bool,
}

impl NativeManifestStore {
    pub(crate) fn initialized(root: &Path, master_key: &[u8; 32]) -> Self {
        Self {
            version: NATIVE_MANIFEST_VERSION,
            generation: 0,
            records: BTreeMap::from([
                (MASTER_KEY_RECORD.to_owned(), master_key.to_vec()),
                (
                    STATE_ROOT_RECORD.to_owned(),
                    state_root_binding(root).to_vec(),
                ),
            ]),
            dirty: true,
        }
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAXIMUM_NATIVE_MANIFEST_BYTES {
            return Err(Error::InvalidConfig("native client manifest is too large"));
        }
        let manifest: Self = serde_json::from_slice(bytes)
            .map_err(|_| Error::InvalidConfig("native client manifest is invalid"))?;
        if manifest.version != NATIVE_MANIFEST_VERSION
            || manifest.records.len() > MAXIMUM_NATIVE_MANIFEST_RECORDS
            || manifest.records.iter().any(|(key, value)| {
                !valid_manifest_key(key) || value.len() > MAXIMUM_NATIVE_MANIFEST_VALUE_BYTES
            })
        {
            return Err(Error::InvalidConfig("native client manifest is invalid"));
        }
        Ok(manifest)
    }

    pub(crate) fn encode(&self) -> Result<Zeroizing<Vec<u8>>> {
        if self.records.len() > MAXIMUM_NATIVE_MANIFEST_RECORDS
            || self.records.iter().any(|(key, value)| {
                !valid_manifest_key(key) || value.len() > MAXIMUM_NATIVE_MANIFEST_VALUE_BYTES
            })
        {
            return Err(Error::InvalidConfig("native client manifest is invalid"));
        }
        let bytes = Zeroizing::new(serde_json::to_vec(self)?);
        if bytes.len() > MAXIMUM_NATIVE_MANIFEST_BYTES {
            return Err(Error::InvalidConfig("native client manifest is too large"));
        }
        Ok(bytes)
    }
}

impl Drop for NativeManifestStore {
    fn drop(&mut self) {
        for value in self.records.values_mut() {
            value.zeroize();
        }
    }
}

impl CheckpointStore for NativeManifestStore {
    fn put(&mut self, key: &str, value: &[u8]) -> foks_keystore::Result<()> {
        if !valid_manifest_key(key) {
            return Err(foks_keystore::Error::InvalidKey);
        }
        if value.len() > MAXIMUM_NATIVE_MANIFEST_VALUE_BYTES {
            return Err(foks_keystore::Error::TooLarge);
        }
        if let Some(mut previous) = self.records.insert(key.to_owned(), value.to_vec()) {
            previous.zeroize();
        }
        self.dirty = true;
        Ok(())
    }

    fn get(&mut self, key: &str) -> foks_keystore::Result<Zeroizing<Vec<u8>>> {
        if !valid_manifest_key(key) {
            return Err(foks_keystore::Error::InvalidKey);
        }
        self.records
            .get(key)
            .cloned()
            .map(Zeroizing::new)
            .ok_or(foks_keystore::Error::Missing)
    }

    fn remove(&mut self, key: &str) -> foks_keystore::Result<bool> {
        if !valid_manifest_key(key) {
            return Err(foks_keystore::Error::InvalidKey);
        }
        let mut removed = self.records.remove(key);
        if let Some(value) = &mut removed {
            value.zeroize();
            self.dirty = true;
        }
        Ok(removed.is_some())
    }
}

fn valid_manifest_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        && key != "."
        && key != ".."
}

thread_local! {
    /// Checked profile operations are synchronous and their file locks are
    /// held by the process, not the thread, so a recursive federation graph
    /// that revisits a profile already on this thread's stack would block on
    /// a lock it is itself holding. Remember what this thread holds so the
    /// nested visit reuses the outer hold instead. The outermost operation
    /// stays responsible for verifying and advancing the rollback checkpoint
    /// once every nested mutation has completed.
    static HELD_CHECKED_PROFILES: std::cell::RefCell<BTreeMap<PathBuf, usize>> =
        const { std::cell::RefCell::new(BTreeMap::new()) };
}

/// Resolves a profile directory to the identity the operation lock actually
/// uses. The lock is a file lock, so it collides on the real directory rather
/// than on how the path was spelled; keying the thread-local the same way
/// keeps reentry detection from missing an equivalent spelling and
/// deadlocking on the lock this thread already owns.
fn held_profile_key(directory: &Path) -> PathBuf {
    std::fs::canonicalize(directory).unwrap_or_else(|_| directory.to_owned())
}

struct HeldCheckedProfile {
    key: PathBuf,
}

impl HeldCheckedProfile {
    fn is_held(key: &Path) -> bool {
        HELD_CHECKED_PROFILES.with(|held| held.borrow().contains_key(key))
    }

    fn enter(key: PathBuf) -> Self {
        HELD_CHECKED_PROFILES.with(|held| {
            *held.borrow_mut().entry(key.clone()).or_insert(0) += 1;
        });
        Self { key }
    }
}

impl Drop for HeldCheckedProfile {
    fn drop(&mut self) {
        HELD_CHECKED_PROFILES.with(|held| {
            let mut held = held.borrow_mut();
            let count = held
                .get_mut(&self.key)
                .expect("checked-profile hold is balanced");
            *count -= 1;
            if *count == 0 {
                held.remove(&self.key);
            }
        });
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CredentialBackend {
    /// Native macOS Keychain or Linux Secret Service storage.
    Native,
    /// Explicit development/test backend with no external rollback boundary.
    PrivateFile,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResetArtifactKind {
    HardState,
    SoftState,
    ProtectedMutations,
    CredentialsAndResumables,
    ExternalRollbackCheckpoint,
    ExternalDatabaseClaim,
    ExternalPublicationAuthorization,
    ExternalImportReadiness,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResetArtifactSummary {
    pub kind: ResetArtifactKind,
    pub entries: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResetStatePreview {
    pub profile: String,
    pub resumables: Vec<PendingOperationSummary>,
    pub artifacts: Vec<ResetArtifactSummary>,
    /// Why this Mac's credentials could not be read, when a best-effort
    /// preview went ahead without them. Such a preview lists only what is on
    /// disk; the reset it authorizes erases that and leaves whatever
    /// credential records it cannot reach.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials_unavailable: Option<String>,
    #[serde(skip)]
    state_digest: [u8; 32],
}

impl ResetStatePreview {
    pub fn state_digest(&self) -> [u8; 32] {
        self.state_digest
    }
}

/// What a completed reset could not do, when it went ahead best-effort.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResetOutcome {
    /// Why credential records outside the profile directory were left in
    /// place: the native credential service could not be used to remove them.
    pub credential_records_retained: Option<String>,
}

/// Whether a reset insists on describing and erasing everything exactly, or
/// goes as far as it can when this Mac's credentials cannot be read.
///
/// A reset is the way out of a client state whose credentials are unreadable
/// or whose profile publication never completed, so the interactive reset
/// must not depend on either; the CLI's explicit reset keeps the exact rule.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResetMode {
    Exact,
    BestEffort,
}

/// A failure to read this Mac's credentials at all, as opposed to one the user
/// can answer (a credential prompt) or one about the state's location.
fn credentials_unreadable(error: &Error) -> bool {
    match error {
        Error::Keystore(foks_keystore::Error::CredentialsRequired) => false,
        Error::Keystore(_) => true,
        Error::InvalidConfig("native client manifest is missing") => true,
        _ => false,
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct ClientStateFile {
    pub(crate) version: u32,
    pub(crate) state_id: String,
    pub(crate) credential_backend: CredentialBackend,
}

/// Passive state-envelope inspection: no directory creation, chmod or native access.
pub(crate) fn inspect_state_file(root: &Path) -> Result<Option<ClientStateFile>> {
    let Some(bytes) = read_private_file_optional(&root.join(STATE_CONFIG_FILE), MAX_CONFIG_BYTES)?
    else {
        return Ok(None);
    };
    let state: ClientStateFile = toml::from_slice(&bytes)?;
    if state.version != STATE_CONFIG_VERSION {
        return Err(Error::InvalidConfig(
            if state.version == 2 && state.credential_backend == CredentialBackend::Native {
                "native client state uses unsupported per-record credential storage"
            } else {
                "unsupported client state version"
            },
        ));
    }
    validate_name(&state.state_id)?;
    Ok(Some(state))
}

/// Resolves the vault wrapping key and security checkpoint independently from
/// profiles and databases. One versioned native manifest remains outside the state root.
pub struct ClientCredentials {
    pub(super) lease: ClientStateLease,
    pub(super) root: PathBuf,
    pub(super) state_id: String,
    pub(super) backend: CredentialBackend,
}

impl ClientCredentials {
    /// Reports whether the state envelope exists without treating an absent
    /// first-run state as corruption. Existing state is still fully parsed
    /// and opened by `open` before the agent enters ready mode.
    pub fn is_initialized(root: impl AsRef<Path>) -> Result<bool> {
        let lease = ClientStateLease::acquire(root.as_ref())?;
        let root = lease.root().to_owned();
        Ok(read_private_file_optional(&root.join(STATE_CONFIG_FILE), MAX_CONFIG_BYTES)?.is_some())
    }

    pub fn initialize(root: impl AsRef<Path>, backend: CredentialBackend) -> Result<Self> {
        let lease = ClientStateLease::acquire(root.as_ref())?;
        let root = lease.root().to_owned();
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
                let manifest = NativeManifestStore::initialized(&root, &master);
                let bytes = manifest.encode()?;
                let _manifest_lock = runtime::NativeManifestLock::acquire(&state_id)?;
                let mut native = foks_keystore::NativeCredentialStore::open(&state_id)?;
                match native.get(NATIVE_MANIFEST_RECORD) {
                    Err(foks_keystore::Error::Missing) => {}
                    Ok(_) => {
                        return Err(Error::InvalidConfig("new native namespace already exists"))
                    }
                    Err(e) => return Err(e.into()),
                }
                native.put(NATIVE_MANIFEST_RECORD, &bytes)?;
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
                        let _ = native.remove(NATIVE_MANIFEST_RECORD);
                    }
                }
                CredentialBackend::PrivateFile => {
                    let _ = fs::remove_file(root.join("master.key"));
                }
            }
            return Err(error);
        }
        // Initialization acquired the path before a namespace existed. Retain a
        // namespace-use lease as soon as its state envelope has been published.
        let lease = ClientStateLease::acquire(&root)?;
        Ok(Self {
            lease,
            root,
            state_id,
            backend,
        })
    }

    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let lease = ClientStateLease::acquire(root.as_ref())?;
        let root = lease.root().to_owned();
        let bytes = read_private_file_optional(&root.join(STATE_CONFIG_FILE), MAX_CONFIG_BYTES)?
            .ok_or(Error::InvalidConfig("client state is not initialized"))?;
        let state: ClientStateFile = toml::from_slice(&bytes)?;
        if state.version != STATE_CONFIG_VERSION {
            return Err(Error::InvalidConfig(
                if state.version == 2 && state.credential_backend == CredentialBackend::Native {
                    "native client state uses unsupported per-record credential storage"
                } else {
                    "unsupported client state version"
                },
            ));
        }
        validate_name(&state.state_id)?;
        let credentials = Self {
            lease,
            root,
            state_id: state.state_id,
            backend: state.credential_backend,
        };
        credentials.verify_native_root_binding()?;
        Ok(credentials)
    }

    pub fn backend(&self) -> CredentialBackend {
        self.backend
    }

    fn open_native_manifest(
        &self,
    ) -> Result<(foks_keystore::NativeCredentialStore, NativeManifestStore)> {
        let mut native = foks_keystore::NativeCredentialStore::open(&self.state_id)?;
        let bytes = native
            .get(NATIVE_MANIFEST_RECORD)
            .map_err(|error| match error {
                foks_keystore::Error::Missing => {
                    Error::InvalidConfig("native client manifest is missing")
                }
                other => Error::Keystore(other),
            })?;
        let manifest = NativeManifestStore::decode(&bytes)?;
        Ok((native, manifest))
    }

    fn persist_native_manifest(
        native: &mut foks_keystore::NativeCredentialStore,
        manifest: &mut NativeManifestStore,
    ) -> Result<()> {
        if manifest.dirty {
            // Native services lack CAS. The stable namespace manifest lock is held
            // by the caller; reject replacement of a generation we did not read.
            let current = NativeManifestStore::decode(&native.get(NATIVE_MANIFEST_RECORD)?)?;
            if current.generation != manifest.generation {
                return Err(Error::StatePathChanged);
            }
            manifest.generation = manifest
                .generation
                .checked_add(1)
                .ok_or(Error::InvalidConfig("native manifest generation exhausted"))?;
            let bytes = manifest.encode()?;
            native.put(NATIVE_MANIFEST_RECORD, &bytes)?;
            manifest.dirty = false;
        }
        Ok(())
    }

    pub(crate) fn with_native_manifest<T>(
        &self,
        operation: impl FnOnce(&mut NativeManifestStore) -> Result<T>,
    ) -> Result<T> {
        self.lease.validate()?;
        let lock = runtime::NativeManifestLock::acquire(&self.state_id)?;
        let (mut native, mut manifest) = self.open_native_manifest()?;
        crate::portability::relocation::require_ready(&manifest)?;
        self.verify_root_binding_with_store(&mut manifest)?;
        let result = operation(&mut manifest);
        let persist = Self::persist_native_manifest(&mut native, &mut manifest);
        let release = lock.release();
        persist?;
        release?;
        result
    }

    fn try_with_native_manifest<T>(
        &self,
        operation: impl FnOnce(&mut NativeManifestStore) -> Result<T>,
    ) -> Result<Option<T>> {
        self.lease.validate()?;
        let Some(lock) = runtime::NativeManifestLock::try_acquire(&self.state_id)? else {
            return Ok(None);
        };
        let (mut native, mut manifest) = self.open_native_manifest()?;
        crate::portability::relocation::require_ready(&manifest)?;
        self.verify_root_binding_with_store(&mut manifest)?;
        let result = operation(&mut manifest);
        let persist = Self::persist_native_manifest(&mut native, &mut manifest);
        let release = lock.release();
        persist?;
        release?;
        result.map(Some)
    }

    pub fn master_key(&self) -> Result<Zeroizing<[u8; 32]>> {
        self.lease.validate()?;
        match self.backend {
            CredentialBackend::Native => self.with_native_manifest(|manifest| {
                let bytes = manifest.get(MASTER_KEY_RECORD)?;
                if bytes.len() != 32 {
                    return Err(foks_keystore::Error::InvalidMasterKey.into());
                }
                let mut key = Zeroizing::new([0u8; 32]);
                key.copy_from_slice(&bytes);
                Ok(key)
            }),
            CredentialBackend::PrivateFile => {
                foks_keystore::load_master_key_file(self.root.join("master.key"))
                    .map_err(Into::into)
            }
        }
    }

    pub(super) fn begin_profile_publication(&self, profile: &str) -> Result<[u8; 32]> {
        validate_name(profile)?;
        if self.backend == CredentialBackend::Native {
            return self.with_native_manifest(|manifest| {
                begin_profile_publication_with_store(profile, manifest)
            });
        }
        random_array()
    }

    pub(super) fn profile_publication_is_authorized(
        &self,
        profile: &str,
        authorization: &[u8; 32],
        marker_digest: &[u8; 32],
    ) -> Result<bool> {
        if self.backend != CredentialBackend::Native {
            return Ok(true);
        }
        self.with_native_manifest(|manifest| {
            profile_publication_is_authorized_with_store(
                profile,
                authorization,
                marker_digest,
                manifest,
            )
        })
    }

    pub(super) fn bind_profile_publication(
        &self,
        profile: &str,
        authorization: &[u8; 32],
        marker_digest: &[u8; 32],
    ) -> Result<()> {
        if self.backend != CredentialBackend::Native {
            return Ok(());
        }
        self.with_native_manifest(|manifest| {
            bind_profile_publication_with_store(profile, authorization, marker_digest, manifest)
        })
    }

    pub(super) fn cancel_profile_publication(
        &self,
        profile: &str,
        authorization: &[u8; 32],
    ) -> Result<()> {
        if self.backend != CredentialBackend::Native {
            return Ok(());
        }
        self.with_native_manifest(|manifest| {
            cancel_profile_publication_with_store(profile, authorization, manifest)
        })
    }

    fn verify_native_root_binding(&self) -> Result<()> {
        if self.backend != CredentialBackend::Native {
            return Ok(());
        }
        self.with_native_manifest(|manifest| self.verify_root_binding_with_store(manifest))
    }

    pub(super) fn verify_root_binding_with_store(
        &self,
        store: &mut impl CheckpointStore,
    ) -> Result<()> {
        let stored = store.get(STATE_ROOT_RECORD).map_err(|error| match error {
            foks_keystore::Error::Missing => {
                Error::InvalidConfig("native client state root binding is missing")
            }
            other => Error::Keystore(other),
        })?;
        if stored.as_slice() != state_root_binding(&self.root) {
            return Err(Error::InvalidConfig(
                "native client state belongs to a different root path",
            ));
        }
        Ok(())
    }

    /// Serializes security-sensitive use per profile across CLI and agent
    /// processes, verifies the external watermark before the operation, and
    /// republishes it afterward even when the operation reports an error.
    pub fn with_checked_session<T, E>(
        &self,
        session: &ProfileSession,
        operation: impl FnOnce(&CheckedProfileSession<'_>) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E>
    where
        E: From<Error>,
    {
        self.with_session_policy(session, false, operation)
    }

    // Only the portability module exposes narrow proof and reauthentication APIs.
    pub fn requires_import_verification(&self, session: &ProfileSession) -> Result<bool> {
        self.ensure_session_root(session)?;
        let native = self.backend == CredentialBackend::Native
            && self.with_native_manifest(|m| {
                Ok(m.records
                    .contains_key(&crate::portability::readiness::key(&session.profile.name)?))
            })?;
        Ok(native
            || foks_client_db::HardStateStore::open(&session.paths.hard_database)?
                .requires_import_verification()?)
    }

    pub(crate) fn with_import_session<T>(
        &self,
        session: &ProfileSession,
        operation: impl FnOnce(&CheckedProfileSession<'_>) -> Result<T>,
    ) -> Result<T> {
        self.with_session_policy(session, true, operation)
    }

    fn with_session_policy<T, E>(
        &self,
        session: &ProfileSession,
        import_proof: bool,
        operation: impl FnOnce(&CheckedProfileSession<'_>) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E>
    where
        E: From<Error>,
    {
        self.ensure_session_root(session).map_err(E::from)?;
        let key = held_profile_key(&session.paths.directory);
        if HeldCheckedProfile::is_held(&key) {
            // Already checked and locked further up this thread's stack.
            let checked = if import_proof {
                crate::portability::readiness::validate_for_proof(self, session)
                    .map_err(E::from)?;
                CheckedProfileSession { session }
            } else {
                checked_profile_for_use(self, session).map_err(E::from)?
            };
            return operation(&checked);
        }
        let lock = runtime::ProfileLock::operation(session.paths()).map_err(E::from)?;
        let database_lock = self.lock_and_verify_checkpoint(session).map_err(E::from)?;
        let checked = if import_proof {
            crate::portability::readiness::validate_for_proof(self, session).map_err(E::from)?;
            CheckedProfileSession { session }
        } else {
            checked_profile_for_use(self, session).map_err(E::from)?
        };
        let held = HeldCheckedProfile::enter(key);
        let result = operation(&checked);
        drop(held);
        let checkpoint = self.advance_checkpoint(session);
        let database_unlock = database_lock
            .map(runtime::DatabaseLock::release)
            .transpose();
        let unlock = lock.release();
        checkpoint.map_err(E::from)?;
        database_unlock.map_err(E::from)?;
        unlock.map_err(E::from)?;
        result
    }

    /// Checks and locks two distinct profiles in a canonical order for one
    /// cross-host operation. Both external rollback watermarks are published
    /// after the closure, including when one side committed recovery state
    /// before returning an error.
    pub fn with_checked_sessions<T, E>(
        &self,
        left: &ProfileSession,
        right: &ProfileSession,
        operation: impl FnOnce(
            &CheckedProfileSession<'_>,
            &CheckedProfileSession<'_>,
        ) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E>
    where
        E: From<Error>,
    {
        self.with_checked_sessions_policy(left, right, false, operation)
            .map(|value| value.expect("blocking checked sessions acquire both locks"))
    }

    /// Returns None only when admission failed before entering the operation.
    /// Every acquired lock is released before returning None, so a caller can
    /// wait with a deadline and retry admission without replaying a mutation.
    pub fn try_with_checked_sessions<T, E>(
        &self,
        left: &ProfileSession,
        right: &ProfileSession,
        operation: impl FnOnce(
            &CheckedProfileSession<'_>,
            &CheckedProfileSession<'_>,
        ) -> std::result::Result<T, E>,
    ) -> std::result::Result<Option<T>, E>
    where
        E: From<Error>,
    {
        self.with_checked_sessions_policy(left, right, true, operation)
    }

    fn with_checked_sessions_policy<T, E>(
        &self,
        left: &ProfileSession,
        right: &ProfileSession,
        nonblocking: bool,
        operation: impl FnOnce(
            &CheckedProfileSession<'_>,
            &CheckedProfileSession<'_>,
        ) -> std::result::Result<T, E>,
    ) -> std::result::Result<Option<T>, E>
    where
        E: From<Error>,
    {
        self.ensure_session_root(left).map_err(E::from)?;
        self.ensure_session_root(right).map_err(E::from)?;
        if left.paths.directory == right.paths.directory {
            return Err(E::from(Error::InvalidConfig(
                "cross-host operation requires two distinct profiles",
            )));
        }
        let left_key = held_profile_key(&left.paths.directory);
        let right_key = held_profile_key(&right.paths.directory);
        if left_key == right_key {
            return Err(E::from(Error::InvalidConfig(
                "cross-host operation requires two distinct profiles",
            )));
        }
        if HeldCheckedProfile::is_held(&left_key) || HeldCheckedProfile::is_held(&right_key) {
            // A paired operation takes both locks in a canonical order, so it
            // cannot reuse a single hold this thread already owns without
            // inverting that order. Return an error instead of blocking on an
            // existing lock held by this thread.
            return Err(E::from(Error::InvalidConfig(
                "cross-host operation cannot nest inside a checked session for either profile",
            )));
        }
        let (first, second) = if left_key < right_key {
            (left, right)
        } else {
            (right, left)
        };
        let acquire = |session: &ProfileSession| {
            if nonblocking {
                runtime::ProfileLock::try_operation(session.paths())
            } else {
                runtime::ProfileLock::operation(session.paths()).map(Some)
            }
        };
        let Some(first_lock) = acquire(first).map_err(E::from)? else {
            return Ok(None);
        };
        let Some(second_lock) = acquire(second).map_err(E::from)? else {
            return Ok(None);
        };
        let database_locks = if nonblocking {
            // Try both database/checkpoint locks without waiting while holding
            // either. A shared database identity remains an integrity error.
            if self.backend == CredentialBackend::Native
                && first.rollback_checkpoint().map_err(E::from)?.database_id
                    == second.rollback_checkpoint().map_err(E::from)?.database_id
            {
                return Err(E::from(self.checkpoint_reset_error(
                    second,
                    "hard-state database identity is already used by another profile",
                )));
            }
            let Some(first_database) = self
                .try_lock_and_verify_checkpoint(first)
                .map_err(E::from)?
            else {
                return Ok(None);
            };
            let Some(second_database) = self
                .try_lock_and_verify_checkpoint(second)
                .map_err(E::from)?
            else {
                return Ok(None);
            };
            first_database.into_iter().chain(second_database).collect()
        } else {
            self.lock_and_verify_checkpoints(first, second)
                .map_err(E::from)?
        };

        let left_checked = checked_profile_for_use(self, left).map_err(E::from)?;
        let right_checked = checked_profile_for_use(self, right).map_err(E::from)?;
        let left_held = HeldCheckedProfile::enter(left_key);
        let right_held = HeldCheckedProfile::enter(right_key);
        let result = operation(&left_checked, &right_checked);
        drop(right_held);
        drop(left_held);

        // Perform every durability and release step before selecting which
        // error to report. An early return here could strand the other
        // profile's external watermark behind a committed SQLite revision.
        let left_checkpoint = self.advance_checkpoint(left);
        let right_checkpoint = self.advance_checkpoint(right);
        let database_release = release_database_locks(database_locks);
        let second_release = second_lock.release();
        let first_release = first_lock.release();
        left_checkpoint.map_err(E::from)?;
        right_checkpoint.map_err(E::from)?;
        database_release.map_err(E::from)?;
        second_release.map_err(E::from)?;
        first_release.map_err(E::from)?;
        result.map(Some)
    }

    /// Attempts the checked-session sequence without waiting for another
    /// process using the profile. Periodic work uses this to skip contention.
    pub fn try_with_checked_session<T, E>(
        &self,
        session: &ProfileSession,
        operation: impl FnOnce(&CheckedProfileSession<'_>) -> std::result::Result<T, E>,
    ) -> std::result::Result<Option<T>, E>
    where
        E: From<Error>,
    {
        self.ensure_session_root(session).map_err(E::from)?;
        let key = held_profile_key(&session.paths.directory);
        if HeldCheckedProfile::is_held(&key) {
            let checked = checked_profile_for_use(self, session).map_err(E::from)?;
            return operation(&checked).map(Some);
        }
        let Some(lock) = runtime::ProfileLock::try_operation(session.paths()).map_err(E::from)?
        else {
            return Ok(None);
        };
        let database_lock = match self
            .try_lock_and_verify_checkpoint(session)
            .map_err(E::from)?
        {
            Some(lock) => lock,
            None => return Ok(None),
        };
        let checked = checked_profile_for_use(self, session).map_err(E::from)?;
        let held = HeldCheckedProfile::enter(key);
        let result = operation(&checked);
        drop(held);
        let checkpoint = self.advance_checkpoint(session);
        let database_unlock = database_lock
            .map(runtime::DatabaseLock::release)
            .transpose();
        let unlock = lock.release();
        checkpoint.map_err(E::from)?;
        database_unlock.map_err(E::from)?;
        unlock.map_err(E::from)?;
        result.map(Some)
    }

    fn lock_and_verify_checkpoint(
        &self,
        session: &ProfileSession,
    ) -> Result<Option<runtime::DatabaseLock>> {
        if self.backend != CredentialBackend::Native {
            self.complete_private_profile_publication(session)?;
            return Ok(None);
        }
        let pristine = !hard_state_artifacts_exist(&session.paths.hard_database)?;
        let current = session.rollback_checkpoint()?;
        let publication = self.profile_publication_authorization(session, &current)?;
        let authorized = publication
            .as_ref()
            .is_some_and(|publication| publication.authorized);
        let lock = runtime::DatabaseLock::acquire(&self.root, &current.database_id)?;
        self.verify_native_checkpoint_for_use(
            session,
            &current,
            pristine || authorized,
            publication.as_ref(),
        )?;
        Ok(Some(lock))
    }

    fn try_lock_and_verify_checkpoint(
        &self,
        session: &ProfileSession,
    ) -> Result<Option<Option<runtime::DatabaseLock>>> {
        if self.backend != CredentialBackend::Native {
            self.complete_private_profile_publication(session)?;
            return Ok(Some(None));
        }
        let pristine = !hard_state_artifacts_exist(&session.paths.hard_database)?;
        let current = session.rollback_checkpoint()?;
        let Some(publication) = self.try_profile_publication_authorization(session, &current)?
        else {
            return Ok(None);
        };
        let authorized = publication
            .as_ref()
            .is_some_and(|publication| publication.authorized);
        let Some(lock) = runtime::DatabaseLock::try_acquire(&self.root, &current.database_id)?
        else {
            return Ok(None);
        };
        let verified = self.try_with_native_manifest(|manifest| {
            self.verify_native_checkpoint_for_use_with_store(
                session,
                &current,
                pristine || authorized,
                publication.as_ref(),
                manifest,
            )
        })?;
        if verified.is_none() {
            return Ok(None);
        }
        Ok(Some(Some(lock)))
    }

    fn lock_and_verify_checkpoints(
        &self,
        first: &ProfileSession,
        second: &ProfileSession,
    ) -> Result<Vec<runtime::DatabaseLock>> {
        if self.backend != CredentialBackend::Native {
            self.complete_private_profile_publication(first)?;
            self.complete_private_profile_publication(second)?;
            return Ok(Vec::new());
        }
        let first_pristine = !hard_state_artifacts_exist(&first.paths.hard_database)?;
        let second_pristine = !hard_state_artifacts_exist(&second.paths.hard_database)?;
        let first_current = first.rollback_checkpoint()?;
        let second_current = second.rollback_checkpoint()?;
        let first_publication = self.profile_publication_authorization(first, &first_current)?;
        let second_publication = self.profile_publication_authorization(second, &second_current)?;
        if first_current.database_id == second_current.database_id {
            return Err(self.checkpoint_reset_error(
                second,
                "hard-state database identity is already used by another profile",
            ));
        }
        let mut database_ids = [first_current.database_id, second_current.database_id];
        database_ids.sort_unstable();
        let mut locks = Vec::with_capacity(database_ids.len());
        for database_id in database_ids {
            locks.push(runtime::DatabaseLock::acquire(&self.root, &database_id)?);
        }
        self.with_native_manifest(|manifest| {
            self.verify_native_checkpoint_with_store(
                first,
                &first_current,
                first_pristine
                    || first_publication
                        .as_ref()
                        .is_some_and(|publication| publication.authorized),
                manifest,
            )?;
            self.verify_native_checkpoint_with_store(
                second,
                &second_current,
                second_pristine
                    || second_publication
                        .as_ref()
                        .is_some_and(|publication| publication.authorized),
                manifest,
            )?;
            self.finish_profile_publication_with_store(
                first,
                first_publication.as_ref(),
                manifest,
            )?;
            self.finish_profile_publication_with_store(
                second,
                second_publication.as_ref(),
                manifest,
            )
        })?;
        Ok(locks)
    }

    fn complete_private_profile_publication(&self, session: &ProfileSession) -> Result<()> {
        if !registry::profile_publication_is_pending(session)? {
            return Ok(());
        }
        let current = session.rollback_checkpoint()?;
        let publication = self
            .profile_publication_authorization(session, &current)?
            .ok_or(Error::InvalidConfig(
                "profile publication marker disappeared",
            ))?;
        self.finish_profile_publication(session, Some(&publication))
    }

    fn profile_publication_authorization(
        &self,
        session: &ProfileSession,
        current: &RollbackCheckpoint,
    ) -> Result<Option<ProfilePublicationAuthorization>> {
        let Some(binding) =
            registry::profile_publication_checkpoint_binding(&self.root, session, current)?
        else {
            return Ok(None);
        };
        Ok(Some(ProfilePublicationAuthorization {
            authorized: self.profile_publication_is_authorized(
                &session.profile.name,
                &binding.nonce,
                &binding.marker_digest,
            )?,
            nonce: binding.nonce,
        }))
    }

    fn try_profile_publication_authorization(
        &self,
        session: &ProfileSession,
        current: &RollbackCheckpoint,
    ) -> Result<Option<Option<ProfilePublicationAuthorization>>> {
        let Some(binding) =
            registry::profile_publication_checkpoint_binding(&self.root, session, current)?
        else {
            return Ok(Some(None));
        };
        let Some(authorized) = self.try_with_native_manifest(|manifest| {
            profile_publication_is_authorized_with_store(
                &session.profile.name,
                &binding.nonce,
                &binding.marker_digest,
                manifest,
            )
        })?
        else {
            return Ok(None);
        };
        Ok(Some(Some(ProfilePublicationAuthorization {
            authorized,
            nonce: binding.nonce,
        })))
    }

    fn verify_native_checkpoint_for_use(
        &self,
        session: &ProfileSession,
        current: &RollbackCheckpoint,
        allow_checkpoint_enrollment: bool,
        publication: Option<&ProfilePublicationAuthorization>,
    ) -> Result<()> {
        self.with_native_manifest(|manifest| {
            self.verify_native_checkpoint_for_use_with_store(
                session,
                current,
                allow_checkpoint_enrollment,
                publication,
                manifest,
            )
        })
    }

    pub(super) fn verify_native_checkpoint_for_use_with_store(
        &self,
        session: &ProfileSession,
        current: &RollbackCheckpoint,
        allow_checkpoint_enrollment: bool,
        publication: Option<&ProfilePublicationAuthorization>,
        store: &mut impl CheckpointStore,
    ) -> Result<()> {
        self.verify_native_checkpoint_with_store(
            session,
            current,
            allow_checkpoint_enrollment,
            store,
        )?;
        self.finish_profile_publication_with_store(session, publication, store)
    }

    fn finish_profile_publication(
        &self,
        session: &ProfileSession,
        publication: Option<&ProfilePublicationAuthorization>,
    ) -> Result<()> {
        let Some(publication) = publication else {
            return Ok(());
        };
        #[cfg(test)]
        if TEST_FAIL_AFTER_CHECKPOINT_PUBLICATION.swap(false, std::sync::atomic::Ordering::SeqCst) {
            return Err(Error::InvalidConfig(
                "test interrupted after external checkpoint publication",
            ));
        }
        self.cancel_profile_publication(&session.profile.name, &publication.nonce)?;
        registry::complete_profile_publication(session)
    }

    fn finish_profile_publication_with_store(
        &self,
        session: &ProfileSession,
        publication: Option<&ProfilePublicationAuthorization>,
        store: &mut impl CheckpointStore,
    ) -> Result<()> {
        let Some(publication) = publication else {
            return Ok(());
        };
        #[cfg(test)]
        if TEST_FAIL_AFTER_CHECKPOINT_PUBLICATION.swap(false, std::sync::atomic::Ordering::SeqCst) {
            return Err(Error::InvalidConfig(
                "test interrupted after external checkpoint publication",
            ));
        }
        cancel_profile_publication_with_store(&session.profile.name, &publication.nonce, store)?;
        registry::complete_profile_publication(session)
    }

    pub(super) fn verify_native_checkpoint_with_store(
        &self,
        session: &ProfileSession,
        current: &RollbackCheckpoint,
        allow_checkpoint_enrollment: bool,
        store: &mut impl CheckpointStore,
    ) -> Result<()> {
        let key = rollback_record_key(&session.profile.name)?;
        let publish = self.reconcile_checkpoint_with_store(
            session,
            &key,
            current,
            allow_checkpoint_enrollment,
            store,
        )?;
        self.claim_database_with_store(session, current.database_id, store)?;
        if publish {
            store.put(&key, &serde_json::to_vec(current)?)?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn verify_checkpoint_with_store(
        &self,
        session: &ProfileSession,
        key: &str,
        store: &mut impl CheckpointStore,
    ) -> Result<()> {
        let allow_checkpoint_enrollment =
            !hard_state_artifacts_exist(&session.paths.hard_database)?;
        let current = session.rollback_checkpoint()?;
        let publish = self.reconcile_checkpoint_with_store(
            session,
            key,
            &current,
            allow_checkpoint_enrollment,
            store,
        )?;
        if publish {
            store.put(key, &serde_json::to_vec(&current)?)?;
        }
        Ok(())
    }

    fn reconcile_checkpoint_with_store(
        &self,
        session: &ProfileSession,
        key: &str,
        current: &RollbackCheckpoint,
        allow_checkpoint_enrollment: bool,
        store: &mut impl CheckpointStore,
    ) -> Result<bool> {
        match store.get(key) {
            Ok(bytes) => {
                let previous: RollbackCheckpoint =
                    serde_json::from_slice(&bytes).map_err(|_| {
                        self.checkpoint_reset_error(session, "external checkpoint is invalid")
                    })?;
                let reconciliation = current.reconciliation(&previous).map_err(|error| {
                    self.checkpoint_reset_error(session, rollback_reason(&error))
                })?;
                Ok(reconciliation == CheckpointReconciliation::AdvanceExternal)
            }
            Err(foks_keystore::Error::Missing) => {
                if !allow_checkpoint_enrollment
                    && hard_state_artifacts_exist(&session.paths.hard_database)?
                {
                    return Err(
                        self.checkpoint_reset_error(session, "external checkpoint is missing")
                    );
                }
                Ok(true)
            }
            Err(error) => Err(error.into()),
        }
    }

    fn claim_database_with_store(
        &self,
        session: &ProfileSession,
        database_id: [u8; 16],
        store: &mut impl CheckpointStore,
    ) -> Result<()> {
        let key = database_claim_record_key(&database_id);
        match store.get(&key) {
            Ok(profile) if profile.as_slice() == session.profile.name.as_bytes() => Ok(()),
            Ok(_) => Err(self.checkpoint_reset_error(
                session,
                "hard-state database identity is already claimed by another profile",
            )),
            Err(foks_keystore::Error::Missing) => {
                store.put(&key, session.profile.name.as_bytes())?;
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }

    fn checkpoint_reset_error(&self, session: &ProfileSession, reason: &'static str) -> Error {
        Error::CheckpointResetRequired {
            profile: session.profile.name.clone(),
            state_dir: self.root.clone(),
            reason,
        }
    }

    /// Called while the checked profile lock is held, before chat can submit
    /// protected work. This also covers prepare+attempt in one checked closure.
    pub(super) fn checkpoint_before_chat_delivery(
        &self,
        session: &CheckedProfileSession<'_>,
    ) -> Result<()> {
        self.ensure_session_root(session)?;
        self.advance_checkpoint(session)
    }

    pub(crate) fn advance_checkpoint(&self, session: &ProfileSession) -> Result<()> {
        if self.backend != CredentialBackend::Native {
            return Ok(());
        }
        let current = session.rollback_checkpoint()?;
        self.with_native_manifest(|manifest| {
            self.verify_native_checkpoint_with_store(session, &current, false, manifest)
        })
    }

    /// Describes the complete profile-local reset scope without requiring a
    /// successful rollback check. Every resumable record is decrypted and
    /// validated before it is advertised.
    pub fn describe_reset_state(&self, session: &ProfileSession) -> Result<ResetStatePreview> {
        self.describe_reset_state_in(session, ResetMode::Exact)
    }

    /// Describes the reset scope as far as it can be seen. When this Mac's
    /// credentials cannot be read, the preview says so and lists what is on
    /// disk rather than refusing; a pending profile publication is part of
    /// what the reset will discard rather than a reason not to.
    pub fn describe_reset_state_best_effort(
        &self,
        session: &ProfileSession,
    ) -> Result<ResetStatePreview> {
        self.describe_reset_state_in(session, ResetMode::BestEffort)
    }

    fn describe_reset_state_in(
        &self,
        session: &ProfileSession,
        mode: ResetMode,
    ) -> Result<ResetStatePreview> {
        self.ensure_session_root(session)?;
        if mode == ResetMode::Exact && registry::profile_publication_is_pending(session)? {
            return Err(Error::InvalidConfig(
                "profile publication checkpoint is still pending",
            ));
        }
        let operation = runtime::ProfileLock::operation(session.paths())?;
        let scheduler = runtime::ProfileLock::scheduler(session.paths())?;
        let result = self.reset_state_preview_in(session, mode);
        let scheduler_release = scheduler.release();
        let operation_release = operation.release();
        scheduler_release?;
        operation_release?;
        result
    }

    /// Deliberately removes every profile-local state artifact, but only if it
    /// still exactly matches a recently described preview. Callers provide
    /// one-use authorization separately; this digest closes the state-change
    /// race between describing and executing the reset.
    pub fn reset_hard_state_if_matches(
        &self,
        session: &ProfileSession,
        expected_digest: [u8; 32],
    ) -> Result<()> {
        self.erase_profile_state(session, Some(expected_digest), ResetMode::Exact)
            .map(|_| ())
    }

    /// Removes every profile-local state artifact that matches a best-effort
    /// preview's digest. Credential records the native service cannot reach
    /// are left in place and named in the outcome; a pending profile
    /// publication is abandoned along with the rest.
    pub fn reset_hard_state_best_effort_if_matches(
        &self,
        session: &ProfileSession,
        expected_digest: [u8; 32],
    ) -> Result<ResetOutcome> {
        self.erase_profile_state(session, Some(expected_digest), ResetMode::BestEffort)
    }

    /// Erases the same artifacts as a reset without checking against a preview.
    /// Used during profile removal, where intermediate state changes do not
    /// constitute a conflict.
    pub(super) fn erase_profile_state_for_removal(&self, session: &ProfileSession) -> Result<()> {
        self.erase_profile_state(session, None, ResetMode::Exact)
            .map(|_| ())
    }

    fn erase_profile_state(
        &self,
        session: &ProfileSession,
        expected_digest: Option<[u8; 32]>,
        mode: ResetMode,
    ) -> Result<ResetOutcome> {
        self.ensure_session_root(session)?;
        if mode == ResetMode::Exact && registry::profile_publication_is_pending(session)? {
            return Err(Error::InvalidConfig(
                "profile publication checkpoint is still pending",
            ));
        }
        let operation = runtime::ProfileLock::operation(session.paths())?;
        let scheduler = runtime::ProfileLock::scheduler(session.paths())?;
        let database_ids = self.reset_database_ids(session, mode)?;
        let mut database_locks = Vec::with_capacity(database_ids.len());
        for database_id in &database_ids {
            database_locks.push(runtime::DatabaseLock::acquire(&self.root, database_id)?);
        }
        let result = (|| {
            if let Some(expected_digest) = expected_digest {
                let preview = self.reset_state_preview_in(session, mode)?;
                if preview.state_digest != expected_digest {
                    return Err(Error::ResetPreviewChanged);
                }
            }
            self.reset_profile_state_locked(session, &database_ids, mode)
        })();
        let database_release = release_database_locks(database_locks);
        let scheduler_release = scheduler.release();
        let operation_release = operation.release();
        let outcome = result?;
        database_release?;
        scheduler_release?;
        operation_release?;
        Ok(outcome)
    }

    /// Compatibility helper for explicit CLI confirmation. Agent callers use
    /// `describe_reset_state` and `reset_hard_state_if_matches` with a
    /// short-lived, one-use authorization.
    pub fn reset_hard_state(&self, session: &ProfileSession) -> Result<()> {
        let preview = self.describe_reset_state(session)?;
        self.reset_hard_state_if_matches(session, preview.state_digest)
    }

    fn ensure_session_root(&self, session: &ProfileSession) -> Result<()> {
        self.lease.validate()?;
        session.lease.validate()?;
        let expected = self
            .root
            .join("profiles")
            .join(session.profile.name.as_str());
        if session.paths.directory != expected {
            return Err(Error::InvalidConfig(
                "profile session belongs to a different client state",
            ));
        }
        Ok(())
    }

    fn reset_database_ids(
        &self,
        session: &ProfileSession,
        mode: ResetMode,
    ) -> Result<Vec<[u8; 16]>> {
        let mut ids = Vec::new();
        if session.paths.hard_database.exists() {
            if let Ok(current) = session.rollback_checkpoint() {
                ids.push(current.database_id);
            }
        }
        if self.backend == CredentialBackend::Native {
            let key = rollback_record_key(&session.profile.name)?;
            let external = self.with_native_manifest(|manifest| match manifest.get(&key) {
                Ok(bytes) => {
                    let checkpoint: RollbackCheckpoint = serde_json::from_slice(&bytes)?;
                    if checkpoint.profile != session.profile.name {
                        return Err(Error::InvalidConfig(
                            "external checkpoint profile binding changed",
                        ));
                    }
                    Ok(Some(checkpoint.database_id))
                }
                Err(foks_keystore::Error::Missing) => Ok(None),
                Err(error) => Err(error.into()),
            });
            match external {
                Ok(Some(database_id)) => ids.push(database_id),
                Ok(None) => {}
                // Without the credential service the external checkpoint
                // cannot be read; the reset then locks what it can see.
                Err(error) if mode == ResetMode::BestEffort && credentials_unreadable(&error) => {}
                Err(error) => return Err(error),
            }
        }
        ids.sort_unstable();
        ids.dedup();
        Ok(ids)
    }

    fn reset_state_preview_locked(&self, session: &ProfileSession) -> Result<ResetStatePreview> {
        let master = self.master_key()?;
        if self.backend == CredentialBackend::Native {
            self.with_native_manifest(|manifest| {
                reset_state_preview(
                    session,
                    Some(&master),
                    Some((&session.profile.name, manifest)),
                    None,
                )
            })
        } else {
            reset_state_preview(session, Some(&master), None, None)
        }
    }

    /// The exact preview, or, best-effort, the on-disk preview when this
    /// Mac's credentials cannot be read. The two hash under different domains,
    /// so a reset authorized by one does not match state described by the
    /// other: credentials that come back, or go away, between the preview and
    /// the reset are a change to describe again.
    fn reset_state_preview_in(
        &self,
        session: &ProfileSession,
        mode: ResetMode,
    ) -> Result<ResetStatePreview> {
        match self.reset_state_preview_locked(session) {
            Err(error) if mode == ResetMode::BestEffort && credentials_unreadable(&error) => {
                reset_state_preview(session, None, None, Some(error.to_string()))
            }
            result => result,
        }
    }

    fn reset_profile_state_locked(
        &self,
        session: &ProfileSession,
        database_ids: &[[u8; 16]],
        mode: ResetMode,
    ) -> Result<ResetOutcome> {
        if self
            .root
            .join(crate::pending_chat::DIRECTORY)
            .try_exists()?
        {
            let master = self.master_key()?;
            crate::PendingChatStore::open(&self.root, &master)?
                .forget_profile(&session.profile.name)?;
        }
        stage_reset_directory(
            &session.paths.credential_store,
            &session.paths.directory.join(RESET_CREDENTIAL_QUARANTINE),
        )?;
        stage_reset_directory(
            &session.paths.protected_mutations,
            &session.paths.directory.join(RESET_MUTATION_QUARANTINE),
        )?;
        File::open(&session.paths.directory)?.sync_all()?;
        #[cfg(test)]
        if TEST_FAIL_AFTER_RESET_STAGING.swap(false, std::sync::atomic::Ordering::SeqCst) {
            return Err(Error::InvalidConfig("injected reset interruption"));
        }
        let mut outcome = ResetOutcome::default();
        if self.backend == CredentialBackend::Native {
            let removed = self.with_native_manifest(|manifest| {
                remove_external_reset_records(manifest, &session.profile.name, database_ids)
            });
            match removed {
                Ok(()) => {}
                // The records this Mac cannot reach stay where they are; the
                // profile's own state is still erased, which is what the user
                // asked for, and the outcome says what was left behind.
                Err(error) if mode == ResetMode::BestEffort && credentials_unreadable(&error) => {
                    outcome.credential_records_retained = Some(error.to_string());
                }
                Err(error) => return Err(error),
            }
        }
        for path in reset_local_artifact_paths(session) {
            remove_reset_artifact(&path)?;
        }
        if mode == ResetMode::BestEffort {
            // A publication that never completed is abandoned with the state
            // it was publishing; its marker would otherwise refuse the next
            // preview too.
            registry::complete_profile_publication(session)?;
        }
        File::open(&session.paths.directory)?.sync_all()?;
        Ok(outcome)
    }
}

fn reset_local_artifact_paths(session: &ProfileSession) -> Vec<PathBuf> {
    hard_state_artifact_paths(&session.paths.hard_database)
        .into_iter()
        .chain(hard_state_artifact_paths(&session.paths.soft_database))
        .chain([
            session.paths.protected_mutations.clone(),
            session.paths.directory.join(RESET_MUTATION_QUARANTINE),
            session.paths.credential_store.clone(),
            session.paths.directory.join(RESET_CREDENTIAL_QUARANTINE),
        ])
        .collect()
}

fn reset_state_preview(
    session: &ProfileSession,
    master_key: Option<&[u8; 32]>,
    native: Option<(&str, &mut dyn CheckpointStore)>,
    credentials_unavailable: Option<String>,
) -> Result<ResetStatePreview> {
    let mut digest = Sha256::new();
    digest.update(if master_key.is_some() {
        b"foks-reset-state-preview-v1\0".as_slice()
    } else {
        b"foks-reset-state-preview-on-disk-v1\0".as_slice()
    });
    digest.update(session.profile.name.as_bytes());

    // Resumable operations are read out of the encrypted credential store,
    // which the master key opens; without it the store's files are still
    // described and erased below as bytes on disk.
    let mut resumables = Vec::new();
    for credential_directory in [
        session.paths.credential_store.clone(),
        session.paths.directory.join(RESET_CREDENTIAL_QUARANTINE),
    ] {
        match fs::symlink_metadata(&credential_directory) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(Error::InvalidConfig(
                        "credential reset artifact path is unsafe",
                    ));
                }
                let Some(master_key) = master_key else {
                    continue;
                };
                let mut store = foks_keystore::EncryptedFileSecretStore::open(
                    &credential_directory,
                    derive_vault_key(master_key),
                )?;
                resumables.extend(AccountVault::new(&mut store).pending_operations()?);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    resumables.sort_by(|left, right| {
        left.alias
            .cmp(&right.alias)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.target.cmp(&right.target))
    });
    digest.update(serde_json::to_vec(&resumables)?);

    let groups = [
        (
            ResetArtifactKind::HardState,
            hard_state_artifact_paths(&session.paths.hard_database).to_vec(),
        ),
        (
            ResetArtifactKind::SoftState,
            hard_state_artifact_paths(&session.paths.soft_database).to_vec(),
        ),
        (
            ResetArtifactKind::ProtectedMutations,
            vec![
                session.paths.protected_mutations.clone(),
                session.paths.directory.join(RESET_MUTATION_QUARANTINE),
            ],
        ),
        (
            ResetArtifactKind::CredentialsAndResumables,
            vec![
                session.paths.credential_store.clone(),
                session.paths.directory.join(RESET_CREDENTIAL_QUARANTINE),
                // A publication that never completed is discarded with the
                // credentials it was publishing, so its marker is part of what
                // the preview describes.
                registry::profile_publication_marker(session),
            ],
        ),
    ];
    let mut artifacts = Vec::new();
    for (kind, paths) in groups {
        let mut entries = 0_u64;
        let mut bytes = 0_u64;
        digest.update([kind as u8]);
        for path in paths {
            hash_reset_artifact(
                &session.paths.directory,
                &path,
                &mut digest,
                &mut entries,
                &mut bytes,
            )?;
        }
        if entries != 0 {
            artifacts.push(ResetArtifactSummary {
                kind,
                entries,
                bytes,
            });
        }
    }

    let root = session
        .paths
        .directory
        .parent()
        .and_then(Path::parent)
        .ok_or(Error::StatePathChanged)?;
    if root.join(crate::pending_chat::DIRECTORY).try_exists()? {
        let master = master_key.ok_or(Error::InvalidConfig(
            "saved chat state requires credentials before resetting this profile",
        ))?;
        let (entries, bytes, commitment) = crate::PendingChatStore::open(root, master)?
            .profile_summary(&session.profile.name, master)?;
        digest.update(b"pending-chat-v1");
        digest.update(commitment);
        if entries != 0 {
            artifacts.push(ResetArtifactSummary {
                kind: ResetArtifactKind::ProtectedMutations,
                entries: entries as u64,
                bytes: bytes as u64,
            });
        }
    }

    if let Some((profile, store)) = native {
        hash_external_reset_state(session, profile, store, &mut digest, &mut artifacts)?;
    }

    Ok(ResetStatePreview {
        profile: session.profile.name.clone(),
        resumables,
        artifacts,
        credentials_unavailable,
        state_digest: digest.finalize().into(),
    })
}

fn hash_external_reset_state(
    session: &ProfileSession,
    profile: &str,
    store: &mut (impl CheckpointStore + ?Sized),
    digest: &mut Sha256,
    artifacts: &mut Vec<ResetArtifactSummary>,
) -> Result<()> {
    let rollback_key = rollback_record_key(profile)?;
    let mut database_ids = Vec::new();
    if session.paths.hard_database.exists() {
        if let Ok(checkpoint) = session.rollback_checkpoint() {
            database_ids.push(checkpoint.database_id);
        }
    }
    let rollback = match store.get(&rollback_key) {
        Ok(value) => {
            let checkpoint: RollbackCheckpoint = serde_json::from_slice(&value)?;
            if checkpoint.profile != profile {
                return Err(Error::InvalidConfig(
                    "external checkpoint profile binding changed",
                ));
            }
            database_ids.push(checkpoint.database_id);
            Some(value)
        }
        Err(foks_keystore::Error::Missing) => None,
        Err(error) => return Err(error.into()),
    };
    hash_external_reset_record(
        ResetArtifactKind::ExternalRollbackCheckpoint,
        &rollback_key,
        rollback.as_ref().map(|value| value.as_slice()),
        digest,
        artifacts,
    );
    database_ids.sort_unstable();
    database_ids.dedup();
    for database_id in database_ids {
        let key = database_claim_record_key(&database_id);
        let claim = match store.get(&key) {
            Ok(value) if value.as_slice() == profile.as_bytes() => Some(value),
            Ok(_) | Err(foks_keystore::Error::Missing) => None,
            Err(error) => return Err(error.into()),
        };
        hash_external_reset_record(
            ResetArtifactKind::ExternalDatabaseClaim,
            &key,
            claim.as_ref().map(|value| value.as_slice()),
            digest,
            artifacts,
        );
    }
    let readiness_key = crate::portability::readiness::key(profile)?;
    let readiness = match store.get(&readiness_key) {
        Ok(value) => Some(value),
        Err(foks_keystore::Error::Missing) => None,
        Err(e) => return Err(e.into()),
    };
    hash_external_reset_record(
        ResetArtifactKind::ExternalImportReadiness,
        &readiness_key,
        readiness.as_ref().map(|v| v.as_slice()),
        digest,
        artifacts,
    );
    let authorization_key = profile_publication_authorization_key(profile)?;
    let authorization = match store.get(&authorization_key) {
        Ok(value) => Some(value),
        Err(foks_keystore::Error::Missing) => None,
        Err(error) => return Err(error.into()),
    };
    hash_external_reset_record(
        ResetArtifactKind::ExternalPublicationAuthorization,
        &authorization_key,
        authorization.as_ref().map(|value| value.as_slice()),
        digest,
        artifacts,
    );
    Ok(())
}

fn hash_external_reset_record(
    kind: ResetArtifactKind,
    key: &str,
    value: Option<&[u8]>,
    digest: &mut Sha256,
    artifacts: &mut Vec<ResetArtifactSummary>,
) {
    digest.update([kind as u8]);
    digest.update((key.len() as u64).to_le_bytes());
    digest.update(key.as_bytes());
    match value {
        Some(value) => {
            digest.update([1]);
            digest.update((value.len() as u64).to_le_bytes());
            digest.update(value);
            artifacts.push(ResetArtifactSummary {
                kind,
                entries: 1,
                bytes: value.len() as u64,
            });
        }
        None => digest.update([0]),
    }
}

fn hash_reset_artifact(
    root: &Path,
    path: &Path,
    digest: &mut Sha256,
    entries: &mut u64,
    bytes: &mut u64,
) -> Result<()> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| Error::InvalidConfig("reset artifact escaped the profile"))?;
    let relative = relative
        .to_str()
        .ok_or(Error::InvalidConfig("reset artifact path is not UTF-8"))?;
    digest.update((relative.len() as u64).to_le_bytes());
    digest.update(relative.as_bytes());
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            digest.update([0]);
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() {
        return Err(Error::InvalidConfig("reset artifact path is a symlink"));
    }
    *entries = entries
        .checked_add(1)
        .ok_or(Error::InvalidConfig("reset artifact count overflow"))?;
    if metadata.is_dir() {
        digest.update([1]);
        let mut children = fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
        children.sort_by_key(std::fs::DirEntry::file_name);
        for child in children {
            hash_reset_artifact(root, &child.path(), digest, entries, bytes)?;
        }
        return Ok(());
    }
    if !metadata.is_file() {
        return Err(Error::InvalidConfig("reset artifact path is not a file"));
    }
    digest.update([2]);
    digest.update(metadata.len().to_le_bytes());
    *bytes = bytes
        .checked_add(metadata.len())
        .ok_or(Error::InvalidConfig("reset artifact size overflow"))?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(())
}

fn remove_reset_artifact(path: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() {
        return Err(Error::InvalidConfig("reset artifact path is a symlink"));
    }
    if metadata.is_dir() {
        let mut children = fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
        children.sort_by_key(std::fs::DirEntry::file_name);
        for child in children {
            remove_reset_artifact(&child.path())?;
        }
        fs::remove_dir(path)?;
    } else if metadata.is_file() {
        fs::remove_file(path)?;
    } else {
        return Err(Error::InvalidConfig("reset artifact path is not a file"));
    }
    Ok(())
}

/// Removes a profile directory once its artifacts have been deleted. Symlinks
/// are rejected at each directory level to prevent traversal outside the root.
/// Callers must release operation and scheduler locks before calling.
pub(super) fn remove_profile_directory(directory: &Path) -> Result<()> {
    let parent = directory
        .parent()
        .ok_or(Error::InvalidConfig("profile directory has no parent"))?;
    remove_reset_artifact(directory)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

fn stage_reset_directory(active: &Path, quarantine: &Path) -> Result<()> {
    if quarantine.exists() {
        remove_reset_artifact(quarantine)?;
    }
    match fs::symlink_metadata(active) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            fs::rename(active, quarantine)?;
        }
        Ok(_) => return Err(Error::InvalidConfig("reset directory path is unsafe")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

pub(super) fn profile_publication_authorization_key(profile: &str) -> Result<String> {
    validate_name(profile)?;
    Ok(format!(
        "{PROFILE_PUBLICATION_AUTHORIZATION_PREFIX}{profile}"
    ))
}

pub(super) fn begin_profile_publication_with_store(
    profile: &str,
    store: &mut impl CheckpointStore,
) -> Result<[u8; 32]> {
    let authorization = random_array()?;
    store.put(
        &profile_publication_authorization_key(profile)?,
        &authorization,
    )?;
    Ok(authorization)
}

pub(super) fn profile_publication_is_authorized_with_store(
    profile: &str,
    authorization: &[u8; 32],
    marker_digest: &[u8; 32],
    store: &mut impl CheckpointStore,
) -> Result<bool> {
    let mut expected = [0_u8; 64];
    expected[..32].copy_from_slice(authorization);
    expected[32..].copy_from_slice(marker_digest);
    match store.get(&profile_publication_authorization_key(profile)?) {
        Ok(stored) => Ok(stored.as_slice() == expected),
        Err(foks_keystore::Error::Missing) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn bind_profile_publication_with_store(
    profile: &str,
    authorization: &[u8; 32],
    marker_digest: &[u8; 32],
    store: &mut impl CheckpointStore,
) -> Result<()> {
    let key = profile_publication_authorization_key(profile)?;
    let stored = store.get(&key)?;
    if stored.as_slice() != authorization {
        return Err(Error::InvalidConfig(
            "profile publication authorization changed",
        ));
    }
    let mut bound = Zeroizing::new([0_u8; 64]);
    bound[..32].copy_from_slice(authorization);
    bound[32..].copy_from_slice(marker_digest);
    store.put(&key, bound.as_slice())?;
    Ok(())
}

pub(super) fn cancel_profile_publication_with_store(
    profile: &str,
    authorization: &[u8; 32],
    store: &mut impl CheckpointStore,
) -> Result<()> {
    let key = profile_publication_authorization_key(profile)?;
    match store.get(&key) {
        Ok(stored)
            if stored.len() >= authorization.len()
                && &stored[..authorization.len()] == authorization =>
        {
            store.remove(&key)?;
        }
        Ok(_) | Err(foks_keystore::Error::Missing) => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn checked_profile_for_use<'a>(
    credentials: &ClientCredentials,
    session: &'a ProfileSession,
) -> Result<CheckedProfileSession<'a>> {
    let key = crate::portability::readiness::key(&session.profile.name)?;
    if credentials.backend == CredentialBackend::Native
        && credentials.with_native_manifest(|m| Ok(m.records.contains_key(&key)))?
    {
        return Err(Error::ImportVerificationRequired);
    }
    if foks_client_db::HardStateStore::open(&session.paths.hard_database)?
        .requires_import_verification()?
    {
        return Err(Error::ImportVerificationRequired);
    }
    Ok(CheckedProfileSession { session })
}

pub(super) trait CheckpointStore {
    fn put(&mut self, key: &str, value: &[u8]) -> foks_keystore::Result<()>;
    fn get(&mut self, key: &str) -> foks_keystore::Result<Zeroizing<Vec<u8>>>;
    fn remove(&mut self, key: &str) -> foks_keystore::Result<bool>;
}

#[cfg(test)]
impl CheckpointStore for foks_keystore::MemorySecretStore {
    fn put(&mut self, key: &str, value: &[u8]) -> foks_keystore::Result<()> {
        foks_keystore::SecretStore::put(self, key, value)
    }

    fn get(&mut self, key: &str) -> foks_keystore::Result<Zeroizing<Vec<u8>>> {
        foks_keystore::SecretStore::get(self, key)
    }

    fn remove(&mut self, key: &str) -> foks_keystore::Result<bool> {
        foks_keystore::SecretStore::remove(self, key)
    }
}

fn release_database_locks(locks: Vec<runtime::DatabaseLock>) -> Result<()> {
    let mut first_error = None;
    for lock in locks.into_iter().rev() {
        if let Err(error) = lock.release() {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

pub(super) fn database_claim_record_key(database_id: &[u8; 16]) -> String {
    format!("database.{}.profile-v1", hex(database_id))
}

fn remove_database_claim_if_owned(
    store: &mut impl CheckpointStore,
    database_id: [u8; 16],
    profile: &str,
) -> Result<()> {
    let key = database_claim_record_key(&database_id);
    match store.get(&key) {
        Ok(owner) if owner.as_slice() == profile.as_bytes() => {
            store.remove(&key)?;
        }
        Ok(_) | Err(foks_keystore::Error::Missing) => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn remove_external_reset_records(
    store: &mut impl CheckpointStore,
    profile: &str,
    database_ids: &[[u8; 16]],
) -> Result<()> {
    for database_id in database_ids {
        remove_database_claim_if_owned(store, *database_id, profile)?;
    }
    store.remove(&crate::portability::readiness::key(profile)?)?;
    store.remove(&rollback_record_key(profile)?)?;
    store.remove(&profile_publication_authorization_key(profile)?)?;
    Ok(())
}

#[cfg(test)]
mod reset_tests {
    use super::*;
    use foks_keystore::{MemorySecretStore, SecretStore};

    fn session(root: &Path) -> ProfileSession {
        let mut registry = ProfileRegistry::open(root).unwrap();
        registry
            .add(Profile {
                name: "local".to_owned(),
                label: None,
                probe: "foks.app".to_owned(),
                protocol: ProtocolPolicy::V019,
                trust: TrustRoot::WebPki,
            })
            .unwrap();
        ProfileSession::open(&registry, "local").unwrap()
    }

    #[test]
    fn external_reset_preview_binds_and_removes_only_owned_records() {
        let temporary = tempfile::tempdir().unwrap();
        let session = session(temporary.path());
        let checkpoint = session.rollback_checkpoint().unwrap();
        let rollback_key = rollback_record_key("local").unwrap();
        let claim_key = database_claim_record_key(&checkpoint.database_id);
        let authorization_key = profile_publication_authorization_key("local").unwrap();
        let mut store = MemorySecretStore::default();
        SecretStore::put(
            &mut store,
            &rollback_key,
            &serde_json::to_vec(&checkpoint).unwrap(),
        )
        .unwrap();
        SecretStore::put(&mut store, &claim_key, b"local").unwrap();
        SecretStore::put(&mut store, &authorization_key, &[7; 64]).unwrap();

        let mut digest = Sha256::new();
        let mut artifacts = Vec::new();
        hash_external_reset_state(&session, "local", &mut store, &mut digest, &mut artifacts)
            .unwrap();
        assert!(artifacts
            .iter()
            .any(|artifact| { artifact.kind == ResetArtifactKind::ExternalRollbackCheckpoint }));
        assert!(artifacts
            .iter()
            .any(|artifact| artifact.kind == ResetArtifactKind::ExternalDatabaseClaim));
        assert!(artifacts.iter().any(|artifact| {
            artifact.kind == ResetArtifactKind::ExternalPublicationAuthorization
        }));

        remove_external_reset_records(&mut store, "local", &[checkpoint.database_id]).unwrap();
        assert!(matches!(
            SecretStore::get(&mut store, &rollback_key),
            Err(foks_keystore::Error::Missing)
        ));
        assert!(matches!(
            SecretStore::get(&mut store, &claim_key),
            Err(foks_keystore::Error::Missing)
        ));
        assert!(matches!(
            SecretStore::get(&mut store, &authorization_key),
            Err(foks_keystore::Error::Missing)
        ));

        SecretStore::put(&mut store, &claim_key, b"another-profile").unwrap();
        remove_external_reset_records(&mut store, "local", &[checkpoint.database_id]).unwrap();
        assert_eq!(
            SecretStore::get(&mut store, &claim_key).unwrap().as_slice(),
            b"another-profile"
        );
    }

    #[test]
    fn external_reset_preview_rejects_a_cross_profile_checkpoint_binding() {
        let temporary = tempfile::tempdir().unwrap();
        let session = session(temporary.path());
        let mut checkpoint = session.rollback_checkpoint().unwrap();
        checkpoint.profile = "other".to_owned();
        let mut store = MemorySecretStore::default();
        SecretStore::put(
            &mut store,
            &rollback_record_key("local").unwrap(),
            &serde_json::to_vec(&checkpoint).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            hash_external_reset_state(
                &session,
                "local",
                &mut store,
                &mut Sha256::new(),
                &mut Vec::new(),
            ),
            Err(Error::InvalidConfig(
                "external checkpoint profile binding changed"
            ))
        ));
    }
}

fn rollback_reason(error: &Error) -> &'static str {
    match error {
        Error::RollbackDetected(reason) => reason,
        _ => "external checkpoint is invalid",
    }
}

pub(super) fn hard_state_artifact_paths(database: &Path) -> [PathBuf; 4] {
    let sidecar = |suffix: &str| {
        let mut path = OsString::from(database.as_os_str());
        path.push(suffix);
        PathBuf::from(path)
    };
    [
        database.to_owned(),
        sidecar("-wal"),
        sidecar("-shm"),
        sidecar("-journal"),
    ]
}

pub(super) fn hard_state_artifacts_exist(database: &Path) -> Result<bool> {
    for path in hard_state_artifact_paths(database) {
        match fs::symlink_metadata(path) {
            Ok(_) => return Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(false)
}

pub(super) fn rollback_record_key(profile: &str) -> Result<String> {
    validate_name(profile)?;
    Ok(format!("rollback.{profile}"))
}

#[cfg(test)]
mod manifest_tests {
    use super::*;

    #[test]
    fn native_manifest_round_trips_all_logical_records_in_one_value() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let mut manifest = NativeManifestStore::initialized(&root, &[7; 32]);
        manifest.put("rollback.local", b"checkpoint").unwrap();
        manifest
            .put("database.0123456789abcdef.profile-v1", b"local")
            .unwrap();

        let encoded = manifest.encode().unwrap();
        let mut decoded = NativeManifestStore::decode(&encoded).unwrap();
        assert_eq!(decoded.get(MASTER_KEY_RECORD).unwrap().as_slice(), &[7; 32]);
        assert_eq!(
            decoded.get(STATE_ROOT_RECORD).unwrap().as_slice(),
            state_root_binding(&root)
        );
        assert_eq!(
            decoded.get("rollback.local").unwrap().as_slice(),
            b"checkpoint"
        );
        assert_eq!(decoded.records.len(), 4);
        assert!(!decoded.dirty);
    }

    #[test]
    fn native_manifest_try_path_skips_root_wide_contention() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let credentials = ClientCredentials {
            lease: ClientStateLease::acquire(root.clone()).unwrap(),
            root: root.clone(),
            state_id: "unused-test-state".to_owned(),
            backend: CredentialBackend::Native,
        };
        let lock = runtime::NativeManifestLock::acquire(&credentials.state_id).unwrap();
        assert!(credentials
            .try_with_native_manifest(|_| Ok(()))
            .unwrap()
            .is_none());
        lock.release().unwrap();
    }

    #[test]
    fn native_manifest_rejects_unknown_versions_fields_keys_and_sizes() {
        assert!(NativeManifestStore::decode(br#"{"version":2,"records":{}}"#).is_err());
        assert!(
            NativeManifestStore::decode(br#"{"version":1,"records":{},"unexpected":true}"#)
                .is_err()
        );
        assert!(
            NativeManifestStore::decode(br#"{"version":1,"records":{"../escape":[1]}}"#).is_err()
        );

        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let mut manifest = NativeManifestStore::initialized(&root, &[7; 32]);
        assert!(matches!(
            manifest.put(
                "rollback.local",
                &vec![0; MAXIMUM_NATIVE_MANIFEST_VALUE_BYTES + 1]
            ),
            Err(foks_keystore::Error::TooLarge)
        ));
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RollbackCheckpoint {
    pub(super) profile: String,
    pub(super) database_id: [u8; 16],
    pub(super) hard_state_revision: u64,
    pub(super) write_token: [u8; 16],
    pub(super) host: Option<RollbackHostCheckpoint>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct RollbackHostCheckpoint {
    pub(super) host_id: Vec<u8>,
    pub(super) host_chain_sequence: u64,
    pub(super) host_chain_tail: [u8; 32],
    #[serde(skip, default)]
    pub(super) host_chain_bytes: Vec<u8>,
    pub(super) merkle_epoch: u64,
    pub(super) merkle_root_hash: [u8; 32],
    #[serde(skip, default)]
    pub(super) authenticated_roots: BTreeMap<u64, [u8; 32]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CheckpointReconciliation {
    Current,
    AdvanceExternal,
}

impl RollbackCheckpoint {
    /// Stable non-secret digest of the published checkpoint fields.
    pub fn digest(&self) -> Result<[u8; 32]> {
        Ok(prefixed_hash(
            0x6c18_37e9_5d04_afe2,
            &serde_json::to_vec(self)?,
        ))
    }

    pub fn verify_descends_from(&self, previous: &Self) -> Result<()> {
        self.reconciliation(previous).map(|_| ())
    }

    pub(super) fn reconciliation(&self, previous: &Self) -> Result<CheckpointReconciliation> {
        if self.profile != previous.profile {
            return Err(Error::RollbackDetected("checkpoint profile changed"));
        }
        if self.database_id != previous.database_id {
            return Err(Error::RollbackDetected(
                "hard-state database identity changed",
            ));
        }
        if self.hard_state_revision < previous.hard_state_revision {
            return Err(Error::RollbackDetected("hard-state revision rolled back"));
        }
        let previous_hard_state_revision = previous.hard_state_revision;
        if self.hard_state_revision == previous_hard_state_revision
            && self.write_token != previous.write_token
        {
            return Err(Error::RollbackDetected(
                "hard-state write token changed at the same revision",
            ));
        }
        if self.hard_state_revision > previous_hard_state_revision
            && self.write_token == previous.write_token
        {
            return Err(Error::RollbackDetected(
                "hard-state write token did not advance",
            ));
        }
        let Some(previous) = previous.host.as_ref() else {
            return Ok(if self.hard_state_revision > previous_hard_state_revision {
                CheckpointReconciliation::AdvanceExternal
            } else {
                CheckpointReconciliation::Current
            });
        };
        let Some(current) = self.host.as_ref() else {
            return Err(Error::RollbackDetected("pinned host is absent"));
        };
        if current.host_id != previous.host_id {
            return Err(Error::RollbackDetected("host identity changed"));
        }
        if current.host_chain_sequence < previous.host_chain_sequence
            || !foks_verify::hostchain_contains_tail(
                &current.host_chain_bytes,
                previous.host_chain_sequence,
                previous.host_chain_tail,
            )?
        {
            return Err(Error::RollbackDetected("hostchain checkpoint is absent"));
        }
        if current.merkle_epoch < previous.merkle_epoch
            || current.authenticated_roots.get(&previous.merkle_epoch)
                != Some(&previous.merkle_root_hash)
        {
            return Err(Error::RollbackDetected("Merkle checkpoint is absent"));
        }
        Ok(if self.hard_state_revision > previous_hard_state_revision {
            CheckpointReconciliation::AdvanceExternal
        } else {
            CheckpointReconciliation::Current
        })
    }
}

impl ClientStateMaintenanceGuard {
    /// A guarded native transaction. An error discards the in-memory replacement;
    /// durable phases each use a separate successful transaction.
    pub(crate) fn publish_new_native_manifest(&self, manifest: &NativeManifestStore) -> Result<()> {
        let id = self.namespace_id()?;
        let _lock = runtime::NativeManifestLock::acquire(id)?;
        let mut native = foks_keystore::NativeCredentialStore::open(id)?;
        match native.get(NATIVE_MANIFEST_RECORD) {
            Err(foks_keystore::Error::Missing) => {}
            Ok(_) => return Err(Error::StateRecoveryRequired),
            Err(e) => return Err(e.into()),
        }
        native.put(NATIVE_MANIFEST_RECORD, &manifest.encode()?)?;
        Ok(())
    }
    pub(crate) fn with_native_manifest<T>(
        &self,
        operation: impl FnOnce(&mut NativeManifestStore) -> Result<T>,
    ) -> Result<T> {
        let id = self.namespace_id()?;
        let _lock = runtime::NativeManifestLock::acquire(id)?;
        let mut native = foks_keystore::NativeCredentialStore::open(id)?;
        let mut manifest = NativeManifestStore::decode(&native.get(NATIVE_MANIFEST_RECORD)?)?;
        let result = operation(&mut manifest)?;
        ClientCredentials::persist_native_manifest(&mut native, &mut manifest)?;
        Ok(result)
    }
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    use crate::{CredentialBackend, Profile, ProfileRegistry, ProtocolPolicy, TrustRoot};

    #[test]
    fn paired_try_admission_releases_first_lock_and_never_enters_body_on_contention() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let credentials =
            ClientCredentials::initialize(&root, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&root).unwrap();
        for name in ["a", "b"] {
            registry
                .add(Profile {
                    name: name.into(),
                    label: None,
                    probe: "example.test".into(),
                    protocol: ProtocolPolicy::V019,
                    trust: TrustRoot::WebPki,
                })
                .unwrap();
        }
        let first = ProfileSession::open(&registry, "a").unwrap();
        let second = ProfileSession::open(&registry, "b").unwrap();
        let external = runtime::ProfileLock::operation(second.paths()).unwrap();
        assert_eq!(
            credentials
                .try_with_checked_sessions(&second, &first, |_, _| {
                    panic!("contended operation must not execute");
                    #[allow(unreachable_code)]
                    Ok::<_, Error>(())
                })
                .unwrap(),
            None
        );
        // Failure on the second lock must not retain the first profile.
        assert!(runtime::ProfileLock::try_operation(first.paths())
            .unwrap()
            .is_some());
        drop(external);
        let mut entered = 0;
        let result = credentials.try_with_checked_sessions(&second, &first, |left, right| {
            entered += 1;
            assert_eq!(left.profile().name, "b");
            assert_eq!(right.profile().name, "a");
            Err::<(), _>(Error::InvalidConfig("operation failed after admission"))
        });
        assert!(matches!(
            result,
            Err(Error::InvalidConfig("operation failed after admission"))
        ));
        assert_eq!(entered, 1);
        assert!(runtime::ProfileLock::try_operation(first.paths())
            .unwrap()
            .is_some());
        assert!(runtime::ProfileLock::try_operation(second.paths())
            .unwrap()
            .is_some());
    }
}
