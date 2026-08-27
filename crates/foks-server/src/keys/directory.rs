use std::fs::{File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use chacha20poly1305::aead::{Aead as _, KeyInit as _, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

use super::{HostKeyProvider, KeyPurpose, SecretKey};
use crate::{Error, Result};

const MAGIC: &[u8; 8] = b"FOKSK01\0";
const GENERATION_BYTES: usize = 16;
const NONCE_BYTES: usize = 24;

pub struct DirectoryKeyProvider {
    directory: PathBuf,
    root_key: Zeroizing<[u8; 32]>,
}

impl DirectoryKeyProvider {
    pub fn open(directory: impl AsRef<Path>, root_key: [u8; 32]) -> Result<Self> {
        let directory = directory.as_ref().to_path_buf();
        std::fs::create_dir_all(&directory)?;
        set_directory_permissions(&directory)?;
        if std::fs::symlink_metadata(&directory)?
            .file_type()
            .is_symlink()
        {
            return Err(Error::Key("key directory is a symlink"));
        }
        Ok(Self {
            directory,
            root_key: Zeroizing::new(root_key),
        })
    }

    fn path(&self, purpose: KeyPurpose) -> PathBuf {
        self.directory.join(format!("{}.key", purpose.label()))
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
        let cipher = XChaCha20Poly1305::new((&*self.root_key).into());
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
        let mut key = Zeroizing::new([0; 32]);
        getrandom::fill(&mut *key).map_err(|_| Error::Key("operating-system entropy"))?;
        let mut nonce = [0; NONCE_BYTES];
        getrandom::fill(&mut nonce).map_err(|_| Error::Key("operating-system entropy"))?;
        let mut generation = [0; GENERATION_BYTES];
        getrandom::fill(&mut generation).map_err(|_| Error::Key("operating-system entropy"))?;
        let aad = associated_data(purpose, &generation);
        let cipher = XChaCha20Poly1305::new((&*self.root_key).into());
        let ciphertext = cipher
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: &*key,
                    aad: &aad,
                },
            )
            .map_err(|_| Error::KeyCrypto)?;
        let mut suffix = [0; 8];
        getrandom::fill(&mut suffix).map_err(|_| Error::Key("operating-system entropy"))?;
        let temporary = self.directory.join(format!(
            ".{}.{}.tmp",
            purpose.label(),
            u64::from_be_bytes(suffix)
        ));
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
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    self.load(path, purpose)
                }
                Err(error) => Err(error.into()),
            }
        })();
        let _ = std::fs::remove_file(temporary);
        result
    }
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
