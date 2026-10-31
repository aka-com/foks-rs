#[path = "../support/fixtures.rs"]
mod fixtures;
#[path = "../support/schema.rs"]
mod schema;
#[path = "../support/verify.rs"]
mod verify;

use schema::Outcome;
use std::path::PathBuf;

fn corpus(target: &str) -> Vec<(String, Vec<u8>)> {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("corpus")
        .join(target);
    let mut seeds: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            (name, std::fs::read(path).unwrap())
        })
        .filter(|(name, _)| name.starts_with("go-") || name.starts_with("call-"))
        .collect();
    seeds.sort();
    seeds
}

/// Runs every decoder in `table` on a Go input and reports whether one
/// accepted it.
fn reaches(table: &[schema::Decoder], name: &str, bytes: &[u8]) -> bool {
    let mut reached = false;
    for (decoder, decode) in table {
        match decode(bytes) {
            Outcome::Rejected => {}
            Outcome::Accepted => reached = true,
            // Go encodes canonically, so an encoder that reproduces a
            // different encoding would change signed or hashed bytes.
            Outcome::Encoded(encoded) => {
                assert!(encoded == bytes, "{decoder} re-encodes {name} differently");
                reached = true;
            }
        }
    }
    reached
}

#[test]
fn every_decoder_handles_every_go_fixture() {
    let fixtures = fixtures::snowp();
    assert_eq!(fixtures.len(), 137);
    for table in [schema::PROTO, schema::SERVER] {
        assert!(table.len() <= 256);
    }
    let mut accepted = 0;
    for (name, bytes) in &fixtures {
        accepted += usize::from(reaches(schema::PROTO, name, bytes));
        reaches(schema::SERVER, name, bytes);
    }
    // Most fixtures are typed values; the rest are raw IDs, inner payloads
    // and derivation inputs that no standalone decoder takes.
    assert!(
        accepted >= 116,
        "only {accepted} fixtures decode as a typed value"
    );
}

#[test]
fn every_decoder_handles_every_go_call_argument() {
    let calls = fixtures::call_arguments();
    assert_eq!(calls.len(), 90);
    let mut accepted = 0;
    for (name, bytes) in &calls {
        reaches(schema::PROTO, name, bytes);
        accepted += usize::from(reaches(schema::SERVER, name, bytes));
    }
    // The rest are calls, such as the KV, signup and SSO requests, whose
    // arguments the server decodes outside `foks_rpc::arguments`.
    assert!(
        accepted >= 58,
        "only {accepted} Go call arguments reach a server decoder"
    );
}

fn assert_seeds_select_their_decoder(target: &str, table: &'static [schema::Decoder]) {
    let seeds = corpus(target);
    assert!(!seeds.is_empty(), "{target} has no Go fixture seeds");
    for (name, seed) in seeds {
        let ((decoder, decode), bytes) = schema::select(table, &seed).unwrap();
        assert!(
            name.ends_with(&schema::seed_name("", decoder)),
            "{target}/{name} selects {decoder}; regenerate the seeds"
        );
        assert!(
            !matches!(decode(bytes), Outcome::Rejected),
            "{decoder} rejects {target}/{name}"
        );
    }
}

#[test]
fn proto_schema_seeds_select_their_decoder() {
    assert_seeds_select_their_decoder("proto_schema", schema::PROTO);
}

#[test]
fn server_argument_seeds_select_their_decoder() {
    assert_seeds_select_their_decoder("server_arguments", schema::SERVER);
}

#[test]
fn verify_seeds_pass_every_stage() {
    let seeds = corpus("verify");
    assert_eq!(seeds.len(), verify::STAGES.len());
    for (name, seed) in seeds {
        let stage = &verify::STAGES[usize::from(seed[0]) % verify::STAGES.len()];
        assert_eq!(
            verify::run(&seed),
            Some(true),
            "{} rejects verify/{name}",
            stage.name
        );
    }
}

#[test]
fn snowpack_and_rpc_corpora_hold_every_go_fixture() {
    let snowpack = corpus("snowpack");
    let rpc = corpus("rpc");
    let fixtures = fixtures::snowp();
    assert_eq!(snowpack.len(), fixtures.len());
    assert_eq!(rpc.len(), fixtures.len());
    for (((name, bytes), seed), response) in fixtures.iter().zip(&snowpack).zip(&rpc) {
        assert_eq!(seed, &(name.clone(), bytes.clone()));
        assert_eq!(response.0, *name);
        let content =
            foks_rpc::read_frame(&mut std::io::Cursor::new(&response.1), 1_048_576).unwrap();
        assert_eq!(
            foks_rpc::decode_probe_response(&content, 0).unwrap(),
            *bytes
        );
    }
}
