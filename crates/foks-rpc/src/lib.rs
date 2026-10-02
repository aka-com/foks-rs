//! Strict framing and request encoding for FOKS v0.1.9 client RPCs.
//!
//! Snowpack RPC uses a standard MessagePack envelope (including named maps)
//! around canonical Snowpack protocol values. This crate keeps the envelope
//! separate from exact public and authenticated protocol payloads and returns
//! response bytes unchanged for protocol verification.

#![forbid(unsafe_code)]

use foks_proto::KvPathVersionVector;
use foks_snowpack::{decode, Value};
use thiserror::Error;

mod invitations;
pub use invitations::*;
mod account;
pub mod arguments;
mod generated;
mod realtime;
mod sso;
pub use account::*;
mod status;
pub use sso::*;
#[cfg(test)]
use status::check_status;
#[cfg(test)]
mod realtime_status_tests;
mod response;
pub use realtime::*;
mod server;

pub use generated::*;
pub use response::{encode_status_response_at, encode_void_success_response_at, RpcStatus};
pub use server::{
    decode_call, decode_message, read_call, read_message, DecodedCall, InboundMessage,
};

pub const CURRENT_COMPATIBILITY_VERSION: u64 = 1;
pub const DEFAULT_MAX_FRAME_LENGTH: usize = 16 * 1024 * 1024;

const METHOD_CALL_V2: u64 = 5;
const METHOD_RESPONSE: u64 = 1;
const RESPONSE_HEADER: &[u8] = &[
    0x82, 0xa1, b'V', 0x01, 0xa2, b'f', b'1', 0x81, 0xa4, b'V', b'e', b'r', b's', 0x01,
];

#[derive(Debug, Error)]
pub enum Error {
    #[error("RPC I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid canonical Snowpack in RPC payload: {0}")]
    Snowpack(#[from] foks_snowpack::Error),
    #[error(
        "invalid canonical Snowpack argument for protocol {protocol_id:#010x} method {method_position}: {source}"
    )]
    ArgumentSnowpack {
        protocol_id: u64,
        method_position: u64,
        source: foks_snowpack::Error,
    },
    #[error("invalid FOKS protocol value: {0}")]
    Protocol(#[from] foks_proto::Error),
    #[error("RPC frame length {received} exceeds limit {maximum}")]
    FrameTooLarge { received: usize, maximum: usize },
    #[error("invalid or noncanonical RPC frame length marker {0:#04x}")]
    FrameLengthMarker(u8),
    #[error("truncated RPC MessagePack envelope")]
    Truncated,
    #[error("RPC MessagePack nesting exceeds the limit")]
    Depth,
    #[error("unsupported RPC MessagePack marker {0:#04x}")]
    Marker(u8),
    #[error("expected {expected}, found {found}")]
    Envelope {
        expected: &'static str,
        found: &'static str,
    },
    #[error("RPC response sequence is {received}, expected {expected}")]
    Sequence { expected: u64, received: u64 },
    #[error("{}", describe_status(*code, detail))]
    RemoteStatus { code: u64, detail: StatusDetail },
    #[error("FOKS server does not implement protocol {protocol_id:#x} method {position}")]
    MethodNotFound { protocol_id: u64, position: u64 },
    #[error("FOKS KV cache is stale")]
    KvStaleCache(KvPathVersionVector),
    #[error("RPC {kind} count {received} exceeds limit {maximum}")]
    CollectionTooLarge {
        kind: &'static str,
        received: usize,
        maximum: usize,
    },
    #[error("unsupported FOKS compatibility header")]
    Compatibility,
    #[error("invalid probe hostname")]
    Hostname,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StatusDetail(Option<String>);

/// A plain phrase for the FOKS status codes a person can provoke, so the
/// message names the refusal rather than only its number. Codes without a
/// phrase fall back to the number alone.
pub fn status_phrase(code: u64) -> Option<&'static str> {
    Some(match code {
        STATUS_USERNAME_IN_USE_ERROR => "that name is already in use on the server",
        STATUS_DUPLICATE_ERROR => "the server already has this record",
        STATUS_PERMISSION_ERROR => "the server refused permission",
        STATUS_BAD_ARGS_ERROR => "the server rejected the request as malformed",
        STATUS_BAD_INVITE_CODE_ERROR => "the invitation code is not valid",
        STATUS_BAD_PASSPHRASE_ERROR => "the passphrase is not correct",
        STATUS_PASSPHRASE_NOT_FOUND_ERROR => "no passphrase is set for this account",
        STATUS_USER_NOT_FOUND_ERROR => "no such user on the server",
        STATUS_WRONG_USER_ERROR => "the credentials belong to a different user",
        STATUS_KEY_NOT_FOUND_ERROR => "the server does not know this key",
        STATUS_DEVICE_ALREADY_PROVISIONED_ERROR => "this device is already set up on the account",
        STATUS_KEX_BAD_SECRET => "the pairing phrase did not match",
        STATUS_EXPIRED_ERROR => "the request has expired",
        STATUS_RATE_LIMIT_ERROR => "the server is rate limiting requests; try again shortly",
        STATUS_OVER_QUOTA_ERROR => "the account is over its storage quota",
        STATUS_TIMEOUT_ERROR => "the server timed out",
        STATUS_NOT_IMPLEMENTED => "the server does not support this operation",
        STATUS_GENERIC_NOT_FOUND_ERROR => "the server has no such record",
        STATUS_TEAM_NOT_FOUND_ERROR => "the server has no such team",
        STATUS_TEAM_INVITE_ALREADY_ACCEPTED_ERROR => "this invitation was already accepted",
        STATUS_TEAM_ADHOC_DUPLICATE_ERROR => "an identical share already exists",
        STATUS_KV_PERM_ERROR => "you do not have permission for this item",
        STATUS_KV_NOENT_ERROR => "the item no longer exists on the server",
        STATUS_KV_LOCK_TIMEOUT_ERROR | STATUS_KV_LOCK_ALREADY_HELD_ERROR => {
            "another change to this item is in progress; try again"
        }
        STATUS_RT_CHANNEL_EXISTS_ERROR => "a channel with this name already exists",
        STATUS_RT_NOT_FOUND_ERROR => "the server has no such channel or message",
        _ => return None,
    })
}

fn describe_status(code: u64, detail: &StatusDetail) -> String {
    match status_phrase(code) {
        Some(phrase) => format!("FOKS server refused: {phrase} (status {code}){detail}"),
        None => format!("FOKS server returned status {code}{detail}"),
    }
}

impl StatusDetail {
    pub fn detail(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

impl std::fmt::Display for StatusDetail {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(detail) = &self.0 {
            write!(formatter, ": {detail}")
        } else {
            Ok(())
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

mod kv;
pub use kv::*;
mod server_info;
pub use server_info::*;
mod call;
pub use call::*;
mod identity;
pub use identity::*;
mod team;
pub use team::*;
mod client_response;
pub use client_response::*;
mod merkle;
pub use merkle::*;
mod codec;
pub use codec::*;
mod go_compat;
use go_compat::*;

#[cfg(test)]
mod tests;
