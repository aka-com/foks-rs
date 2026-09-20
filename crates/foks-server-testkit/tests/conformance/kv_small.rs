use std::io::Cursor;
use std::time::Duration;

use foks_client::{KvFetchedNode, KvWriteOptions};
use foks_proto::{KvNodeId, Role};
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
    let hello = root_projection
        .entries
        .iter()
        .find(|entry| entry.name == b"hello.txt")
        .unwrap();
    let hello_node = KvNodeId(hello.node_id);
    assert!(hello.content.is_none());
    let link = root_projection
        .entries
        .iter()
        .find(|entry| entry.name == b"hello-link")
        .unwrap();
    let link_node = KvNodeId(link.node_id);
    assert!(link.symlink.is_none());
    let child_projection = tree
        .iter()
        .find(|directory| directory.directory_id == child)
        .unwrap();
    let nested = &child_projection.entries[0];
    let nested_node = KvNodeId(nested.node_id);
    assert!(nested.content.is_none());
    drop(session);

    assert_eq!(
        fixture
            .client
            .foks()
            .read_user_kv_node(
                fixture.host(),
                &created.credential,
                &created.authenticated.verified,
                &created.authenticated.puks,
                hello_node,
            )
            .unwrap(),
        KvFetchedNode::SmallFile(b"hello from sqlite".to_vec())
    );
    assert_eq!(
        fixture
            .client
            .foks()
            .read_user_kv_node(
                fixture.host(),
                &created.credential,
                &created.authenticated.verified,
                &created.authenticated.puks,
                link_node,
            )
            .unwrap(),
        KvFetchedNode::Symlink(b"hello.txt".to_vec())
    );
    assert_eq!(
        fixture
            .client
            .foks()
            .read_user_kv_node(
                fixture.host(),
                &created.credential,
                &created.authenticated.verified,
                &created.authenticated.puks,
                nested_node,
            )
            .unwrap(),
        KvFetchedNode::SmallFile(b"nested content".to_vec())
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
    let replaced_node = KvNodeId(replaced.node_id);
    assert!(replaced.content.is_none());
    let maximum = tree[0]
        .entries
        .iter()
        .find(|entry| entry.name == b"maximum-small")
        .unwrap();
    let maximum_node = KvNodeId(maximum.node_id);
    assert!(maximum.content.is_none());
    drop(session);

    assert_eq!(
        fixture
            .client
            .foks()
            .read_user_kv_node(
                fixture.host(),
                &created.credential,
                &created.authenticated.verified,
                &created.authenticated.puks,
                replaced_node,
            )
            .unwrap(),
        KvFetchedNode::SmallFile(b"second".to_vec())
    );
    assert_eq!(
        fixture
            .client
            .foks()
            .read_user_kv_node(
                fixture.host(),
                &created.credential,
                &created.authenticated.verified,
                &created.authenticated.puks,
                maximum_node,
            )
            .unwrap(),
        KvFetchedNode::SmallFile(vec![0x5a; 2040])
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
    // One normal write supplies a valid node. The pagination fixture needs
    // 101 independently sealed names, not 101 upload/journal/sync workflows.
    session
        .put_file(root, "entry-000", &mut Cursor::new([]), options())
        .unwrap();
    let tree = session.sync().unwrap();
    let directory = foks_proto::KvDirectoryPair::decode(&tree[0].directory_bytes).unwrap();
    let key = created.authenticated.current_puk().unwrap();
    let seed = foks_crypto::derive_kv_keys(&key.seed)
        .unwrap()
        .open_directory_seed(&directory.active)
        .unwrap();
    let template = foks_proto::KvDirent::decode(&tree[0].entries[0].dirent_bytes).unwrap();
    let dirents = (1..101)
        .map(|index| {
            let mut entry = template.clone();
            entry.id = [index; 16];
            let (mac, boxed) = foks_crypto::seal_kv_dirent_name(
                &seed,
                root,
                entry.directory_version,
                format!("entry-{index:03}").into_bytes(),
                [index; 16],
            )
            .unwrap();
            entry.name_mac = mac;
            entry.name_box = boxed;
            entry.binding_mac = foks_crypto::bind_kv_dirent(&seed, &entry).unwrap();
            entry
        })
        .collect();
    fixture
        .environment
        .seed_kv_dirents(created.credential.uid.clone(), dirents)
        .unwrap();
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

/// The server accepts a precondition that names only the directories one path
/// walked. A peer's change to a directory outside that path is therefore not
/// asserted by the write and does not reject it, while the write still sees
/// its own parent at the version it read.
#[test]
pub(crate) fn a_path_scoped_write_cites_only_the_directories_its_path_walked() {
    let fixture = Fixture::start("kv-path-scope");
    let created = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("kvpathuser", 0x3a))
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
    let alpha = session
        .mkdir(root, "alpha", options())
        .unwrap()
        .node_id
        .object_id();
    let beta = session
        .mkdir(alpha, "beta", options())
        .unwrap()
        .node_id
        .object_id();
    let bystander = session
        .mkdir(root, "bystander", options())
        .unwrap()
        .node_id
        .object_id();
    session
        .put_file(
            bystander,
            "moving.txt",
            &mut Cursor::new(b"first"),
            options(),
        )
        .unwrap();

    let resolved = session
        .resolve_path(&[b"alpha".to_vec(), b"beta".to_vec()])
        .unwrap();
    assert_eq!(resolved.len(), 3, "only the path's directories are read");
    assert_eq!(resolved[0].directory_id, root);
    assert_eq!(resolved[1].directory_id, alpha);
    assert_eq!(resolved[2].directory_id, beta);

    // A second device changes a dirent the complete traversal would have
    // cited, in a directory this path never walked.
    let peer =
        foks_server_testkit::TestClient::new(&fixture.environment, "kv-path-scope-peer").unwrap();
    let peer_probe = peer.probe_and_pin().unwrap();
    let peer_authenticated = peer
        .foks()
        .authenticate_and_pin(&peer_probe.pinned, &created.credential)
        .unwrap();
    let mut peer_protected = peer.open_protected_store().unwrap();
    let mut peer_session = peer
        .foks()
        .user_kv_write_session(
            &peer_probe.pinned,
            &created.credential,
            &peer_authenticated.verified,
            &peer_authenticated.puks,
            peer.soft_state_path(),
            &mut peer_protected,
        )
        .unwrap();
    peer_session
        .put_file(
            bystander,
            "moving.txt",
            &mut Cursor::new(b"second"),
            KvWriteOptions {
                overwrite: true,
                expected_version: Some(1),
                ..options()
            },
        )
        .unwrap();

    let written = session
        .put_file(beta, "scoped.txt", &mut Cursor::new(b"scoped"), options())
        .unwrap();
    assert_eq!(written.dirent_version, 1);
    assert_eq!(
        written.path.len(),
        3,
        "the mutation projected its own path and not the namespace"
    );
    let entry = written
        .path
        .last()
        .unwrap()
        .entries
        .iter()
        .find(|entry| entry.name == b"scoped.txt")
        .expect("the written entry is projected in its parent");
    assert_eq!(entry.node_id, written.node_id.0);

    // The write is durable and the bystander kept the peer's value.
    let tree = session.sync().unwrap();
    let names = |directory: [u8; 16]| {
        tree.iter()
            .find(|projection| projection.directory_id == directory)
            .unwrap()
            .entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect::<std::collections::BTreeSet<_>>()
    };
    assert!(names(beta).contains(b"scoped.txt".as_slice()));
    assert!(names(bystander).contains(b"moving.txt".as_slice()));
    let moved = tree
        .iter()
        .find(|projection| projection.directory_id == bystander)
        .unwrap()
        .entries
        .iter()
        .find(|entry| entry.name == b"moving.txt")
        .unwrap();
    assert_eq!(moved.version, 2, "the peer's change was applied");
}

/// Counts the KV requests a write costs in a branching store, with and
/// without a resolved path. An unscoped session keeps the complete traversal,
/// so one run measures both shapes against the same server.
#[test]
pub(crate) fn a_resolved_path_costs_a_fraction_of_a_complete_traversal() {
    const WIDTH: usize = 5;
    let fixture = Fixture::start("kv-path-cost");
    let created = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("kvcostuser", 0x3b))
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
    // A small live tree covers the wire behavior; the synthetic path tests
    // retain the fifty-directory fixture and exact request counts.
    // Scope setup so it does not repeatedly traverse unrelated directories.
    for index in 0..WIDTH {
        session.resolve_path(&[]).unwrap();
        session
            .mkdir(root, &format!("dir-{index:03}"), options())
            .unwrap();
    }
    session.resolve_path(&[]).unwrap();
    let alpha = session
        .mkdir(root, "alpha", options())
        .unwrap()
        .node_id
        .object_id();
    session.resolve_path(&[b"alpha".to_vec()]).unwrap();
    let beta = session
        .mkdir(alpha, "beta", options())
        .unwrap()
        .node_id
        .object_id();

    // A session with no resolved path keeps the complete traversal, so this
    // measures the shape every write had before paths were resolved.
    session.sync().unwrap();
    let before_unscoped = fixture.server.metrics().requests_started;
    session
        .put_file(
            beta,
            "unscoped.txt",
            &mut Cursor::new(b"unscoped"),
            options(),
        )
        .unwrap();
    let unscoped = fixture.server.metrics().requests_started - before_unscoped;

    let before_scoped = fixture.server.metrics().requests_started;
    session
        .resolve_path(&[b"alpha".to_vec(), b"beta".to_vec()])
        .unwrap();
    session
        .put_file(beta, "scoped.txt", &mut Cursor::new(b"scoped"), options())
        .unwrap();
    let scoped = fixture.server.metrics().requests_started - before_scoped;

    // The sequence a mkdir-p write follows: resolve the prefix that names
    // each missing directory, create it, then resolve the finished path.
    let before_created = fixture.server.metrics().requests_started;
    session.resolve_path(&[]).unwrap();
    let gamma = session
        .mkdir(root, "gamma", options())
        .unwrap()
        .node_id
        .object_id();
    session.resolve_path(&[b"gamma".to_vec()]).unwrap();
    let delta = session
        .mkdir(gamma, "delta", options())
        .unwrap()
        .node_id
        .object_id();
    session
        .resolve_path(&[b"gamma".to_vec(), b"delta".to_vec()])
        .unwrap();
    session
        .put_file(
            delta,
            "created.txt",
            &mut Cursor::new(b"created"),
            options(),
        )
        .unwrap();
    let created = fixture.server.metrics().requests_started - before_created;

    println!(
        "KV requests: scoped-write={scoped} unscoped-write={unscoped} \
         two-parents-created={created}"
    );
    assert_eq!(
        unscoped - scoped,
        4 * WIDTH as u64,
        "each unrelated directory adds metadata and listing reads before and after the write"
    );
    assert!(
        created < unscoped * 2,
        "creating two parents and writing should stay near one complete traversal: \
         created={created} unscoped={unscoped}"
    );
}
