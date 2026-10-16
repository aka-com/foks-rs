//! Private persisted-state admission, preserving the existing typed serde codec.
//! The reservation is a live requested-allocation envelope, not allocator traffic.
use super::{Activation, Identity};
use crate::{Error, Result, MAX_BLOB_BYTES};
use serde::de::DeserializeOwned;

const STATIC_ERROR_BYTES: usize = 512;
const ERROR_RECORD_BYTES: usize = 256;
const NUMBER_TEMPORARY_BYTES: usize = 64;

fn allowance(bytes: usize) -> Result<usize> {
    if bytes > MAX_BLOB_BYTES {
        return Err(Error::Bounds);
    }
    // Two scratch/growing buffers, one Number String/canonical temporary, one
    // reusable numeric token, and two conservatively overlapping error messages.
    let grow = bytes.checked_mul(3).and_then(|n| n.checked_add(8));
    let message = bytes
        .checked_mul(10)
        .and_then(|n| n.checked_add(STATIC_ERROR_BYTES));
    grow.and_then(|n| n.checked_mul(2))
        .and_then(|n| n.checked_add(bytes))
        .and_then(|n| n.checked_add(68 + NUMBER_TEMPORARY_BYTES))
        .and_then(|n| n.checked_add(message?.checked_mul(6)?))
        .and_then(|n| n.checked_add(ERROR_RECORD_BYTES))
        .ok_or(Error::Bounds)
}

// Only raw unsigned integer tokens can occur in these two successful typed
// values. Signed Number fields are strings. Avoid invalid-only serde floating
// point/bigint paths, without replacing its object/sequence/string validation.
fn numeric_guard(bytes: &[u8]) -> Result<()> {
    let mut quoted = false;
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        if quoted {
            match byte {
                b'\\' => {
                    // Any invalid escape still fails in serde before a hidden
                    // token can be parsed. Escaped quotes do not end a string.
                    at += 1;
                    if at == bytes.len() {
                        return Err(Error::Storage);
                    }
                }
                b'"' => quoted = false,
                _ => {}
            }
        } else {
            match byte {
                b'"' => quoted = true,
                b'-' | b'.' | b'e' | b'E' => return Err(Error::Storage),
                b'0'..=b'9' => {
                    let start = at;
                    let mut value = 0u64;
                    while at < bytes.len() && bytes[at].is_ascii_digit() {
                        if at - start == 20 {
                            return Err(Error::Storage);
                        }
                        value = value
                            .checked_mul(10)
                            .and_then(|n| n.checked_add(u64::from(bytes[at] - b'0')))
                            .ok_or(Error::Storage)?;
                        at += 1;
                    }
                    continue;
                }
                _ => {}
            }
        }
        at += 1;
    }
    Ok(())
}

fn decode<T: DeserializeOwned>(text: &str) -> Result<T> {
    // The parent retains every reservation until invocation disposal, including
    // after parsing and while Identity.host remains in State. Never reset/refund.
    crate::page::charge(allowance(text.len())?)?;
    numeric_guard(text.as_bytes())?;
    // Existing infallible serde allocations are covered logically; global OOM
    // still follows process/instance disposal, not a recoverable Bounds promise.
    serde_json::from_str(text).map_err(|_| Error::Storage)
}
pub(super) fn identity(text: &str) -> Result<Identity> {
    decode(text)
}
pub(super) fn activation(text: &str) -> Result<Activation> {
    decode(text)
}

#[cfg(test)]
mod tests;
