use super::*;
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
    pub(super) root: PathBuf,
    pub(super) state_id: String,
    pub(super) backend: CredentialBackend,
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
                let initialized = (|| {
                    native.put(MASTER_KEY_RECORD, &*master)?;
                    native.put(STATE_ROOT_RECORD, &state_root_binding(&root))?;
                    Ok::<_, foks_keystore::Error>(())
                })();
                if let Err(error) = initialized {
                    let _ = native.remove(MASTER_KEY_RECORD);
                    let _ = native.remove(STATE_ROOT_RECORD);
                    return Err(error.into());
                }
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
                        let _ = native.remove(STATE_ROOT_RECORD);
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
        let credentials = Self {
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

    pub fn master_key(&self) -> Result<Zeroizing<[u8; 32]>> {
        match self.backend {
            CredentialBackend::Native => {
                self.verify_native_root_binding()?;
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

    fn verify_native_root_binding(&self) -> Result<()> {
        if self.backend != CredentialBackend::Native {
            return Ok(());
        }
        let mut native = foks_keystore::NativeCredentialStore::open(&self.state_id)?;
        self.verify_root_binding_with_store(&mut native)
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
        self.ensure_session_root(session).map_err(E::from)?;
        let lock = runtime::ProfileLock::operation(session.paths()).map_err(E::from)?;
        let database_lock = self.lock_and_verify_checkpoint(session).map_err(E::from)?;
        let checked = CheckedProfileSession { session };
        let result = operation(&checked);
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
        self.ensure_session_root(left).map_err(E::from)?;
        self.ensure_session_root(right).map_err(E::from)?;
        if left.paths.directory == right.paths.directory {
            return Err(E::from(Error::InvalidConfig(
                "cross-host operation requires two distinct profiles",
            )));
        }
        let (first, second) = if left.paths.directory < right.paths.directory {
            (left, right)
        } else {
            (right, left)
        };
        let first_lock = runtime::ProfileLock::operation(first.paths()).map_err(E::from)?;
        let second_lock = runtime::ProfileLock::operation(second.paths()).map_err(E::from)?;
        let database_locks = self
            .lock_and_verify_checkpoints(first, second)
            .map_err(E::from)?;

        let left_checked = CheckedProfileSession { session: left };
        let right_checked = CheckedProfileSession { session: right };
        let result = operation(&left_checked, &right_checked);

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
        result
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
        let checked = CheckedProfileSession { session };
        let result = operation(&checked);
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
            return Ok(None);
        }
        let allow_checkpoint_enrollment =
            !hard_state_artifacts_exist(&session.paths.hard_database)?;
        let current = session.rollback_checkpoint()?;
        let lock = runtime::DatabaseLock::acquire(&self.root, &current.database_id)?;
        self.verify_native_checkpoint(session, &current, allow_checkpoint_enrollment)?;
        Ok(Some(lock))
    }

    fn try_lock_and_verify_checkpoint(
        &self,
        session: &ProfileSession,
    ) -> Result<Option<Option<runtime::DatabaseLock>>> {
        if self.backend != CredentialBackend::Native {
            return Ok(Some(None));
        }
        let allow_checkpoint_enrollment =
            !hard_state_artifacts_exist(&session.paths.hard_database)?;
        let current = session.rollback_checkpoint()?;
        let Some(lock) = runtime::DatabaseLock::try_acquire(&self.root, &current.database_id)?
        else {
            return Ok(None);
        };
        self.verify_native_checkpoint(session, &current, allow_checkpoint_enrollment)?;
        Ok(Some(Some(lock)))
    }

    fn lock_and_verify_checkpoints(
        &self,
        first: &ProfileSession,
        second: &ProfileSession,
    ) -> Result<Vec<runtime::DatabaseLock>> {
        if self.backend != CredentialBackend::Native {
            return Ok(Vec::new());
        }
        let first_allows_enrollment = !hard_state_artifacts_exist(&first.paths.hard_database)?;
        let second_allows_enrollment = !hard_state_artifacts_exist(&second.paths.hard_database)?;
        let first_current = first.rollback_checkpoint()?;
        let second_current = second.rollback_checkpoint()?;
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
        let mut native = foks_keystore::NativeCredentialStore::open(&self.state_id)?;
        self.verify_native_checkpoint_with_store(
            first,
            &first_current,
            first_allows_enrollment,
            &mut native,
        )?;
        self.verify_native_checkpoint_with_store(
            second,
            &second_current,
            second_allows_enrollment,
            &mut native,
        )?;
        Ok(locks)
    }

    fn verify_native_checkpoint(
        &self,
        session: &ProfileSession,
        current: &RollbackCheckpoint,
        allow_checkpoint_enrollment: bool,
    ) -> Result<()> {
        let mut native = foks_keystore::NativeCredentialStore::open(&self.state_id)?;
        self.verify_native_checkpoint_with_store(
            session,
            current,
            allow_checkpoint_enrollment,
            &mut native,
        )
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

    fn advance_checkpoint(&self, session: &ProfileSession) -> Result<()> {
        if self.backend != CredentialBackend::Native {
            return Ok(());
        }
        let current = session.rollback_checkpoint()?;
        let mut native = foks_keystore::NativeCredentialStore::open(&self.state_id)?;
        self.verify_native_checkpoint_with_store(session, &current, false, &mut native)
    }

    /// Deliberately removes a profile's external rollback watermark and local
    /// hard-state database. The next checked operation enrolls a new database.
    pub fn reset_hard_state(&self, session: &ProfileSession) -> Result<()> {
        self.ensure_session_root(session)?;
        let operation = runtime::ProfileLock::operation(session.paths())?;
        let scheduler = runtime::ProfileLock::scheduler(session.paths())?;
        let current =
            if self.backend == CredentialBackend::Native && session.paths.hard_database.exists() {
                // Reset is the explicit recovery path for malformed hard
                // state. If its metadata cannot be read, no database-ID lock
                // or claim cleanup is possible; the profile locks still
                // serialize deletion and the unreachable claim is harmless.
                session.rollback_checkpoint().ok()
            } else {
                None
            };
        let database_lock = current
            .as_ref()
            .map(|checkpoint| runtime::DatabaseLock::acquire(&self.root, &checkpoint.database_id))
            .transpose()?;
        let result = self.reset_hard_state_locked(session, current.as_ref());
        let database_release = database_lock
            .map(runtime::DatabaseLock::release)
            .transpose();
        let scheduler_release = scheduler.release();
        let operation_release = operation.release();
        result?;
        database_release?;
        scheduler_release?;
        operation_release
    }

    fn ensure_session_root(&self, session: &ProfileSession) -> Result<()> {
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

    fn reset_hard_state_locked(
        &self,
        session: &ProfileSession,
        current: Option<&RollbackCheckpoint>,
    ) -> Result<()> {
        if self.backend == CredentialBackend::Native {
            let key = rollback_record_key(&session.profile.name)?;
            let mut native = foks_keystore::NativeCredentialStore::open(&self.state_id)?;
            if let Some(current) = current {
                remove_database_claim_if_owned(
                    &mut native,
                    current.database_id,
                    &session.profile.name,
                )?;
            }
            native.remove(&key)?;
        }
        remove_hard_state_artifacts(&session.paths.hard_database)?;
        if let Some(parent) = session.paths.hard_database.parent() {
            File::open(parent)?.sync_all()?;
        }
        Ok(())
    }
}

pub(super) trait CheckpointStore {
    fn put(&mut self, key: &str, value: &[u8]) -> foks_keystore::Result<()>;
    fn get(&mut self, key: &str) -> foks_keystore::Result<Zeroizing<Vec<u8>>>;
    fn remove(&mut self, key: &str) -> foks_keystore::Result<bool>;
}

impl CheckpointStore for foks_keystore::NativeCredentialStore {
    fn put(&mut self, key: &str, value: &[u8]) -> foks_keystore::Result<()> {
        foks_keystore::NativeCredentialStore::put(self, key, value)
    }

    fn get(&mut self, key: &str) -> foks_keystore::Result<Zeroizing<Vec<u8>>> {
        foks_keystore::NativeCredentialStore::get(self, key)
    }

    fn remove(&mut self, key: &str) -> foks_keystore::Result<bool> {
        foks_keystore::NativeCredentialStore::remove(self, key)
    }
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

fn hard_state_artifacts_exist(database: &Path) -> Result<bool> {
    for path in hard_state_artifact_paths(database) {
        match fs::symlink_metadata(path) {
            Ok(_) => return Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(false)
}

fn remove_hard_state_artifacts(database: &Path) -> Result<()> {
    for path in hard_state_artifact_paths(database) {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub(super) fn rollback_record_key(profile: &str) -> Result<String> {
    validate_name(profile)?;
    Ok(format!("rollback.{profile}"))
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
