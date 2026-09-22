use rusqlite::{params, OptionalExtension as _, Transaction, TransactionBehavior};

use crate::{error::sql_integer, error::unsigned, Config, Database, Error, Result};

#[derive(Clone, Copy)]
pub struct GenericPassphraseInfo<'a> {
    pub generation: u64,
    pub salt: Option<&'a [u8; 16]>,
    pub stretch_version: u64,
}

pub struct GenericLinkMutation<'a> {
    pub entity_id: &'a [u8],
    pub chain_type: u64,
    pub signer_credential_id: &'a [u8],
    pub sequence: u64,
    pub previous: Option<&'a [u8; 32]>,
    pub link_root_epoch: u64,
    pub link_root_hash: &'a [u8; 32],
    pub current_tree_location: &'a [u8; 32],
    pub next_tree_location: &'a [u8; 32],
    pub link_hash: &'a [u8; 32],
    pub exact_link: &'a [u8],
    pub passphrase_info: Option<GenericPassphraseInfo<'a>>,
}

pub enum GenericPassphraseAction<'a> {
    Set {
        credential_id: &'a [u8],
        mutation: crate::PassphraseMutation<'a>,
    },
    Change {
        credential_id: &'a [u8],
        mutation: crate::PassphraseMutation<'a>,
    },
}

pub struct GenericMutation<'a> {
    pub invitation: Option<&'a crate::LocalInvitationAcceptance>,
    pub link: GenericLinkMutation<'a>,
    pub passphrase: Option<GenericPassphraseAction<'a>>,
    pub expected_root_epoch: u64,
    pub expected_root_hash: &'a [u8; 32],
    pub merkle_commit: &'a foks_merkle_store::Commit,
    pub merkle_leaves: &'a [([u8; 32], [u8; 32])],
    pub root_epoch: u64,
    pub root_hash: &'a [u8; 32],
    pub exact_root: &'a [u8],
    pub exact_signed_root: &'a [u8],
    pub back_pointers: &'a [(u64, [u8; 32])],
    pub now: u64,
}

impl Database {
    pub fn commit_generic_mutation(&mut self, mutation: &GenericMutation<'_>) -> Result<()> {
        validate(self, mutation)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let root: (i64, Vec<u8>) = transaction.query_row(
            "SELECT r.epoch, r.root_hash FROM merkle_root_heads h
             JOIN merkle_roots r ON r.epoch = h.epoch WHERE h.singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if unsigned(root.0)? != mutation.expected_root_epoch
            || root.1.as_slice() != mutation.expected_root_hash
            || mutation.root_epoch != mutation.expected_root_epoch.saturating_add(1)
        {
            return Err(Error::StaleRoot);
        }
        if let Some(passphrase) = &mutation.passphrase {
            match passphrase {
                GenericPassphraseAction::Set {
                    credential_id,
                    mutation: passphrase,
                } => Database::apply_set_passphrase(
                    &transaction,
                    &self.config,
                    mutation.link.entity_id,
                    credential_id,
                    *passphrase,
                )?,
                GenericPassphraseAction::Change {
                    credential_id,
                    mutation: passphrase,
                } => Database::apply_change_passphrase(
                    &transaction,
                    &self.config,
                    mutation.link.entity_id,
                    credential_id,
                    *passphrase,
                )?,
            }
        }
        if let Some(invitation) = mutation.invitation {
            if mutation.link.entity_id != invitation.joiner.as_bytes()
                || mutation.link.chain_type != foks_proto::CHAIN_TYPE_TEAM_MEMBERSHIP
            {
                return Err(Error::Invalid("invitation membership link"));
            }
            crate::team_invitations::commit_local_invitation_acceptance(
                &transaction,
                invitation,
                mutation.now,
            )?;
        }
        insert_generic_link(
            &transaction,
            &self.config,
            &mutation.link,
            mutation.root_epoch,
        )?;
        publish_merkle(&transaction, mutation)?;
        transaction.commit()?;
        Ok(())
    }
}

pub(crate) fn insert_generic_link(
    transaction: &Transaction<'_>,
    config: &Config,
    mutation: &GenericLinkMutation<'_>,
    publication_epoch: u64,
) -> Result<()> {
    insert_generic_link_inner(transaction, config, mutation, publication_epoch)
}

fn insert_generic_link_inner(
    transaction: &Transaction<'_>,
    config: &Config,
    mutation: &GenericLinkMutation<'_>,
    publication_epoch: u64,
) -> Result<()> {
    validate_link(config, mutation)?;
    let entity = foks_proto::EntityId::from_bytes(mutation.entity_id.to_vec())
        .map_err(|_| Error::Invalid("generic entity ID"))?;
    let signer_start: Option<i64> = match entity.entity_type() {
        foks_proto::ENTITY_USER => transaction
            .query_row(
                "SELECT start_epoch FROM devices
                 WHERE uid = ?1 AND active = 1 AND device_id = ?2",
                params![mutation.entity_id, mutation.signer_credential_id],
                |row| row.get(0),
            )
            .optional()?,
        foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM => transaction
            .query_row(
                "SELECT start_epoch FROM team_shared_keys AS k
                 WHERE k.team_id = ?1 AND k.verify_key = ?2
                   AND k.role_type IN (2, 3) AND k.visibility = 0
                   AND k.generation = (
                     SELECT max(k2.generation) FROM team_shared_keys AS k2
                     WHERE k2.team_id = k.team_id AND k2.role_type = k.role_type
                       AND k2.visibility = k.visibility)",
                params![mutation.entity_id, mutation.signer_credential_id],
                |row| row.get(0),
            )
            .optional()?,
        _ => None,
    };
    if signer_start
        .map(unsigned)
        .transpose()?
        .is_none_or(|start| start > mutation.link_root_epoch)
    {
        return Err(Error::AuthorizationChanged);
    }
    let cited_hash: Option<Vec<u8>> = transaction
        .query_row(
            "SELECT root_hash FROM merkle_roots WHERE epoch = ?1",
            [sql_integer(mutation.link_root_epoch)?],
            |row| row.get(0),
        )
        .optional()?;
    if cited_hash.as_deref() != Some(mutation.link_root_hash.as_slice()) {
        return Err(Error::StaleRoot);
    }
    let head: Option<(i64, Vec<u8>)> = transaction
        .query_row(
            "SELECT seqno, link_hash FROM generic_chain_heads
             WHERE entity_id = ?1 AND chain_type = ?2",
            params![mutation.entity_id, sql_integer(mutation.chain_type)?],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match head {
        None if mutation.sequence == 1 && mutation.previous.is_none() => {}
        Some((sequence, hash))
            if unsigned(sequence)?.checked_add(1) == Some(mutation.sequence)
                && mutation
                    .previous
                    .is_some_and(|previous| hash.as_slice() == previous) => {}
        _ => return Err(Error::StaleRoot),
    }
    let expected_location = if mutation.sequence == 1 {
        let seed: Vec<u8> = transaction.query_row(
            "SELECT seed FROM subchain_tree_location_seeds WHERE entity_id = ?1",
            [mutation.entity_id],
            |row| row.get(0),
        )?;
        let seed: [u8; 32] = seed
            .try_into()
            .map_err(|_| Error::Invalid("stored subchain seed"))?;
        foks_crypto::subchain_tree_location(&seed, mutation.chain_type)
            .map_err(|_| Error::Invalid("subchain location derivation"))?
    } else {
        let location: Vec<u8> = transaction.query_row(
            "SELECT location FROM generic_tree_locations
             WHERE entity_id = ?1 AND chain_type = ?2 AND seqno = ?3",
            params![
                mutation.entity_id,
                sql_integer(mutation.chain_type)?,
                sql_integer(mutation.sequence)?
            ],
            |row| row.get(0),
        )?;
        location
            .try_into()
            .map_err(|_| Error::Invalid("stored generic tree location"))?
    };
    if expected_location != *mutation.current_tree_location {
        return Err(Error::Invalid("generic current tree location"));
    }
    validate_passphrase_payload(transaction, mutation)?;
    transaction.execute(
        "INSERT INTO generic_chain_links
         (entity_id, chain_type, seqno, link_hash, exact_link, root_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            mutation.entity_id,
            sql_integer(mutation.chain_type)?,
            sql_integer(mutation.sequence)?,
            mutation.link_hash,
            mutation.exact_link,
            sql_integer(publication_epoch)?
        ],
    )?;
    transaction.execute(
        "INSERT INTO generic_chain_heads(entity_id, chain_type, seqno, link_hash)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(entity_id, chain_type) DO UPDATE SET
           seqno = excluded.seqno, link_hash = excluded.link_hash",
        params![
            mutation.entity_id,
            sql_integer(mutation.chain_type)?,
            sql_integer(mutation.sequence)?,
            mutation.link_hash
        ],
    )?;
    let next_sequence = mutation
        .sequence
        .checked_add(1)
        .ok_or(Error::IntegerRange)?;
    transaction.execute(
        "INSERT INTO generic_tree_locations(entity_id, chain_type, seqno, location)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            mutation.entity_id,
            sql_integer(mutation.chain_type)?,
            sql_integer(next_sequence)?,
            mutation.next_tree_location
        ],
    )?;
    Ok(())
}

fn validate_passphrase_payload(
    transaction: &Transaction<'_>,
    mutation: &GenericLinkMutation<'_>,
) -> Result<()> {
    match (mutation.chain_type, mutation.passphrase_info) {
        (foks_proto::CHAIN_TYPE_USER_SETTINGS, Some(info)) => {
            let current = crate::passphrases::snapshot(transaction, mutation.entity_id)?
                .ok_or(Error::PassphraseNotFound)?;
            if current.generation != info.generation
                || current.stretch_version != info.stretch_version
                || info.salt.is_some_and(|salt| current.salt != *salt)
            {
                return Err(Error::PassphraseGeneration);
            }
            Ok(())
        }
        (foks_proto::CHAIN_TYPE_TEAM_MEMBERSHIP, None) => Ok(()),
        _ => Err(Error::Invalid("generic link payload")),
    }
}

fn validate(database: &Database, mutation: &GenericMutation<'_>) -> Result<()> {
    validate_link(&database.config, &mutation.link)?;
    if mutation.root_epoch != mutation.expected_root_epoch.saturating_add(1)
        || mutation.merkle_commit.nodes.len() > database.config.maximum_merkle_nodes_per_commit
        || mutation.back_pointers.len() > database.config.maximum_back_pointers
        || mutation.exact_root.is_empty()
        || mutation.exact_root.len() > database.config.maximum_blob_bytes
        || mutation.exact_signed_root.is_empty()
        || mutation.exact_signed_root.len() > database.config.maximum_blob_bytes
        || mutation.merkle_leaves.len() != 1
    {
        return Err(Error::Invalid("generic mutation"));
    }
    let entity = foks_proto::EntityId::from_bytes(mutation.link.entity_id.to_vec())
        .map_err(|_| Error::Invalid("generic entity ID"))?;
    let expected_key = foks_merkle_store::chain_key(
        mutation.link.chain_type,
        &entity,
        mutation.link.sequence,
        Some(mutation.link.current_tree_location),
    )?;
    if mutation.merkle_leaves[0] != (expected_key, *mutation.link.link_hash)
        || mutation.back_pointers.iter().map(|(epoch, _)| *epoch).ne(
            foks_merkle_store::back_pointer_sequence(mutation.root_epoch),
        )
    {
        return Err(Error::Invalid("generic Merkle binding"));
    }
    for (hash, encoded) in &mutation.merkle_commit.nodes {
        if encoded.len() > database.config.maximum_blob_bytes
            || foks_merkle_store::hash_node(&foks_merkle_store::Node::decode(encoded)?)? != *hash
        {
            return Err(Error::Invalid("Merkle node hash"));
        }
    }
    Ok(())
}

fn validate_link(config: &Config, mutation: &GenericLinkMutation<'_>) -> Result<()> {
    let entity_type = mutation.entity_id.first().copied();
    let valid_entity_and_chain = matches!(
        (entity_type, mutation.chain_type),
        (
            Some(foks_proto::ENTITY_USER),
            foks_proto::CHAIN_TYPE_USER_SETTINGS
        ) | (
            Some(foks_proto::ENTITY_USER),
            foks_proto::CHAIN_TYPE_TEAM_MEMBERSHIP
        ) | (
            Some(foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM),
            foks_proto::CHAIN_TYPE_TEAM_MEMBERSHIP
        )
    );
    if mutation.entity_id.len() != 33
        || !valid_entity_and_chain
        || !matches!(mutation.signer_credential_id.len(), 33 | 34)
        || mutation.sequence == 0
        || mutation.link_root_epoch == 0
        || mutation.exact_link.is_empty()
        || mutation.exact_link.len() > config.maximum_blob_bytes
    {
        return Err(Error::Invalid("generic link mutation"));
    }
    Ok(())
}

fn publish_merkle(transaction: &Transaction<'_>, mutation: &GenericMutation<'_>) -> Result<()> {
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
    Ok(())
}
