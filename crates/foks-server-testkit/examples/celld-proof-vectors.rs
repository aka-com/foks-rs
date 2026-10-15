//! Generate full native-schema SQL vectors; no network, real credentials or FOKS
//! protocol-valid identity is claimed by this lower-level authority fixture.
use std::path::PathBuf;

use base64::Engine as _;
use foks_server_testkit::celld_proof::{write_json, Schema};

fn main() -> foks_server_testkit::celld_proof::Result<()> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: celld-proof-vectors NEW_OUTPUT_DIRECTORY")?;
    std::fs::create_dir(&output)?;
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("fixture.sqlite");
    drop(foks_server_db::Database::open(
        &path,
        foks_server_db::Config::default(),
    )?);
    let connection = rusqlite::Connection::open(&path)?;
    connection.execute_batch("PRAGMA foreign_keys=ON;
      INSERT INTO host_metadata VALUES(1,zeroblob(33),'proof.invalid',X'00',X'00');
      INSERT INTO kv_namespaces(namespace_id,party_kind,host_id) VALUES(zeroblob(33),1,zeroblob(33));
      INSERT INTO kv_directories VALUES(zeroblob(33),zeroblob(16),1,3,0,1,0,X'010203');
      INSERT INTO kv_directory_heads VALUES(zeroblob(33),zeroblob(16),1);
      INSERT INTO kv_roots VALUES(zeroblob(33),1,zeroblob(16),1,X'0405');")?;
    let schema = Schema::export(&connection)?;
    let before = schema.snapshot(&connection)?;
    connection.execute_batch("BEGIN IMMEDIATE;
      INSERT INTO kv_nodes VALUES(zeroblob(33),X'0300000000000000000000000000000000',3,X'000102ff');
      INSERT INTO kv_dirents VALUES(zeroblob(33),zeroblob(16),zeroblob(16),1,1,X'0300000000000000000000000000000000',zeroblob(32),9007199254740993,X'070809');
      INSERT INTO kv_dirent_heads VALUES(zeroblob(33),zeroblob(16),zeroblob(16),1);
      COMMIT;")?;
    let after = schema.snapshot(&connection)?;
    let delta = schema.delta(&before, &after)?;
    let vectors = serde_json::json!({"schema":schema,"before":before,"after":after,"delta":delta,"host_id":base64::engine::general_purpose::STANDARD.encode([0;33])});
    let file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output.join("vectors.json"))?;
    write_json(&vectors, &file)?;
    file.sync_all()?;
    // Deployment reads this schema as trusted code, never as public request DDL.
    let file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output.join("generated-schema.json"))?;
    write_json(&schema, &file)?;
    file.sync_all()?;
    Ok(())
}
