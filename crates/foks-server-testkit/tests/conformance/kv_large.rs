use std::io::{Cursor, Read, Write};

use foks_client::KvWriteOptions;
use foks_client_db::SoftStateStore;
use foks_proto::Role;
use foks_server_testkit::TestAccountSpec;

use crate::support::Fixture;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

struct PatternReader {
    size: usize,
    offset: usize,
    seed: u8,
}

impl PatternReader {
    fn new(size: usize, seed: u8) -> Self {
        Self {
            size,
            offset: 0,
            seed,
        }
    }
}

impl Read for PatternReader {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let length = output.len().min(self.size.saturating_sub(self.offset));
        for (index, byte) in output[..length].iter_mut().enumerate() {
            *byte = pattern_byte(self.offset + index, self.seed);
        }
        self.offset += length;
        Ok(length)
    }
}

struct DigestWriter {
    size: u64,
    digest: u64,
}

impl Default for DigestWriter {
    fn default() -> Self {
        Self {
            size: 0,
            digest: FNV_OFFSET,
        }
    }
}

impl Write for DigestWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        for byte in bytes {
            self.digest ^= u64::from(*byte);
            self.digest = self.digest.wrapping_mul(FNV_PRIME);
        }
        self.size += bytes.len() as u64;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn pattern_byte(offset: usize, seed: u8) -> u8 {
    (offset as u8).wrapping_mul(31).wrapping_add(seed)
}

fn expected_digest(size: usize, seed: u8) -> u64 {
    (0..size).fold(FNV_OFFSET, |digest, offset| {
        (digest ^ u64::from(pattern_byte(offset, seed))).wrapping_mul(FNV_PRIME)
    })
}

#[test]
pub(crate) fn kv_large_success() {
    let fixture = Fixture::start("kv-large-client");
    let created = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("kvlargeuser", 0x41))
        .unwrap();
    let root = created.kv_projection[0].root_directory_id;
    let options = KvWriteOptions {
        read_role: Role::OWNER,
        write_role: Role::OWNER,
        overwrite: false,
        expected_version: None,
    };
    let content = vec![0x5a; 4 * 1024 * 1024 + 123];
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
        .put_file(root, "large.bin", &mut Cursor::new(&content), options)
        .unwrap();
    let metadata = session.sync().unwrap();
    let entry = metadata[0]
        .entries
        .iter()
        .find(|entry| entry.name == b"large.bin")
        .unwrap();
    assert!(entry.large_file_size.is_none());
    drop(session);
    let tree = fixture
        .client
        .foks()
        .sync_user_kv(
            fixture.host(),
            &created.credential,
            &created.authenticated.verified,
            &created.authenticated.puks,
            fixture.client.soft_state_path(),
        )
        .unwrap();
    let entry = tree[0]
        .entries
        .iter()
        .find(|entry| entry.name == b"large.bin")
        .unwrap();
    assert_eq!(entry.large_file_size, Some(content.len() as u64));
    let node_id = entry.node_id;

    let store = SoftStateStore::open(fixture.client.soft_state_path()).unwrap();
    let mut read_back = Vec::new();
    assert_eq!(
        store
            .write_large_file(
                created.authenticated.verified.host().as_bytes(),
                created.credential.uid.as_bytes(),
                &node_id,
                &mut read_back,
            )
            .unwrap(),
        Some(content.len() as u64)
    );
    assert_eq!(read_back, content);
}

#[test]
fn large_file_cutoff_and_chunk_boundaries_stream_exactly() {
    let fixture = Fixture::start("kv-large-boundary-client");
    let created = fixture
        .client
        .create_account(
            fixture.host(),
            &TestAccountSpec::new("kvlargeboundary", 0x43),
        )
        .unwrap();
    let root = created.kv_projection[0].root_directory_id;
    let options = KvWriteOptions {
        read_role: Role::OWNER,
        write_role: Role::OWNER,
        overwrite: false,
        expected_version: None,
    };
    let cases = [
        ("cutoff-plus-one", 2041, 0x11),
        ("exact-chunk", 4 * 1024 * 1024, 0x22),
        ("chunk-plus-one", 4 * 1024 * 1024 + 1, 0x33),
    ];
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
    for (name, size, seed) in cases {
        session
            .put_file(root, name, &mut PatternReader::new(size, seed), options)
            .unwrap();
    }
    let metadata = session.sync().unwrap();
    assert!(metadata[0]
        .entries
        .iter()
        .all(|entry| entry.large_file_size.is_none()));
    drop(session);
    let tree = fixture
        .client
        .foks()
        .sync_user_kv(
            fixture.host(),
            &created.credential,
            &created.authenticated.verified,
            &created.authenticated.puks,
            fixture.client.soft_state_path(),
        )
        .unwrap();
    let store = SoftStateStore::open(fixture.client.soft_state_path()).unwrap();
    for (name, size, seed) in cases {
        let entry = tree[0]
            .entries
            .iter()
            .find(|entry| entry.name == name.as_bytes())
            .unwrap();
        assert_eq!(entry.large_file_size, Some(size as u64));
        let mut digest = DigestWriter::default();
        assert_eq!(
            store
                .write_large_file(
                    created.authenticated.verified.host().as_bytes(),
                    created.credential.uid.as_bytes(),
                    &entry.node_id,
                    &mut digest,
                )
                .unwrap(),
            Some(size as u64)
        );
        assert_eq!(digest.size, size as u64);
        assert_eq!(digest.digest, expected_digest(size, seed));
    }
}

struct FailingReader {
    inner: PatternReader,
}

impl Read for FailingReader {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if self.inner.offset == self.inner.size {
            return Err(std::io::Error::other("injected upload read failure"));
        }
        self.inner.read(output)
    }
}

#[test]
fn interrupted_upload_is_hidden_across_restart_and_a_fresh_retry_succeeds() {
    let fixture = Fixture::start("kv-interrupted-client");
    let created = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("kvinterrupted", 0x47))
        .unwrap();
    let root = created.kv_projection[0].root_directory_id;
    let options = KvWriteOptions {
        read_role: Role::OWNER,
        write_role: Role::OWNER,
        overwrite: false,
        expected_version: None,
    };
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
    let error = session
        .put_file(
            root,
            "interrupted.bin",
            &mut FailingReader {
                inner: PatternReader::new(4 * 1024 * 1024 + 1, 0x66),
            },
            options,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        foks_client::Error::Rpc(foks_rpc::Error::Io(_))
    ));
    assert!(session.sync().unwrap()[0].entries.is_empty());
    drop(session);
    drop(protected);

    let crate::support::Fixture {
        environment,
        server,
        client,
        probe,
    } = fixture;
    server.shutdown().unwrap();
    let restarted = environment.start_server().unwrap();
    let reconstructed =
        foks_server_testkit::TestClient::new(&environment, "kv-interrupted-client").unwrap();
    let authenticated = reconstructed
        .foks()
        .authenticate_and_pin(&probe.pinned, &created.credential)
        .unwrap();
    let mut reopened = reconstructed.open_protected_store().unwrap();
    let mut retry = reconstructed
        .foks()
        .user_kv_write_session(
            &probe.pinned,
            &created.credential,
            &authenticated.verified,
            &authenticated.puks,
            reconstructed.soft_state_path(),
            &mut reopened,
        )
        .unwrap();
    let retry_size = 4 * 1024 * 1024 + 2;
    retry
        .put_file(
            root,
            "interrupted.bin",
            &mut PatternReader::new(retry_size, 0x77),
            options,
        )
        .unwrap();
    let metadata = retry.sync().unwrap();
    assert_eq!(metadata[0].entries.len(), 1);
    assert!(metadata[0].entries[0].large_file_size.is_none());
    drop(retry);
    let tree = reconstructed
        .foks()
        .sync_user_kv(
            &probe.pinned,
            &created.credential,
            &authenticated.verified,
            &authenticated.puks,
            reconstructed.soft_state_path(),
        )
        .unwrap();
    assert_eq!(tree[0].entries[0].large_file_size, Some(retry_size as u64));
    let retry_node = tree[0].entries[0].node_id;
    let store = SoftStateStore::open(reconstructed.soft_state_path()).unwrap();
    let mut digest = DigestWriter::default();
    assert_eq!(
        store
            .write_large_file(
                authenticated.verified.host().as_bytes(),
                created.credential.uid.as_bytes(),
                &retry_node,
                &mut digest,
            )
            .unwrap(),
        Some(retry_size as u64)
    );
    assert_eq!(digest.size, retry_size as u64);
    assert_eq!(digest.digest, expected_digest(retry_size, 0x77));
    drop(client);
    restarted.shutdown().unwrap();
}

fn write_options() -> KvWriteOptions {
    KvWriteOptions {
        read_role: Role::OWNER,
        write_role: Role::OWNER,
        overwrite: false,
        expected_version: None,
    }
}

/// Counts the server requests one delivered chunk costs, with the path walked
/// for that chunk and with the path remembered from an earlier walk.
///
/// A download issues one chunk read per delivered chunk, and walking the path
/// for each of them is what made a large read cost a request per directory
/// per chunk. A remembered path replaces the walk with one version-vector
/// check, which is what keeps the conflict semantics the walk provided.
#[test]
fn a_remembered_path_reads_a_chunk_in_fewer_requests_than_a_walk() {
    const STORED_CHUNK: usize = 4 * 1024 * 1024;
    let fixture = Fixture::start("kv-read-cost-client");
    let created = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("kvreadcost", 0x51))
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
    session.resolve_path(&[]).unwrap();
    let alpha = session
        .mkdir(root, "alpha", write_options())
        .unwrap()
        .node_id
        .object_id();
    session.resolve_path(&[b"alpha".to_vec()]).unwrap();
    let beta = session
        .mkdir(alpha, "beta", write_options())
        .unwrap()
        .node_id
        .object_id();
    let components = vec![b"alpha".to_vec(), b"beta".to_vec()];
    session.resolve_path(&components).unwrap();
    session
        .put_file(
            beta,
            "large.bin",
            &mut PatternReader::new(3 * STORED_CHUNK, 0x51),
            write_options(),
        )
        .unwrap();
    drop(session);
    drop(protected);

    let resolve = || {
        fixture
            .client
            .foks()
            .resolve_user_kv_path(
                fixture.host(),
                &created.credential,
                &created.authenticated.verified,
                &created.authenticated.puks,
                fixture.client.soft_state_path(),
                &components,
            )
            .unwrap()
    };
    let directories = resolve();
    let entry = directories
        .last()
        .unwrap()
        .entries
        .iter()
        .find(|entry| entry.name == b"large.bin")
        .unwrap();
    let node = foks_proto::KvNodeId(entry.node_id);
    let versions = foks_client::kv_path_version_vector(&directories);

    // The shape every chunk had: resolve the path, then read under the node
    // that walk named.
    let before = fixture.server.metrics().requests_started;
    let walked = resolve();
    assert!(!walked.is_empty());
    let walked_chunk = fixture
        .client
        .foks()
        .read_user_kv_chunk(
            fixture.host(),
            &created.credential,
            &created.authenticated.verified,
            &created.authenticated.puks,
            node,
            STORED_CHUNK as u64,
            STORED_CHUNK,
        )
        .unwrap();
    let with_walk = fixture.server.metrics().requests_started - before;

    // The shape with the path remembered: one version-vector check in place
    // of the walk.
    let before = fixture.server.metrics().requests_started;
    let remembered_chunk = fixture
        .client
        .foks()
        .read_user_kv_chunk_if_current(
            fixture.host(),
            &created.credential,
            &created.authenticated.verified,
            &created.authenticated.puks,
            node,
            STORED_CHUNK as u64,
            STORED_CHUNK,
            &versions,
        )
        .unwrap()
        .expect("the walk's version vector is still current");
    let with_memo = fixture.server.metrics().requests_started - before;

    assert_eq!(remembered_chunk.content, walked_chunk.content);
    assert_eq!(remembered_chunk.content.len(), STORED_CHUNK);
    println!("KV chunk requests: walked={with_walk} remembered={with_memo}");
    assert_eq!(
        with_memo, 3,
        "one cache check, one node read and one chunk read"
    );
    assert!(
        with_walk >= 3 * with_memo,
        "walked={with_walk} remembered={with_memo}"
    );
}

/// The property the obvious memo key would break.
///
/// A dirent version restarts at 1 after an unlink and a re-create, on a fresh
/// random dirent identifier, so the same `(path, version)` pair names a
/// different node before and after. A memo keyed on that pair alone would
/// serve the node it first resolved. The walk's version vector is what
/// refuses it: the unlink writes a tombstone to the dirent the vector cites,
/// which moves that dirent's head, and the server answers the vector stale.
#[test]
fn a_replaced_entry_refuses_a_remembered_path() {
    let fixture = Fixture::start("kv-read-replace-client");
    let created = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("kvreadreplace", 0x53))
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
    session.resolve_path(&[]).unwrap();
    session
        .put_file(
            root,
            "large.bin",
            &mut PatternReader::new(3000, 0x61),
            write_options(),
        )
        .unwrap();
    drop(session);
    drop(protected);

    let resolve = || {
        fixture
            .client
            .foks()
            .resolve_user_kv_path(
                fixture.host(),
                &created.credential,
                &created.authenticated.verified,
                &created.authenticated.puks,
                fixture.client.soft_state_path(),
                &[],
            )
            .unwrap()
    };
    let directories = resolve();
    let first = directories[0]
        .entries
        .iter()
        .find(|entry| entry.name == b"large.bin")
        .unwrap()
        .clone();
    assert_eq!(first.version, 1);
    let versions = foks_client::kv_path_version_vector(&directories);
    // The remembered node answers while the path is untouched.
    assert!(fixture
        .client
        .foks()
        .read_user_kv_chunk_if_current(
            fixture.host(),
            &created.credential,
            &created.authenticated.verified,
            &created.authenticated.puks,
            foks_proto::KvNodeId(first.node_id),
            0,
            3000,
            &versions,
        )
        .unwrap()
        .is_some());

    // A peer unlinks the entry and writes another file at the same name.
    let mut protected = fixture.client.open_protected_store().unwrap();
    let mut peer = fixture
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
    peer.resolve_path(&[]).unwrap();
    peer.unlink(root, "large.bin", None, Role::OWNER, false)
        .unwrap();
    peer.resolve_path(&[]).unwrap();
    peer.put_file(
        root,
        "large.bin",
        &mut PatternReader::new(3000, 0x62),
        write_options(),
    )
    .unwrap();
    drop(peer);
    drop(protected);

    let second = resolve()[0]
        .entries
        .iter()
        .find(|entry| entry.name == b"large.bin")
        .unwrap()
        .clone();
    // This is the hazard, stated: the path and the version are identical,
    // and they name a different dirent and a different node.
    assert_eq!(second.version, first.version);
    assert_ne!(second.dirent_id, first.dirent_id);
    assert_ne!(second.node_id, first.node_id);

    assert!(
        fixture
            .client
            .foks()
            .read_user_kv_chunk_if_current(
                fixture.host(),
                &created.credential,
                &created.authenticated.verified,
                &created.authenticated.puks,
                foks_proto::KvNodeId(first.node_id),
                0,
                3000,
                &versions,
            )
            .unwrap()
            .is_none(),
        "the superseded path was served rather than refused"
    );
}
