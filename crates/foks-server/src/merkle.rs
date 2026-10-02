//! Merkle publication preparation shared by application mutations.
//!
//! Callers retain authorization, clock policy and the atomic database commit in
//! their writer job. Preparing a publication does not persist its tree or root.

use foks_proto::{HostchainTail, MerkleRoot, SignedBlob};
use foks_server_db::{Database, RootSnapshot};

use crate::keys::{HostKeyProvider, KeyPurpose};
use crate::{Error, Result};

pub(crate) struct PreparedPublication {
    pub merkle_commit: foks_merkle_store::Commit,
    pub root_epoch: u64,
    pub root_hash: [u8; 32],
    pub exact_root: Vec<u8>,
    pub exact_signed_root: Vec<u8>,
    pub back_pointers: Vec<(u64, [u8; 32])>,
}

pub(crate) fn prepare_publication(
    database: &Database,
    authoritative: &RootSnapshot,
    leaves: &[([u8; 32], [u8; 32])],
    hostchain_tail: HostchainTail,
    now: u64,
    keys: &dyn HostKeyProvider,
) -> Result<PreparedPublication> {
    let changes = leaves
        .iter()
        .map(|(key, value)| foks_merkle_store::LeafChange::Set {
            key: *key,
            value: *value,
        })
        .collect::<Vec<_>>();
    let merkle_commit =
        foks_merkle_store::prepare(&database.node_reader(), authoritative.root_node, &changes)?;
    let root_epoch = authoritative
        .epoch
        .checked_add(1)
        .ok_or(Error::Signup("Merkle epoch overflow"))?;
    let pointer_epochs = foks_merkle_store::back_pointer_sequence(root_epoch);
    let pointer_roots = database
        .roots_at(&pointer_epochs)?
        .ok_or(Error::Database(foks_server_db::Error::StaleRoot))?;
    for root in &pointer_roots {
        decode_stored_root(root)?;
    }
    let back_pointers = pointer_roots
        .into_iter()
        .map(|root| (root.epoch, root.root_hash))
        .collect::<Vec<_>>();
    let root = MerkleRoot {
        epoch: root_epoch,
        time: now / 1_000,
        back_pointers: foks_merkle_store::back_pointer_hash(root_epoch, &back_pointers)?,
        root_node: merkle_commit.root,
        hostchain: hostchain_tail,
        extensions: Vec::new(),
    };
    let exact_root = root.encoded()?;
    let root_hash =
        foks_crypto::prefixed_hash_signable(foks_proto::MERKLE_ROOT_TYPE_ID, &exact_root)?;
    let merkle_key = keys.load_or_create(KeyPurpose::Merkle)?;
    let exact_signed_root = SignedBlob {
        inner: exact_root.clone(),
        signature: foks_crypto::sign_ed25519_blob(
            merkle_key.expose(),
            foks_proto::MERKLE_ROOT_BLOB_TYPE_ID,
            &exact_root,
        )?,
    }
    .encoded()?;
    Ok(PreparedPublication {
        merkle_commit,
        root_epoch,
        root_hash,
        exact_root,
        exact_signed_root,
        back_pointers,
    })
}

pub(crate) fn decode_stored_root(root: &RootSnapshot) -> Result<MerkleRoot> {
    let decoded = MerkleRoot::decode(&root.exact_root)?;
    let hash =
        foks_crypto::prefixed_hash_signable(foks_proto::MERKLE_ROOT_TYPE_ID, &root.exact_root)?;
    if decoded.epoch != root.epoch || decoded.root_node != root.root_node || hash != root.root_hash
    {
        return Err(Error::Signup("stored Merkle root binding mismatch"));
    }
    Ok(decoded)
}
