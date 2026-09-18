//! Small encrypted record store for standalone FOKS application credentials.
//!
//! The record store accepts an externally managed 32-byte master key, typically
//! provided by a native platform credential service. [`create_master_key_file`]
//! is provided for tests, development environments, and headless deployments with
//! protected filesystems.

#![forbid(unsafe_code)]

pub mod state_archive;

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use chacha20poly1305::aead::{Aead as _, KeyInit as _, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use thiserror::Error;
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"FOKSKS\0\x01";
const NONCE_BYTES: usize = 24;
const TAG_BYTES: usize = 16;
const MAX_RECORD_BYTES: u64 = 16 * 1024 * 1024;
const MAX_KEY_BYTES: usize = 128;
const AAD_DOMAIN: &[u8] = b"foks-keystore-record-v1";

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid keystore record key")]
    InvalidKey,
    #[error("keystore record is missing")]
    Missing,
    #[error("keystore record exceeds the size limit")]
    TooLarge,
    #[error("keystore record has an invalid format")]
    InvalidFormat,
    #[error("keystore record authentication failed")]
    Authentication,
    #[error("keystore path is not a regular private path")]
    UnsafePath,
    #[error("keystore master key has an invalid length")]
    InvalidMasterKey,
    #[error("OS randomness is unavailable")]
    Randomness,
    #[error("native credentials require user interaction")]
    CredentialsRequired,
    #[error("native credential service failed: {0}")]
    Native(String),
    #[error("keystore I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Minimal interface shared by native and test credential stores.
pub trait SecretStore: Send {
    fn put(&mut self, key: &str, value: &[u8]) -> Result<()>;
    fn get(&mut self, key: &str) -> Result<Zeroizing<Vec<u8>>>;
    fn remove(&mut self, key: &str) -> Result<bool>;
    fn keys(&mut self) -> Result<Vec<String>>;
}

/// Small secrets kept outside the application state directory.
///
/// On macOS this uses generic-password records in the login Keychain. On
/// Linux it uses the freedesktop Secret Service default collection with an
/// encrypted D-Bus session. The namespace is an opaque, random state-root ID;
/// callers must not derive it from a username or filesystem path.
pub struct NativeCredentialStore {
    namespace: String,
}

thread_local! {
    static UNATTENDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn without_user_interaction<T>(operation: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            UNATTENDED.with(|state| state.set(self.0));
        }
    }
    let _restore = Restore(UNATTENDED.with(|state| state.replace(true)));
    operation()
}

#[cfg(any(target_os = "macos", test))]
fn serialized_native_call<T>(operation: impl FnOnce() -> T) -> T {
    static INTERACTION: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _lock = INTERACTION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    operation()
}

fn native_call<T>(operation: impl FnOnce() -> Result<T>) -> Result<T> {
    #[cfg(target_os = "macos")]
    {
        use security_framework::os::macos::keychain::SecKeychain;
        serialized_native_call(|| {
            let _disabled = if UNATTENDED.with(std::cell::Cell::get)
                && SecKeychain::user_interaction_allowed().map_err(native::map_error)?
            {
                Some(SecKeychain::disable_user_interaction().map_err(native::map_error)?)
            } else {
                None
            };
            operation()
        })
    }
    #[cfg(not(target_os = "macos"))]
    operation()
}

impl NativeCredentialStore {
    pub fn open(namespace: &str) -> Result<Self> {
        validate_key(namespace)?;
        Ok(Self {
            namespace: namespace.to_owned(),
        })
    }

    pub fn put(&mut self, key: &str, value: &[u8]) -> Result<()> {
        validate_key(key)?;
        validate_size(value.len())?;
        native_call(|| native::put(&self.namespace, key, value))
    }

    pub fn get(&mut self, key: &str) -> Result<Zeroizing<Vec<u8>>> {
        validate_key(key)?;
        native_call(|| native::get(&self.namespace, key)).map(Zeroizing::new)
    }

    pub fn remove(&mut self, key: &str) -> Result<bool> {
        validate_key(key)?;
        native_call(|| native::remove(&self.namespace, key))
    }
}

#[cfg(target_os = "macos")]
mod native {
    use super::{Error, Result};
    use security_framework::item::{update_item, ItemClass, ItemSearchOptions, ItemUpdateOptions};
    use security_framework::passwords::{
        delete_generic_password, get_generic_password, set_generic_password,
    };

    const ITEM_NOT_FOUND: i32 = -25_300;
    const INTERACTION_NOT_ALLOWED: i32 = -25_308;
    const INTERACTION_REQUIRED: i32 = -25_315;

    pub(super) fn map_error(error: security_framework::base::Error) -> Error {
        match error.code() {
            ITEM_NOT_FOUND => Error::Missing,
            INTERACTION_NOT_ALLOWED | INTERACTION_REQUIRED => Error::CredentialsRequired,
            _ => Error::Native(error.to_string()),
        }
    }

    fn service(namespace: &str) -> String {
        format!("org.foks.client.{namespace}")
    }

    pub(super) fn label(key: &str) -> String {
        if key == "native-state-v1" {
            return "FOKS protected client state".to_owned();
        }
        if key == "master-key-v1" {
            return "FOKS master key".to_owned();
        }
        if key == "state-root-v1" {
            return "FOKS state root binding".to_owned();
        }
        if let Some(profile) = key.strip_prefix("rollback.") {
            return format!("FOKS rollback checkpoint — {profile}");
        }
        if let Some(database) = key
            .strip_prefix("database.")
            .and_then(|key| key.strip_suffix(".profile-v1"))
        {
            return format!(
                "FOKS database ownership — {}",
                &database[..8.min(database.len())]
            );
        }
        if let Some(profile) = key.strip_prefix("profile-publication.") {
            return format!("FOKS profile publication — {profile}");
        }
        "FOKS protected record".to_owned()
    }

    fn update_label(service: &str, key: &str) -> security_framework::base::Result<()> {
        let mut search = ItemSearchOptions::new();
        search
            .class(ItemClass::generic_password())
            .service(service)
            .account(key);
        let mut update = ItemUpdateOptions::new();
        update.set_label(label(key));
        update_item(&search, &update)
    }

    pub(super) fn put(namespace: &str, key: &str, value: &[u8]) -> Result<()> {
        let service = service(namespace);
        set_generic_password(&service, key, value).map_err(map_error)?;
        let _ = update_label(&service, key);
        Ok(())
    }

    pub(super) fn get(namespace: &str, key: &str) -> Result<Vec<u8>> {
        let service = service(namespace);
        let value = get_generic_password(&service, key).map_err(map_error)?;
        Ok(value)
    }

    pub(super) fn remove(namespace: &str, key: &str) -> Result<bool> {
        match delete_generic_password(&service(namespace), key) {
            Ok(()) => Ok(true),
            Err(error) if error.code() == ITEM_NOT_FOUND => Ok(false),
            Err(error) => Err(map_error(error)),
        }
    }
}

#[cfg(target_os = "linux")]
mod native {
    use std::collections::HashMap;

    use secret_service::blocking::SecretService;
    use secret_service::EncryptionType;

    use super::{Error, Result};

    const APPLICATION: &str = "foks-rs";

    pub(super) fn map_error(error: secret_service::Error) -> Error {
        match error {
            secret_service::Error::Locked | secret_service::Error::Prompt => {
                Error::CredentialsRequired
            }
            _ => Error::Native(error.to_string()),
        }
    }

    fn attributes<'a>(namespace: &'a str, key: &'a str) -> HashMap<&'a str, &'a str> {
        HashMap::from([
            ("application", APPLICATION),
            ("foks-state", namespace),
            ("foks-key", key),
        ])
    }

    fn connect() -> Result<SecretService<'static>> {
        SecretService::connect(EncryptionType::Dh).map_err(map_error)
    }

    fn prepare_collection(collection: &secret_service::blocking::Collection<'_>) -> Result<()> {
        if super::UNATTENDED.with(std::cell::Cell::get) {
            if collection.is_locked().map_err(map_error)? {
                return Err(Error::CredentialsRequired);
            }
            Ok(())
        } else {
            collection.unlock().map_err(map_error)
        }
    }

    pub(super) fn put(namespace: &str, key: &str, value: &[u8]) -> Result<()> {
        let service = connect()?;
        let collection = service.get_default_collection().map_err(map_error)?;
        prepare_collection(&collection)?;
        if super::UNATTENDED.with(std::cell::Cell::get) {
            let mut items = collection
                .search_items(attributes(namespace, key))
                .map_err(map_error)?;
            let item = items.pop().ok_or(Error::CredentialsRequired)?;
            if !items.is_empty() {
                return Err(Error::Native(
                    "duplicate credential records match the requested key".into(),
                ));
            }
            if item.is_locked().map_err(map_error)? {
                return Err(Error::CredentialsRequired);
            }
            return item
                .set_secret(value, "application/octet-stream")
                .map_err(map_error);
        }
        collection
            .create_item(
                &format!("FOKS {key}"),
                attributes(namespace, key),
                value,
                true,
                "application/octet-stream",
            )
            .map_err(map_error)?;
        Ok(())
    }

    pub(super) fn get(namespace: &str, key: &str) -> Result<Vec<u8>> {
        let service = connect()?;
        let collection = service.get_default_collection().map_err(map_error)?;
        prepare_collection(&collection)?;
        let mut items = collection
            .search_items(attributes(namespace, key))
            .map_err(map_error)?;
        let Some(item) = items.pop() else {
            return Err(Error::Missing);
        };
        if !items.is_empty() {
            return Err(Error::Native(
                "duplicate credential records match the requested key".to_owned(),
            ));
        }
        if super::UNATTENDED.with(std::cell::Cell::get) && item.is_locked().map_err(map_error)? {
            return Err(Error::CredentialsRequired);
        }
        item.get_secret().map_err(map_error)
    }

    pub(super) fn remove(namespace: &str, key: &str) -> Result<bool> {
        if super::UNATTENDED.with(std::cell::Cell::get) {
            return Err(Error::CredentialsRequired);
        }
        let service = connect()?;
        let collection = service.get_default_collection().map_err(map_error)?;
        prepare_collection(&collection)?;
        let items = collection
            .search_items(attributes(namespace, key))
            .map_err(map_error)?;
        let removed = !items.is_empty();
        for item in items {
            item.delete().map_err(map_error)?;
        }
        Ok(removed)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod native {
    use super::{Error, Result};

    fn unsupported<T>() -> Result<T> {
        Err(Error::Native(
            "this build has no native credential backend".to_owned(),
        ))
    }

    pub(super) fn put(_namespace: &str, _key: &str, _value: &[u8]) -> Result<()> {
        unsupported()
    }
    pub(super) fn get(_namespace: &str, _key: &str) -> Result<Vec<u8>> {
        unsupported()
    }
    pub(super) fn remove(_namespace: &str, _key: &str) -> Result<bool> {
        unsupported()
    }
}

/// In-memory implementation for testing and transient storage.
#[derive(Default)]
pub struct MemorySecretStore {
    records: BTreeMap<String, Zeroizing<Vec<u8>>>,
}

impl SecretStore for MemorySecretStore {
    fn put(&mut self, key: &str, value: &[u8]) -> Result<()> {
        validate_key(key)?;
        validate_size(value.len())?;
        self.records
            .insert(key.to_owned(), Zeroizing::new(value.to_vec()));
        Ok(())
    }

    fn get(&mut self, key: &str) -> Result<Zeroizing<Vec<u8>>> {
        validate_key(key)?;
        self.records.get(key).cloned().ok_or(Error::Missing)
    }

    fn remove(&mut self, key: &str) -> Result<bool> {
        validate_key(key)?;
        Ok(self.records.remove(key).is_some())
    }

    fn keys(&mut self) -> Result<Vec<String>> {
        Ok(self.records.keys().cloned().collect())
    }
}

/// File-backed, independently authenticated encrypted records.
pub struct EncryptedFileSecretStore {
    directory: PathBuf,
    master_key: Zeroizing<[u8; 32]>,
}

impl EncryptedFileSecretStore {
    pub fn open(directory: impl AsRef<Path>, master_key: Zeroizing<[u8; 32]>) -> Result<Self> {
        let directory = prepare_private_directory(directory.as_ref())?;
        Ok(Self {
            directory,
            master_key,
        })
    }

    /// Opens a reserved existing record directory without creating or chmodding it.
    pub fn inspect_existing(
        directory: impl AsRef<Path>,
        master_key: Zeroizing<[u8; 32]>,
    ) -> Result<Self> {
        let path = directory.as_ref();
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(Error::UnsafePath);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(Error::UnsafePath);
            }
        }
        Ok(Self {
            directory: path.canonicalize()?,
            master_key,
        })
    }

    fn path(&self, key: &str) -> Result<PathBuf> {
        validate_key(key)?;
        Ok(self.directory.join(format!("{key}.fks")))
    }

    fn encode(&self, key: &str, value: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        validate_size(value.len())?;
        let mut nonce = [0u8; NONCE_BYTES];
        getrandom::fill(&mut nonce).map_err(|_| Error::Randomness)?;
        let cipher = XChaCha20Poly1305::new((&*self.master_key).into());
        let ciphertext = cipher
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: value,
                    aad: &record_aad(key),
                },
            )
            .map_err(|_| Error::Authentication)?;
        let mut encoded = Zeroizing::new(Vec::with_capacity(
            MAGIC.len() + NONCE_BYTES + ciphertext.len(),
        ));
        encoded.extend_from_slice(MAGIC);
        encoded.extend_from_slice(&nonce);
        encoded.extend_from_slice(&ciphertext);
        Ok(encoded)
    }

    fn decode(&self, key: &str, encoded: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        let maximum = MAGIC.len() as u64 + NONCE_BYTES as u64 + MAX_RECORD_BYTES + TAG_BYTES as u64;
        if encoded.len() as u64 > maximum {
            return Err(Error::TooLarge);
        }
        if encoded.len() < MAGIC.len() + NONCE_BYTES + TAG_BYTES || &encoded[..MAGIC.len()] != MAGIC
        {
            return Err(Error::InvalidFormat);
        }
        let nonce_start = MAGIC.len();
        let ciphertext_start = nonce_start + NONCE_BYTES;
        let nonce: [u8; NONCE_BYTES] = encoded[nonce_start..ciphertext_start]
            .try_into()
            .map_err(|_| Error::InvalidFormat)?;
        let cipher = XChaCha20Poly1305::new((&*self.master_key).into());
        cipher
            .decrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: &encoded[ciphertext_start..],
                    aad: &record_aad(key),
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| Error::Authentication)
    }
}

impl SecretStore for EncryptedFileSecretStore {
    fn put(&mut self, key: &str, value: &[u8]) -> Result<()> {
        let path = self.path(key)?;
        let encoded = self.encode(key, value)?;
        let (temporary, mut file) = create_temporary(&self.directory)?;
        let result = (|| {
            file.write_all(&encoded)?;
            file.sync_all()?;
            fs::rename(&temporary, &path)?;
            sync_directory(&self.directory)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    fn get(&mut self, key: &str) -> Result<Zeroizing<Vec<u8>>> {
        let path = self.path(key)?;
        let file = match open_private_read(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::Missing)
            }
            Err(error) => return Err(error.into()),
        };
        let metadata = file.metadata()?;
        let maximum = MAGIC.len() as u64 + NONCE_BYTES as u64 + MAX_RECORD_BYTES + TAG_BYTES as u64;
        if !metadata.is_file() || metadata.len() > maximum {
            return Err(Error::UnsafePath);
        }
        let mut encoded = Zeroizing::new(Vec::with_capacity(metadata.len() as usize));
        file.take(maximum + 1).read_to_end(&mut encoded)?;
        if encoded.len() as u64 > maximum {
            return Err(Error::TooLarge);
        }
        self.decode(key, &encoded)
    }

    fn remove(&mut self, key: &str) -> Result<bool> {
        let path = self.path(key)?;
        match fs::remove_file(path) {
            Ok(()) => {
                sync_directory(&self.directory)?;
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    fn keys(&mut self) -> Result<Vec<String>> {
        let mut keys = Vec::new();
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Some(key) = name.strip_suffix(".fks") else {
                continue;
            };
            if file_type.is_file() && validate_key(key).is_ok() {
                keys.push(key.to_owned());
            }
        }
        keys.sort();
        Ok(keys)
    }
}

/// Creates a new raw master-key file without replacing an existing path.
pub fn create_master_key_file(path: impl AsRef<Path>) -> Result<Zeroizing<[u8; 32]>> {
    let path = path.as_ref();
    let parent = path.parent().ok_or(Error::UnsafePath)?;
    prepare_private_directory(parent)?;
    let mut key = Zeroizing::new([0u8; 32]);
    getrandom::fill(&mut *key).map_err(|_| Error::Randomness)?;
    let mut file = create_private_new(path)?;
    file.write_all(&*key)?;
    file.sync_all()?;
    sync_directory(parent)?;
    Ok(key)
}

/// Loads a raw master-key file, rejecting symlinks and insecure Unix modes.
pub fn load_master_key_file(path: impl AsRef<Path>) -> Result<Zeroizing<[u8; 32]>> {
    let path = path.as_ref();
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(Error::UnsafePath);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(Error::UnsafePath);
        }
    }
    let file = open_private_read(path)?;
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(33).read_to_end(&mut bytes)?;
    if bytes.len() != 32 {
        return Err(Error::InvalidMasterKey);
    }
    let mut key = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(&bytes);
    Ok(key)
}

fn validate_key(key: &str) -> Result<()> {
    if key.is_empty()
        || key.len() > MAX_KEY_BYTES
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || key == "."
        || key == ".."
    {
        return Err(Error::InvalidKey);
    }
    Ok(())
}

fn validate_size(size: usize) -> Result<()> {
    if size as u64 > MAX_RECORD_BYTES {
        Err(Error::TooLarge)
    } else {
        Ok(())
    }
}

fn record_aad(key: &str) -> Vec<u8> {
    let mut aad = Vec::with_capacity(AAD_DOMAIN.len() + 8 + key.len());
    aad.extend_from_slice(AAD_DOMAIN);
    aad.extend_from_slice(&(key.len() as u64).to_be_bytes());
    aad.extend_from_slice(key.as_bytes());
    aad
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
            return Err(Error::UnsafePath);
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
            return Err(Error::UnsafePath);
        }
    }
    path.canonicalize().map_err(Error::from)
}

fn create_temporary(directory: &Path) -> Result<(PathBuf, File)> {
    for _ in 0..32 {
        let mut suffix = [0u8; 16];
        getrandom::fill(&mut suffix).map_err(|_| Error::Randomness)?;
        let name = suffix.iter().fold(String::from(".tmp-"), |mut name, byte| {
            use std::fmt::Write as _;
            write!(&mut name, "{byte:02x}").expect("writing to a String cannot fail");
            name
        });
        let path = directory.join(name);
        match create_private_new(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(Error::Randomness)
}

fn create_private_new(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path)
}

fn open_private_read(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path)
}

fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_call_serialization_covers_normal_calls_and_policy_restoration() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            mpsc, Arc,
        };
        use std::time::Duration;
        struct Suppression(Arc<AtomicBool>);
        impl Drop for Suppression {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let suppressed = Arc::new(AtomicBool::new(false));
        let (entered, observed) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let background_flag = suppressed.clone();
        let background = std::thread::spawn(move || {
            without_user_interaction(|| {
                serialized_native_call(|| {
                    assert!(UNATTENDED.with(std::cell::Cell::get));
                    background_flag.store(true, Ordering::SeqCst);
                    let _restore = Suppression(background_flag);
                    entered.send(()).unwrap();
                    released.recv_timeout(Duration::from_secs(5)).unwrap();
                });
            })
        });
        observed.recv_timeout(Duration::from_secs(5)).unwrap();
        let (attempting, attempted) = mpsc::channel();
        let (finished, completed) = mpsc::channel();
        let normal = std::thread::spawn(move || {
            assert!(!UNATTENDED.with(std::cell::Cell::get));
            attempting.send(()).unwrap();
            serialized_native_call(|| {
                assert!(!suppressed.load(Ordering::SeqCst));
                finished.send(()).unwrap();
            });
        });
        attempted.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            completed.recv_timeout(Duration::from_millis(25)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        release.send(()).unwrap();
        completed.recv_timeout(Duration::from_secs(5)).unwrap();
        background.join().unwrap();
        normal.join().unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_interaction_statuses_are_typed_without_keychain_access() {
        for code in [-25_308, -25_315] {
            assert!(matches!(
                native::map_error(security_framework::base::Error::from_code(code)),
                Error::CredentialsRequired
            ));
        }
        assert!(matches!(
            native::map_error(security_framework::base::Error::from_code(-25_300)),
            Error::Missing
        ));
        assert!(matches!(
            native::map_error(security_framework::base::Error::from_code(-50)),
            Error::Native(_)
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn secret_service_interaction_statuses_are_typed_without_service_access() {
        assert!(matches!(
            native::map_error(secret_service::Error::Locked),
            Error::CredentialsRequired
        ));
        assert!(matches!(
            native::map_error(secret_service::Error::Prompt),
            Error::CredentialsRequired
        ));
        assert!(matches!(
            native::map_error(secret_service::Error::Unavailable),
            Error::Native(_)
        ));
    }

    #[test]
    fn unattended_scope_restores_interaction_policy_after_unwind() {
        assert!(!UNATTENDED.with(std::cell::Cell::get));
        without_user_interaction(|| {
            assert!(UNATTENDED.with(std::cell::Cell::get));
            without_user_interaction(|| assert!(UNATTENDED.with(std::cell::Cell::get)));
            assert!(UNATTENDED.with(std::cell::Cell::get));
        });
        assert!(!UNATTENDED.with(std::cell::Cell::get));
        let _ = std::panic::catch_unwind(|| without_user_interaction(|| panic!("scope exit")));
        assert!(!UNATTENDED.with(std::cell::Cell::get));
    }

    fn key(byte: u8) -> Zeroizing<[u8; 32]> {
        Zeroizing::new([byte; 32])
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_record_labels_distinguish_keychain_purposes() {
        assert_eq!(
            native::label("native-state-v1"),
            "FOKS protected client state"
        );
        assert_eq!(native::label("master-key-v1"), "FOKS master key");
        assert_eq!(native::label("state-root-v1"), "FOKS state root binding");
        assert_eq!(
            native::label("rollback.work.example"),
            "FOKS rollback checkpoint — work.example"
        );
        assert_eq!(
            native::label("database.0123456789abcdef.profile-v1"),
            "FOKS database ownership — 01234567"
        );
        assert_eq!(
            native::label("profile-publication.work.example"),
            "FOKS profile publication — work.example"
        );
    }

    #[test]
    fn encrypted_records_round_trip_and_replace() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = EncryptedFileSecretStore::open(directory.path(), key(7)).unwrap();
        store.put("account.demo", b"first").unwrap();
        assert_eq!(&*store.get("account.demo").unwrap(), b"first");
        store.put("account.demo", b"second").unwrap();
        assert_eq!(&*store.get("account.demo").unwrap(), b"second");
        assert!(store.remove("account.demo").unwrap());
        assert!(!store.remove("account.demo").unwrap());
        assert!(matches!(store.get("account.demo"), Err(Error::Missing)));
    }

    #[test]
    fn wrong_key_and_tampering_fail_authentication() {
        let directory = tempfile::tempdir().unwrap();
        EncryptedFileSecretStore::open(directory.path(), key(1))
            .unwrap()
            .put("record", b"secret")
            .unwrap();
        assert!(matches!(
            EncryptedFileSecretStore::open(directory.path(), key(2))
                .unwrap()
                .get("record"),
            Err(Error::Authentication)
        ));
        let path = directory.path().join("record.fks");
        let mut bytes = fs::read(&path).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        fs::write(path, bytes).unwrap();
        assert!(matches!(
            EncryptedFileSecretStore::open(directory.path(), key(1))
                .unwrap()
                .get("record"),
            Err(Error::Authentication)
        ));
    }

    #[test]
    fn rejects_path_traversal_and_oversized_records() {
        let mut store = MemorySecretStore::default();
        assert!(matches!(
            store.put("../escape", b"x"),
            Err(Error::InvalidKey)
        ));
        assert!(matches!(
            store.put("large", &vec![0; MAX_RECORD_BYTES as usize + 1]),
            Err(Error::TooLarge)
        ));
    }

    #[test]
    fn master_key_file_is_create_once_and_exact_length() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("master.key");
        let created = create_master_key_file(&path).unwrap();
        let loaded = load_master_key_file(&path).unwrap();
        assert_eq!(&*created, &*loaded);
        assert!(matches!(
            create_master_key_file(&path),
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists
        ));
    }
}
