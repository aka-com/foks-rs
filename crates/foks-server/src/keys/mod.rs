mod directory;
mod manifest;
mod memory;

pub use directory::DirectoryKeyProvider;
pub use manifest::{KeyGenerationManifest, MANIFEST_PURPOSES};
pub use memory::MemoryKeyProvider;

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
