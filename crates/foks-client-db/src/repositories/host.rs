use rusqlite::{params, OptionalExtension};

use crate::*;

impl HardStateStore {
    /// Atomically accepts a complete, already-verified host snapshot.
    ///
    /// A failed monotonicity check rolls back every part of the update,
    /// including the host chain when the accompanying Merkle root is stale.
    pub fn accept_verified_host(&mut self, snapshot: &VerifiedHostSnapshot) -> Result<Acceptance> {
        let snapshot = snapshot.parts();
        self.accept_host_parts(snapshot)
    }

    pub(crate) fn accept_host_parts(
        &mut self,
        snapshot: VerifiedHostSnapshotParts<'_>,
    ) -> Result<Acceptance> {
        validate_snapshot(snapshot)?;
        let chain_seqno = sqlite_integer("chain sequence", snapshot.chain_seqno)?;
        let merkle_epoch = sqlite_integer("Merkle epoch", snapshot.merkle_root.epoch)?;
        let mut service_types = snapshot
            .services
            .iter()
            .map(|service| sqlite_integer("service type", service.service_type.protocol_value()))
            .collect::<Result<Vec<_>>>()?;
        service_types.sort_unstable();
        if service_types.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(Error::InvalidSnapshot("service types must be unique"));
        }

        let transaction = self.write_transaction()?;

        let pinned_host_id = transaction
            .query_row(
                "SELECT host_id FROM host_lookups WHERE lookup_name = ?1",
                [&snapshot.lookup_name],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        if pinned_host_id
            .as_ref()
            .is_some_and(|host_id| host_id != snapshot.host_id)
        {
            return Err(Error::HostIdentityChanged {
                lookup_name: snapshot.lookup_name.to_owned(),
            });
        }

        let stored = load_host_row(&transaction, snapshot.host_id)?;
        let chain_advanced = match &stored {
            None => {
                transaction.execute(
                    "INSERT INTO hosts (host_id, canonical_name, genesis_key, chain_seqno, \
                     chain_tail_hash, chain_bytes, public_zone_bytes) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        snapshot.host_id,
                        snapshot.canonical_name,
                        snapshot.genesis_key,
                        chain_seqno,
                        snapshot.chain_tail_hash.as_slice(),
                        snapshot.chain_bytes,
                        snapshot.public_zone_bytes,
                    ],
                )?;
                true
            }
            Some(stored) => {
                if stored.genesis_key != snapshot.genesis_key {
                    return Err(Error::GenesisChanged);
                }
                match snapshot.chain_seqno.cmp(&stored.chain_seqno) {
                    std::cmp::Ordering::Less => {
                        return Err(Error::ChainRollback {
                            stored: stored.chain_seqno,
                            received: snapshot.chain_seqno,
                        });
                    }
                    std::cmp::Ordering::Equal => {
                        if stored.chain_tail_hash != snapshot.chain_tail_hash
                            || stored.chain_bytes != snapshot.chain_bytes
                        {
                            return Err(Error::ChainFork {
                                seqno: snapshot.chain_seqno,
                            });
                        }
                        let stored_services = load_services(&transaction, snapshot.host_id)?;
                        if stored.canonical_name != snapshot.canonical_name
                            || stored.public_zone_bytes != snapshot.public_zone_bytes
                            || stored_services != normalized_services(snapshot.services)
                        {
                            return Err(Error::ProjectionChanged {
                                seqno: snapshot.chain_seqno,
                            });
                        }
                        false
                    }
                    std::cmp::Ordering::Greater => {
                        if !encoded_array_is_prefix(&stored.chain_bytes, snapshot.chain_bytes)? {
                            return Err(Error::ChainFork {
                                seqno: stored.chain_seqno.saturating_add(1),
                            });
                        }
                        transaction.execute(
                            "UPDATE hosts SET canonical_name = ?2, genesis_key = ?3, \
                             chain_seqno = ?4, chain_tail_hash = ?5, chain_bytes = ?6, \
                             public_zone_bytes = ?7 \
                             WHERE host_id = ?1",
                            params![
                                snapshot.host_id,
                                snapshot.canonical_name,
                                snapshot.genesis_key,
                                chain_seqno,
                                snapshot.chain_tail_hash.as_slice(),
                                snapshot.chain_bytes,
                                snapshot.public_zone_bytes,
                            ],
                        )?;
                        true
                    }
                }
            }
        };

        transaction.execute(
            "INSERT OR IGNORE INTO host_lookups (lookup_name, host_id) VALUES (?1, ?2)",
            params![snapshot.lookup_name, snapshot.host_id],
        )?;

        if chain_advanced {
            transaction.execute(
                "DELETE FROM host_services WHERE host_id = ?1",
                [&snapshot.host_id],
            )?;
            for service in snapshot.services {
                transaction.execute(
                    "INSERT INTO host_services \
                     (host_id, service_type, endpoint_bytes, valid_at_chain_seqno) \
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        snapshot.host_id,
                        sqlite_integer("service type", service.service_type.protocol_value())?,
                        service.endpoint_bytes,
                        chain_seqno,
                    ],
                )?;
            }
        }

        let merkle_advanced = accept_merkle_root(
            &transaction,
            snapshot.host_id,
            snapshot.merkle_root,
            merkle_epoch,
        )?;
        transaction.commit()?;

        Ok(if stored.is_none() {
            Acceptance::Inserted
        } else if chain_advanced || merkle_advanced {
            Acceptance::Advanced
        } else {
            Acceptance::Unchanged
        })
    }
    /// Advances only the already-verified Merkle head for a pinned host.
    pub fn accept_verified_merkle_root(
        &mut self,
        host_id: &[u8],
        root: &VerifiedMerkleRoot,
    ) -> Result<Acceptance> {
        let root = root.parts();
        let epoch = sqlite_integer("Merkle epoch", root.epoch)?;
        let transaction = self.write_transaction()?;
        let host_exists = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM hosts WHERE host_id = ?1)",
            [host_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !host_exists {
            return Err(Error::UnknownHost);
        }
        let advanced = accept_merkle_root(&transaction, host_id, root, epoch)?;
        transaction.commit()?;
        Ok(if advanced {
            Acceptance::Advanced
        } else {
            Acceptance::Unchanged
        })
    }

    /// Loads the accepted projection for a discovery name.
    pub fn host_for_lookup(&self, lookup_name: &str) -> Result<Option<StoredHostSnapshot>> {
        let host_id = self
            .connection
            .query_row(
                "SELECT host_id FROM host_lookups WHERE lookup_name = ?1",
                [lookup_name],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        host_id
            .map(|host_id| load_snapshot(&self.connection, lookup_name, &host_id))
            .transpose()
    }
}
