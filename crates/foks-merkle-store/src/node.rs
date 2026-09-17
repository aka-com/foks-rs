use foks_proto::MERKLE_NODE_TYPE_ID;
use foks_snowpack::{decode, encode, Value};

use crate::{prefixed_hash_signable, Error, Result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Node {
    Leaf {
        key: [u8; 32],
        value: [u8; 32],
    },
    Interior {
        prefix_bit_start: u64,
        prefix_bit_count: u64,
        prefix: Vec<u8>,
        left: [u8; 32],
        right: [u8; 32],
    },
}

impl Node {
    pub fn encoded(&self) -> Result<Vec<u8>> {
        let (tag, payload) = match self {
            Self::Leaf { key, value } => (
                b"1".to_vec(),
                Value::Array(vec![
                    Value::Binary(key.to_vec()),
                    Value::Binary(value.to_vec()),
                ]),
            ),
            Self::Interior {
                prefix_bit_start,
                prefix_bit_count,
                prefix,
                left,
                right,
            } => (
                b"0".to_vec(),
                Value::Array(vec![
                    Value::Unsigned(*prefix_bit_start),
                    Value::Unsigned(*prefix_bit_count),
                    Value::Binary(prefix.clone()),
                    Value::Binary(left.to_vec()),
                    Value::Binary(right.to_vec()),
                ]),
            ),
        };
        let discriminant = if matches!(self, Self::Leaf { .. }) {
            0
        } else {
            1
        };
        Ok(encode(&Value::Array(vec![
            Value::Unsigned(discriminant),
            Value::Variant(Some((tag, Box::new(payload)))),
        ]))?)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let Value::Array(outer) = decode(bytes)? else {
            return Err(Error::InvalidNode);
        };
        if outer.len() != 2 {
            return Err(Error::InvalidNode);
        }
        let Value::Unsigned(discriminant) = outer[0] else {
            return Err(Error::InvalidNode);
        };
        let Value::Variant(Some((tag, payload))) = &outer[1] else {
            return Err(Error::InvalidNode);
        };
        let Value::Array(fields) = payload.as_ref() else {
            return Err(Error::InvalidNode);
        };
        match (discriminant, tag.as_slice(), fields.as_slice()) {
            (0, b"1", [Value::Binary(key), Value::Binary(value)]) => Ok(Self::Leaf {
                key: fixed_32(key)?,
                value: fixed_32(value)?,
            }),
            (
                1,
                b"0",
                [Value::Unsigned(start), Value::Unsigned(count), Value::Binary(prefix), Value::Binary(left), Value::Binary(right)],
            ) if valid_prefix(*start, *count, prefix) => Ok(Self::Interior {
                prefix_bit_start: *start,
                prefix_bit_count: *count,
                prefix: prefix.clone(),
                left: fixed_32(left)?,
                right: fixed_32(right)?,
            }),
            _ => Err(Error::InvalidNode),
        }
    }
}

fn valid_prefix(start: u64, count: u64, prefix: &[u8]) -> bool {
    let Some(end) = start.checked_add(count) else {
        return false;
    };
    if start >= 256 || count >= 256 || end >= 256 {
        return false;
    }
    if count == 0 {
        return prefix.is_empty();
    }
    let expected_length = ((end - 1) >> 3) - (start >> 3) + 1;
    if usize::try_from(expected_length).ok() != Some(prefix.len()) {
        return false;
    }
    let mut canonical = prefix.to_vec();
    canonical[0] &= 0xff >> (start as usize & 7);
    let last = canonical.len() - 1;
    canonical[last] &= 0xff << (7 - ((end - 1) as usize & 7));
    canonical == prefix
}

pub fn hash_node(node: &Node) -> Result<[u8; 32]> {
    prefixed_hash_signable(MERKLE_NODE_TYPE_ID, &node.encoded()?)
}

fn fixed_32(bytes: &[u8]) -> Result<[u8; 32]> {
    bytes.try_into().map_err(|_| Error::InvalidNode)
}
