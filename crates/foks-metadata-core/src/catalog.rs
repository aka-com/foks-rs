//! One versioned catalog for both SQL hosts. Integers cross the JS boundary as
//! canonical decimal text; SQLite, not JavaScript, performs the i64 conversion.
use crate::{Error, Result, MAX_BLOB_BYTES};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Value {
    Null,
    Integer(#[serde(with = "decimal")] i64),
    Text(String),
    Blob(Vec<u8>),
}

pub(crate) mod decimal {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(n: &i64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&n.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
        let text = String::deserialize(d)?;
        let n: i64 = text.parse().map_err(serde::de::Error::custom)?;
        if text != n.to_string() {
            return Err(serde::de::Error::custom("noncanonical integer"));
        }
        Ok(n)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Integer,
    Blob,
    Text,
    NullableBlob,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Statement {
    NamespaceParty = 1,
    NamespaceEnsure,
    Head,
    RootVersion,
    DirectoryVersion,
    EntryVersion,
    Usage,
    DirectoryExists,
    DirectoryNode,
    FileNode,
    SmallNode,
    NameConflict,
    InsertDirent,
    SetHead,
    ProbeTypes,
    AuthorityState,
    AuthorityInitialize,
    AuthorityClock,
    AuthorityPromote,
    AuthorityActivate,
    AuthorityRevision,
    AuthorityAllocate,
    AuthorityPrincipal,
    AuthoritySetPrincipal,
    AuthorityIntent,
    AuthorityRegister,
    AuthorityOutcome,
    AuthorityPrune,
    AuthoritySavepoint,
    AuthorityRollback,
    AuthorityRelease,
    AuthoritySettle,
    PersonalIssuedCredential,
    PersonalCurrentCredential,
    PersonalAnchor,
    PersonalSsoPolicy,
    PersonalRoleGeneration,
    PersonalRoot,
    PersonalDirectory,
    PersonalSmallNode,
    HistoricalName,
    PublicUsage,
    ReadDependency,
    ReadSchemaCookie,
    ListIssuedCredential,
    ListCurrentOwner,
}

#[derive(Serialize)]
pub struct Spec {
    pub id: u32,
    pub sql: &'static str,
    pub parameters: &'static [Kind],
    pub columns: &'static [Kind],
    pub write: bool,
    pub maximum_blob_bytes: usize,
}

impl Statement {
    pub const ALL: [Self; 46] = [
        Self::NamespaceParty,
        Self::NamespaceEnsure,
        Self::Head,
        Self::RootVersion,
        Self::DirectoryVersion,
        Self::EntryVersion,
        Self::Usage,
        Self::DirectoryExists,
        Self::DirectoryNode,
        Self::FileNode,
        Self::SmallNode,
        Self::NameConflict,
        Self::InsertDirent,
        Self::SetHead,
        Self::ProbeTypes,
        Self::AuthorityState,
        Self::AuthorityInitialize,
        Self::AuthorityClock,
        Self::AuthorityPromote,
        Self::AuthorityActivate,
        Self::AuthorityRevision,
        Self::AuthorityAllocate,
        Self::AuthorityPrincipal,
        Self::AuthoritySetPrincipal,
        Self::AuthorityIntent,
        Self::AuthorityRegister,
        Self::AuthorityOutcome,
        Self::AuthorityPrune,
        Self::AuthoritySavepoint,
        Self::AuthorityRollback,
        Self::AuthorityRelease,
        Self::AuthoritySettle,
        Self::PersonalIssuedCredential,
        Self::PersonalCurrentCredential,
        Self::PersonalAnchor,
        Self::PersonalSsoPolicy,
        Self::PersonalRoleGeneration,
        Self::PersonalRoot,
        Self::PersonalDirectory,
        Self::PersonalSmallNode,
        Self::HistoricalName,
        Self::PublicUsage,
        Self::ReadDependency,
        Self::ReadSchemaCookie,
        Self::ListIssuedCredential,
        Self::ListCurrentOwner,
    ];

    pub fn from_id(id: u32) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|s| *s as u32 == id)
            .ok_or(Error::Bounds)
    }

    pub fn spec(self) -> Spec {
        if self == Self::ReadDependency {
            return Spec {
                id: self as u32,
                sql: READ_DEPENDENCY_SQL,
                parameters: &[],
                columns: &[
                    Kind::Text,
                    Kind::Integer,
                    Kind::NullableBlob,
                    Kind::Text,
                    Kind::Integer,
                ],
                write: false,
                maximum_blob_bytes: 32,
            };
        }
        if self == Self::ReadSchemaCookie {
            return Spec {
                id: self as u32,
                sql: READ_SCHEMA_COOKIE_SQL,
                parameters: &[],
                columns: &[Kind::Integer],
                write: false,
                maximum_blob_bytes: 0,
            };
        }
        if matches!(
            self,
            Self::AuthorityState
                | Self::AuthorityInitialize
                | Self::AuthorityClock
                | Self::AuthorityPromote
                | Self::AuthorityActivate
                | Self::AuthorityRevision
                | Self::AuthorityAllocate
                | Self::AuthorityPrincipal
                | Self::AuthoritySetPrincipal
                | Self::AuthorityIntent
                | Self::AuthorityRegister
                | Self::AuthorityOutcome
                | Self::AuthorityPrune
                | Self::AuthoritySavepoint
                | Self::AuthorityRollback
                | Self::AuthorityRelease
                | Self::AuthoritySettle
        ) {
            return crate::authority::catalog::spec(self);
        }
        if matches!(
            self,
            Self::PersonalIssuedCredential
                | Self::PersonalCurrentCredential
                | Self::PersonalAnchor
                | Self::PersonalSsoPolicy
                | Self::PersonalRoleGeneration
                | Self::PersonalRoot
                | Self::PersonalDirectory
                | Self::PersonalSmallNode
                | Self::PublicUsage
                | Self::ListIssuedCredential
                | Self::ListCurrentOwner
        ) {
            return crate::personal::catalog::spec(self);
        }
        if self == Self::HistoricalName {
            return Spec {
                id: self as u32,
                sql: HISTORICAL_NAME_SQL.as_str(),
                parameters: &[Kind::Blob, Kind::Blob, Kind::Blob, Kind::Integer],
                columns: &[
                    Kind::Integer,
                    Kind::NullableBlob,
                    Kind::Blob,
                    Kind::Integer,
                    Kind::NullableBlob,
                ],
                write: false,
                maximum_blob_bytes: MAX_BLOB_BYTES,
            };
        }
        use Kind::{Blob as B, Integer as I, NullableBlob as N, Text as T};
        let (sql, parameters, columns, write): (_, &'static [_], &'static [_], _) = match self {
            Self::NamespaceParty => ("SELECT CAST(party_kind AS TEXT),host_id FROM (SELECT 1 AS party_kind,COALESCE((SELECT host_id FROM host_metadata WHERE singleton=1),zeroblob(33)) AS host_id FROM users WHERE uid=?1 UNION ALL SELECT team_kind,host_id FROM teams WHERE team_id=?1) LIMIT 1", &[B], &[I,B], false),
            Self::NamespaceEnsure => ("INSERT INTO kv_namespaces(namespace_id,party_kind,host_id) VALUES(?1,CAST(?2 AS INTEGER),?3) ON CONFLICT(namespace_id) DO UPDATE SET party_kind=excluded.party_kind,host_id=excluded.host_id WHERE kv_namespaces.party_kind=excluded.party_kind AND kv_namespaces.host_id=excluded.host_id", &[B,I,B], &[], true),
            // CASE prevents an oversized payload crossing the SQLite/V8 boundary.
            // Length is returned first and checked before NULL is interpreted.
            Self::Head => ("SELECT CAST(h.version AS TEXT),CAST(length(d.exact_dirent) AS TEXT),CASE WHEN length(d.exact_dirent)<=4096 THEN d.exact_dirent ELSE NULL END FROM kv_dirent_heads h JOIN kv_dirents d ON d.uid=h.uid AND d.parent_id=h.parent_id AND d.dirent_id=h.dirent_id AND d.version=h.version WHERE h.uid=?1 AND h.parent_id=?2 AND h.dirent_id=?3", &[B,B,B], &[I,I,N], false),
            Self::RootVersion => ("SELECT CAST(root_version AS TEXT) FROM kv_roots WHERE uid=?1", &[B], &[I], false),
            Self::DirectoryVersion => ("SELECT CAST(version AS TEXT) FROM kv_directory_heads WHERE uid=?1 AND directory_id=?2", &[B,B], &[I], false),
            Self::EntryVersion => ("SELECT CAST(version AS TEXT) FROM kv_dirent_heads WHERE uid=?1 AND parent_id=?2 AND dirent_id=?3", &[B,B,B], &[I], false),
            Self::Usage => ("SELECT CAST(accounted_bytes AS TEXT),CAST(accounted_objects AS TEXT),CAST(directory_count AS TEXT),CAST(dirent_count AS TEXT) FROM kv_namespaces WHERE namespace_id=?1", &[B], &[I,I,I,I], false),
            Self::DirectoryExists => ("SELECT '1' FROM kv_directories WHERE uid=?1 AND directory_id=?2 AND version=CAST(?3 AS INTEGER) AND status=0", &[B,B,I], &[I], false),
            Self::DirectoryNode => ("SELECT '1' FROM kv_directory_heads WHERE uid=?1 AND directory_id=?2", &[B,B], &[I], false),
            Self::FileNode => ("SELECT '1' FROM kv_file_uploads WHERE uid=?1 AND file_id=?2 AND reclaiming=0", &[B,B], &[I], false),
            Self::SmallNode => ("SELECT '1' FROM kv_nodes WHERE uid=?1 AND node_id=?2 AND node_type=CAST(?3 AS INTEGER)", &[B,B,I], &[I], false),
            Self::NameConflict => ("SELECT '1' FROM kv_dirent_current\nWHERE uid = ?1 AND parent_id = ?2 AND dirent_id != ?3 AND name_mac = ?4 LIMIT 1", &[B,B,B,B], &[I], false),
            Self::InsertDirent => ("INSERT INTO kv_dirents(uid,parent_id,dirent_id,version,directory_version,node_id,name_mac,creation_time,exact_dirent) VALUES(?1,?2,?3,CAST(?4 AS INTEGER),CAST(?5 AS INTEGER),?6,?7,CAST(?8 AS INTEGER),?9)", &[B,B,B,I,I,B,B,I,B], &[], true),
            Self::SetHead => ("INSERT INTO kv_dirent_heads(uid,parent_id,dirent_id,version) VALUES(?1,?2,?3,CAST(?4 AS INTEGER)) ON CONFLICT(uid,parent_id,dirent_id) DO UPDATE SET version=excluded.version", &[B,B,B,I], &[], true),
            Self::ProbeTypes => ("SELECT CAST(CAST(?1 AS INTEGER) AS TEXT),?2,?3,?4", &[I,B,N,T], &[I,B,N,T], false),
            _ => unreachable!("other catalog groups explicitly delegated above"),
        };
        Spec {
            id: self as u32,
            sql,
            parameters,
            columns,
            write,
            maximum_blob_bytes: MAX_BLOB_BYTES,
        }
    }

    pub fn validate_parameters(self, values: &[Value]) -> Result<()> {
        let spec = self.spec();
        if values.len() != spec.parameters.len() {
            return Err(Error::Bounds);
        }
        for (value, kind) in values.iter().zip(spec.parameters) {
            let valid = match (value, kind) {
                (Value::Integer(_), Kind::Integer) => true,
                (Value::Blob(b), Kind::Blob | Kind::NullableBlob) => b.len() <= MAX_BLOB_BYTES,
                (Value::Null, Kind::NullableBlob) => true,
                (Value::Text(s), Kind::Text) => s.len() <= MAX_BLOB_BYTES,
                _ => false,
            };
            if !valid {
                return Err(Error::Bounds);
            }
        }
        Ok(())
    }
}

/// Guarded point projection only. Callers must reject missing/suppressed data,
/// invalid types, zero incarnation and nonpositive revision before trusting it.
pub const READ_DEPENDENCY_SQL: &str = "SELECT typeof(incarnation),CASE WHEN typeof(incarnation)='blob' THEN CAST(length(incarnation) AS TEXT) ELSE '-1' END,CASE WHEN typeof(incarnation)='blob' AND length(incarnation)=32 THEN incarnation ELSE NULL END,typeof(revision),CASE WHEN typeof(revision)='integer' THEN CAST(revision AS TEXT) ELSE '0' END FROM metadata_read_dependency WHERE singleton=1";

/// Qualified on pinned celld with transactional DDL/rollback. A cookie alone is
/// not a connection-bound schema proof; only an admitted guard can use it.
pub const READ_SCHEMA_COOKIE_SQL: &str =
    "SELECT CAST(schema_version AS TEXT) FROM pragma_schema_version";

/// Kept byte-identical to server-db's selection until the shared query layer lands.
pub const HISTORICAL_NAME_SELECTION_SQL: &str = "SELECT e.dirent_id,e.version FROM kv_dirents e INDEXED BY kv_dirent_historical_name JOIN kv_directories d ON d.uid=e.uid AND d.directory_id=e.parent_id AND d.version=e.directory_version WHERE e.uid=?1 AND e.parent_id=?2 AND e.name_mac=?3 AND e.directory_version=CAST(?4 AS INTEGER) ORDER BY e.version DESC,e.dirent_id ASC LIMIT 1";
static HISTORICAL_NAME_SQL: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!("WITH selected AS ({}) SELECT CAST(length(e.exact_dirent) AS TEXT),CASE WHEN length(e.exact_dirent)<=4096 THEN e.exact_dirent ELSE NULL END,e.node_id,CAST(length(d.exact_directory) AS TEXT),CASE WHEN length(d.exact_directory)<=4096 THEN d.exact_directory ELSE NULL END FROM selected s JOIN kv_dirents e ON e.uid=?1 AND e.parent_id=?2 AND e.dirent_id=s.dirent_id AND e.version=s.version JOIN kv_directories d ON d.uid=e.uid AND d.directory_id=e.parent_id AND d.version=e.directory_version",HISTORICAL_NAME_SELECTION_SQL)
});
