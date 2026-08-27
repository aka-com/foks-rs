use std::collections::{BTreeMap, BTreeSet};

use crate::{hash_node, Commit, Error, LeafChange, Node, NodeReader, Result, EMPTY_ROOT};

pub fn prepare<R: NodeReader>(
    reader: &R,
    current_root: [u8; 32],
    changes: &[LeafChange],
) -> Result<Commit> {
    let mut leaves = BTreeMap::new();
    if current_root != EMPTY_ROOT {
        collect_leaves(reader, current_root, 0, &mut BTreeSet::new(), &mut leaves)?;
    }
    let mut changed_keys = BTreeSet::new();
    for change in changes {
        let key = match change {
            LeafChange::Set { key, .. } | LeafChange::Remove { key } => key,
        };
        if !changed_keys.insert(*key) {
            return Err(Error::DuplicateKey);
        }
        match change {
            LeafChange::Set { key, value } => {
                leaves.insert(*key, *value);
            }
            LeafChange::Remove { key } => {
                leaves.remove(key);
            }
        }
    }
    if leaves.is_empty() {
        return Ok(Commit {
            root: EMPTY_ROOT,
            leaf_count: 0,
            nodes: Vec::new(),
        });
    }

    let entries = leaves.into_iter().collect::<Vec<_>>();
    let mut nodes = BTreeMap::new();
    let root = build(&entries, 0, &mut nodes)?;
    let mut new_nodes = Vec::new();
    for (hash, encoded) in nodes {
        match reader.get_node(&hash)? {
            Some(existing) if existing == encoded => {}
            Some(_) => return Err(Error::ConflictingNode),
            None => new_nodes.push((hash, encoded)),
        }
    }
    Ok(Commit {
        root,
        leaf_count: entries.len(),
        nodes: new_nodes,
    })
}

fn collect_leaves<R: NodeReader>(
    reader: &R,
    hash: [u8; 32],
    expected_start: usize,
    visited: &mut BTreeSet<[u8; 32]>,
    leaves: &mut BTreeMap<[u8; 32], [u8; 32]>,
) -> Result<()> {
    if !visited.insert(hash) {
        return Err(Error::Cycle);
    }
    let encoded = reader.get_node(&hash)?.ok_or(Error::MissingNode(hash))?;
    let node = Node::decode(&encoded)?;
    if hash_node(&node)? != hash {
        return Err(Error::HashMismatch);
    }
    match node {
        Node::Leaf { key, value } => {
            if leaves.insert(key, value).is_some() {
                return Err(Error::DuplicateKey);
            }
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
            if start != expected_start {
                return Err(Error::InvalidNode);
            }
            let branch = start
                .checked_add(count)
                .filter(|bit| *bit < 256)
                .ok_or(Error::Depth)?;
            let mut left_leaves = BTreeMap::new();
            let mut right_leaves = BTreeMap::new();
            collect_leaves(reader, left, branch + 1, visited, &mut left_leaves)?;
            collect_leaves(reader, right, branch + 1, visited, &mut right_leaves)?;
            if left_leaves
                .keys()
                .any(|key| !prefix_matches(key, start, count, &prefix) || bit_at(key, branch))
                || right_leaves
                    .keys()
                    .any(|key| !prefix_matches(key, start, count, &prefix) || !bit_at(key, branch))
            {
                return Err(Error::InvalidNode);
            }
            for (key, value) in left_leaves.into_iter().chain(right_leaves) {
                if leaves.insert(key, value).is_some() {
                    return Err(Error::DuplicateKey);
                }
            }
        }
    }
    Ok(())
}

fn build(
    entries: &[([u8; 32], [u8; 32])],
    start: usize,
    nodes: &mut BTreeMap<[u8; 32], Vec<u8>>,
) -> Result<[u8; 32]> {
    if entries.len() == 1 {
        return insert_node(
            Node::Leaf {
                key: entries[0].0,
                value: entries[0].1,
            },
            nodes,
        );
    }
    let prefix_count = common_prefix(entries, start)?;
    let branch = start.checked_add(prefix_count).ok_or(Error::Depth)?;
    if branch >= 256 {
        return Err(Error::DuplicateKey);
    }
    let split = entries.partition_point(|(key, _)| !bit_at(key, branch));
    if split == 0 || split == entries.len() {
        return Err(Error::DuplicateKey);
    }
    let left = build(&entries[..split], branch + 1, nodes)?;
    let right = build(&entries[split..], branch + 1, nodes)?;
    insert_node(
        Node::Interior {
            prefix_bit_start: start as u64,
            prefix_bit_count: prefix_count as u64,
            prefix: copy_and_clamp(&entries[0].0, start, prefix_count)?,
            left,
            right,
        },
        nodes,
    )
}

fn insert_node(node: Node, nodes: &mut BTreeMap<[u8; 32], Vec<u8>>) -> Result<[u8; 32]> {
    let hash = hash_node(&node)?;
    nodes.insert(hash, node.encoded()?);
    Ok(hash)
}

fn common_prefix(entries: &[([u8; 32], [u8; 32])], start: usize) -> Result<usize> {
    for bit in start..256 {
        let expected = bit_at(&entries[0].0, bit);
        if entries
            .iter()
            .skip(1)
            .any(|(key, _)| bit_at(key, bit) != expected)
        {
            return Ok(bit - start);
        }
    }
    Err(Error::DuplicateKey)
}

pub(crate) fn bit_at(key: &[u8; 32], bit: usize) -> bool {
    key[bit >> 3] & (1 << (7 - (bit & 7))) != 0
}

pub(crate) fn copy_and_clamp(key: &[u8; 32], start: usize, count: usize) -> Result<Vec<u8>> {
    if count == 0 {
        return Ok(Vec::new());
    }
    let end_bit = start
        .checked_add(count)
        .and_then(|value| value.checked_sub(1))
        .filter(|value| *value < 256)
        .ok_or(Error::Depth)?;
    let mut output = key[start >> 3..=end_bit >> 3].to_vec();
    output[0] &= 0xff >> (start & 7);
    let last = output.len() - 1;
    output[last] &= 0xff << (7 - (end_bit & 7));
    Ok(output)
}

pub(crate) fn prefix_matches(key: &[u8; 32], start: usize, count: usize, prefix: &[u8]) -> bool {
    copy_and_clamp(key, start, count).is_ok_and(|candidate| candidate == prefix)
}
