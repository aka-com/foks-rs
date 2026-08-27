use std::io::Cursor;

use foks_client::KvWriteOptions;
use foks_client_db::Acceptance;
use foks_proto::Role;
use foks_server_testkit::{TestAccountSpec, TestClient, TestEnvironment};

#[test]
fn public_client_completes_the_release_smoke_scenario() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "release-smoke").unwrap();
    let probe = client.probe_and_pin().unwrap();
    assert_eq!(probe.acceptance, Acceptance::Inserted);
    let created = client
        .create_account(&probe.pinned, &TestAccountSpec::new("releasesmoke", 0x71))
        .unwrap();
    let root = created.kv_projection[0].root_directory_id;
    let options = KvWriteOptions {
        read_role: Role::OWNER,
        write_role: Role::OWNER,
        overwrite: false,
        expected_version: None,
    };
    let mut protected = client.open_protected_store().unwrap();
    let mut session = client
        .foks()
        .user_kv_write_session(
            &probe.pinned,
            &created.credential,
            &created.authenticated.verified,
            &created.authenticated.puks,
            client.soft_state_path(),
            &mut protected,
        )
        .unwrap();
    session
        .put_file(
            root,
            "small.txt",
            &mut Cursor::new(b"release smoke"),
            options,
        )
        .unwrap();
    let large = vec![0x5a; 4 * 1024 * 1024 + 1];
    session
        .put_file(root, "large.bin", &mut Cursor::new(&large), options)
        .unwrap();
    let projection = session.sync().unwrap();
    assert_eq!(projection[0].entries.len(), 2);
    drop(session);

    let authenticated = client
        .foks()
        .authenticate_and_pin(&probe.pinned, &created.credential)
        .unwrap();
    assert_eq!(authenticated.verified.username(), b"releasesmoke");
    let backup = server.backup_named("release-smoke").unwrap();
    assert!(server.backup_is_valid(&backup).unwrap());
    server.shutdown().unwrap();
}
