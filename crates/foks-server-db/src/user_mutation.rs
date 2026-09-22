use rusqlite::{params, OptionalExtension as _, TransactionBehavior};

use crate::{error::sql_integer, idempotency, Database, Error, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserMutationFailurePoint {
    Chain,
    Projection,
    Passphrase,
    MerkleNodes,
    MerkleRoot,
    IdempotencyRecord,
}

pub struct AddedCredential<'a> {
    pub device_id: &'a [u8],
    pub self_token: &'a [u8; 17],
    pub hepk_fingerprint: &'a [u8; 32],
    pub exact_hepk: &'a [u8],
    pub exact_name: &'a [u8],
    pub role_type: u64,
    pub visibility: i64,
    pub subkey_id: Option<&'a [u8]>,
    pub exact_subkey_box: Option<&'a [u8]>,
    pub yubi_pq_hint: Option<(u8, &'a [u8; 32])>,
}

pub struct SharedKeyMutation<'a> {
    pub role_type: u64,
    pub visibility: i64,
    pub generation: u64,
    pub verify_key: &'a [u8],
    pub exact_hepk: &'a [u8],
}

pub struct ParcelMutation<'a> {
    pub device_id: &'a [u8],
    pub sender_id: &'a [u8],
    pub role_type: u64,
    pub visibility: i64,
    pub generation: u64,
    pub exact_parcel: &'a [u8],
}

pub struct SeedChainMutation<'a> {
    pub role_type: u64,
    pub visibility: i64,
    pub generation: u64,
    pub exact_box: &'a [u8],
}

#[derive(Clone, Debug)]
pub struct UsernameMutation {
    pub normalized_name: Vec<u8>,
    pub username_utf8: Vec<u8>,
    pub commitment_key: [u8; 16],
    pub token: [u8; 17],
    pub name_sequence: u64,
    pub reservation_expires_at: u64,
}

pub struct UserMutation<'a> {
    pub username: Option<&'a UsernameMutation>,
    pub uid: &'a [u8],
    pub signer_device_id: &'a [u8],
    pub expected_sequence: u64,
    pub expected_tail_hash: &'a [u8; 32],
    pub link_hash: &'a [u8; 32],
    pub exact_link: &'a [u8],
    pub current_tree_location: &'a [u8; 32],
    pub next_tree_location: &'a [u8; 32],
    pub added_credential: Option<AddedCredential<'a>>,
    pub revoked_device_id: Option<&'a [u8]>,
    pub shared_keys: &'a [SharedKeyMutation<'a>],
    pub parcels: &'a [ParcelMutation<'a>],
    pub seed_chain: &'a [SeedChainMutation<'a>],
    pub passphrase: Option<crate::PassphraseMutation<'a>>,
    pub user_settings: Option<crate::GenericLinkMutation<'a>>,
    /// Merkle epoch cited by the signed user-chain link. This can trail the
    /// authoritative head, but it must not trail side-chain work signed by a
    /// credential whose signing interval this mutation ends.
    pub cited_root_epoch: u64,
    pub expected_root_epoch: u64,
    pub expected_root_hash: &'a [u8; 32],
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
    pub idempotency_expires_at: u64,
}

impl Database {
    pub fn commit_user_mutation(&mut self, mutation: &UserMutation<'_>) -> Result<Vec<u8>> {
        self.commit_user_mutation_with_failure(mutation, None)
    }

    #[doc(hidden)]
    pub fn commit_user_mutation_with_failure(
        &mut self,
        mutation: &UserMutation<'_>,
        failure: Option<UserMutationFailurePoint>,
    ) -> Result<Vec<u8>> {
        validate(self, mutation)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(idempotency_record) = idempotency::lookup(
            &transaction,
            mutation.idempotency_key,
            mutation.request_hash,
            mutation.now,
        )? {
            return Ok(idempotency_record.response);
        }
        let current: Option<(i64, Vec<u8>, i64, Vec<u8>)> = transaction
            .query_row(
                "SELECT h.seqno, h.link_hash, r.epoch, r.root_hash
                 FROM user_chain_heads h
                 JOIN merkle_root_heads rh ON rh.singleton = 1
                 JOIN merkle_roots r ON r.epoch = rh.epoch
                 WHERE h.uid = ?1",
                [mutation.uid],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((sequence, tail_hash, root_epoch, root_hash)) = current else {
            return Err(Error::Invalid("unknown user mutation authority"));
        };
        if crate::error::unsigned(sequence)?
            .checked_add(1)
            .is_none_or(|next| next != mutation.expected_sequence)
            || tail_hash.as_slice() != mutation.expected_tail_hash
            || crate::error::unsigned(root_epoch)? != mutation.expected_root_epoch
            || root_hash.as_slice() != mutation.expected_root_hash
            || mutation.root_epoch != mutation.expected_root_epoch.saturating_add(1)
        {
            return Err(Error::StaleRoot);
        }
        let signer_active: bool = transaction
            .query_row(
                "SELECT active FROM devices WHERE uid = ?1 AND device_id = ?2",
                params![mutation.uid, mutation.signer_device_id],
                |row| Ok(row.get::<_, i64>(0)? == 1),
            )
            .optional()?
            .unwrap_or(false);
        if !signer_active {
            return Err(Error::Invalid("inactive user mutation signer"));
        }
        assert_no_racing_revoked_credential(&transaction, mutation)?;
        if let Some(name) = mutation.username {
            let expiry = name
                .reservation_expires_at
                .checked_mul(1000)
                .ok_or(Error::IntegerRange)?;
            let valid: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM names WHERE normalized_name=?1 AND uid IS NULL
                 AND reservation_token=?2 AND reservation_sequence=?3 AND expires_at=?4 AND expires_at>?5)",
                params![name.normalized_name, name.token, sql_integer(name.name_sequence)?, sql_integer(expiry)?, sql_integer(mutation.now)?],
                |r| r.get(0))?;
            if !valid {
                return Err(Error::Reservation);
            }
            transaction.execute(
                "UPDATE names SET dead=1 WHERE uid=?1 AND dead=0",
                [mutation.uid],
            )?;
            transaction.execute("UPDATE names SET uid=?1, reservation_token=NULL, expires_at=NULL WHERE normalized_name=?2", params![mutation.uid, name.normalized_name])?;
            transaction.execute("UPDATE users SET normalized_name=?1, username_utf8=?2, username_sequence=?3, username_commitment_key=?4 WHERE uid=?5",
                params![name.normalized_name, name.username_utf8, sql_integer(name.name_sequence)?, name.commitment_key, mutation.uid])?;
            transaction.execute(
                "INSERT INTO user_name_history VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    mutation.uid,
                    sql_integer(mutation.expected_sequence)?,
                    name.normalized_name,
                    sql_integer(name.name_sequence)?,
                    name.commitment_key
                ],
            )?;
        }

        enforce_credential_capacity(&transaction, &self.config, mutation)?;

        transaction.execute(
            "INSERT INTO user_chain_links(uid, seqno, link_hash, exact_link, root_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                mutation.uid,
                sql_integer(mutation.expected_sequence)?,
                mutation.link_hash,
                mutation.exact_link,
                sql_integer(mutation.root_epoch)?
            ],
        )?;
        transaction.execute(
            "UPDATE user_chain_heads SET seqno = ?1, link_hash = ?2 WHERE uid = ?3",
            params![
                sql_integer(mutation.expected_sequence)?,
                mutation.link_hash,
                mutation.uid
            ],
        )?;
        transaction.execute(
            "INSERT INTO tree_locations(uid, seqno, location) VALUES (?1, ?2, ?3)",
            params![
                mutation.uid,
                sql_integer(mutation.expected_sequence)?,
                mutation.next_tree_location
            ],
        )?;
        inject(failure, UserMutationFailurePoint::Chain)?;

        if let Some(added) = &mutation.added_credential {
            transaction.execute(
                "INSERT INTO devices
                 (device_id, uid, active, role_type, visibility, subkey_id,
                  hepk_fingerprint, self_token, exact_hepk, exact_name, start_epoch)
                 VALUES (?1, ?2, 1, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    added.device_id,
                    mutation.uid,
                    sql_integer(added.role_type)?,
                    added.visibility,
                    added.subkey_id,
                    added.hepk_fingerprint,
                    added.self_token,
                    added.exact_hepk,
                    added.exact_name,
                    sql_integer(mutation.root_epoch)?
                ],
            )?;
            crate::identity::insert_yubi_projection(
                &transaction,
                added.device_id,
                added.subkey_id,
                added.exact_subkey_box,
                added.yubi_pq_hint,
                mutation.now,
            )?;
        }
        if let Some(revoked) = mutation.revoked_device_id {
            if transaction.execute(
                "UPDATE devices SET active = 0 WHERE uid = ?1 AND device_id = ?2 AND active = 1",
                params![mutation.uid, revoked],
            )? != 1
            {
                return Err(Error::Invalid("revoked credential is not active"));
            }
        }
        for key in mutation.shared_keys {
            transaction.execute(
                "INSERT INTO shared_keys
                 (uid, role_type, visibility, generation, verify_key, exact_hepk)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    mutation.uid,
                    sql_integer(key.role_type)?,
                    key.visibility,
                    sql_integer(key.generation)?,
                    key.verify_key,
                    key.exact_hepk
                ],
            )?;
        }
        for parcel in mutation.parcels {
            transaction.execute(
                "INSERT INTO parcels
                 (uid, device_id, sender_id, role_type, visibility, generation, exact_parcel)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    mutation.uid,
                    parcel.device_id,
                    parcel.sender_id,
                    sql_integer(parcel.role_type)?,
                    parcel.visibility,
                    sql_integer(parcel.generation)?,
                    parcel.exact_parcel
                ],
            )?;
        }
        for boxed in mutation.seed_chain {
            if transaction.execute(
                "INSERT INTO seed_chain_boxes
                 (uid, role_type, visibility, generation, exact_box)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(uid, role_type, visibility, generation)
                 DO UPDATE SET exact_box = excluded.exact_box
                 WHERE exact_box = excluded.exact_box",
                params![
                    mutation.uid,
                    sql_integer(boxed.role_type)?,
                    boxed.visibility,
                    sql_integer(boxed.generation)?,
                    boxed.exact_box
                ],
            )? != 1
            {
                return Err(Error::Invalid("conflicting PUK seed-chain box"));
            }
        }
        inject(failure, UserMutationFailurePoint::Projection)?;

        let owner_generation = mutation
            .shared_keys
            .iter()
            .find(|key| key.role_type == 3 && key.visibility == 0)
            .map(|key| key.generation);
        crate::passphrases::apply_owner_rotation(
            &transaction,
            &self.config,
            mutation.uid,
            owner_generation,
            mutation.passphrase,
        )?;
        if let Some(settings) = &mutation.user_settings {
            crate::generic::insert_generic_link(
                &transaction,
                &self.config,
                settings,
                mutation.root_epoch,
            )?;
        }
        inject(failure, UserMutationFailurePoint::Passphrase)?;

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
                "INSERT INTO merkle_leaves(leaf_key, leaf_value, epoch) VALUES (?1, ?2, ?3)
                 ON CONFLICT(leaf_key) DO UPDATE SET leaf_value = excluded.leaf_value",
                params![key, value, sql_integer(mutation.root_epoch)?],
            )?;
        }
        inject(failure, UserMutationFailurePoint::MerkleNodes)?;
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
            "UPDATE merkle_root_heads SET epoch = ?1, root_hash = ?2 WHERE singleton = 1",
            params![sql_integer(mutation.root_epoch)?, mutation.root_hash],
        )?;
        inject(failure, UserMutationFailurePoint::MerkleRoot)?;
        idempotency::insert(
            &transaction,
            mutation.idempotency_key,
            mutation.request_hash,
            mutation.response,
            mutation.now,
            mutation.idempotency_expires_at,
        )?;
        inject(failure, UserMutationFailurePoint::IdempotencyRecord)?;
        transaction.commit()?;
        Ok(mutation.response.to_vec())
    }
}

fn validate(database: &Database, mutation: &UserMutation<'_>) -> Result<()> {
    if mutation.uid.len() != 33
        || !matches!(mutation.signer_device_id.len(), 33 | 34)
        || mutation.expected_sequence < 2
        || mutation.expected_sequence > database.config.maximum_user_chain_links
        || mutation.root_epoch != mutation.expected_root_epoch.saturating_add(1)
        || mutation.exact_link.is_empty()
        || mutation.exact_link.len() > database.config.maximum_blob_bytes
        || mutation.parcels.len() > database.config.maximum_boxes_per_mutation
        || mutation.seed_chain.len() > database.config.maximum_boxes_per_mutation
        || mutation.shared_keys.len() > 16
        || mutation.merkle_commit.nodes.len() > database.config.maximum_merkle_nodes_per_commit
        || mutation.back_pointers.len() > database.config.maximum_back_pointers
        || mutation.response.len() > database.config.maximum_idempotency_response_bytes
        || mutation.idempotency_expires_at <= mutation.now
        || mutation.cited_root_epoch == 0
        || mutation.cited_root_epoch > mutation.expected_root_epoch
    {
        return Err(Error::Invalid("user mutation"));
    }
    let uid = foks_proto::EntityId::from_bytes(mutation.uid.to_vec())
        .map_err(|_| Error::Invalid("user mutation entity"))?;
    let mut expected_leaves = vec![(
        foks_merkle_store::chain_key(
            0,
            &uid,
            mutation.expected_sequence,
            Some(mutation.current_tree_location),
        )?,
        *mutation.link_hash,
    )];
    if let Some(name) = mutation.username {
        if name.normalized_name.is_empty()
            || name.normalized_name.len() > database.config.maximum_name_bytes
            || name.name_sequence != 1
            || name.username_utf8.len() > 4096
        {
            return Err(Error::Invalid("username mutation"));
        }
        let host = database
            .host_bootstrap()?
            .ok_or(Error::Invalid("missing host"))?;
        let host = foks_proto::EntityId::from_bytes(host.host_id)
            .map_err(|_| Error::Invalid("host ID"))?;
        expected_leaves.push((
            foks_merkle_store::username_key(&name.normalized_name, &host, name.name_sequence)?,
            foks_merkle_store::username_leaf(&uid)?,
        ));
    }
    if let Some(settings) = &mutation.user_settings {
        expected_leaves.push((
            foks_merkle_store::chain_key(
                settings.chain_type,
                &uid,
                settings.sequence,
                Some(settings.current_tree_location),
            )?,
            *settings.link_hash,
        ));
    }
    if mutation.merkle_leaves != expected_leaves {
        return Err(Error::Invalid("user mutation Merkle leaves"));
    }
    let expected = foks_merkle_store::back_pointer_sequence(mutation.root_epoch);
    if mutation
        .back_pointers
        .iter()
        .map(|(epoch, _)| *epoch)
        .ne(expected)
    {
        return Err(Error::Invalid("Merkle back-pointer sequence"));
    }
    if let Some(added) = &mutation.added_credential {
        let is_yubi = added.device_id.first() == Some(&foks_proto::ENTITY_YUBI);
        let valid = match (added.subkey_id, added.exact_subkey_box, added.yubi_pq_hint) {
            (Some(subkey), Some(boxed), Some((slot, _))) => {
                is_yubi
                    && subkey.len() == 33
                    && subkey.first() == Some(&foks_proto::ENTITY_SUBKEY)
                    && !boxed.is_empty()
                    && boxed.len() <= database.config.maximum_blob_bytes
                    && (0x82..=0x95).contains(&slot)
            }
            (None, None, None) => !is_yubi,
            _ => false,
        };
        if !valid {
            return Err(Error::Invalid("Yubi credential projection"));
        }
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
    Ok(())
}

fn assert_no_racing_revoked_credential(
    transaction: &rusqlite::Transaction<'_>,
    mutation: &UserMutation<'_>,
) -> Result<()> {
    let Some(revoked) = mutation.revoked_device_id else {
        return Ok(());
    };
    let mut statement = transaction.prepare(
        "SELECT exact_link FROM generic_chain_links
         WHERE entity_id = ?1 AND root_epoch > ?2",
    )?;
    let mut rows = statement.query(params![
        mutation.uid,
        sql_integer(mutation.cited_root_epoch)?
    ])?;
    while let Some(row) = rows.next()? {
        let exact: Vec<u8> = row.get(0)?;
        let link = foks_proto::UserLink::decode(&exact)
            .map_err(|_| Error::Invalid("stored generic user link"))?;
        let decoded = link
            .decode_generic()
            .map_err(|_| Error::Invalid("stored generic user link"))?;
        if decoded.signer.as_bytes() == revoked {
            return Err(Error::StaleRoot);
        }
    }
    Ok(())
}

fn enforce_credential_capacity(
    transaction: &rusqlite::Transaction<'_>,
    config: &crate::Config,
    mutation: &UserMutation<'_>,
) -> Result<()> {
    let Some(added) = &mutation.added_credential else {
        return Ok(());
    };
    let active: i64 = transaction.query_row(
        "SELECT count(*) FROM devices WHERE uid = ?1 AND active = 1",
        [mutation.uid],
        |row| row.get(0),
    )?;
    if crate::error::unsigned(active)? >= config.maximum_active_credentials_per_user as u64 {
        return Err(Error::QuotaExceeded);
    }
    if added.device_id.first() == Some(&foks_proto::ENTITY_BACKUP_KEY) {
        let backups: i64 = transaction.query_row(
            "SELECT count(*) FROM devices
             WHERE uid = ?1 AND active = 1 AND substr(device_id, 1, 1) = ?2",
            params![mutation.uid, [foks_proto::ENTITY_BACKUP_KEY]],
            |row| row.get(0),
        )?;
        if crate::error::unsigned(backups)? >= config.maximum_backup_credentials_per_user as u64 {
            return Err(Error::QuotaExceeded);
        }
    }
    Ok(())
}

fn inject(
    requested: Option<UserMutationFailurePoint>,
    current: UserMutationFailurePoint,
) -> Result<()> {
    if requested == Some(current) {
        Err(Error::UserMutationInjected(current))
    } else {
        Ok(())
    }
}
