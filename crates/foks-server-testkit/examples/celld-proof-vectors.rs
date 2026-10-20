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
    let mut database = foks_server_db::Database::open(&path, foks_server_db::Config::default())?;
    let connection = rusqlite::Connection::open(&path)?;
    connection.execute_batch("PRAGMA foreign_keys=ON;
      INSERT INTO names(normalized_name,reservation_sequence,uid) VALUES(X'70726f6f66',1,zeroblob(33));
      INSERT INTO users(uid,normalized_name,username_utf8,username_sequence,username_commitment_key,created_at) VALUES(zeroblob(33),X'70726f6f66',X'70726f6f66',1,zeroblob(16),0);
      INSERT INTO host_metadata VALUES(1,zeroblob(33),'proof.invalid',X'00',X'00');
      INSERT INTO kv_namespaces(namespace_id,party_kind,host_id) VALUES(zeroblob(33),1,zeroblob(33));
      INSERT INTO kv_directories VALUES(zeroblob(33),zeroblob(16),1,3,0,1,0,X'010203');
      INSERT INTO kv_directory_heads VALUES(zeroblob(33),zeroblob(16),1);
      INSERT INTO kv_roots VALUES(zeroblob(33),1,zeroblob(16),1,X'0405');")?;
    let schema = Schema::export(&connection)?;
    let before = schema.snapshot(&connection)?;
    connection.execute_batch("INSERT INTO kv_nodes VALUES(zeroblob(33),X'0300000000000000000000000000000000',3,X'000102ff');")?;
    let mut node = [0_u8; 17];
    node[0] = 3;
    database.put_kv_dirents(
        &[0; 33],
        None,
        foks_proto::Role::OWNER,
        &[foks_server_db::KvDirentMutation {
            parent: &[0; 16],
            id: &[0; 16],
            version: 1,
            directory_version: 1,
            node_id: &node,
            name_mac: &[0; 32],
            creation_time: 9_007_199_254_740_993,
            exact: &[7, 8, 9],
        }],
    )?;
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
