//! Isolated local server/account fixture for the production WebView driver.
//! All mutations under test are sent by the real desktop UI; stdin exposes
//! only fault arming and read-only assertions after initial synthetic seeding.
use std::io::{BufRead, Write};
use std::path::PathBuf;

use foks_client_app::{
    derive_vault_key, AccountVault, ClientCredentials, CredentialBackend, Profile, ProfileRegistry,
    ProfileSession, ProtocolPolicy, TrustRoot,
};
use foks_keystore::EncryptedFileSecretStore;
use foks_server_testkit::{TestEnvironment, TestFault};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let state = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("state directory required")?,
    );
    // The Python parent creates a unique private directory. Never initialize
    // over an existing credential store or use the user's normal state path.
    if state.exists() {
        return Err("fixture state directory must not exist".into());
    }
    std::fs::create_dir(&state)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700))?;
    }
    let environment = TestEnvironment::new()?;
    let _server = environment.start_server()?;
    let root = state.join("probe-root.der");
    environment.write_probe_root(&root)?;
    ClientCredentials::initialize(&state, CredentialBackend::PrivateFile)?;
    let mut registry = ProfileRegistry::open(&state)?;
    registry.add(Profile {
        name: "webview".into(),
        label: None,
        probe: format!(
            "localhost:{}",
            environment
                .addresses()
                .ok_or("server address")?
                .probe
                .port()
        ),
        protocol: ProtocolPolicy::V019,
        trust: TrustRoot::CertificateDer { path: root },
    })?;
    let profile = ProfileSession::open(&registry, "webview")?;
    let credentials = ClientCredentials::open(&state)?;
    credentials.with_checked_session(
        &profile,
        |session| -> Result<_, Box<dyn std::error::Error>> {
            session.probe_and_pin()?;
            let master = credentials.master_key()?;
            let mut store = EncryptedFileSecretStore::open(
                &session.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut vault = AccountVault::new(&mut store);
            session.create_account(
                "owner",
                "webviewuser",
                "fixture",
                "webview@example.test",
                "",
                None,
                &mut vault,
                &master,
            )?;
            session.put_kv_file(
                "owner",
                "/webview-note.txt",
                &mut b"initial text".as_slice(),
                false,
                false,
                &mut vault,
                &master,
            )?;
            session.put_kv_file(
                "owner",
                "/webview-file.bin",
                &mut vec![0x61; 4096].as_slice(),
                false,
                false,
                &mut vault,
                &master,
            )?;
            Ok(())
        },
    )?;
    println!("{}", json!({"ready": true}));
    std::io::stdout().flush()?;
    for line in std::io::stdin().lock().lines() {
        let command: serde_json::Value = serde_json::from_str(&line?)?;
        let result = match command["command"].as_str() {
            Some("arm-lost-reply") => json!({"hits_before": environment.arm_fault(TestFault::KvPutAfterCommitBeforeResponse)}),
            Some("inspect") => credentials.with_checked_session(&profile, |session| -> Result<_, Box<dyn std::error::Error>> {
                let master = credentials.master_key()?;
                let mut store = EncryptedFileSecretStore::open(&session.paths().credential_store, derive_vault_key(&master))?;
                let mut vault = AccountVault::new(&mut store);
                let catalog = session.list_kv_metadata("owner", &mut vault)?;
                let note = session.read_kv_file("owner", "/webview-note.txt", &mut vault)?;
                let file = session.read_kv_file("owner", "/webview-file.bin", &mut vault)?;
                let database = rusqlite::Connection::open_with_flags(&session.paths().hard_database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
                let journals: i64 = database.query_row("SELECT count(*) FROM mutation_operations WHERE operation_kind=5", [], |row| row.get(0))?;
                Ok(json!({"entries": catalog.entries, "note": String::from_utf8_lossy(&note), "file_bytes": file.len(), "file_all_replacement": file.iter().all(|b| *b == 0x62), "namespace_journals": journals, "fault_hits": environment.fault_hits()}))
            })?,
            Some("stop") => break,
            _ => return Err("unknown fixture command".into()),
        };
        println!("{result}");
        std::io::stdout().flush()?;
    }
    Ok(())
}
