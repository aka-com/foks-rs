//! Durable SQLite hard state for a native FOKS client.
//!
//! This crate is intentionally below the protocol verifier. It preserves the
//! exact signed bytes supplied by that verifier and enforces monotonic pins,
//! but it does not parse Snowpack or verify signatures itself.

#![forbid(unsafe_code)]

mod schema;

use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::Path;
use std::time::Duration;

use foks_snowpack::{decode, encode, Value};
use foks_verify::{
    AuthenticatedMerkleRoot, HostService, MerkleRootEvidence, VerifiedHostSnapshot,
    VerifiedHostSnapshotParts, VerifiedMerkleRoot, VerifiedMerkleRootParts, VerifiedUserSnapshot,
    VerifiedUserSnapshotParts,
};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension as _, TransactionBehavior};
use thiserror::Error;

use schema::{APPLICATION_ID, INITIAL as SCHEMA, VERSION as SCHEMA_VERSION};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredHostSnapshot {
    pub lookup_name: String,
    pub host_id: Vec<u8>,
    pub canonical_name: String,
    pub genesis_key: Vec<u8>,
    pub chain_seqno: u64,
    pub chain_tail_hash: [u8; 32],
    pub chain_bytes: Vec<u8>,
    pub public_zone_bytes: Vec<u8>,
    pub services: Vec<HostService>,
    pub merkle_root: StoredMerkleRoot,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredMerkleRoot {
    pub epoch: u64,
    pub root_hash: [u8; 32],
    pub root_bytes: Vec<u8>,
    pub evidence: MerkleRootEvidence,
    pub authenticated_roots: Vec<AuthenticatedMerkleRoot>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct StoredRole {
    role_type: u64,
    visibility: i64,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct StoredUserDevice {
    device_id: Vec<u8>,
    role: StoredRole,
    hepk_bytes: Vec<u8>,
    subkey_id: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct StoredUserSharedKey {
    role: StoredRole,
    generation: u64,
    verify_key: Vec<u8>,
    hepk_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredUserSnapshot {
    pub host_id: Vec<u8>,
    pub uid: Vec<u8>,
    pub chain_seqno: u64,
    pub chain_tail_hash: [u8; 32],
    pub chain_bytes: Vec<u8>,
    pub evidence_bytes: Vec<u8>,
    pub username: Vec<u8>,
    pub username_utf8: Vec<u8>,
    pub username_sequence: u64,
    pub merkle_epoch: u64,
    pub merkle_root_hash: [u8; 32],
    pub merkle_root_bytes: Vec<u8>,
    pub devices: Vec<foks_verify::VerifiedUserDevice>,
    pub shared_keys: Vec<foks_verify::VerifiedUserSharedKey>,
}

impl StoredUserSnapshot {
    pub fn parts(&self) -> VerifiedUserSnapshotParts<'_> {
        VerifiedUserSnapshotParts {
            host_id: &self.host_id,
            uid: &self.uid,
            chain_seqno: self.chain_seqno,
            chain_tail_hash: self.chain_tail_hash,
            chain_bytes: &self.chain_bytes,
            evidence_bytes: &self.evidence_bytes,
            username: &self.username,
            username_utf8: &self.username_utf8,
            username_sequence: self.username_sequence,
            merkle_epoch: self.merkle_epoch,
            merkle_root_hash: self.merkle_root_hash,
            merkle_root_bytes: &self.merkle_root_bytes,
            devices: &self.devices,
            shared_keys: &self.shared_keys,
        }
    }
}

/// The durable result of accepting a verified snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Acceptance {
    Inserted,
    Advanced,
    Unchanged,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("hard-state filesystem operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("SQLite hard-state operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("invalid persisted Merkle evidence: {0}")]
    Snowpack(#[from] foks_snowpack::Error),
    #[error("invalid verified host snapshot: {0}")]
    InvalidSnapshot(&'static str),
    #[error("hard-state database path must not be a symbolic link")]
    SymlinkDatabase,
    #[error("hard-state database has application id {found:#x}, expected {expected:#x}")]
    WrongApplicationId { found: i64, expected: i64 },
    #[error(
        "system store is out of date (v{found}), could not auto-update to current version (v{supported})"
    )]
    UnsupportedSchema { found: u32, supported: u32 },
    #[error("lookup name {lookup_name:?} is pinned to a different HostID")]
    HostIdentityChanged { lookup_name: String },
    #[error("the genesis key changed for an already-pinned HostID")]
    GenesisChanged,
    #[error("host chain rolled back from sequence {stored} to {received}")]
    ChainRollback { stored: u64, received: u64 },
    #[error("host chain forked at sequence {seqno}")]
    ChainFork { seqno: u64 },
    #[error("host projection changed without a host-chain advance at sequence {seqno}")]
    ProjectionChanged { seqno: u64 },
    #[error("Merkle root rolled back from epoch {stored} to {received}")]
    MerkleRollback { stored: u64, received: u64 },
    #[error("Merkle root forked at epoch {epoch}")]
    MerkleFork { epoch: u64 },
    #[error("verified user references a host that is not pinned")]
    UnknownHost,
    #[error("invalid verified user snapshot: {0}")]
    InvalidUser(&'static str),
    #[error("user chain rolled back from sequence {stored} to {received}")]
    UserRollback { stored: u64, received: u64 },
    #[error("user chain forked at sequence {seqno}")]
    UserFork { seqno: u64 },
    #[error("user projection changed without a chain advance at sequence {seqno}")]
    UserProjectionChanged { seqno: u64 },
    #[error("{field} value {value} cannot be represented by SQLite")]
    IntegerOutOfRange { field: &'static str, value: u64 },
    #[error("stored {field} value {value} cannot be represented by the protocol")]
    StoredIntegerOutOfRange { field: &'static str, value: i64 },
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// A connection-owning hard-state repository.
///
/// Callers should keep one instance on their storage worker thread. The API is
/// synchronous by design so a future generic SQLite runtime can own and invoke
/// it without this crate taking an async-runtime dependency.
pub struct HardStateStore {
    connection: Connection,
}

impl HardStateStore {
    /// Opens or initializes a FOKS hard-state database at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        // SQLite cannot combine CREATE and NOFOLLOW on every supported build.
        // create_new is itself symlink-safe and gives NOFOLLOW an existing
        // inode to open without weakening the subsequent database open.
        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(file) => drop(file),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
            return Err(Error::SymlinkDatabase);
        }
        // macOS exposes /tmp as a symlink to /private/tmp. SQLite's NOFOLLOW
        // rejects symlinks in any path component, so resolve safe parent
        // aliases after separately rejecting a symlink at the database leaf.
        let database_path = path.canonicalize()?;
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_FULL_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW;
        let mut connection = Connection::open_with_flags(database_path, flags)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "trusted_schema", false)?;
        connection.pragma_update(None, "secure_delete", true)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        initialize_or_verify(&mut connection)?;
        Ok(Self { connection })
    }

    /// Atomically accepts a complete, already-verified host snapshot.
    ///
    /// A failed monotonicity check rolls back every part of the update,
    /// including the host chain when the accompanying Merkle root is stale.
    pub fn accept_verified_host(&mut self, snapshot: &VerifiedHostSnapshot) -> Result<Acceptance> {
        let snapshot = snapshot.parts();
        self.accept_host_parts(snapshot)
    }

    fn accept_host_parts(&mut self, snapshot: VerifiedHostSnapshotParts<'_>) -> Result<Acceptance> {
        validate_snapshot(snapshot)?;
        let chain_seqno = sqlite_integer("chain sequence", snapshot.chain_seqno)?;
        let merkle_epoch = sqlite_integer("Merkle epoch", snapshot.merkle_root.epoch)?;
        let mut service_types = snapshot
            .services
            .iter()
            .map(|service| sqlite_integer("service type", service.service_type))
            .collect::<Result<Vec<_>>>()?;
        service_types.sort_unstable();
        if service_types.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(Error::InvalidSnapshot("service types must be unique"));
        }

        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

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
                        sqlite_integer("service type", service.service_type)?,
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

    /// Atomically pins a verified user chain, its authenticating Merkle root,
    /// and the public device/PUK projection. No private key material is stored.
    pub fn accept_verified_user(&mut self, snapshot: &VerifiedUserSnapshot) -> Result<Acceptance> {
        let snapshot = snapshot.parts();
        self.accept_user_parts(snapshot)
    }

    fn accept_user_parts(&mut self, snapshot: VerifiedUserSnapshotParts<'_>) -> Result<Acceptance> {
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
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
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
                evidence,
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
                    if evidence != snapshot.evidence_bytes
                        || username != snapshot.username
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

    /// Advances only the already-verified Merkle head for a pinned host.
    pub fn accept_verified_merkle_root(
        &mut self,
        host_id: &[u8],
        root: &VerifiedMerkleRoot,
    ) -> Result<Acceptance> {
        let root = root.parts();
        let epoch = sqlite_integer("Merkle epoch", root.epoch)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
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

    /// Loads an untrusted persisted user projection. Callers must pass its
    /// `parts()` through `foks_verify::restore_verified_user` before use.
    pub fn user_for_host(&self, host_id: &[u8], uid: &[u8]) -> Result<Option<StoredUserSnapshot>> {
        load_user_snapshot(&self.connection, host_id, uid)
    }
}

#[derive(Debug)]
struct StoredHost {
    canonical_name: String,
    genesis_key: Vec<u8>,
    chain_seqno: u64,
    chain_tail_hash: [u8; 32],
    chain_bytes: Vec<u8>,
    public_zone_bytes: Vec<u8>,
}

fn initialize_or_verify(connection: &mut Connection) -> Result<()> {
    let application_id: i64 =
        connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if application_id == 0 && version == 0 {
        let object_count: i64 = connection.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )?;
        if object_count != 0 {
            return Err(Error::WrongApplicationId {
                found: application_id,
                expected: APPLICATION_ID,
            });
        }
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(SCHEMA)?;
        transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
        transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        transaction.commit()?;
        return Ok(());
    }
    if application_id != APPLICATION_ID {
        return Err(Error::WrongApplicationId {
            found: application_id,
            expected: APPLICATION_ID,
        });
    }
    if version != SCHEMA_VERSION {
        return Err(Error::UnsupportedSchema {
            found: version,
            supported: SCHEMA_VERSION,
        });
    }
    Ok(())
}

fn validate_snapshot(snapshot: VerifiedHostSnapshotParts<'_>) -> Result<()> {
    if snapshot.lookup_name.trim().is_empty() {
        return Err(Error::InvalidSnapshot("lookup name must not be empty"));
    }
    if snapshot.host_id.is_empty() {
        return Err(Error::InvalidSnapshot("HostID must not be empty"));
    }
    if snapshot.canonical_name.trim().is_empty() {
        return Err(Error::InvalidSnapshot("canonical name must not be empty"));
    }
    if snapshot.genesis_key.is_empty() {
        return Err(Error::InvalidSnapshot("genesis key must not be empty"));
    }
    if snapshot.chain_bytes.is_empty() {
        return Err(Error::InvalidSnapshot("host-chain bytes must not be empty"));
    }
    if snapshot.public_zone_bytes.is_empty() {
        return Err(Error::InvalidSnapshot(
            "signed public-zone bytes must not be empty",
        ));
    }
    if snapshot.merkle_root.root_bytes.is_empty() {
        return Err(Error::InvalidSnapshot(
            "Merkle-root bytes must not be empty",
        ));
    }
    validate_merkle_root(snapshot.merkle_root)?;
    if snapshot
        .services
        .iter()
        .any(|service| service.endpoint_bytes.is_empty())
    {
        return Err(Error::InvalidSnapshot("service endpoint must not be empty"));
    }
    Ok(())
}

fn validate_user_snapshot(snapshot: VerifiedUserSnapshotParts<'_>) -> Result<()> {
    if snapshot.host_id.is_empty()
        || snapshot.uid.len() != 33
        || snapshot.chain_seqno == 0
        || snapshot.chain_bytes.is_empty()
        || snapshot.evidence_bytes.is_empty()
        || !(3..=25).contains(&snapshot.username.len())
        || snapshot.username_utf8.is_empty()
        || snapshot.username_sequence == 0
        || snapshot.merkle_root_bytes.is_empty()
        || snapshot.devices.is_empty()
        || snapshot.shared_keys.is_empty()
        || snapshot.devices.iter().any(|device| {
            !matches!(device.device_id.len(), 33 | 34)
                || !valid_stored_role(stored_role(device.role))
                || device.hepk_bytes.is_empty()
                || device
                    .subkey_id
                    .as_ref()
                    .is_some_and(|subkey| subkey.len() != 33)
        })
        || snapshot.shared_keys.iter().any(|key| {
            key.verify_key.len() != 33
                || !valid_stored_role(stored_role(key.role))
                || key.generation == 0
                || key.hepk_bytes.is_empty()
        })
    {
        return Err(Error::InvalidUser("required field is missing or malformed"));
    }
    Ok(())
}

fn valid_stored_role(role: StoredRole) -> bool {
    match role.role_type {
        1 => (-32_768..32_768).contains(&role.visibility),
        2 | 3 => role.visibility == 0,
        _ => false,
    }
}

fn protocol_role(role: StoredRole) -> Result<foks_proto::Role> {
    match (role.role_type, role.visibility) {
        (1, visibility) => i16::try_from(visibility)
            .map(foks_proto::Role::member)
            .map_err(|_| Error::InvalidUser("stored member visibility is invalid")),
        (2, 0) => Ok(foks_proto::Role::ADMIN),
        (3, 0) => Ok(foks_proto::Role::OWNER),
        _ => Err(Error::InvalidUser("stored role is invalid")),
    }
}

fn load_user_snapshot(
    connection: &Connection,
    host_id: &[u8],
    uid: &[u8],
) -> Result<Option<StoredUserSnapshot>> {
    let row = connection
        .query_row(
            "SELECT chain_seqno, chain_tail_hash, chain_bytes, evidence_bytes, username, \
             username_utf8, username_sequence, merkle_epoch, merkle_root_hash, \
             merkle_root_bytes FROM users WHERE host_id = ?1 AND uid = ?2",
            params![host_id, uid],
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
    let Some((
        seqno,
        tail,
        chain_bytes,
        evidence_bytes,
        username,
        username_utf8,
        username_sequence,
        merkle_epoch,
        root_hash,
        root_bytes,
    )) = row
    else {
        return Ok(None);
    };
    let devices = load_user_devices(connection, host_id, uid)?
        .into_iter()
        .map(|device| {
            Ok(foks_verify::VerifiedUserDevice {
                device_id: device.device_id,
                role: protocol_role(device.role)?,
                hepk_bytes: device.hepk_bytes,
                subkey_id: device.subkey_id,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let shared_keys = load_user_shared_keys(connection, host_id, uid)?
        .into_iter()
        .map(|key| {
            Ok(foks_verify::VerifiedUserSharedKey {
                role: protocol_role(key.role)?,
                generation: key.generation,
                verify_key: key.verify_key,
                hepk_bytes: key.hepk_bytes,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(StoredUserSnapshot {
        host_id: host_id.to_vec(),
        uid: uid.to_vec(),
        chain_seqno: stored_unsigned("user chain sequence", seqno)?,
        chain_tail_hash: fixed_hash(tail, "stored user-chain tail has an invalid length")?,
        chain_bytes,
        evidence_bytes,
        username,
        username_utf8,
        username_sequence: stored_unsigned("username sequence", username_sequence)?,
        merkle_epoch: stored_unsigned("user Merkle epoch", merkle_epoch)?,
        merkle_root_hash: fixed_hash(root_hash, "stored user Merkle hash has an invalid length")?,
        merkle_root_bytes: root_bytes,
        devices,
        shared_keys,
    }))
}

fn load_user_devices(
    connection: &Connection,
    host_id: &[u8],
    uid: &[u8],
) -> Result<Vec<StoredUserDevice>> {
    let mut statement = connection.prepare(
        "SELECT device_id, role_type, role_visibility, hepk_bytes, subkey_id FROM user_devices \
         WHERE host_id = ?1 AND uid = ?2 \
         ORDER BY device_id, role_type, role_visibility, hepk_bytes",
    )?;
    let rows = statement.query_map(params![host_id, uid], |row| {
        Ok((
            row.get::<_, Vec<u8>>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, Vec<u8>>(3)?,
            row.get::<_, Option<Vec<u8>>>(4)?,
        ))
    })?;
    rows.map(|row| {
        let (device_id, role_type, visibility, hepk_bytes, subkey_id) = row?;
        Ok(StoredUserDevice {
            device_id,
            role: StoredRole {
                role_type: stored_unsigned("device role type", role_type)?,
                visibility,
            },
            hepk_bytes,
            subkey_id,
        })
    })
    .collect()
}

fn load_user_shared_keys(
    connection: &Connection,
    host_id: &[u8],
    uid: &[u8],
) -> Result<Vec<StoredUserSharedKey>> {
    let mut statement = connection.prepare(
        "SELECT role_type, role_visibility, generation, verify_key, hepk_bytes \
         FROM user_shared_keys WHERE host_id = ?1 AND uid = ?2 \
         ORDER BY role_type, role_visibility, generation, verify_key, hepk_bytes",
    )?;
    let rows = statement.query_map(params![host_id, uid], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, Vec<u8>>(3)?,
            row.get::<_, Vec<u8>>(4)?,
        ))
    })?;
    rows.map(|row| {
        let (role_type, visibility, generation, verify_key, hepk_bytes) = row?;
        Ok(StoredUserSharedKey {
            role: StoredRole {
                role_type: stored_unsigned("shared-key role type", role_type)?,
                visibility,
            },
            generation: stored_unsigned("shared-key generation", generation)?,
            verify_key,
            hepk_bytes,
        })
    })
    .collect()
}

fn sqlite_integer(field: &'static str, value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| Error::IntegerOutOfRange { field, value })
}

fn stored_unsigned(field: &'static str, value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|_| Error::StoredIntegerOutOfRange { field, value })
}

fn stored_role(role: foks_proto::Role) -> StoredRole {
    StoredRole {
        role_type: role.protocol_value(),
        visibility: i64::from(role.visibility().unwrap_or(0)),
    }
}

fn stored_devices(devices: &[foks_verify::VerifiedUserDevice]) -> Vec<StoredUserDevice> {
    devices
        .iter()
        .map(|device| StoredUserDevice {
            device_id: device.device_id.clone(),
            role: stored_role(device.role),
            hepk_bytes: device.hepk_bytes.clone(),
            subkey_id: device.subkey_id.clone(),
        })
        .collect()
}

fn stored_shared_keys(keys: &[foks_verify::VerifiedUserSharedKey]) -> Vec<StoredUserSharedKey> {
    keys.iter()
        .map(|key| StoredUserSharedKey {
            role: stored_role(key.role),
            generation: key.generation,
            verify_key: key.verify_key.clone(),
            hepk_bytes: key.hepk_bytes.clone(),
        })
        .collect()
}

fn load_host_row(connection: &Connection, host_id: &[u8]) -> Result<Option<StoredHost>> {
    connection
        .query_row(
            "SELECT canonical_name, genesis_key, chain_seqno, chain_tail_hash, chain_bytes, \
             public_zone_bytes \
             FROM hosts WHERE host_id = ?1",
            [host_id],
            |row| {
                let seqno = row.get::<_, i64>(2)?;
                let tail = row.get::<_, Vec<u8>>(3)?;
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    seqno,
                    tail,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?
        .map(
            |(canonical_name, genesis_key, chain_seqno, tail, chain_bytes, public_zone_bytes)| {
                Ok(StoredHost {
                    canonical_name,
                    genesis_key,
                    chain_seqno: stored_unsigned("host-chain sequence", chain_seqno)?,
                    chain_tail_hash: fixed_hash(
                        tail,
                        "stored host-chain tail has an invalid length",
                    )?,
                    chain_bytes,
                    public_zone_bytes,
                })
            },
        )
        .transpose()
}

fn normalized_services(services: &[HostService]) -> Vec<HostService> {
    let mut services = services.to_vec();
    services.sort_unstable_by_key(|service| service.service_type);
    services
}

fn load_services(connection: &Connection, host_id: &[u8]) -> Result<Vec<HostService>> {
    let mut statement = connection.prepare(
        "SELECT service_type, endpoint_bytes FROM host_services \
         WHERE host_id = ?1 ORDER BY service_type",
    )?;
    let rows = statement.query_map([host_id], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
    })?;
    rows.map(|row| {
        let (service_type, endpoint_bytes) = row?;
        Ok(HostService {
            service_type: stored_unsigned("service type", service_type)?,
            endpoint_bytes,
        })
    })
    .collect()
}

fn accept_merkle_root(
    connection: &Connection,
    host_id: &[u8],
    root: VerifiedMerkleRootParts<'_>,
    epoch: i64,
) -> Result<bool> {
    validate_merkle_root(root)?;
    let stored = connection
        .query_row(
            "SELECT epoch, root_hash FROM merkle_heads WHERE host_id = ?1",
            [host_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )
        .optional()?;
    if let Some((stored_epoch, stored_hash)) = stored {
        let stored_epoch = stored_unsigned("Merkle epoch", stored_epoch)?;
        match root.epoch.cmp(&stored_epoch) {
            std::cmp::Ordering::Less => {
                return Err(Error::MerkleRollback {
                    stored: stored_epoch,
                    received: root.epoch,
                });
            }
            std::cmp::Ordering::Equal => {
                let stored_head = connection.query_row(
                    "SELECT r.root_bytes, h.evidence_kind, h.anchor_epoch, h.evidence_bytes \
                     FROM merkle_heads h JOIN merkle_roots r \
                     ON r.host_id = h.host_id AND r.epoch = h.epoch \
                     WHERE h.host_id = ?1 AND h.epoch = ?2",
                    params![host_id, epoch],
                    |row| {
                        Ok((
                            row.get::<_, Option<Vec<u8>>>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, Option<i64>>(2)?,
                            row.get::<_, Vec<u8>>(3)?,
                        ))
                    },
                )?;
                if stored_hash != root.root_hash
                    || stored_head.0.as_deref() != Some(root.root_bytes)
                    || &decode_evidence(stored_head.1, stored_head.2, stored_head.3)?
                        != root.evidence
                {
                    return Err(Error::MerkleFork { epoch: root.epoch });
                }
                accept_authenticated_roots(connection, host_id, root.authenticated_roots)?;
                return Ok(false);
            }
            std::cmp::Ordering::Greater => {}
        }
    }

    accept_authenticated_roots(connection, host_id, root.authenticated_roots)?;
    let (evidence_kind, anchor_epoch, evidence_bytes) = encode_evidence(root.evidence)?;
    connection.execute(
        "INSERT INTO merkle_heads \
         (host_id, epoch, root_hash, evidence_kind, anchor_epoch, evidence_bytes) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(host_id) DO UPDATE SET \
         epoch = excluded.epoch, root_hash = excluded.root_hash, \
         evidence_kind = excluded.evidence_kind, anchor_epoch = excluded.anchor_epoch, \
         evidence_bytes = excluded.evidence_bytes",
        params![
            host_id,
            epoch,
            root.root_hash.as_slice(),
            evidence_kind,
            anchor_epoch,
            evidence_bytes,
        ],
    )?;
    Ok(true)
}

fn load_snapshot(
    connection: &Connection,
    lookup_name: &str,
    host_id: &[u8],
) -> Result<StoredHostSnapshot> {
    let host = load_host_row(connection, host_id)?
        .ok_or(Error::InvalidSnapshot("lookup points to a missing host"))?;
    let merkle = connection.query_row(
        "SELECT h.epoch, h.root_hash, r.root_bytes, h.evidence_kind, \
         h.anchor_epoch, h.evidence_bytes \
         FROM merkle_heads h JOIN merkle_roots r \
         ON r.host_id = h.host_id AND r.epoch = h.epoch WHERE h.host_id = ?1",
        [host_id],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Option<Vec<u8>>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Vec<u8>>(5)?,
            ))
        },
    )?;
    Ok(StoredHostSnapshot {
        lookup_name: lookup_name.to_owned(),
        host_id: host_id.to_vec(),
        canonical_name: host.canonical_name,
        genesis_key: host.genesis_key,
        chain_seqno: host.chain_seqno,
        chain_tail_hash: host.chain_tail_hash,
        chain_bytes: host.chain_bytes,
        public_zone_bytes: host.public_zone_bytes,
        services: load_services(connection, host_id)?,
        merkle_root: StoredMerkleRoot {
            epoch: stored_unsigned("Merkle epoch", merkle.0)?,
            root_hash: fixed_hash(merkle.1, "stored Merkle root has an invalid length")?,
            root_bytes: merkle.2.ok_or(Error::InvalidSnapshot(
                "stored Merkle head has no root bytes",
            ))?,
            evidence: decode_evidence(merkle.3, merkle.4, merkle.5)?,
            authenticated_roots: load_authenticated_roots(connection, host_id)?,
        },
    })
}

fn validate_merkle_root(root: VerifiedMerkleRootParts<'_>) -> Result<()> {
    if root.root_bytes.is_empty() || root.authenticated_roots.is_empty() {
        return Err(Error::InvalidSnapshot(
            "Merkle root must carry bytes and authenticated history",
        ));
    }
    let mut roots = root.authenticated_roots.to_vec();
    roots.sort_unstable_by_key(|entry| entry.epoch);
    if roots.windows(2).any(|pair| pair[0].epoch == pair[1].epoch) {
        return Err(Error::InvalidSnapshot(
            "authenticated Merkle epochs must be unique",
        ));
    }
    let Some(head) = roots.iter().find(|entry| entry.epoch == root.epoch) else {
        return Err(Error::InvalidSnapshot(
            "authenticated Merkle history must contain its head",
        ));
    };
    if head.root_hash != root.root_hash || head.root_bytes.as_deref() != Some(root.root_bytes) {
        return Err(Error::InvalidSnapshot(
            "authenticated Merkle head does not match the snapshot",
        ));
    }
    match &root.evidence {
        MerkleRootEvidence::SignedBootstrap(bytes) if bytes.is_empty() => {
            Err(Error::InvalidSnapshot("signed Merkle evidence is empty"))
        }
        MerkleRootEvidence::SkipPath {
            anchor_epoch,
            historical_response,
            ..
        } if *anchor_epoch >= root.epoch || historical_response.is_empty() => Err(
            Error::InvalidSnapshot("Merkle skip-path evidence is malformed"),
        ),
        MerkleRootEvidence::SkipPath { prior, .. } => validate_evidence_order(prior, root.epoch),
        _ => Ok(()),
    }
}

fn accept_authenticated_roots(
    connection: &Connection,
    host_id: &[u8],
    roots: &[AuthenticatedMerkleRoot],
) -> Result<()> {
    for root in roots {
        let epoch = sqlite_integer("authenticated Merkle epoch", root.epoch)?;
        let existing = connection
            .query_row(
                "SELECT root_hash, root_bytes FROM merkle_roots \
                 WHERE host_id = ?1 AND epoch = ?2",
                params![host_id, epoch],
                |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Option<Vec<u8>>>(1)?)),
            )
            .optional()?;
        if let Some((hash, bytes)) = existing {
            if hash.as_slice() != root.root_hash
                || bytes
                    .as_ref()
                    .zip(root.root_bytes.as_ref())
                    .is_some_and(|(stored, received)| stored != received)
            {
                return Err(Error::MerkleFork { epoch: root.epoch });
            }
            if bytes.is_none() && root.root_bytes.is_some() {
                connection.execute(
                    "UPDATE merkle_roots SET root_bytes = ?3 \
                     WHERE host_id = ?1 AND epoch = ?2",
                    params![host_id, epoch, root.root_bytes],
                )?;
            }
        } else {
            connection.execute(
                "INSERT INTO merkle_roots (host_id, epoch, root_hash, root_bytes) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![host_id, epoch, root.root_hash.as_slice(), root.root_bytes],
            )?;
        }
    }
    Ok(())
}

fn load_authenticated_roots(
    connection: &Connection,
    host_id: &[u8],
) -> Result<Vec<AuthenticatedMerkleRoot>> {
    let mut statement = connection.prepare(
        "SELECT epoch, root_hash, root_bytes FROM merkle_roots \
         WHERE host_id = ?1 ORDER BY epoch",
    )?;
    let rows = statement.query_map([host_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Vec<u8>>(1)?,
            row.get::<_, Option<Vec<u8>>>(2)?,
        ))
    })?;
    rows.map(|row| {
        let (epoch, hash, root_bytes) = row?;
        Ok(AuthenticatedMerkleRoot {
            epoch: stored_unsigned("Merkle epoch", epoch)?,
            root_hash: fixed_hash(hash, "stored Merkle root has an invalid length")?,
            root_bytes,
        })
    })
    .collect()
}

fn validate_evidence_order(evidence: &MerkleRootEvidence, upper: u64) -> Result<()> {
    match evidence {
        MerkleRootEvidence::SignedBootstrap(bytes) if bytes.is_empty() => {
            Err(Error::InvalidSnapshot("signed Merkle evidence is empty"))
        }
        MerkleRootEvidence::SignedBootstrap(_) => Ok(()),
        MerkleRootEvidence::SkipPath {
            anchor_epoch,
            historical_response,
            prior,
        } if *anchor_epoch < upper && !historical_response.is_empty() => {
            validate_evidence_order(prior, *anchor_epoch)
        }
        MerkleRootEvidence::SkipPath { .. } => Err(Error::InvalidSnapshot(
            "Merkle evidence anchors must strictly descend",
        )),
    }
}

fn encode_evidence(evidence: &MerkleRootEvidence) -> Result<(i64, Option<i64>, Vec<u8>)> {
    let (kind, anchor) = match evidence {
        MerkleRootEvidence::SignedBootstrap(_) => (1, None),
        MerkleRootEvidence::SkipPath { anchor_epoch, .. } => (
            2,
            Some(sqlite_integer("Merkle anchor epoch", *anchor_epoch)?),
        ),
    };
    Ok((kind, anchor, encode(&evidence_value(evidence))?))
}

fn evidence_value(evidence: &MerkleRootEvidence) -> Value {
    match evidence {
        MerkleRootEvidence::SignedBootstrap(bytes) => {
            Value::Array(vec![Value::Unsigned(0), Value::Binary(bytes.clone())])
        }
        MerkleRootEvidence::SkipPath {
            anchor_epoch,
            historical_response,
            prior,
        } => Value::Array(vec![
            Value::Unsigned(1),
            Value::Unsigned(*anchor_epoch),
            Value::Binary(historical_response.clone()),
            evidence_value(prior),
        ]),
    }
}

fn decode_evidence(
    kind: i64,
    anchor_epoch: Option<i64>,
    bytes: Vec<u8>,
) -> Result<MerkleRootEvidence> {
    let evidence = evidence_from_value(&decode(&bytes)?, 0)?;
    match (&evidence, kind, anchor_epoch) {
        (MerkleRootEvidence::SignedBootstrap(_), 1, None) => Ok(evidence),
        (MerkleRootEvidence::SkipPath { anchor_epoch, .. }, 2, Some(stored_anchor))
            if *anchor_epoch == stored_unsigned("Merkle anchor epoch", stored_anchor)? =>
        {
            Ok(evidence)
        }
        _ => Err(Error::InvalidSnapshot(
            "stored Merkle evidence has an invalid shape",
        )),
    }
}

fn evidence_from_value(value: &Value, depth: usize) -> Result<MerkleRootEvidence> {
    if depth > 4096 {
        return Err(Error::InvalidSnapshot(
            "persisted Merkle evidence is too deep",
        ));
    }
    let Value::Array(fields) = value else {
        return Err(Error::InvalidSnapshot(
            "persisted Merkle evidence is not an array",
        ));
    };
    match fields.as_slice() {
        [Value::Unsigned(0), Value::Binary(bytes)] => {
            Ok(MerkleRootEvidence::SignedBootstrap(bytes.clone()))
        }
        [Value::Unsigned(1), Value::Unsigned(anchor_epoch), Value::Binary(historical_response), prior] => {
            Ok(MerkleRootEvidence::SkipPath {
                anchor_epoch: *anchor_epoch,
                historical_response: historical_response.clone(),
                prior: Box::new(evidence_from_value(prior, depth + 1)?),
            })
        }
        _ => Err(Error::InvalidSnapshot(
            "persisted Merkle evidence has an invalid shape",
        )),
    }
}

fn fixed_hash(bytes: Vec<u8>, error: &'static str) -> Result<[u8; 32]> {
    bytes.try_into().map_err(|_| Error::InvalidSnapshot(error))
}

fn encoded_array_is_prefix(stored: &[u8], received: &[u8]) -> Result<bool> {
    let Value::Array(stored) = decode(stored)? else {
        return Err(Error::InvalidSnapshot(
            "stored chain is not an encoded array",
        ));
    };
    let Value::Array(received) = decode(received)? else {
        return Err(Error::InvalidSnapshot(
            "received chain is not an encoded array",
        ));
    };
    Ok(stored.len() <= received.len() && stored.iter().zip(&received).all(|(a, b)| a == b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_schema_error_describes_failed_auto_update() {
        let error = Error::UnsupportedSchema {
            found: 23,
            supported: 27,
        };

        assert_eq!(
            error.to_string(),
            "system store is out of date (v23), could not auto-update to current version (v27)"
        );
    }

    fn chain(values: &[u64]) -> Vec<u8> {
        encode(&Value::Array(
            values.iter().copied().map(Value::Unsigned).collect(),
        ))
        .unwrap()
    }

    #[derive(Clone)]
    struct TestMerkleRoot {
        epoch: u64,
        root_hash: [u8; 32],
        root_bytes: Vec<u8>,
        evidence: MerkleRootEvidence,
        authenticated_roots: Vec<AuthenticatedMerkleRoot>,
    }

    impl TestMerkleRoot {
        fn parts(&self) -> VerifiedMerkleRootParts<'_> {
            VerifiedMerkleRootParts {
                epoch: self.epoch,
                root_hash: self.root_hash,
                root_bytes: &self.root_bytes,
                evidence: &self.evidence,
                authenticated_roots: &self.authenticated_roots,
            }
        }
    }

    #[derive(Clone)]
    struct TestHostSnapshot {
        lookup_name: String,
        host_id: Vec<u8>,
        canonical_name: String,
        genesis_key: Vec<u8>,
        chain_seqno: u64,
        chain_tail_hash: [u8; 32],
        chain_bytes: Vec<u8>,
        public_zone_bytes: Vec<u8>,
        services: Vec<HostService>,
        merkle_root: TestMerkleRoot,
    }

    #[derive(Clone)]
    struct TestUserSnapshot {
        host_id: Vec<u8>,
        uid: Vec<u8>,
        chain_seqno: u64,
        chain_tail_hash: [u8; 32],
        chain_bytes: Vec<u8>,
        evidence_bytes: Vec<u8>,
        username: Vec<u8>,
        username_utf8: Vec<u8>,
        username_sequence: u64,
        merkle_epoch: u64,
        merkle_root_hash: [u8; 32],
        merkle_root_bytes: Vec<u8>,
        devices: Vec<foks_verify::VerifiedUserDevice>,
        shared_keys: Vec<foks_verify::VerifiedUserSharedKey>,
    }

    impl TestUserSnapshot {
        fn parts(&self) -> VerifiedUserSnapshotParts<'_> {
            VerifiedUserSnapshotParts {
                host_id: &self.host_id,
                uid: &self.uid,
                chain_seqno: self.chain_seqno,
                chain_tail_hash: self.chain_tail_hash,
                chain_bytes: &self.chain_bytes,
                evidence_bytes: &self.evidence_bytes,
                username: &self.username,
                username_utf8: &self.username_utf8,
                username_sequence: self.username_sequence,
                merkle_epoch: self.merkle_epoch,
                merkle_root_hash: self.merkle_root_hash,
                merkle_root_bytes: &self.merkle_root_bytes,
                devices: &self.devices,
                shared_keys: &self.shared_keys,
            }
        }
    }

    impl TestHostSnapshot {
        fn parts(&self) -> VerifiedHostSnapshotParts<'_> {
            VerifiedHostSnapshotParts {
                lookup_name: &self.lookup_name,
                host_id: &self.host_id,
                canonical_name: &self.canonical_name,
                genesis_key: &self.genesis_key,
                chain_seqno: self.chain_seqno,
                chain_tail_hash: self.chain_tail_hash,
                chain_bytes: &self.chain_bytes,
                public_zone_bytes: &self.public_zone_bytes,
                services: &self.services,
                merkle_root: self.merkle_root.parts(),
            }
        }
    }

    #[test]
    fn stored_unsigned_values_reject_negative_rows() {
        assert!(matches!(
            stored_unsigned("test", -1),
            Err(Error::StoredIntegerOutOfRange {
                field: "test",
                value: -1
            })
        ));
    }

    fn snapshot() -> TestHostSnapshot {
        TestHostSnapshot {
            lookup_name: "foks.example".into(),
            host_id: vec![1; 33],
            canonical_name: "foks.example".into(),
            genesis_key: vec![2; 32],
            chain_seqno: 4,
            chain_tail_hash: [3; 32],
            chain_bytes: chain(&[1, 2, 3, 4]),
            public_zone_bytes: vec![4; 96],
            services: vec![
                HostService {
                    service_type: 2,
                    endpoint_bytes: vec![6; 24],
                },
                HostService {
                    service_type: 7,
                    endpoint_bytes: vec![5; 20],
                },
            ],
            merkle_root: TestMerkleRoot {
                epoch: 11,
                root_hash: [7; 32],
                root_bytes: vec![9; 80],
                evidence: MerkleRootEvidence::SignedBootstrap(vec![8; 96]),
                authenticated_roots: vec![AuthenticatedMerkleRoot {
                    epoch: 11,
                    root_hash: [7; 32],
                    root_bytes: Some(vec![9; 80]),
                }],
            },
        }
    }

    fn store() -> (tempfile::TempDir, HardStateStore) {
        let directory = tempfile::tempdir().unwrap();
        let store = HardStateStore::open(&directory.path().join("hard.sqlite")).unwrap();
        (directory, store)
    }

    fn stored(snapshot: &TestHostSnapshot) -> StoredHostSnapshot {
        let parts = snapshot.parts();
        let root = parts.merkle_root;
        StoredHostSnapshot {
            lookup_name: parts.lookup_name.to_owned(),
            host_id: parts.host_id.to_vec(),
            canonical_name: parts.canonical_name.to_owned(),
            genesis_key: parts.genesis_key.to_vec(),
            chain_seqno: parts.chain_seqno,
            chain_tail_hash: parts.chain_tail_hash,
            chain_bytes: parts.chain_bytes.to_vec(),
            public_zone_bytes: parts.public_zone_bytes.to_vec(),
            services: parts.services.to_vec(),
            merkle_root: StoredMerkleRoot {
                epoch: root.epoch,
                root_hash: root.root_hash,
                root_bytes: root.root_bytes.to_vec(),
                evidence: root.evidence.clone(),
                authenticated_roots: root.authenticated_roots.to_vec(),
            },
        }
    }

    fn user_snapshot() -> TestUserSnapshot {
        TestUserSnapshot {
            host_id: vec![1; 33],
            uid: vec![1; 33],
            chain_seqno: 1,
            chain_tail_hash: [21; 32],
            chain_bytes: chain(&[21]),
            evidence_bytes: vec![22; 80],
            username: b"fixtureuser".to_vec(),
            username_utf8: b"FixtureUser".to_vec(),
            username_sequence: 1,
            merkle_epoch: 11,
            merkle_root_hash: [7; 32],
            merkle_root_bytes: vec![9; 80],
            devices: vec![foks_verify::VerifiedUserDevice {
                device_id: vec![4; 33],
                role: foks_proto::Role::OWNER,
                hepk_bytes: vec![25; 1280],
                subkey_id: None,
            }],
            shared_keys: vec![foks_verify::VerifiedUserSharedKey {
                role: foks_proto::Role::OWNER,
                generation: 1,
                verify_key: vec![14; 33],
                hepk_bytes: vec![26; 1280],
            }],
        }
    }

    #[test]
    fn exact_authenticated_bytes_round_trip() {
        let (_directory, mut store) = store();
        let snapshot = snapshot();
        assert_eq!(
            store.accept_host_parts(snapshot.parts()).unwrap(),
            Acceptance::Inserted
        );
        assert_eq!(
            store.accept_host_parts(snapshot.parts()).unwrap(),
            Acceptance::Unchanged
        );
        assert_eq!(
            store.host_for_lookup("foks.example").unwrap(),
            Some(stored(&snapshot))
        );
    }

    #[test]
    fn discovery_name_cannot_change_host_identity() {
        let (_directory, mut store) = store();
        store.accept_host_parts(snapshot().parts()).unwrap();
        let mut replacement = snapshot();
        replacement.host_id = vec![9; 33];
        assert!(matches!(
            store.accept_host_parts(replacement.parts()),
            Err(Error::HostIdentityChanged { .. })
        ));
    }

    #[test]
    fn same_sequence_chain_fork_is_rejected() {
        let (_directory, mut store) = store();
        store.accept_host_parts(snapshot().parts()).unwrap();
        let mut fork = snapshot();
        fork.chain_seqno = 4;
        fork.chain_tail_hash = [9; 32];
        fork.chain_bytes = chain(&[1, 2, 3, 9]);
        assert!(matches!(
            store.accept_host_parts(fork.parts()),
            Err(Error::ChainFork { seqno: 4 })
        ));
    }

    #[test]
    fn chain_rollback_is_rejected() {
        let (_directory, mut store) = store();
        store.accept_host_parts(snapshot().parts()).unwrap();
        let mut rollback = snapshot();
        rollback.chain_seqno = 3;
        rollback.chain_tail_hash = [3; 32];
        rollback.chain_bytes = chain(&[1, 2, 3]);
        assert!(matches!(
            store.accept_host_parts(rollback.parts()),
            Err(Error::ChainRollback {
                stored: 4,
                received: 3
            })
        ));
    }

    #[test]
    fn merkle_fork_and_rollback_are_rejected() {
        let (_directory, mut store) = store();
        store.accept_host_parts(snapshot().parts()).unwrap();

        let mut fork = snapshot();
        fork.merkle_root.root_hash = [9; 32];
        fork.merkle_root.authenticated_roots[0].root_hash = [9; 32];
        assert!(matches!(
            store.accept_host_parts(fork.parts()),
            Err(Error::MerkleFork { epoch: 11 })
        ));

        let mut rollback = snapshot();
        rollback.merkle_root.epoch = 10;
        rollback.merkle_root.authenticated_roots[0].epoch = 10;
        assert!(matches!(
            store.accept_host_parts(rollback.parts()),
            Err(Error::MerkleRollback {
                stored: 11,
                received: 10
            })
        ));
    }

    #[test]
    fn stale_merkle_root_rolls_back_a_chain_update_atomically() {
        let (_directory, mut store) = store();
        let original = snapshot();
        store.accept_host_parts(original.parts()).unwrap();

        let mut update = snapshot();
        update.chain_seqno = 5;
        update.chain_tail_hash = [10; 32];
        update.chain_bytes = chain(&[1, 2, 3, 4, 10]);
        update.services[0].endpoint_bytes = vec![11; 20];
        update.merkle_root.epoch = 10;
        update.merkle_root.authenticated_roots[0].epoch = 10;
        assert!(matches!(
            store.accept_host_parts(update.parts()),
            Err(Error::MerkleRollback { .. })
        ));
        assert_eq!(
            store.host_for_lookup("foks.example").unwrap(),
            Some(stored(&original))
        );
    }

    #[test]
    fn chain_and_merkle_advance_together() {
        let (_directory, mut store) = store();
        store.accept_host_parts(snapshot().parts()).unwrap();
        let mut update = snapshot();
        update.canonical_name = "new.foks.example".into();
        update.chain_seqno = 5;
        update.chain_tail_hash = [10; 32];
        update.chain_bytes = chain(&[1, 2, 3, 4, 10]);
        update.services[0].endpoint_bytes = vec![11; 20];
        update.merkle_root.epoch = 12;
        update.merkle_root.root_hash = [12; 32];
        update.merkle_root.root_bytes = vec![13; 80];
        update.merkle_root.evidence = MerkleRootEvidence::SkipPath {
            anchor_epoch: 11,
            historical_response: vec![12; 96],
            prior: Box::new(snapshot().merkle_root.evidence),
        };
        update
            .merkle_root
            .authenticated_roots
            .push(AuthenticatedMerkleRoot {
                epoch: 12,
                root_hash: [12; 32],
                root_bytes: Some(vec![13; 80]),
            });
        assert_eq!(
            store.accept_host_parts(update.parts()).unwrap(),
            Acceptance::Advanced
        );
        let loaded = store.host_for_lookup("foks.example").unwrap().unwrap();
        assert_eq!(loaded, stored(&update));
    }

    #[test]
    fn projection_cannot_change_without_chain_advance() {
        let (_directory, mut store) = store();
        store.accept_host_parts(snapshot().parts()).unwrap();
        let mut changed = snapshot();
        changed.services[0].endpoint_bytes = vec![12; 20];
        assert!(matches!(
            store.accept_host_parts(changed.parts()),
            Err(Error::ProjectionChanged { seqno: 4 })
        ));
    }

    #[test]
    fn database_reopens_without_losing_pins() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("hard.sqlite");
        {
            let mut store = HardStateStore::open(&path).unwrap();
            store.accept_host_parts(snapshot().parts()).unwrap();
        }
        let store = HardStateStore::open(&path).unwrap();
        assert_eq!(
            store.host_for_lookup("foks.example").unwrap(),
            Some(stored(&snapshot()))
        );
    }

    #[test]
    fn user_projection_is_atomic_and_monotonic() {
        let (_directory, mut store) = store();
        store.accept_host_parts(snapshot().parts()).unwrap();
        let user = user_snapshot();
        assert_eq!(
            store.accept_user_parts(user.parts()).unwrap(),
            Acceptance::Inserted
        );
        assert_eq!(
            store.accept_user_parts(user.parts()).unwrap(),
            Acceptance::Unchanged
        );

        let mut fork = user.clone();
        let mut fork_hash = fork.chain_tail_hash;
        fork_hash[0] ^= 1;
        fork.chain_tail_hash = fork_hash;
        assert!(matches!(
            store.accept_user_parts(fork.parts()),
            Err(Error::UserFork { seqno: 1 })
        ));

        let mut changed_projection = user.clone();
        let mut changed_hash = changed_projection.merkle_root_hash;
        changed_hash[0] ^= 1;
        changed_projection.merkle_root_hash = changed_hash;
        assert!(matches!(
            store.accept_user_parts(changed_projection.parts()),
            Err(Error::InvalidUser(_))
        ));

        let mut changed_device = user;
        changed_device.devices[0].hepk_bytes[0] ^= 1;
        assert!(matches!(
            store.accept_user_parts(changed_device.parts()),
            Err(Error::UserProjectionChanged { seqno: 1 })
        ));
    }

    #[test]
    fn yubi_device_ids_are_valid_durable_user_state() {
        let (_directory, mut store) = store();
        store.accept_host_parts(snapshot().parts()).unwrap();
        let mut user = user_snapshot();
        user.devices[0].device_id = vec![8; 34];
        assert_eq!(
            store.accept_user_parts(user.parts()).unwrap(),
            Acceptance::Inserted
        );
        assert_eq!(
            store.accept_user_parts(user.parts()).unwrap(),
            Acceptance::Unchanged
        );
    }

    #[test]
    fn longer_chain_forks_are_rejected() {
        let (_directory, mut store) = store();
        store.accept_host_parts(snapshot().parts()).unwrap();

        let mut host_fork = snapshot();
        host_fork.chain_seqno = 5;
        host_fork.chain_tail_hash = [44; 32];
        host_fork.chain_bytes = chain(&[1, 2, 9, 4, 5]);
        assert!(matches!(
            store.accept_host_parts(host_fork.parts()),
            Err(Error::ChainFork { seqno: 5 })
        ));

        let user = user_snapshot();
        store.accept_user_parts(user.parts()).unwrap();
        let mut user_fork = user;
        user_fork.chain_seqno = 2;
        user_fork.chain_tail_hash = [45; 32];
        user_fork.chain_bytes = chain(&[99, 2]);
        assert!(matches!(
            store.accept_user_parts(user_fork.parts()),
            Err(Error::UserFork { seqno: 2 })
        ));
    }

    #[test]
    fn unchanged_user_chain_can_be_reauthenticated_at_a_new_root() {
        let (_directory, mut store) = store();
        let mut host = snapshot();
        store.accept_host_parts(host.parts()).unwrap();
        let mut user = user_snapshot();
        store.accept_user_parts(user.parts()).unwrap();

        host.merkle_root.epoch = 12;
        host.merkle_root.root_hash = [12; 32];
        host.merkle_root.root_bytes = vec![13; 80];
        host.merkle_root.evidence = MerkleRootEvidence::SkipPath {
            anchor_epoch: 11,
            historical_response: vec![12; 96],
            prior: Box::new(snapshot().merkle_root.evidence),
        };
        host.merkle_root
            .authenticated_roots
            .push(AuthenticatedMerkleRoot {
                epoch: 12,
                root_hash: [12; 32],
                root_bytes: Some(vec![13; 80]),
            });
        store.accept_host_parts(host.parts()).unwrap();

        user.merkle_epoch = 12;
        user.merkle_root_hash = [12; 32];
        user.merkle_root_bytes = vec![13; 80];
        assert_eq!(
            store.accept_user_parts(user.parts()).unwrap(),
            Acceptance::Advanced
        );
        assert_eq!(
            store.accept_user_parts(user.parts()).unwrap(),
            Acceptance::Unchanged
        );
    }
}
