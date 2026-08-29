use std::fs::{File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use chacha20poly1305::aead::{Aead as _, KeyInit as _, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

use super::{HostKeyProvider, KeyGenerationId, KeyPurpose, SecretKey};
use crate::{Error, Result};

const MAGIC: &[u8; 8] = b"FOKSK01\0";
const WRAPPING_MAGIC: &[u8; 8] = b"FOKSW01\0";
pub(crate) const WRAPPING_KEY_FILE: &str = "key-encryption.key";
const ROTATION_LOCK_FILE: &str = ".key-encryption.lock";
const GENERATION_BYTES: usize = 16;
const NONCE_BYTES: usize = 24;

pub struct DirectoryKeyProvider {
    directory: PathBuf,
    wrapping_key: Zeroizing<[u8; 32]>,
    exclusive: bool,
    _process_lock: File,
}

impl DirectoryKeyProvider {
    pub fn open(directory: impl AsRef<Path>, root_key: [u8; 32]) -> Result<Self> {
        Self::open_with_lock(directory, root_key, false)
    }

    /// Opens the key directory exclusively for an offline public-key
    /// rotation. This fails while a server or backup process has it open.
    pub fn open_for_rotation(directory: impl AsRef<Path>, root_key: [u8; 32]) -> Result<Self> {
        Self::open_with_lock(directory, root_key, true)
    }

    fn open_with_lock(
        directory: impl AsRef<Path>,
        root_key: [u8; 32],
        exclusive: bool,
    ) -> Result<Self> {
        let root_key = Zeroizing::new(root_key);
        let directory = directory.as_ref().to_path_buf();
        std::fs::create_dir_all(&directory)?;
        set_directory_permissions(&directory)?;
        if std::fs::symlink_metadata(&directory)?
            .file_type()
            .is_symlink()
        {
            return Err(Error::Key("key directory is a symlink"));
        }
        let process_lock = open_rotation_lock(&directory)?;
        if exclusive {
            process_lock
                .try_lock()
                .map_err(|_| Error::Config("key directory is in use"))?;
        } else {
            process_lock
                .try_lock_shared()
                .map_err(|_| Error::Config("key directory rotation is active"))?;
        }
        let wrapping_key = load_or_create_wrapping_key(&directory, &root_key)?;
        if exclusive {
            cleanup_private_temporaries(&directory)?;
        }
        Ok(Self {
            directory,
            wrapping_key,
            exclusive,
            _process_lock: process_lock,
        })
    }

    /// Atomically rewraps the installation key-encryption key. Purpose keys
    /// and their generation IDs are unchanged, so the database and clients do
    /// not observe operator-root rotation.
    pub fn rotate_operator_root(
        directory: impl AsRef<Path>,
        old_root_key: [u8; 32],
        new_root_key: [u8; 32],
    ) -> Result<()> {
        let old_root_key = Zeroizing::new(old_root_key);
        let new_root_key = Zeroizing::new(new_root_key);
        if *old_root_key == *new_root_key {
            return Err(Error::Key("new operator root key is unchanged"));
        }
        let directory = directory.as_ref();
        let metadata = std::fs::symlink_metadata(directory)?;
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            return Err(Error::Key("key directory is not a regular directory"));
        }
        let process_lock = open_rotation_lock(directory)?;
        process_lock
            .try_lock()
            .map_err(|_| Error::Config("key directory is in use"))?;
        let path = directory.join(WRAPPING_KEY_FILE);
        let wrapping_key = load_wrapping_key(&path, &old_root_key)?;
        let encoded = encode_wrapping_key(&wrapping_key, &new_root_key)?;
        let temporary = temporary_path(directory, "operator-root")?;
        let result = (|| {
            write_new_secret(&temporary, &encoded)?;
            let verified = load_wrapping_key(&temporary, &new_root_key)?;
            if *verified != *wrapping_key {
                return Err(Error::Key("operator root rotation verification"));
            }
            std::fs::rename(&temporary, &path)?;
            sync_directory(directory)?;
            Ok(())
        })();
        let _ = std::fs::remove_file(temporary);
        result
    }

    pub fn load_existing(&self, purpose: KeyPurpose) -> Result<SecretKey> {
        self.load(&self.path(purpose), purpose)
    }

    fn path(&self, purpose: KeyPurpose) -> PathBuf {
        self.directory.join(format!("{}.key", purpose.label()))
    }

    fn generation_path(&self, purpose: KeyPurpose, generation: KeyGenerationId) -> PathBuf {
        let generation = generation
            .as_bytes()
            .into_iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        self.directory
            .join(format!("{}.{generation}.key", purpose.label()))
    }

    fn load(&self, path: &Path, purpose: KeyPurpose) -> Result<SecretKey> {
        if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
            return Err(Error::Key("key file is a symlink"));
        }
        let mut file = secure_open_read(path)?;
        validate_file_permissions(&file)?;
        let maximum = MAGIC.len() + GENERATION_BYTES + NONCE_BYTES + 32 + 16;
        let length =
            usize::try_from(file.metadata()?.len()).map_err(|_| Error::Key("key file size"))?;
        if length != maximum {
            return Err(Error::Key("invalid key file length"));
        }
        let mut encoded = Zeroizing::new(Vec::with_capacity(length));
        file.read_to_end(&mut encoded)?;
        if &encoded[..MAGIC.len()] != MAGIC {
            return Err(Error::Key("invalid key file magic"));
        }
        let generation_start = MAGIC.len();
        let nonce_start = generation_start + GENERATION_BYTES;
        let ciphertext_start = nonce_start + NONCE_BYTES;
        let generation: [u8; GENERATION_BYTES] = encoded[generation_start..nonce_start]
            .try_into()
            .map_err(|_| Error::Key("invalid key generation"))?;
        let aad = associated_data(purpose, &generation);
        let cipher = XChaCha20Poly1305::new((&*self.wrapping_key).into());
        let plaintext = Zeroizing::new(
            cipher
                .decrypt(
                    <&XNonce>::try_from(&encoded[nonce_start..ciphertext_start])
                        .map_err(|_| Error::Key("invalid key nonce"))?,
                    Payload {
                        msg: &encoded[ciphertext_start..],
                        aad: &aad,
                    },
                )
                .map_err(|_| Error::KeyCrypto)?,
        );
        let key: [u8; 32] = plaintext
            .as_slice()
            .try_into()
            .map_err(|_| Error::Key("invalid key plaintext"))?;
        Ok(SecretKey::new(key, generation))
    }

    fn create(&self, path: &Path, purpose: KeyPurpose) -> Result<SecretKey> {
        let mut generation = [0; GENERATION_BYTES];
        getrandom::fill(&mut generation).map_err(|_| Error::Key("operating-system entropy"))?;
        match self.create_with_generation(path, purpose, KeyGenerationId::from_bytes(generation)) {
            Ok(key) => Ok(key),
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                self.load(path, purpose)
            }
            Err(error) => Err(error),
        }
    }

    fn create_with_generation(
        &self,
        path: &Path,
        purpose: KeyPurpose,
        generation: KeyGenerationId,
    ) -> Result<SecretKey> {
        let mut key = Zeroizing::new([0; 32]);
        getrandom::fill(&mut *key).map_err(|_| Error::Key("operating-system entropy"))?;
        let mut nonce = [0; NONCE_BYTES];
        getrandom::fill(&mut nonce).map_err(|_| Error::Key("operating-system entropy"))?;
        let generation = generation.as_bytes();
        let aad = associated_data(purpose, &generation);
        let cipher = XChaCha20Poly1305::new((&*self.wrapping_key).into());
        let ciphertext = cipher
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: &*key,
                    aad: &aad,
                },
            )
            .map_err(|_| Error::KeyCrypto)?;
        let temporary = temporary_path(&self.directory, purpose.label())?;
        let result = (|| {
            let mut file = secure_create(&temporary)?;
            file.write_all(MAGIC)?;
            file.write_all(&generation)?;
            file.write_all(&nonce)?;
            file.write_all(&ciphertext)?;
            file.sync_all()?;
            match std::fs::hard_link(&temporary, path) {
                Ok(()) => {
                    sync_directory(&self.directory)?;
                    Ok(SecretKey::new(*key, generation))
                }
                Err(error) => Err(error.into()),
            }
        })();
        let _ = std::fs::remove_file(temporary);
        result
    }
}

fn load_or_create_wrapping_key(
    directory: &Path,
    root_key: &[u8; 32],
) -> Result<Zeroizing<[u8; 32]>> {
    let path = directory.join(WRAPPING_KEY_FILE);
    match load_wrapping_key(&path, root_key) {
        Ok(key) => Ok(key),
        Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            create_wrapping_key(directory, &path, root_key)
        }
        Err(error) => Err(error),
    }
}

fn load_wrapping_key(path: &Path, root_key: &[u8; 32]) -> Result<Zeroizing<[u8; 32]>> {
    if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(Error::Key("key-encryption file is a symlink"));
    }
    let mut file = secure_open_read(path)?;
    validate_file_permissions(&file)?;
    let expected = WRAPPING_MAGIC.len() + NONCE_BYTES + 32 + 16;
    if usize::try_from(file.metadata()?.len()).map_err(|_| Error::Key("key file size"))? != expected
    {
        return Err(Error::Key("invalid key-encryption file length"));
    }
    let mut encoded = Zeroizing::new(Vec::with_capacity(expected));
    file.read_to_end(&mut encoded)?;
    if &encoded[..WRAPPING_MAGIC.len()] != WRAPPING_MAGIC {
        return Err(Error::Key("invalid key-encryption file magic"));
    }
    let nonce_start = WRAPPING_MAGIC.len();
    let ciphertext_start = nonce_start + NONCE_BYTES;
    let cipher = XChaCha20Poly1305::new(root_key.into());
    let plaintext = Zeroizing::new(
        cipher
            .decrypt(
                <&XNonce>::try_from(&encoded[nonce_start..ciphertext_start])
                    .map_err(|_| Error::Key("invalid key-encryption nonce"))?,
                Payload {
                    msg: &encoded[ciphertext_start..],
                    aad: b"foks-server-operator-root-v1",
                },
            )
            .map_err(|_| Error::KeyCrypto)?,
    );
    Ok(Zeroizing::new(plaintext.as_slice().try_into().map_err(
        |_| Error::Key("invalid key-encryption plaintext"),
    )?))
}

fn create_wrapping_key(
    directory: &Path,
    path: &Path,
    root_key: &[u8; 32],
) -> Result<Zeroizing<[u8; 32]>> {
    let mut wrapping_key = Zeroizing::new([0; 32]);
    getrandom::fill(&mut *wrapping_key).map_err(|_| Error::Key("operating-system entropy"))?;
    let encoded = encode_wrapping_key(&wrapping_key, root_key)?;
    let temporary = temporary_path(directory, "key-encryption")?;
    let result = (|| {
        write_new_secret(&temporary, &encoded)?;
        match std::fs::hard_link(&temporary, path) {
            Ok(()) => {
                sync_directory(directory)?;
                Ok(wrapping_key)
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                load_wrapping_key(path, root_key)
            }
            Err(error) => Err(error.into()),
        }
    })();
    let _ = std::fs::remove_file(temporary);
    result
}

fn encode_wrapping_key(key: &[u8; 32], root_key: &[u8; 32]) -> Result<Vec<u8>> {
    let mut nonce = [0; NONCE_BYTES];
    getrandom::fill(&mut nonce).map_err(|_| Error::Key("operating-system entropy"))?;
    let cipher = XChaCha20Poly1305::new(root_key.into());
    let ciphertext = cipher
        .encrypt(
            &XNonce::from(nonce),
            Payload {
                msg: key,
                aad: b"foks-server-operator-root-v1",
            },
        )
        .map_err(|_| Error::KeyCrypto)?;
    let mut encoded = Vec::with_capacity(WRAPPING_MAGIC.len() + nonce.len() + ciphertext.len());
    encoded.extend_from_slice(WRAPPING_MAGIC);
    encoded.extend_from_slice(&nonce);
    encoded.extend_from_slice(&ciphertext);
    Ok(encoded)
}

fn temporary_path(directory: &Path, label: &str) -> Result<PathBuf> {
    let mut suffix = [0; 8];
    getrandom::fill(&mut suffix).map_err(|_| Error::Key("operating-system entropy"))?;
    Ok(directory.join(format!(".{label}.{}.tmp", u64::from_be_bytes(suffix))))
}

fn write_new_secret(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = secure_create(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn open_rotation_lock(directory: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    Ok(options.open(directory.join(ROTATION_LOCK_FILE))?)
}

fn associated_data(purpose: KeyPurpose, generation: &[u8; GENERATION_BYTES]) -> Vec<u8> {
    let mut aad = Vec::with_capacity(20 + purpose.label().len() + generation.len());
    aad.extend_from_slice(b"foks-server-key-v1\0");
    aad.extend_from_slice(purpose.label().as_bytes());
    aad.extend_from_slice(generation);
    aad
}

impl HostKeyProvider for DirectoryKeyProvider {
    fn load_or_create(&self, purpose: KeyPurpose) -> Result<SecretKey> {
        let path = self.path(purpose);
        match self.load(&path, purpose) {
            Ok(key) => Ok(key),
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                self.create(&path, purpose)
            }
            Err(error) => Err(error),
        }
    }

    fn load_existing(&self, purpose: KeyPurpose) -> Result<SecretKey> {
        DirectoryKeyProvider::load_existing(self, purpose)
    }

    fn create_generation(&self, purpose: KeyPurpose) -> Result<SecretKey> {
        if !self.exclusive {
            return Err(Error::Config(
                "new key generations require an exclusive rotation lock",
            ));
        }
        // The random generation ID is part of the authenticated ciphertext and
        // the immutable filename. A collision is retried rather than opening
        // an existing generation whose secret the caller did not create.
        for _ in 0..8 {
            let mut generation = [0; GENERATION_BYTES];
            getrandom::fill(&mut generation).map_err(|_| Error::Key("operating-system entropy"))?;
            let generation = KeyGenerationId::from_bytes(generation);
            let path = self.generation_path(purpose, generation);
            match self.create_with_generation(&path, purpose, generation) {
                Ok(key) => return Ok(key),
                Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(Error::Key("repeated key generation collision"))
    }

    fn load_generation(
        &self,
        purpose: KeyPurpose,
        generation: KeyGenerationId,
    ) -> Result<SecretKey> {
        let generated = self.generation_path(purpose, generation);
        let key = match self.load(&generated, purpose) {
            Ok(key) => key,
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                let canonical = self.load(&self.path(purpose), purpose)?;
                if canonical.generation() != generation {
                    return Err(Error::Key("key generation does not exist"));
                }
                canonical
            }
            Err(error) => return Err(error),
        };
        if key.generation() != generation {
            return Err(Error::Key("key generation filename mismatch"));
        }
        Ok(key)
    }

    fn list_generations(&self, purpose: KeyPurpose) -> Result<Vec<KeyGenerationId>> {
        let prefix = format!("{}.", purpose.label());
        let mut generations = Vec::new();
        for entry in std::fs::read_dir(&self.directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(hex) = name
                .strip_prefix(&prefix)
                .and_then(|name| name.strip_suffix(".key"))
            else {
                continue;
            };
            if hex.len() != 32 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(Error::Key("invalid generation-qualified key filename"));
            }
            let mut generation = [0; GENERATION_BYTES];
            for (index, output) in generation.iter_mut().enumerate() {
                *output = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
                    .map_err(|_| Error::Key("invalid generation-qualified key filename"))?;
            }
            generations.push(KeyGenerationId::from_bytes(generation));
        }
        generations.sort();
        generations.dedup();
        Ok(generations)
    }

    fn remove_generation(&self, purpose: KeyPurpose, generation: KeyGenerationId) -> Result<()> {
        if !self.exclusive {
            return Err(Error::Config(
                "key retirement requires an exclusive rotation lock",
            ));
        }
        let generated = self.generation_path(purpose, generation);
        match self.load(&generated, purpose) {
            Ok(key) => {
                if key.generation() != generation {
                    return Err(Error::Key("key generation filename mismatch"));
                }
                std::fs::remove_file(generated)?;
                sync_directory(&self.directory)?;
                return Ok(());
            }
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }

        // The genesis host generation uses the canonical `host.key` name.
        // Later generations always use generation-qualified immutable names.
        let canonical = self.path(purpose);
        match self.load(&canonical, purpose) {
            Ok(key) if key.generation() == generation => {
                std::fs::remove_file(canonical)?;
                sync_directory(&self.directory)?;
                Ok(())
            }
            Ok(_) => Ok(()),
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

#[cfg(unix)]
fn secure_open_read(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
fn secure_open_read(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().read(true).open(path)
}

fn secure_create(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path)
}

#[cfg(unix)]
fn set_directory_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_directory_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn validate_file_permissions(file: &File) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    if file.metadata()?.permissions().mode() & 0o077 != 0 {
        return Err(Error::Key("key file is accessible by another user"));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_file_permissions(_file: &File) -> Result<()> {
    Ok(())
}

fn sync_directory(path: &Path) -> std::io::Result<()> {
    File::open(path)?.sync_all()
}

fn cleanup_private_temporaries(directory: &Path) -> Result<()> {
    let mut removed = false;
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(body) = name
            .strip_prefix('.')
            .and_then(|name| name.strip_suffix(".tmp"))
        else {
            continue;
        };
        let Some((label, random)) = body.rsplit_once('.') else {
            continue;
        };
        let owned_label = MANAGED_TEMPORARY_LABELS.contains(&label);
        if !owned_label || random.parse::<u64>().is_err() {
            continue;
        }
        let metadata = std::fs::symlink_metadata(entry.path())?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(Error::Key("invalid private-key temporary file"));
        }
        std::fs::remove_file(entry.path())?;
        removed = true;
    }
    if removed {
        sync_directory(directory)?;
    }
    Ok(())
}

const MANAGED_TEMPORARY_LABELS: &[&str] = &[
    "host",
    "metadata",
    "merkle",
    "client-ca",
    "delegated-tls",
    "recovery",
    "capability",
    "operator-root",
    "key-encryption",
];
