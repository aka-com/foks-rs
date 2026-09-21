use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::Zeroizing;

use crate::{KvUploadFrame, Request, Response, PROTOCOL_VERSION};

pub const MAXIMUM_MESSAGE_BYTES: usize = 1024 * 1024;

/// Frame bytes held back from any payload bound, for everything that travels
/// beside the payload: the version, the correlation ID, the status or
/// operation tag, the store reference, the path and the response timing.
///
/// The largest of those is the path, at 4 KiB, and `serde_json` escapes a
/// control byte to six characters, so a pathological path costs 24 KiB of
/// frame. The profile, account alias and team alias are each bounded well
/// below that. 32 KiB covers all of it.
pub const FRAME_ENVELOPE_RESERVE_BYTES: usize = 32 * 1024;

/// The largest plaintext byte payload this protocol carries in one frame.
///
/// Byte payloads are base64 (see [`crate::base64_bytes`]), which costs
/// `ceil(n/3)*4` plus two quotes. 716,800 bytes encode to 955,738, which fits
/// the 1,015,808-byte budget that remains once
/// [`FRAME_ENVELOPE_RESERVE_BYTES`] is held back, with 60,070 bytes to spare.
///
/// Every KV bound in this workspace — the agent's chunk and inline limits,
/// the desktop's download chunk and the agent client's upload frame — is this
/// value, so the four cannot drift apart and out of the budget. A request
/// that asks for more is refused as an invalid request; nothing truncates.
pub const MAXIMUM_KV_PAYLOAD_BYTES: usize = 700 * 1024;

#[derive(Debug, Error)]
pub enum Error {
    #[error("agent message exceeds its size limit")]
    TooLarge,
    #[error("agent frame length does not match its payload")]
    Length,
    #[error("agent protocol version is unsupported")]
    Version,
    #[error("agent JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Deserialize)]
struct VersionEnvelope {
    version: u32,
}

pub fn encode<T: Serialize>(message: &T) -> Result<Zeroizing<Vec<u8>>> {
    let payload = Zeroizing::new(serde_json::to_vec(message)?);
    if payload.len() > MAXIMUM_MESSAGE_BYTES {
        return Err(Error::TooLarge);
    }
    let length = u32::try_from(payload.len()).map_err(|_| Error::TooLarge)?;
    let mut frame = Zeroizing::new(Vec::with_capacity(4 + payload.len()));
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

pub fn decode_request(frame: &[u8]) -> Result<Request> {
    validate_version(frame)?;
    let request: Request = decode(frame)?;
    Ok(request)
}

/// Recovers a correlation ID from a length-valid JSON envelope even when the
/// version or operation is unsupported. Malformed frames intentionally have
/// no response ID.
pub fn request_id(frame: &[u8]) -> Option<u64> {
    let value: serde_json::Value = decode(frame).ok()?;
    value.get("id")?.as_u64()
}

pub fn decode_response(frame: &[u8]) -> Result<Response> {
    validate_version(frame)?;
    let response: Response = decode(frame)?;
    Ok(response)
}

pub fn decode_upload_frame(frame: &[u8]) -> Result<KvUploadFrame> {
    validate_version(frame)?;
    let frame: KvUploadFrame = decode(frame)?;
    Ok(frame)
}

fn validate_version(frame: &[u8]) -> Result<()> {
    let envelope: VersionEnvelope = decode(frame)?;
    if envelope.version != PROTOCOL_VERSION {
        return Err(Error::Version);
    }
    Ok(())
}

fn decode<T: for<'de> Deserialize<'de>>(frame: &[u8]) -> Result<T> {
    if frame.len() < 4 {
        return Err(Error::Length);
    }
    let length = u32::from_be_bytes(frame[..4].try_into().expect("four-byte prefix")) as usize;
    if length > MAXIMUM_MESSAGE_BYTES {
        return Err(Error::TooLarge);
    }
    if frame.len() != 4 + length {
        return Err(Error::Length);
    }
    Ok(serde_json::from_slice(&frame[4..])?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base64_bytes::encoded_frame_bytes;
    use crate::{AccountStoreRef, KvChunkResult, KvStoreRef, Response, ResponseTiming};

    /// A timing block larger than the agent sends: every counter set and a
    /// long admission label, so the reserve is measured against more than the
    /// empty default.
    fn worst_case_timing() -> ResponseTiming {
        ResponseTiming {
            queue_ms: u32::MAX,
            pool_ms: u32::MAX,
            start_ms: u32::MAX,
            session_ms: u32::MAX,
            lock_ms: u32::MAX,
            lock_retries: u16::MAX,
            body_ms: u32::MAX,
            auth_cached: true,
            report_cached: true,
            waited_behind: Some("w".repeat(256)),
            prepare_ms: u32::MAX,
            wait_ms: u32::MAX,
            rescope_ms: u32::MAX,
            phases: (0..64)
                .map(|index| (format!("phase-{index}"), u32::MAX))
                .collect(),
        }
    }

    /// The worst envelope a chunk response can carry: the longest path this
    /// protocol accepts, made entirely of control bytes, which `serde_json`
    /// escapes to six characters each, beside the longest store reference.
    fn worst_case_store_and_path() -> (KvStoreRef, String) {
        (
            KvStoreRef::Account(AccountStoreRef {
                profile: "\u{1}".repeat(64),
                account_alias: "\u{1}".repeat(256),
            }),
            format!("/{}", "\u{1}".repeat(4095)),
        )
    }

    #[test]
    fn the_largest_kv_payload_fits_the_frame_budget_once_encoded() {
        let encoded = encoded_frame_bytes(MAXIMUM_KV_PAYLOAD_BYTES);
        assert_eq!(encoded, 955_738);
        let budget = MAXIMUM_MESSAGE_BYTES - FRAME_ENVELOPE_RESERVE_BYTES;
        assert_eq!(budget, 1_015_808);
        assert!(encoded <= budget, "{encoded} exceeds {budget}");
    }

    /// The bound is not arithmetic on the payload alone: a chunk of exactly
    /// the bound, in the largest envelope the protocol admits, still encodes
    /// to a frame the peer accepts.
    #[test]
    fn a_maximum_chunk_in_the_worst_envelope_still_encodes() {
        let (store, path) = worst_case_store_and_path();
        let response = Response::success(
            u64::MAX,
            serde_json::to_value(KvChunkResult {
                store,
                path,
                version: u64::MAX,
                offset: u64::MAX,
                content: vec![0x5a; MAXIMUM_KV_PAYLOAD_BYTES],
                eof: true,
            })
            .unwrap(),
        )
        .with_timing(worst_case_timing());
        let frame = encode(&response).expect("largest chunk fits the frame");
        assert!(frame.len() <= 4 + MAXIMUM_MESSAGE_BYTES, "{}", frame.len());
        // The envelope the reserve pays for is what is left over.
        let envelope = frame.len() - 4 - encoded_frame_bytes(MAXIMUM_KV_PAYLOAD_BYTES);
        println!("largest chunk frame: {} envelope: {envelope}", frame.len());
        assert!(envelope <= FRAME_ENVELOPE_RESERVE_BYTES, "{envelope}");
        assert_eq!(decode_response(&frame).unwrap(), response);
    }

    /// A payload past the cap is refused whole. Nothing between a bound check
    /// and the socket shortens a payload to make it fit, so the failure mode
    /// of an over-large chunk is a rejected frame, never truncated bytes.
    #[test]
    fn a_payload_past_the_frame_cap_is_refused_rather_than_truncated() {
        let (store, path) = worst_case_store_and_path();
        let response = Response::success(
            u64::MAX,
            serde_json::to_value(KvChunkResult {
                store,
                path,
                version: u64::MAX,
                offset: u64::MAX,
                // Past what the ceiling admits once encoded, before the
                // envelope is even counted.
                content: vec![0x5a; 3 * (MAXIMUM_MESSAGE_BYTES / 4)],
                eof: true,
            })
            .unwrap(),
        );
        assert!(matches!(encode(&response), Err(Error::TooLarge)));
    }
}
