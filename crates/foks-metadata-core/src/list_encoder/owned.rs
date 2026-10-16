//! Qualification ownership boundary, not a scratch budget or authority proof.
use super::{ListEncoder, Retention};
use crate::{Error, Result};
use foks_proto::{KvDirent, KvSmallFileBox};
use std::sync::Arc;

pub(crate) trait ListRetainedOwner: Send + Sync {
    fn reserve_retained(&self, bytes: usize) -> bool;
    fn failed(&self) -> bool;
    fn fail(&self);
}

pub(crate) struct OwnedListEncoder {
    inner: ListEncoder,
}
impl OwnedListEncoder {
    pub(crate) fn new(maximum_entries: usize, owner: Arc<dyn ListRetainedOwner>) -> Result<Self> {
        let mut inner = ListEncoder::new(maximum_entries)?;
        inner.retention = Retention::Owned(owner);
        Ok(Self { inner })
    }
    pub(crate) fn append(
        &mut self,
        entry: &KvDirent,
        small: Option<&KvSmallFileBox>,
    ) -> Result<()> {
        let Retention::Owned(owner) = &self.inner.retention else {
            unreachable!()
        };
        if owner.failed() {
            return Err(Error::Bounds);
        }
        let result = self.inner.append(entry, small);
        if result.is_err() {
            self.inner.retention.fail();
        }
        result
    }
    pub(crate) fn finish(self, final_page: bool) -> Result<OwnedListBytes> {
        let Retention::Owned(owner) = &self.inner.retention else {
            unreachable!()
        };
        // Declared before the output; retain ownership while finish consumes the encoder.
        let owner = owner.clone();
        let bytes = self.inner.finish(final_page)?;
        if owner.failed() {
            drop(bytes);
            return Err(Error::Bounds);
        }
        Ok(OwnedListBytes { bytes, owner })
    }
}

pub(crate) struct OwnedListBytes {
    bytes: Vec<u8>,
    owner: Arc<dyn ListRetainedOwner>,
}
impl OwnedListBytes {
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }
}

pub(crate) struct OwnedFrameBytes {
    bytes: Vec<u8>,
    _owner: Arc<dyn ListRetainedOwner>,
}
impl OwnedFrameBytes {
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }
}

/// The original response hook remains installed by the invocation. Never expose
/// a bare Vec transfer or reset/refund the retained owner during framing.
pub(crate) fn frame_owned_list(
    value: OwnedListBytes,
    sequence: u64,
) -> foks_rpc::Result<OwnedFrameBytes> {
    let owner = value.owner;
    let raw = value.bytes;
    if owner.failed() {
        drop(raw);
        return Err(foks_rpc::Error::Io(std::io::ErrorKind::WouldBlock.into()));
    }
    // Preserve the original framing error if framing itself fails.
    let bytes = match foks_rpc::encode_success_response_owned_at(raw, sequence) {
        Ok(bytes) => bytes,
        Err(error) => {
            owner.fail();
            return Err(error);
        }
    };
    if owner.failed() {
        drop(bytes);
        return Err(foks_rpc::Error::Io(std::io::ErrorKind::WouldBlock.into()));
    }
    Ok(OwnedFrameBytes {
        bytes,
        _owner: owner,
    })
}

#[cfg(test)]
mod tests;
