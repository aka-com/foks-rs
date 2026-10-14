use std::io::{Cursor, Write as _};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::sync::{mpsc, Condvar, Mutex};
use std::time::Duration;

use foks_client::{CreatedSoftwareAccount, KvWriteOptions, PinnedHost};
use foks_client_db::SoftStateStore;
use foks_proto::Role;
use foks_server::chunks::ChunkStore;
use foks_server_testkit::{TestAccountSpec, TestClient, TestEnvironment, TestFault};

struct FaultStore {
    inner: Arc<dyn ChunkStore>,
    // 1: reject put; 2: reject get; 3: corrupt get.
    mode: AtomicU8,
}

impl ChunkStore for FaultStore {
    fn put(&self, digest: &[u8; 32], bytes: &[u8]) -> foks_server::Result<()> {
        if self.mode.load(Ordering::SeqCst) == 1 {
            return Err(foks_server::Error::Chunk("injected object put failure"));
        }
        self.inner.put(digest, bytes)
    }

    fn get(&self, digest: &[u8; 32], length: u64) -> foks_server::Result<Vec<u8>> {
        let mode = self.mode.load(Ordering::SeqCst);
        if mode == 2 {
            return Err(foks_server::Error::Chunk("injected missing object"));
        }
        let mut bytes = self.inner.get(digest, length)?;
        if mode == 3 {
            bytes[0] ^= 1;
        }
        Ok(bytes)
    }
}

struct GatedStore {
    inner: Arc<dyn ChunkStore>,
    entered: mpsc::Sender<()>,
    released: Mutex<bool>,
    release: Condvar,
}

impl ChunkStore for GatedStore {
    fn put(&self, digest: &[u8; 32], bytes: &[u8]) -> foks_server::Result<()> {
        // The object is durable before the gate: authorization can change
        // during storage I/O, leaving an orphan but never a published chunk.
        self.inner.put(digest, bytes)?;
        self.entered.send(()).unwrap();
        let (_guard, timeout) = self
            .release
            .wait_timeout_while(
                self.released.lock().unwrap(),
                Duration::from_secs(4),
                |released| !*released,
            )
            .unwrap();
        if timeout.timed_out() {
            return Err(foks_server::Error::Chunk("test publication gate timed out"));
        }
        Ok(())
    }

    fn get(&self, digest: &[u8; 32], length: u64) -> foks_server::Result<Vec<u8>> {
        self.inner.get(digest, length)
    }
}

#[test]
fn revocation_during_external_put_prevents_publication_without_blocking_writer() {
    use foks_client::{
        NewSoftwareDeviceSecrets, NoPassphraseConfigured, SoftwareDeviceProvisionRequest,
        UserPukRotation,
    };
    use foks_proto::SecretSeed;
    let environment = TestEnvironment::new().unwrap();
    let (entered, entry) = mpsc::channel();
    let store = Arc::new(GatedStore {
        inner: environment.filesystem_chunk_store().unwrap(),
        entered,
        released: Mutex::new(false),
        release: Condvar::new(),
    });
    environment.set_chunk_store(store.clone()).unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "external-revoke").unwrap();
    let host = client.probe_and_pin().unwrap().pinned;
    let account = client
        .create_account(&host, &TestAccountSpec::new("externalrevoke", 0x91))
        .unwrap();
    let mut protected = client.open_protected_store().unwrap();
    let second = client
        .foks()
        .provision_software_device(
            &host,
            &account.credential,
            SoftwareDeviceProvisionRequest {
                role: Role::OWNER,
                device_name: "revoker".to_owned(),
                serial: 2,
            },
            NewSoftwareDeviceSecrets::new(SecretSeed::new([0xa1; 32]), None, [0xa2; 17]),
            &mut protected,
        )
        .unwrap();
    drop(protected);
    let revoker = TestClient::new(&environment, "external-revoker").unwrap();
    let original = foks_crypto::derive_device_public(&SecretSeed::new([0x91; 32])).unwrap();
    std::thread::scope(|scope| {
        let upload =
            scope.spawn(|| put(&client, &host, &account, "revoked.bin", &vec![0x34; 8192]));
        entry
            .recv_timeout(Duration::from_secs(3))
            .expect("upload reached object storage");
        let mut protected = revoker.open_protected_store().unwrap();
        // Retain the originating hard-state attestation that this account was
        // created without a passphrase; a fresh pin alone cannot attest that.
        let revoked = revoker.foks().revoke_user_credential_with_software_device(
            &host,
            &second.credential,
            &original.id,
            &[UserPukRotation {
                role: Role::OWNER,
                previous_generation: 1,
                previous_seed: SecretSeed::new([0x92; 32]),
                new_seed: SecretSeed::new([0xb1; 32]),
            }],
            Some(NoPassphraseConfigured),
            &mut protected,
        );
        *store.released.lock().unwrap() = true;
        store.release.notify_all();
        revoked.expect("revocation must complete while storage I/O is blocked");
        assert!(
            upload.join().unwrap().is_err(),
            "revoked upload must not publish"
        );
    });
    let usage = environment
        .read_database()
        .unwrap()
        .kv_usage(account.credential.uid.as_bytes())
        .unwrap();
    assert_eq!(usage.large.base.number, 0);
    assert_eq!(usage.large.chunks, 0);
    server.shutdown().unwrap();
}

#[test]
fn mixed_inline_and_external_files_survive_restart_and_self_contained_backup() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "external-mixed").unwrap();
    let host = client.probe_and_pin().unwrap().pinned;
    let account = client
        .create_account(&host, &TestAccountSpec::new("externalmixed", 0x61))
        .unwrap();
    let inline = vec![0x31; 8192];
    put(&client, &host, &account, "inline.bin", &inline).unwrap();
    server.shutdown().unwrap();

    environment.use_filesystem_chunks().unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "external-mixed").unwrap();
    let external = vec![0x42; 4 * 1024 * 1024 + 17];
    put(&client, &host, &account, "external.bin", &external).unwrap();
    let connection = rusqlite::Connection::open(environment.database_path()).unwrap();
    let (inline_rows, external_rows, embedded_external_bytes): (i64, i64, i64) = connection.query_row(
        "SELECT sum(external_digest IS NULL), sum(external_digest IS NOT NULL), \
         coalesce(sum(CASE WHEN external_digest IS NOT NULL THEN length(ciphertext) + length(exact_upload_chunk) ELSE 0 END),0) FROM kv_file_chunks",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!(
        (inline_rows, external_rows, embedded_external_bytes),
        (1, 2, 0)
    );
    drop(connection);
    assert_files(
        &client,
        &host,
        &account,
        &[("inline.bin", &inline), ("external.bin", &external)],
    );
    let backup = server.backup_named("mixed").unwrap();
    assert!(server
        .operator_backup_named("without-object-store")
        .is_err());
    let backend = environment.filesystem_chunk_store().unwrap();
    assert!(
        foks_server::externalize_chunks(
            environment.database_path(),
            foks_server_db::Config::default(),
            backend.as_ref()
        )
        .is_err(),
        "conversion must refuse a running writer"
    );
    let addresses = server.addresses();
    server.shutdown().unwrap();

    assert_eq!(
        foks_server::externalize_chunks(
            environment.database_path(),
            foks_server_db::Config::default(),
            backend.as_ref()
        )
        .unwrap(),
        1
    );
    assert_eq!(
        foks_server::externalize_chunks(
            environment.database_path(),
            foks_server_db::Config::default(),
            backend.as_ref()
        )
        .unwrap(),
        0
    );

    environment.clear_chunk_store().unwrap();
    assert!(
        environment.start_server().is_err(),
        "external references require a backend at startup"
    );
    environment.use_filesystem_chunks().unwrap();

    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "external-mixed").unwrap();
    assert_files(
        &client,
        &host,
        &account,
        &[("inline.bin", &inline), ("external.bin", &external)],
    );
    server.shutdown().unwrap();
    // The restored installation deliberately has no configured object store.
    std::fs::remove_dir_all(environment.root().join("chunks")).unwrap();
    let restored = TestEnvironment::restore_backup(&backup, addresses).unwrap();
    let server = restored.start_server().unwrap();
    let client = TestClient::new(&environment, "external-mixed").unwrap();
    assert_files(
        &client,
        &host,
        &account,
        &[("inline.bin", &inline), ("external.bin", &external)],
    );
    server.shutdown().unwrap();
}

#[test]
fn object_failures_never_publish_missing_chunks_or_serve_corrupt_ciphertext() {
    let environment = TestEnvironment::new().unwrap();
    let inner = environment.filesystem_chunk_store().unwrap();
    let store = Arc::new(FaultStore {
        inner,
        mode: AtomicU8::new(1),
    });
    environment.set_chunk_store(store.clone()).unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "external-failure").unwrap();
    let host = client.probe_and_pin().unwrap().pinned;
    let account = client
        .create_account(&host, &TestAccountSpec::new("externalfailure", 0x71))
        .unwrap();
    let bytes = vec![0x63; 8192];
    assert!(put(&client, &host, &account, "failed.bin", &bytes).is_err());
    let usage = environment
        .read_database()
        .unwrap()
        .kv_usage(account.credential.uid.as_bytes())
        .unwrap();
    assert_eq!(usage.large.chunks, 0);
    assert_eq!(usage.large.base.number, 0);
    assert!(sync(&client, &host, &account).unwrap()[0]
        .entries
        .is_empty());

    // Retry with the same name and bytes after a rejected object put. This is
    // a new upload; the failed attempt published no metadata or quota charge.
    store.mode.store(0, Ordering::SeqCst);
    put(&client, &host, &account, "failed.bin", &bytes).unwrap();
    assert_files(&client, &host, &account, &[("failed.bin", &bytes)]);
    for mode in [2, 3] {
        store.mode.store(mode, Ordering::SeqCst);
        // Content sync fetches chunks afresh despite existing soft-state data.
        assert!(sync(&client, &host, &account).is_err());
        assert!(server.backup_named(&format!("bad-{mode}")).is_err());
        assert!(!environment
            .root()
            .join(format!("backup/bad-{mode}/key-manifest.txt"))
            .exists());
    }
    store.mode.store(0, Ordering::SeqCst);
    assert_files(&client, &host, &account, &[("failed.bin", &bytes)]);
    server.shutdown().unwrap();
}

#[test]
fn external_upload_reconciles_lost_response_without_duplicate_quota_charge() {
    let environment = TestEnvironment::new().unwrap();
    environment.use_filesystem_chunks().unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "external-retry").unwrap();
    let host = client.probe_and_pin().unwrap().pinned;
    let account = client
        .create_account(&host, &TestAccountSpec::new("externalretry", 0x81))
        .unwrap();
    let before = environment.arm_fault(TestFault::UploadInitAfterCommitBeforeResponse);
    let bytes = vec![0x52; 8192];
    let mut node = [0x53; 17];
    node[0] = 2;
    let node = foks_proto::KvNodeId(node);
    let file_seed = foks_proto::SecretSeed::new([0x54; 32]);
    let key = &account.authenticated.puks[0];
    let metadata = foks_crypto::derive_kv_keys(&key.seed)
        .unwrap()
        .seal_file_seed(
            node,
            foks_proto::RoleAndGeneration {
                role: key.role,
                generation: key.generation,
            },
            1,
            &file_seed,
            [0x55; 16],
        )
        .unwrap();
    let chunk = foks_crypto::seal_kv_chunk(&file_seed, node, 0, true, &bytes, 0).unwrap();
    let request = foks_rpc::encode_kv_file_upload_init_request_at(
        foks_rpc::KvAuth::User,
        node.object_id(),
        &metadata,
        &chunk,
        1,
    )
    .unwrap();
    let mut connection =
        authenticated_stream(server.addresses().authenticated, &host, &account.credential);
    connection.write_all(&request).unwrap();
    assert!(
        foks_rpc::read_void_response(&mut connection, foks_rpc::DEFAULT_MAX_FRAME_LENGTH, 1)
            .is_err()
    );
    drop(connection);
    assert_eq!(environment.fault_hits(), before + 1);
    let before_replay = environment
        .read_database()
        .unwrap()
        .kv_usage(account.credential.uid.as_bytes())
        .unwrap();
    // Reconnect and replay the SAME encoded request. Calling put_file again
    // generates a fresh file ID and is a distinct upload, not exact replay.
    let mut connection =
        authenticated_stream(server.addresses().authenticated, &host, &account.credential);
    connection.write_all(&request).unwrap();
    foks_rpc::read_void_response(&mut connection, foks_rpc::DEFAULT_MAX_FRAME_LENGTH, 1).unwrap();
    let usage = environment
        .read_database()
        .unwrap()
        .kv_usage(account.credential.uid.as_bytes())
        .unwrap();
    assert_eq!(usage, before_replay);
    assert_eq!(usage.large.base.number, 1);
    assert_eq!(usage.large.chunks, 1);
    let fetched = client
        .foks()
        .read_user_kv_chunk(
            &host,
            &account.credential,
            &account.authenticated.verified,
            &account.authenticated.puks,
            node,
            0,
            bytes.len(),
        )
        .unwrap();
    assert_eq!(fetched.content, bytes);
    assert!(fetched.eof);
    server.shutdown().unwrap();
}

fn authenticated_stream(
    address: std::net::SocketAddr,
    host: &PinnedHost,
    credential: &foks_client::DeviceCredential,
) -> rustls::StreamOwned<rustls::ClientConnection, std::net::TcpStream> {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
    let certificates = credential
        .certificate_chain
        .iter()
        .cloned()
        .map(CertificateDer::from)
        .collect();
    let key = foks_crypto::device_signing_key_pkcs8(&credential.seed).unwrap();
    let mut roots = rustls::RootCertStore::empty();
    for certificate in host.tls_ca_certificates() {
        roots
            .add(CertificateDer::from(certificate.clone()))
            .unwrap();
    }
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_client_auth_cert(
        certificates,
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.to_vec())),
    )
    .unwrap();
    let tcp = std::net::TcpStream::connect_timeout(&address, Duration::from_secs(3)).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    tcp.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
    let connection = rustls::ClientConnection::new(
        Arc::new(config),
        ServerName::try_from("localhost".to_owned()).unwrap(),
    )
    .unwrap();
    let mut stream = rustls::StreamOwned::new(connection, tcp);
    stream
        .write_all(&foks_rpc::encode_kv_select_vhost_request(host.host_id()).unwrap())
        .unwrap();
    foks_rpc::read_void_response(&mut stream, foks_rpc::DEFAULT_MAX_FRAME_LENGTH, 0).unwrap();
    stream
}

fn put(
    client: &TestClient,
    host: &PinnedHost,
    account: &CreatedSoftwareAccount,
    name: &str,
    bytes: &[u8],
) -> foks_client::Result<()> {
    let mut protected = client.open_protected_store().unwrap();
    let mut session = client.foks().user_kv_write_session(
        host,
        &account.credential,
        &account.authenticated.verified,
        &account.authenticated.puks,
        client.soft_state_path(),
        &mut protected,
    )?;
    session.put_file(
        account.kv_projection[0].root_directory_id,
        name,
        &mut Cursor::new(bytes),
        KvWriteOptions {
            read_role: Role::OWNER,
            write_role: Role::OWNER,
            overwrite: false,
            expected_version: None,
        },
    )?;
    Ok(())
}

fn sync(
    client: &TestClient,
    host: &PinnedHost,
    account: &CreatedSoftwareAccount,
) -> foks_client::Result<Vec<foks_client_db::KvDirectoryProjection>> {
    client.foks().sync_user_kv(
        host,
        &account.credential,
        &account.authenticated.verified,
        &account.authenticated.puks,
        client.soft_state_path(),
    )
}

fn assert_files(
    client: &TestClient,
    host: &PinnedHost,
    account: &CreatedSoftwareAccount,
    files: &[(&str, &[u8])],
) {
    let tree = sync(client, host, account).unwrap();
    assert_eq!(tree[0].entries.len(), files.len());
    let store = SoftStateStore::open(client.soft_state_path()).unwrap();
    for (name, expected) in files {
        let entry = tree[0]
            .entries
            .iter()
            .find(|entry| entry.name == name.as_bytes())
            .unwrap();
        let mut bytes = Vec::new();
        assert_eq!(
            store
                .write_large_file(
                    host.host_id().as_bytes(),
                    account.credential.uid.as_bytes(),
                    &entry.node_id,
                    &mut bytes
                )
                .unwrap(),
            Some(expected.len() as u64)
        );
        assert_eq!(&bytes, expected);
    }
}
