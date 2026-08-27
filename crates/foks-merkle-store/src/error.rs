use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid canonical Snowpack node: {0}")]
    Snowpack(#[from] foks_snowpack::Error),
    #[error("invalid FOKS Merkle node")]
    InvalidNode,
    #[error("Merkle node {0:02x?} is missing")]
    MissingNode([u8; 32]),
    #[error("Merkle node content does not match its address")]
    HashMismatch,
    #[error("content-addressed node already exists with different bytes")]
    ConflictingNode,
    #[error("Merkle tree contains a cycle or duplicate node reference")]
    Cycle,
    #[error("Merkle key set contains an impossible duplicate or unsplittable key")]
    DuplicateKey,
    #[error("cannot generate a proof for an empty tree")]
    EmptyTree,
    #[error("Merkle tree exceeds the 256-bit key depth")]
    Depth,
    #[error("node storage failed: {0}")]
    Storage(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
