use foks_proto::KvPathVersionVector;
use foks_snowpack::{encode, Value};

use super::{
    encode_text, encode_unsigned, frame, Error, Result, DEFAULT_MAX_FRAME_LENGTH, METHOD_RESPONSE,
    RESPONSE_HEADER,
};

/// Application errors that a v1 server may place on the v0.1.9 wire.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RpcStatus {
    BadArguments(String),
    Locked,
    NameInUse,
    NotFound(String),
    PermissionDenied(String),
    QuotaExceeded,
    RateLimited,
    StaleCache(KvPathVersionVector),
    StaleRoot,
    TransactionRetry,
    Unsupported,
}

impl RpcStatus {
    fn code(&self) -> u64 {
        match self {
            Self::BadArguments(_) => 1030,
            Self::Locked => 8014,
            Self::NameInUse => 1023,
            Self::NotFound(_) => 1049,
            Self::PermissionDenied(_) => 1013,
            Self::QuotaExceeded => 1060,
            Self::RateLimited => 1012,
            Self::StaleCache(_) => 8012,
            Self::StaleRoot | Self::TransactionRetry => 1014,
            Self::Unsupported => 1020,
        }
    }
}

/// Encodes a successful response for a method whose result is `Void`.
pub fn encode_void_success_response_at(sequence: u64) -> Result<Vec<u8>> {
    let mut content = Vec::new();
    content.push(0x94);
    encode_unsigned(METHOD_RESPONSE, &mut content);
    encode_unsigned(sequence, &mut content);
    content.push(0xc0);
    content.push(0x81);
    encode_text(b"Header", &mut content);
    content.extend_from_slice(RESPONSE_HEADER);
    frame(&content, DEFAULT_MAX_FRAME_LENGTH)
}

/// Encodes a typed v0.1.9 application error response.
pub fn encode_status_response_at(status: &RpcStatus, sequence: u64) -> Result<Vec<u8>> {
    let mut content = Vec::new();
    content.push(0x94);
    encode_unsigned(METHOD_RESPONSE, &mut content);
    encode_unsigned(sequence, &mut content);
    encode_status(status, &mut content)?;
    // Go omits Data on an error and retains only the compatibility header.
    content.push(0x81);
    encode_text(b"Header", &mut content);
    content.extend_from_slice(RESPONSE_HEADER);
    frame(&content, DEFAULT_MAX_FRAME_LENGTH)
}

fn encode_status(status: &RpcStatus, output: &mut Vec<u8>) -> Result<()> {
    let has_payload = matches!(
        status,
        RpcStatus::BadArguments(_)
            | RpcStatus::NotFound(_)
            | RpcStatus::PermissionDenied(_)
            | RpcStatus::StaleCache(_)
    );
    output.push(if has_payload { 0x82 } else { 0x81 });
    encode_text(b"Sc", output);
    encode_unsigned(status.code(), output);
    match status {
        RpcStatus::BadArguments(message)
        | RpcStatus::NotFound(message)
        | RpcStatus::PermissionDenied(message) => {
            encode_text(b"f1", output);
            encode_text(message.as_bytes(), output);
        }
        RpcStatus::StaleCache(versions) => {
            encode_text(b"f11", output);
            encode_named_path_version_vector(versions, output)?;
        }
        _ => {}
    }
    Ok(())
}

fn encode_named_path_version_vector(
    versions: &KvPathVersionVector,
    output: &mut Vec<u8>,
) -> Result<()> {
    output.push(0x82);
    encode_text(b"Path", output);
    encode_optional_array_len(versions.directories.len(), output)?;
    for directory in &versions.directories {
        output.push(0x83);
        encode_text(b"De", output);
        encode_optional_array_len(directory.entries.len(), output)?;
        for entry in &directory.entries {
            output.push(0x82);
            encode_text(b"Id", output);
            output.extend_from_slice(&encode(&Value::Binary(entry.id.to_vec()))?);
            encode_text(b"Vers", output);
            encode_unsigned(entry.version, output);
        }
        encode_text(b"Id", output);
        output.extend_from_slice(&encode(&Value::Binary(directory.id.to_vec()))?);
        encode_text(b"Vers", output);
        encode_unsigned(directory.version, output);
    }
    encode_text(b"Root", output);
    encode_unsigned(versions.root_version, output);
    Ok(())
}

fn encode_optional_array_len(length: usize, output: &mut Vec<u8>) -> Result<()> {
    if length == 0 {
        output.push(0xc0);
        Ok(())
    } else {
        encode_array_len(length, output)
    }
}

fn encode_array_len(length: usize, output: &mut Vec<u8>) -> Result<()> {
    if length <= 15 {
        output.push(0x90 | length as u8);
    } else if let Ok(length) = u16::try_from(length) {
        output.push(0xdc);
        output.extend_from_slice(&length.to_be_bytes());
    } else {
        let length = u32::try_from(length).map_err(|_| Error::FrameTooLarge {
            received: length,
            maximum: u32::MAX as usize,
        })?;
        output.push(0xdd);
        output.extend_from_slice(&length.to_be_bytes());
    }
    Ok(())
}
