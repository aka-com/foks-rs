use foks_proto::{EntityId, ENTITY_HOST};
use foks_snowpack::{decode, Value};

use crate::{Error, Result};

const MAXIMUM_MULTI_LOOKUP_KEYS: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MerkleLookupArgument {
    pub host: Option<EntityId>,
    pub key: [u8; 32],
    pub signed: bool,
    pub root: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MerkleMultiLookupArgument {
    pub host: Option<EntityId>,
    pub keys: Vec<[u8; 32]>,
    pub signed: bool,
    pub root: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MerkleCheckKeyArgument {
    pub host: Option<EntityId>,
    pub key: [u8; 32],
}

pub fn decode_merkle_lookup(bytes: &[u8]) -> Result<MerkleLookupArgument> {
    let fields = fields(bytes, 4, "Merkle lookup argument")?;
    let Value::Bool(signed) = fields[2] else {
        return Err(shape("Merkle lookup signed flag"));
    };
    Ok(MerkleLookupArgument {
        host: optional_host(&fields[0])?,
        key: fixed_key(&fields[1])?,
        signed,
        root: optional_epoch(&fields[3])?,
    })
}

pub fn decode_merkle_multi_lookup(bytes: &[u8]) -> Result<MerkleMultiLookupArgument> {
    let fields = fields(bytes, 4, "Merkle multi-lookup argument")?;
    let Value::Bool(signed) = fields[2] else {
        return Err(shape("Merkle multi-lookup signed flag"));
    };
    let keys = match &fields[1] {
        Value::Null => Vec::new(),
        Value::Array(values) if values.len() <= MAXIMUM_MULTI_LOOKUP_KEYS => {
            values.iter().map(fixed_key).collect::<Result<Vec<_>>>()?
        }
        Value::Array(_) => return Err(shape("bounded Merkle multi-lookup keys")),
        _ => return Err(shape("Merkle multi-lookup keys")),
    };
    Ok(MerkleMultiLookupArgument {
        host: optional_host(&fields[0])?,
        keys,
        signed,
        root: optional_epoch(&fields[3])?,
    })
}

pub fn decode_merkle_check_key(bytes: &[u8]) -> Result<MerkleCheckKeyArgument> {
    let fields = fields(bytes, 2, "Merkle check-key argument")?;
    Ok(MerkleCheckKeyArgument {
        host: optional_host(&fields[0])?,
        key: fixed_key(&fields[1])?,
    })
}

pub fn decode_merkle_optional_host(bytes: &[u8]) -> Result<Option<EntityId>> {
    let fields = fields(bytes, 1, "optional Merkle host argument")?;
    optional_host(&fields[0])
}

fn fields(bytes: &[u8], expected: usize, kind: &'static str) -> Result<Vec<Value>> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape(kind));
    };
    if fields.len() != expected {
        return Err(shape(kind));
    }
    Ok(fields)
}

fn optional_host(value: &Value) -> Result<Option<EntityId>> {
    match value {
        Value::Null => Ok(None),
        Value::Binary(bytes) => Ok(Some(
            EntityId::from_bytes(bytes.clone())?.require_type(ENTITY_HOST)?,
        )),
        _ => Err(shape("optional Merkle host")),
    }
}

fn fixed_key(value: &Value) -> Result<[u8; 32]> {
    let Value::Binary(bytes) = value else {
        return Err(shape("32-byte Merkle key"));
    };
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| shape("32-byte Merkle key"))
}

fn optional_epoch(value: &Value) -> Result<Option<u64>> {
    match value {
        Value::Null => Ok(None),
        Value::Unsigned(epoch) => Ok(Some(*epoch)),
        _ => Err(shape("optional Merkle epoch")),
    }
}

fn shape(expected: &'static str) -> Error {
    Error::Envelope {
        expected,
        found: "another Snowpack value",
    }
}

#[cfg(test)]
mod tests {
    use foks_snowpack::encode;

    use super::*;

    #[test]
    fn lookup_arguments_are_strict_and_bounded() {
        let host = [vec![ENTITY_HOST], vec![1; 32]].concat();
        let single = encode(&Value::Array(vec![
            Value::Binary(host.clone()),
            Value::Binary(vec![2; 32]),
            Value::Bool(true),
            Value::Unsigned(7),
        ]))
        .unwrap();
        let decoded = decode_merkle_lookup(&single).unwrap();
        assert_eq!(decoded.key, [2; 32]);
        assert_eq!(decoded.root, Some(7));
        assert!(decoded.signed);

        let excessive = encode(&Value::Array(vec![
            Value::Binary(host),
            Value::Array(vec![Value::Binary(vec![3; 32]); 4097]),
            Value::Bool(false),
            Value::Null,
        ]))
        .unwrap();
        assert!(decode_merkle_multi_lookup(&excessive).is_err());
    }
}
