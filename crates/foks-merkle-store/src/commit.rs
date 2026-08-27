use crate::{NodeWriter, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LeafChange {
    Set { key: [u8; 32], value: [u8; 32] },
    Remove { key: [u8; 32] },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Commit {
    pub root: [u8; 32],
    pub leaf_count: usize,
    pub nodes: Vec<([u8; 32], Vec<u8>)>,
}

pub fn apply_commit<W: NodeWriter>(writer: &mut W, commit: &Commit) -> Result<()> {
    for (hash, encoded) in &commit.nodes {
        writer.put_node(*hash, encoded.clone())?;
    }
    Ok(())
}
