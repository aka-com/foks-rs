use foks_server::host::{load_or_bootstrap, BootstrapEndpoints, BootstrapInput};
use foks_server::keys::{
    retire_capability_keys, rotate_capability_key, validate_capability_key_generations,
    DirectoryKeyProvider, HostKeyProvider, KeyPurpose,
};
use foks_server_db::{CapabilityKeyGenerationState, Config, Database};
use rusqlite::params;
use std::sync::atomic::{AtomicBool, Ordering};

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

fn seed_live_permission(database_path: &std::path::Path, generation: [u8; 16]) {
    let connection = rusqlite::Connection::open(database_path).unwrap();
    let mut uid = vec![0x11; 33];
    uid[0] = foks_proto::ENTITY_USER;
    let mut viewer = vec![0x22; 33];
    viewer[0] = foks_proto::ENTITY_USER;
    let mut viewer_host = vec![0x33; 33];
    viewer_host[0] = foks_proto::ENTITY_HOST;
    connection
        .execute(
            "INSERT INTO names
             (normalized_name, reservation_token, reservation_sequence, expires_at, uid)
             VALUES (x'61', NULL, 1, NULL, ?1)",
            [&uid],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO users
             (uid, normalized_name, username_utf8, username_sequence,
              username_commitment_key, created_at)
             VALUES (?1, x'61', x'61', 1, zeroblob(16), 1)",
            [&uid],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO federation_user_view_permissions
             (target_user_id, viewer_party_id, viewer_host_id, token_hash,
              token_nonce, token_ciphertext, key_generation, state, issued_at,
              updated_at, expires_at, revoked_at)
             VALUES (?1, ?2, ?3, zeroblob(32), zeroblob(24), zeroblob(33),
                     ?4, 1, 2, 2, 100, NULL)",
            params![uid, viewer, viewer_host, generation],
        )
        .unwrap();
}

fn restore_and_start(
    root: &std::path::Path,
    label: &str,
    artifacts: &foks_server::BackupArtifacts,
    root_key: [u8; 32],
) -> std::path::PathBuf {
    let database_path = root.join(format!("restored-{label}/server.sqlite"));
    let key_directory = root.join(format!("restored-{label}/keys"));
    foks_server::restore_backup(artifacts, &database_path, &key_directory, Config::default())
        .unwrap();
    let provider = DirectoryKeyProvider::open(&key_directory, root_key).unwrap();
    let mut database = Database::open(&database_path, Config::default()).unwrap();
    load_or_bootstrap(&mut database, &provider, &input(999)).unwrap();
    key_directory
}

#[test]
fn rotation_retains_live_generations_and_backup_restores_each_phase() {
    let temporary = tempfile::tempdir().unwrap();
    let database_path = temporary.path().join("server.sqlite");
    let key_directory = temporary.path().join("keys");
    let root_key = [0x51; 32];
    let provider = DirectoryKeyProvider::open(&key_directory, root_key).unwrap();
    let mut database = Database::open(&database_path, Config::default()).unwrap();
    load_or_bootstrap(&mut database, &provider, &input(1)).unwrap();
    let genesis = database.active_capability_key_generation().unwrap();
    seed_live_permission(&database_path, genesis);
    drop(provider);

    let provider = DirectoryKeyProvider::open_for_rotation(&key_directory, root_key).unwrap();
    let rotated = rotate_capability_key(&mut database, &provider, 10).unwrap();
    assert_ne!(rotated.active_generation_id, genesis);
    assert_eq!(rotated.retiring_generation_ids, vec![genesis]);
    assert!(key_directory.join("capability.key").is_file());
    assert!(provider
        .load_generation(
            KeyPurpose::Capability,
            foks_server::keys::KeyGenerationId::from_bytes(genesis),
        )
        .is_ok());
    drop(provider);

    let retiring_backup = foks_server::backup_standalone_installation(
        &database_path,
        &key_directory,
        root_key,
        temporary.path().join("backup-retiring"),
        Config::default(),
    )
    .unwrap();
    assert!(retiring_backup
        .key_directory
        .join("capability.key")
        .is_file());
    let restored_retiring =
        restore_and_start(temporary.path(), "retiring", &retiring_backup, root_key);
    assert!(restored_retiring.join("capability.key").is_file());

    let provider = DirectoryKeyProvider::open_for_rotation(&key_directory, root_key).unwrap();
    let before_expiry = retire_capability_keys(&mut database, &provider, 99).unwrap();
    assert_eq!(before_expiry.retiring_generation_ids, vec![genesis]);
    assert!(key_directory.join("capability.key").is_file());
    let retired = retire_capability_keys(&mut database, &provider, 100).unwrap();
    assert!(retired.retiring_generation_ids.is_empty());
    assert!(!key_directory.join("capability.key").exists());
    assert_eq!(
        database
            .capability_key_generations()
            .unwrap()
            .into_iter()
            .find(|generation| generation.generation_id == genesis)
            .unwrap()
            .state,
        CapabilityKeyGenerationState::Revoked
    );
    validate_capability_key_generations(&database, &provider).unwrap();
    drop(provider);
    drop(database);

    let provider = DirectoryKeyProvider::open(&key_directory, root_key).unwrap();
    let mut database = Database::open(&database_path, Config::default()).unwrap();
    load_or_bootstrap(&mut database, &provider, &input(101)).unwrap();
    drop(database);
    drop(provider);

    let retired_backup = foks_server::backup_standalone_installation(
        &database_path,
        &key_directory,
        root_key,
        temporary.path().join("backup-retired"),
        Config::default(),
    )
    .unwrap();
    assert!(!retired_backup.key_directory.join("capability.key").exists());
    let restored_retired =
        restore_and_start(temporary.path(), "retired", &retired_backup, root_key);
    assert!(!restored_retired.join("capability.key").exists());
}

#[test]
fn startup_rejects_untracked_or_missing_capability_generations() {
    let temporary = tempfile::tempdir().unwrap();
    let database_path = temporary.path().join("server.sqlite");
    let key_directory = temporary.path().join("keys");
    let root_key = [0x61; 32];
    let provider = DirectoryKeyProvider::open(&key_directory, root_key).unwrap();
    let mut database = Database::open(&database_path, Config::default()).unwrap();
    load_or_bootstrap(&mut database, &provider, &input(1)).unwrap();
    drop(provider);

    let provider = DirectoryKeyProvider::open_for_rotation(&key_directory, root_key).unwrap();
    let orphan = provider.create_generation(KeyPurpose::Capability).unwrap();
    assert!(validate_capability_key_generations(&database, &provider).is_err());
    let rotated = rotate_capability_key(&mut database, &provider, 2).unwrap();
    assert!(provider
        .load_generation(KeyPurpose::Capability, orphan.generation())
        .is_err());
    let active = rotated.active_generation_id;
    provider
        .remove_generation(
            KeyPurpose::Capability,
            foks_server::keys::KeyGenerationId::from_bytes(active),
        )
        .unwrap();
    assert!(validate_capability_key_generations(&database, &provider).is_err());
    assert!(load_or_bootstrap(&mut database, &provider, &input(3)).is_err());
}

struct FailFirstRetirement {
    inner: DirectoryKeyProvider,
    fail: AtomicBool,
}

impl HostKeyProvider for FailFirstRetirement {
    fn load_or_create(
        &self,
        purpose: KeyPurpose,
    ) -> foks_server::Result<foks_server::keys::SecretKey> {
        self.inner.load_or_create(purpose)
    }

    fn load_existing(
        &self,
        purpose: KeyPurpose,
    ) -> foks_server::Result<foks_server::keys::SecretKey> {
        self.inner.load_existing(purpose)
    }

    fn create_generation(
        &self,
        purpose: KeyPurpose,
    ) -> foks_server::Result<foks_server::keys::SecretKey> {
        self.inner.create_generation(purpose)
    }

    fn load_generation(
        &self,
        purpose: KeyPurpose,
        generation: foks_server::keys::KeyGenerationId,
    ) -> foks_server::Result<foks_server::keys::SecretKey> {
        self.inner.load_generation(purpose, generation)
    }

    fn list_generations(
        &self,
        purpose: KeyPurpose,
    ) -> foks_server::Result<Vec<foks_server::keys::KeyGenerationId>> {
        self.inner.list_generations(purpose)
    }

    fn remove_generation(
        &self,
        purpose: KeyPurpose,
        generation: foks_server::keys::KeyGenerationId,
    ) -> foks_server::Result<()> {
        if self.fail.swap(false, Ordering::AcqRel) {
            return Err(foks_server::Error::Key(
                "injected post-commit capability retirement failure",
            ));
        }
        self.inner.remove_generation(purpose, generation)
    }
}

#[test]
fn retirement_retries_after_database_commit_before_key_erasure() {
    let temporary = tempfile::tempdir().unwrap();
    let database_path = temporary.path().join("server.sqlite");
    let key_directory = temporary.path().join("keys");
    let root_key = [0x71; 32];
    let provider = DirectoryKeyProvider::open(&key_directory, root_key).unwrap();
    let mut database = Database::open(&database_path, Config::default()).unwrap();
    load_or_bootstrap(&mut database, &provider, &input(1)).unwrap();
    let genesis = database.active_capability_key_generation().unwrap();
    drop(provider);

    let provider = DirectoryKeyProvider::open_for_rotation(&key_directory, root_key).unwrap();
    rotate_capability_key(&mut database, &provider, 2).unwrap();
    let provider = FailFirstRetirement {
        inner: provider,
        fail: AtomicBool::new(true),
    };
    let error = retire_capability_keys(&mut database, &provider, 2).unwrap_err();
    assert!(error.to_string().contains("post-commit"));
    assert!(key_directory.join("capability.key").is_file());
    assert_eq!(
        database
            .capability_key_generations()
            .unwrap()
            .into_iter()
            .find(|generation| generation.generation_id == genesis)
            .unwrap()
            .state,
        CapabilityKeyGenerationState::Revoked
    );
    drop(provider);
    drop(database);

    let provider = DirectoryKeyProvider::open(&key_directory, root_key).unwrap();
    let mut database = Database::open(&database_path, Config::default()).unwrap();
    load_or_bootstrap(&mut database, &provider, &input(3)).unwrap();
    drop(provider);
    let provider = DirectoryKeyProvider::open_for_rotation(&key_directory, root_key).unwrap();
    retire_capability_keys(&mut database, &provider, 3).unwrap();
    assert!(!key_directory.join("capability.key").exists());
}
