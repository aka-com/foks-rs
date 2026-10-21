//! Removed names, overwrites and stale caches across independent devices.
//!
//! go-foks main (35dc037, 438f351) relies on the server returning a removed
//! name's tombstone dirent from `kvGet`, and adds integration scenarios for
//! writes over removed names and for reads through a stale cache. These tests
//! pin the Rust server's answer and run the same scenarios with the Rust
//! client. Each device is a client with its own soft-state cache; operations
//! open a fresh write session so that each one starts from that cache.

use std::io::{Cursor, Write as _};

use foks_client::{CreatedSoftwareAccount, KvFetchedNode, KvWriteOptions, KvWriteSession};
use foks_client_db::{KvDirectoryProjection, KvProjectedEntry};
use foks_proto::{KvGetResponse, KvNodeId, KvNodeType, Role};
use foks_rpc::KvAuth;
use foks_server_testkit::{TestAccountSpec, TestClient};

use crate::go_client_kv::authenticated_stream;
use crate::support::Fixture;

const OVERWRITE: KvWriteOptions = KvWriteOptions {
    read_role: Role::OWNER,
    write_role: Role::OWNER,
    overwrite: true,
    expected_version: None,
};

struct Device<'a> {
    fixture: &'a Fixture,
    account: &'a CreatedSoftwareAccount,
    client: TestClient,
}

impl<'a> Device<'a> {
    fn new(
        fixture: &'a Fixture,
        account: &'a CreatedSoftwareAccount,
        durable_id: &str,
        name: &str,
    ) -> Self {
        let client =
            TestClient::new_with_fresh_soft_state(&fixture.environment, durable_id, name).unwrap();
        Self {
            fixture,
            account,
            client,
        }
    }

    fn session<T>(&self, operation: impl FnOnce(&mut KvWriteSession<'_>) -> T) -> T {
        let mut protected = self.client.open_protected_store().unwrap();
        let mut session = self
            .client
            .foks()
            .user_kv_write_session(
                self.fixture.host(),
                &self.account.credential,
                &self.account.authenticated.verified,
                &self.account.authenticated.puks,
                self.client.soft_state_path(),
                &mut protected,
            )
            .unwrap();
        operation(&mut session)
    }

    /// Writes `data` at `path`, creating missing directories and replacing an
    /// existing file, like `foks kv put --mkdir-p --force`.
    fn echo(&self, data: &str, path: &str) {
        let (directories, name) = split(path);
        self.session(|session| {
            let parent = ensure_directories(session, &directories);
            session
                .put_file(parent, name, &mut Cursor::new(data.as_bytes()), OVERWRITE)
                .unwrap();
        });
    }

    fn unlink(&self, path: &str) {
        let (directories, name) = split(path);
        self.session(|session| {
            let parent = existing_directory(session, &directories).unwrap();
            session
                .unlink(parent, name, None, Role::OWNER, true)
                .unwrap();
        });
    }

    fn ln(&self, path: &str, target: &str) {
        let (directories, name) = split(path);
        self.session(|session| {
            let parent = ensure_directories(session, &directories);
            session
                .put_symlink(parent, name, target, OVERWRITE)
                .unwrap();
        });
    }

    fn mv(&self, source: &str, destination: &str) {
        let (source_directories, source_name) = split(source);
        let (destination_directories, destination_name) = split(destination);
        self.session(|session| {
            let source_parent = existing_directory(session, &source_directories).unwrap();
            let destination_parent = existing_directory(session, &destination_directories).unwrap();
            session
                .move_entry(
                    source_parent,
                    source_name,
                    destination_parent,
                    destination_name,
                    OVERWRITE,
                )
                .unwrap();
        });
    }

    /// Reads the small file at `path`, following a symlink in any directory
    /// component. `None` means the name does not resolve to a live entry.
    fn cat(&self, path: &str) -> Option<String> {
        let components = self.session(|session| follow_symlinks(session, path))?;
        let (directories, name) = components.split_at(components.len() - 1);
        let node = self.session(|session| {
            let tree = resolve_complete(session, directories)?;
            live_entry(tree.last().unwrap(), &name[0]).map(|entry| KvNodeId(entry.node_id))
        })?;
        let fetched = self
            .client
            .foks()
            .read_user_kv_node(
                self.fixture.host(),
                &self.account.credential,
                &self.account.authenticated.verified,
                &self.account.authenticated.puks,
                node,
            )
            .unwrap();
        let KvFetchedNode::SmallFile(bytes) = fetched else {
            panic!("{path} is not a small file: {fetched:?}");
        };
        Some(String::from_utf8(bytes).unwrap())
    }
}

fn split(path: &str) -> (Vec<Vec<u8>>, &str) {
    let mut components = path
        .trim_start_matches('/')
        .split('/')
        .map(|component| component.as_bytes().to_vec())
        .collect::<Vec<_>>();
    let name = path.rsplit('/').next().unwrap();
    components.pop();
    (components, name)
}

fn live_entry<'p>(
    directory: &'p KvDirectoryProjection,
    name: &[u8],
) -> Option<&'p KvProjectedEntry> {
    directory
        .entries
        .iter()
        .find(|entry| entry.name == name && entry.node_id[0] != KvNodeType::None as u8)
}

/// The resolved path, or `None` when a component is missing or removed.
fn resolve_complete(
    session: &mut KvWriteSession<'_>,
    directories: &[Vec<u8>],
) -> Option<Vec<KvDirectoryProjection>> {
    let tree = session.resolve_path(directories).unwrap();
    (tree.len() == directories.len() + 1).then_some(tree)
}

fn existing_directory(
    session: &mut KvWriteSession<'_>,
    directories: &[Vec<u8>],
) -> Option<[u8; 16]> {
    resolve_complete(session, directories).map(|tree| tree.last().unwrap().directory_id)
}

fn ensure_directories(session: &mut KvWriteSession<'_>, directories: &[Vec<u8>]) -> [u8; 16] {
    loop {
        let tree = session.resolve_path(directories).unwrap();
        let resolved = tree.len() - 1;
        let parent = tree.last().unwrap().directory_id;
        if resolved == directories.len() {
            return parent;
        }
        let name = std::str::from_utf8(&directories[resolved]).unwrap();
        session.mkdir(parent, name, OVERWRITE).unwrap();
    }
}

/// Replaces each directory component that names a symlink with the symlink's
/// absolute target, as the Go client's path walk does.
fn follow_symlinks(session: &mut KvWriteSession<'_>, path: &str) -> Option<Vec<Vec<u8>>> {
    let (directories, name) = split(path);
    let mut resolved: Vec<Vec<u8>> = Vec::new();
    for component in directories {
        let tree = resolve_complete(session, &resolved)?;
        let entry = live_entry(tree.last().unwrap(), &component)?;
        if entry.node_id[0] == KvNodeType::Symlink as u8 {
            let target = session
                .projected_symlink_target(
                    KvNodeId(entry.node_id),
                    entry.node_bytes.as_deref().unwrap(),
                )
                .unwrap();
            let target = std::str::from_utf8(&target).unwrap().to_owned();
            resolved = target
                .trim_start_matches('/')
                .split('/')
                .map(|component| component.as_bytes().to_vec())
                .collect();
        } else {
            resolved.push(component);
        }
    }
    resolved.push(name.as_bytes().to_vec());
    Some(resolved)
}

fn account(fixture: &Fixture, name: &str, seed: u8) -> CreatedSoftwareAccount {
    fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new(name, seed))
        .unwrap()
}

/// The Go-main client edits a removed name through the tombstone `kvGet`
/// returns; a `KV_NOENT_ERROR` would make it create a second dirent for the
/// name.
#[test]
pub(crate) fn kv_get_returns_the_tombstone_dirent_of_a_removed_name() {
    let fixture = Fixture::start("kv-tombstone-get");
    let created = account(&fixture, "kvtombstoneget", 0x71);
    let root = created.kv_projection[0].root_directory_id;
    let device = Device::new(&fixture, &created, "kv-tombstone-get", "kv-tombstone-get-x");
    device.echo("v1", "/f");
    let live = fixture
        .server
        .read_database()
        .unwrap()
        .kv_list(created.credential.uid.as_bytes(), &root, None, 10)
        .unwrap();
    let [live] = live.as_slice() else {
        panic!("expected one dirent, found {live:?}");
    };
    let live = foks_proto::KvDirent::decode(&live.exact).unwrap();
    device.unlink("/f");

    let mut tls = authenticated_stream(&fixture, &created.credential);
    tls.write_all(&foks_rpc::encode_kv_select_vhost_request(fixture.host().host_id()).unwrap())
        .unwrap();
    foks_rpc::read_void_response(&mut tls, foks_rpc::DEFAULT_MAX_FRAME_LENGTH, 0).unwrap();
    for (sequence, follow) in [(1, 0), (2, 1), (3, 2)] {
        tls.write_all(
            &foks_rpc::encode_kv_get_request_at(
                KvAuth::User,
                None,
                &root,
                &[(live.directory_version, live.name_mac)],
                follow,
                sequence,
            )
            .unwrap(),
        )
        .unwrap();
        let response =
            foks_rpc::read_response(&mut tls, foks_rpc::DEFAULT_MAX_FRAME_LENGTH, sequence)
                .unwrap();
        let response = KvGetResponse::decode(&response).unwrap();
        assert_eq!(response.dirent.id, live.id);
        assert_eq!(response.dirent.parent, root);
        assert_eq!(response.dirent.name_mac, live.name_mac);
        assert!(response.dirent.version > live.version);
        assert_eq!(response.dirent.value.node_type().unwrap(), KvNodeType::None);
        assert!(response.node.is_none());
    }

    // A name that never existed is still not found.
    tls.write_all(
        &foks_rpc::encode_kv_get_request_at(
            KvAuth::User,
            None,
            &root,
            &[(live.directory_version, [0x5a; 32])],
            0,
            4,
        )
        .unwrap(),
    )
    .unwrap();
    let error =
        foks_rpc::read_response(&mut tls, foks_rpc::DEFAULT_MAX_FRAME_LENGTH, 4).unwrap_err();
    assert!(
        matches!(
            error,
            foks_rpc::Error::RemoteStatus {
                code: foks_rpc::STATUS_KV_NOENT_ERROR,
                ..
            }
        ),
        "{error:?}"
    );
}

/// go-foks `TestWriteOverTombstone`: a removed name takes a write from any
/// device, not only from one whose cache holds the tombstone. x and y write;
/// z only reads.
#[test]
pub(crate) fn a_removed_name_takes_a_write_from_any_device() {
    let fixture = Fixture::start("kv-tombstone-write");
    let created = account(&fixture, "kvtombstonewrite", 0x72);
    let device = |name| Device::new(&fixture, &created, "kv-tombstone-write", name);
    let (x, y, z) = (device("kv-dev-x"), device("kv-dev-y"), device("kv-dev-z"));

    // Put from the removing device.
    x.echo("v1", "/s1/a/f");
    x.unlink("/s1/a/f");
    x.echo("v2", "/s1/a/f");
    assert_eq!(z.cat("/s1/a/f").as_deref(), Some("v2"));

    // Put from a device that never saw the name.
    x.echo("v1", "/s2/a/f");
    x.unlink("/s2/a/f");
    y.echo("v2", "/s2/a/f");
    assert_eq!(z.cat("/s2/a/f").as_deref(), Some("v2"));

    // Put from a device holding the live dirent.
    x.echo("v1", "/s3/a/f");
    assert_eq!(y.cat("/s3/a/f").as_deref(), Some("v1"));
    x.unlink("/s3/a/f");
    y.echo("v2", "/s3/a/f");
    assert_eq!(z.cat("/s3/a/f").as_deref(), Some("v2"));

    // mkdir -p through a removed directory.
    x.echo("v1", "/s4/a/b/f");
    x.unlink("/s4/a/b");
    y.echo("v2", "/s4/a/b/g");
    assert_eq!(z.cat("/s4/a/b/g").as_deref(), Some("v2"));
    assert_eq!(z.cat("/s4/a/b/f"), None);

    // Move a file onto a removed name.
    x.echo("v1", "/s5/a/f");
    x.unlink("/s5/a/f");
    x.echo("v2", "/s5/a/g");
    y.mv("/s5/a/g", "/s5/a/f");
    assert_eq!(z.cat("/s5/a/f").as_deref(), Some("v2"));
    assert_eq!(z.cat("/s5/a/g"), None);

    // Move a directory onto a removed name.
    x.echo("v1", "/s6/a/f");
    x.unlink("/s6/a/f");
    x.echo("v2", "/s6/a/d/h");
    y.mv("/s6/a/d", "/s6/a/f");
    assert_eq!(z.cat("/s6/a/f/h").as_deref(), Some("v2"));

    // A read of a removed name finds nothing.
    x.echo("v1", "/s7/a/f");
    x.unlink("/s7/a/f");
    assert_eq!(z.cat("/s7/a/f"), None);
}

/// go-foks `TestKVStaleCache*`: a device that cached a file, or a removal,
/// reads another device's later write rather than its cache.
#[test]
pub(crate) fn reads_through_a_stale_cache_see_another_devices_writes() {
    let fixture = Fixture::start("kv-stale-cache");
    let created = account(&fixture, "kvstalecache", 0x73);
    let device = |name| Device::new(&fixture, &created, "kv-stale-cache", name);
    let (a, b) = (device("kv-dev-a"), device("kv-dev-b"));

    // Overwrite by the other device.
    a.echo("v1", "/o/docs/note.txt");
    assert_eq!(a.cat("/o/docs/note.txt").as_deref(), Some("v1"));
    b.echo("v2", "/o/docs/note.txt");
    assert_eq!(a.cat("/o/docs/note.txt").as_deref(), Some("v2"));

    // A symlink a has never seen leads to a directory a has cached, where b
    // has since overwritten the file.
    a.echo("v1", "/l/a/b/c.txt");
    assert_eq!(a.cat("/l/a/b/c.txt").as_deref(), Some("v1"));
    b.ln("/l/link", "/l/a/b");
    b.echo("v2", "/l/a/b/c.txt");
    assert_eq!(a.cat("/l/link/c.txt").as_deref(), Some("v2"));

    // A cached removal does not hide a later recreation.
    a.echo("v1", "/t/docs/note.txt");
    assert_eq!(a.cat("/t/docs/note.txt").as_deref(), Some("v1"));
    b.unlink("/t/docs/note.txt");
    assert_eq!(a.cat("/t/docs/note.txt"), None);
    b.echo("v2", "/t/docs/note.txt");
    assert_eq!(a.cat("/t/docs/note.txt").as_deref(), Some("v2"));
}
