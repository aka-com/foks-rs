use rusqlite::{params, OptionalExtension};

use crate::*;

type StoredGenericChain = (i64, Option<Vec<u8>>, Vec<u8>, i64, Vec<u8>);

impl HardStateStore {
    pub fn accept_verified_user_generic_chain(
        &mut self,
        snapshot: &VerifiedUserGenericChainSnapshot<'_>,
    ) -> Result<Acceptance> {
        if snapshot.host_id.len() != 33
            || snapshot.uid.len() != 33
            || !matches!(snapshot.chain_type, 2 | 4)
            || snapshot.tail_hash.is_some() != (snapshot.sequence > 0)
        {
            return Err(Error::InvalidUser("invalid generic-chain snapshot"));
        }
        let Value::Array(chain) = decode(snapshot.chain_bytes)? else {
            return Err(Error::InvalidUser("generic-chain evidence is not an array"));
        };
        let Some((Value::Unsigned(encoded_type), links)) = chain.split_first() else {
            return Err(Error::InvalidUser(
                "generic-chain evidence omitted its type",
            ));
        };
        if *encoded_type != snapshot.chain_type
            || links.len()
                != usize::try_from(snapshot.sequence).map_err(|_| Error::IntegerOutOfRange {
                    field: "generic-chain sequence",
                    value: snapshot.sequence,
                })?
        {
            return Err(Error::InvalidUser(
                "generic-chain evidence length does not match its sequence",
            ));
        }
        let chain_type = sqlite_integer("generic-chain type", snapshot.chain_type)?;
        let sequence = sqlite_integer("generic-chain sequence", snapshot.sequence)?;
        let merkle_epoch = sqlite_integer("generic-chain Merkle epoch", snapshot.merkle_epoch)?;
        let transaction = self.write_transaction()?;
        let authority: Option<(i64, Vec<u8>)> = transaction
            .query_row(
                "SELECT h.epoch, r.root_hash FROM merkle_heads h
                 JOIN merkle_roots r ON r.host_id = h.host_id AND r.epoch = h.epoch
                 JOIN users u ON u.host_id = h.host_id
                 WHERE h.host_id = ?1 AND u.uid = ?2",
                params![snapshot.host_id, snapshot.uid],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((current_epoch, current_hash)) = authority else {
            return Err(Error::InvalidUser(
                "generic chain has no accepted user authority",
            ));
        };
        if stored_unsigned("generic-chain Merkle epoch", current_epoch)? != snapshot.merkle_epoch
            || current_hash.as_slice() != snapshot.merkle_root_hash
        {
            return Err(Error::InvalidUser(
                "generic chain is not anchored at the current Merkle head",
            ));
        }
        let stored: Option<StoredGenericChain> = transaction
            .query_row(
                "SELECT seqno, tail_hash, chain_bytes, merkle_epoch, merkle_root_hash
                 FROM user_generic_chains
                 WHERE host_id = ?1 AND uid = ?2 AND chain_type = ?3",
                params![snapshot.host_id, snapshot.uid, chain_type],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;
        let acceptance = match stored {
            None => Acceptance::Inserted,
            Some((stored_sequence, stored_tail, stored_chain, stored_epoch, stored_root)) => {
                let stored_sequence = stored_unsigned("generic-chain sequence", stored_sequence)?;
                if snapshot.sequence < stored_sequence {
                    return Err(Error::UserGenericRollback {
                        chain_type: snapshot.chain_type,
                        stored: stored_sequence,
                        received: snapshot.sequence,
                    });
                }
                if snapshot.sequence == stored_sequence {
                    if stored_tail.as_deref()
                        != snapshot.tail_hash.as_ref().map(<[u8; 32]>::as_slice)
                        || stored_chain != snapshot.chain_bytes
                    {
                        return Err(Error::UserGenericFork {
                            chain_type: snapshot.chain_type,
                            seqno: snapshot.sequence,
                        });
                    }
                    if stored_unsigned("generic-chain Merkle epoch", stored_epoch)?
                        == snapshot.merkle_epoch
                        && stored_root.as_slice() == snapshot.merkle_root_hash
                    {
                        Acceptance::Unchanged
                    } else {
                        Acceptance::Advanced
                    }
                } else {
                    if !encoded_array_is_prefix(&stored_chain, snapshot.chain_bytes)? {
                        return Err(Error::UserGenericFork {
                            chain_type: snapshot.chain_type,
                            seqno: stored_sequence.saturating_add(1),
                        });
                    }
                    Acceptance::Advanced
                }
            }
        };
        if acceptance != Acceptance::Unchanged {
            transaction.execute(
                "INSERT INTO user_generic_chains
                     (host_id, uid, chain_type, seqno, tail_hash, chain_bytes,
                      merkle_epoch, merkle_root_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(host_id, uid, chain_type) DO UPDATE SET
                     seqno = excluded.seqno,
                     tail_hash = excluded.tail_hash,
                     chain_bytes = excluded.chain_bytes,
                     merkle_epoch = excluded.merkle_epoch,
                     merkle_root_hash = excluded.merkle_root_hash",
                params![
                    snapshot.host_id,
                    snapshot.uid,
                    chain_type,
                    sequence,
                    snapshot.tail_hash.as_ref().map(<[u8; 32]>::as_slice),
                    snapshot.chain_bytes,
                    merkle_epoch,
                    snapshot.merkle_root_hash.as_slice(),
                ],
            )?;
        }
        if snapshot.chain_type == foks_proto::CHAIN_TYPE_USER_SETTINGS && snapshot.sequence > 0 {
            transaction.execute(
                "DELETE FROM user_local_security
                 WHERE host_id = ?1 AND uid = ?2 AND passphrase_absence_attested = 1",
                params![snapshot.host_id, snapshot.uid],
            )?;
        }
        transaction.commit()?;
        Ok(acceptance)
    }

    /// Records that this client created the account from an exact signup
    /// request with no passphrase. Server responses alone cannot establish
    /// this fact for legacy Go accounts, whose initial PPE was not linked.
    pub fn attest_user_has_no_passphrase(&mut self, host_id: &[u8], uid: &[u8]) -> Result<()> {
        if uid.len() != 33 {
            return Err(Error::InvalidUser("invalid passphrase-attestation UID"));
        }
        let transaction = self.write_transaction()?;
        let inserted = transaction.execute(
            "INSERT INTO user_local_security
                 (host_id, uid, passphrase_absence_attested, trusted_ppe_hash)
             SELECT ?1, ?2, 1, NULL
             WHERE EXISTS (
                 SELECT 1 FROM users WHERE host_id = ?1 AND uid = ?2
             )
             ON CONFLICT(host_id, uid) DO UPDATE SET
                 passphrase_absence_attested = 1,
                 trusted_ppe_hash = NULL",
            params![host_id, uid],
        )?;
        if inserted == 0 {
            return Err(Error::InvalidUser(
                "passphrase attestation has no accepted user",
            ));
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn user_has_no_passphrase_attestation(&self, host_id: &[u8], uid: &[u8]) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM user_local_security
                 WHERE host_id = ?1 AND uid = ?2
                    AND passphrase_absence_attested = 1
             )",
            params![host_id, uid],
            |row| row.get(0),
        )?)
    }

    pub fn clear_user_no_passphrase_attestation(
        &mut self,
        host_id: &[u8],
        uid: &[u8],
    ) -> Result<()> {
        let transaction = self.write_transaction()?;
        transaction.execute(
            "DELETE FROM user_local_security WHERE host_id = ?1 AND uid = ?2",
            params![host_id, uid],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn trust_user_passphrase_parcel(
        &mut self,
        host_id: &[u8],
        uid: &[u8],
        parcel_hash: &[u8; 32],
    ) -> Result<()> {
        if uid.len() != 33 {
            return Err(Error::InvalidUser("invalid trusted-PPE UID"));
        }
        let transaction = self.write_transaction()?;
        let inserted = transaction.execute(
            "INSERT INTO user_local_security
                 (host_id, uid, passphrase_absence_attested, trusted_ppe_hash)
             SELECT ?1, ?2, 0, ?3
             WHERE EXISTS (
                 SELECT 1 FROM users WHERE host_id = ?1 AND uid = ?2
             )
             ON CONFLICT(host_id, uid) DO UPDATE SET
                 passphrase_absence_attested = 0,
                 trusted_ppe_hash = excluded.trusted_ppe_hash",
            params![host_id, uid, parcel_hash.as_slice()],
        )?;
        if inserted == 0 {
            return Err(Error::InvalidUser("trusted PPE has no accepted user"));
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn trusted_user_passphrase_parcel_hash(
        &self,
        host_id: &[u8],
        uid: &[u8],
    ) -> Result<Option<[u8; 32]>> {
        self.connection
            .query_row(
                "SELECT trusted_ppe_hash FROM user_local_security
                 WHERE host_id = ?1 AND uid = ?2
                   AND passphrase_absence_attested = 0",
                params![host_id, uid],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?
            .map(|hash| {
                hash.try_into()
                    .map_err(|_| Error::InvalidUser("invalid trusted PPE hash"))
            })
            .transpose()
    }

    /// Atomically pins a verified user chain, its authenticating Merkle root,
    /// and the public device/PUK projection. No private key material is stored.
    pub fn accept_verified_user(&mut self, snapshot: &VerifiedUserSnapshot) -> Result<Acceptance> {
        let snapshot = snapshot.parts();
        self.accept_user_parts(snapshot)
    }

    pub(crate) fn accept_user_parts(
        &mut self,
        snapshot: VerifiedUserSnapshotParts<'_>,
    ) -> Result<Acceptance> {
        validate_user_snapshot(snapshot)?;
        let chain_seqno = sqlite_integer("user chain sequence", snapshot.chain_seqno)?;
        let merkle_epoch = sqlite_integer("user Merkle epoch", snapshot.merkle_epoch)?;
        let mut devices = snapshot.devices.to_vec();
        devices.sort();
        let mut shared_keys = snapshot.shared_keys.to_vec();
        shared_keys.sort();
        if devices
            .windows(2)
            .any(|pair| pair[0].device_id == pair[1].device_id)
            || shared_keys
                .windows(2)
                .any(|pair| pair[0].role == pair[1].role)
        {
            return Err(Error::InvalidUser(
                "device IDs and current shared-key roles must be unique",
            ));
        }
        let transaction = self.write_transaction()?;
        let host_exists = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM hosts WHERE host_id = ?1)",
            [&snapshot.host_id],
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
            return Err(Error::InvalidUser(
                "user projection host has no accepted Merkle head",
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
                return Err(Error::InvalidUser(
                    "user projection is ahead of the accepted Merkle head",
                ));
            }
        }
        let stored = transaction
            .query_row(
                "SELECT chain_seqno, chain_tail_hash, chain_bytes, evidence_bytes, username, \
                 username_utf8, username_sequence, merkle_epoch, merkle_root_hash, \
                 merkle_root_bytes FROM users \
                 WHERE host_id = ?1 AND uid = ?2",
                params![snapshot.host_id, snapshot.uid],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        row.get::<_, Vec<u8>>(4)?,
                        row.get::<_, Vec<u8>>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, Vec<u8>>(8)?,
                        row.get::<_, Vec<u8>>(9)?,
                    ))
                },
            )
            .optional()?;
        let acceptance = match &stored {
            None => Acceptance::Inserted,
            Some((
                stored_seqno,
                tail,
                chain,
                _evidence,
                username,
                username_utf8,
                username_sequence,
                epoch,
                root_hash,
                root_bytes,
            )) => {
                let stored_seqno = stored_unsigned("user chain sequence", *stored_seqno)?;
                let stored_epoch = stored_unsigned("user Merkle epoch", *epoch)?;
                if snapshot.chain_seqno < stored_seqno {
                    return Err(Error::UserRollback {
                        stored: stored_seqno,
                        received: snapshot.chain_seqno,
                    });
                }
                if snapshot.chain_seqno == stored_seqno {
                    if tail.as_slice() != snapshot.chain_tail_hash || chain != snapshot.chain_bytes
                    {
                        return Err(Error::UserFork {
                            seqno: snapshot.chain_seqno,
                        });
                    }
                    if username != snapshot.username
                        || stored_unsigned("username sequence", *username_sequence)?
                            != snapshot.username_sequence
                        || load_user_devices(&transaction, snapshot.host_id, snapshot.uid)?
                            != stored_devices(&devices)
                        || load_user_shared_keys(&transaction, snapshot.host_id, snapshot.uid)?
                            != stored_shared_keys(&shared_keys)
                    {
                        return Err(Error::UserProjectionChanged {
                            seqno: snapshot.chain_seqno,
                        });
                    }
                    match snapshot.merkle_epoch.cmp(&stored_epoch) {
                        std::cmp::Ordering::Less => {
                            return Err(Error::MerkleRollback {
                                stored: stored_epoch,
                                received: snapshot.merkle_epoch,
                            });
                        }
                        std::cmp::Ordering::Equal => {
                            if root_hash.as_slice() != snapshot.merkle_root_hash
                                || root_bytes != snapshot.merkle_root_bytes
                            {
                                return Err(Error::MerkleFork {
                                    epoch: snapshot.merkle_epoch,
                                });
                            }
                            if username_utf8 != snapshot.username_utf8 {
                                Acceptance::Advanced
                            } else {
                                Acceptance::Unchanged
                            }
                        }
                        std::cmp::Ordering::Greater => Acceptance::Advanced,
                    }
                } else {
                    if !encoded_array_is_prefix(chain, snapshot.chain_bytes)? {
                        return Err(Error::UserFork {
                            seqno: stored_seqno.saturating_add(1),
                        });
                    }
                    if snapshot.merkle_epoch < stored_epoch {
                        return Err(Error::MerkleRollback {
                            stored: stored_epoch,
                            received: snapshot.merkle_epoch,
                        });
                    }
                    Acceptance::Advanced
                }
            }
        };

        if acceptance != Acceptance::Unchanged {
            transaction.execute(
                "INSERT INTO users (host_id, uid, chain_seqno, chain_tail_hash, chain_bytes, \
                 evidence_bytes, username, username_utf8, username_sequence, merkle_epoch, \
                 merkle_root_hash, merkle_root_bytes) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) \
                 ON CONFLICT(host_id, uid) DO UPDATE SET chain_seqno = excluded.chain_seqno, \
                 chain_tail_hash = excluded.chain_tail_hash, chain_bytes = excluded.chain_bytes, \
                 evidence_bytes = excluded.evidence_bytes, username = excluded.username, \
                 username_utf8 = excluded.username_utf8, \
                 username_sequence = excluded.username_sequence, \
                 merkle_epoch = excluded.merkle_epoch, merkle_root_hash = excluded.merkle_root_hash, \
                 merkle_root_bytes = excluded.merkle_root_bytes",
                params![
                    snapshot.host_id,
                    snapshot.uid,
                    chain_seqno,
                    snapshot.chain_tail_hash.as_slice(),
                    snapshot.chain_bytes,
                    snapshot.evidence_bytes,
                    snapshot.username,
                    snapshot.username_utf8,
                    sqlite_integer("username sequence", snapshot.username_sequence)?,
                    merkle_epoch,
                    snapshot.merkle_root_hash.as_slice(),
                    snapshot.merkle_root_bytes,
                ],
            )?;
            transaction.execute(
                "DELETE FROM user_devices WHERE host_id = ?1 AND uid = ?2",
                params![snapshot.host_id, snapshot.uid],
            )?;
            transaction.execute(
                "DELETE FROM user_shared_keys WHERE host_id = ?1 AND uid = ?2",
                params![snapshot.host_id, snapshot.uid],
            )?;
            for device in &devices {
                transaction.execute(
                    "INSERT INTO user_devices \
                     (host_id, uid, device_id, role_type, role_visibility, hepk_bytes, subkey_id) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        snapshot.host_id,
                        snapshot.uid,
                        device.device_id,
                        sqlite_integer("device role", device.role.protocol_value())?,
                        i64::from(device.role.visibility().unwrap_or(0)),
                        device.hepk_bytes,
                        device.subkey_id,
                    ],
                )?;
            }
            for shared_key in &shared_keys {
                transaction.execute(
                    "INSERT INTO user_shared_keys \
                     (host_id, uid, role_type, role_visibility, generation, verify_key, hepk_bytes) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        snapshot.host_id,
                        snapshot.uid,
                        sqlite_integer("PUK role", shared_key.role.protocol_value())?,
                        i64::from(shared_key.role.visibility().unwrap_or(0)),
                        sqlite_integer("PUK generation", shared_key.generation)?,
                        shared_key.verify_key,
                        shared_key.hepk_bytes,
                    ],
                )?;
            }
        }
        transaction.commit()?;
        Ok(acceptance)
    }
    /// Loads an untrusted persisted user projection. Callers must pass its
    /// `parts()` through `foks_verify::restore_verified_user` before use.
    pub fn user_for_host(&self, host_id: &[u8], uid: &[u8]) -> Result<Option<StoredUserSnapshot>> {
        load_user_snapshot(&self.connection, host_id, uid)
    }

    /// The generic chain pinned for this user, if one was accepted before.
    ///
    /// The row is the tail a later incremental load resumes from, so it is
    /// keyed by the exact host and uid the caller names: a row written for
    /// another identity is not reachable through this lookup, and a caller
    /// that cannot match the returned owner bindings must reload the chain in
    /// full rather than resume on it.
    pub fn user_generic_chain(
        &self,
        host_id: &[u8],
        uid: &[u8],
        chain_type: u64,
    ) -> Result<Option<StoredUserGenericChain>> {
        if host_id.len() != 33 || uid.len() != 33 {
            return Err(Error::InvalidUser("invalid generic-chain lookup"));
        }
        let chain_type_column = sqlite_integer("generic-chain type", chain_type)?;
        let row: Option<StoredGenericChain> = self
            .connection
            .query_row(
                "SELECT seqno, tail_hash, chain_bytes, merkle_epoch, merkle_root_hash
                 FROM user_generic_chains
                 WHERE host_id = ?1 AND uid = ?2 AND chain_type = ?3",
                params![host_id, uid, chain_type_column],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((sequence, tail_hash, chain_bytes, merkle_epoch, merkle_root_hash)) = row else {
            return Ok(None);
        };
        let sequence = stored_unsigned("generic-chain sequence", sequence)?;
        let tail_hash = tail_hash
            .map(|hash| {
                <[u8; 32]>::try_from(hash.as_slice())
                    .map_err(|_| Error::InvalidUser("stored generic-chain tail hash is invalid"))
            })
            .transpose()?;
        if tail_hash.is_some() != (sequence > 0) {
            return Err(Error::InvalidUser(
                "stored generic-chain tail does not match its sequence",
            ));
        }
        Ok(Some(StoredUserGenericChain {
            host_id: host_id.to_vec(),
            uid: uid.to_vec(),
            chain_type,
            sequence,
            tail_hash,
            chain_bytes,
            merkle_epoch: stored_unsigned("generic-chain Merkle epoch", merkle_epoch)?,
            merkle_root_hash: <[u8; 32]>::try_from(merkle_root_hash.as_slice())
                .map_err(|_| Error::InvalidUser("stored generic-chain root hash is invalid"))?,
        }))
    }
}
