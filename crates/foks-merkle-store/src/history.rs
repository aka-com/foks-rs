use foks_proto::MERKLE_BACK_POINTERS_TYPE_ID;
use foks_snowpack::{encode, Value};
use std::collections::BTreeSet;

use crate::{prefixed_hash_signable, Result};

/// The first Merkle epoch that go-foks v0.1.9 cannot mint or verify.
///
/// Its skip list has 16 entries. go-codec encodes that list with `array16`,
/// while v0.1.9's signable validator rejects `array16` lengths that could have
/// used a fixarray. A v0.1.9-compatible tree therefore cannot advance past
/// epoch 65,535 without an upstream wire/canonicality fix.
pub const GO_V019_FIRST_UNMINTABLE_EPOCH: u64 = 65_536;

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
    // Do not attempt an alternate encoding here: that would produce a root
    // go-foks v0.1.9 cannot verify. Surface the compatibility ceiling as a
    // stable, actionable error before the lower-level Snowpack failure.
    if (16..=31).contains(&pointers.len()) {
        return Err(crate::Error::GoV019EpochCliff {
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
