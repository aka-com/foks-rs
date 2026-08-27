use std::collections::BTreeSet;

use foks_proto::{MerklePathCompressed, MerkleTerminal};

use crate::{
    hash_node, tree::bit_at, tree::prefix_matches, Error, Node, NodeReader, Result, EMPTY_ROOT,
};

pub fn proof<R: NodeReader>(
    reader: &R,
    root: [u8; 32],
    query: [u8; 32],
) -> Result<MerklePathCompressed> {
    if root == EMPTY_ROOT {
        return Err(Error::EmptyTree);
    }
    let mut current = root;
    let mut bit = 0usize;
    let mut edges = Vec::new();
    let mut visited = BTreeSet::new();
    loop {
        if !visited.insert(current) {
            return Err(Error::Cycle);
        }
        let encoded = reader
            .get_node(&current)?
            .ok_or(Error::MissingNode(current))?;
        let node = Node::decode(&encoded)?;
        if hash_node(&node)? != current {
            return Err(Error::HashMismatch);
        }
        match node {
            Node::Leaf { key, value } => {
                if !(0..bit).all(|index| bit_at(&key, index) == bit_at(&query, index)) {
                    return Err(Error::InvalidNode);
                }
                return Ok(MerklePathCompressed {
                    edges,
                    terminal: MerkleTerminal::Leaf {
                        leaf: value,
                        found_key: (key != query).then_some(key),
                    },
                });
            }
            Node::Interior {
                prefix_bit_start,
                prefix_bit_count,
                prefix,
                left,
                right,
            } => {
                let start = usize::try_from(prefix_bit_start).map_err(|_| Error::Depth)?;
                let count = usize::try_from(prefix_bit_count).map_err(|_| Error::Depth)?;
                if start != bit || count > u8::MAX as usize {
                    return Err(Error::InvalidNode);
                }
                if !prefix_matches(&query, start, count, &prefix) {
                    return Ok(MerklePathCompressed {
                        edges,
                        terminal: MerkleTerminal::PrefixMiss {
                            prefix_bit_start,
                            prefix_bit_count,
                            prefix,
                            left,
                            right,
                        },
                    });
                }
                let branch = start.checked_add(count).ok_or(Error::Depth)?;
                if branch >= 256 {
                    return Err(Error::Depth);
                }
                let (next, sibling) = if bit_at(&query, branch) {
                    (right, left)
                } else {
                    (left, right)
                };
                let mut edge = [0; 33];
                edge[0] = count as u8;
                edge[1..].copy_from_slice(&sibling);
                edges.push(edge);
                bit = branch + 1;
                current = next;
            }
        }
    }
}
