//! Selected List-auth caller copies only; scalar transport has separate charges.
//! Reservations stay with the current invocation. No scratch refund or proof.
use super::{PersonalError, Result};
use crate::Value;

fn allocate(length: usize) -> Result<Vec<u8>> {
    allocate_with(length, |bytes, length| {
        bytes.try_reserve_exact(length).map_err(|_| ())
    })
}
fn allocate_with(
    length: usize,
    reserve: impl FnOnce(&mut Vec<u8>, usize) -> std::result::Result<(), ()>,
) -> Result<Vec<u8>> {
    crate::page::charge(length)?;
    let mut bytes = Vec::new();
    reserve(&mut bytes, length).map_err(|()| {
        #[cfg(feature = "list-allocation-qualification")]
        crate::allocation_trace::allocator_failed();
        PersonalError::Kernel(crate::Error::Bounds)
    })?;
    Ok(bytes)
}
pub(super) fn copy(input: &[u8]) -> Result<Vec<u8>> {
    let mut bytes = allocate(input.len())?;
    bytes.extend_from_slice(input);
    Ok(bytes)
}
pub(super) fn zeroed(length: usize) -> Result<Vec<u8>> {
    let mut bytes = allocate(length)?;
    bytes.resize(length, 0);
    Ok(bytes)
}
pub(super) fn into_blob(value: Value) -> Result<Vec<u8>> {
    match value {
        Value::Blob(bytes) => Ok(bytes),
        _ => Err(PersonalError::Corrupt),
    }
}

#[cfg(test)]
mod tests;
