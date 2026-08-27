use foks_crypto::prefixed_hash;
use foks_proto::{
    EntityId, ENTITY_ID_MERKLE_VALUE_TYPE_ID, MERKLE_TREE_RF_INPUT_TYPE_ID,
    NAME_HASH_PREIMAGE_TYPE_ID,
};
use foks_snowpack::{encode, Value};

use crate::Result;

pub fn chain_key(
    chain_type: u64,
    entity: &EntityId,
    sequence: u64,
    location: Option<&[u8; 32]>,
) -> Result<[u8; 32]> {
    let value = Value::Array(vec![
        Value::Unsigned(chain_type),
        Value::Binary(entity.as_bytes().to_vec()),
        Value::Unsigned(sequence),
        location.map_or(Value::Null, |location| Value::Binary(location.to_vec())),
    ]);
    Ok(prefixed_hash(
        MERKLE_TREE_RF_INPUT_TYPE_ID,
        &encode(&value)?,
    ))
}

pub fn username_key(name: &[u8], host: &EntityId, sequence: u64) -> Result<[u8; 32]> {
    let preimage = encode(&Value::Array(vec![
        Value::Text(name.to_vec()),
        Value::Binary(host.as_bytes().to_vec()),
    ]))?;
    let mut name_entity = Vec::with_capacity(33);
    name_entity.push(9);
    name_entity.extend_from_slice(&prefixed_hash(NAME_HASH_PREIMAGE_TYPE_ID, &preimage));
    let input = Value::Array(vec![
        Value::Unsigned(1),
        Value::Binary(name_entity),
        Value::Unsigned(sequence),
        Value::Null,
    ]);
    Ok(prefixed_hash(
        MERKLE_TREE_RF_INPUT_TYPE_ID,
        &encode(&input)?,
    ))
}

pub fn username_leaf(entity: &EntityId) -> Result<[u8; 32]> {
    let value = Value::Array(vec![
        Value::Unsigned(1),
        Value::Variant(Some((
            b"1".to_vec(),
            Box::new(Value::Binary(entity.as_bytes().to_vec())),
        ))),
    ]);
    Ok(prefixed_hash(
        ENTITY_ID_MERKLE_VALUE_TYPE_ID,
        &encode(&value)?,
    ))
}
