//! Shared compressed-Merkle-path verification.

use crate::{
    encode, prefixed_hash, Error, MerklePathCompressed, MerkleTerminal, Result, Value,
    MERKLE_NODE_TYPE_ID,
};

pub fn verify_merkle_path(
    path: &MerklePathCompressed,
    query_key: &[u8; 32],
    expected_leaf: Option<&[u8; 32]>,
    expected_root_node: &[u8; 32],
) -> Result<()> {
    let expected = expected_leaf.map_or(ExpectedLeaf::Absent, ExpectedLeaf::Exact);
    verify_merkle_path_with(path, query_key, expected, expected_root_node)
}

pub fn verify_merkle_path_present(
    path: &MerklePathCompressed,
    query_key: &[u8; 32],
    expected_root_node: &[u8; 32],
) -> Result<()> {
    verify_merkle_path_with(path, query_key, ExpectedLeaf::Present, expected_root_node)
}

#[derive(Clone, Copy)]
enum ExpectedLeaf<'a> {
    Absent,
    Exact(&'a [u8; 32]),
    Present,
}

fn verify_merkle_path_with(
    path: &MerklePathCompressed,
    query_key: &[u8; 32],
    expected_leaf: ExpectedLeaf<'_>,
    expected_root_node: &[u8; 32],
) -> Result<()> {
    let mut bit_cursor = 0usize;
    let mut interiors = Vec::with_capacity(path.edges.len());
    for edge in &path.edges {
        let prefix_count = usize::from(edge[0]);
        let prefix =
            copy_and_clamp(query_key, bit_cursor, prefix_count).ok_or(Error::UserMerkleProof)?;
        bit_cursor = bit_cursor
            .checked_add(prefix_count)
            .ok_or(Error::UserMerkleProof)?;
        let branch = bit_at(query_key, bit_cursor).ok_or(Error::UserMerkleProof)?;
        bit_cursor += 1;
        let sibling: [u8; 32] = edge[1..].try_into().map_err(|_| Error::UserMerkleProof)?;
        interiors.push((
            bit_cursor - prefix_count - 1,
            prefix_count,
            prefix,
            branch,
            sibling,
        ));
    }
    let mut current = match &path.terminal {
        MerkleTerminal::Leaf { leaf, found_key } => {
            match expected_leaf {
                ExpectedLeaf::Exact(expected) if found_key.is_none() && leaf == expected => {}
                ExpectedLeaf::Present if found_key.is_none() => {}
                ExpectedLeaf::Absent if found_key.is_some_and(|found| found != *query_key) => {}
                _ => return Err(Error::UserMerkleProof),
            }
            if let Some(found) = found_key.as_ref() {
                if !bits_equal(query_key, found, bit_cursor) {
                    return Err(Error::UserMerkleProof);
                }
            }
            let leaf_key = found_key.as_ref().unwrap_or(query_key);
            let wire = Value::Array(vec![
                Value::Unsigned(0),
                Value::Variant(Some((
                    b"1".to_vec(),
                    Box::new(Value::Array(vec![
                        Value::Binary(leaf_key.to_vec()),
                        Value::Binary(leaf.to_vec()),
                    ])),
                ))),
            ]);
            prefixed_hash(MERKLE_NODE_TYPE_ID, &encode(&wire)?)?
        }
        MerkleTerminal::PrefixMiss {
            prefix_bit_start,
            prefix_bit_count,
            prefix,
            left,
            right,
        } => {
            let prefix_bit_count_usize =
                usize::try_from(*prefix_bit_count).map_err(|_| Error::UserMerkleProof)?;
            if !matches!(expected_leaf, ExpectedLeaf::Absent)
                || usize::try_from(*prefix_bit_start).ok() != Some(bit_cursor)
                || prefix_matches(query_key, prefix, bit_cursor, prefix_bit_count_usize)
            {
                return Err(Error::UserMerkleProof);
            }
            let wire = Value::Array(vec![
                Value::Unsigned(1),
                Value::Variant(Some((
                    b"0".to_vec(),
                    Box::new(Value::Array(vec![
                        Value::Unsigned(*prefix_bit_start),
                        Value::Unsigned(*prefix_bit_count),
                        Value::Binary(prefix.clone()),
                        Value::Binary(left.to_vec()),
                        Value::Binary(right.to_vec()),
                    ])),
                ))),
            ]);
            prefixed_hash(MERKLE_NODE_TYPE_ID, &encode(&wire)?)?
        }
    };
    for (start, count, prefix, branch, sibling) in interiors.into_iter().rev() {
        let start = u64::try_from(start).map_err(|_| Error::UserMerkleProof)?;
        let count = u64::try_from(count).map_err(|_| Error::UserMerkleProof)?;
        let (left, right) = if branch {
            (sibling, current)
        } else {
            (current, sibling)
        };
        let node = Value::Array(vec![
            Value::Unsigned(1),
            Value::Variant(Some((
                b"0".to_vec(),
                Box::new(Value::Array(vec![
                    Value::Unsigned(start),
                    Value::Unsigned(count),
                    Value::Binary(prefix),
                    Value::Binary(left.to_vec()),
                    Value::Binary(right.to_vec()),
                ])),
            ))),
        ]);
        current = prefixed_hash(MERKLE_NODE_TYPE_ID, &encode(&node)?)?;
    }
    if &current == expected_root_node {
        Ok(())
    } else {
        Err(Error::UserMerkleProof)
    }
}

fn prefix_matches(key: &[u8], prefix: &[u8], start: usize, count: usize) -> bool {
    if count == 0 {
        return true;
    }
    let start_byte = start >> 3;
    let end_bit = match start
        .checked_add(count)
        .and_then(|value| value.checked_sub(1))
    {
        Some(value) => value,
        None => return false,
    };
    let end_byte = end_bit >> 3;
    let Some(key_slice) = key.get(start_byte..=end_byte) else {
        return false;
    };
    if prefix.len() != key_slice.len() {
        return false;
    }
    key_slice
        .iter()
        .zip(prefix)
        .enumerate()
        .all(|(index, (left, right))| {
            let mut mask = 0xff;
            if index == 0 {
                mask >>= start & 7;
            }
            if index + 1 == key_slice.len() {
                mask &= 0xff << (7 - (end_bit & 7));
            }
            (left ^ right) & mask == 0
        })
}

fn bit_at(bytes: &[u8], bit: usize) -> Option<bool> {
    bytes
        .get(bit >> 3)
        .map(|byte| byte & (1 << (7 - (bit & 7))) != 0)
}

fn copy_and_clamp(bytes: &[u8], start: usize, count: usize) -> Option<Vec<u8>> {
    if count == 0 {
        return Some(Vec::new());
    }
    let start_byte = start >> 3;
    let end_bit = start.checked_add(count)?.checked_sub(1)?;
    let end_byte = end_bit >> 3;
    let mut output = bytes.get(start_byte..=end_byte)?.to_vec();
    output[0] &= 0xff >> (start & 7);
    let last = output.len() - 1;
    output[last] &= 0xff << (7 - (end_bit & 7));
    Some(output)
}

fn bits_equal(left: &[u8], right: &[u8], count: usize) -> bool {
    (0..count).all(|bit| bit_at(left, bit) == bit_at(right, bit))
}
