use serde::Serialize;

pub(super) type Result<T> = std::result::Result<T, String>;
pub(super) const VALUE_LIMIT: usize = 1_048_576;
pub(super) const RECORD_LIMIT: usize = 2_097_152;
pub(super) const ARTIFACT_LIMIT: usize = 8_388_608;
pub(super) const MAX_TABLES: usize = 256;
pub(super) const MAX_RECORDS: usize = 1024;
pub(super) const MAX_FIELDS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StorageType {
    Integer,
    Real,
    Blob,
    Text,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct BytePolicy {
    pub maximum_bytes: Option<usize>,
    pub source: &'static str,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct Column {
    pub ordinal: usize,
    pub name: String,
    pub declared_type: String,
    pub storage_type: StorageType,
    pub nullable: bool,
    pub explicit_not_null: bool,
    pub primary_key_ordinal: usize,
    pub integer_rowid_alias: bool,
    pub bytes: BytePolicy,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum Source {
    Column {
        ordinal: usize,
    },
    Rowid,
    NodeFileSlice {
        ordinal: usize,
        maximum_bytes: usize,
    },
    ScopedHostOrEmpty {
        ordinal: usize,
        maximum_bytes: usize,
    },
    IntegerOne {
        maximum_bytes: usize,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct Operand {
    pub source: Source,
    pub descending: bool,
    pub collation: String,
    pub key: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum Locator {
    Rowid { spelling: String },
    PrimaryKey { operands: Vec<Operand> },
}
#[derive(Debug, Serialize)]
pub(super) struct Record {
    pub id: String,
    pub table: String,
    pub kind: &'static str,
    pub operands: Vec<Operand>,
}
#[derive(Debug, Serialize)]
pub(super) struct Index {
    pub name: String,
    pub origin: String,
    pub unique: bool,
    pub partial: bool,
    pub partial_policy: &'static str,
    pub sql_sha256: Option<String>,
    pub record_id: String,
    pub table_btree_alias: bool,
}
#[derive(Debug, Serialize)]
pub(super) struct Table {
    pub name: String,
    pub sql_sha256: String,
    pub strict: bool,
    pub without_rowid: bool,
    pub columns: Vec<Column>,
    pub record_id: String,
    pub locator: Locator,
    pub indexes: Vec<Index>,
}
#[derive(Debug, Serialize)]
pub(super) struct Component {
    pub format_version: u32,
    pub component: String,
    pub scope: &'static str,
    pub value_limit: usize,
    pub record_limit: usize,
    pub record_formula: &'static str,
    pub tables: Vec<Table>,
    pub records: Vec<Record>,
}
