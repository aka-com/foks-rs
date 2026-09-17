//! Deterministic, storage-neutral FOKS v0.1.9 Merkle construction.

#![forbid(unsafe_code)]

mod commit;
mod error;
mod history;
mod key;
mod memory;
mod node;
mod proof;
mod store;
mod tree;

pub use commit::{apply_commit, Commit, LeafChange};
pub use error::{Error, Result};
pub use history::{
    back_pointer_hash, back_pointer_sequence, collect_roots, GO_V019_FIRST_UNMINTABLE_EPOCH,
};
pub use key::{chain_key, username_key, username_leaf};
pub use memory::MemoryStore;
pub use node::{hash_node, Node};
pub use proof::proof;
pub use store::{NodeReader, NodeWriter};
pub use tree::prepare;

pub const EMPTY_ROOT: [u8; 32] = [0; 32];

fn prefixed_hash_signable(type_id: u64, canonical_object: &[u8]) -> Result<[u8; 32]> {
    foks_snowpack::validate_signable(canonical_object)?;
    Ok(foks_crypto::prefixed_hash(type_id, canonical_object))
}
