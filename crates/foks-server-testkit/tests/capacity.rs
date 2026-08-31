use std::io::Cursor;

use foks_client::KvWriteOptions;
use foks_proto::Role;
use foks_server_testkit::{TestAccountSpec, TestClient, TestEnvironment, TestProfile};

fn options() -> KvWriteOptions {
    KvWriteOptions {
        read_role: Role::OWNER,
        write_role: Role::OWNER,
        overwrite: false,
        expected_version: None,
    }
}

#[test]
fn small_object_capacity_rejects_before_commit_without_partial_writes() {
    let environment = TestEnvironment::with_profile(TestProfile::SmallCapacity).unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "object-capacity-client").unwrap();
    let probe = client.probe_and_pin().unwrap();
    let created = client
        .create_account(&probe.pinned, &TestAccountSpec::new("objectcapacity", 0xa1))
        .unwrap();
    let root = created.kv_projection[0].root_directory_id;
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
    let mut committed = 0;
    let quota_error = loop {
        let name = format!("object-{committed:03}.txt");
        match session.put_file(root, &name, &mut Cursor::new([0x41]), options()) {
            Ok(_) => committed += 1,
            Err(error) => break error,
        }
        assert!(
            committed < 128,
            "small profile failed to enforce object capacity"
        );
    };
    assert!(
        matches!(
            &quota_error,
            foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus { code: 1060, .. })
        ),
        "unexpected capacity error after {committed} objects: {quota_error:?}"
    );
    let tree = session.sync().unwrap();
    assert_eq!(tree[0].entries.len(), committed);
    assert!(!tree[0]
        .entries
        .iter()
        .any(|entry| entry.name == format!("object-{committed:03}.txt").as_bytes()));
    server.shutdown().unwrap();
}

#[test]
fn oversized_upload_hits_namespace_quota_and_remains_invisible() {
    let environment = TestEnvironment::with_profile(TestProfile::SmallCapacity).unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "byte-capacity-client").unwrap();
    let probe = client.probe_and_pin().unwrap();
    let created = client
        .create_account(&probe.pinned, &TestAccountSpec::new("bytecapacity", 0xa2))
        .unwrap();
    let root = created.kv_projection[0].root_directory_id;
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
    let error = session
        .put_file(
            root,
            "too-large.bin",
            &mut Cursor::new(vec![0x52; 4 * 1024 * 1024 + 1]),
            options(),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus { code: 1060, .. })
    ));
    assert!(session.sync().unwrap()[0].entries.is_empty());
    server.shutdown().unwrap();
}
