//! Transaction-local personal authorization for the qualified hosted profile.
//!
//! The frontend, not this module, proves TLS possession and parses the exact
//! certificate. No result here commits, publishes, or establishes a tenant fence.
pub(crate) mod catalog;
mod generations;
mod list;
pub use generations::current_key_generations;
pub use list::{read_personal_list, ListCursor, ListRequest};

use crate::{Sql, Statement as S, Value as V};
use foks_proto::{EntityId, Role, RoleAndGeneration};
use serde::{Deserialize, Serialize};

pub const MAX_PEER_CERTIFICATE_BYTES: usize = 4096;

/// Structurally validated trusted-frontend inputs, not a TLS proof. Construction
/// neither parses DER nor accepts an asserted user identity or role.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PeerCertificate {
    exact_der: Vec<u8>,
    serial_magnitude: Vec<u8>,
    public_key: [u8; 32],
    #[serde(with = "micros")]
    expires_at_micros: u64,
}
impl PeerCertificate {
    pub fn from_frontend_parts(
        exact_der: Vec<u8>,
        canonical_serial_magnitude: Vec<u8>,
        ed25519_public_key: [u8; 32],
        parsed_expiry_micros: u64,
    ) -> crate::Result<Self> {
        if exact_der.is_empty() || exact_der.len() > MAX_PEER_CERTIFICATE_BYTES {
            return Err(crate::Error::Bounds);
        }
        if canonical_serial_magnitude.is_empty()
            || canonical_serial_magnitude.len() > 20
            || (canonical_serial_magnitude.len() > 1 && canonical_serial_magnitude[0] == 0)
        {
            return Err(crate::Error::Invalid("canonical certificate serial"));
        }
        crate::integer(parsed_expiry_micros)?;
        Ok(Self {
            exact_der,
            serial_magnitude: canonical_serial_magnitude,
            public_key: ed25519_public_key,
            expires_at_micros: parsed_expiry_micros,
        })
    }
    pub fn exact_der(&self) -> &[u8] {
        &self.exact_der
    }
    pub fn serial_magnitude(&self) -> &[u8] {
        &self.serial_magnitude
    }
    pub fn public_key(&self) -> &[u8; 32] {
        &self.public_key
    }
    pub fn expires_at_micros(&self) -> u64 {
        self.expires_at_micros
    }
}
impl<'de> Deserialize<'de> for PeerCertificate {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Parts {
            exact_der: Vec<u8>,
            serial_magnitude: Vec<u8>,
            public_key: [u8; 32],
            #[serde(with = "micros")]
            expires_at_micros: u64,
        }
        let parts = Parts::deserialize(deserializer)?;
        Self::from_frontend_parts(
            parts.exact_der,
            parts.serial_magnitude,
            parts.public_key,
            parts.expires_at_micros,
        )
        .map_err(|_| serde::de::Error::custom("invalid hosted peer certificate parts"))
    }
}
mod micros {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(
        value: &u64,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<u64, D::Error> {
        let text = String::deserialize(deserializer)?;
        let value = text.parse::<u64>().map_err(serde::de::Error::custom)?;
        if value.to_string() != text || value > i64::MAX as u64 {
            return Err(serde::de::Error::custom("noncanonical certificate time"));
        }
        Ok(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialKind {
    SoftwareDevice,
    DelegatedSubkey,
    BackupKey,
    BotToken,
}
/// Only SQL authentication constructs this value. It cannot be deserialized from
/// caller-selected UID/role fields. Reauthenticate after leaving its transaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CurrentCredential {
    uid: [u8; 33],
    credential: EntityId,
    owner: EntityId,
    role: Role,
    kind: CredentialKind,
    expires_at_micros: u64,
}
impl CurrentCredential {
    pub fn uid(&self) -> &[u8; 33] {
        &self.uid
    }
    pub fn credential_id(&self) -> &[u8] {
        self.credential.as_bytes()
    }
    pub fn owner_id(&self) -> &[u8] {
        self.owner.as_bytes()
    }
    pub fn role(&self) -> Role {
        self.role
    }
    pub fn kind(&self) -> CredentialKind {
        self.kind
    }
    pub fn expires_at_micros(&self) -> u64 {
        self.expires_at_micros
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PersonalError {
    Unauthenticated,
    Permission,
    Missing,
    UnsupportedProfile,
    Corrupt,
    BadArguments,
    StalePreparation,
    CachedObjectMissing,
    StaleCache(foks_proto::KvPathVersionVector),
    Kernel(crate::Error),
}
pub type Result<T> = std::result::Result<T, PersonalError>;
impl From<crate::Error> for PersonalError {
    fn from(error: crate::Error) -> Self {
        Self::Kernel(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyAccess {
    Read,
    Write,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyRequirement {
    pub key: RoleAndGeneration,
    pub access: KeyAccess,
}

fn blob(value: &[u8]) -> V {
    V::Blob(value.to_vec())
}
fn stored_role(kind: &V, visibility: &V) -> Result<Role> {
    let (V::Integer(kind), V::Integer(visibility)) = (kind, visibility) else {
        return Err(PersonalError::Corrupt);
    };
    match (*kind, *visibility) {
        (1, v) => i16::try_from(v)
            .map(Role::member)
            .map_err(|_| PersonalError::Corrupt),
        (2, 0) => Ok(Role::ADMIN),
        (3, 0) => Ok(Role::OWNER),
        _ => Err(PersonalError::Corrupt),
    }
}
fn bytes(value: &V) -> Result<&[u8]> {
    if let V::Blob(value) = value {
        Ok(value)
    } else {
        Err(PersonalError::Corrupt)
    }
}
fn unsigned(value: &V) -> Result<u64> {
    crate::unsigned(value).map_err(|_| PersonalError::Corrupt)
}

/// Requires the current trusted frontend's TLS proof, and an already-open SQL
/// transaction. The bounded zero-padding probes reproduce serial storage that
/// predates DER's positive-integer normalization without a certificate-table scan.
pub fn authenticate_peer(
    sql: &mut impl Sql,
    peer: &PeerCertificate,
    now_micros: u64,
) -> Result<CurrentCredential> {
    let now = crate::integer(now_micros)?;
    if now_micros >= peer.expires_at_micros {
        return Err(PersonalError::Unauthenticated);
    }
    let mut found = None;
    for width in peer.serial_magnitude.len()..=20 {
        let mut serial = vec![0; width];
        serial[width - peer.serial_magnitude.len()..].copy_from_slice(&peer.serial_magnitude);
        let row = sql.query(
            S::PersonalIssuedCredential,
            &[V::Blob(serial), blob(peer.exact_der()), now.clone()],
        )?;
        let Some(row) = row else { continue };
        if found.is_some() {
            return Err(PersonalError::Corrupt);
        }
        let [uid, credential, owner, kind, visibility] = row.as_slice() else {
            return Err(PersonalError::Corrupt);
        };
        let uid: [u8; 33] = bytes(uid)?.try_into().map_err(|_| PersonalError::Corrupt)?;
        if uid[0] != foks_proto::ENTITY_USER {
            return Err(PersonalError::Corrupt);
        }
        let credential = EntityId::from_bytes(bytes(credential)?.to_vec())
            .map_err(|_| PersonalError::Corrupt)?;
        let owner =
            EntityId::from_bytes(bytes(owner)?.to_vec()).map_err(|_| PersonalError::Corrupt)?;
        if credential
            .ed25519_key()
            .map_err(|_| PersonalError::Corrupt)?
            != *peer.public_key()
        {
            return Err(PersonalError::Unauthenticated);
        }
        let role = stored_role(kind, visibility)?;
        let kind = match credential.entity_type() {
            foks_proto::ENTITY_DEVICE => CredentialKind::SoftwareDevice,
            foks_proto::ENTITY_SUBKEY => CredentialKind::DelegatedSubkey,
            foks_proto::ENTITY_BACKUP_KEY => CredentialKind::BackupKey,
            foks_proto::ENTITY_BOT_TOKEN_KEY => CredentialKind::BotToken,
            _ => return Err(PersonalError::Unauthenticated),
        };
        if (kind == CredentialKind::DelegatedSubkey
            && owner.entity_type() != foks_proto::ENTITY_YUBI)
            || (kind != CredentialKind::DelegatedSubkey && owner != credential)
        {
            return Err(PersonalError::Corrupt);
        }
        found = Some(CurrentCredential {
            uid,
            credential,
            owner,
            role,
            kind,
            expires_at_micros: peer.expires_at_micros,
        });
    }
    found.ok_or(PersonalError::Unauthenticated)
}

/// Rechecks this canonical owner and actual TLS credential on the same row.
/// This deliberately rejects historical delegated-key rebinding across owners.
pub fn recheck_current_credential(
    sql: &mut impl Sql,
    credential: &CurrentCredential,
    now_micros: u64,
) -> Result<()> {
    if now_micros >= credential.expires_at_micros {
        return Err(PersonalError::Unauthenticated);
    }
    let row = sql
        .query(
            S::PersonalCurrentCredential,
            &[
                blob(credential.owner_id()),
                blob(credential.uid()),
                blob(credential.credential_id()),
            ],
        )?
        .ok_or(PersonalError::Permission)?;
    let [kind, visibility] = row.as_slice() else {
        return Err(PersonalError::Corrupt);
    };
    if stored_role(kind, visibility)? != credential.role {
        return Err(PersonalError::StalePreparation);
    }
    Ok(())
}

pub fn require_personal_profile(sql: &mut impl Sql, credential: &CurrentCredential) -> Result<()> {
    if credential.kind == CredentialKind::BackupKey {
        return Err(PersonalError::Permission);
    }
    if sql.query(S::PersonalSsoPolicy, &[])?.is_some() {
        return Err(PersonalError::UnsupportedProfile);
    }
    if sql
        .query(S::PersonalAnchor, &[blob(credential.uid())])?
        .is_none()
    {
        return Err(PersonalError::Permission);
    }
    Ok(())
}

/// Bootstrap must validate all stored role encodings before this targeted path
/// is enabled; supported typed mutations preserve that invariant afterward.
pub fn current_key_generation(sql: &mut impl Sql, uid: &[u8], role: Role) -> Result<Option<u64>> {
    if uid.len() != 33 || uid[0] != foks_proto::ENTITY_USER || role == Role::NONE {
        return Err(PersonalError::BadArguments);
    }
    sql.query(
        S::PersonalRoleGeneration,
        &[
            blob(uid),
            crate::integer(role.protocol_value())?,
            V::Integer(i64::from(role.visibility().unwrap_or(0))),
        ],
    )?
    .map(|row| {
        let [value] = row.as_slice() else {
            return Err(PersonalError::Corrupt);
        };
        unsigned(value)
    })
    .transpose()
}

pub fn require_key(
    sql: &mut impl Sql,
    credential: &CurrentCredential,
    requirement: KeyRequirement,
) -> Result<()> {
    if requirement.key.role == Role::NONE || requirement.key.role > credential.role {
        return Err(PersonalError::Permission);
    }
    let current = current_key_generation(sql, credential.uid(), requirement.key.role)?;
    let permitted = match requirement.access {
        KeyAccess::Read => {
            requirement.key.generation > 0
                && current.is_some_and(|v| requirement.key.generation <= v)
        }
        KeyAccess::Write => current == Some(requirement.key.generation),
    };
    if permitted {
        Ok(())
    } else {
        Err(PersonalError::Permission)
    }
}

/// Scalar operations only. Public Get alternatives/List bundles require the A2
/// bounded page ABI; these primitives do not silently lower their public limits.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PersonalPoint {
    Root {},
    Directory {
        id: [u8; 16],
    },
    Node {
        id: [u8; 17],
    },
    /// Internal namespace charging counters, not the public KvUsage RPC.
    QuotaCounters {},
    /// Public ciphertext/file/chunk semantics, encoded as the existing KvUsage.
    PublicUsage {},
}

fn exact_row(row: Vec<V>) -> Result<(u64, Vec<u8>)> {
    let [tag, length, exact] = row.as_slice() else {
        return Err(PersonalError::Corrupt);
    };
    let length = unsigned(length)?;
    if length > crate::MAX_BLOB_BYTES as u64 {
        return Err(PersonalError::UnsupportedProfile);
    }
    let exact = bytes(exact)?;
    if exact.len() as u64 != length {
        return Err(PersonalError::Corrupt);
    }
    Ok((unsigned(tag)?, exact.to_vec()))
}
fn root(
    sql: &mut impl Sql,
    credential: &CurrentCredential,
) -> Result<(foks_proto::KvRoot, Vec<u8>)> {
    let row = sql
        .query(S::PersonalRoot, &[blob(credential.uid())])?
        .ok_or(PersonalError::Missing)?;
    let (version, exact) = exact_row(row)?;
    let root = foks_proto::KvRoot::decode(&exact).map_err(|_| PersonalError::Corrupt)?;
    if root.version != version {
        return Err(PersonalError::Corrupt);
    }
    require_key(
        sql,
        credential,
        KeyRequirement {
            key: root.key,
            access: KeyAccess::Read,
        },
    )?;
    Ok((root, exact))
}
fn directory(
    sql: &mut impl Sql,
    credential: &CurrentCredential,
    id: &[u8; 16],
) -> Result<foks_proto::KvDirectory> {
    let row = sql
        .query(S::PersonalDirectory, &[blob(credential.uid()), blob(id)])?
        .ok_or(PersonalError::Missing)?;
    let (version, exact) = exact_row(row)?;
    let directory = foks_proto::KvDirectory::decode(&exact).map_err(|_| PersonalError::Corrupt)?;
    if directory.id != *id || directory.version != version {
        return Err(PersonalError::Corrupt);
    }
    require_key(
        sql,
        credential,
        KeyRequirement {
            key: directory.key,
            access: KeyAccess::Read,
        },
    )?;
    Ok(directory)
}

pub fn read_personal_point(
    sql: &mut impl Sql,
    credential: &CurrentCredential,
    point: &PersonalPoint,
    now_micros: u64,
) -> Result<crate::wire::Reply> {
    recheck_current_credential(sql, credential, now_micros)?;
    require_personal_profile(sql, credential)?;
    let exact = match point {
        PersonalPoint::Root {} => root(sql, credential)?.1,
        PersonalPoint::Directory { id } => {
            foks_proto::KvDirectoryPair::from_active(directory(sql, credential, id)?)
                .map_err(|_| PersonalError::Corrupt)?
                .encoded()
                .to_vec()
        }
        PersonalPoint::Node { id } => {
            let node = match id[0] {
                1 => foks_proto::KvNode::Directory(
                    foks_proto::KvDirectoryPair::from_active(directory(
                        sql,
                        credential,
                        id[1..].try_into().map_err(|_| PersonalError::Corrupt)?,
                    )?)
                    .map_err(|_| PersonalError::Corrupt)?,
                ),
                2 => return Err(PersonalError::UnsupportedProfile),
                3 | 4 => {
                    let row = sql
                        .query(S::PersonalSmallNode, &[blob(credential.uid()), blob(id)])?
                        .ok_or(PersonalError::Missing)?;
                    let (kind, exact) = exact_row(row)?;
                    if kind != u64::from(id[0]) {
                        return Err(PersonalError::Corrupt);
                    }
                    let boxed = foks_proto::KvSmallFileBox::decode(&exact)
                        .map_err(|_| PersonalError::Corrupt)?;
                    require_key(
                        sql,
                        credential,
                        KeyRequirement {
                            key: boxed.key,
                            access: KeyAccess::Read,
                        },
                    )?;
                    if kind == 3 {
                        foks_proto::KvNode::SmallFile(boxed)
                    } else {
                        foks_proto::KvNode::Symlink(boxed)
                    }
                }
                _ => return Err(PersonalError::Missing),
            };
            node.encoded().map_err(|_| PersonalError::Corrupt)?
        }
        PersonalPoint::QuotaCounters {} => {
            let values = sql
                .query(S::Usage, &[blob(credential.uid())])?
                .unwrap_or_else(|| vec![V::Integer(0); 4]);
            if values.len() != 4 || values.iter().any(|v| unsigned(v).is_err()) {
                return Err(PersonalError::Corrupt);
            }
            return Ok(crate::wire::Reply { values });
        }
        PersonalPoint::PublicUsage {} => {
            let values = sql
                .query(S::PublicUsage, &[blob(credential.uid())])?
                .unwrap_or_else(|| vec![V::Integer(0); 6]);
            let [small_count, small_bytes, large_count, large_bytes, large_chunks, invalid] =
                values.as_slice()
            else {
                return Err(PersonalError::Corrupt);
            };
            if unsigned(invalid)? != 0 {
                return Err(PersonalError::Corrupt);
            }
            let usage = foks_proto::KvUsage {
                small: foks_proto::KvUsageStats {
                    number: unsigned(small_count)?,
                    bytes: unsigned(small_bytes)?,
                },
                large: foks_proto::KvChunkedUsageStats {
                    base: foks_proto::KvUsageStats {
                        number: unsigned(large_count)?,
                        bytes: unsigned(large_bytes)?,
                    },
                    chunks: unsigned(large_chunks)?,
                },
            };
            foks_snowpack::encode(&usage.to_value()).map_err(|_| PersonalError::Corrupt)?
        }
    };
    if exact.len() > crate::MAX_BLOB_BYTES {
        return Err(PersonalError::UnsupportedProfile);
    }
    Ok(crate::wire::Reply {
        values: vec![V::Blob(exact)],
    })
}

fn checked_version(sql: &mut impl Sql, statement: S, parameters: &[V]) -> Result<u64> {
    let row = sql
        .query(statement, parameters)?
        .ok_or(PersonalError::CachedObjectMissing)?;
    let [value] = row.as_slice() else {
        return Err(PersonalError::Corrupt);
    };
    unsigned(value)
}
/// Mirrors native service precondition authorization and stale/future/missing
/// ordering; the lower-level mutation kernel still checks its write precondition.
pub fn check_personal_precondition(
    sql: &mut impl Sql,
    credential: &CurrentCredential,
    supplied: Option<&foks_proto::KvPathVersionVector>,
) -> Result<()> {
    let Some(supplied) = supplied else {
        return Ok(());
    };
    if supplied.directories.len() > 256
        || supplied
            .directories
            .iter()
            .map(|d| d.entries.len())
            .sum::<usize>()
            > 1024
    {
        return Err(PersonalError::UnsupportedProfile);
    }
    for d in &supplied.directories {
        directory(sql, credential, &d.id).map_err(|error| {
            if error == PersonalError::Missing {
                PersonalError::CachedObjectMissing
            } else {
                error
            }
        })?;
    }
    let current_root = root(sql, credential)
        .map_err(|error| {
            if error == PersonalError::Missing {
                PersonalError::CachedObjectMissing
            } else {
                error
            }
        })?
        .0;
    if current_root.version < supplied.root_version {
        return Err(PersonalError::BadArguments);
    }
    let mut stale_directories = Vec::new();
    for d in &supplied.directories {
        let current = checked_version(
            sql,
            S::DirectoryVersion,
            &[blob(credential.uid()), blob(&d.id)],
        )?;
        if current < d.version {
            return Err(PersonalError::BadArguments);
        }
        let mut stale_entries = Vec::new();
        for e in &d.entries {
            let version = checked_version(
                sql,
                S::EntryVersion,
                &[blob(credential.uid()), blob(&d.id), blob(&e.id)],
            )?;
            if version < e.version {
                return Err(PersonalError::BadArguments);
            }
            if version > e.version {
                stale_entries.push(foks_proto::KvDirentVersion { id: e.id, version });
            }
        }
        if current > d.version || !stale_entries.is_empty() {
            stale_directories.push(foks_proto::KvDirectoryVersion {
                id: d.id,
                version: current,
                entries: stale_entries,
            });
        }
    }
    if (supplied.root_version != 0 && current_root.version != supplied.root_version)
        || !stale_directories.is_empty()
    {
        return Err(PersonalError::StaleCache(foks_proto::KvPathVersionVector {
            root_version: current_root.version,
            directories: stale_directories,
        }));
    }
    Ok(())
}

/// Derives every permission from the parsed operation and current stored rows.
/// Caller must freshly authenticate the peer and validate the actual tenant
/// fence/domain revision in this same transaction before invoking this hook.
pub fn authorize_personal_put(
    sql: &mut impl Sql,
    credential: &CurrentCredential,
    command: &crate::wire::Command,
) -> Result<()> {
    let crate::wire::Command::PutDirents {
        uid,
        precondition,
        role,
        mutations,
        ..
    } = command
    else {
        return Err(PersonalError::UnsupportedProfile);
    };
    require_personal_profile(sql, credential)?;
    if uid.as_slice() != credential.uid()
        || Role::decode(role).map_err(|_| PersonalError::BadArguments)? != credential.role
    {
        return Err(PersonalError::Permission);
    }
    let precondition = precondition
        .as_deref()
        .map(foks_proto::KvPathVersionVector::decode)
        .transpose()
        .map_err(|_| PersonalError::BadArguments)?;
    check_personal_precondition(sql, credential, precondition.as_ref())?;
    if mutations.is_empty() || mutations.len() > 64 {
        return Err(PersonalError::BadArguments);
    }
    let mut decoded = Vec::with_capacity(mutations.len());
    for mutation in mutations {
        if mutation.exact.len() > crate::MAX_BLOB_BYTES {
            return Err(PersonalError::UnsupportedProfile);
        }
        let dirent = foks_proto::KvDirent::decode(&mutation.exact)
            .map_err(|_| PersonalError::BadArguments)?;
        if dirent.version == 0
            || dirent.directory_version == 0
            || dirent.write_role > credential.role
            || dirent.directory_status != foks_proto::KvDirectoryStatus::Active
        {
            return Err(PersonalError::BadArguments);
        }
        if dirent.parent != mutation.parent
            || dirent.id != mutation.id
            || dirent.version != mutation.version
            || dirent.directory_version != mutation.directory_version
            || dirent.value.0 != mutation.node_id
            || dirent.name_mac != mutation.name_mac
            || dirent.creation_time != mutation.creation_time
        {
            return Err(PersonalError::BadArguments);
        }
        if dirent.value.0[0] == 2 {
            return Err(PersonalError::UnsupportedProfile);
        }
        decoded.push(dirent);
    }
    for dirent in decoded {
        let parent = directory(sql, credential, &dirent.parent)?;
        if parent.write_role > credential.role {
            return Err(PersonalError::Permission);
        }
    }
    Ok(())
}
