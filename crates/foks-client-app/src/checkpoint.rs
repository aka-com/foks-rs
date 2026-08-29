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
        self.verify_checkpoint(session).map_err(E::from)?;
        let checked = CheckedProfileSession { session };
        let result = operation(&checked);
        let checkpoint = self.advance_checkpoint(session);
        let unlock = lock.release();
        checkpoint.map_err(E::from)?;
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
        self.verify_checkpoint(first).map_err(E::from)?;
        self.verify_checkpoint(second).map_err(E::from)?;

        let left_checked = CheckedProfileSession { session: left };
        let right_checked = CheckedProfileSession { session: right };
        let result = operation(&left_checked, &right_checked);

        // Perform every durability and release step before selecting which
        // error to report. An early return here could strand the other
        // profile's external watermark behind a committed SQLite revision.
        let left_checkpoint = self.advance_checkpoint(left);
        let right_checkpoint = self.advance_checkpoint(right);
        let second_release = second_lock.release();
        let first_release = first_lock.release();
        left_checkpoint.map_err(E::from)?;
        right_checkpoint.map_err(E::from)?;
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
        self.verify_checkpoint(session).map_err(E::from)?;
        let checked = CheckedProfileSession { session };
        let result = operation(&checked);
        let checkpoint = self.advance_checkpoint(session);
        let unlock = lock.release();
        checkpoint.map_err(E::from)?;
        unlock.map_err(E::from)?;
        result.map(Some)
    }

    fn verify_checkpoint(&self, session: &ProfileSession) -> Result<()> {
        if self.backend != CredentialBackend::Native {
            return Ok(());
        }
        let key = rollback_record_key(&session.profile.name)?;
        let mut native = foks_keystore::NativeCredentialStore::open(&self.state_id)?;
        self.verify_checkpoint_with_store(session, &key, &mut native)
    }

    pub(super) fn verify_checkpoint_with_store(
        &self,
        session: &ProfileSession,
        key: &str,
        store: &mut impl CheckpointStore,
    ) -> Result<()> {
        match store.get(key) {
            Ok(bytes) => {
                let previous: RollbackCheckpoint =
                    serde_json::from_slice(&bytes).map_err(|_| {
                        self.checkpoint_reset_error(session, "external checkpoint is invalid")
                    })?;
                let current = session.rollback_checkpoint()?;
                let reconciliation = current.reconciliation(&previous).map_err(|error| {
                    self.checkpoint_reset_error(session, rollback_reason(&error))
                })?;
                if reconciliation == CheckpointReconciliation::AdvanceExternal {
                    store.put(key, &serde_json::to_vec(&current)?)?;
                }
            }
            Err(foks_keystore::Error::Missing) => {
                if hard_state_artifacts_exist(&session.paths.hard_database)? {
                    return Err(
                        self.checkpoint_reset_error(session, "external checkpoint is missing")
                    );
                }
                let current = session.rollback_checkpoint()?;
                store.put(key, &serde_json::to_vec(&current)?)?;
            }
            Err(error) => return Err(error.into()),
        }
        Ok(())
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
        let key = rollback_record_key(&session.profile.name)?;
        let mut native = foks_keystore::NativeCredentialStore::open(&self.state_id)?;
        self.verify_checkpoint_with_store(session, &key, &mut native)
    }

    /// Deliberately removes a profile's external rollback watermark and local
    /// hard-state database. The next checked operation enrolls a new database.
    pub fn reset_hard_state(&self, session: &ProfileSession) -> Result<()> {
        self.ensure_session_root(session)?;
        let operation = runtime::ProfileLock::operation(session.paths())?;
        let scheduler = runtime::ProfileLock::scheduler(session.paths())?;
        let result = self.reset_hard_state_locked(session);
        let scheduler_release = scheduler.release();
        let operation_release = operation.release();
        result?;
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

    fn reset_hard_state_locked(&self, session: &ProfileSession) -> Result<()> {
        if self.backend == CredentialBackend::Native {
            let key = rollback_record_key(&session.profile.name)?;
            let mut native = foks_keystore::NativeCredentialStore::open(&self.state_id)?;
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
}

impl CheckpointStore for foks_keystore::NativeCredentialStore {
    fn put(&mut self, key: &str, value: &[u8]) -> foks_keystore::Result<()> {
        foks_keystore::NativeCredentialStore::put(self, key, value)
    }

    fn get(&mut self, key: &str) -> foks_keystore::Result<Zeroizing<Vec<u8>>> {
        foks_keystore::NativeCredentialStore::get(self, key)
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
