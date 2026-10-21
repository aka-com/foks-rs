use foks_proto::MERKLE_BACK_POINTERS_TYPE_ID;
use foks_snowpack::{encode, Value};
use std::collections::BTreeSet;

use crate::{prefixed_hash_signable, Result};

/// The first Merkle epoch that cannot be published under go-foks' canonical
/// encoding check.
///
/// Epoch 65,536 has 16 back pointers, more than a 15-element fixarray holds,
/// so the list is encoded as `array16`, which go-foks' signable validator
/// rejects for 16 to 31 elements. Every later multiple of 32,768 also has 16
/// or more back pointers; other later epochs (65,537 has one) could be hashed,
/// but roots are sequential and epoch 65,536 must be published first, so no
/// tree advances past 65,535.
pub const MAX_CANONICAL_MERKLE_EPOCH: u64 = 65_536;

pub fn back_pointer_sequence(epoch: u64) -> Vec<u64> {
    match epoch {
        0 | 1 => return Vec::new(),
        2 => return vec![1],
        3 => return vec![2, 1],
        4 => return vec![3, 2, 1],
        _ => {}
    }
    let mut distance = 1u64;
    let mut output = Vec::new();
    while epoch > distance {
        output.push(epoch - distance);
        if epoch & distance != 0 {
            break;
        }
        distance <<= 1;
    }
    output
}

pub fn back_pointer_hash(epoch: u64, pointers: &[(u64, [u8; 32])]) -> Result<[u8; 32]> {
    // Reject back-pointer arrays that fall within the non-canonical array16 range
    // (16..=31) before serialization.
    if (16..=31).contains(&pointers.len()) {
        return Err(crate::Error::EpochLimitExceeded {
            epoch,
            pointer_count: pointers.len(),
        });
    }
    let value = if pointers.is_empty() {
        Value::Null
    } else {
        Value::Array(
            pointers
                .iter()
                .map(|(epoch, hash)| {
                    Value::Array(vec![Value::Unsigned(*epoch), Value::Binary(hash.to_vec())])
                })
                .collect(),
        )
    };
    prefixed_hash_signable(MERKLE_BACK_POINTERS_TYPE_ID, &encode(&value)?)
}

/// Returns the v0.1.9 skip path and the root hashes needed alongside it.
pub fn collect_roots(mut start: u64, end: u64) -> (Vec<u64>, Vec<u64>) {
    let mut path = Vec::new();
    let mut roots = BTreeSet::new();
    let mut siblings = BTreeSet::new();
    while start > end {
        path.push(start);
        roots.insert(start);
        for current in back_pointer_sequence(start) {
            if current >= end {
                start = current;
            }
            if !roots.contains(&current) {
                siblings.insert(current);
            }
        }
    }
    let mut siblings = siblings.into_iter().collect::<Vec<_>>();
    siblings.sort_unstable_by(|left, right| right.cmp(left));
    (path, siblings)
}
