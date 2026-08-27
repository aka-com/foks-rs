use std::collections::BTreeMap;

use crate::{NodeReader, NodeWriter, Result};

#[derive(Clone, Debug, Default)]
pub struct MemoryStore {
    nodes: BTreeMap<[u8; 32], Vec<u8>>,
}

impl MemoryStore {
    pub fn corrupt(&mut self, hash: &[u8; 32], encoded: Vec<u8>) -> bool {
        self.nodes.insert(*hash, encoded).is_some()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

impl NodeReader for MemoryStore {
    fn get_node(&self, hash: &[u8; 32]) -> Result<Option<Vec<u8>>> {
        Ok(self.nodes.get(hash).cloned())
    }
}

impl NodeWriter for MemoryStore {
    fn put_node(&mut self, hash: [u8; 32], encoded: Vec<u8>) -> Result<()> {
        match self.nodes.get(&hash) {
            Some(existing) if existing != &encoded => Err(crate::Error::ConflictingNode),
            Some(_) => Ok(()),
            None => {
                self.nodes.insert(hash, encoded);
                Ok(())
            }
        }
    }
}
