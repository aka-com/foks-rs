use rusqlite::{Connection, OptionalExtension as _};

use crate::{error::unsigned, Database, ReadDatabase, Result};

type StoredRoot = (i64, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>);
type StoredUserHeader = (Vec<u8>, Vec<u8>, i64, Vec<u8>);
type StoredUserAuthorityHead = (i64, Vec<u8>, Vec<u8>, i64, Vec<u8>);

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserDeviceSnapshot {
    pub device_id: Vec<u8>,
    pub active: bool,
    pub role_type: u64,
    pub visibility: i64,
    pub subkey_id: Option<Vec<u8>>,
    pub exact_hepk: Vec<u8>,
    pub exact_name: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserSharedKeySnapshot {
    pub role_type: u64,
    pub visibility: i64,
    pub generation: u64,
    pub verify_key: Vec<u8>,
    pub exact_hepk: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserAuthoritySnapshot {
    pub uid: Vec<u8>,
    pub chain_sequence: u64,
    pub chain_tail_hash: [u8; 32],
    pub next_tree_location: [u8; 32],
    pub current_root_epoch: u64,
    pub current_root_hash: [u8; 32],
    pub devices: Vec<UserDeviceSnapshot>,
    pub shared_keys: Vec<UserSharedKeySnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserChainLinkSnapshot {
    pub sequence: u64,
    pub exact_link: Vec<u8>,
    pub root_epoch: u64,
    pub next_tree_location: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserChainSnapshot {
    pub uid: Vec<u8>,
    pub normalized_name: Vec<u8>,
    pub username_utf8: Vec<u8>,
    pub username_sequence: u64,
    pub username_commitment_key: [u8; 16],
    pub links: Vec<UserChainLinkSnapshot>,
    pub devices: Vec<UserDeviceSnapshot>,
    pub exact_shared_hepks: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PukMaterialSnapshot {
    pub exact_box_set: Vec<u8>,
    pub sender_id: Vec<u8>,
    pub exact_seed_chain: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamLinkSnapshot {
    pub sequence: u64,
    pub exact_link: Vec<u8>,
    pub root_epoch: u64,
    pub next_tree_location: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamMemberSnapshot {
    pub party_id: Vec<u8>,
    pub scoped_host_id: Option<Vec<u8>>,
    pub source_role_type: u64,
    pub source_visibility: i64,
    pub role_type: u64,
    pub visibility: i64,
    pub generation: u64,
    pub verify_key: Vec<u8>,
    pub hepk_fingerprint: [u8; 32],
    pub removal_key_commitment: Option<[u8; 32]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamSnapshot {
    pub team_id: Vec<u8>,
    pub kind: u8,
    pub host_id: Vec<u8>,
    pub normalized_name: Option<Vec<u8>>,
    pub team_name_utf8: Vec<u8>,
    pub team_name_sequence: u64,
    pub team_name_commitment_key: Option<[u8; 16]>,
    pub links: Vec<TeamLinkSnapshot>,
    pub members: Vec<TeamMemberSnapshot>,
    pub shared_keys: Vec<UserSharedKeySnapshot>,
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

    pub fn user_authority(&self, uid: &[u8]) -> Result<Option<UserAuthoritySnapshot>> {
        user_authority(&self.connection, uid)
    }

    pub fn user_chain(&self, uid: &[u8]) -> Result<Option<UserChainSnapshot>> {
        user_chain(&self.connection, uid)
    }

    pub fn team(&self, team_id: &[u8]) -> Result<Option<TeamSnapshot>> {
        team_snapshot(&self.connection, team_id)
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

    pub fn user_authority(&self, uid: &[u8]) -> Result<Option<UserAuthoritySnapshot>> {
        user_authority(&self.connection, uid)
    }

    pub fn user_chain(&self, uid: &[u8]) -> Result<Option<UserChainSnapshot>> {
        user_chain(&self.connection, uid)
    }

    pub fn team(&self, team_id: &[u8]) -> Result<Option<TeamSnapshot>> {
        team_snapshot(&self.connection, team_id)
    }

    pub fn team_parcels(&self, team_id: &[u8], party_id: &[u8]) -> Result<Vec<Vec<u8>>> {
        let mut statement = self.connection.prepare(
            "SELECT p.exact_parcel FROM team_parcels p
             WHERE p.team_id = ?1 AND p.party_id = ?2
               AND p.generation = (
                   SELECT max(latest.generation) FROM team_parcels latest
                   WHERE latest.team_id = p.team_id AND latest.party_id = p.party_id
                     AND latest.role_type = p.role_type
                     AND latest.visibility = p.visibility
               )
             ORDER BY role_type, visibility, generation",
        )?;
        let parcels = statement
            .query_map(rusqlite::params![team_id, party_id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(parcels)
    }

    pub fn team_member_hepks(&self, team_id: &[u8]) -> Result<Vec<Vec<u8>>> {
        let mut statement = self.connection.prepare(
            "SELECT DISTINCT k.exact_hepk
             FROM team_members m JOIN shared_keys k
               ON k.uid = m.party_id AND k.role_type = m.source_role_type
              AND k.visibility = m.source_visibility AND k.generation = m.generation
              AND k.verify_key = m.verify_key
             WHERE m.team_id = ?1 ORDER BY k.exact_hepk",
        )?;
        let hepks = statement
            .query_map([team_id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(hepks)
    }

    pub fn team_removal_box(
        &self,
        team_id: &[u8],
        member_id: &[u8],
        member_host_id: &[u8],
        source_role_type: u64,
        source_visibility: i64,
    ) -> Result<Option<Vec<u8>>> {
        Ok(self
            .connection
            .query_row(
                "SELECT exact_box FROM team_removal_boxes
                 WHERE team_id = ?1 AND member_id = ?2 AND member_host_id = ?3
                   AND source_role_type = ?4 AND source_visibility = ?5",
                rusqlite::params![
                    team_id,
                    member_id,
                    member_host_id,
                    crate::error::sql_integer(source_role_type)?,
                    source_visibility
                ],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn puk_material(
        &self,
        uid: &[u8],
        device_id: &[u8],
        role_type: u64,
        visibility: i64,
    ) -> Result<Option<PukMaterialSnapshot>> {
        puk_material(&self.connection, uid, device_id, role_type, visibility)
    }
}

fn team_snapshot(connection: &Connection, team_id: &[u8]) -> Result<Option<TeamSnapshot>> {
    type TeamRow = (i64, Vec<u8>, Option<Vec<u8>>, Vec<u8>, i64, Option<Vec<u8>>);
    let row: Option<TeamRow> = connection
        .query_row(
            "SELECT team_kind, host_id, normalized_name, team_name_utf8,
                    team_name_sequence, team_name_commitment_key
             FROM teams WHERE team_id = ?1",
            [team_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?;
    let Some((kind, host_id, normalized_name, team_name_utf8, name_sequence, commitment)) = row
    else {
        return Ok(None);
    };
    let mut statement = connection.prepare(
        "SELECT l.seqno, l.exact_link, l.root_epoch, t.location
         FROM team_chain_links l JOIN team_tree_locations t
           ON t.team_id = l.team_id AND t.seqno = l.seqno
         WHERE l.team_id = ?1 ORDER BY l.seqno",
    )?;
    let links = statement
        .query_map([team_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Vec<u8>>(3)?,
            ))
        })?
        .map(|row| {
            let (sequence, exact_link, root_epoch, location) = row?;
            Ok(TeamLinkSnapshot {
                sequence: unsigned(sequence)?,
                exact_link,
                root_epoch: unsigned(root_epoch)?,
                next_tree_location: location
                    .try_into()
                    .map_err(|_| crate::Error::Invalid("stored team location"))?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut statement = connection.prepare(
        "SELECT party_id, scoped_host_id, source_role_type, source_visibility,
                role_type, visibility, generation, verify_key, hepk_fingerprint,
                removal_key_commitment
         FROM team_members WHERE team_id = ?1 ORDER BY party_id",
    )?;
    let members = statement
        .query_map([team_id], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Option<Vec<u8>>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, Vec<u8>>(7)?,
                row.get::<_, Vec<u8>>(8)?,
                row.get::<_, Option<Vec<u8>>>(9)?,
            ))
        })?
        .map(|row| {
            let (
                party_id,
                scoped_host_id,
                source_role_type,
                source_visibility,
                role_type,
                visibility,
                generation,
                verify_key,
                fingerprint,
                removal,
            ) = row?;
            Ok(TeamMemberSnapshot {
                party_id,
                scoped_host_id,
                source_role_type: unsigned(source_role_type)?,
                source_visibility,
                role_type: unsigned(role_type)?,
                visibility,
                generation: unsigned(generation)?,
                verify_key,
                hepk_fingerprint: fingerprint
                    .try_into()
                    .map_err(|_| crate::Error::Invalid("stored team member HEPK fingerprint"))?,
                removal_key_commitment: removal
                    .map(|value| {
                        value
                            .try_into()
                            .map_err(|_| crate::Error::Invalid("stored removal commitment"))
                    })
                    .transpose()?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut statement = connection.prepare(
        "SELECT role_type, visibility, generation, verify_key, exact_hepk
         FROM team_shared_keys WHERE team_id = ?1 ORDER BY role_type, visibility, generation",
    )?;
    let shared_keys = statement
        .query_map([team_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, Vec<u8>>(4)?,
            ))
        })?
        .map(|row| {
            let (role_type, visibility, generation, verify_key, exact_hepk) = row?;
            Ok(UserSharedKeySnapshot {
                role_type: unsigned(role_type)?,
                visibility,
                generation: unsigned(generation)?,
                verify_key,
                exact_hepk,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(TeamSnapshot {
        team_id: team_id.to_vec(),
        kind: u8::try_from(kind).map_err(|_| crate::Error::Invalid("stored team kind"))?,
        host_id,
        normalized_name,
        team_name_utf8,
        team_name_sequence: unsigned(name_sequence)?,
        team_name_commitment_key: commitment
            .map(|value| {
                value
                    .try_into()
                    .map_err(|_| crate::Error::Invalid("stored team-name key"))
            })
            .transpose()?,
        links,
        members,
        shared_keys,
    }))
}

pub fn user_chain(connection: &Connection, uid: &[u8]) -> Result<Option<UserChainSnapshot>> {
    let user: Option<StoredUserHeader> = connection
        .query_row(
            "SELECT normalized_name, username_utf8, username_sequence, username_commitment_key
             FROM users WHERE uid = ?1",
            [uid],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((normalized_name, username_utf8, username_sequence, commitment_key)) = user else {
        return Ok(None);
    };
    let mut link_statement = connection.prepare(
        "SELECT l.seqno, l.exact_link, l.root_epoch, t.location
         FROM user_chain_links l
         JOIN tree_locations t ON t.uid = l.uid AND t.seqno = l.seqno
         WHERE l.uid = ?1 ORDER BY l.seqno",
    )?;
    let links = link_statement
        .query_map([uid], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Vec<u8>>(3)?,
            ))
        })?
        .map(|row| {
            let (sequence, exact_link, root_epoch, location) = row?;
            Ok(UserChainLinkSnapshot {
                sequence: unsigned(sequence)?,
                exact_link,
                root_epoch: unsigned(root_epoch)?,
                next_tree_location: location
                    .try_into()
                    .map_err(|_| crate::Error::Invalid("stored user tree location"))?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let authority = user_authority(connection, uid)?
        .ok_or(crate::Error::Invalid("user chain has no authority"))?;
    let mut hepk_statement = connection.prepare(
        "SELECT exact_hepk FROM shared_keys WHERE uid = ?1
         UNION SELECT exact_hepk FROM devices WHERE uid = ?1",
    )?;
    let exact_shared_hepks = hepk_statement
        .query_map([uid], |row| row.get(0))?
        .collect::<std::result::Result<Vec<Vec<u8>>, _>>()?;
    Ok(Some(UserChainSnapshot {
        uid: uid.to_vec(),
        normalized_name,
        username_utf8,
        username_sequence: unsigned(username_sequence)?,
        username_commitment_key: commitment_key
            .try_into()
            .map_err(|_| crate::Error::Invalid("stored username commitment key"))?,
        links,
        devices: authority.devices,
        exact_shared_hepks,
    }))
}

fn puk_material(
    connection: &Connection,
    uid: &[u8],
    device_id: &[u8],
    role_type: u64,
    visibility: i64,
) -> Result<Option<PukMaterialSnapshot>> {
    let parcel: Option<(Vec<u8>, Vec<u8>)> = connection
        .query_row(
            "SELECT exact_parcel, sender_id FROM parcels
             WHERE uid = ?1 AND device_id = ?2 AND role_type = ?3 AND visibility = ?4
             ORDER BY generation DESC LIMIT 1",
            rusqlite::params![
                uid,
                device_id,
                crate::error::sql_integer(role_type)?,
                visibility
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((exact_box_set, sender_id)) = parcel else {
        return Ok(None);
    };
    let mut statement = connection.prepare(
        "SELECT exact_box FROM seed_chain_boxes
         WHERE uid = ?1 AND role_type = ?2 AND visibility = ?3
         ORDER BY generation",
    )?;
    let exact_seed_chain = statement
        .query_map(
            rusqlite::params![uid, crate::error::sql_integer(role_type)?, visibility],
            |row| row.get(0),
        )?
        .collect::<std::result::Result<Vec<Vec<u8>>, _>>()?;
    Ok(Some(PukMaterialSnapshot {
        exact_box_set,
        sender_id,
        exact_seed_chain,
    }))
}

pub fn user_authority(
    connection: &Connection,
    uid: &[u8],
) -> Result<Option<UserAuthoritySnapshot>> {
    let head: Option<StoredUserAuthorityHead> = connection
        .query_row(
            "SELECT h.seqno, h.link_hash, t.location, r.epoch, r.root_hash
             FROM user_chain_heads h
             JOIN tree_locations t ON t.uid = h.uid AND t.seqno = h.seqno
             JOIN merkle_root_heads rh ON rh.singleton = 1
             JOIN merkle_roots r ON r.epoch = rh.epoch
             WHERE h.uid = ?1",
            [uid],
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
    let Some((sequence, tail_hash, next_tree_location, root_epoch, root_hash)) = head else {
        return Ok(None);
    };
    let mut device_statement = connection.prepare(
        "SELECT device_id, active, role_type, visibility, subkey_id, exact_hepk, exact_name
         FROM devices WHERE uid = ?1 ORDER BY device_id",
    )?;
    let devices = device_statement
        .query_map([uid], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<Vec<u8>>>(4)?,
                row.get::<_, Vec<u8>>(5)?,
                row.get::<_, Vec<u8>>(6)?,
            ))
        })?
        .map(|row| {
            let (device_id, active, role_type, visibility, subkey_id, exact_hepk, exact_name) =
                row?;
            Ok(UserDeviceSnapshot {
                device_id,
                active: active == 1,
                role_type: unsigned(role_type)?,
                visibility,
                subkey_id,
                exact_hepk,
                exact_name,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut key_statement = connection.prepare(
        "SELECT role_type, visibility, generation, verify_key, exact_hepk
         FROM shared_keys WHERE uid = ?1
         ORDER BY role_type, visibility, generation",
    )?;
    let shared_keys = key_statement
        .query_map([uid], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get(1)?,
                row.get::<_, i64>(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .map(|row| {
            let (role_type, visibility, generation, verify_key, exact_hepk) = row?;
            Ok(UserSharedKeySnapshot {
                role_type: unsigned(role_type)?,
                visibility,
                generation: unsigned(generation)?,
                verify_key,
                exact_hepk,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(UserAuthoritySnapshot {
        uid: uid.to_vec(),
        chain_sequence: unsigned(sequence)?,
        chain_tail_hash: tail_hash
            .try_into()
            .map_err(|_| crate::Error::Invalid("stored user-chain tail hash"))?,
        next_tree_location: next_tree_location
            .try_into()
            .map_err(|_| crate::Error::Invalid("stored next tree location"))?,
        current_root_epoch: unsigned(root_epoch)?,
        current_root_hash: root_hash
            .try_into()
            .map_err(|_| crate::Error::Invalid("stored current root hash"))?,
        devices,
        shared_keys,
    }))
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
