use rusqlite::{Connection, OptionalExtension as _};

use crate::{error::unsigned, Database, Result};

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
    pub device_id: Vec<u8>,
    pub exact_link: Vec<u8>,
    pub exact_parcel: Vec<u8>,
}

impl Database {
    pub fn current_root(&self) -> Result<Option<RootSnapshot>> {
        root_snapshot(&self.connection)
    }

    pub fn identity(&self, uid: &[u8]) -> Result<Option<IdentitySnapshot>> {
        identity_snapshot(&self.connection, uid)
    }
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
    connection
        .query_row(
            "SELECT u.uid, u.normalized_name, d.device_id, l.exact_link, p.exact_parcel
             FROM users u
             JOIN devices d ON d.uid = u.uid AND d.active = 1
             JOIN user_chain_heads h ON h.uid = u.uid
             JOIN user_chain_links l ON l.uid = h.uid AND l.seqno = h.seqno
             JOIN parcels p ON p.uid = u.uid AND p.device_id = d.device_id
             WHERE u.uid = ?1",
            [uid],
            |row| {
                Ok(IdentitySnapshot {
                    uid: row.get(0)?,
                    normalized_name: row.get(1)?,
                    device_id: row.get(2)?,
                    exact_link: row.get(3)?,
                    exact_parcel: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}
