//! Qualification-only bounded row catalog. No statement authorizes a caller.
//! Integer SQL results remain canonical decimal TEXT until the host converts
//! them to the exact binary i64 page representation.
use crate::page::{Column, IntegerRange, PageKind, PageSpec};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PageStatement {
    ProbeTypes = 1,
    ListInitial,
    ListMacStart,
    ListMacContinue,
    ListTimeStart,
    ListTimeContinue,
    DirectoryProbe,
    DirectorySlice,
    SchemaObjects,
    RoleGenerations,
}

const INTEGER: Column = Column {
    kind: PageKind::Integer,
    maximum_bytes: 8,
    nullable: false,
};
const fn blob(maximum_bytes: u32, nullable: bool) -> Column {
    Column {
        kind: PageKind::Blob,
        maximum_bytes,
        nullable,
    }
}
const fn text(maximum_bytes: u32) -> Column {
    Column {
        kind: PageKind::Text,
        maximum_bytes,
        nullable: false,
    }
}
const fn nullable_text(maximum_bytes: u32) -> Column {
    Column {
        kind: PageKind::Text,
        maximum_bytes,
        nullable: true,
    }
}
const ROLE_PARAMETERS: &[Column] = &[
    blob(33, false),
    INTEGER,
    INTEGER,
    INTEGER,
    INTEGER,
    INTEGER,
    INTEGER,
    INTEGER,
    INTEGER,
    INTEGER,
    INTEGER,
    INTEGER,
    INTEGER,
    INTEGER,
    INTEGER,
    INTEGER,
];
const ROLE_COLUMNS: &[Column] = &[INTEGER, INTEGER, text(7), INTEGER];
/// Seven point lookups, independent of the number of stored roles/generations.
/// Selecting rowid preserves the distinction between absent and malformed NULL.
pub const ROLE_GENERATIONS_SQL: &str = "WITH requested(ordinal,role,visibility) AS (VALUES (0,CAST(?3 AS INTEGER),CAST(?4 AS INTEGER)),(1,CAST(?5 AS INTEGER),CAST(?6 AS INTEGER)),(2,CAST(?7 AS INTEGER),CAST(?8 AS INTEGER)),(3,CAST(?9 AS INTEGER),CAST(?10 AS INTEGER)),(4,CAST(?11 AS INTEGER),CAST(?12 AS INTEGER)),(5,CAST(?13 AS INTEGER),CAST(?14 AS INTEGER)),(6,CAST(?15 AS INTEGER),CAST(?16 AS INTEGER))) SELECT CAST(r.ordinal AS TEXT),CASE WHEN k.rowid IS NULL THEN '0' ELSE '1' END,typeof(k.generation),CASE WHEN typeof(k.generation)='integer' THEN CAST(k.generation AS TEXT) ELSE '0' END FROM requested r LEFT JOIN shared_keys k ON k.rowid=(SELECT rowid FROM shared_keys WHERE uid=?1 AND role_type=r.role AND visibility=r.visibility ORDER BY generation DESC LIMIT 1) WHERE r.ordinal<CAST(?2 AS INTEGER) ORDER BY r.ordinal";

/// Structurally typed parameters, not proof that a caller owns this namespace.
pub fn role_generation_parameters(
    uid: &[u8],
    roles: &[foks_proto::Role],
) -> crate::Result<[crate::Value; 16]> {
    use crate::{Error, Value};
    if uid.len() != 33
        || uid[0] != foks_proto::ENTITY_USER
        || roles.is_empty()
        || roles.len() > 7
        || roles.contains(&foks_proto::Role::NONE)
    {
        return Err(Error::Invalid("role generation arguments"));
    }
    crate::page::charge(uid.len())?;
    let mut parameters = std::array::from_fn(|_| Value::Integer(0));
    parameters[0] = Value::Blob(uid.to_vec());
    parameters[1] = Value::Integer(roles.len() as i64);
    for (index, role) in roles.iter().enumerate() {
        parameters[2 + index * 2] = Value::Integer(role.protocol_value() as i64);
        parameters[3 + index * 2] = Value::Integer(i64::from(role.visibility().unwrap_or(0)));
    }
    PageStatement::RoleGenerations
        .spec()
        .validate_parameters(&parameters)?;
    Ok(parameters)
}

const SCHEMA_PARAMETERS: &[Column] = &[INTEGER, text(8), text(256), INTEGER];
const SCHEMA_COLUMNS: &[Column] = &[
    text(7),
    INTEGER,
    nullable_text(8),
    text(7),
    INTEGER,
    nullable_text(256),
    text(7),
    INTEGER,
    nullable_text(256),
    text(7),
    INTEGER,
    nullable_text(65536),
];
macro_rules! schema_field {
    ($field:literal, $cap:literal) => {
        concat!(
            "typeof(",
            $field,
            "),CASE WHEN typeof(",
            $field,
            ")='text' THEN CAST(length(CAST(",
            $field,
            " AS BLOB)) AS TEXT) ELSE '-1' END,CASE WHEN typeof(",
            $field,
            ")='text' AND length(CAST(",
            $field,
            " AS BLOB))<=",
            $cap,
            " THEN ",
            $field,
            " ELSE NULL END"
        )
    };
}
/// Every non-internal object is visible, including unexpected NULL SQL. Guarded
/// fields preserve type/length errors without copying oversized SQL or names.
pub const SCHEMA_OBJECTS_SQL: &str = concat!(
    "SELECT ", schema_field!("type", "8"), ",", schema_field!("name", "256"),
    ",", schema_field!("tbl_name", "256"), ",", schema_field!("sql", "65536"),
    " FROM sqlite_schema WHERE (typeof(name)!='text' OR substr(name,1,7) != 'sqlite_' COLLATE NOCASE) AND (CAST(?1 AS INTEGER)=1 OR (type COLLATE BINARY,name COLLATE BINARY)>(?2 COLLATE BINARY,?3 COLLATE BINARY)) ORDER BY type COLLATE BINARY,name COLLATE BINARY LIMIT CAST(?4 AS INTEGER)"
);

/// Activation-owned cursor construction. Generic PageSpec admission checks only
/// types/bounds; it does not prove this initial/continuation relationship or that
/// a cursor came from the last successfully validated schema page.
pub fn schema_objects_parameters(
    after: Option<(&str, &str)>,
    limit: u16,
) -> crate::Result<[crate::Value; 4]> {
    use crate::{Error, Value};
    let (initial, kind, name) = match after {
        None => (1, "", ""),
        Some((kind, name)) => {
            if !matches!(kind, "table" | "index" | "trigger" | "view")
                || name.is_empty()
                || name.len() > 256
            {
                return Err(Error::Bounds);
            }
            (0, kind, name)
        }
    };
    crate::page::charge(kind.len() + name.len())?;
    let parameters = [
        Value::Integer(initial),
        Value::Text(kind.into()),
        Value::Text(name.into()),
        Value::Integer(i64::from(limit)),
    ];
    PageStatement::SchemaObjects
        .spec()
        .validate_parameters(&parameters)?;
    Ok(parameters)
}

// Probes precede payloads, distinguishing a missing optional row from a
// wrong-type/oversized value suppressed by CASE. NULL is never a decoding repair.
const LIST_COLUMNS: &[Column] = &[
    blob(16, false),
    blob(32, false),
    INTEGER,
    blob(17, false),
    text(7),
    INTEGER,
    blob(4096, true),
    INTEGER,
    text(7),
    INTEGER,
    blob(65536, true),
];

// Const storage gives every descriptor its required static lifetime.
const PROBE_PARAMETERS: &[Column] = &[
    INTEGER,
    INTEGER,
    blob(64, false),
    text(64),
    blob(64, true),
    INTEGER,
];
const PROBE_COLUMNS: &[Column] = &[INTEGER, INTEGER, blob(64, false), text(64), blob(64, true)];
const INITIAL_PARAMETERS: &[Column] = &[blob(33, false), blob(16, false), INTEGER, INTEGER];
const MAC_START_PARAMETERS: &[Column] = &[
    blob(33, false),
    blob(16, false),
    blob(32, false),
    INTEGER,
    INTEGER,
];
const MAC_CONTINUE_PARAMETERS: &[Column] = &[
    blob(33, false),
    blob(16, false),
    blob(32, false),
    blob(16, false),
    INTEGER,
    INTEGER,
];
const TIME_START_PARAMETERS: &[Column] =
    &[blob(33, false), blob(16, false), INTEGER, INTEGER, INTEGER];
const TIME_CONTINUE_PARAMETERS: &[Column] = &[
    blob(33, false),
    blob(16, false),
    INTEGER,
    blob(16, false),
    INTEGER,
    INTEGER,
];
const DIRECTORY_PROBE_PARAMETERS: &[Column] = &[blob(33, false), blob(16, false), INTEGER];
const DIRECTORY_PROBE_COLUMNS: &[Column] = &[INTEGER, text(7), INTEGER];
const DIRECTORY_SLICE_PARAMETERS: &[Column] = &[
    blob(33, false),
    blob(16, false),
    INTEGER,
    INTEGER,
    INTEGER,
    INTEGER,
];
const DIRECTORY_SLICE_COLUMNS: &[Column] = &[INTEGER, text(7), INTEGER, blob(512 * 1024, true)];

macro_rules! list_projection {
    () => {
        "SELECT c.dirent_id,c.name_mac,CAST(c.creation_time AS TEXT),c.node_id,typeof(d.exact_dirent),CAST(COALESCE(length(CAST(d.exact_dirent AS BLOB)),-1) AS TEXT),CASE WHEN typeof(d.exact_dirent)='blob' AND length(d.exact_dirent)<=4096 THEN d.exact_dirent ELSE NULL END,CAST(n.node_id IS NOT NULL AS TEXT),typeof(n.exact_node),CAST(COALESCE(length(CAST(n.exact_node AS BLOB)),-1) AS TEXT),CASE WHEN typeof(n.node_type)='integer' AND n.node_type>=0 AND typeof(n.exact_node)='blob' AND length(n.exact_node)<=65536 THEN n.exact_node ELSE NULL END "
    };
}
macro_rules! current_join {
    () => {
        "JOIN kv_dirents d ON d.uid=c.uid AND d.parent_id=c.parent_id AND d.dirent_id=c.dirent_id AND d.version=c.version "
    };
}
macro_rules! node_join {
    ($include:literal) => {
        concat!("LEFT JOIN kv_nodes n ON CAST(",$include," AS INTEGER)=1 AND substr(c.node_id,1,1)=X'03' AND n.uid=c.uid AND n.node_id=c.node_id ")
    };
}

const INITIAL: &str =
    concat!(list_projection!(),
    "FROM kv_dirent_current c INDEXED BY kv_dirent_current_name ",current_join!(),node_join!("?3"),
    "WHERE c.uid=?1 AND c.parent_id=?2 ORDER BY c.name_mac,c.dirent_id LIMIT CAST(?4 AS INTEGER)");
const MAC_START: &str = concat!(list_projection!(),
    "FROM kv_dirent_current c INDEXED BY kv_dirent_current_name ",current_join!(),node_join!("?4"),
    "WHERE c.uid=?1 AND c.parent_id=?2 AND c.name_mac>?3 ORDER BY c.name_mac,c.dirent_id LIMIT CAST(?5 AS INTEGER)");
const MAC_CONTINUE: &str = concat!(list_projection!(),
    "FROM kv_dirent_current c INDEXED BY kv_dirent_current_name ",current_join!(),node_join!("?5"),
    "WHERE c.uid=?1 AND c.parent_id=?2 AND (c.name_mac,c.dirent_id)>(?3,?4) ORDER BY c.name_mac,c.dirent_id LIMIT CAST(?6 AS INTEGER)");
const TIME_START: &str = concat!(list_projection!(),
    "FROM kv_dirent_current c INDEXED BY kv_dirent_current_time ",current_join!(),node_join!("?4"),
    "WHERE c.uid=?1 AND c.parent_id=?2 AND c.creation_time>=CAST(?3 AS INTEGER) ORDER BY c.creation_time,c.dirent_id LIMIT CAST(?5 AS INTEGER)");
const TIME_CONTINUE: &str = concat!(list_projection!(),
    "FROM kv_dirent_current c INDEXED BY kv_dirent_current_time ",current_join!(),node_join!("?5"),
    "WHERE c.uid=?1 AND c.parent_id=?2 AND (c.creation_time,c.dirent_id)>(CAST(?3 AS INTEGER),?4) ORDER BY c.creation_time,c.dirent_id LIMIT CAST(?6 AS INTEGER)");

impl PageStatement {
    pub const ALL: [Self; 10] = [
        Self::ProbeTypes,
        Self::ListInitial,
        Self::ListMacStart,
        Self::ListMacContinue,
        Self::ListTimeStart,
        Self::ListTimeContinue,
        Self::DirectoryProbe,
        Self::DirectorySlice,
        Self::SchemaObjects,
        Self::RoleGenerations,
    ];
    pub fn from_id(id: u32) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|statement| *statement as u32 == id)
    }
    pub fn spec(self) -> PageSpec {
        let (sql,parameters,columns,integer_ranges,limit_parameter):(_, &'static [_], &'static [_], &'static [_], _) = match self {
            Self::RoleGenerations => (
                ROLE_GENERATIONS_SQL, ROLE_PARAMETERS, ROLE_COLUMNS,
                &[IntegerRange{parameter:1,minimum:1,maximum:7}], None),
            Self::SchemaObjects => (
                SCHEMA_OBJECTS_SQL, SCHEMA_PARAMETERS, SCHEMA_COLUMNS,
                &[IntegerRange{parameter:0,minimum:0,maximum:1}], Some(3)),
            Self::DirectoryProbe => (
                "SELECT CAST(d.version AS TEXT),typeof(d.exact_directory),CAST(length(CAST(d.exact_directory AS BLOB)) AS TEXT) FROM kv_directory_heads h JOIN kv_directories d ON d.uid=h.uid AND d.directory_id=h.directory_id AND d.version=h.version WHERE h.uid=?1 AND h.directory_id=?2 AND CAST(?3 AS INTEGER)>0",
                DIRECTORY_PROBE_PARAMETERS,DIRECTORY_PROBE_COLUMNS,
                &[IntegerRange{parameter:2,minimum:1,maximum:16*1024*1024}],None),
            Self::DirectorySlice => (
                "SELECT CAST(d.version AS TEXT),typeof(d.exact_directory),CAST(length(CAST(d.exact_directory AS BLOB)) AS TEXT),CASE WHEN typeof(d.exact_directory)='blob' AND length(d.exact_directory)<=CAST(?6 AS INTEGER) THEN substr(d.exact_directory,CAST(?4 AS INTEGER)+1,CAST(?5 AS INTEGER)) ELSE NULL END FROM kv_directory_heads h JOIN kv_directories d ON d.uid=h.uid AND d.directory_id=h.directory_id AND d.version=h.version WHERE h.uid=?1 AND h.directory_id=?2 AND h.version=CAST(?3 AS INTEGER)",
                DIRECTORY_SLICE_PARAMETERS,DIRECTORY_SLICE_COLUMNS,
                &[IntegerRange{parameter:2,minimum:1,maximum:i64::MAX},IntegerRange{parameter:3,minimum:0,maximum:16*1024*1024-1},IntegerRange{parameter:4,minimum:1,maximum:512*1024},IntegerRange{parameter:5,minimum:1,maximum:16*1024*1024}],None),
            Self::ProbeTypes => (
                "WITH RECURSIVE seq(n) AS (SELECT 0 WHERE CAST(?1 AS INTEGER)>0 UNION ALL SELECT n+1 FROM seq WHERE n+1<CAST(?1 AS INTEGER)) SELECT CAST(n AS TEXT),CAST(CAST(?2 AS INTEGER) AS TEXT),?3,?4,?5 FROM seq LIMIT CAST(?6 AS INTEGER)",
                PROBE_PARAMETERS,
                PROBE_COLUMNS,
                &[IntegerRange{parameter:0,minimum:0,maximum:64}], Some(5)),
            Self::ListInitial => (INITIAL,INITIAL_PARAMETERS,LIST_COLUMNS,&[IntegerRange{parameter:2,minimum:0,maximum:1}],Some(3)),
            Self::ListMacStart => (MAC_START,MAC_START_PARAMETERS,LIST_COLUMNS,&[IntegerRange{parameter:3,minimum:0,maximum:1}],Some(4)),
            Self::ListMacContinue => (MAC_CONTINUE,MAC_CONTINUE_PARAMETERS,LIST_COLUMNS,&[IntegerRange{parameter:4,minimum:0,maximum:1}],Some(5)),
            Self::ListTimeStart => (TIME_START,TIME_START_PARAMETERS,LIST_COLUMNS,&[IntegerRange{parameter:2,minimum:0,maximum:i64::MAX},IntegerRange{parameter:3,minimum:0,maximum:1}],Some(4)),
            Self::ListTimeContinue => (TIME_CONTINUE,TIME_CONTINUE_PARAMETERS,LIST_COLUMNS,&[IntegerRange{parameter:2,minimum:0,maximum:i64::MAX},IntegerRange{parameter:4,minimum:0,maximum:1}],Some(5)),
        };
        PageSpec {
            id: self as u32,
            sql,
            parameters,
            columns,
            maximum_rows: match self {
                Self::DirectoryProbe | Self::DirectorySlice => 1,
                Self::RoleGenerations => 7,
                _ => 64,
            },
            maximum_bytes: if self == Self::RoleGenerations {
                512
            } else {
                1_048_576
            },
            integer_ranges,
            limit_parameter,
        }
    }
}
