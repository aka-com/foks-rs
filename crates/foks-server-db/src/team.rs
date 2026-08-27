use rusqlite::{params, OptionalExtension as _, TransactionBehavior};

use crate::{error::sql_integer, receipts, Database, Error, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TeamMutationFailurePoint {
    Chain,
    Projection,
    MerkleNodes,
    MerkleRoot,
    Receipt,
}

pub struct TeamHeader<'a> {
    pub kind: u8,
    pub host_id: &'a [u8],
    pub normalized_name: Option<&'a [u8]>,
    pub team_name_utf8: &'a [u8],
    pub name_sequence: u64,
    pub name_commitment_key: Option<&'a [u8; 16]>,
    pub reservation_token: Option<&'a [u8; 17]>,
    pub reservation_expires_at: Option<u64>,
}

pub struct TeamMemberMutation<'a> {
    pub party_id: &'a [u8],
    pub scoped_host_id: Option<&'a [u8]>,
    pub source_role_type: u64,
    pub source_visibility: i64,
    pub role_type: u64,
    pub visibility: i64,
    pub generation: u64,
    pub verify_key: &'a [u8],
    pub hepk_fingerprint: &'a [u8; 32],
    pub removal_key_commitment: Option<&'a [u8; 32]>,
}

pub struct TeamSharedKeyMutation<'a> {
    pub role_type: u64,
    pub visibility: i64,
    pub generation: u64,
    pub verify_key: &'a [u8],
    pub exact_hepk: &'a [u8],
}

pub struct TeamParcelMutation<'a> {
    pub party_id: &'a [u8],
    pub sender_id: &'a [u8],
    pub role_type: u64,
    pub visibility: i64,
    pub generation: u64,
    pub exact_parcel: &'a [u8],
}

pub struct TeamSeedChainMutation<'a> {
    pub role_type: u64,
    pub visibility: i64,
    pub generation: u64,
    pub exact_box: &'a [u8],
}

pub struct TeamRemovalBoxMutation<'a> {
    pub member_id: &'a [u8],
    pub member_host_id: &'a [u8],
    pub source_role_type: u64,
    pub source_visibility: i64,
    pub exact_box: &'a [u8],
}

pub struct TeamMutation<'a> {
    pub team_id: &'a [u8],
    pub signer_credential_id: &'a [u8],
    pub header: Option<TeamHeader<'a>>,
    pub expected_sequence: u64,
    pub expected_tail_hash: Option<&'a [u8; 32]>,
    pub link_hash: &'a [u8; 32],
    pub exact_link: &'a [u8],
    pub next_tree_location: &'a [u8; 32],
    pub members: &'a [TeamMemberMutation<'a>],
    pub shared_keys: &'a [TeamSharedKeyMutation<'a>],
    pub parcels: &'a [TeamParcelMutation<'a>],
    pub seed_chain: &'a [TeamSeedChainMutation<'a>],
    pub removal_boxes: &'a [TeamRemovalBoxMutation<'a>],
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
    pub receipt_expires_at: u64,
}

impl Database {
    pub fn commit_team_mutation(&mut self, mutation: &TeamMutation<'_>) -> Result<Vec<u8>> {
        self.commit_team_mutation_with_failure(mutation, None)
    }

    #[doc(hidden)]
    pub fn commit_team_mutation_with_failure(
        &mut self,
        mutation: &TeamMutation<'_>,
        failure: Option<TeamMutationFailurePoint>,
    ) -> Result<Vec<u8>> {
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
            return Ok(receipt.response);
        }
        let active_signer = transaction
            .query_row(
                "SELECT 1 FROM devices WHERE device_id = ?1 AND active = 1",
                [mutation.signer_credential_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !active_signer {
            return Err(Error::Invalid("inactive team mutation signer"));
        }
        let root: (i64, Vec<u8>) = transaction.query_row(
            "SELECT r.epoch, r.root_hash FROM merkle_root_heads h
             JOIN merkle_roots r ON r.epoch = h.epoch WHERE h.singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if crate::error::unsigned(root.0)? != mutation.expected_root_epoch
            || root.1.as_slice() != mutation.expected_root_hash
            || mutation.root_epoch != mutation.expected_root_epoch.saturating_add(1)
        {
            return Err(Error::StaleRoot);
        }

        let (team_kind, team_host) = if let Some(header) = &mutation.header {
            if mutation.expected_sequence != 1 || mutation.expected_tail_hash.is_some() {
                return Err(Error::Invalid("team creation sequence"));
            }
            let count: i64 =
                transaction.query_row("SELECT count(*) FROM teams", [], |row| row.get(0))?;
            if usize::try_from(count).unwrap_or(usize::MAX) >= self.config.maximum_teams {
                return Err(Error::QuotaExceeded);
            }
            if header.kind == foks_proto::ENTITY_NAMED_TEAM {
                let name = header
                    .normalized_name
                    .ok_or(Error::Invalid("missing named-team name"))?;
                let token = header
                    .reservation_token
                    .ok_or(Error::Invalid("missing team-name reservation"))?;
                let expiry = header
                    .reservation_expires_at
                    .ok_or(Error::Invalid("missing team-name expiry"))?;
                if transaction.execute(
                    "UPDATE team_names SET reservation_token = NULL, expires_at = NULL,
                     team_id = ?1 WHERE normalized_name = ?2 AND reservation_token = ?3
                     AND reservation_sequence = ?4 AND expires_at = ?5 AND expires_at > ?6
                     AND team_id IS NULL",
                    params![
                        mutation.team_id,
                        name,
                        token,
                        sql_integer(header.name_sequence)?,
                        sql_integer(expiry)?,
                        sql_integer(mutation.now)?
                    ],
                )? != 1
                {
                    return Err(Error::Reservation);
                }
            }
            transaction.execute(
                "INSERT INTO teams
                 (team_id, team_kind, host_id, normalized_name, team_name_utf8,
                  team_name_sequence, team_name_commitment_key, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    mutation.team_id,
                    i64::from(header.kind),
                    header.host_id,
                    header.normalized_name,
                    header.team_name_utf8,
                    sql_integer(header.name_sequence)?,
                    header.name_commitment_key.map(|key| key.as_slice()),
                    sql_integer(mutation.now)?
                ],
            )?;
            (header.kind, header.host_id.to_vec())
        } else {
            let head: Option<(i64, Vec<u8>, i64, Vec<u8>)> = transaction
                .query_row(
                    "SELECT h.seqno, h.link_hash, t.team_kind, t.host_id
                     FROM team_chain_heads h JOIN teams t ON t.team_id = h.team_id
                     WHERE h.team_id = ?1",
                    [mutation.team_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()?;
            let Some((sequence, tail, kind, host)) = head else {
                return Err(Error::Invalid("unknown team mutation authority"));
            };
            if crate::error::unsigned(sequence)?.saturating_add(1) != mutation.expected_sequence
                || mutation
                    .expected_tail_hash
                    .is_none_or(|expected| tail.as_slice() != expected)
            {
                return Err(Error::StaleRoot);
            }
            let kind = u8::try_from(kind).map_err(|_| Error::Invalid("stored team kind"))?;
            if kind == foks_proto::ENTITY_AD_HOC_TEAM {
                return Err(Error::Invalid("ad-hoc teams are immutable"));
            }
            (kind, host)
        };
        if mutation.team_id.first() != Some(&team_kind)
            || team_host.first() != Some(&foks_proto::ENTITY_HOST)
            || (team_kind == foks_proto::ENTITY_AD_HOC_TEAM && mutation.members.len() != 1)
        {
            return Err(Error::Invalid("team identity or founding roster"));
        }
        for member in mutation.members {
            if member.party_id.first() != Some(&foks_proto::ENTITY_USER)
                || member.scoped_host_id.is_some_and(|host| host != team_host)
                || transaction
                    .query_row(
                        "SELECT 1 FROM users WHERE uid = ?1",
                        [member.party_id],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_none()
            {
                return Err(Error::Invalid("non-local team member"));
            }
        }

        transaction.execute(
            "INSERT INTO team_chain_links(team_id, seqno, link_hash, exact_link, root_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                mutation.team_id,
                sql_integer(mutation.expected_sequence)?,
                mutation.link_hash,
                mutation.exact_link,
                sql_integer(mutation.root_epoch)?
            ],
        )?;
        transaction.execute(
            "INSERT INTO team_chain_heads(team_id, seqno, link_hash) VALUES (?1, ?2, ?3)
             ON CONFLICT(team_id) DO UPDATE SET seqno = excluded.seqno,
                 link_hash = excluded.link_hash",
            params![
                mutation.team_id,
                sql_integer(mutation.expected_sequence)?,
                mutation.link_hash
            ],
        )?;
        transaction.execute(
            "INSERT INTO team_tree_locations(team_id, seqno, location) VALUES (?1, ?2, ?3)",
            params![
                mutation.team_id,
                sql_integer(mutation.expected_sequence)?,
                mutation.next_tree_location
            ],
        )?;
        inject(failure, TeamMutationFailurePoint::Chain)?;

        transaction.execute(
            "DELETE FROM team_members WHERE team_id = ?1",
            [mutation.team_id],
        )?;
        for member in mutation.members {
            transaction.execute(
                "INSERT INTO team_members
                 (team_id, party_id, scoped_host_id, source_role_type, source_visibility,
                  role_type, visibility, generation, verify_key, hepk_fingerprint,
                  removal_key_commitment)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    mutation.team_id,
                    member.party_id,
                    member.scoped_host_id,
                    sql_integer(member.source_role_type)?,
                    member.source_visibility,
                    sql_integer(member.role_type)?,
                    member.visibility,
                    sql_integer(member.generation)?,
                    member.verify_key,
                    member.hepk_fingerprint,
                    member.removal_key_commitment.map(|value| value.as_slice())
                ],
            )?;
        }
        for key in mutation.shared_keys {
            insert_exact(
                &transaction,
                "INSERT INTO team_shared_keys
                 (team_id, role_type, visibility, generation, verify_key, exact_hepk)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(team_id, role_type, visibility, generation) DO UPDATE SET
                   verify_key = excluded.verify_key, exact_hepk = excluded.exact_hepk
                 WHERE verify_key = excluded.verify_key AND exact_hepk = excluded.exact_hepk",
                params![
                    mutation.team_id,
                    sql_integer(key.role_type)?,
                    key.visibility,
                    sql_integer(key.generation)?,
                    key.verify_key,
                    key.exact_hepk
                ],
                "conflicting team shared key",
            )?;
        }
        for parcel in mutation.parcels {
            insert_exact(
                &transaction,
                "INSERT INTO team_parcels
                 (team_id, party_id, sender_id, role_type, visibility, generation, exact_parcel)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(team_id, party_id, role_type, visibility, generation) DO UPDATE SET
                   sender_id = excluded.sender_id, exact_parcel = excluded.exact_parcel
                 WHERE sender_id = excluded.sender_id AND exact_parcel = excluded.exact_parcel",
                params![
                    mutation.team_id,
                    parcel.party_id,
                    parcel.sender_id,
                    sql_integer(parcel.role_type)?,
                    parcel.visibility,
                    sql_integer(parcel.generation)?,
                    parcel.exact_parcel
                ],
                "conflicting team parcel",
            )?;
        }
        for boxed in mutation.seed_chain {
            insert_exact(
                &transaction,
                "INSERT INTO team_seed_chain_boxes
                 (team_id, role_type, visibility, generation, exact_box)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(team_id, role_type, visibility, generation) DO UPDATE SET
                   exact_box = excluded.exact_box WHERE exact_box = excluded.exact_box",
                params![
                    mutation.team_id,
                    sql_integer(boxed.role_type)?,
                    boxed.visibility,
                    sql_integer(boxed.generation)?,
                    boxed.exact_box
                ],
                "conflicting team seed-chain box",
            )?;
        }
        for boxed in mutation.removal_boxes {
            insert_exact(
                &transaction,
                "INSERT INTO team_removal_boxes
                 (team_id, member_id, member_host_id, source_role_type,
                  source_visibility, exact_box) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(team_id, member_id, member_host_id, source_role_type,
                             source_visibility) DO UPDATE SET exact_box = excluded.exact_box
                 WHERE exact_box = excluded.exact_box",
                params![
                    mutation.team_id,
                    boxed.member_id,
                    boxed.member_host_id,
                    sql_integer(boxed.source_role_type)?,
                    boxed.source_visibility,
                    boxed.exact_box
                ],
                "conflicting team removal box",
            )?;
        }
        inject(failure, TeamMutationFailurePoint::Projection)?;

        publish_merkle(&transaction, mutation, failure)?;
        inject(failure, TeamMutationFailurePoint::MerkleRoot)?;
        receipts::insert(
            &transaction,
            mutation.idempotency_key,
            mutation.request_hash,
            mutation.response,
            mutation.now,
            mutation.receipt_expires_at,
        )?;
        inject(failure, TeamMutationFailurePoint::Receipt)?;
        transaction.commit()?;
        Ok(mutation.response.to_vec())
    }
}

fn publish_merkle(
    transaction: &rusqlite::Transaction<'_>,
    mutation: &TeamMutation<'_>,
    failure: Option<TeamMutationFailurePoint>,
) -> Result<()> {
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
    inject(failure, TeamMutationFailurePoint::MerkleNodes)?;
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

fn insert_exact(
    transaction: &rusqlite::Transaction<'_>,
    sql: &str,
    parameters: impl rusqlite::Params,
    error: &'static str,
) -> Result<()> {
    if transaction.execute(sql, parameters)? != 1 {
        return Err(Error::Invalid(error));
    }
    Ok(())
}

fn validate(database: &Database, mutation: &TeamMutation<'_>) -> Result<()> {
    if mutation.team_id.len() != 33
        || mutation.signer_credential_id.len() < 33
        || mutation.expected_sequence == 0
        || mutation.expected_sequence > database.config.maximum_team_chain_links
        || mutation.exact_link.is_empty()
        || mutation.exact_link.len() > database.config.maximum_blob_bytes
        || mutation.members.is_empty()
        || mutation.members.len() > database.config.maximum_team_members
        || mutation.shared_keys.len() > database.config.maximum_team_role_bands
        || mutation.parcels.len() > database.config.maximum_boxes_per_mutation
        || mutation.seed_chain.len() > database.config.maximum_boxes_per_mutation
        || mutation.removal_boxes.len() > database.config.maximum_boxes_per_mutation
        || mutation.merkle_commit.nodes.len() > database.config.maximum_merkle_nodes_per_commit
        || mutation.back_pointers.len() > database.config.maximum_back_pointers
        || mutation.receipt_expires_at <= mutation.now
        || mutation.response.len() > database.config.maximum_receipt_bytes
    {
        return Err(Error::Invalid("team mutation"));
    }
    if mutation.back_pointers.iter().map(|(epoch, _)| *epoch).ne(
        foks_merkle_store::back_pointer_sequence(mutation.root_epoch),
    ) {
        return Err(Error::Invalid("Merkle back-pointer sequence"));
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

fn inject(
    requested: Option<TeamMutationFailurePoint>,
    current: TeamMutationFailurePoint,
) -> Result<()> {
    if requested == Some(current) {
        Err(Error::TeamMutationInjected(current))
    } else {
        Ok(())
    }
}
