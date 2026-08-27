mod directory;
mod manifest;
mod memory;

pub use directory::DirectoryKeyProvider;
pub use manifest::{KeyGenerationManifest, MANIFEST_PURPOSES};
pub use memory::MemoryKeyProvider;

use std::fs::{File, OpenOptions};
use std::io::Read as _;
use std::path::Path;

use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum KeyPurpose {
    Host,
    Metadata,
    Merkle,
    ClientCa,
    DelegatedTls,
}

impl KeyPurpose {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Metadata => "metadata",
            Self::Merkle => "merkle",
            Self::ClientCa => "client-ca",
            Self::DelegatedTls => "delegated-tls",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct KeyGenerationId([u8; 16]);

impl KeyGenerationId {
    pub fn as_bytes(self) -> [u8; 16] {
        self.0
    }

    fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

pub struct SecretKey {
    bytes: Zeroizing<[u8; 32]>,
    generation: KeyGenerationId,
}

impl SecretKey {
    pub fn expose(&self) -> &[u8; 32] {
        &self.bytes
    }

    pub fn generation(&self) -> KeyGenerationId {
        self.generation
    }

    fn new(bytes: [u8; 32], generation: [u8; 16]) -> Self {
        Self {
            bytes: Zeroizing::new(bytes),
            generation: KeyGenerationId::new(generation),
        }
    }
}

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SecretKey([REDACTED])")
    }
}

pub trait HostKeyProvider: Send + Sync {
    fn load_or_create(&self, purpose: KeyPurpose) -> crate::Result<SecretKey>;
}

pub fn read_secret_file(
    path: impl AsRef<Path>,
    maximum_bytes: usize,
) -> crate::Result<Zeroizing<Vec<u8>>> {
    let path = path.as_ref();
    if maximum_bytes == 0 || std::fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(crate::Error::Key(
            "secret file is a symlink or has a zero limit",
        ));
    }
    let mut file = secure_open_secret(path)?;
    validate_secret_file(&file, maximum_bytes)?;
    let length = usize::try_from(file.metadata()?.len())
        .map_err(|_| crate::Error::Key("secret file size"))?;
    let mut bytes = Zeroizing::new(Vec::with_capacity(length));
    let read_limit = u64::try_from(maximum_bytes)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    (&mut file).take(read_limit).read_to_end(&mut bytes)?;
    if bytes.len() != length {
        return Err(crate::Error::Key("secret file changed while reading"));
    }
    Ok(bytes)
}

pub fn read_root_key_file(path: impl AsRef<Path>) -> crate::Result<Zeroizing<[u8; 32]>> {
    let bytes = read_secret_file(path, 32)?;
    let key = bytes
        .as_slice()
        .try_into()
        .map_err(|_| crate::Error::Key("root key must contain exactly 32 bytes"))?;
    Ok(Zeroizing::new(key))
}

#[cfg(unix)]
fn secure_open_secret(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
fn secure_open_secret(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().read(true).open(path)
}

fn validate_secret_file(file: &File, maximum_bytes: usize) -> crate::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > u64::try_from(maximum_bytes).unwrap_or(u64::MAX)
    {
        return Err(crate::Error::Key("invalid secret file size or type"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(crate::Error::Key(
                "secret file is accessible by another user",
            ));
        }
    }
    Ok(())
}
