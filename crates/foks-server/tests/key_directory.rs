use std::sync::Arc;

use foks_server::keys::{
    read_root_key_file, DirectoryKeyProvider, HostKeyProvider, KeyGenerationManifest, KeyPurpose,
};

const ROOT_KEY: [u8; 32] = [0x42; 32];
const PURPOSES: [KeyPurpose; 5] = [
    KeyPurpose::Host,
    KeyPurpose::Metadata,
    KeyPurpose::Merkle,
    KeyPurpose::ClientCa,
    KeyPurpose::DelegatedTls,
];

#[test]
fn keys_are_encrypted_and_stable_across_reopen() {
    let temporary = tempfile::tempdir().unwrap();
    let provider = DirectoryKeyProvider::open(temporary.path().join("keys"), ROOT_KEY).unwrap();
    let first = PURPOSES.map(|purpose| {
        let key = provider.load_or_create(purpose).unwrap();
        (*key.expose(), key.generation())
    });

    for (purpose, (secret, _)) in PURPOSES.into_iter().zip(first) {
        let encoded = std::fs::read(
            temporary
                .path()
                .join(format!("keys/{}.key", label(purpose))),
        )
        .unwrap();
        assert!(!encoded.windows(secret.len()).any(|window| window == secret));
    }

    drop(provider);
    let reopened = DirectoryKeyProvider::open(temporary.path().join("keys"), ROOT_KEY).unwrap();
    let second = PURPOSES.map(|purpose| {
        let key = reopened.load_or_create(purpose).unwrap();
        (*key.expose(), key.generation())
    });
    assert_eq!(first, second);

    let manifest = KeyGenerationManifest::load_or_create(&reopened).unwrap();
    assert_eq!(
        KeyGenerationManifest::decode(&manifest.encode()).unwrap(),
        manifest
    );
}

#[test]
fn wrong_root_and_tampering_are_rejected() {
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("keys");
    let provider = DirectoryKeyProvider::open(&directory, ROOT_KEY).unwrap();
    provider.load_or_create(KeyPurpose::Host).unwrap();

    assert!(matches!(
        DirectoryKeyProvider::open(&directory, [0x24; 32]),
        Err(foks_server::Error::KeyCrypto)
    ));

    let path = directory.join("host.key");
    let mut encoded = std::fs::read(&path).unwrap();
    *encoded.last_mut().unwrap() ^= 1;
    std::fs::write(&path, encoded).unwrap();
    assert!(matches!(
        provider.load_or_create(KeyPurpose::Host),
        Err(foks_server::Error::KeyCrypto)
    ));
}

#[test]
fn operator_root_rotation_is_atomic_and_preserves_every_key_generation() {
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("keys");
    let provider = DirectoryKeyProvider::open(&directory, ROOT_KEY).unwrap();
    let before = foks_server::keys::MANIFEST_PURPOSES.map(|purpose| {
        let key = provider.load_or_create(purpose).unwrap();
        (*key.expose(), key.generation())
    });
    assert!(matches!(
        DirectoryKeyProvider::rotate_operator_root(&directory, ROOT_KEY, [0x83; 32]),
        Err(foks_server::Error::Config("key directory is in use"))
    ));
    drop(provider);

    let new_root = [0x83; 32];
    DirectoryKeyProvider::rotate_operator_root(&directory, ROOT_KEY, new_root).unwrap();
    assert!(matches!(
        DirectoryKeyProvider::open(&directory, ROOT_KEY),
        Err(foks_server::Error::KeyCrypto)
    ));
    let reopened = DirectoryKeyProvider::open(&directory, new_root).unwrap();
    let after = foks_server::keys::MANIFEST_PURPOSES.map(|purpose| {
        let key = reopened.load_existing(purpose).unwrap();
        (*key.expose(), key.generation())
    });
    assert_eq!(after, before);
    assert!(std::fs::read_dir(&directory).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .ends_with(".tmp")));
    assert!(DirectoryKeyProvider::rotate_operator_root(&directory, new_root, new_root).is_err());
}

#[test]
fn concurrent_creation_publishes_one_key() {
    let temporary = tempfile::tempdir().unwrap();
    let provider =
        Arc::new(DirectoryKeyProvider::open(temporary.path().join("keys"), ROOT_KEY).unwrap());
    let threads = (0..16)
        .map(|_| {
            let provider = Arc::clone(&provider);
            std::thread::spawn(move || {
                *provider
                    .load_or_create(KeyPurpose::Merkle)
                    .unwrap()
                    .expose()
            })
        })
        .collect::<Vec<_>>();
    let keys = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect::<Vec<_>>();
    assert!(keys.iter().all(|key| key == &keys[0]));
}

#[test]
fn secret_debug_output_is_redacted() {
    let provider = foks_server::keys::MemoryKeyProvider::default();
    let secret = provider.load_or_create(KeyPurpose::Host).unwrap();
    assert_eq!(format!("{secret:?}"), "SecretKey([REDACTED])");
}

#[cfg(unix)]
#[test]
fn owner_only_permissions_and_no_symlinks_are_enforced() {
    use std::os::unix::fs::{symlink, PermissionsExt as _};

    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("keys");
    let provider = DirectoryKeyProvider::open(&directory, ROOT_KEY).unwrap();
    provider.load_or_create(KeyPurpose::Host).unwrap();
    assert_eq!(
        std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let host = directory.join("host.key");
    assert_eq!(
        std::fs::metadata(&host).unwrap().permissions().mode() & 0o777,
        0o600
    );

    std::fs::set_permissions(&host, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(
        provider.load_or_create(KeyPurpose::Host),
        Err(foks_server::Error::Key(
            "key file is accessible by another user"
        ))
    ));

    let target = temporary.path().join("target.key");
    std::fs::write(&target, b"not a key").unwrap();
    symlink(&target, directory.join("metadata.key")).unwrap();
    assert!(matches!(
        provider.load_or_create(KeyPurpose::Metadata),
        Err(foks_server::Error::Key("key file is a symlink"))
    ));

    let target_directory = temporary.path().join("target-directory");
    std::fs::create_dir(&target_directory).unwrap();
    let linked_directory = temporary.path().join("linked-directory");
    symlink(&target_directory, &linked_directory).unwrap();
    assert!(matches!(
        DirectoryKeyProvider::open(linked_directory, ROOT_KEY),
        Err(foks_server::Error::Key("key directory is a symlink"))
    ));
}

#[cfg(unix)]
#[test]
fn operator_root_key_file_must_be_exact_private_and_not_a_symlink() {
    use std::os::unix::fs::{symlink, PermissionsExt as _};

    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("root.key");
    std::fs::write(&path, [0x91; 32]).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(&*read_root_key_file(&path).unwrap(), &[0x91; 32]);

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    assert!(read_root_key_file(&path).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&path, [0x91; 31]).unwrap();
    assert!(read_root_key_file(&path).is_err());

    let link = temporary.path().join("root-link.key");
    symlink(&path, &link).unwrap();
    assert!(read_root_key_file(link).is_err());
}

fn label(purpose: KeyPurpose) -> &'static str {
    match purpose {
        KeyPurpose::Host => "host",
        KeyPurpose::Metadata => "metadata",
        KeyPurpose::Merkle => "merkle",
        KeyPurpose::ClientCa => "client-ca",
        KeyPurpose::DelegatedTls => "delegated-tls",
        KeyPurpose::Recovery => "recovery",
        KeyPurpose::Capability => "capability",
    }
}
