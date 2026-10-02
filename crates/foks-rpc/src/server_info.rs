//! Probe, server information, and diagnostic request encoding.

use crate::call::{encode_call, encode_call_with_validated_argument};
use crate::generated::{
    LOG_SEND_INIT_FILE_METHOD_POSITION, LOG_SEND_INIT_METHOD_POSITION, LOG_SEND_PROTOCOL_ID,
    LOG_SEND_UPLOAD_BLOCK_METHOD_POSITION, PROBE_METHOD_POSITION, PROBE_PROTOCOL_ID,
    REG_GET_CLIENT_VERSION_INFO_METHOD_POSITION, REG_JOIN_WAIT_LIST_METHOD_POSITION,
    REG_PROTOCOL_ID, USER_GET_HOST_CONFIG_METHOD_POSITION, USER_PROTOCOL_ID,
};
use crate::{arguments, Error, Result};
use foks_proto::ClientVersionExt;
use foks_snowpack::{decode, encode, Value};
use std::io::Write;

/// Encodes one complete, length-prefixed v0.1.9 `Probe.probe` call.
pub fn encode_probe_request(
    hostname: &str,
    hostchain_last_seqno: u64,
    host_id: Option<&[u8]>,
) -> Result<Vec<u8>> {
    if hostname.is_empty() || !hostname.is_ascii() || hostname.bytes().any(|byte| byte == 0) {
        return Err(Error::Hostname);
    }

    let probe_arg = encode(&Value::Array(vec![
        Value::Text(hostname.as_bytes().to_vec()),
        Value::Unsigned(hostchain_last_seqno),
        host_id.map_or(Value::Null, |bytes| Value::Binary(bytes.to_vec())),
    ]))?;

    encode_call(PROBE_PROTOCOL_ID, PROBE_METHOD_POSITION, &probe_arg, 0)
}

pub fn encode_join_waitlist_request_at(email: &[u8], sequence: u64) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_JOIN_WAIT_LIST_METHOD_POSITION,
        &encode(&Value::Array(vec![Value::Text(email.to_vec())]))?,
        sequence,
    )
}

pub fn encode_log_send_init_request_at(sequence: u64) -> Result<Vec<u8>> {
    encode_call_with_validated_argument(
        LOG_SEND_PROTOCOL_ID,
        LOG_SEND_INIT_METHOD_POSITION,
        &[0x90],
        sequence,
    )
}

pub fn encode_log_send_init_file_request_at(
    argument: &arguments::LogSendInitFileArgument,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_call(
        LOG_SEND_PROTOCOL_ID,
        LOG_SEND_INIT_FILE_METHOD_POSITION,
        &encode(&Value::Array(vec![
            Value::Binary(argument.id.to_vec()),
            Value::Unsigned(argument.file_id),
            Value::Text(argument.filename.clone()),
            Value::Unsigned(argument.content_length),
            Value::Binary(argument.content_hash.to_vec()),
            Value::Unsigned(argument.block_count),
        ]))?,
        sequence,
    )
}

pub fn encode_log_send_upload_block_request_at(
    argument: &arguments::LogSendUploadBlockArgument,
    sequence: u64,
) -> Result<Vec<u8>> {
    if argument.block.len() > arguments::MAXIMUM_LOG_SEND_BLOCK_BYTES {
        return Err(Error::CollectionTooLarge {
            kind: "log-send block byte",
            received: argument.block.len(),
            maximum: arguments::MAXIMUM_LOG_SEND_BLOCK_BYTES,
        });
    }
    encode_call(
        LOG_SEND_PROTOCOL_ID,
        LOG_SEND_UPLOAD_BLOCK_METHOD_POSITION,
        &encode(&Value::Array(vec![
            Value::Binary(argument.id.to_vec()),
            Value::Unsigned(argument.file_id),
            Value::Unsigned(argument.block_number),
            Value::Binary(argument.block.clone()),
        ]))?,
        sequence,
    )
}

pub fn encode_get_client_version_info_request(version: &ClientVersionExt) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![decode(&version.encoded()?)?]))?;
    encode_call(
        REG_PROTOCOL_ID,
        REG_GET_CLIENT_VERSION_INFO_METHOD_POSITION,
        &argument,
        0,
    )
}

pub fn encode_get_host_config_request() -> Result<Vec<u8>> {
    // A generated zero-field Snowpack struct is the one sanctioned empty
    // array in the v0.1.9 RPC schema. It is not a general Snowpack Value.
    encode_call_with_validated_argument(
        USER_PROTOCOL_ID,
        USER_GET_HOST_CONFIG_METHOD_POSITION,
        &[0x90],
        0,
    )
}

/// Writes one probe call and flushes it before waiting for the response.
pub fn write_probe_request<W: Write>(
    writer: &mut W,
    hostname: &str,
    hostchain_last_seqno: u64,
    host_id: Option<&[u8]>,
) -> Result<()> {
    writer.write_all(&encode_probe_request(
        hostname,
        hostchain_last_seqno,
        host_id,
    )?)?;
    writer.flush()?;
    Ok(())
}
