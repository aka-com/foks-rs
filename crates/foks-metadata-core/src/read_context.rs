//! Bounded private host-context data. Decoding is not authentication, schema
//! admission, ownership proof, or permission to execute/publish a request.
//!
//! The caller retains the admitted context allowance for the lifetime of this
//! value. No Wasm export or production route accepts this type yet.
mod decode;

use crate::authority::Fence;
use crate::personal::PeerCertificate;

pub const MAX_CONTEXT_BYTES: usize = 18_496;
const VARIABLE_BYTES: usize = 4096 + 20 + 33;
const LAYOUT_BYTES: usize = 2 * 2048;
const ERROR_BYTES: usize = 256;
const NUMBER_TEMPORARY_BYTES: usize = 32;

/// A fixed classification, never a copy of rejected input or a public RPC status.
/// Admission denotes the precharge hook's refusal. A bounded parser allocation
/// failure uses the same static Invalid classification as other parser failures;
/// neither is a publishable public error or permission to continue work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextError {
    InputBound,
    Admission,
    Invalid,
}
impl ContextError {
    pub fn code(self) -> &'static str {
        match self {
            Self::InputBound => "context_input_bound",
            Self::Admission => "context_admission",
            Self::Invalid => "invalid_host_context",
        }
    }
}

/// Structurally checked data supplied by a trusted host, not an authority guard.
pub struct HostListContext {
    peer: PeerCertificate,
    fence: Fence,
    now_micros: u64,
    execution_deadline_ms: i64,
}
impl HostListContext {
    pub fn decode(bytes: &[u8]) -> Result<Self, ContextError> {
        if bytes.is_empty() || bytes.len() > MAX_CONTEXT_BYTES {
            return Err(ContextError::InputBound);
        }
        // One retained reservation covers reusable parser temporaries. It is not
        // a claim of zero allocator traffic while parsing thousands of bytes.
        let allowance =
            bytes.len() + VARIABLE_BYTES + LAYOUT_BYTES + ERROR_BYTES + NUMBER_TEMPORARY_BYTES;
        foks_snowpack::reserve_allocation(allowance).map_err(|_| ContextError::Admission)?;
        lexical_preflight(bytes)?;
        // Custom visitors use only static errors. Do not format this error: even
        // serde_json's Debug implementation constructs diagnostic strings.
        let parts: decode::ContextParts =
            serde_json::from_slice(bytes).map_err(|_| ContextError::Invalid)?;
        parts.into_context()
    }

    pub fn peer(&self) -> &PeerCertificate {
        &self.peer
    }
    pub fn fence(&self) -> &Fence {
        &self.fence
    }
    pub fn now_micros(&self) -> u64 {
        self.now_micros
    }
    pub fn now_ms(&self) -> i64 {
        // Construction checked the original microsecond sample against i64::MAX.
        (self.now_micros / 1000) as i64
    }
    pub fn execution_deadline_ms(&self) -> i64 {
        self.execution_deadline_ms
    }
}

fn lexical_preflight(bytes: &[u8]) -> Result<(), ContextError> {
    let mut stack = [0u8; 4];
    let mut depth = 0usize;
    let mut quoted = false;
    let mut at = 0usize;
    while at < bytes.len() {
        let byte = bytes[at];
        if byte == b'\\' || !byte.is_ascii() {
            return Err(ContextError::Invalid);
        }
        if quoted {
            if byte == b'"' {
                quoted = false;
            } else if !(0x20..=0x7e).contains(&byte) {
                return Err(ContextError::Invalid);
            }
        } else {
            match byte {
                b'"' => quoted = true,
                b'{' | b'[' => {
                    if depth == stack.len() {
                        return Err(ContextError::Invalid);
                    }
                    stack[depth] = byte;
                    depth += 1;
                }
                b'}' | b']' => {
                    let expected = if byte == b'}' { b'{' } else { b'[' };
                    if depth == 0 || stack[depth - 1] != expected {
                        return Err(ContextError::Invalid);
                    }
                    depth -= 1;
                }
                b'0'..=b'9' => {
                    let start = at;
                    while at < bytes.len() && bytes[at].is_ascii_digit() {
                        at += 1;
                    }
                    if at - start > 3 || (at - start > 1 && byte == b'0') {
                        return Err(ContextError::Invalid);
                    }
                    continue;
                }
                b':' | b',' | b' ' | b'\n' | b'\r' | b'\t' => {}
                _ => return Err(ContextError::Invalid),
            }
        }
        at += 1;
    }
    if quoted || depth != 0 {
        return Err(ContextError::Invalid);
    }
    Ok(())
}

const _: () = assert!(std::mem::size_of::<HostListContext>() <= 2048);

#[cfg(test)]
mod tests;
