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
    let tree = session.sync().unwrap();
    let entry = tree[0]
        .entries
        .iter()
        .find(|entry| entry.name == b"large.bin")
        .unwrap();
    assert_eq!(entry.large_file_size, Some(content.len() as u64));
    let node_id = entry.node_id;
    drop(session);

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
    let tree = session.sync().unwrap();
    drop(session);
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
    let tree = retry.sync().unwrap();
    assert_eq!(tree[0].entries.len(), 1);
    assert_eq!(tree[0].entries[0].large_file_size, Some(retry_size as u64));
    let retry_node = tree[0].entries[0].node_id;
    drop(retry);
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
