//! Allocation ownership only: not transaction, authentication or publication proof.
use super::{ListEncoder, Retention};
use crate::{Error, Result};
use foks_proto::{KvDirent, KvSmallFileBox};
use std::sync::Arc;

/// The original invocation's cumulative allocation owner, retained through bytes.
/// Implementations must keep accepted reservations until their final destruction.
/// This trait provides no SQL, authentication, durability or serving authority.
pub trait ListRetainedOwner:
    foks_snowpack::AllocationBudget + foks_rpc::ResponseAllocationBudget
{
    fn reserve_retained(&self, bytes: usize) -> bool;
    /// One held page/control reservation, separate from cumulative retained work.
    fn reserve_page_workspace(&self, _bytes: usize) -> bool {
        false
    }
    fn failed(&self) -> bool;
    fn fail(&self);
}

// This guard is declared outside hook scopes. Restoration runs first on unwind;
// normal domain errors disarm it and preserve the established failure semantics.
pub(crate) struct UnwindFailure {
    owner: Arc<dyn ListRetainedOwner>,
    complete: bool,
}
impl UnwindFailure {
    pub(crate) fn new(owner: Arc<dyn ListRetainedOwner>) -> Self {
        Self {
            owner,
            complete: false,
        }
    }
    pub(crate) fn complete(&mut self) {
        self.complete = true;
    }
}
impl Drop for UnwindFailure {
    fn drop(&mut self) {
        if !self.complete {
            self.owner.fail();
        }
    }
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
    pub(crate) fn owner(&self) -> Arc<dyn ListRetainedOwner> {
        let Retention::Owned(owner) = &self.inner.retention else {
            unreachable!()
        };
        owner.clone()
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

/// Exact List bytes retaining their original allocation owner.
pub struct OwnedListBytes {
    bytes: Vec<u8>,
    owner: Arc<dyn ListRetainedOwner>,
}
impl OwnedListBytes {
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn len(&self) -> usize {
        self.bytes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// Exact framed bytes retaining their original allocation owner.
pub struct OwnedFrameBytes {
    bytes: Vec<u8>,
    _owner: Arc<dyn ListRetainedOwner>,
}
impl OwnedFrameBytes {
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn len(&self) -> usize {
        self.bytes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// Frame using the captured owner's working and response hooks, restoring any
/// unrelated ambient owner afterward. This confers no publication authority;
/// callers must still complete their transaction, durability and freshness gates.
/// No bare Vec transfer or reservation refund occurs.
pub fn frame_owned_list(value: OwnedListBytes, sequence: u64) -> foks_rpc::Result<OwnedFrameBytes> {
    let owner = value.owner;
    let raw = value.bytes;
    if owner.failed() {
        drop(raw);
        return Err(foks_rpc::Error::Io(std::io::ErrorKind::WouldBlock.into()));
    }
    let mut unwind = UnwindFailure::new(owner.clone());
    let allocation: Arc<dyn foks_snowpack::AllocationBudget> = owner.clone();
    let response: Arc<dyn foks_rpc::ResponseAllocationBudget> = owner.clone();
    // Preserve the original framing error; neither budget may come from a
    // different ambient invocation. Arc upcasts allocate no adapter/control box.
    let result = foks_snowpack::with_allocation_budget(allocation, || {
        foks_rpc::with_response_budget(response, || {
            foks_rpc::encode_success_response_owned_at(raw, sequence)
        })
    });
    unwind.complete();
    let bytes = match result {
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
