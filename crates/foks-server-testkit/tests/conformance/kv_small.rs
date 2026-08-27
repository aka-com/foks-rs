use std::io::Cursor;
use std::time::Duration;

use foks_client::KvWriteOptions;
use foks_proto::Role;
use foks_server_testkit::TestAccountSpec;

use crate::support::Fixture;

fn options() -> KvWriteOptions {
    KvWriteOptions {
        read_role: Role::OWNER,
        write_role: Role::OWNER,
        overwrite: false,
        expected_version: None,
    }
}

#[test]
pub(crate) fn kv_small_success() {
    let fixture = Fixture::start("kv-small-client");
    let created = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("kvsmalluser", 0x31))
        .unwrap();
    let root = created.kv_projection[0].root_directory_id;
    let options = options();
    let mut protected = fixture.client.open_protected_store().unwrap();
    let mut session = fixture
        .client
        .foks()
        .user_kv_write_session(
            fixture.host(),
            &created.credential,
            &created.authenticated.verified,
            &created.authenticated.puks,
            fixture.client.soft_state_path(),
            &mut protected,
        )
        .unwrap();
    session
        .put_file(
            root,
            "hello.txt",
            &mut Cursor::new(b"hello from sqlite"),
            options,
        )
        .unwrap();
    session
        .put_symlink(root, "hello-link", "hello.txt", options)
        .unwrap();
    let child = session
        .mkdir(root, "documents", options)
        .unwrap()
        .node_id
        .object_id();
    session
        .put_file(
            child,
            "nested.txt",
            &mut Cursor::new(b"nested content"),
            options,
        )
        .unwrap();
    let lock_target = [0x44; 16];
    let lock = session
        .acquire_lock(root, lock_target, Duration::from_secs(30))
        .unwrap();
    assert!(session
        .acquire_lock(root, lock_target, Duration::from_secs(30))
        .is_err());
    session.release_lock(root, lock_target, lock).unwrap();

    let tree = session.sync().unwrap();
    let root_projection = tree
        .iter()
        .find(|directory| directory.directory_id == root)
        .unwrap();
    assert_eq!(root_projection.entries.len(), 3);
    assert_eq!(
        root_projection
            .entries
            .iter()
            .find(|entry| entry.name == b"hello.txt")
            .unwrap()
            .content
            .as_deref(),
        Some(b"hello from sqlite".as_slice())
    );
    assert_eq!(
        root_projection
            .entries
            .iter()
            .find(|entry| entry.name == b"hello-link")
            .unwrap()
            .symlink
            .as_deref(),
        Some(b"hello.txt".as_slice())
    );
    let child_projection = tree
        .iter()
        .find(|directory| directory.directory_id == child)
        .unwrap();
    assert_eq!(
        child_projection.entries[0].content.as_deref(),
        Some(b"nested content".as_slice())
    );
}

#[test]
fn small_file_boundaries_duplicates_and_stale_versions_are_atomic() {
    let fixture = Fixture::start("kv-boundary-client");
    let created = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("kvboundary", 0x35))
        .unwrap();
    let root = created.kv_projection[0].root_directory_id;
    let mut protected = fixture.client.open_protected_store().unwrap();
    let mut session = fixture
        .client
        .foks()
        .user_kv_write_session(
            fixture.host(),
            &created.credential,
            &created.authenticated.verified,
            &created.authenticated.puks,
            fixture.client.soft_state_path(),
            &mut protected,
        )
        .unwrap();
    for (name, content) in [
        ("empty", Vec::new()),
        ("one", vec![0x01]),
        ("maximum-small", vec![0x5a; 2040]),
    ] {
        session
            .put_file(root, name, &mut Cursor::new(&content), options())
            .unwrap();
    }
    let first = session
        .put_file(root, "replace-me", &mut Cursor::new(b"first"), options())
        .unwrap();
    let duplicate = session
        .put_file(
            root,
            "replace-me",
            &mut Cursor::new(b"duplicate"),
            options(),
        )
        .unwrap_err();
    assert!(matches!(duplicate, foks_client::Error::KvResponse(_)));

    let replaced = session
        .put_file(
            root,
            "replace-me",
            &mut Cursor::new(b"second"),
            KvWriteOptions {
                overwrite: true,
                expected_version: Some(first.dirent_version),
                ..options()
            },
        )
        .unwrap();
    let stale = session
        .put_file(
            root,
            "replace-me",
            &mut Cursor::new(b"stale"),
            KvWriteOptions {
                overwrite: true,
                expected_version: Some(first.dirent_version),
                ..options()
            },
        )
        .unwrap_err();
    assert!(matches!(stale, foks_client::Error::KvResponse(_)));
    assert!(replaced.dirent_version > first.dirent_version);

    for invalid in ["", ".", "..", "a/b", "nul\0name"] {
        assert!(session
            .put_file(root, invalid, &mut Cursor::new(b"invalid"), options())
            .is_err());
    }
    let excessive = "x".repeat(256);
    assert!(session
        .put_file(root, &excessive, &mut Cursor::new(b"invalid"), options(),)
        .is_err());

    let tree = session.sync().unwrap();
    assert_eq!(tree[0].entries.len(), 4);
    let replaced = tree[0]
        .entries
        .iter()
        .find(|entry| entry.name == b"replace-me")
        .unwrap();
    assert_eq!(replaced.content.as_deref(), Some(b"second".as_slice()));
    assert_eq!(
        tree[0]
            .entries
            .iter()
            .find(|entry| entry.name == b"maximum-small")
            .unwrap()
            .content
            .as_deref(),
        Some(vec![0x5a; 2040].as_slice())
    );
}

#[test]
fn move_unlink_and_cross_user_parent_checks_preserve_reachable_state() {
    let fixture = Fixture::start("kv-move-client");
    let created = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("kvmove", 0x39))
        .unwrap();
    let root = created.kv_projection[0].root_directory_id;
    let mut protected = fixture.client.open_protected_store().unwrap();
    let mut session = fixture
        .client
        .foks()
        .user_kv_write_session(
            fixture.host(),
            &created.credential,
            &created.authenticated.verified,
            &created.authenticated.puks,
            fixture.client.soft_state_path(),
            &mut protected,
        )
        .unwrap();
    session
        .put_file(
            root,
            "source.txt",
            &mut Cursor::new(b"move content"),
            options(),
        )
        .unwrap();
    let child = session
        .mkdir(root, "child", options())
        .unwrap()
        .node_id
        .object_id();
    let moved = session
        .move_entry(root, "source.txt", child, "moved.txt", options())
        .unwrap();
    assert!(session
        .unlink(root, "child", None, Role::OWNER, false)
        .is_err());
    let tree = session
        .unlink(
            child,
            "moved.txt",
            Some(moved.dirent_version),
            Role::OWNER,
            false,
        )
        .unwrap();
    assert!(tree
        .iter()
        .find(|directory| directory.directory_id == child)
        .unwrap()
        .entries
        .is_empty());
    let tree = session
        .unlink(root, "child", None, Role::OWNER, true)
        .unwrap();
    assert!(tree[0].entries.is_empty());
    drop(session);
    drop(protected);

    let other_client =
        foks_server_testkit::TestClient::new(&fixture.environment, "kv-other").unwrap();
    let other_probe = other_client.probe_and_pin().unwrap();
    let other = other_client
        .create_account(&other_probe.pinned, &TestAccountSpec::new("kvother", 0x49))
        .unwrap();
    let mut other_protected = other_client.open_protected_store().unwrap();
    let mut other_session = other_client
        .foks()
        .user_kv_write_session(
            &other_probe.pinned,
            &other.credential,
            &other.authenticated.verified,
            &other.authenticated.puks,
            other_client.soft_state_path(),
            &mut other_protected,
        )
        .unwrap();
    assert!(other_session
        .put_file(
            root,
            "cross-user.txt",
            &mut Cursor::new(b"denied"),
            options(),
        )
        .is_err());
    assert!(other_session.sync().unwrap()[0].entries.is_empty());
}

#[test]
fn directory_listing_crosses_the_v019_page_boundary() {
    let fixture = Fixture::start("kv-pagination-client");
    let created = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("kvpages", 0x3d))
        .unwrap();
    let root = created.kv_projection[0].root_directory_id;
    let mut protected = fixture.client.open_protected_store().unwrap();
    let mut session = fixture
        .client
        .foks()
        .user_kv_write_session(
            fixture.host(),
            &created.credential,
            &created.authenticated.verified,
            &created.authenticated.puks,
            fixture.client.soft_state_path(),
            &mut protected,
        )
        .unwrap();
    for index in 0..101 {
        session
            .put_file(
                root,
                &format!("entry-{index:03}"),
                &mut Cursor::new([]),
                options(),
            )
            .unwrap();
    }
    let tree = session.sync().unwrap();
    assert_eq!(tree[0].entries.len(), 101);
    let names = tree[0]
        .entries
        .iter()
        .map(|entry| entry.name.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(names.len(), 101);
    assert!(names.contains(b"entry-000".as_slice()));
    assert!(names.contains(b"entry-100".as_slice()));
}
