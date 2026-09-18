//! Durable encrypted storage for crash-recovery mutation material.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use foks_crypto::prefixed_hash;
use zeroize::Zeroizing;

use crate::{ProtectedMutationStore, ProtectedStoreError};

const FILE_MAGIC: &[u8; 8] = b"FOKSPMS\x01";
const NONCE_BYTES: usize = 24;
const TAG_BYTES: usize = 16;
const KEY_NAME_TYPE_ID: u64 = 0xc309_46fb_ea2f_762b;
const AAD_DOMAIN: &[u8] = b"foks-protected-mutation-store-v1";
const MAX_PROTECTED_MATERIAL_BYTES: u64 = 128 * 1024 * 1024;

/// Durable encrypted implementation of [`ProtectedMutationStore`].
///
/// The embedding application should load `master_key` from its platform
/// keychain or encrypted vault. The key is never persisted here. Each record
/// is independently authenticated with XChaCha20-Poly1305 and installed with
/// an atomic no-replace operation after its contents have been synced.
pub struct EncryptedFileMutationStore {
    directory: PathBuf,
    master_key: Zeroizing<[u8; 32]>,
}

/// Private maintenance cursor. Each pass must hold the profile operation lock;
/// callers must discard it when relocating or replacing the protected directory.
pub struct ProtectedTemporaryScan {
    directory: PathBuf,
    entries: fs::ReadDir,
    identity: fs::Metadata,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ProtectedTemporaryReport {
    pub examined: u64,
    pub removed: u64,
    pub final_removed: u64,
    pub complete: bool,
}

impl EncryptedFileMutationStore {
    pub fn open(
        directory: impl AsRef<Path>,
        master_key: Zeroizing<[u8; 32]>,
    ) -> Result<Self, ProtectedStoreError> {
        let directory = prepare_directory(directory.as_ref())?;
        Ok(Self {
            directory,
            master_key,
        })
    }

    /// Opens an existing exclusively reserved directory without creation or permission repair.
    pub fn inspect_existing(
        directory: impl AsRef<Path>,
        master_key: Zeroizing<[u8; 32]>,
    ) -> Result<Self, ProtectedStoreError> {
        let path = directory.as_ref();
        let metadata = fs::symlink_metadata(path).map_err(io_backend)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(backend("protected directory is not real"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(backend("protected directory is not private"));
            }
        }
        Ok(Self {
            directory: path.canonicalize().map_err(io_backend)?,
            master_key,
        })
    }

    /// Enumerates only while the embedding application holds writer exclusion.
    pub fn temporary_scan(&self) -> Result<ProtectedTemporaryScan, ProtectedStoreError> {
        Ok(ProtectedTemporaryScan {
            directory: self.directory.clone(),
            entries: fs::read_dir(&self.directory).map_err(io_backend)?,
            identity: fs::symlink_metadata(&self.directory).map_err(io_backend)?,
        })
    }

    pub fn cleanup_temporary_batch(
        &self,
        scan: &mut ProtectedTemporaryScan,
    ) -> Result<ProtectedTemporaryReport, ProtectedStoreError> {
        self.cleanup_temporary_until(
            scan,
            std::time::Instant::now() + std::time::Duration::from_millis(25),
        )
    }

    pub fn cleanup_temporary_until(
        &self,
        scan: &mut ProtectedTemporaryScan,
        deadline: std::time::Instant,
    ) -> Result<ProtectedTemporaryReport, ProtectedStoreError> {
        self.cleanup_batch(scan, deadline, None)
    }

    /// A complete, current inventory is the only authority for final-file absence.
    /// Caller holds checked-profile exclusion across revision lookup and deletion.
    pub fn reconcile_unowned_until(
        &self,
        scan: &mut ProtectedTemporaryScan,
        inventory: &crate::ProtectedRecordInventory,
        revision: foks_client_db::HardStateMetadata,
        deadline: std::time::Instant,
    ) -> Result<ProtectedTemporaryReport, ProtectedStoreError> {
        self.cleanup_batch(scan, deadline, Some(inventory.records(revision)?))
    }

    fn cleanup_batch(
        &self,
        scan: &mut ProtectedTemporaryScan,
        deadline: std::time::Instant,
        owners: Option<&std::collections::BTreeMap<String, crate::ProtectedRecordDescriptor>>,
    ) -> Result<ProtectedTemporaryReport, ProtectedStoreError> {
        if scan.directory != self.directory {
            return Err(backend("protected scan directory changed"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let current = fs::symlink_metadata(&self.directory).map_err(io_backend)?;
            if !current.is_dir()
                || current.dev() != scan.identity.dev()
                || current.ino() != scan.identity.ino()
            {
                return Err(backend("protected scan directory identity changed"));
            }
        }
        let mut report = ProtectedTemporaryReport::default();
        let result = (|| {
            while report.examined < 256 && std::time::Instant::now() < deadline {
                let Some(entry) = scan.entries.next() else {
                    report.complete = true;
                    break;
                };
                let entry = entry.map_err(io_backend)?;
                report.examined += 1;
                let name = entry.file_name();
                let name = name
                    .to_str()
                    .ok_or_else(|| backend("invalid protected filename"))?;
                let lower_hex =
                    |value: &str| value.bytes().all(|b| matches!(b,b'0'..=b'9'|b'a'..=b'f'));
                let temporary = name
                    .strip_prefix(".tmp-")
                    .is_some_and(|suffix| suffix.len() == 32 && lower_hex(suffix));
                let final_record = name
                    .strip_suffix(".pms")
                    .is_some_and(|prefix| prefix.len() == 64 && lower_hex(prefix));
                if !temporary && !final_record {
                    return Err(backend("invalid protected filename"));
                }
                if !entry.file_type().map_err(io_backend)?.is_file() {
                    return Err(backend("protected entry is not a regular file"));
                }
                if temporary || owners.is_some_and(|owners| !owners.contains_key(name)) {
                    // Revalidate the candidate itself immediately before unlinking;
                    // do not trust a cached directory entry across lock reacquisition.
                    let identity = fs::symlink_metadata(entry.path()).map_err(io_backend)?;
                    if !identity.is_file() {
                        return Err(backend("protected candidate identity changed"));
                    }
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::{DirEntryExt, MetadataExt};
                        if identity.ino() != entry.ino() || identity.dev() != scan.identity.dev() {
                            return Err(backend("protected candidate identity changed"));
                        }
                    }
                    match fs::remove_file(entry.path()) {
                        Ok(()) => {
                            report.removed += 1;
                            if final_record {
                                report.final_removed += 1;
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                        Err(error) => return Err(io_backend(error)),
                    }
                }
            }
            Ok(report)
        })();
        // Persist partial progress even if a later malformed entry blocked the pass.
        self.sync()?;
        result
    }

    /// Completes durable erasure even when an idempotent removal found no file.
    pub fn sync(&self) -> Result<(), ProtectedStoreError> {
        sync_directory(&self.directory)
    }

    pub fn record_filename(key: &[u8]) -> Result<String, ProtectedStoreError> {
        if key.is_empty() {
            return Err(backend("protected material key is empty"));
        }
        let digest = prefixed_hash(KEY_NAME_TYPE_ID, key);
        use std::fmt::Write as _;
        let mut name = String::with_capacity(68);
        for byte in digest {
            write!(&mut name, "{byte:02x}").expect("String writing cannot fail");
        }
        name.push_str(".pms");
        Ok(name)
    }

    fn record_path(&self, key: &[u8]) -> Result<PathBuf, ProtectedStoreError> {
        Ok(self.directory.join(Self::record_filename(key)?))
    }

    fn encrypt(&self, key: &[u8], material: &[u8]) -> Result<Vec<u8>, ProtectedStoreError> {
        let material_len = u64::try_from(material.len())
            .map_err(|_| backend("protected material length overflow"))?;
        if material_len > MAX_PROTECTED_MATERIAL_BYTES {
            return Err(backend("protected material exceeds the store limit"));
        }

        let mut nonce = [0u8; NONCE_BYTES];
        getrandom::fill(&mut nonce).map_err(|_| backend("OS randomness unavailable"))?;
        let aad = record_aad(key)?;
        let cipher = XChaCha20Poly1305::new((&*self.master_key).into());
        let nonce = XNonce::try_from(nonce.as_slice())
            .map_err(|_| backend("protected material nonce length is invalid"))?;
        let ciphertext = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: material,
                    aad: &aad,
                },
            )
            .map_err(|_| backend("protected material encryption failed"))?;
        let mut encoded = Vec::with_capacity(FILE_MAGIC.len() + NONCE_BYTES + ciphertext.len());
        encoded.extend_from_slice(FILE_MAGIC);
        encoded.extend_from_slice(&nonce);
        encoded.extend_from_slice(&ciphertext);
        Ok(encoded)
    }

    fn read_record(
        &self,
        path: &Path,
        key: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, ProtectedStoreError> {
        let file = open_record_for_read(path)?;
        let metadata = file.metadata().map_err(io_backend)?;
        let maximum_file_len = FILE_MAGIC.len() as u64
            + NONCE_BYTES as u64
            + MAX_PROTECTED_MATERIAL_BYTES
            + TAG_BYTES as u64;
        if metadata.len() > maximum_file_len {
            return Err(backend("protected material record exceeds the store limit"));
        }

        let mut encoded = Zeroizing::new(Vec::new());
        file.take(maximum_file_len + 1)
            .read_to_end(&mut encoded)
            .map_err(io_backend)?;
        if encoded.len() as u64 > maximum_file_len {
            return Err(backend("protected record grew beyond limit"));
        }
        if encoded.len() < FILE_MAGIC.len() + NONCE_BYTES + TAG_BYTES
            || &encoded[..FILE_MAGIC.len()] != FILE_MAGIC
        {
            return Err(backend("protected material record has an invalid format"));
        }
        let nonce_start = FILE_MAGIC.len();
        let ciphertext_start = nonce_start + NONCE_BYTES;
        let aad = record_aad(key)?;
        let cipher = XChaCha20Poly1305::new((&*self.master_key).into());
        let nonce = XNonce::try_from(&encoded[nonce_start..ciphertext_start])
            .map_err(|_| backend("protected material nonce length is invalid"))?;
        let plaintext = cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: &encoded[ciphertext_start..],
                    aad: &aad,
                },
            )
            .map_err(|_| backend("protected material authentication failed"))?;
        Ok(Zeroizing::new(plaintext))
    }

    fn create_temporary(&self) -> Result<(PathBuf, File), ProtectedStoreError> {
        for _ in 0..32 {
            let mut suffix = [0u8; 16];
            getrandom::fill(&mut suffix).map_err(|_| backend("OS randomness unavailable"))?;
            let mut name = String::from(".tmp-");
            for byte in suffix {
                use std::fmt::Write as _;
                write!(&mut name, "{byte:02x}").expect("writing to String cannot fail");
            }
            let path = self.directory.join(name);
            match create_private_file(&path) {
                Ok(file) => return Ok((path, file)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(io_backend(error)),
            }
        }
        Err(backend(
            "unable to allocate a protected store temporary file",
        ))
    }
}

impl ProtectedMutationStore for EncryptedFileMutationStore {
    fn put_if_absent(&mut self, key: &[u8], material: &[u8]) -> Result<(), ProtectedStoreError> {
        let path = self.record_path(key)?;
        match self.read_record(&path, key) {
            Ok(existing) if existing.as_slice() == material => return self.sync(),
            Ok(_) => return Err(ProtectedStoreError::Conflict),
            Err(ProtectedStoreError::Missing) => {}
            Err(error) => return Err(error),
        }

        let encoded = Zeroizing::new(self.encrypt(key, material)?);
        let (temporary, mut file) = self.create_temporary()?;
        let install_result = (|| {
            file.write_all(&encoded).map_err(io_backend)?;
            file.sync_all().map_err(io_backend)?;
            fs::hard_link(&temporary, &path).map_err(io_backend)?;
            sync_directory(&self.directory)?;
            Ok(())
        })();
        if fs::remove_file(&temporary).is_ok() {
            sync_directory(&self.directory)?;
        }

        match install_result {
            Ok(()) => Ok(()),
            Err(ProtectedStoreError::Backend(_)) if path.exists() => {
                let existing = self.read_record(&path, key)?;
                if existing.as_slice() == material {
                    self.sync()
                } else {
                    Err(ProtectedStoreError::Conflict)
                }
            }
            Err(error) => Err(error),
        }
    }

    fn get(&mut self, key: &[u8]) -> Result<Zeroizing<Vec<u8>>, ProtectedStoreError> {
        self.read_record(&self.record_path(key)?, key)
    }

    fn remove(&mut self, key: &[u8]) -> Result<(), ProtectedStoreError> {
        let path = self.record_path(key)?;
        match fs::remove_file(path) {
            Ok(()) => sync_directory(&self.directory),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.sync()?;
                Err(ProtectedStoreError::Missing)
            }
            Err(error) => Err(io_backend(error)),
        }
    }
}

fn record_aad(key: &[u8]) -> Result<Vec<u8>, ProtectedStoreError> {
    let key_len = u64::try_from(key.len()).map_err(|_| backend("protected key length overflow"))?;
    let mut aad = Vec::with_capacity(AAD_DOMAIN.len() + 8 + key.len());
    aad.extend_from_slice(AAD_DOMAIN);
    aad.extend_from_slice(&key_len.to_be_bytes());
    aad.extend_from_slice(key);
    Ok(aad)
}

fn prepare_directory(path: &Path) -> Result<PathBuf, ProtectedStoreError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};

        if !path.exists() {
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true).mode(0o700);
            builder.create(path).map_err(io_backend)?;
        }
        let metadata = fs::symlink_metadata(path).map_err(io_backend)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(backend("protected store path is not a real directory"));
        }
        if metadata.mode() & 0o077 != 0 {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(io_backend)?;
        }
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(path).map_err(io_backend)?;
        let metadata = fs::symlink_metadata(path).map_err(io_backend)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(backend("protected store path is not a real directory"));
        }
    }
    path.canonicalize().map_err(io_backend)
}

fn create_private_file(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path)
}

fn open_record_for_read(path: &Path) -> Result<File, ProtectedStoreError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    match options.open(path) {
        Ok(file) => Ok(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Err(ProtectedStoreError::Missing)
        }
        Err(error) => Err(io_backend(error)),
    }
}

#[cfg(test)]
thread_local! {static FAIL_NEXT_DIRECTORY_SYNC: std::cell::Cell<bool> = const {std::cell::Cell::new(false)};}

fn sync_directory(path: &Path) -> Result<(), ProtectedStoreError> {
    #[cfg(test)]
    if FAIL_NEXT_DIRECTORY_SYNC.replace(false) {
        return Err(backend("injected directory sync failure"));
    }
    File::open(path)
        .map_err(io_backend)?
        .sync_all()
        .map_err(io_backend)
}

fn io_backend(error: std::io::Error) -> ProtectedStoreError {
    backend(format!("I/O error: {error}"))
}

fn backend(message: impl Into<String>) -> ProtectedStoreError {
    ProtectedStoreError::Backend(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(path: &Path, byte: u8) -> EncryptedFileMutationStore {
        EncryptedFileMutationStore::open(path, Zeroizing::new([byte; 32])).unwrap()
    }

    #[test]
    fn temporary_cleanup_is_bounded_preserves_final_records_and_resumes() {
        let directory = tempfile::tempdir().unwrap();
        let mut protected = store(directory.path(), 7);
        protected
            .put_if_absent(b"live", b"retained evidence")
            .unwrap();
        for n in 0..600 {
            fs::write(
                directory.path().join(format!(".tmp-{n:032x}")),
                b"interrupted",
            )
            .unwrap();
        }
        let mut scan = protected.temporary_scan().unwrap();
        let mut removed = 0;
        loop {
            let report = protected.cleanup_temporary_batch(&mut scan).unwrap();
            assert!(report.examined <= 256);
            assert!(report.removed <= 256);
            removed += report.removed;
            if report.complete {
                break;
            }
        }
        assert_eq!(removed, 600);
        assert_eq!(
            protected.get(b"live").unwrap().as_slice(),
            b"retained evidence"
        );
        let mut scan = protected.temporary_scan().unwrap();
        assert_eq!(
            protected
                .cleanup_temporary_batch(&mut scan)
                .unwrap()
                .removed,
            0
        );
    }

    #[cfg(unix)]
    #[test]
    fn temporary_cleanup_refuses_symlinks_and_malformed_names() {
        let directory = tempfile::tempdir().unwrap();
        let protected = store(directory.path(), 7);
        let outside = tempfile::NamedTempFile::new().unwrap();
        let name = directory.path().join(format!(".tmp-{:032x}", 1));
        std::os::unix::fs::symlink(outside.path(), &name).unwrap();
        assert!(protected
            .cleanup_temporary_batch(&mut protected.temporary_scan().unwrap())
            .is_err());
        assert!(outside.path().exists());
        fs::remove_file(name).unwrap();
        fs::write(directory.path().join(".tmp-bad"), b"bad").unwrap();
        assert!(protected
            .cleanup_temporary_batch(&mut protected.temporary_scan().unwrap())
            .is_err());
    }

    #[test]
    fn idempotent_install_reestablishes_directory_durability() {
        let directory = tempfile::tempdir().unwrap();
        let mut protected = store(directory.path(), 7);
        protected
            .put_if_absent(b"operation", b"exact bytes")
            .unwrap();
        FAIL_NEXT_DIRECTORY_SYNC.set(true);
        assert!(protected
            .put_if_absent(b"operation", b"exact bytes")
            .is_err());
        protected
            .put_if_absent(b"operation", b"exact bytes")
            .unwrap();
        assert_eq!(&*protected.get(b"operation").unwrap(), b"exact bytes");
    }

    #[test]
    fn interrupted_directory_sync_is_retryable_without_forgetting_ownership() {
        let directory = tempfile::tempdir().unwrap();
        let mut protected = store(directory.path(), 7);
        protected.put_if_absent(b"live", b"retained").unwrap();
        let temporary = directory.path().join(format!(".tmp-{:032x}", 1));
        fs::write(&temporary, b"abandoned").unwrap();
        FAIL_NEXT_DIRECTORY_SYNC.set(true);
        assert!(protected
            .cleanup_temporary_batch(&mut protected.temporary_scan().unwrap())
            .is_err());
        // The unlink may have happened, but no successful durable-cleanup report
        // was returned. A retry syncs even when the file is already absent.
        let mut scan = protected.temporary_scan().unwrap();
        let retry = protected.cleanup_temporary_batch(&mut scan).unwrap();
        assert!(retry.complete);
        assert!(!temporary.exists());
        assert_eq!(&*protected.get(b"live").unwrap(), b"retained");
    }

    #[test]
    fn records_survive_reopen_and_are_not_plaintext() {
        let directory = tempfile::tempdir().unwrap();
        let key = b"operation";
        let secret = b"recognizable mutation material";
        store(directory.path(), 7)
            .put_if_absent(key, secret)
            .unwrap();

        let mut reopened = store(directory.path(), 7);
        assert_eq!(reopened.get(key).unwrap().as_slice(), secret);
        let record = fs::read(reopened.record_path(key).unwrap()).unwrap();
        assert!(!record.windows(secret.len()).any(|window| window == secret));
    }

    #[test]
    fn put_if_absent_is_idempotent_but_never_replaces() {
        let directory = tempfile::tempdir().unwrap();
        let mut protected = store(directory.path(), 8);
        protected.put_if_absent(b"key", b"first").unwrap();
        protected.put_if_absent(b"key", b"first").unwrap();
        assert!(matches!(
            protected.put_if_absent(b"key", b"second"),
            Err(ProtectedStoreError::Conflict)
        ));
        assert_eq!(protected.get(b"key").unwrap().as_slice(), b"first");
    }

    #[test]
    fn wrong_key_and_tampering_fail_authentication() {
        let directory = tempfile::tempdir().unwrap();
        let mut protected = store(directory.path(), 9);
        protected.put_if_absent(b"key", b"secret").unwrap();
        assert!(matches!(
            store(directory.path(), 10).get(b"key"),
            Err(ProtectedStoreError::Backend(_))
        ));

        let path = protected.record_path(b"key").unwrap();
        let mut bytes = fs::read(&path).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        fs::write(path, bytes).unwrap();
        assert!(matches!(
            protected.get(b"key"),
            Err(ProtectedStoreError::Backend(_))
        ));
    }

    #[test]
    fn missing_removal_retries_directory_sync_before_reporting_completion() {
        let directory = tempfile::tempdir().unwrap();
        let mut protected = store(directory.path(), 11);
        protected.put_if_absent(b"key", b"secret").unwrap();
        FAIL_NEXT_DIRECTORY_SYNC.set(true);
        assert!(matches!(
            protected.remove(b"key"),
            Err(ProtectedStoreError::Backend(_))
        ));
        assert!(matches!(
            protected.get(b"key"),
            Err(ProtectedStoreError::Missing)
        ));
        FAIL_NEXT_DIRECTORY_SYNC.set(true);
        assert!(matches!(
            protected.remove(b"key"),
            Err(ProtectedStoreError::Backend(_))
        ));
        drop(protected);
        let mut protected = store(directory.path(), 11);
        assert!(matches!(
            protected.remove(b"key"),
            Err(ProtectedStoreError::Missing)
        ));
    }

    #[test]
    fn remove_is_durable_and_missing_is_explicit() {
        let directory = tempfile::tempdir().unwrap();
        let mut protected = store(directory.path(), 11);
        protected.put_if_absent(b"key", b"secret").unwrap();
        protected.remove(b"key").unwrap();
        assert!(matches!(
            protected.get(b"key"),
            Err(ProtectedStoreError::Missing)
        ));
        assert!(matches!(
            protected.remove(b"key"),
            Err(ProtectedStoreError::Missing)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn record_symlinks_are_not_followed() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let mut protected = store(directory.path(), 12);
        let target = directory.path().join("outside");
        fs::write(&target, b"outside").unwrap();
        symlink(&target, protected.record_path(b"key").unwrap()).unwrap();
        assert!(matches!(
            protected.get(b"key"),
            Err(ProtectedStoreError::Backend(_))
        ));
    }
}
