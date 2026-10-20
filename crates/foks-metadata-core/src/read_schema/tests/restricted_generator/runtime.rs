//! R1a development renderer. It does not depend on the future runtime module.
use super::*;
use std::collections::BTreeSet;

const PINS: [(&str, &str); 4] = [
    (
        "authority-records-v1.json",
        "0de8a187678b889de5d7b9c4552aed8e44d7fedf1be5f849180d9c5e78bbf518",
    ),
    (
        "engine-ownership-v1.json",
        "b96142c9c163ccf9f0dda87e3adda2fdf64d2c2b4ace94614c0074d5e5ed378e",
    ),
    (
        "foks-records-v1.json",
        "7addbf7e7b75d33908036c9b7280b246c4a5c2b6a65508ba277199be7c139df2",
    ),
    (
        "storage-descriptors-manifest.json",
        "6c3628d4acd708b519150e7f4a6f6255adb185c97973ce42da60724fb35203e2",
    ),
];
const FILE_NAME: &str = "generated.rs";

// The generated format owns its private static DTOs. R1b will expose opaque IDs
// and borrowed getters rather than making these constructors public.
const HEADER: &str = r#"// Generated from reviewed R0 trusted sources. Do not edit.
// Storage dimensions only: no source admission or authorization capability.
#[rustfmt::skip]
pub(crate) mod data {
#[derive(Clone, Copy, Debug)]
pub(crate) enum StorageType { Integer, Real, Blob, Text }
#[derive(Clone, Copy, Debug)]
pub(crate) enum Source {
    Column { ordinal: u16 }, Rowid,
    NodeFileSlice { ordinal: u16, maximum_bytes: u64 },
    ScopedHostOrEmpty { ordinal: u16, maximum_bytes: u64 },
    IntegerOne { maximum_bytes: u64 },
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Operand { pub source: Source, pub descending: bool, pub collation: &'static str, pub key: bool }
#[derive(Clone, Copy, Debug)]
pub(crate) enum Locator { Rowid { spelling: &'static str }, PrimaryKey { operands: &'static [Operand] } }
#[derive(Clone, Copy, Debug)]
pub(crate) struct Column {
    pub ordinal: u16, pub name: &'static str, pub declared_type: &'static str,
    pub storage_type: StorageType, pub nullable: bool, pub explicit_not_null: bool,
    pub primary_key_ordinal: u16, pub integer_rowid_alias: bool,
    pub maximum_bytes: Option<u64>, pub policy_source: &'static str,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Index {
    pub name: &'static str, pub origin: &'static str, pub unique: bool,
    pub partial: bool, pub partial_policy: &'static str,
    pub sql_sha256: Option<&'static str>, pub record_id: &'static str, pub table_btree_alias: bool,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Table {
    pub name: &'static str, pub sql_sha256: &'static str, pub strict: bool,
    pub without_rowid: bool, pub columns: &'static [Column], pub record_id: &'static str,
    pub locator: Locator, pub indexes: &'static [Index],
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Record { pub id: &'static str, pub table: &'static str, pub kind: &'static str, pub operands: &'static [Operand] }
#[derive(Clone, Copy, Debug)]
pub(crate) struct Component {
    pub format_version: u32, pub component: &'static str, pub scope: &'static str,
    pub value_limit: u64, pub record_limit: u64, pub record_formula: &'static str,
    pub tables: &'static [Table], pub records: &'static [Record],
}
// Complete immutable inventory/provenance tree, already compiled as borrowed
// values. No JSON parser is present or required in a runtime consumer.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Inventory {
    Null, Bool(bool), Unsigned(u64), Text(&'static str),
    Array(&'static [Inventory]), Object(&'static [(&'static str, Inventory)]),
}
pub(crate) const SOURCE_FORMAT: u32 = 1;
pub(crate) const PROFILE: &str = "hosted-metadata-small-v1";
pub(crate) const REQUIRED_ENCODING: &str = "UTF-8";
"#;

struct Writer(BoundedJson);
impl std::fmt::Write for Writer {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        self.0
            .write_all(text.as_bytes())
            .map_err(|_| std::fmt::Error)
    }
}
impl Writer {
    fn new() -> Self {
        Self(BoundedJson(Vec::new()))
    }
    fn text(&mut self, text: &str) -> Result<()> {
        std::fmt::Write::write_str(self, text).map_err(|_| "Rust source allocation or bound".into())
    }
}
macro_rules! emit {
    ($out:expr, $($arg:tt)*) => {
        std::fmt::Write::write_fmt($out, format_args!($($arg)*)).map_err(|_| "Rust source allocation or bound".to_owned())?
    }
}
fn identifier(text: &str) -> Result<()> {
    if text.is_empty() || text.len() > 256 {
        return Err("descriptor identifier bound".into());
    }
    Ok(())
}
fn operands(values: &[Operand], columns: usize) -> Result<()> {
    if values.is_empty() || values.len() > 32 {
        return Err("record/locator operand bound".into());
    }
    for value in values {
        if value.collation != "BINARY" {
            return Err("unknown collation".into());
        }
        let ordinal = match value.source {
            Source::Column { ordinal } => Some(ordinal),
            Source::NodeFileSlice {
                ordinal,
                maximum_bytes: 16,
            }
            | Source::ScopedHostOrEmpty {
                ordinal,
                maximum_bytes: 33,
            } => Some(ordinal),
            Source::Rowid | Source::IntegerOne { maximum_bytes: 8 } => None,
            _ => return Err("unknown expression bound".into()),
        };
        if ordinal.is_some_and(|o| o >= columns) {
            return Err("operand column reference".into());
        }
    }
    Ok(())
}
fn validate(component: &Component) -> Result<()> {
    if component.tables.is_empty()
        || component.tables.len() > MAX_TABLES
        || component.records.len() > MAX_RECORDS
        || component.format_version != 1
        || component.value_limit != VALUE_LIMIT
        || component.record_limit != RECORD_LIMIT
    {
        return Err("component format or bounds".into());
    }
    let mut names = BTreeSet::new();
    let mut records = BTreeMap::new();
    for record in &component.records {
        if records.insert(record.id.as_str(), record).is_some() {
            return Err("duplicate record".into());
        }
    }
    let mut used = BTreeSet::new();
    let mut all_indexes = BTreeSet::new();
    for table in &component.tables {
        identifier(&table.name)?;
        if !names.insert(table.name.as_str()) {
            return Err("duplicate table".into());
        }
        if table.columns.is_empty() || table.columns.len() > 32 {
            return Err("column bound".into());
        }
        let mut columns = BTreeSet::new();
        for (ordinal, column) in table.columns.iter().enumerate() {
            identifier(&column.name)?;
            if column.ordinal != ordinal
                || !columns.insert(column.name.as_str())
                || column.primary_key_ordinal > table.columns.len()
            {
                return Err("column ordinal/name".into());
            }
        }
        let record = records
            .get(table.record_id.as_str())
            .ok_or("missing table record")?;
        if record.table != table.name || record.kind != "table" || !used.insert(record.id.as_str())
        {
            return Err("table record ownership".into());
        }
        operands(&record.operands, table.columns.len())?;
        match &table.locator {
            Locator::Rowid { spelling } => {
                if table.without_rowid
                    || !["_rowid_", "rowid", "oid"].contains(&spelling.as_str())
                    || table
                        .columns
                        .iter()
                        .any(|c| c.name.eq_ignore_ascii_case(spelling))
                {
                    return Err("rowid locator".into());
                }
            }
            Locator::PrimaryKey { operands: key } => {
                if !table.without_rowid
                    || key
                        .iter()
                        .any(|v| !matches!(v.source, Source::Column { .. }))
                {
                    return Err("primary key locator".into());
                }
                operands(key, table.columns.len())?;
            }
        }
        for index in &table.indexes {
            identifier(&index.name)?;
            if !all_indexes.insert(index.name.as_str()) {
                return Err("duplicate index".into());
            }
            let record = records
                .get(index.record_id.as_str())
                .ok_or("missing index record")?;
            if record.table != table.name {
                return Err("index record ownership".into());
            }
            if index.table_btree_alias {
                if !table.without_rowid
                    || index.origin != "pk"
                    || index.record_id != table.record_id
                {
                    return Err("invalid table alias".into());
                }
            } else if record.kind != "index" || !used.insert(record.id.as_str()) {
                return Err("duplicate physical record ownership".into());
            }
            operands(&record.operands, table.columns.len())?;
        }
    }
    if used.len() != records.len() {
        return Err("orphan record".into());
    }
    Ok(())
}
fn render_operands(out: &mut Writer, values: &[Operand]) -> Result<()> {
    out.text("&[")?;
    for value in values {
        out.text("Operand { source: ")?;
        match value.source {
            Source::Column { ordinal } => emit!(out, "Source::Column {{ ordinal: {ordinal}u16 }}"),
            Source::Rowid => out.text("Source::Rowid")?,
            Source::NodeFileSlice { ordinal, maximum_bytes } => emit!(out, "Source::NodeFileSlice {{ ordinal: {ordinal}u16, maximum_bytes: {maximum_bytes}u64 }}"),
            Source::ScopedHostOrEmpty { ordinal, maximum_bytes } => emit!(out, "Source::ScopedHostOrEmpty {{ ordinal: {ordinal}u16, maximum_bytes: {maximum_bytes}u64 }}"),
            Source::IntegerOne { maximum_bytes } => emit!(out, "Source::IntegerOne {{ maximum_bytes: {maximum_bytes}u64 }}"),
        }
        emit!(
            out,
            ", descending: {}, collation: {:?}, key: {} }},",
            value.descending,
            value.collation,
            value.key
        );
    }
    out.text("]")
}
fn render_component(out: &mut Writer, component: &Component) -> Result<()> {
    emit!(out, "Component {{ format_version: {}u32, component: {:?}, scope: {:?}, value_limit: {}u64, record_limit: {}u64, record_formula: {:?}, tables: &[\n", component.format_version, component.component, component.scope, component.value_limit, component.record_limit, component.record_formula);
    for table in &component.tables {
        emit!(
            out,
            "Table {{ name: {:?}, sql_sha256: {:?}, strict: {}, without_rowid: {}, columns: &[\n",
            table.name,
            table.sql_sha256,
            table.strict,
            table.without_rowid
        );
        for column in &table.columns {
            let kind = match column.storage_type {
                StorageType::Integer => "Integer",
                StorageType::Real => "Real",
                StorageType::Blob => "Blob",
                StorageType::Text => "Text",
            };
            emit!(out, "Column {{ ordinal: {}u16, name: {:?}, declared_type: {:?}, storage_type: StorageType::{kind}, nullable: {}, explicit_not_null: {}, primary_key_ordinal: {}u16, integer_rowid_alias: {}, maximum_bytes: ", column.ordinal, column.name, column.declared_type, column.nullable, column.explicit_not_null, column.primary_key_ordinal, column.integer_rowid_alias);
            match column.bytes.maximum_bytes {
                Some(n) => emit!(out, "Some({n}u64)"),
                None => out.text("None")?,
            }
            emit!(out, ", policy_source: {:?} }},\n", column.bytes.source);
        }
        emit!(out, "], record_id: {:?}, locator: ", table.record_id);
        match &table.locator {
            Locator::Rowid { spelling } => {
                emit!(out, "Locator::Rowid {{ spelling: {spelling:?} }}")
            }
            Locator::PrimaryKey { operands } => {
                out.text("Locator::PrimaryKey { operands: ")?;
                render_operands(out, operands)?;
                out.text(" }")?;
            }
        }
        out.text(", indexes: &[\n")?;
        for index in &table.indexes {
            emit!(out, "Index {{ name: {:?}, origin: {:?}, unique: {}, partial: {}, partial_policy: {:?}, sql_sha256: {:?}, record_id: {:?}, table_btree_alias: {} }},\n", index.name, index.origin, index.unique, index.partial, index.partial_policy, index.sql_sha256, index.record_id, index.table_btree_alias);
        }
        out.text("] },\n")?;
    }
    out.text("], records: &[\n")?;
    for record in &component.records {
        emit!(
            out,
            "Record {{ id: {:?}, table: {:?}, kind: {:?}, operands: ",
            record.id,
            record.table,
            record.kind
        );
        render_operands(out, &record.operands)?;
        out.text(" },\n")?;
    }
    out.text("] }")
}
fn inventory(out: &mut Writer, value: &serde_json::Value, depth: usize) -> Result<()> {
    if depth > 16 {
        return Err("inventory depth".into());
    }
    match value {
        serde_json::Value::Null => out.text("Inventory::Null")?,
        serde_json::Value::Bool(value) => emit!(out, "Inventory::Bool({value})"),
        serde_json::Value::Number(value) => emit!(
            out,
            "Inventory::Unsigned({}u64)",
            value.as_u64().ok_or("inventory unsigned number")?
        ),
        serde_json::Value::String(value) => emit!(out, "Inventory::Text({value:?})"),
        serde_json::Value::Array(values) => {
            out.text("Inventory::Array(&[")?;
            for value in values {
                inventory(out, value, depth + 1)?;
                out.text(",")?;
            }
            out.text("])")?;
        }
        serde_json::Value::Object(values) => {
            out.text("Inventory::Object(&[")?;
            // Explicit sort, independent of serde_json's map feature selection.
            for (key, value) in values.iter().collect::<BTreeMap<_, _>>() {
                emit!(out, "({key:?},");
                inventory(out, value, depth + 1)?;
                out.text("),")?;
            }
            out.text("])")?;
        }
    }
    Ok(())
}
fn render(bundle: &ReferenceBundle) -> Result<Vec<u8>> {
    if bundle.files.len() != PINS.len() {
        return Err("pinned source set".into());
    }
    for (name, pin) in PINS {
        let bytes = bundle.files.get(name).ok_or("pinned source missing")?;
        if bytes.len() > ARTIFACT_LIMIT || hash(bytes) != pin {
            return Err(format!("pinned source differs: {name}"));
        }
    }
    if bytes(&bundle.foks)? != bundle.files["foks-records-v1.json"]
        || bytes(&bundle.authority)? != bundle.files["authority-records-v1.json"]
        || bytes(&bundle.ownership)? != bundle.files["engine-ownership-v1.json"]
    {
        return Err("typed bundle differs from pinned bytes".into());
    }
    validate(&bundle.foks)?;
    validate(&bundle.authority)?;
    let manifest: serde_json::Value =
        serde_json::from_slice(&bundle.files["storage-descriptors-manifest.json"])
            .map_err(|e| e.to_string())?;
    let mut out = Writer::new();
    out.text(HEADER)?;
    out.text("pub(crate) static SOURCE_PINS: &[(&str, &str)] = &[\n")?;
    for (name, pin) in PINS {
        emit!(&mut out, "({name:?}, {pin:?}),\n");
    }
    out.text("];\npub(crate) static COMPONENTS: &[Component] = &[\n")?;
    for component in [&bundle.foks, &bundle.authority] {
        render_component(&mut out, component)?;
        out.text(",\n")?;
    }
    out.text("];\npub(crate) static OWNERSHIP: Inventory = ")?;
    inventory(&mut out, &bundle.ownership, 0)?;
    out.text(";\npub(crate) static MANIFEST: Inventory = ")?;
    inventory(&mut out, &manifest, 0)?;
    out.text(";\n}\n")?;
    Ok(out.0 .0)
}
fn publish(directory: &Path, source: &[u8]) -> Result<()> {
    if source.is_empty() || source.len() > ARTIFACT_LIMIT {
        return Err("Rust artifact bound".into());
    }
    std::fs::create_dir(directory).map_err(|e| e.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join(FILE_NAME))
        .map_err(|e| e.to_string())?;
    file.write_all(source).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    std::fs::File::open(directory)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[test]
fn pinned_reference_bundle_and_rust_are_reproducible() {
    let first = reference_bundle().unwrap();
    let second = reference_bundle().unwrap();
    assert_eq!(first.files, second.files);
    let source = render(&first).unwrap();
    assert_eq!(source, render(&second).unwrap());
    assert!(source.len() < ARTIFACT_LIMIT);
    let source = std::str::from_utf8(&source).unwrap();
    assert_eq!(source.matches("Table { name:").count(), 102);
    assert_eq!(source.matches("Record { id:").count(), 287);
    assert!(source.contains("sqlite_sequence"));
    assert!(source.contains("sso_single_host_policy"));
    assert!(source.contains("maximum_bytes: Some(65536u64)"));
    assert!(source.contains("REQUIRED_ENCODING: &str = \"UTF-8\""));
    assert!(!source.contains("serde_json"));
    assert!(source.ends_with(";\n}\n"));
    assert!(source.contains("#[rustfmt::skip]\npub(crate) mod data {"));
    assert!(!source.contains("allow(dead_code)"));
}
#[test]
fn mutated_bundle_and_pins_are_not_accepted() {
    let mut bundle = reference_bundle().unwrap();
    bundle.foks.tables[0].columns[0].nullable = !bundle.foks.tables[0].columns[0].nullable;
    assert_eq!(
        render(&bundle).unwrap_err(),
        "typed bundle differs from pinned bytes"
    );
    let mut bundle = reference_bundle().unwrap();
    bundle
        .files
        .get_mut("foks-records-v1.json")
        .unwrap()
        .push(b' ');
    assert!(render(&bundle)
        .unwrap_err()
        .contains("pinned source differs"));
}
fn small() -> Component {
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch("CREATE TABLE one(a TEXT NOT NULL,b BLOB,PRIMARY KEY(a)) WITHOUT ROWID; CREATE INDEX one_b ON one(b);").unwrap();
    introspection::component(&connection, "miniature").unwrap()
}
#[test]
fn graph_rejects_missing_duplicate_orphan_and_oversized_layouts() {
    let mut value = small();
    validate(&value).unwrap();
    value.records.pop();
    assert!(validate(&value).unwrap_err().contains("missing"));
    let mut value = small();
    let table_record = value.tables[0].record_id.clone();
    value.tables[0]
        .indexes
        .iter_mut()
        .find(|i| !i.table_btree_alias)
        .unwrap()
        .record_id = table_record;
    assert!(validate(&value).is_err());
    let mut value = small();
    let record = &value.records[0];
    value.records.push(Record {
        id: record.id.clone(),
        table: record.table.clone(),
        kind: record.kind,
        operands: record.operands.clone(),
    });
    assert_eq!(validate(&value).unwrap_err(), "duplicate record");
    let mut value = small();
    value.tables[0].indexes.clear();
    assert_eq!(validate(&value).unwrap_err(), "orphan record");
    let mut value = small();
    value.tables[0].columns = vec![value.tables[0].columns[0].clone(); 33];
    assert_eq!(validate(&value).unwrap_err(), "column bound");
    let mut value = small();
    value.records[0].operands = vec![value.records[0].operands[0].clone(); 33];
    assert_eq!(
        validate(&value).unwrap_err(),
        "record/locator operand bound"
    );
    let mut value = small();
    value.records[0].operands[0].source = Source::Column { ordinal: 32 };
    assert_eq!(validate(&value).unwrap_err(), "operand column reference");
}
#[test]
fn rust_literals_inventory_and_output_bounds_are_explicit() {
    let mut out = Writer::new();
    inventory(&mut out, &serde_json::json!({"quote\"":"line\n\0\\é"}), 0).unwrap();
    let source = String::from_utf8(out.0 .0).unwrap();
    assert!(source.contains(r#"("quote\"",Inventory::Text("line\n\0\\é"))"#));
    let mut out = Writer::new();
    let maximum = Operand {
        source: Source::Column {
            ordinal: usize::from(u16::MAX),
        },
        descending: false,
        collation: "BINARY".into(),
        key: true,
    };
    render_operands(&mut out, std::slice::from_ref(&maximum)).unwrap();
    assert!(std::str::from_utf8(&out.0 .0)
        .unwrap()
        .contains("ordinal: 65535u16"));
    assert!(operands(&[maximum], 32).is_err());
    let mut out = Writer::new();
    inventory(&mut out, &serde_json::Value::from(u64::MAX), 0).unwrap();
    assert_eq!(
        std::str::from_utf8(&out.0 .0).unwrap(),
        "Inventory::Unsigned(18446744073709551615u64)"
    );
    let mut out = Writer::new();
    assert!(inventory(&mut out, &serde_json::json!(-1), 0).is_err());
    assert!(inventory(&mut out, &serde_json::Value::Null, 17).is_err());
    let mut out = Writer::new();
    let block = "x".repeat(1024);
    for _ in 0..ARTIFACT_LIMIT / 1024 {
        out.text(&block).unwrap();
    }
    assert!(out.text("x").is_err());
    assert_eq!(out.0 .0.len(), ARTIFACT_LIMIT);
}
#[test]
fn rust_export_is_create_new_and_preserves_existing_destination() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("new");
    publish(&path, b"reviewed source\n").unwrap();
    assert!(publish(&path, b"replacement\n").is_err());
    assert_eq!(
        std::fs::read(path.join(FILE_NAME)).unwrap(),
        b"reviewed source\n"
    );
    assert!(publish(&temporary.path().join("empty"), &[]).is_err());
    assert!(!temporary.path().join("empty").exists());
}
#[test]
#[ignore = "explicit development source export into a new directory"]
fn export_runtime_source() {
    let directory =
        std::env::var_os("FOKS_STORAGE_RUNTIME_OUTPUT").expect("FOKS_STORAGE_RUNTIME_OUTPUT");
    let source = render(&reference_bundle().unwrap()).unwrap();
    publish(Path::new(&directory), &source).unwrap();
}

// R1b: compiled runtime descriptors must retain every R0 field, independently of
// the renderer's source-string shape assertions above.
fn compiled_inventory(value: foks_schema_contract::storage::Inventory) -> serde_json::Value {
    use foks_schema_contract::storage::InventoryKind;
    match value.kind() {
        InventoryKind::Null => serde_json::Value::Null,
        InventoryKind::Bool => value.boolean().unwrap().into(),
        InventoryKind::Unsigned => value.unsigned().unwrap().into(),
        InventoryKind::Text => value.text().unwrap().into(),
        InventoryKind::Array => value.elements().unwrap().map(compiled_inventory).collect(),
        InventoryKind::Object => serde_json::Value::Object(
            value
                .members()
                .unwrap()
                .map(|(key, value)| (key.to_owned(), compiled_inventory(value)))
                .collect(),
        ),
    }
}
fn compiled_operand(value: foks_schema_contract::storage::Operand) -> serde_json::Value {
    use foks_schema_contract::storage::OperandSource;
    let source = match value.source() {
        OperandSource::Column { ordinal } => serde_json::json!({"kind":"column","ordinal":ordinal}),
        OperandSource::Rowid => serde_json::json!({"kind":"rowid"}),
        OperandSource::NodeFileSlice {
            ordinal,
            maximum_bytes,
        } => {
            serde_json::json!({"kind":"node_file_slice","ordinal":ordinal,"maximum_bytes":maximum_bytes})
        }
        OperandSource::ScopedHostOrEmpty {
            ordinal,
            maximum_bytes,
        } => {
            serde_json::json!({"kind":"scoped_host_or_empty","ordinal":ordinal,"maximum_bytes":maximum_bytes})
        }
        OperandSource::IntegerOne { maximum_bytes } => {
            serde_json::json!({"kind":"integer_one","maximum_bytes":maximum_bytes})
        }
    };
    serde_json::json!({"source":source,"descending":value.descending(),"collation":value.collation(),"key":value.key()})
}
fn compiled_component(component: foks_schema_contract::storage::Component) -> serde_json::Value {
    use foks_schema_contract::storage::{StorageType, HOSTED_SMALL_V1};
    let c = HOSTED_SMALL_V1.descriptor(component);
    let tables: Vec<_> = c.tables().map(|t| {
        let columns: Vec<_> = t.columns().map(|column| {
            let storage_type = match column.storage_type() { StorageType::Integer=>"integer", StorageType::Real=>"real", StorageType::Blob=>"blob", StorageType::Text=>"text" };
            serde_json::json!({"ordinal":column.ordinal(),"name":column.name(),"declared_type":column.declared_type(),"storage_type":storage_type,"nullable":column.nullable(),"explicit_not_null":column.explicit_not_null(),"primary_key_ordinal":column.primary_key_ordinal(),"integer_rowid_alias":column.integer_rowid_alias(),"bytes":{"maximum_bytes":column.maximum_bytes(),"source":column.policy_source()}})
        }).collect();
        let indexes: Vec<_> = t.indexes().map(|index| serde_json::json!({"name":index.name(),"origin":index.origin(),"unique":index.unique(),"partial":index.partial(),"partial_policy":index.partial_policy(),"sql_sha256":index.sql_sha256(),"record_id":index.record().name(),"table_btree_alias":index.table_btree_alias()})).collect();
        let locator = if let Some(spelling)=t.locator().rowid_spelling() { serde_json::json!({"kind":"rowid","spelling":spelling}) } else { serde_json::json!({"kind":"primary_key","operands":t.locator().primary_key().unwrap().map(compiled_operand).collect::<Vec<_>>()}) };
        serde_json::json!({"name":t.name(),"sql_sha256":t.sql_sha256(),"strict":t.strict(),"without_rowid":t.without_rowid(),"columns":columns,"record_id":t.record().name(),"locator":locator,"indexes":indexes})
    }).collect();
    let records: Vec<_> = c.records().map(|record| serde_json::json!({"id":record.name(),"table":record.table(),"kind":record.kind(),"operands":record.operands().map(compiled_operand).collect::<Vec<_>>()})).collect();
    serde_json::json!({"format_version":c.format_version(),"component":c.name(),"scope":c.scope(),"value_limit":c.value_limit(),"record_limit":c.record_limit(),"record_formula":c.record_formula(),"tables":tables,"records":records})
}
#[test]
fn compiled_runtime_preserves_every_trusted_field_and_exact_source() {
    use foks_schema_contract::storage::{Component, HOSTED_SMALL_V1};
    let bundle = reference_bundle().unwrap();
    assert_eq!(
        compiled_component(Component::Foks),
        serde_json::to_value(&bundle.foks).unwrap()
    );
    assert_eq!(
        compiled_component(Component::Authority),
        serde_json::to_value(&bundle.authority).unwrap()
    );
    assert_eq!(
        compiled_inventory(HOSTED_SMALL_V1.ownership()),
        bundle.ownership
    );
    assert_eq!(
        compiled_inventory(HOSTED_SMALL_V1.manifest()),
        serde_json::from_slice::<serde_json::Value>(
            &bundle.files["storage-descriptors-manifest.json"]
        )
        .unwrap()
    );
    assert_eq!(HOSTED_SMALL_V1.source_pins(), PINS);
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../foks-schema-contract/src/storage/generated.rs");
    let checked_in = std::fs::read(path).unwrap();
    assert_eq!(checked_in, render(&bundle).unwrap());
    assert_eq!(checked_in, render(&reference_bundle().unwrap()).unwrap());
}
