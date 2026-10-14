//! Opt-in real-binary recovery experiments; see ../RECOVERY.md.

use std::io::Cursor;
use std::path::PathBuf;
use std::time::Duration;

use foks_client::{AuthenticatedUserOutcome, DeviceCredential, KvWriteOptions, PinnedHost};
use foks_proto::Role;
use foks_server_testkit::{
    BinaryServer, RecoveryLedger, TestAccountSpec, TestClient, TestEnvironment,
};

fn binary() -> PathBuf {
    std::env::var_os("FOKS_SERVER_BIN")
        .map(PathBuf::from)
        .expect("set FOKS_SERVER_BIN to the freshly built foks-server binary")
}

fn budget() -> Duration {
    let milliseconds = std::env::var("FOKS_RECOVERY_BUDGET_MS")
        .map(|value| {
            value
                .parse::<u64>()
                .expect("invalid FOKS_RECOVERY_BUDGET_MS")
        })
        .unwrap_or(15_000);
    Duration::from_millis(milliseconds)
}

#[test]
#[ignore = "requires a freshly built FOKS_SERVER_BIN; run per RECOVERY.md"]
fn acknowledged_writes_survive_process_crashes_with_bounded_authenticated_recovery() {
    let binary = binary();
    let environment = TestEnvironment::new().unwrap();
    let mut server = BinaryServer::start(&environment, &binary).unwrap();
    let mut client = TestClient::new(&environment, "recovery-controller").unwrap();
    let host = client.probe_and_pin().unwrap().pinned;
    let account = client
        .create_account(&host, &TestAccountSpec::new("recoverycontroller", 0x91))
        .unwrap();
    let root = account.kv_projection[0].root_directory_id;
    // A caller-selected path survives fixture cleanup and can be collected by CI.
    let path = std::env::var_os("FOKS_RECOVERY_LEDGER")
        .map(PathBuf::from)
        .unwrap_or_else(|| environment.root().join("controller-observations.jsonl"));
    let mut ledger = RecoveryLedger::create(&path, budget()).unwrap();
    let mut expected = 0;
    for cycle in 0..3 {
        for index in 0..4 {
            let name = format!("before-{cycle}-{index}.txt");
            let content = format!("synthetic recovery payload {cycle}/{index}");
            write_file(
                &client,
                &host,
                &account.credential,
                root,
                &name,
                content.as_bytes(),
            );
            ledger.acknowledged(&name, content.as_bytes()).unwrap();
            expected += 1;
        }
        ledger.fault("kill-local-writer-process").unwrap();
        assert!(!server.force_shutdown().unwrap().status.success());
        server = BinaryServer::start(&environment, &binary).unwrap();
        // Reconnect using the same durable hard/soft/protected client stores.
        // Do not clear rollback anchors to make recovery pass.
        client = TestClient::new(&environment, "recovery-controller").unwrap();
        assert_eq!(client.pinned_host().unwrap().host_id(), host.host_id());
        verify_all(&client, &host, &account.credential, expected, &mut ledger);
        let name = format!("after-{cycle}.txt");
        write_file(
            &client,
            &host,
            &account.credential,
            root,
            &name,
            b"post-fault write",
        );
        ledger.acknowledged(&name, b"post-fault write").unwrap();
        expected += 1;
        verify_all(&client, &host, &account.credential, expected, &mut ledger);
        let elapsed = ledger.recovered().unwrap();
        eprintln!("local crash cycle {cycle}: authenticated read/write recovered in {elapsed:?}");
    }
    assert!(server.graceful_shutdown().unwrap().status.success());
}

#[test]
#[ignore = "requires a freshly built FOKS_SERVER_BIN; run per RECOVERY.md"]
fn restoring_stale_kv_backup_is_rejected_by_existing_client_anchors() {
    stale_kv_backup(true);
}

#[test]
#[ignore = "requires a freshly built FOKS_SERVER_BIN; run per RECOVERY.md"]
fn restoring_before_an_insertion_can_look_like_deletion_but_fails_controller_evidence() {
    stale_kv_backup(false);
}

fn stale_kv_backup(overwrite: bool) {
    let binary = binary();
    let source = TestEnvironment::new().unwrap();
    let server = BinaryServer::start(&source, &binary).unwrap();
    let addresses = server.addresses();
    let client = TestClient::new(&source, "rewind-kv").unwrap();
    let host = client.probe_and_pin().unwrap().pinned;
    let account = client
        .create_account(&host, &TestAccountSpec::new("rewindkv", 0x92))
        .unwrap();
    let root = account.kv_projection[0].root_directory_id;
    let mut ledger =
        RecoveryLedger::create(source.root().join("rewind-ledger.jsonl"), budget()).unwrap();
    write_file(
        &client,
        &host,
        &account.credential,
        root,
        "before.txt",
        b"before snapshot",
    );
    ledger
        .acknowledged("before.txt", b"before snapshot")
        .unwrap();
    let backup = server.backup_named("stale-kv").unwrap();
    write_file_with_options(
        &client,
        &host,
        &account.credential,
        root,
        if overwrite { "before.txt" } else { "after.txt" },
        b"after snapshot",
        KvWriteOptions {
            read_role: Role::OWNER,
            write_role: Role::OWNER,
            overwrite,
            expected_version: overwrite.then_some(1),
        },
    );
    if !overwrite {
        ledger.acknowledged("after.txt", b"after snapshot").unwrap();
    }
    let authentication = authenticate(&client, &host, &account.credential);
    let advanced = client
        .foks()
        .sync_user_kv(
            &host,
            &account.credential,
            &authentication.verified,
            &authentication.puks,
            client.soft_state_path(),
        )
        .unwrap();
    assert_eq!(advanced[0].entries.len(), if overwrite { 1 } else { 2 });
    if overwrite {
        assert_eq!(advanced[0].entries[0].version, 2);
    }
    ledger.fault("restore-stale-local-backup").unwrap();
    server.force_shutdown().unwrap();
    let restored =
        TestEnvironment::restore_backup_with_binary(&binary, &backup, addresses).unwrap();
    let server = BinaryServer::start(&restored, &binary).unwrap();
    let client = TestClient::new(&source, "rewind-kv").unwrap();
    let authentication = authenticate(&client, &host, &account.credential);
    let result = client.foks().sync_user_kv(
        &host,
        &account.credential,
        &authentication.verified,
        &authentication.puks,
        client.soft_state_path(),
    );
    if overwrite {
        let error = result.unwrap_err();
        assert!(
            matches!(
                error,
                foks_client::Error::Database(foks_client_db::Error::KvProjectionConflict(
                    "dirent rolled back or changed at the same version"
                ))
            ),
            "expected specific dirent rollback rejection, got {error:?}"
        );
    } else {
        // Directory encryption versions do not advance on every insertion.
        // A missing dirent can look like a legitimate deletion. Record this
        // existing protocol/client limitation rather than claim every rewind
        // is detected by the client's trust anchors.
        let tree = result.unwrap();
        assert_eq!(tree[0].entries.len(), 1);
        assert_eq!(tree[0].entries[0].name, b"before.txt");
        ledger
            .observed("before.txt", tree[0].entries[0].content.as_deref().unwrap())
            .unwrap();
        write_file(
            &client,
            &host,
            &account.credential,
            root,
            "postrestore.txt",
            b"new write",
        );
        ledger
            .acknowledged("postrestore.txt", b"new write")
            .unwrap();
        verify_all(&client, &host, &account.credential, 2, &mut ledger);
        assert_eq!(
            ledger.recovered().unwrap_err().to_string(),
            "recovery needs all acknowledged reads and a new write"
        );
    }
    server.graceful_shutdown().unwrap();
}

#[test]
#[ignore = "requires a freshly built FOKS_SERVER_BIN; run per RECOVERY.md"]
fn restoring_stale_identity_backup_is_rejected_by_existing_merkle_pin() {
    let binary = binary();
    let source = TestEnvironment::new().unwrap();
    let server = BinaryServer::start(&source, &binary).unwrap();
    let addresses = server.addresses();
    let client = TestClient::new(&source, "rewind-identity").unwrap();
    let host = client.probe_and_pin().unwrap().pinned;
    let account = client
        .create_account(&host, &TestAccountSpec::new("rewindidentity", 0x93))
        .unwrap();
    let backup = server.backup_named("stale-identity").unwrap();
    // An independent signup advances authoritative state, then our existing
    // client observes it. Reads alone create a rollback-protection obligation.
    let other = TestClient::new(&source, "other-identity").unwrap();
    let other_host = other.probe_and_pin().unwrap().pinned;
    other
        .create_account(&other_host, &TestAccountSpec::new("otheridentity", 0x94))
        .unwrap();
    client.foks().advance_merkle_root(&host).unwrap();
    authenticate(&client, &host, &account.credential);
    server.force_shutdown().unwrap();
    let restored =
        TestEnvironment::restore_backup_with_binary(&binary, &backup, addresses).unwrap();
    let server = BinaryServer::start(&restored, &binary).unwrap();
    let client = TestClient::new(&source, "rewind-identity").unwrap();
    let host = client.pinned_host().unwrap();
    let error = client.foks().advance_merkle_root(&host).unwrap_err();
    assert!(
        matches!(
            error,
            foks_client::Error::Verify(foks_verify::Error::MerkleRollback { .. })
                | foks_client::Error::Database(foks_client_db::Error::MerkleRollback { .. })
        ),
        "expected specific Merkle rollback rejection, got {error:?}"
    );
    server.graceful_shutdown().unwrap();
}

fn authenticate(
    client: &TestClient,
    host: &PinnedHost,
    credential: &DeviceCredential,
) -> AuthenticatedUserOutcome {
    client
        .foks()
        .authenticate_and_pin(host, credential)
        .unwrap()
}

fn write_file(
    client: &TestClient,
    host: &PinnedHost,
    credential: &DeviceCredential,
    root: [u8; 16],
    name: &str,
    content: &[u8],
) {
    write_file_with_options(
        client,
        host,
        credential,
        root,
        name,
        content,
        KvWriteOptions {
            read_role: Role::OWNER,
            write_role: Role::OWNER,
            overwrite: false,
            expected_version: None,
        },
    );
}

fn write_file_with_options(
    client: &TestClient,
    host: &PinnedHost,
    credential: &DeviceCredential,
    root: [u8; 16],
    name: &str,
    content: &[u8],
    options: KvWriteOptions,
) {
    let authenticated = authenticate(client, host, credential);
    let mut protected = client.open_protected_store().unwrap();
    let mut session = client
        .foks()
        .user_kv_write_session(
            host,
            credential,
            &authenticated.verified,
            &authenticated.puks,
            client.soft_state_path(),
            &mut protected,
        )
        .unwrap();
    session
        .put_file(root, name, &mut Cursor::new(content), options)
        .unwrap();
}

fn verify_all(
    client: &TestClient,
    host: &PinnedHost,
    credential: &DeviceCredential,
    expected: usize,
    ledger: &mut RecoveryLedger,
) {
    let authenticated = authenticate(client, host, credential);
    let tree = client
        .foks()
        .sync_user_kv(
            host,
            credential,
            &authenticated.verified,
            &authenticated.puks,
            client.soft_state_path(),
        )
        .unwrap();
    assert_eq!(tree.len(), 1);
    assert_eq!(tree[0].entries.len(), expected);
    for entry in &tree[0].entries {
        ledger
            .observed(
                std::str::from_utf8(&entry.name).unwrap(),
                entry.content.as_deref().unwrap(),
            )
            .unwrap();
    }
}
