//! Development-only restricted-storage descriptors. No runtime admission API.
mod descriptor;
mod introspection;
mod policy;
mod record_oracle;
mod runtime;
mod tests;

use descriptor::*;
use foks_schema_contract::{Contract, ContractObject, SchemaObject};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt::Write as _, io::Write as _, path::Path};

const ENGINE: &[u8] =
    include_bytes!("../../../../../../tools/foks-metadata-spike/celld-directory-schema.json");
const FOKS_60_HASH: &str = "3111af3c9542eb8845461009f55042925c0fa7d056b4d417f591042ae4074c64";
type Object = [String; 4];
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EngineObject {
    kind: String,
    sql: String,
}
fn hash(value: &[u8]) -> String {
    let mut output = String::with_capacity(64);
    for byte in Sha256::digest(value) {
        write!(output, "{byte:02x}").unwrap();
    }
    output
}
fn objects(connection: &Connection) -> Result<Vec<Object>> {
    introspection::ensure_namespaces(connection)?;
    let mut statement = connection.prepare("SELECT type,name,tbl_name,sql FROM main.sqlite_schema WHERE typeof(name) != 'text' OR substr(name,1,7) != 'sqlite_' COLLATE NOCASE ORDER BY type COLLATE BINARY,name COLLATE BINARY,tbl_name COLLATE BINARY").map_err(|e|e.to_string())?;
    let mut rows = statement.query([]).map_err(|e| e.to_string())?;
    let mut output = Vec::new();
    let mut sql_bytes = 0usize;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        if output.len() >= 1024 {
            return Err("schema object count bound".into());
        }
        let mut object: Object = std::array::from_fn(|_| String::new());
        for (i, maximum) in [8, 256, 256, 65_536].into_iter().enumerate() {
            let raw = row.get_ref(i).map_err(|e| e.to_string())?;
            let text = raw.as_str().map_err(|e| e.to_string())?;
            if text.len() > maximum {
                return Err("schema field byte bound".into());
            }
            object[i] = text.to_owned();
        }
        if !matches!(object[0].as_str(), "table" | "index" | "trigger") {
            return Err("unsupported schema object kind".into());
        }
        sql_bytes = sql_bytes
            .checked_add(object[3].len())
            .ok_or("schema size overflow")?;
        if sql_bytes > 2_097_152 {
            return Err("schema SQL aggregate bound".into());
        }
        output.push(object);
    }
    Ok(output)
}
fn sqlite_owned(connection: &Connection, component: &Component) -> Result<serde_json::Value> {
    introspection::ensure_namespaces(connection)?;
    let mut statement=connection.prepare("SELECT type,name,tbl_name,sql FROM main.sqlite_schema WHERE substr(name,1,7) = 'sqlite_' COLLATE NOCASE ORDER BY type,name,tbl_name").map_err(|e|e.to_string())?;
    let mut rows = statement.query([]).map_err(|e| e.to_string())?;
    let mut owned = Vec::new();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        if owned.len() >= MAX_RECORDS {
            return Err("SQLite-owned object bound".into());
        }
        let bounded = |index, maximum| -> Result<String> {
            let v = row.get_ref(index).map_err(|e| e.to_string())?;
            let t = v.as_str().map_err(|e| e.to_string())?;
            if t.len() > maximum {
                return Err("SQLite-owned metadata bound".into());
            }
            Ok(t.into())
        };
        let kind = bounded(0, 8)?;
        let name = bounded(1, 256)?;
        let table = bounded(2, 256)?;
        let sql = match row.get_ref(3).map_err(|e| e.to_string())? {
            rusqlite::types::ValueRef::Null => None,
            _ => Some(bounded(3, 65_536)?),
        };
        let classification = if kind == "index" && sql.is_none() {
            let t = component
                .tables
                .iter()
                .find(|t| t.name == table)
                .ok_or("SQLite index has no descriptor owner")?;
            let i = t
                .indexes
                .iter()
                .find(|i| i.name == name && i.origin != "c")
                .ok_or("SQLite index not covered by descriptor")?;
            if !component.records.iter().any(|r| r.id == i.record_id) {
                return Err("SQLite index record absent".into());
            }
            if component.component == "engine_inventory_only" {
                serde_json::json!({"kind":"automatic_index_engine_owned","owning_platform_table":table,"table_btree_alias":i.table_btree_alias})
            } else {
                serde_json::json!({"kind":"automatic_index_covered","record_id":i.record_id,"table_btree_alias":i.table_btree_alias})
            }
        } else if kind == "table"
            && name == "sqlite_sequence"
            && table == "sqlite_sequence"
            && sql.as_deref() == Some("CREATE TABLE sqlite_sequence(name,seq)")
        {
            serde_json::json!({"kind":"sqlite_owned_data_unadmitted","import_policy":"R2_R3_explicit_semantics_required_no_raw_snapshot_readiness"})
        } else {
            return Err(format!("unclassified SQLite-owned object {name}"));
        };
        owned.push(serde_json::json!({"schema":"main","kind":kind,"name":name,"table":table,"sql":sql,"classification":classification}));
    }
    // sqlite_schema is implicit rather than a row in its own catalog. Statistics
    // tables are unsupported above, so absence is a checked statement here.
    Ok(
        serde_json::json!({"implicit_catalog":"main.sqlite_schema","statistics_tables_present":[],"objects":owned,"scope":"ownership_only_not_data_admission"}),
    )
}
fn contract_hash(objects: &[Object]) -> Result<String> {
    let contract = Contract::new(
        objects
            .iter()
            .map(|o| {
                ContractObject::borrowed(SchemaObject {
                    kind: &o[0],
                    name: &o[1],
                    table: &o[2],
                    sql: &o[3],
                })
            })
            .collect(),
    )
    .map_err(|e| e.to_string())?;
    let mut output = String::with_capacity(64);
    for byte in contract.hash() {
        write!(output, "{byte:02x}").unwrap();
    }
    Ok(output)
}
fn union(parts: &[&[Object]]) -> Result<Vec<Object>> {
    let count = parts.iter().try_fold(0usize, |count, part| {
        count.checked_add(part.len()).ok_or("union count overflow")
    })?;
    if count > 1024 {
        return Err("union object bound".into());
    }
    let mut output: Vec<_> = parts.iter().flat_map(|p| p.iter().cloned()).collect();
    output.sort_by(|a, b| (&a[0], &a[1], &a[2]).cmp(&(&b[0], &b[1], &b[2])));
    contract_hash(&output)?;
    Ok(output)
}
struct BoundedJson(Vec<u8>);
impl std::io::Write for BoundedJson {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let end = self
            .0
            .len()
            .checked_add(bytes.len())
            .ok_or(std::io::ErrorKind::InvalidData)?;
        if end > ARTIFACT_LIMIT {
            return Err(std::io::ErrorKind::FileTooLarge.into());
        }
        if end > self.0.capacity() {
            let doubled = self
                .0
                .capacity()
                .checked_mul(2)
                .ok_or(std::io::ErrorKind::InvalidData)?;
            let target = end.max(doubled.clamp(256, ARTIFACT_LIMIT));
            self.0
                .try_reserve_exact(target - self.0.len())
                .map_err(|_| std::io::ErrorKind::OutOfMemory)?;
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn bytes(value: &impl Serialize) -> Result<Vec<u8>> {
    let mut writer = BoundedJson(Vec::new());
    serde_json::to_writer(&mut writer, value).map_err(|e| e.to_string())?;
    writer.write_all(b"\n").map_err(|e| e.to_string())?;
    Ok(writer.0)
}
struct ReferenceBundle {
    foks: Component,
    authority: Component,
    ownership: serde_json::Value,
    files: BTreeMap<String, Vec<u8>>,
}
fn generate() -> Result<BTreeMap<String, Vec<u8>>> {
    Ok(reference_bundle()?.files)
}
fn reference_bundle() -> Result<ReferenceBundle> {
    if foks_server_db::SCHEMA_VERSION != 60 {
        return Err("schema version needs review".into());
    }
    let temporary = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = temporary.path().join("fresh-restricted-reference.sqlite");
    let database = foks_server_db::Database::open(&path, foks_server_db::Config::default())
        .map_err(|e| e.to_string())?;
    let connection = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    let foks_objects = objects(&connection)?;
    let foks_hash = contract_hash(&foks_objects)?;
    if foks_hash != FOKS_60_HASH {
        return Err("native source golden changed".into());
    }
    let foks = introspection::component(&connection, "foks")?;
    policy::require_overrides(&foks)?;
    policy::require_expressions(&foks)?;
    policy::require_collations(&foks)?;
    let foks_sqlite_owned = sqlite_owned(&connection, &foks)?;
    drop(connection);
    drop(database);

    let authority_connection = Connection::open_in_memory().map_err(|e| e.to_string())?;
    authority_connection
        .execute_batch(crate::authority::SCHEMA)
        .map_err(|e| e.to_string())?;
    let authority_objects = objects(&authority_connection)?;
    let authority = introspection::component(&authority_connection, "qualification_authority_v1")?;
    let authority_sqlite_owned = sqlite_owned(&authority_connection, &authority)?;
    let engine_connection = Connection::open_in_memory().map_err(|e| e.to_string())?;
    let engine_source: Vec<EngineObject> =
        serde_json::from_slice(ENGINE).map_err(|e| e.to_string())?;
    if engine_source.len() != 10 {
        return Err("engine source count changed".into());
    }
    for object in &engine_source {
        if !matches!(object.kind.as_str(), "table" | "trigger") || object.sql.len() > 65_536 {
            return Err("engine source shape".into());
        }
        engine_connection
            .execute_batch(&object.sql)
            .map_err(|e| e.to_string())?;
    }
    let engine_objects = objects(&engine_connection)?;
    if engine_objects.len() != 10 {
        return Err("engine objects changed".into());
    }
    let engine_component = introspection::component(&engine_connection, "engine_inventory_only")?;
    let engine_sqlite_owned = sqlite_owned(&engine_connection, &engine_component)?;
    let complete = union(&[&foks_objects, &authority_objects, &engine_objects])?;
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    files.insert("foks-records-v1.json".into(), bytes(&foks)?);
    files.insert("authority-records-v1.json".into(), bytes(&authority)?);
    let ownership = serde_json::json!({"format_version":1,"scope":"engine_owned_not_application_storage_admission","objects":engine_objects,"sqlite_owned":{"foks":foks_sqlite_owned,"authority":authority_sqlite_owned,"engine":engine_sqlite_owned}});
    files.insert("engine-ownership-v1.json".into(), bytes(&ownership)?);
    let file_hashes: BTreeMap<&str, String> = files
        .iter()
        .map(|(name, value)| (name.as_str(), hash(value)))
        .collect();
    let manifest = serde_json::json!({
        "format_version":1,"profile":"hosted-metadata-small-v1","scope":"trusted_descriptors_only","schema_version":60,
        "native_sqlite_version":rusqlite::version(),"pinned_celld_version":"0.6.2",
        "value_limit":VALUE_LIMIT,"record_limit":RECORD_LIMIT,"record_formula":"9 + 9 * operand_count + sum(payload_upper_bounds)",
        "partial_index_policy":"check_record_bound_for_every_row","files":file_hashes,
        "source_hashes":{"foks_contract":foks_hash,"authority_sql":hash(crate::authority::SCHEMA.as_bytes()),"authority_contract":contract_hash(&authority_objects)?,"engine_json":hash(ENGINE),"complete_union":contract_hash(&complete)?},
        "foks":{"tables":foks.tables.len(),"index_list_entries":foks.tables.iter().map(|t|t.indexes.len()).sum::<usize>(),"table_btree_aliases":foks.tables.iter().flat_map(|t|&t.indexes).filter(|i|i.table_btree_alias).count(),"physical_records":foks.records.len()},
        "authority":{"tables":authority.tables.len(),"physical_records":authority.records.len()},
        "overrides":[["foks","kv_nodes","exact_node","65536"],["foks","kv_dirents","exact_dirent","4096"],["foks","team_members","scoped_host_id","33"]]
    });
    let manifest_bytes = bytes(&manifest)?;
    files.insert("storage-descriptors-manifest.json".into(), manifest_bytes);
    Ok(ReferenceBundle {
        foks,
        authority,
        ownership,
        files,
    })
}
fn publish_new(directory: &Path, files: BTreeMap<String, Vec<u8>>) -> Result<()> {
    const NAMES: [&str; 4] = [
        "authority-records-v1.json",
        "engine-ownership-v1.json",
        "foks-records-v1.json",
        "storage-descriptors-manifest.json",
    ];
    if !files.keys().map(String::as_str).eq(NAMES)
        || files.values().any(|v| v.len() > ARTIFACT_LIMIT)
    {
        return Err("output set or bound differs".into());
    }
    std::fs::create_dir(directory).map_err(|e| e.to_string())?;
    for (name, bytes) in files {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join(name))
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
    }
    std::fs::File::open(directory)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(())
}
