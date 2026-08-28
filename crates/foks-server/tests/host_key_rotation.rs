use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::sync::atomic::{AtomicBool, Ordering};

use foks_client_db::{Acceptance, HardStateStore};
use foks_server::host::{
    begin_host_key_rotation, complete_host_key_rotation, load_or_bootstrap,
    stage_host_key_rotation, BootstrapEndpoints, BootstrapInput, HostKeyRotationObservation,
};
use foks_server::keys::{
    DirectoryKeyProvider, HostKeyProvider, KeyGenerationId, KeyPurpose, SecretKey,
};
use foks_server_db::{Config, Database, HostKeyGenerationState, HostRotationPhase, ReadDatabase};

fn input(now_microseconds: u64) -> BootstrapInput {
    BootstrapInput {
        canonical_name: "localhost".to_owned(),
        endpoints: BootstrapEndpoints {
            probe: "localhost:4430".to_owned(),
            public_services: "localhost:4431".to_owned(),
            authenticated: "localhost:4432".to_owned(),
        },
        ttl_seconds: 60,
        now_microseconds,
    }
}

fn observation(state: &foks_server::host::HostKeyRotationState) -> HostKeyRotationObservation {
    HostKeyRotationObservation {
        add_link_seqno: state.add_link_seqno.unwrap(),
    }
}

fn restore_and_validate(
    root: &std::path::Path,
    label: &str,
    artifacts: &foks_server::BackupArtifacts,
    root_key: [u8; 32],
    expected_chain_seqno: u64,
) {
    let database_path = root.join(format!("restored-{label}/server.sqlite"));
    let keys_path = root.join(format!("restored-{label}/keys"));
    foks_server::restore_backup(artifacts, &database_path, &keys_path, Config::default()).unwrap();
    let provider = DirectoryKeyProvider::open(&keys_path, root_key).unwrap();
    let mut database = Database::open(&database_path, Config::default()).unwrap();
    let state = load_or_bootstrap(&mut database, &provider, &input(999)).unwrap();
    assert_eq!(
        foks_verify::verify_public_host("localhost", &state.probe_response)
            .unwrap()
            .snapshot
            .chain_seqno(),
        expected_chain_seqno
    );
}

#[test]
fn pinned_clients_follow_add_and_revoke_without_repinning() {
    let temporary = tempfile::tempdir().unwrap();
    let database_path = temporary.path().join("server.sqlite");
    let keys_path = temporary.path().join("keys");
    let root_key = [0x55; 32];
    let provider = DirectoryKeyProvider::open(&keys_path, root_key).unwrap();
    let mut database = Database::open(&database_path, Config::default()).unwrap();
    let bootstrap = load_or_bootstrap(&mut database, &provider, &input(10)).unwrap();
    let original_host_id = bootstrap.host_id;
    let original_probe = bootstrap.probe_response;
    let original = foks_verify::verify_public_host("localhost", &original_probe).unwrap();
    let mut online = HardStateStore::open(&temporary.path().join("online.sqlite")).unwrap();
    let mut offline = HardStateStore::open(&temporary.path().join("offline.sqlite")).unwrap();
    assert_eq!(
        online.accept_verified_host(&original.snapshot).unwrap(),
        Acceptance::Inserted
    );
    assert_eq!(
        offline.accept_verified_host(&original.snapshot).unwrap(),
        Acceptance::Inserted
    );

    drop(provider);
    let provider = DirectoryKeyProvider::open_for_rotation(&keys_path, root_key).unwrap();
    let published = begin_host_key_rotation(&mut database, &provider, 20).unwrap();
    assert_eq!(published.phase, HostRotationPhase::Published);
    assert_eq!(published.add_link_seqno, Some(2));
    assert_eq!(
        published.old_public_entity_id.as_slice(),
        original_host_id.as_bytes()
    );
    assert_ne!(
        published.old_public_entity_id,
        published.new_public_entity_id
    );
    let probe = database.host_bootstrap().unwrap().unwrap().probe_response;
    let added = foks_verify::verify_public_host("localhost", &probe).unwrap();
    assert_eq!(added.snapshot.host_id(), original_host_id.as_bytes());
    assert_eq!(
        online.accept_verified_host(&added.snapshot).unwrap(),
        Acceptance::Advanced
    );

    let too_early = complete_host_key_rotation(
        &mut database,
        &provider,
        published.operation_id,
        observation(&published),
        published.observation_not_before.unwrap() - 1,
    )
    .unwrap_err();
    assert!(too_early.to_string().contains("interval has not elapsed"));
    let wrong_observation = complete_host_key_rotation(
        &mut database,
        &provider,
        published.operation_id,
        HostKeyRotationObservation {
            add_link_seqno: 999,
        },
        published.observation_not_before.unwrap(),
    )
    .unwrap_err();
    assert!(wrong_observation.to_string().contains("does not match"));

    let complete = complete_host_key_rotation(
        &mut database,
        &provider,
        published.operation_id,
        observation(&published),
        published.observation_not_before.unwrap(),
    )
    .unwrap();
    assert_eq!(complete.phase, HostRotationPhase::Complete);
    assert_eq!(complete.revoke_link_seqno, Some(3));
    assert!(!keys_path.join("host.key").exists());
    assert_eq!(
        complete_host_key_rotation(
            &mut database,
            &provider,
            published.operation_id,
            observation(&published),
            published.observation_not_before.unwrap() + 1,
        )
        .unwrap(),
        complete
    );

    let probe = database.host_bootstrap().unwrap().unwrap().probe_response;
    let revoked = foks_verify::verify_public_host("localhost", &probe).unwrap();
    assert_eq!(revoked.snapshot.host_id(), original_host_id.as_bytes());
    assert_eq!(
        online.accept_verified_host(&revoked.snapshot).unwrap(),
        Acceptance::Advanced
    );
    assert_eq!(
        offline.accept_verified_host(&revoked.snapshot).unwrap(),
        Acceptance::Advanced
    );

    drop(database);
    drop(provider);
    let provider = DirectoryKeyProvider::open(&keys_path, root_key).unwrap();
    let mut database = Database::open(&database_path, Config::default()).unwrap();
    let restarted = load_or_bootstrap(&mut database, &provider, &input(999)).unwrap();
    assert_eq!(restarted.host_id, original_host_id);
}

struct FailFirstRetirement {
    inner: DirectoryKeyProvider,
    fail: AtomicBool,
}

impl HostKeyProvider for FailFirstRetirement {
    fn load_or_create(&self, purpose: KeyPurpose) -> foks_server::Result<SecretKey> {
        self.inner.load_or_create(purpose)
    }

    fn load_existing(&self, purpose: KeyPurpose) -> foks_server::Result<SecretKey> {
        self.inner.load_existing(purpose)
    }

    fn create_generation(&self, purpose: KeyPurpose) -> foks_server::Result<SecretKey> {
        self.inner.create_generation(purpose)
    }

    fn load_generation(
        &self,
        purpose: KeyPurpose,
        generation: KeyGenerationId,
    ) -> foks_server::Result<SecretKey> {
        self.inner.load_generation(purpose, generation)
    }

    fn list_generations(&self, purpose: KeyPurpose) -> foks_server::Result<Vec<KeyGenerationId>> {
        self.inner.list_generations(purpose)
    }

    fn remove_generation(
        &self,
        purpose: KeyPurpose,
        generation: KeyGenerationId,
    ) -> foks_server::Result<()> {
        if self.fail.swap(false, Ordering::AcqRel) {
            return Err(foks_server::Error::Key(
                "injected post-commit key retirement failure",
            ));
        }
        self.inner.remove_generation(purpose, generation)
    }
}

#[test]
fn crash_recovery_resumes_staging_publication_and_key_retirement() {
    let temporary = tempfile::tempdir().unwrap();
    let database_path = temporary.path().join("server.sqlite");
    let keys_path = temporary.path().join("keys");
    let root_key = [0x45; 32];
    let provider = DirectoryKeyProvider::open(&keys_path, root_key).unwrap();
    let mut database = Database::open(&database_path, Config::default()).unwrap();
    load_or_bootstrap(&mut database, &provider, &input(1)).unwrap();
    drop(provider);

    let provider = DirectoryKeyProvider::open_for_rotation(&keys_path, root_key).unwrap();
    let orphan = provider.create_generation(KeyPurpose::Host).unwrap();
    let staged = stage_host_key_rotation(&mut database, &provider, 2).unwrap();
    assert_eq!(staged.phase, HostRotationPhase::Staged);
    assert!(provider
        .load_generation(KeyPurpose::Host, orphan.generation())
        .is_err());
    drop(database);
    drop(provider);

    let provider = DirectoryKeyProvider::open_for_rotation(&keys_path, root_key).unwrap();
    let mut database = Database::open(&database_path, Config::default()).unwrap();
    let published = begin_host_key_rotation(&mut database, &provider, 3).unwrap();
    assert_eq!(published.operation_id, staged.operation_id);
    assert_eq!(
        begin_host_key_rotation(&mut database, &provider, 4).unwrap(),
        published
    );

    let provider = FailFirstRetirement {
        inner: provider,
        fail: AtomicBool::new(true),
    };
    let completion_time = published.observation_not_before.unwrap();
    let error = complete_host_key_rotation(
        &mut database,
        &provider,
        published.operation_id,
        observation(&published),
        completion_time,
    )
    .unwrap_err();
    assert!(error.to_string().contains("post-commit"));
    assert_eq!(
        database
            .host_rotation_operation(published.operation_id)
            .unwrap()
            .unwrap()
            .phase,
        HostRotationPhase::Complete
    );
    assert!(keys_path.join("host.key").is_file());
    drop(database);
    drop(provider);

    let provider = DirectoryKeyProvider::open(&keys_path, root_key).unwrap();
    let mut database = Database::open(&database_path, Config::default()).unwrap();
    load_or_bootstrap(&mut database, &provider, &input(5)).unwrap();
    drop(database);
    drop(provider);
    let provider = DirectoryKeyProvider::open_for_rotation(&keys_path, root_key).unwrap();
    let mut database = Database::open(&database_path, Config::default()).unwrap();
    complete_host_key_rotation(
        &mut database,
        &provider,
        published.operation_id,
        observation(&published),
        completion_time + 1,
    )
    .unwrap();
    assert!(!keys_path.join("host.key").exists());
}

#[test]
fn backup_and_restore_cover_every_durable_rotation_phase() {
    let temporary = tempfile::tempdir().unwrap();
    let database_path = temporary.path().join("server.sqlite");
    let keys_path = temporary.path().join("keys");
    let root_key = [0x33; 32];
    let provider = DirectoryKeyProvider::open(&keys_path, root_key).unwrap();
    let mut database = Database::open(&database_path, Config::default()).unwrap();
    load_or_bootstrap(&mut database, &provider, &input(1)).unwrap();
    drop(provider);

    let provider = DirectoryKeyProvider::open_for_rotation(&keys_path, root_key).unwrap();
    let staged = stage_host_key_rotation(&mut database, &provider, 2).unwrap();
    drop(provider);
    let staged_backup = foks_server::backup_standalone_installation(
        &database_path,
        &keys_path,
        root_key,
        temporary.path().join("backup-staged"),
        Config::default(),
    )
    .unwrap();
    assert!(staged_backup.key_directory.join("host.key").is_file());
    restore_and_validate(temporary.path(), "staged", &staged_backup, root_key, 1);

    let provider = DirectoryKeyProvider::open_for_rotation(&keys_path, root_key).unwrap();
    let published = begin_host_key_rotation(&mut database, &provider, 3).unwrap();
    assert_eq!(published.operation_id, staged.operation_id);
    drop(provider);
    let published_backup = foks_server::backup_standalone_installation(
        &database_path,
        &keys_path,
        root_key,
        temporary.path().join("backup-published"),
        Config::default(),
    )
    .unwrap();
    assert!(published_backup.key_directory.join("host.key").is_file());
    restore_and_validate(
        temporary.path(),
        "published",
        &published_backup,
        root_key,
        2,
    );

    let provider = DirectoryKeyProvider::open_for_rotation(&keys_path, root_key).unwrap();
    complete_host_key_rotation(
        &mut database,
        &provider,
        published.operation_id,
        observation(&published),
        published.observation_not_before.unwrap(),
    )
    .unwrap();
    drop(provider);
    let complete_backup = foks_server::backup_standalone_installation(
        &database_path,
        &keys_path,
        root_key,
        temporary.path().join("backup-complete"),
        Config::default(),
    )
    .unwrap();
    assert!(!keys_path.join("host.key").exists());
    assert!(!complete_backup.key_directory.join("host.key").exists());
    restore_and_validate(temporary.path(), "complete", &complete_backup, root_key, 3);
}

#[test]
fn restored_rotation_state_rejects_secret_ledger_chain_probe_and_root_tampering() {
    let temporary = tempfile::tempdir().unwrap();
    let database_path = temporary.path().join("server.sqlite");
    let keys_path = temporary.path().join("keys");
    let root_key = [0x73; 32];
    let provider = DirectoryKeyProvider::open(&keys_path, root_key).unwrap();
    let mut database = Database::open(&database_path, Config::default()).unwrap();
    load_or_bootstrap(&mut database, &provider, &input(1)).unwrap();
    drop(provider);
    let provider = DirectoryKeyProvider::open_for_rotation(&keys_path, root_key).unwrap();
    let published = begin_host_key_rotation(&mut database, &provider, 2).unwrap();
    complete_host_key_rotation(
        &mut database,
        &provider,
        published.operation_id,
        observation(&published),
        published.observation_not_before.unwrap(),
    )
    .unwrap();
    drop(database);
    drop(provider);

    for (label, sql) in [
        (
            "ledger",
            "UPDATE host_key_generations SET public_entity_id = zeroblob(33) WHERE state = 4",
        ),
        (
            "hostchain",
            "UPDATE hostchain_links SET exact_link = X'00' WHERE seqno = 2",
        ),
        (
            "probe",
            "UPDATE host_metadata SET bootstrap_blob = X'00' WHERE singleton = 1",
        ),
        (
            "root",
            "UPDATE merkle_roots SET exact_root = X'00' WHERE epoch = (SELECT epoch FROM merkle_root_heads WHERE singleton = 1)",
        ),
    ] {
        let artifacts = foks_server::backup_standalone_installation(
            &database_path,
            &keys_path,
            root_key,
            temporary.path().join(format!("backup-tamper-{label}")),
            Config::default(),
        )
        .unwrap();
        let connection = rusqlite::Connection::open(&artifacts.database).unwrap();
        connection.execute(sql, []).unwrap();
        drop(connection);
        let restored_database = temporary.path().join(format!("tampered-{label}/server.sqlite"));
        let restored_keys = temporary.path().join(format!("tampered-{label}/keys"));
        foks_server::restore_backup(
            &artifacts,
            &restored_database,
            &restored_keys,
            Config::default(),
        )
        .unwrap();
        let provider = DirectoryKeyProvider::open(&restored_keys, root_key).unwrap();
        let mut database = Database::open(&restored_database, Config::default()).unwrap();
        assert!(load_or_bootstrap(&mut database, &provider, &input(99)).is_err());
    }

    let artifacts = foks_server::backup_standalone_installation(
        &database_path,
        &keys_path,
        root_key,
        temporary.path().join("backup-tamper-secret"),
        Config::default(),
    )
    .unwrap();
    let active = std::fs::read_dir(&artifacts.key_directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("host.") && name.ends_with(".key"))
        })
        .unwrap();
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(active)
        .unwrap();
    let mut first = [0; 1];
    file.read_exact(&mut first).unwrap();
    first[0] ^= 0xff;
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&first).unwrap();
    file.sync_all().unwrap();
    let restored_database = temporary.path().join("tampered-secret/server.sqlite");
    let restored_keys = temporary.path().join("tampered-secret/keys");
    foks_server::restore_backup(
        &artifacts,
        &restored_database,
        &restored_keys,
        Config::default(),
    )
    .unwrap();
    let provider = DirectoryKeyProvider::open(&restored_keys, root_key).unwrap();
    let mut database = Database::open(&restored_database, Config::default()).unwrap();
    assert!(load_or_bootstrap(&mut database, &provider, &input(99)).is_err());
}

#[test]
fn repeated_rotations_keep_only_public_history_and_one_live_private_key() {
    let temporary = tempfile::tempdir().unwrap();
    let database_path = temporary.path().join("server.sqlite");
    let keys_path = temporary.path().join("keys");
    let root_key = [0x24; 32];
    let provider = DirectoryKeyProvider::open(&keys_path, root_key).unwrap();
    let mut database = Database::open(&database_path, Config::default()).unwrap();
    load_or_bootstrap(&mut database, &provider, &input(1)).unwrap();
    drop(provider);

    let mut next_rotation_time = 10;
    for round in 0..3_u64 {
        let provider = DirectoryKeyProvider::open_for_rotation(&keys_path, root_key).unwrap();
        assert!(DirectoryKeyProvider::open_for_rotation(&keys_path, root_key).is_err());
        let published =
            begin_host_key_rotation(&mut database, &provider, next_rotation_time).unwrap();
        let completion_time = published.observation_not_before.unwrap();
        complete_host_key_rotation(
            &mut database,
            &provider,
            published.operation_id,
            observation(&published),
            completion_time,
        )
        .unwrap();
        next_rotation_time = completion_time + 1;
        drop(provider);
        let provider = DirectoryKeyProvider::open(&keys_path, root_key).unwrap();
        load_or_bootstrap(&mut database, &provider, &input(100 + round)).unwrap();
        drop(provider);
    }

    let generations = database.host_key_generations().unwrap();
    assert_eq!(generations.len(), 4);
    assert_eq!(
        generations
            .iter()
            .filter(|generation| generation.state == HostKeyGenerationState::Active)
            .count(),
        1
    );
    assert_eq!(
        generations
            .iter()
            .filter(|generation| generation.state == HostKeyGenerationState::Revoked)
            .count(),
        3
    );
    let host_secrets = std::fs::read_dir(&keys_path)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name == "host.key" || name.starts_with("host."))
        })
        .count();
    assert_eq!(host_secrets, 1);
}

#[test]
fn generated_key_files_are_immutable_generation_bound_and_redacted() {
    let temporary = tempfile::tempdir().unwrap();
    let provider = DirectoryKeyProvider::open_for_rotation(temporary.path(), [0x44; 32]).unwrap();
    let first = provider.create_generation(KeyPurpose::Host).unwrap();
    let second = provider.create_generation(KeyPurpose::Host).unwrap();
    assert_ne!(first.generation(), second.generation());
    assert_eq!(
        provider
            .load_generation(KeyPurpose::Host, first.generation())
            .unwrap()
            .expose(),
        first.expose()
    );
    let missing = KeyGenerationId::from_bytes([0xff; 16]);
    assert!(provider.load_generation(KeyPurpose::Host, missing).is_err());
    let rendered = format!("{first:?}");
    assert_eq!(rendered, "SecretKey([REDACTED])");
    assert!(!rendered.contains(&format!("{:?}", first.expose())));
}

#[test]
fn read_only_database_exposes_public_generation_and_hostchain_ledgers() {
    let temporary = tempfile::tempdir().unwrap();
    let database_path = temporary.path().join("server.sqlite");
    let provider = foks_server::keys::MemoryKeyProvider::default();
    let mut database = Database::open(&database_path, Config::default()).unwrap();
    load_or_bootstrap(&mut database, &provider, &input(1)).unwrap();
    drop(database);
    let read = ReadDatabase::open(&database_path, Config::default()).unwrap();
    let generations = read.host_key_generations().unwrap();
    assert_eq!(generations.len(), 1);
    assert_eq!(generations[0].state, HostKeyGenerationState::Active);
    assert_eq!(read.hostchain_links().unwrap().len(), 1);
}
