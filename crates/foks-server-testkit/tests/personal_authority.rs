#![cfg(feature = "celld-proof")]
//! Actual Rust/Go public account setup; private real-principal SQL qualification
//! after stopping that server. This does not enable an HTTP production adapter.
#[path = "personal_authority/mod.rs"]
mod support;
use base64::Engine;
use foks_client::KvWriteOptions;
use foks_proto::Role;
use foks_server_testkit::{TestAccountSpec, TestClient, TestEnvironment};
use std::{io::Cursor, process::Command};
use support::*;

#[test]
fn actual_rust_certificate_personal_authority_and_vectors() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "personal-rust").unwrap();
    let host = client.probe_and_pin().unwrap().pinned;
    let account = client
        .create_account(&host, &TestAccountSpec::new("personalrust", 0x73))
        .unwrap();
    let mut protected = client.open_protected_store().unwrap();
    client
        .foks()
        .user_kv_write_session(
            &host,
            &account.credential,
            &account.authenticated.verified,
            &account.authenticated.puks,
            client.soft_state_path(),
            &mut protected,
        )
        .unwrap()
        .put_file(
            account.kv_projection[0].root_directory_id,
            "personal.txt",
            &mut Cursor::new(b"real encrypted small text"),
            KvWriteOptions {
                read_role: Role::OWNER,
                write_role: Role::OWNER,
                overwrite: false,
                expected_version: None,
            },
        )
        .unwrap();
    // Exercise actual native renewal issuance while both old/new certificates
    // remain valid at the explicitly advanced fixture clock. This is not a claim
    // that wall-clock TLS accepts a future-dated renewal today.
    let now = environment.advance_clock(6 * 24 * 60 * 60 * 1_000_000 + 1_000_000);
    let renewed = client
        .foks()
        .fetch_device_certificate_chain(&host, &account.credential.uid, &account.credential.seed)
        .unwrap();
    assert_ne!(renewed[0], account.credential.certificate_chain[0]);
    let material = account.credential.public_material().unwrap();
    compare_native_keys(&environment, &account.credential.certificate_chain[0], now);
    server.shutdown().unwrap();
    qualify(
        &environment,
        "rust",
        &account.credential.certificate_chain[0],
        account.credential.uid.as_bytes(),
        material.id.as_bytes(),
        Some(&renewed[0]),
        now,
    );
}

#[test]
#[ignore = "requires pinned Go v0.1.9 oracle; mandatory explicit A1 qualification gate"]
fn actual_go_certificate_personal_authority_and_vectors() {
    let oracle = std::env::var_os("FOKS_GO_ORACLE_DIR").expect("pinned Go oracle path required");
    let environment = TestEnvironment::new().unwrap();
    // The unchanged full Go flow also uploads a large file. Keep its ciphertext
    // external so scalar metadata snapshots remain within fixture cell bounds;
    // these A1 vectors never read, mutate, or qualify those large-file objects.
    environment.use_filesystem_chunks().unwrap();
    environment
        .set_invite_regime(foks_server_db::InviteRegime::Optional)
        .unwrap();
    let server = environment.start_server().unwrap();
    let root = environment
        .client_path("personal-go", "probe-root.der")
        .unwrap();
    let context = environment
        .client_path("personal-go", "public-context.json")
        .unwrap();
    environment.write_probe_root(&root).unwrap();
    let output = Command::new("go")
        .args(["test", "-C"])
        .arg(oracle)
        .args([
            "-mod=readonly",
            "-run",
            "^TestGoClientAgainstRustServer$",
            "-count=1",
            "-timeout=2m",
            "-v",
        ])
        .env("FOKS_GO_RUST_PROBE", server.addresses().probe.to_string())
        .env("FOKS_GO_RUST_CA_DER", root)
        .env("FOKS_GO_PERSONAL_CONTEXT_PATH", &context)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Go flow failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(context).unwrap()).unwrap();
    assert_eq!(value["schema"], 1);
    let der = base64::engine::general_purpose::STANDARD
        .decode(value["certificate_der"].as_str().unwrap())
        .unwrap();
    let uid = hex(value["uid"].as_str().unwrap());
    let credential = hex(value["credential_id"].as_str().unwrap());
    let now = environment.advance_clock(0);
    compare_native_keys(&environment, &der, now);
    server.shutdown().unwrap();
    qualify(&environment, "go", &der, &uid, &credential, None, now);
}
fn hex(text: &str) -> Vec<u8> {
    assert_eq!(text.len() % 2, 0);
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}
