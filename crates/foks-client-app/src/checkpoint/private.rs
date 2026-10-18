//! A private-file checkpoint survives replacement of an individual profile.
//! It cannot detect a rollback of the entire state root, including this authority.
use super::*;

const DIRECTORY: &str = "checkpoint-authority";
const RECORD: &str = "authority-v1";
const MIGRATION: &str = ".checkpoint-authority-upgrade";
pub(super) const VERSION: u32 = 4;

fn key(root: &Path) -> Result<Zeroizing<[u8; 32]>> {
    let master = foks_keystore::load_master_key_file(root.join("master.key"))?;
    Ok(Zeroizing::new(prefixed_hash(
        0x7456_f989_6c7d_7e17,
        master.as_slice(),
    )))
}

fn store(root: &Path) -> Result<foks_keystore::EncryptedFileSecretStore> {
    Ok(foks_keystore::EncryptedFileSecretStore::inspect_existing(
        root.join(DIRECTORY),
        key(root)?,
    )?)
}

pub(super) fn initialize(root: &Path, state_id: &str) -> Result<()> {
    let master = foks_keystore::load_master_key_file(root.join("master.key"))?;
    let mut manifest = NativeManifestStore::initialized(root, &master);
    manifest.put("private-state-id", state_id.as_bytes())?;
    let mut target =
        foks_keystore::EncryptedFileSecretStore::open(root.join(DIRECTORY), key(root)?)?;
    match target.get(RECORD) {
        Err(foks_keystore::Error::Missing) => target.put(RECORD, &manifest.encode()?)?,
        Ok(_) => {
            return Err(Error::InvalidConfig(
                "private checkpoint authority already exists",
            ))
        }
        Err(error) => return Err(error.into()),
    }
    File::open(root)?.sync_all()?;
    Ok(())
}

fn read(
    root: &Path,
    state_id: &str,
) -> Result<(foks_keystore::EncryptedFileSecretStore, NativeManifestStore)> {
    let mut target = store(root)?;
    let mut manifest = NativeManifestStore::decode(&target.get(RECORD)?)?;
    if manifest.get(STATE_ROOT_RECORD)?.as_slice() != state_root_binding(root)
        || manifest.get("private-state-id")?.as_slice() != state_id.as_bytes()
    {
        return Err(Error::InvalidConfig(
            "private checkpoint authority belongs to another state root",
        ));
    }
    Ok((target, manifest))
}

pub(super) fn with_authority<T>(
    credentials: &ClientCredentials,
    try_lock: bool,
    operation: impl FnOnce(&mut NativeManifestStore) -> Result<T>,
) -> Result<Option<T>> {
    credentials.lease.validate()?;
    let state = inspect_state_file(&credentials.root)?.ok_or(Error::StatePathChanged)?;
    if state.credential_backend != CredentialBackend::PrivateFile
        || state.state_id != credentials.state_id
    {
        return Err(Error::StatePathChanged);
    }
    if state.version != VERSION {
        // An unavailable v3 key permits only metadata/recovery handles. Once
        // repaired, old handles must close so exclusive migration can finish.
        let _ = key(&credentials.root)?;
        return Err(Error::StateRecoveryRequired);
    }
    let lock = if try_lock {
        let Some(lock) = runtime::NativeManifestLock::try_acquire(&credentials.state_id)? else {
            return Ok(None);
        };
        lock
    } else {
        runtime::NativeManifestLock::acquire(&credentials.state_id)?
    };
    let (mut target, mut manifest) = read(&credentials.root, &credentials.state_id)?;
    let result = operation(&mut manifest);
    // Persist even when an admission error has advanced a rejection watermark.
    if manifest.dirty {
        manifest.generation = manifest
            .generation
            .checked_add(1)
            .ok_or(Error::InvalidConfig(
                "private checkpoint generation exhausted",
            ))?;
        target.put(RECORD, &manifest.encode()?)?;
    }
    lock.release()?;
    result.map(Some)
}

/// Runs before acquiring an ordinary lifetime lease. The exclusive path lease
/// also excludes old binaries, which do not know the authority's manifest lock.
pub(crate) fn upgrade_if_needed(root: &Path) -> Result<()> {
    let Some(state) = inspect_state_file(root)? else {
        return Ok(());
    };
    if state.credential_backend != CredentialBackend::PrivateFile || state.version == VERSION {
        return Ok(());
    }
    match key(root) {
        Ok(_) => (),
        Err(Error::Keystore(foks_keystore::Error::InvalidMasterKey)) => return Ok(()),
        Err(Error::Keystore(foks_keystore::Error::Io(error)))
            if error.kind() == std::io::ErrorKind::NotFound =>
        {
            return Ok(())
        }
        // Never mask unsafe paths, permissions, envelope or identity errors.
        Err(error) => return Err(error),
    }
    let _reservation = ClientStateMaintenanceGuard::acquire(&[root.to_owned()])?;
    let root = root.canonicalize()?;
    let state = inspect_state_file(&root)?.ok_or(Error::StatePathChanged)?;
    if state.version == VERSION {
        return Ok(());
    }
    if state.credential_backend != CredentialBackend::PrivateFile
        || state.version != STATE_CONFIG_VERSION
    {
        return Err(Error::StatePathChanged);
    }
    let marker = root.join(MIGRATION);
    match read_private_file_optional(&marker, 128)? {
        Some(bytes) if bytes == state.state_id.as_bytes() => (),
        Some(_) => return Err(Error::StatePathChanged),
        None => {
            // A preexisting authority without our durable migration intent is not
            // permission to enroll or replace it.
            if root.join(DIRECTORY).exists() {
                return Err(Error::StateRecoveryRequired);
            }
            create_private_config(&marker, state.state_id.as_bytes())?;
            File::open(&root)?.sync_all()?;
        }
    }
    if !root.join(DIRECTORY).join(format!("{RECORD}.fks")).exists() {
        initialize(&root, &state.state_id)?;
    }
    let (mut target, mut manifest) = read(&root, &state.state_id)?;
    for profile in registry::load_registry(&root)?.into_values() {
        let directory = root.join("profiles").join(&profile.name);
        let quarantine = reset_required_key(&profile.name)?;
        if let Some(digest) = registry::legacy_publication_digest(&root, &profile)? {
            // No database enrollment: the old publication's checkpoint may
            // precede a schema upgrade and its authorization is untrusted here.
            manifest.put(&quarantine, &digest)?;
        }
        match manifest.get(&quarantine) {
            Ok(_) => continue, // Durable refusal wins even if the marker vanished.
            Err(foks_keystore::Error::Missing) => (),
            Err(error) => return Err(error.into()),
        }
        let database = directory.join("hard.sqlite3");
        // An absent or uninitialized database has no identity to enroll; the
        // non-creating open never initializes one here.
        let hard = match HardStateStore::open_existing(&database) {
            Ok(hard) => hard,
            Err(foks_client_db::Error::StateMissing) => continue,
            Err(error) => return Err(error.into()),
        };
        let checkpoint = registry::checkpoint_for_store(&profile, &hard)?;
        let checkpoint_key = rollback_record_key(&profile.name)?;
        match manifest.get(&checkpoint_key) {
            Ok(previous) => checkpoint.verify_descends_from(&serde_json::from_slice(&previous)?)?,
            Err(foks_keystore::Error::Missing) => (),
            Err(error) => return Err(error.into()),
        }
        let claim = database_claim_record_key(&checkpoint.database_id);
        match manifest.get(&claim) {
            Ok(owner) if owner.as_slice() == profile.name.as_bytes() => (),
            Err(foks_keystore::Error::Missing) => (),
            _ => {
                return Err(Error::InvalidConfig(
                    "private checkpoint database is claimed by another profile",
                ))
            }
        }
        manifest.put(&claim, profile.name.as_bytes())?;
        manifest.put(&checkpoint_key, &serde_json::to_vec(&checkpoint)?)?;
    }
    target.put(RECORD, &manifest.encode()?)?;
    let updated = ClientStateFile {
        version: VERSION,
        ..state
    };
    atomic_private_write(
        &root.join(STATE_CONFIG_FILE),
        toml::to_string_pretty(&updated)?.as_bytes(),
    )?;
    fs::remove_file(marker)?;
    File::open(root)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(root: &Path) -> ProfileSession {
        let mut registry = ProfileRegistry::open(root).unwrap();
        registry
            .add(Profile {
                name: "local".into(),
                label: None,
                probe: "foks.app".into(),
                protocol: ProtocolPolicy::V019,
                trust: TrustRoot::WebPki,
            })
            .unwrap();
        ProfileSession::open(&registry, "local").unwrap()
    }

    #[test]
    fn private_authority_rejects_profile_only_rollback_and_missing_anchor() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let credentials =
            ClientCredentials::initialize(&root, CredentialBackend::PrivateFile).unwrap();
        let session = profile(&root);
        let database = &session.paths.hard_database;
        credentials
            .with_checked_session(&session, |_| Ok::<_, Error>(()))
            .unwrap();
        HardStateStore::open(database)
            .unwrap()
            .checkpoint_for_snapshot()
            .unwrap();
        let saved = temporary.path().join("old.sqlite3");
        fs::copy(database, &saved).unwrap();
        credentials
            .with_checked_session(&session, |_| {
                let mut hard = HardStateStore::open(database)?;
                let mut limits = hard.upload_limits()?;
                limits.max_pending += 1;
                hard.set_upload_limits(limits)?;
                Ok::<_, Error>(())
            })
            .unwrap();
        HardStateStore::open(database)
            .unwrap()
            .checkpoint_for_snapshot()
            .unwrap();
        fs::copy(saved, database).unwrap();
        let mut called = false;
        assert!(credentials
            .with_checked_session(&session, |_| {
                called = true;
                Ok::<_, Error>(())
            })
            .is_err());
        assert!(!called);
        drop(session);
        drop(credentials);
        fs::remove_dir_all(root.join(DIRECTORY)).unwrap();
        let reopened = ClientCredentials::open(&root).unwrap();
        let registry = ProfileRegistry::open(&root).unwrap();
        let session = ProfileSession::open(&registry, "local").unwrap();
        assert!(reopened
            .with_checked_session(&session, |_| {
                panic!("missing authority admitted operation");
                #[allow(unreachable_code)]
                Ok::<_, Error>(())
            })
            .is_err());
        assert!(!root.join(DIRECTORY).exists());
    }

    #[test]
    fn corrupt_private_authority_allows_recovery_handle_but_no_checked_operation() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let credentials =
            ClientCredentials::initialize(&root, CredentialBackend::PrivateFile).unwrap();
        let session = profile(&root);
        credentials
            .with_checked_session(&session, |_| Ok::<_, Error>(()))
            .unwrap();
        drop(session);
        drop(credentials);
        let record = root.join(DIRECTORY).join(format!("{RECORD}.fks"));
        let mut bytes = fs::read(&record).unwrap();
        let end = bytes.len() - 1;
        bytes[end] ^= 1;
        fs::write(&record, &bytes).unwrap();
        let reopened = ClientCredentials::open(&root).unwrap();
        let registry = ProfileRegistry::open(&root).unwrap();
        let session = ProfileSession::open(&registry, "local").unwrap();
        assert!(reopened
            .with_checked_session(&session, |_| {
                panic!("corrupt authority admitted operation");
                #[allow(unreachable_code)]
                Ok::<_, Error>(())
            })
            .is_err());
        assert_eq!(fs::read(&record).unwrap(), bytes);
    }

    #[test]
    fn private_v3_upgrade_resumes_durable_intent_and_enrolls_existing_profile() {
        for interrupted in [false, true] {
            let temporary = tempfile::tempdir().unwrap();
            let root = temporary.path().join("state");
            let credentials =
                ClientCredentials::initialize(&root, CredentialBackend::PrivateFile).unwrap();
            let session = profile(&root);
            credentials
                .with_checked_session(&session, |_| Ok::<_, Error>(()))
                .unwrap();
            let expected = session.rollback_checkpoint().unwrap().digest().unwrap();
            let state_id = credentials.state_id.clone();
            drop(session);
            drop(credentials);
            fs::remove_dir_all(root.join(DIRECTORY)).unwrap();
            let mut state = inspect_state_file(&root).unwrap().unwrap();
            state.version = STATE_CONFIG_VERSION;
            atomic_private_write(
                &root.join(STATE_CONFIG_FILE),
                toml::to_string(&state).unwrap().as_bytes(),
            )
            .unwrap();
            if interrupted {
                create_private_config(&root.join(MIGRATION), state_id.as_bytes()).unwrap();
                initialize(&root.canonicalize().unwrap(), &state_id).unwrap();
            }
            let migrated = ClientCredentials::open(&root).unwrap();
            assert_eq!(inspect_state_file(&root).unwrap().unwrap().version, VERSION);
            assert!(!root.join(MIGRATION).exists());
            let registry = ProfileRegistry::open(&root).unwrap();
            let session = ProfileSession::open(&registry, "local").unwrap();
            migrated
                .with_checked_session(&session, |s| {
                    assert_eq!(s.rollback_checkpoint()?.digest()?, expected);
                    Ok::<_, Error>(())
                })
                .unwrap();
        }
    }

    #[test]
    fn v3_unavailable_key_preserves_recovery_without_admitting_work_or_partial_upgrade() {
        for reset in [false, true] {
            let temporary = tempfile::tempdir().unwrap();
            let root = temporary.path().join("state");
            let credentials =
                ClientCredentials::initialize(&root, CredentialBackend::PrivateFile).unwrap();
            let session = profile(&root);
            credentials
                .with_checked_session(&session, |_| Ok::<_, Error>(()))
                .unwrap();
            let database = session.paths.hard_database.clone();
            HardStateStore::open(&database)
                .unwrap()
                .checkpoint_for_snapshot()
                .unwrap();
            let before = fs::read(&database).unwrap();
            let master = fs::read(root.join("master.key")).unwrap();
            drop(session);
            drop(credentials);
            fs::remove_dir_all(root.join(DIRECTORY)).unwrap();
            let mut state = inspect_state_file(&root).unwrap().unwrap();
            state.version = STATE_CONFIG_VERSION;
            atomic_private_write(
                &root.join(STATE_CONFIG_FILE),
                toml::to_string(&state).unwrap().as_bytes(),
            )
            .unwrap();
            fs::write(root.join("master.key"), b"short").unwrap();
            let credentials = ClientCredentials::open(&root).unwrap();
            let registry = ProfileRegistry::open(&root).unwrap();
            let session = ProfileSession::open(&registry, "local").unwrap();
            assert_eq!(fs::read(&database).unwrap(), before);
            assert!(!root.join(DIRECTORY).exists());
            assert!(!root.join(MIGRATION).exists());
            assert!(credentials
                .with_checked_session(&session, |_| {
                    panic!("v3 unavailable key admitted work");
                    #[allow(unreachable_code)]
                    Ok::<_, Error>(())
                })
                .is_err());
            if reset {
                let preview = credentials
                    .describe_reset_state_best_effort(&session)
                    .unwrap();
                let result = credentials
                    .reset_hard_state_best_effort_if_matches(&session, preview.state_digest())
                    .unwrap();
                assert!(result.credential_records_retained.is_some());
                assert!(!database.exists());
            }
            fs::write(root.join("master.key"), &master).unwrap();
            assert!(matches!(
                credentials.with_checked_session(&session, |_| Ok::<_, Error>(())),
                Err(Error::StateRecoveryRequired)
            ));
            assert_eq!(
                inspect_state_file(&root).unwrap().unwrap().version,
                STATE_CONFIG_VERSION
            );
            drop(session);
            drop(registry);
            drop(credentials);
            let credentials = ClientCredentials::open(&root).unwrap();
            assert_eq!(inspect_state_file(&root).unwrap().unwrap().version, VERSION);
            let registry = ProfileRegistry::open(&root).unwrap();
            let session = ProfileSession::open(&registry, "local").unwrap();
            credentials
                .with_checked_session(&session, |_| Ok::<_, Error>(()))
                .unwrap();
        }
    }

    #[test]
    fn v3_pending_publication_is_quarantined_until_explicit_profile_reset() {
        for partial_enrollment in [false, true] {
            let temporary = tempfile::tempdir().unwrap();
            let root = temporary.path().join("state");
            let credentials =
                ClientCredentials::initialize(&root, CredentialBackend::PrivateFile).unwrap();
            let pending = profile(&root);
            credentials
                .with_checked_session(&pending, |_| Ok::<_, Error>(()))
                .unwrap();
            let mut registry = ProfileRegistry::open(&root).unwrap();
            let mut sibling_profile = pending.profile.clone();
            sibling_profile.name = "sibling".into();
            registry.add(sibling_profile).unwrap();
            let sibling = ProfileSession::open(&registry, "sibling").unwrap();
            credentials
                .with_checked_session(&sibling, |_| Ok::<_, Error>(()))
                .unwrap();
            let sibling_checkpoint = sibling.rollback_checkpoint().unwrap();
            let pending_checkpoint = pending.rollback_checkpoint().unwrap();
            let marker = registry::profile_publication_marker(&pending);
            let marker_bytes = serde_json::to_vec(&serde_json::json!({
                "version": 1,
                "profile": pending.profile,
                "probe": ProbeReport {
                    acceptance: ProbeAcceptance::Inserted,
                    lookup_name: "foks.app".into(), canonical_name: "foks.app".into(),
                    host_id_hex: hex(&[0u8; 33]), host_chain_sequence: 1,
                    merkle_epoch: 1, server_version: None,
                },
                "checkpoint": pending_checkpoint,
                "authorization": vec![1u8; 32],
            }))
            .unwrap();
            create_private_config(&marker, &marker_bytes).unwrap();
            HardStateStore::open(&pending.paths.hard_database)
                .unwrap()
                .checkpoint_for_snapshot()
                .unwrap();
            let database_bytes = fs::read(&pending.paths.hard_database).unwrap();
            drop(sibling);
            drop(pending);
            drop(registry);
            drop(credentials);
            let mut state = inspect_state_file(&root).unwrap().unwrap();
            state.version = STATE_CONFIG_VERSION;
            atomic_private_write(
                &root.join(STATE_CONFIG_FILE),
                toml::to_string(&state).unwrap().as_bytes(),
            )
            .unwrap();
            if partial_enrollment {
                create_private_config(&root.join(MIGRATION), state.state_id.as_bytes()).unwrap();
            } else {
                fs::remove_dir_all(root.join(DIRECTORY)).unwrap();
            }
            // An unpublished staging directory retains the existing scanner's
            // cleanup policy; it is not enrolled or made into a profile.
            let staging = root.join("profiles/.pending-profile-unpublished");
            prepare_private_directory(&staging).unwrap();
            create_private_config(
                &staging.join(".profile-publication-v1"),
                b"profile-publication-v1",
            )
            .unwrap();
            let credentials = ClientCredentials::open(&root).unwrap();
            assert_eq!(inspect_state_file(&root).unwrap().unwrap().version, VERSION);
            assert_eq!(
                fs::read(root.join("profiles/local/hard.sqlite3")).unwrap(),
                database_bytes
            );
            assert!(staging.exists());
            let registry = ProfileRegistry::open(&root).unwrap();
            assert!(!staging.exists());
            assert!(matches!(
                registry.profile("unpublished"),
                Err(Error::ProfileMissing)
            ));
            let pending = ProfileSession::open(&registry, "local").unwrap();
            let sibling = ProfileSession::open(&registry, "sibling").unwrap();
            credentials
                .with_checked_session(&sibling, |_| Ok::<_, Error>(()))
                .unwrap();
            assert_eq!(sibling.rollback_checkpoint().unwrap(), sibling_checkpoint);
            assert!(credentials
                .with_checked_session(&pending, |_| {
                    panic!("pending publication admitted work");
                    #[allow(unreachable_code)]
                    Ok::<_, Error>(())
                })
                .is_err());
            // A disappearing local marker cannot erase the encrypted refusal.
            fs::remove_file(&marker).unwrap();
            assert!(credentials
                .with_checked_session(&pending, |_| Ok::<_, Error>(()))
                .is_err());
            assert!(credentials
                .try_with_shared_checked_session(&pending, |_| Ok::<_, Error>(()))
                .is_err());
            assert!(credentials
                .try_with_checked_sessions(&pending, &sibling, |_, _| Ok::<_, Error>(()))
                .is_err());
            let before = credentials
                .describe_reset_state_best_effort(&pending)
                .unwrap();
            credentials
                .with_native_manifest(|manifest| {
                    manifest.put(&reset_required_key("local")?, b"changed quarantine")?;
                    Ok(())
                })
                .unwrap();
            assert!(matches!(
                credentials
                    .reset_hard_state_best_effort_if_matches(&pending, before.state_digest()),
                Err(Error::ResetPreviewChanged)
            ));
            // Simulate migration interruption after authority publication, but
            // before the version write: retained quarantine must still win.
            drop(sibling);
            drop(pending);
            drop(registry);
            drop(credentials);
            atomic_private_write(
                &root.join(STATE_CONFIG_FILE),
                toml::to_string(&state).unwrap().as_bytes(),
            )
            .unwrap();
            create_private_config(&root.join(MIGRATION), state.state_id.as_bytes()).unwrap();
            let credentials = ClientCredentials::open(&root).unwrap();
            let registry = ProfileRegistry::open(&root).unwrap();
            let pending = ProfileSession::open(&registry, "local").unwrap();
            let sibling = ProfileSession::open(&registry, "sibling").unwrap();
            assert!(credentials
                .with_checked_session(&pending, |_| Ok::<_, Error>(()))
                .is_err());
            // Reproduce the durable staging boundary of an interrupted reset.
            // Quarantine survives; a new preview can finish the same reset.
            prepare_private_directory(&pending.paths.protected_mutations).unwrap();
            let staged = pending.paths.directory.join(RESET_MUTATION_QUARANTINE);
            stage_reset_directory(&pending.paths.protected_mutations, &staged).unwrap();
            File::open(&pending.paths.directory)
                .unwrap()
                .sync_all()
                .unwrap();
            assert!(staged.exists());
            assert!(credentials
                .with_checked_session(&pending, |_| Ok::<_, Error>(()))
                .is_err());
            let preview = credentials
                .describe_reset_state_best_effort(&pending)
                .unwrap();
            let result = credentials
                .reset_hard_state_best_effort_if_matches(&pending, preview.state_digest())
                .unwrap();
            assert!(result.credential_records_retained.is_none());
            assert!(!staged.exists());
            credentials
                .with_checked_session(&pending, |_| Ok::<_, Error>(()))
                .unwrap();
            assert_ne!(
                pending.rollback_checkpoint().unwrap().database_id,
                pending_checkpoint.database_id
            );
            credentials
                .with_checked_session(&sibling, |_| Ok::<_, Error>(()))
                .unwrap();
            assert_eq!(sibling.rollback_checkpoint().unwrap(), sibling_checkpoint);
        }
    }

    #[test]
    fn v3_upgrade_never_adopts_an_unattributed_existing_authority() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let credentials =
            ClientCredentials::initialize(&root, CredentialBackend::PrivateFile).unwrap();
        drop(credentials);
        let mut state = inspect_state_file(&root).unwrap().unwrap();
        state.version = STATE_CONFIG_VERSION;
        atomic_private_write(
            &root.join(STATE_CONFIG_FILE),
            toml::to_string(&state).unwrap().as_bytes(),
        )
        .unwrap();
        assert!(matches!(
            ClientCredentials::open(&root),
            Err(Error::StateRecoveryRequired)
        ));
    }
}
