//! Merkle query and history request encoding.

use crate::call::{encode_call, encode_select_vhost};
use crate::generated::{
    MERKLE_CHECK_KEY_EXISTS_METHOD_POSITION, MERKLE_GET_CURRENT_ROOT_HASH_METHOD_POSITION,
    MERKLE_GET_CURRENT_ROOT_METHOD_POSITION, MERKLE_GET_CURRENT_ROOT_SIGNED_METHOD_POSITION,
    MERKLE_GET_HISTORICAL_ROOTS_METHOD_POSITION, MERKLE_LOOKUP_METHOD_POSITION,
    MERKLE_MULTI_LOOKUP_METHOD_POSITION, MERKLE_QUERY_PROTOCOL_ID,
    MERKLE_SELECT_VHOST_METHOD_POSITION,
};
use crate::Result;
use foks_proto::EntityId;
use foks_snowpack::{encode, Value};

pub fn encode_get_current_merkle_root_request(host: &EntityId, sequence: u64) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![Value::Binary(host.as_bytes().to_vec())]))?;
    encode_call(
        MERKLE_QUERY_PROTOCOL_ID,
        MERKLE_GET_CURRENT_ROOT_METHOD_POSITION,
        &argument,
        sequence,
    )
}

pub fn encode_merkle_lookup_request(
    host: Option<&EntityId>,
    key: [u8; 32],
    signed: bool,
    root: Option<u64>,
    sequence: u64,
) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![
        optional_entity(host),
        Value::Binary(key.to_vec()),
        Value::Bool(signed),
        root.map_or(Value::Null, Value::Unsigned),
    ]))?;
    encode_call(
        MERKLE_QUERY_PROTOCOL_ID,
        MERKLE_LOOKUP_METHOD_POSITION,
        &argument,
        sequence,
    )
}

pub fn encode_merkle_multi_lookup_request(
    host: Option<&EntityId>,
    keys: &[[u8; 32]],
    signed: bool,
    root: Option<u64>,
    sequence: u64,
) -> Result<Vec<u8>> {
    let keys = if keys.is_empty() {
        Value::Null
    } else {
        Value::Array(keys.iter().map(|key| Value::Binary(key.to_vec())).collect())
    };
    let argument = encode(&Value::Array(vec![
        optional_entity(host),
        keys,
        Value::Bool(signed),
        root.map_or(Value::Null, Value::Unsigned),
    ]))?;
    encode_call(
        MERKLE_QUERY_PROTOCOL_ID,
        MERKLE_MULTI_LOOKUP_METHOD_POSITION,
        &argument,
        sequence,
    )
}

pub fn encode_get_current_merkle_root_hash_request(
    host: Option<&EntityId>,
    sequence: u64,
) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![optional_entity(host)]))?;
    encode_call(
        MERKLE_QUERY_PROTOCOL_ID,
        MERKLE_GET_CURRENT_ROOT_HASH_METHOD_POSITION,
        &argument,
        sequence,
    )
}

pub fn encode_merkle_check_key_exists_request(
    host: Option<&EntityId>,
    key: [u8; 32],
    sequence: u64,
) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![
        optional_entity(host),
        Value::Binary(key.to_vec()),
    ]))?;
    encode_call(
        MERKLE_QUERY_PROTOCOL_ID,
        MERKLE_CHECK_KEY_EXISTS_METHOD_POSITION,
        &argument,
        sequence,
    )
}

pub(super) fn optional_entity(entity: Option<&EntityId>) -> Value {
    entity.map_or(Value::Null, |entity| {
        Value::Binary(entity.as_bytes().to_vec())
    })
}

/// Requests the signed current Merkle root (getCurrentRootSigned @5). The v0.1.9
/// getCurrentRoot @2 route returns a bare unsigned root; the authenticated
/// advance path must use the signed form so it can verify the delegated
/// Merkle-signer signature before accepting the root.
pub fn encode_get_current_merkle_root_signed_request(
    host: &EntityId,
    sequence: u64,
) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![Value::Binary(host.as_bytes().to_vec())]))?;
    encode_call(
        MERKLE_QUERY_PROTOCOL_ID,
        MERKLE_GET_CURRENT_ROOT_SIGNED_METHOD_POSITION,
        &argument,
        sequence,
    )
}

pub fn encode_get_historical_merkle_roots_request(
    host: &EntityId,
    full_epochs: &[u64],
    hash_epochs: &[u64],
    sequence: u64,
) -> Result<Vec<u8>> {
    let list = |epochs: &[u64]| {
        if epochs.is_empty() {
            Value::Null
        } else {
            Value::Array(epochs.iter().copied().map(Value::Unsigned).collect())
        }
    };
    let argument = encode(&Value::Array(vec![
        Value::Binary(host.as_bytes().to_vec()),
        list(full_epochs),
        list(hash_epochs),
    ]))?;
    encode_call(
        MERKLE_QUERY_PROTOCOL_ID,
        MERKLE_GET_HISTORICAL_ROOTS_METHOD_POSITION,
        &argument,
        sequence,
    )
}

pub fn encode_merkle_select_vhost_request(host: &EntityId) -> Result<Vec<u8>> {
    encode_select_vhost(
        MERKLE_QUERY_PROTOCOL_ID,
        MERKLE_SELECT_VHOST_METHOD_POSITION,
        host,
    )
}
