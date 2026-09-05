use rusqlite::{params, OptionalExtension};

use crate::*;

impl HardStateStore {
    /// Atomically pins a verified team chain and its public roster/PTK
    /// projection. PTK seeds and other private material are never stored.
    pub fn accept_verified_team(&mut self, snapshot: &VerifiedTeamSnapshot) -> Result<Acceptance> {
        self.accept_team_parts(snapshot.parts())
    }

    pub(crate) fn accept_team_parts(
        &mut self,
        snapshot: VerifiedTeamSnapshotParts<'_>,
    ) -> Result<Acceptance> {
        validate_team_snapshot(snapshot)?;
        let chain_seqno = sqlite_integer("team chain sequence", snapshot.chain_seqno)?;
        let merkle_epoch = sqlite_integer("team Merkle epoch", snapshot.merkle_epoch)?;
        let mut members = snapshot.members.to_vec();
        members.sort();
        let mut shared_keys = snapshot.shared_keys.to_vec();
        shared_keys.sort();
        if members.windows(2).any(|pair| {
            pair[0].party_id == pair[1].party_id
                && pair[0].scoped_host_id == pair[1].scoped_host_id
                && pair[0].source_role == pair[1].source_role
        }) || shared_keys
            .windows(2)
            .any(|pair| pair[0].role == pair[1].role)
        {
            return Err(Error::InvalidTeam(
                "member identities and current PTK roles must be unique",
            ));
        }
        let transaction = self.write_transaction()?;
        let host_exists = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM hosts WHERE host_id = ?1)",
            [snapshot.host_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !host_exists {
            return Err(Error::UnknownHost);
        }
        let current_root = transaction
            .query_row(
                "SELECT epoch, root_hash FROM merkle_heads WHERE host_id = ?1",
                [snapshot.host_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
            )
            .optional()?;
        let Some((current_epoch, current_hash)) = current_root else {
            return Err(Error::InvalidTeam(
                "team projection host has no accepted Merkle head",
            ));
        };
        let current_epoch = stored_unsigned("current Merkle epoch", current_epoch)?;
        match snapshot.merkle_epoch.cmp(&current_epoch) {
            std::cmp::Ordering::Less => {
                return Err(Error::MerkleRollback {
                    stored: current_epoch,
                    received: snapshot.merkle_epoch,
                });
            }
            std::cmp::Ordering::Equal if current_hash.as_slice() != snapshot.merkle_root_hash => {
                return Err(Error::MerkleFork {
                    epoch: snapshot.merkle_epoch,
                });
            }
            std::cmp::Ordering::Equal => {}
            std::cmp::Ordering::Greater => {
                return Err(Error::InvalidTeam(
                    "team projection is ahead of the accepted Merkle head",
                ));
            }
        }
        let stored = load_team_snapshot(&transaction, snapshot.host_id, snapshot.team_id)?;
        let acceptance = match &stored {
            None => Acceptance::Inserted,
            Some(stored) if snapshot.chain_seqno < stored.chain_seqno => {
                return Err(Error::TeamRollback {
                    stored: stored.chain_seqno,
                    received: snapshot.chain_seqno,
                });
            }
            Some(stored) if snapshot.chain_seqno == stored.chain_seqno => {
                if stored.chain_tail_hash != snapshot.chain_tail_hash
                    || stored.chain_bytes != snapshot.chain_bytes
                {
                    return Err(Error::TeamFork {
                        seqno: snapshot.chain_seqno,
                    });
                }
                if stored.team_name != snapshot.team_name
                    || stored.team_name_utf8 != snapshot.team_name_utf8
                    || stored.team_name_sequence != snapshot.team_name_sequence
                    || stored.index_range != *snapshot.index_range
                    || stored.members != members
                    || stored.shared_keys != shared_keys
                {
                    return Err(Error::TeamProjectionChanged {
                        seqno: snapshot.chain_seqno,
                    });
                }
                match snapshot.merkle_epoch.cmp(&stored.merkle_epoch) {
                    std::cmp::Ordering::Less => {
                        return Err(Error::MerkleRollback {
                            stored: stored.merkle_epoch,
                            received: snapshot.merkle_epoch,
                        });
                    }
                    std::cmp::Ordering::Equal => {
                        if stored.merkle_root_hash != snapshot.merkle_root_hash
                            || stored.merkle_root_bytes != snapshot.merkle_root_bytes
                        {
                            return Err(Error::MerkleFork {
                                epoch: snapshot.merkle_epoch,
                            });
                        }
                        Acceptance::Unchanged
                    }
                    std::cmp::Ordering::Greater => Acceptance::Advanced,
                }
            }
            Some(stored) => {
                if !encoded_array_is_prefix(&stored.chain_bytes, snapshot.chain_bytes)? {
                    return Err(Error::TeamFork {
                        seqno: stored.chain_seqno.saturating_add(1),
                    });
                }
                if snapshot.merkle_epoch < stored.merkle_epoch {
                    return Err(Error::MerkleRollback {
                        stored: stored.merkle_epoch,
                        received: snapshot.merkle_epoch,
                    });
                }
                Acceptance::Advanced
            }
        };
        if acceptance != Acceptance::Unchanged {
            transaction.execute(
                "INSERT INTO teams (host_id, team_id, chain_seqno, chain_tail_hash, chain_bytes, \
                 evidence_bytes, team_name, team_name_utf8, team_name_sequence, index_low_infinity, \
                 index_low_base, index_low_exponent, index_high_infinity, index_high_base, \
                 index_high_exponent, merkle_epoch, merkle_root_hash, merkle_root_bytes) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, \
                 ?16, ?17, ?18) ON CONFLICT(host_id, team_id) DO UPDATE SET \
                 chain_seqno = excluded.chain_seqno, chain_tail_hash = excluded.chain_tail_hash, \
                 chain_bytes = excluded.chain_bytes, evidence_bytes = excluded.evidence_bytes, \
                 team_name = excluded.team_name, team_name_utf8 = excluded.team_name_utf8, \
                 team_name_sequence = excluded.team_name_sequence, \
                 index_low_infinity = excluded.index_low_infinity, \
                 index_low_base = excluded.index_low_base, \
                 index_low_exponent = excluded.index_low_exponent, \
                 index_high_infinity = excluded.index_high_infinity, \
                 index_high_base = excluded.index_high_base, \
                 index_high_exponent = excluded.index_high_exponent, \
                 merkle_epoch = excluded.merkle_epoch, merkle_root_hash = excluded.merkle_root_hash, \
                 merkle_root_bytes = excluded.merkle_root_bytes",
                params![
                    snapshot.host_id,
                    snapshot.team_id,
                    chain_seqno,
                    snapshot.chain_tail_hash.as_slice(),
                    snapshot.chain_bytes,
                    snapshot.evidence_bytes,
                    snapshot.team_name,
                    snapshot.team_name_utf8,
                    sqlite_integer("team-name sequence", snapshot.team_name_sequence)?,
                    snapshot.index_range.low.infinity,
                    snapshot.index_range.low.base.as_slice(),
                    snapshot.index_range.low.exponent,
                    snapshot.index_range.high.infinity,
                    snapshot.index_range.high.base.as_slice(),
                    snapshot.index_range.high.exponent,
                    merkle_epoch,
                    snapshot.merkle_root_hash.as_slice(),
                    snapshot.merkle_root_bytes,
                ],
            )?;
            transaction.execute(
                "DELETE FROM team_members WHERE host_id = ?1 AND team_id = ?2",
                params![snapshot.host_id, snapshot.team_id],
            )?;
            transaction.execute(
                "DELETE FROM team_shared_keys WHERE host_id = ?1 AND team_id = ?2",
                params![snapshot.host_id, snapshot.team_id],
            )?;
            for member in &members {
                let (
                    range_present,
                    low_infinity,
                    low_base,
                    low_exponent,
                    high_infinity,
                    high_base,
                    high_exponent,
                ) = match &member.index_range {
                    Some(range) => (
                        true,
                        range.low.infinity,
                        range.low.base.as_slice(),
                        range.low.exponent,
                        range.high.infinity,
                        range.high.base.as_slice(),
                        range.high.exponent,
                    ),
                    None => (false, false, &[][..], 0, false, &[][..], 0),
                };
                transaction.execute(
                    "INSERT INTO team_members (host_id, team_id, party_id, scoped_host_id, \
                     source_role_type, source_role_visibility, role_type, role_visibility, \
                     generation, verify_key, hepk_fingerprint, removal_key_commitment, \
                     index_range_present, index_low_infinity, index_low_base, index_low_exponent, \
                     index_high_infinity, index_high_base, index_high_exponent) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, \
                     ?15, ?16, ?17, ?18, ?19)",
                    params![
                        snapshot.host_id,
                        snapshot.team_id,
                        member.party_id,
                        member.scoped_host_id.as_deref().unwrap_or_default(),
                        sqlite_integer(
                            "team member source role",
                            member.source_role.protocol_value()
                        )?,
                        i64::from(member.source_role.visibility().unwrap_or(0)),
                        sqlite_integer("team member role", member.role.protocol_value())?,
                        i64::from(member.role.visibility().unwrap_or(0)),
                        sqlite_integer("team member generation", member.generation)?,
                        member.verify_key,
                        member.hepk_fingerprint.as_slice(),
                        member
                            .removal_key_commitment
                            .as_ref()
                            .map_or(&[][..], |commitment| commitment.as_slice()),
                        range_present,
                        low_infinity,
                        low_base,
                        low_exponent,
                        high_infinity,
                        high_base,
                        high_exponent,
                    ],
                )?;
            }
            for key in &shared_keys {
                transaction.execute(
                    "INSERT INTO team_shared_keys (host_id, team_id, role_type, role_visibility, \
                     generation, verify_key, hepk_bytes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        snapshot.host_id,
                        snapshot.team_id,
                        sqlite_integer("PTK role", key.role.protocol_value())?,
                        i64::from(key.role.visibility().unwrap_or(0)),
                        sqlite_integer("PTK generation", key.generation)?,
                        key.verify_key,
                        key.hepk_bytes,
                    ],
                )?;
            }
        }
        transaction.commit()?;
        Ok(acceptance)
    }
    /// Loads an untrusted persisted team projection. Re-authenticate it with
    /// `foks_verify::restore_verified_team` before use.
    pub fn team_for_host(
        &self,
        host_id: &[u8],
        team_id: &[u8],
    ) -> Result<Option<StoredTeamSnapshot>> {
        load_team_snapshot(&self.connection, host_id, team_id)
    }
}
