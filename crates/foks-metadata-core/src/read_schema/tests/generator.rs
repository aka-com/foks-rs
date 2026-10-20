//! Development-only trusted source generation. This module cannot be called by
//! serving code and never accepts an existing database or expected schema path.
use foks_schema_contract::{Contract, ContractObject, SchemaObject};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt::Write as _, io::Write as _, path::Path};

const ENGINE: &[u8] =
    include_bytes!("../../../../../tools/foks-metadata-spike/celld-directory-schema.json");
const ENUMERATE: &str = "SELECT type,name,tbl_name,sql FROM sqlite_schema WHERE typeof(name) != 'text' OR substr(name,1,7) != 'sqlite_' COLLATE NOCASE ORDER BY type COLLATE BINARY,name COLLATE BINARY,tbl_name COLLATE BINARY";
const FOKS_59_HASH: [u8; 32] = [
    0x44, 0x4f, 0xc9, 0x34, 0xa1, 0x33, 0x6a, 0x96, 0x7d, 0xb1, 0xd8, 0x9e, 0x07, 0x86, 0x32, 0x18,
    0x3c, 0x2c, 0x5b, 0x00, 0x1a, 0xdb, 0x42, 0x2d, 0xf7, 0x07, 0xa6, 0x43, 0x9b, 0x5e, 0xdb, 0x3b,
];
type Object = [String; 4];
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EngineObject {
    kind: String,
    sql: String,
}
fn schema(connection: &Connection) -> Vec<Object> {
    connection
        .prepare(ENUMERATE)
        .unwrap()
        .query_map([], |row| {
            Ok([row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?])
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}
fn contract(objects: &[Object]) -> Result<Contract<'_>, foks_schema_contract::ContractError> {
    Contract::new(
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
}
fn union(parts: &[&[Object]]) -> Result<Vec<Object>, foks_schema_contract::ContractError> {
    let mut objects: Vec<_> = parts.iter().flat_map(|p| p.iter().cloned()).collect();
    objects.sort_by(|a, b| (&a[0], &a[1], &a[2]).cmp(&(&b[0], &b[1], &b[2])));
    contract(&objects)?; // rejects cross-component duplicate keys, even equal SQL
    Ok(objects)
}
fn bytes(value: &impl Serialize) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(value).unwrap();
    bytes.push(b'\n');
    bytes
}
fn hash(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(text, "{byte:02x}").unwrap();
    }
    text
}
fn source_objects(name: &str, objects: &[Object], rust: &mut String) {
    writeln!(
        rust,
        "pub(super) const {name}: &[foks_schema_contract::SchemaObject<'static>] = &["
    )
    .unwrap();
    for [kind, name, table, sql] in objects {
        writeln!(rust, "    foks_schema_contract::SchemaObject {{ kind: {kind:?}, name: {name:?}, table: {table:?}, sql: {sql:?} }},").unwrap();
    }
    writeln!(rust, "];\n").unwrap();
}
fn generate() -> BTreeMap<String, Vec<u8>> {
    assert_eq!(foks_server_db::SCHEMA_VERSION, 59);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("new-profile-reference.sqlite");
    let database =
        foks_server_db::Database::open(&path, foks_server_db::Config::default()).unwrap();
    // The Database stays alive, but no DDL/data mutations occur after admission.
    let reader =
        Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let foks = schema(&reader);
    assert_eq!(*contract(&foks).unwrap().hash(), FOKS_59_HASH);
    drop(reader);
    drop(database);

    let authority_db = Connection::open_in_memory().unwrap();
    authority_db
        .execute_batch(crate::authority::SCHEMA)
        .unwrap();
    let authority = schema(&authority_db);
    let engine_db = Connection::open_in_memory().unwrap();
    let definitions: Vec<EngineObject> = serde_json::from_slice(ENGINE).unwrap();
    assert_eq!(definitions.len(), 10);
    for definition in &definitions {
        assert!(matches!(definition.kind.as_str(), "table" | "trigger"));
        engine_db.execute_batch(&definition.sql).unwrap();
    }
    let engine = schema(&engine_db);
    assert_eq!(engine.len(), 10);
    let native = union(&[&foks, &authority]).unwrap();
    let celld = union(&[&foks, &authority, &engine]).unwrap();
    for objects in [&foks, &authority, &engine, &native, &celld] {
        assert!(objects.len() <= 1024);
        let mut sql_bytes = 0usize;
        for [kind, name, table, sql] in objects {
            assert!(
                kind.len() <= 8 && name.len() <= 256 && table.len() <= 256 && sql.len() <= 65_536
            );
            sql_bytes = sql_bytes.checked_add(sql.len()).unwrap();
        }
        assert!(sql_bytes <= 2_097_152);
    }

    let scalar = crate::catalog::Statement::ALL.map(|statement| statement.spec());
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    files.insert("foks-59.json".into(), bytes(&foks));
    files.insert("authority-v1.json".into(), bytes(&authority));
    files.insert("celld-0.6.2.json".into(), bytes(&engine));
    // Preserve exact deployed artifact encodings: scalar has no added newline;
    // page includes its ABI descriptor and fingerprint, not a raw Spec array.
    let scalar_bytes = serde_json::to_vec(scalar.as_slice()).unwrap();
    let page_bytes = crate::page::catalog_artifact().unwrap();
    assert_eq!(
        hex(&hash(&scalar_bytes)),
        "07b52d5b3405ec6d6826a3cb66c8f7a41422edd890179c6615d46d469af1f705"
    );
    assert_eq!(
        hex(&hash(&page_bytes)),
        "a2b83eb458c38a39b3245fbcaa669bd9bd85255db0be08c4124813248b4f955b"
    );
    files.insert("scalar-catalog.json".into(), scalar_bytes);
    files.insert("page-catalog.json".into(), page_bytes);
    let mut rust = String::from(
        "// Generated from reviewed source DDL; do not use observed target schemas.\n",
    );
    source_objects("FOKS", &foks, &mut rust);
    source_objects("AUTHORITY", &authority, &mut rust);
    source_objects("CELLD", &engine, &mut rust);
    files.insert("objects.rs".into(), rust.into_bytes());
    let hashes: BTreeMap<_, _> = files
        .iter()
        .map(|(name, bytes)| (name.clone(), hash(bytes)))
        .collect();
    // This intermediate manifest is intentionally not a complete/readable
    // activation profile. Final profile hashes wait for the guarded ABI review.
    let manifest = serde_json::json!({
        "encoding_version":1, "schema_version":59, "qualification_only":true,
        "stage":"trusted_source_only", "asset_hashes":hashes,
        "source_hashes":{"authority_sql":hash(crate::authority::SCHEMA.as_bytes()), "engine_json":hash(ENGINE)},
        "components": {
            "foks":{"count":foks.len(), "hash":contract(&foks).unwrap().hash()},
            "authority":{"count":authority.len(), "hash":contract(&authority).unwrap().hash()},
            "engine":{"count":engine.len(), "hash":contract(&engine).unwrap().hash()},
            "empty_engine":{"count":0, "hash":contract(&[]).unwrap().hash()},
            "native_union":{"count":native.len(), "hash":contract(&native).unwrap().hash()},
            "celld_union":{"count":celld.len(), "hash":contract(&celld).unwrap().hash()}
        }
    });
    files.insert("source-manifest.json".into(), bytes(&manifest));
    files
}
fn publish_new(directory: &Path, files: BTreeMap<String, Vec<u8>>) {
    std::fs::create_dir(directory).expect("output must be a new directory; no overwrite");
    for (name, bytes) in files {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join(name))
            .unwrap();
        file.write_all(&bytes).unwrap();
        file.sync_all().unwrap();
    }
}
#[test]
fn trusted_generation_is_reproducible_and_rejects_component_collisions() {
    assert_eq!(generate(), generate());
    let object = [
        "table".into(),
        "a".into(),
        "a".into(),
        "CREATE TABLE a(x)".into(),
    ];
    let part = [object];
    assert!(matches!(
        union(&[&part, &part]),
        Err(foks_schema_contract::ContractError::DuplicateObject)
    ));
}
#[test]
#[ignore = "explicit reviewed source generation; no serving profile is produced"]
fn export_trusted_profile_sources() {
    let output = std::env::var_os("FOKS_READ_SCHEMA_PROFILE_OUTPUT")
        .expect("explicit fresh output directory required");
    publish_new(Path::new(&output), generate());
}
