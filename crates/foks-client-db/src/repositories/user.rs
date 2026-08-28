use rusqlite::{params, OptionalExtension};

use crate::*;

impl HardStateStore {
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
        let accepted_root = transaction
            .query_row(
                "SELECT root_hash FROM merkle_roots \
                 WHERE host_id = ?1 AND epoch = ?2",
                params![snapshot.host_id, merkle_epoch],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        if accepted_root.as_deref() != Some(snapshot.merkle_root_hash.as_slice()) {
            return Err(Error::InvalidUser(
                "user projection is not bound to an accepted Merkle root",
            ));
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
                        || username_utf8 != snapshot.username_utf8
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
                            Acceptance::Unchanged
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
}
