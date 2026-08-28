//! Versioned, bounded local protocol between standalone FOKS frontends and agent.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAXIMUM_MESSAGE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Request {
    pub version: u32,
    pub id: u64,
    pub operation: Operation,
}

impl Request {
    pub fn new(id: u64, operation: Operation) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id,
            operation,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "operation", rename_all = "kebab-case")]
pub enum Operation {
    Ping,
    ListProfiles,
    Probe { profile: String },
    ListAccounts { profile: String },
    SyncAccount { profile: String, alias: String },
    ListKv { profile: String, alias: String },
    ListTeams { profile: String },
    SyncTeam { profile: String, team_alias: String },
    RunDueJobs { profile: String },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Response {
    pub version: u32,
    pub id: u64,
    #[serde(flatten)]
    pub result: ResponseResult,
}

impl Response {
    pub fn success(id: u64, value: serde_json::Value) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id,
            result: ResponseResult::Success { value },
        }
    }

    pub fn error(id: u64, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id,
            result: ResponseResult::Error {
                code,
                message: message.into(),
            },
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum ResponseResult {
    Success { value: serde_json::Value },
    Error { code: ErrorCode, message: String },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorCode {
    InvalidRequest,
    VersionMismatch,
    Busy,
    DeadlineExceeded,
    OperationFailed,
}

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

pub fn encode<T: Serialize>(message: &T) -> Result<Vec<u8>> {
    let payload = serde_json::to_vec(message)?;
    if payload.len() > MAXIMUM_MESSAGE_BYTES {
        return Err(Error::TooLarge);
    }
    let length = u32::try_from(payload.len()).map_err(|_| Error::TooLarge)?;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

pub fn decode_request(frame: &[u8]) -> Result<Request> {
    let request: Request = decode(frame)?;
    if request.version != PROTOCOL_VERSION {
        return Err(Error::Version);
    }
    Ok(request)
}

pub fn decode_response(frame: &[u8]) -> Result<Response> {
    let response: Response = decode(frame)?;
    if response.version != PROTOCOL_VERSION {
        return Err(Error::Version);
    }
    Ok(response)
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

    #[test]
    fn requests_and_responses_round_trip_with_version_binding() {
        let request = Request::new(
            7,
            Operation::SyncAccount {
                profile: "hosted".into(),
                alias: "personal".into(),
            },
        );
        assert_eq!(decode_request(&encode(&request).unwrap()).unwrap(), request);
        let response = Response::success(7, serde_json::json!({ "ok": true }));
        assert_eq!(
            decode_response(&encode(&response).unwrap()).unwrap(),
            response
        );
    }

    #[test]
    fn rejects_truncation_trailing_bytes_and_oversized_prefixes() {
        let request = encode(&Request::new(1, Operation::Ping)).unwrap();
        assert!(matches!(
            decode_request(&request[..request.len() - 1]),
            Err(Error::Length)
        ));
        let mut trailing = request.clone();
        trailing.push(0);
        assert!(matches!(decode_request(&trailing), Err(Error::Length)));
        assert!(matches!(
            decode_request(&(u32::MAX).to_be_bytes()),
            Err(Error::TooLarge)
        ));
    }

    #[test]
    fn unsupported_versions_fail_before_dispatch() {
        let request = Request {
            version: PROTOCOL_VERSION + 1,
            id: 1,
            operation: Operation::Ping,
        };
        assert!(matches!(
            decode_request(&encode(&request).unwrap()),
            Err(Error::Version)
        ));
    }
}
