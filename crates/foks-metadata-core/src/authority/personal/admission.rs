//! Resolver-only fixed digest work and admitted retained role bytes.
//! Not a generic authority codec, memory-owner capability, or List allowance.
use crate::personal::{CurrentCredential, PeerCertificate, PersonalError, Result};
use foks_proto::Role;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{self, Write};

const PEER_BYTES: usize = 4 * (4096 + 20 + 32) + 119;
const PRINCIPAL_BYTES: usize = 4 * (33 + 33 + 34) + 36;

struct HashWriter {
    hash: Sha256,
    bytes: usize,
    maximum: usize,
    failed: bool,
}
impl HashWriter {
    fn new(maximum: usize) -> Self {
        Self {
            hash: Sha256::new(),
            bytes: 0,
            maximum,
            failed: false,
        }
    }
    fn finish(self) -> Result<[u8; 32]> {
        if self.failed {
            Err(crate::Error::Bounds.into())
        } else {
            Ok(self.hash.finalize().into())
        }
    }
}
impl Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        // The serializer may continue after an exceeded ceiling. Consume without
        // hashing or allocating an io/serde error; finish refuses every prefix.
        if !self.failed {
            match self.bytes.checked_add(bytes.len()) {
                Some(total) if total <= self.maximum => {
                    self.hash.update(bytes);
                    self.bytes = total;
                }
                _ => self.failed = true,
            }
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
// Only the two audited finite tuples below call this private helper. Their
// serializers use stack state, borrowed fields and stack integer formatting.
fn digest(value: &impl Serialize, maximum: usize) -> Result<[u8; 32]> {
    let mut writer = HashWriter::new(maximum);
    serde_json::to_writer(&mut writer, value)
        .map_err(|_| PersonalError::Kernel(crate::Error::Bounds))?;
    writer.finish()
}
pub(super) fn peer_context_digest(peer: &PeerCertificate) -> Result<[u8; 32]> {
    digest(&("foks-personal-peer-v1", peer), PEER_BYTES)
}
pub(super) fn principal_digest(current: &CurrentCredential) -> Result<[u8; 32]> {
    digest(
        &(
            "foks-personal-principal-v1",
            current.uid().as_slice(),
            current.credential_id(),
            current.owner_id(),
        ),
        PRINCIPAL_BYTES,
    )
}

// This private fixed Role schema avoids the generic encoder's temporary Value
// tree/path allocations. Exhaustive parity against Role::encoded is required.
fn role_bytes(role: Role) -> ([u8; 8], usize) {
    let mut bytes = [0u8; 8];
    bytes[0] = 0x92;
    bytes[1] = role.protocol_value() as u8;
    let Some(visibility) = role.visibility() else {
        bytes[2] = 0x80;
        return (bytes, 3);
    };
    bytes[2..5].copy_from_slice(&[0x81, 0xa1, b'0']);
    let integer = &mut bytes[5..];
    let length = match visibility {
        -32..=127 => {
            integer[0] = visibility as i8 as u8;
            1
        }
        -128..=-33 => {
            integer[..2].copy_from_slice(&[0xd0, visibility as i8 as u8]);
            2
        }
        128..=255 => {
            integer[..2].copy_from_slice(&[0xcc, visibility as u8]);
            2
        }
        _ => {
            integer[0] = if visibility < 0 { 0xd1 } else { 0xcd };
            integer[1..3].copy_from_slice(&visibility.to_be_bytes());
            3
        }
    };
    (bytes, 5 + length)
}
pub(super) fn role(role: Role) -> Result<Vec<u8>> {
    let (bytes, length) = role_bytes(role);
    crate::personal::admission::copy(&bytes[..length])
}

#[cfg(test)]
mod tests;
