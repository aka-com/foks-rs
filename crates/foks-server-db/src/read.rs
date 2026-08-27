use rusqlite::{Connection, OptionalExtension as _};

use crate::{error::unsigned, Database, ReadDatabase, Result};

type StoredRoot = (i64, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RootSnapshot {
    pub epoch: u64,
    pub root_hash: [u8; 32],
    pub root_node: [u8; 32],
    pub exact_root: Vec<u8>,
    pub exact_signed_root: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentitySnapshot {
    pub uid: Vec<u8>,
    pub normalized_name: Vec<u8>,
    pub username_utf8: Vec<u8>,
    pub username_sequence: u64,
    pub username_commitment_key: [u8; 16],
    pub device_id: Vec<u8>,
    pub exact_device_hepk: Vec<u8>,
    pub exact_device_name: Vec<u8>,
    pub exact_link: Vec<u8>,
    pub chain_root_epoch: u64,
    pub next_tree_location: [u8; 32],
    pub exact_shared_hepk: Vec<u8>,
    pub exact_parcel: Vec<u8>,
}

impl Database {
    pub fn current_root(&self) -> Result<Option<RootSnapshot>> {
        root_snapshot(&self.connection)
    }

    pub fn identity(&self, uid: &[u8]) -> Result<Option<IdentitySnapshot>> {
        identity_snapshot(&self.connection, uid)
    }

    pub fn roots_at(&self, epochs: &[u64]) -> Result<Option<Vec<RootSnapshot>>> {
        roots_at(&self.connection, epochs)
    }
}

impl ReadDatabase {
    pub fn current_root(&self) -> Result<Option<RootSnapshot>> {
        root_snapshot(&self.connection)
    }

    pub fn roots_at(&self, epochs: &[u64]) -> Result<Option<Vec<RootSnapshot>>> {
        roots_at(&self.connection, epochs)
    }

    pub fn identity(&self, uid: &[u8]) -> Result<Option<IdentitySnapshot>> {
        identity_snapshot(&self.connection, uid)
    }

    pub fn identity_for_active_device(
        &self,
        uid: &[u8],
        device_id: &[u8],
    ) -> Result<Option<IdentitySnapshot>> {
        identity_snapshot_filtered(&self.connection, Some(uid), Some(device_id))
    }

    pub fn identity_by_active_device(&self, device_id: &[u8]) -> Result<Option<IdentitySnapshot>> {
        identity_snapshot_filtered(&self.connection, None, Some(device_id))
    }
}

pub fn roots_at(connection: &Connection, epochs: &[u64]) -> Result<Option<Vec<RootSnapshot>>> {
    let mut statement = connection.prepare(
        "SELECT epoch, root_hash, root_node, exact_root, exact_signed_root
         FROM merkle_roots WHERE epoch = ?1",
    )?;
    let mut roots = Vec::with_capacity(epochs.len());
    for &epoch in epochs {
        let epoch = crate::error::sql_integer(epoch)?;
        let row: Option<StoredRoot> = statement
            .query_row([epoch], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .optional()?;
        let Some((epoch, root_hash, root_node, exact_root, exact_signed_root)) = row else {
            return Ok(None);
        };
        roots.push(RootSnapshot {
            epoch: unsigned(epoch)?,
            root_hash: root_hash
                .try_into()
                .map_err(|_| crate::Error::Invalid("stored root hash"))?,
            root_node: root_node
                .try_into()
                .map_err(|_| crate::Error::Invalid("stored root node"))?,
            exact_root,
            exact_signed_root,
        });
    }
    Ok(Some(roots))
}

pub fn root_snapshot(connection: &Connection) -> Result<Option<RootSnapshot>> {
    let row: Option<StoredRoot> = connection
        .query_row(
            "SELECT r.epoch, r.root_hash, r.root_node, r.exact_root, r.exact_signed_root
             FROM merkle_root_heads h JOIN merkle_roots r ON r.epoch = h.epoch
             WHERE h.singleton = 1",
            [],
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
    row.map(
        |(epoch, root_hash, root_node, exact_root, exact_signed_root)| {
            Ok(RootSnapshot {
                epoch: unsigned(epoch)?,
                root_hash: root_hash
                    .try_into()
                    .map_err(|_| crate::Error::Invalid("stored root hash"))?,
                root_node: root_node
                    .try_into()
                    .map_err(|_| crate::Error::Invalid("stored root node"))?,
                exact_root,
                exact_signed_root,
            })
        },
    )
    .transpose()
}

pub fn identity_snapshot(connection: &Connection, uid: &[u8]) -> Result<Option<IdentitySnapshot>> {
    identity_snapshot_filtered(connection, Some(uid), None)
}

fn identity_snapshot_filtered(
    connection: &Connection,
    uid: Option<&[u8]>,
    device_id: Option<&[u8]>,
) -> Result<Option<IdentitySnapshot>> {
    type StoredIdentity = (
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        i64,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        i64,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
    );
    let stored: Option<StoredIdentity> = connection
        .query_row(
            "SELECT u.uid, u.normalized_name, u.username_utf8, u.username_sequence,
                    u.username_commitment_key, d.device_id, d.exact_hepk, d.exact_name,
                    l.exact_link, l.root_epoch, t.location, s.exact_hepk, p.exact_parcel
             FROM users u
             JOIN devices d ON d.uid = u.uid AND d.active = 1
             JOIN user_chain_heads h ON h.uid = u.uid
             JOIN user_chain_links l ON l.uid = h.uid AND l.seqno = h.seqno
             JOIN tree_locations t ON t.uid = h.uid AND t.seqno = h.seqno
             JOIN shared_keys s ON s.uid = u.uid AND s.role_type = 3
                 AND s.visibility = 0 AND s.generation = 1
             JOIN parcels p ON p.uid = u.uid AND p.device_id = d.device_id
                 AND p.role_type = s.role_type AND p.visibility = s.visibility
                 AND p.generation = s.generation
            WHERE (?1 IS NULL OR u.uid = ?1) AND (?2 IS NULL OR d.device_id = ?2)",
            rusqlite::params![uid, device_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                    row.get(11)?,
                    row.get(12)?,
                ))
            },
        )
        .optional()?;
    stored
        .map(
            |(
                uid,
                normalized_name,
                username_utf8,
                username_sequence,
                username_commitment_key,
                device_id,
                exact_device_hepk,
                exact_device_name,
                exact_link,
                chain_root_epoch,
                next_tree_location,
                exact_shared_hepk,
                exact_parcel,
            )| {
                Ok(IdentitySnapshot {
                    uid,
                    normalized_name,
                    username_utf8,
                    username_sequence: unsigned(username_sequence)?,
                    username_commitment_key: username_commitment_key
                        .try_into()
                        .map_err(|_| crate::Error::Invalid("stored username commitment key"))?,
                    device_id,
                    exact_device_hepk,
                    exact_device_name,
                    exact_link,
                    chain_root_epoch: unsigned(chain_root_epoch)?,
                    next_tree_location: next_tree_location
                        .try_into()
                        .map_err(|_| crate::Error::Invalid("stored tree location"))?,
                    exact_shared_hepk,
                    exact_parcel,
                })
            },
        )
        .transpose()
}
