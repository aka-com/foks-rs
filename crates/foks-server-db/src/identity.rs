use rusqlite::{params, OptionalExtension as _, TransactionBehavior};

use crate::{error::sql_integer, receipts, transaction::inject, Database, Error, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailurePoint {
    Name,
    User,
    Device,
    Chain,
    MerkleNodes,
    MerkleRoot,
    Receipt,
}

pub struct IdentityMutation<'a> {
    pub normalized_name: &'a [u8],
    pub reservation_token: &'a [u8; 17],
    pub reservation_sequence: u64,
    pub reservation_expires_at: u64,
    pub username_utf8: &'a [u8],
    pub username_commitment_key: &'a [u8; 16],
    pub uid: &'a [u8],
    pub device_id: &'a [u8],
    pub device_hepk_fingerprint: &'a [u8; 32],
    pub exact_device_hepk: &'a [u8],
    pub exact_device_name: &'a [u8],
    pub link_hash: &'a [u8; 32],
    pub exact_link: &'a [u8],
    pub tree_location: &'a [u8; 32],
    pub shared_role_type: u64,
    pub shared_visibility: i64,
    pub shared_generation: u64,
    pub shared_verify_key: &'a [u8],
    pub exact_shared_hepk: &'a [u8],
    pub exact_parcel: &'a [u8],
    pub expected_root_hash: Option<[u8; 32]>,
    pub merkle_commit: &'a foks_merkle_store::Commit,
    pub merkle_leaves: &'a [([u8; 32], [u8; 32])],
    pub root_epoch: u64,
    pub root_hash: &'a [u8; 32],
    pub exact_root: &'a [u8],
    pub exact_signed_root: &'a [u8],
    pub back_pointers: &'a [(u64, [u8; 32])],
    pub idempotency_key: &'a [u8],
    pub request_hash: &'a [u8; 32],
    pub response: &'a [u8],
    pub now: u64,
    pub receipt_expires_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommitOutcome {
    Committed(Vec<u8>),
    Replayed(Vec<u8>),
}

impl Database {
    pub fn commit_identity(&mut self, mutation: &IdentityMutation<'_>) -> Result<CommitOutcome> {
        self.commit_identity_with_failure(mutation, None)
    }

    #[doc(hidden)]
    pub fn commit_identity_with_failure(
        &mut self,
        mutation: &IdentityMutation<'_>,
        failure: Option<FailurePoint>,
    ) -> Result<CommitOutcome> {
        validate(self, mutation)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(receipt) = receipts::lookup(
            &transaction,
            mutation.idempotency_key,
            mutation.request_hash,
            mutation.now,
        )? {
            return Ok(CommitOutcome::Replayed(receipt.response));
        }
        let reservation: Option<(Vec<u8>, i64, i64)> = transaction
            .query_row(
                "SELECT reservation_token, reservation_sequence, expires_at
                 FROM names WHERE normalized_name = ?1 AND uid IS NULL",
                [mutation.normalized_name],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((token, sequence, expires_at)) = reservation else {
            return Err(Error::Reservation);
        };
        if token.as_slice() != mutation.reservation_token
            || sequence != sql_integer(mutation.reservation_sequence)?
            || expires_at != sql_integer(mutation.reservation_expires_at)?
            || expires_at <= sql_integer(mutation.now)?
        {
            return Err(Error::Reservation);
        }
        transaction.execute(
            "UPDATE names SET reservation_token = NULL, expires_at = NULL, uid = ?1
             WHERE normalized_name = ?2",
            params![mutation.uid, mutation.normalized_name],
        )?;
        inject(failure, FailurePoint::Name)?;

        transaction.execute(
            "INSERT INTO users
             (uid, normalized_name, username_utf8, username_sequence,
              username_commitment_key, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                mutation.uid,
                mutation.normalized_name,
                mutation.username_utf8,
                sql_integer(mutation.reservation_sequence)?,
                mutation.username_commitment_key,
                sql_integer(mutation.now)?
            ],
        )?;
        inject(failure, FailurePoint::User)?;
        transaction.execute(
            "INSERT INTO devices
             (device_id, uid, active, hepk_fingerprint, exact_hepk, exact_name)
             VALUES (?1, ?2, 1, ?3, ?4, ?5)",
            params![
                mutation.device_id,
                mutation.uid,
                mutation.device_hepk_fingerprint,
                mutation.exact_device_hepk,
                mutation.exact_device_name
            ],
        )?;
        inject(failure, FailurePoint::Device)?;

        transaction.execute(
            "INSERT INTO user_chain_links(uid, seqno, link_hash, exact_link, root_epoch)
             VALUES (?1, 1, ?2, ?3, ?4)",
            params![
                mutation.uid,
                mutation.link_hash,
                mutation.exact_link,
                sql_integer(mutation.root_epoch)?
            ],
        )?;
        transaction.execute(
            "INSERT INTO user_chain_heads(uid, seqno, link_hash) VALUES (?1, 1, ?2)",
            params![mutation.uid, mutation.link_hash],
        )?;
        transaction.execute(
            "INSERT INTO tree_locations(uid, seqno, location) VALUES (?1, 1, ?2)",
            params![mutation.uid, mutation.tree_location],
        )?;
        transaction.execute(
            "INSERT INTO shared_keys
             (uid, role_type, visibility, generation, verify_key, exact_hepk)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                mutation.uid,
                sql_integer(mutation.shared_role_type)?,
                mutation.shared_visibility,
                sql_integer(mutation.shared_generation)?,
                mutation.shared_verify_key,
                mutation.exact_shared_hepk
            ],
        )?;
        transaction.execute(
            "INSERT INTO parcels
             (uid, device_id, role_type, visibility, generation, exact_parcel)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                mutation.uid,
                mutation.device_id,
                sql_integer(mutation.shared_role_type)?,
                mutation.shared_visibility,
                sql_integer(mutation.shared_generation)?,
                mutation.exact_parcel
            ],
        )?;
        inject(failure, FailurePoint::Chain)?;

        let current: Option<(i64, Vec<u8>)> = transaction
            .query_row(
                "SELECT epoch, root_hash FROM merkle_root_heads WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match (current, mutation.expected_root_hash) {
            (None, None) if mutation.root_epoch == 1 => {}
            (Some((epoch, hash)), Some(expected))
                if hash.as_slice() == expected
                    && mutation.root_epoch == crate::error::unsigned(epoch)? + 1 => {}
            _ => return Err(Error::StaleRoot),
        }
        for (hash, encoded) in &mutation.merkle_commit.nodes {
            let existing: Option<Vec<u8>> = transaction
                .query_row(
                    "SELECT exact_node FROM merkle_nodes WHERE node_hash = ?1",
                    [hash],
                    |row| row.get(0),
                )
                .optional()?;
            match existing {
                Some(existing) if existing == *encoded => {}
                Some(_) => return Err(Error::Invalid("conflicting Merkle node")),
                None => {
                    transaction.execute(
                        "INSERT INTO merkle_nodes(node_hash, exact_node) VALUES (?1, ?2)",
                        params![hash, encoded],
                    )?;
                }
            }
        }
        for (key, value) in mutation.merkle_leaves {
            transaction.execute(
                "INSERT INTO merkle_leaves(leaf_key, leaf_value) VALUES (?1, ?2)
                 ON CONFLICT(leaf_key) DO UPDATE SET leaf_value = excluded.leaf_value",
                params![key, value],
            )?;
        }
        inject(failure, FailurePoint::MerkleNodes)?;

        transaction.execute(
            "INSERT INTO merkle_roots
             (epoch, root_hash, root_node, exact_root, exact_signed_root, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                sql_integer(mutation.root_epoch)?,
                mutation.root_hash,
                mutation.merkle_commit.root,
                mutation.exact_root,
                mutation.exact_signed_root,
                sql_integer(mutation.now)?
            ],
        )?;
        for (ordinal, (target_epoch, target_hash)) in mutation.back_pointers.iter().enumerate() {
            transaction.execute(
                "INSERT INTO merkle_back_pointers
                 (root_epoch, target_epoch, target_hash, ordinal) VALUES (?1, ?2, ?3, ?4)",
                params![
                    sql_integer(mutation.root_epoch)?,
                    sql_integer(*target_epoch)?,
                    target_hash,
                    i64::try_from(ordinal).map_err(|_| Error::IntegerRange)?
                ],
            )?;
        }
        transaction.execute(
            "INSERT INTO merkle_root_heads(singleton, epoch, root_hash) VALUES (1, ?1, ?2)
             ON CONFLICT(singleton) DO UPDATE SET epoch = excluded.epoch, root_hash = excluded.root_hash",
            params![sql_integer(mutation.root_epoch)?, mutation.root_hash],
        )?;
        inject(failure, FailurePoint::MerkleRoot)?;

        receipts::insert(
            &transaction,
            mutation.idempotency_key,
            mutation.request_hash,
            mutation.response,
            mutation.now,
            mutation.receipt_expires_at,
        )?;
        inject(failure, FailurePoint::Receipt)?;
        transaction.commit()?;
        Ok(CommitOutcome::Committed(mutation.response.to_vec()))
    }
}

fn validate(database: &Database, mutation: &IdentityMutation<'_>) -> Result<()> {
    let blobs = [
        mutation.username_utf8,
        mutation.exact_device_hepk,
        mutation.exact_device_name,
        mutation.exact_link,
        mutation.exact_shared_hepk,
        mutation.exact_parcel,
        mutation.exact_root,
        mutation.exact_signed_root,
    ];
    if mutation.uid.len() != 33
        || !matches!(mutation.device_id.len(), 33 | 34)
        || !matches!(mutation.shared_verify_key.len(), 33 | 34)
        || mutation.normalized_name.is_empty()
        || mutation.username_utf8.is_empty()
        || mutation.normalized_name.len() > database.config.maximum_name_bytes
        || mutation.response.len() > database.config.maximum_receipt_bytes
        || blobs
            .iter()
            .any(|blob| blob.len() > database.config.maximum_blob_bytes)
        || mutation.merkle_commit.nodes.len() > database.config.maximum_merkle_nodes_per_commit
        || mutation.back_pointers.len() > database.config.maximum_back_pointers
        || mutation.shared_generation == 0
        || mutation.reservation_sequence == 0
        || mutation.reservation_expires_at <= mutation.now
        || mutation.receipt_expires_at <= mutation.now
        || mutation.merkle_commit.root == [0; 32]
    {
        return Err(Error::Invalid("identity mutation"));
    }
    for (hash, encoded) in &mutation.merkle_commit.nodes {
        if encoded.len() > database.config.maximum_blob_bytes {
            return Err(Error::Invalid("oversized Merkle node"));
        }
        let node = foks_merkle_store::Node::decode(encoded)?;
        if foks_merkle_store::hash_node(&node)? != *hash {
            return Err(Error::Invalid("Merkle node hash"));
        }
    }
    let root_is_new = mutation
        .merkle_commit
        .nodes
        .iter()
        .any(|(hash, _)| hash == &mutation.merkle_commit.root);
    let root_exists = database
        .connection
        .query_row(
            "SELECT 1 FROM merkle_nodes WHERE node_hash = ?1",
            [mutation.merkle_commit.root],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !root_is_new && !root_exists {
        return Err(Error::Invalid("missing Merkle root node"));
    }
    let expected_pointer_epochs = foks_merkle_store::back_pointer_sequence(mutation.root_epoch);
    if mutation
        .back_pointers
        .iter()
        .map(|(epoch, _)| *epoch)
        .ne(expected_pointer_epochs)
    {
        return Err(Error::Invalid("Merkle back-pointer sequence"));
    }
    Ok(())
}
