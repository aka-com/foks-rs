use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::Zeroizing;

use crate::{KvUploadFrame, Request, Response, PROTOCOL_VERSION};

pub const MAXIMUM_MESSAGE_BYTES: usize = 1024 * 1024;

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
