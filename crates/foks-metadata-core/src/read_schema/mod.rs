//! Immutable schema-profile descriptions, not database admission capabilities.
//!
//! No public constructor accepts expected DDL, hashes, or a schema cookie. The
//! generated qualification profiles will be integrated after their source and
//! artifact review. A description alone does not attest a live connection,
//! tenant, owner, key, artifact, storage binding, or successful commit/sync.
use sha2::{Digest, Sha256};

pub const PROFILE_BYTES: usize = 296;
const HASH_DOMAIN: &[u8] = b"foks-hosted-read-schema-profile-v1\0";

/// These IDs are deliberately outside the future production profile space.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QualificationProfileId {
    NativeSchema59,
    Celld062Schema59,
}
impl QualificationProfileId {
    pub fn encoded(self) -> u32 {
        match self {
            Self::NativeSchema59 => 0x8000_0001,
            Self::Celld062Schema59 => 0x8000_0002,
        }
    }
}

/// An immutable descriptor, not a VerifiedSchema. Construction stays inside this
/// module's reviewed generated-profile integration; callers cannot mint one from
/// supplied contract/catalog hashes. The current component exposes the pure
/// encoding only and installs no static profiles or runtime serving factory.
#[derive(Debug)]
pub struct ReadSchemaProfile {
    id: QualificationProfileId,
    // FOKS, authority, engine, exact union, scalar catalog, page catalog, ABI.
    hashes: [[u8; 32]; 7],
}
impl ReadSchemaProfile {
    pub fn id(&self) -> QualificationProfileId {
        self.id
    }

    /// Fixed private encoding, independent of native pointer width/struct layout.
    pub fn encoded(&self) -> [u8; PROFILE_BYTES] {
        let mut out = [0u8; PROFILE_BYTES];
        out[..4].copy_from_slice(b"FKSP");
        out[4..6].copy_from_slice(&1u16.to_le_bytes());
        // Reserved bytes 6..8 remain zero.
        let fields = [
            self.id.encoded(),
            59, // FOKS schema version
            1,  // unchanged native physical representation
            1,  // shared schema checker version
            1,  // activation ABI
            1,  // guarded read ABI (deployment signatures frozen separately)
            1,  // page ABI
            2,  // private binding domain version
            1,  // private token version
        ];
        for (slot, value) in out[8..44].chunks_exact_mut(4).zip(fields) {
            slot.copy_from_slice(&value.to_le_bytes());
        }
        for (slot, hash) in out[44..268].chunks_exact_mut(32).zip(self.hashes) {
            slot.copy_from_slice(&hash);
        }
        let bounds = [1024u32, 2_097_152, 65_536, 256, 1_048_576, 64, 1024];
        for (slot, value) in out[268..].chunks_exact_mut(4).zip(bounds) {
            slot.copy_from_slice(&value.to_le_bytes());
        }
        out
    }

    /// Domain separated from the unchanged native FOKS-only contract hash.
    pub fn composite_hash(&self) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(HASH_DOMAIN);
        digest.update(self.encoded());
        digest.finalize().into()
    }
}

#[cfg(test)]
mod tests;
