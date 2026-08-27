use std::io::Cursor;

use foks_client::KvWriteOptions;
use foks_client_db::Acceptance;
use foks_proto::Role;
use foks_server_testkit::{TestAccountSpec, TestClient, TestEnvironment};

#[test]
fn server_and_client_reconstruct_the_same_pinned_host() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let addresses = server.addresses();
    let first_client = TestClient::new(&environment, "restart-client").unwrap();
    let first = first_client.probe_and_pin().unwrap();
    assert_eq!(first.acceptance, Acceptance::Inserted);
    let host_id = first.pinned.host_id().clone();
    let genesis_host_id = first.verified.snapshot.host_id().to_vec();
    let created = first_client
        .create_account(&first.pinned, &TestAccountSpec::new("restartuser", 0x51))
        .unwrap();
    let root = created.kv_projection[0].root_directory_id;
    let options = KvWriteOptions {
        read_role: Role::OWNER,
        write_role: Role::OWNER,
        overwrite: false,
        expected_version: None,
    };
    let mut protected = first_client.open_protected_store().unwrap();
    let mut session = first_client
        .foks()
        .user_kv_write_session(
            &first.pinned,
            &created.credential,
            &created.authenticated.verified,
            &created.authenticated.puks,
            first_client.soft_state_path(),
            &mut protected,
        )
        .unwrap();
    session
        .put_file(
            root,
            "durable.txt",
            &mut Cursor::new(b"survives both restarts"),
            options,
        )
        .unwrap();
    drop(session);
    drop(protected);
    let before_restart = first_client.probe_and_pin().unwrap();
    assert_eq!(before_restart.acceptance, Acceptance::Unchanged);
    assert_eq!(before_restart.verified.merkle_root.epoch, 2);
    server.shutdown().unwrap();

    environment.advance_clock(1_000_000);
    let restarted = environment.start_server().unwrap();
    assert_eq!(restarted.addresses(), addresses);
    let reconstructed = TestClient::new(&environment, "restart-client").unwrap();
    let second = reconstructed.probe_and_pin().unwrap();
    assert_eq!(second.acceptance, Acceptance::Unchanged);
    assert_eq!(second.pinned.host_id(), &host_id);
    assert_eq!(second.verified, before_restart.verified);
    assert_eq!(second.verified.snapshot.host_id(), genesis_host_id);
    let authenticated = reconstructed
        .foks()
        .authenticate_and_pin(&second.pinned, &created.credential)
        .unwrap();
    assert_eq!(authenticated.verified.username(), b"restartuser");
    assert_eq!(authenticated.puks[0].seed.as_slice(), &[0x52; 32]);
    let tree = reconstructed
        .foks()
        .sync_user_kv(
            &second.pinned,
            &created.credential,
            &authenticated.verified,
            &authenticated.puks,
            reconstructed.soft_state_path(),
        )
        .unwrap();
    assert_eq!(tree.len(), 1);
    assert_eq!(tree[0].root_directory_id, root);
    assert_eq!(tree[0].entries.len(), 1);
    assert_eq!(tree[0].entries[0].name, b"durable.txt");
    assert_eq!(
        tree[0].entries[0].content.as_deref(),
        Some(b"survives both restarts".as_slice())
    );
    let mut reopened_protected = reconstructed.open_protected_store().unwrap();
    let mut resumed_session = reconstructed
        .foks()
        .user_kv_write_session(
            &second.pinned,
            &created.credential,
            &authenticated.verified,
            &authenticated.puks,
            reconstructed.soft_state_path(),
            &mut reopened_protected,
        )
        .unwrap();
    resumed_session
        .put_file(
            root,
            "after-restart.txt",
            &mut Cursor::new(b"new mutation after restart"),
            options,
        )
        .unwrap();
    assert_eq!(resumed_session.sync().unwrap()[0].entries.len(), 2);
    drop(resumed_session);
    restarted.shutdown().unwrap();
}

#[test]
fn one_test_environment_rejects_two_authoritative_servers() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    assert!(environment.start_server().is_err());
    server.shutdown().unwrap();
    environment.start_server().unwrap().shutdown().unwrap();
}
