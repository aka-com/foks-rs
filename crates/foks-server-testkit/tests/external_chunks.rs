use std::io::{Cursor, Write as _};
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
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
    // 1: reject put; 2: reject get; 3: corrupt get; 4: publish then lose reply.
    puts: AtomicUsize,
    mode: AtomicU8,
}

impl ChunkStore for FaultStore {
    fn namespace_lease(
        &self,
        exclusive: bool,
    ) -> foks_server::Result<Option<foks_server::chunks::ChunkNamespaceLease>> {
        self.inner.namespace_lease(exclusive)
    }
    fn namespace_is_empty(&self) -> foks_server::Result<bool> {
        self.inner.namespace_is_empty()
    }
    fn inventory_page(
        &self,
        cursor: Option<&str>,
        maximum: usize,
    ) -> foks_server::Result<foks_server::chunks::ChunkInventoryPage> {
        self.inner.inventory_page(cursor, maximum)
    }
    fn locator(&self) -> foks_server::Result<String> {
        self.inner.locator()
    }
    fn read_namespace_marker(&self) -> foks_server::Result<Option<Vec<u8>>> {
        self.inner.read_namespace_marker()
    }
    fn create_namespace_marker_if_absent(&self, marker: &[u8]) -> foks_server::Result<()> {
        self.inner.create_namespace_marker_if_absent(marker)
    }

    fn put_object(
        &self,
        object: &foks_server_db::ChunkObject,
        bytes: &[u8],
    ) -> foks_server::Result<()> {
        self.puts.fetch_add(1, Ordering::SeqCst);
        if self.mode.load(Ordering::SeqCst) == 1 {
            return Err(foks_server::Error::Chunk("injected object put failure"));
        }
        self.inner.put_object(object, bytes)?;
        if self.mode.load(Ordering::SeqCst) == 4 {
            return Err(foks_server::Error::Chunk("injected lost PUT reply"));
        }
        Ok(())
    }

    fn get_object(&self, object: &foks_server_db::ChunkObject) -> foks_server::Result<Vec<u8>> {
        let mode = self.mode.load(Ordering::SeqCst);
        if mode == 2 {
            return Err(foks_server::Error::Chunk("injected missing object"));
        }
        let mut bytes = self.inner.get_object(object)?;
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
    fn namespace_lease(
        &self,
        exclusive: bool,
    ) -> foks_server::Result<Option<foks_server::chunks::ChunkNamespaceLease>> {
        self.inner.namespace_lease(exclusive)
    }
    fn namespace_is_empty(&self) -> foks_server::Result<bool> {
        self.inner.namespace_is_empty()
    }
    fn inventory_page(
        &self,
        cursor: Option<&str>,
        maximum: usize,
    ) -> foks_server::Result<foks_server::chunks::ChunkInventoryPage> {
        self.inner.inventory_page(cursor, maximum)
    }
    fn locator(&self) -> foks_server::Result<String> {
        self.inner.locator()
    }
    fn read_namespace_marker(&self) -> foks_server::Result<Option<Vec<u8>>> {
        self.inner.read_namespace_marker()
    }
    fn create_namespace_marker_if_absent(&self, marker: &[u8]) -> foks_server::Result<()> {
        self.inner.create_namespace_marker_if_absent(marker)
    }

    fn put_object(
        &self,
        object: &foks_server_db::ChunkObject,
        bytes: &[u8],
    ) -> foks_server::Result<()> {
        // The object is durable before the gate: authorization can change
        // during storage I/O, leaving an orphan but never a published chunk.
        self.inner.put_object(object, bytes)?;
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

    fn get_object(&self, object: &foks_server_db::ChunkObject) -> foks_server::Result<Vec<u8>> {
        self.inner.get_object(object)
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
    let physical = environment
        .read_database()
        .unwrap()
        .chunk_physical_usage()
        .unwrap();
    assert_eq!(
        (
            physical.objects,
            physical.orphan_objects,
            physical.unresolved_attempts
        ),
        (1, 1, 0)
    );
    assert!(physical.present_bytes > 0);
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
        puts: AtomicUsize::new(0),
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
    exact_upload_retry(false);
}

#[test]
fn lost_put_reply_remains_charged_across_restart_and_reconciles_without_another_put() {
    exact_upload_retry(true);
}

fn exact_upload_retry(lost_put: bool) {
    let environment = TestEnvironment::new().unwrap();
    let store = Arc::new(FaultStore {
        inner: environment.filesystem_chunk_store().unwrap(),
        puts: AtomicUsize::new(0),
        mode: AtomicU8::new(if lost_put { 4 } else { 0 }),
    });
    environment.set_chunk_store(store.clone()).unwrap();
    let mut server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "external-retry").unwrap();
    let host = client.probe_and_pin().unwrap().pinned;
    let account = client
        .create_account(&host, &TestAccountSpec::new("externalretry", 0x81))
        .unwrap();
    let before = if lost_put {
        environment.fault_hits()
    } else {
        environment.arm_fault(TestFault::UploadInitAfterCommitBeforeResponse)
    };
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
    if !lost_put {
        assert_eq!(environment.fault_hits(), before + 1);
    }
    if lost_put {
        let usage = environment
            .read_database()
            .unwrap()
            .chunk_physical_usage()
            .unwrap();
        assert!(usage.uncertain_bytes > 0);
        assert_eq!((usage.objects, usage.unresolved_attempts), (1, 1));
        store.mode.store(0, Ordering::SeqCst);
        server.shutdown().unwrap();
        server = environment.start_server().unwrap();
    }
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
    if !lost_put {
        assert_eq!(usage, before_replay);
    } else {
        assert_eq!(before_replay.large.chunks, 0);
    }
    assert_eq!(store.puts.load(Ordering::SeqCst), 1);
    let physical = environment
        .read_database()
        .unwrap()
        .chunk_physical_usage()
        .unwrap();
    assert_eq!(physical.objects, 1);
    assert_eq!(physical.uncertain_bytes, 0);
    assert_eq!(physical.unresolved_attempts, u64::from(lost_put));
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

#[test]
fn binding_rejects_wrong_store_missing_marker_and_foreign_marker_even_without_chunks() {
    let environment = TestEnvironment::new().unwrap();
    environment.use_filesystem_chunks().unwrap();
    let store = environment.filesystem_chunk_store().unwrap();
    environment.start_server().unwrap().shutdown().unwrap();
    let marker_path = environment.root().join("chunks/_foks_namespace_v1");
    let marker = std::fs::read(&marker_path).unwrap();
    let wrong = foks_server::chunks::ChunkStoreConfig::Filesystem {
        directory: environment.root().join("wrong-chunks"),
    }
    .build()
    .unwrap();
    environment.set_chunk_store(wrong.clone()).unwrap();
    assert!(environment.start_server().is_err());
    assert!(foks_server::externalize_chunks(
        environment.database_path(),
        foks_server_db::Config::default(),
        wrong.as_ref()
    )
    .is_err());
    let backup_path = environment.root().join("wrong-backup");
    assert!(
        foks_server::backup_standalone_installation_with_chunk_store(
            environment.database_path(),
            environment.root().join("keys"),
            [0x51; 32],
            &backup_path,
            foks_server_db::Config::default(),
            Some(wrong.as_ref())
        )
        .is_err()
    );
    assert!(!backup_path.join("key-manifest.txt").exists());
    environment.set_chunk_store(store.clone()).unwrap();
    std::fs::remove_file(&marker_path).unwrap();
    assert!(environment.start_server().is_err());
    assert!(
        !marker_path.exists(),
        "active binding must not recreate a missing marker"
    );
    std::fs::write(&marker_path, b"foreign marker").unwrap();
    assert!(environment.start_server().is_err());
    std::fs::write(&marker_path, &marker).unwrap();
    environment.start_server().unwrap().shutdown().unwrap();
}

#[test]
fn legacy_external_adoption_is_explicit_resumable_and_preserves_wire_reads() {
    let environment = TestEnvironment::new().unwrap();
    environment.use_filesystem_chunks().unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "legacy-adopt").unwrap();
    let host = client.probe_and_pin().unwrap().pinned;
    let account = client
        .create_account(&host, &TestAccountSpec::new("legacyadopt", 0x71))
        .unwrap();
    let content = vec![0x41; 4 * 1024 * 1024 + 17];
    put(&client, &host, &account, "adopt.bin", &content).unwrap();
    server.shutdown().unwrap();
    // Reconstruct a real version-50 database and unmarked legacy store.
    // Version 50 used digest keys. Materialize exactly those historical objects
    // before reconstructing its schema, rather than retaining modern v2 keys.
    let historical_store = environment.filesystem_chunk_store().unwrap();
    {
        let db = foks_server_db::Database::open(environment.database_path(), Default::default())
            .unwrap();
        let mut after = None;
        while let Some(chunk) = db.next_kv_chunk(after.as_ref(), true).unwrap() {
            let foks_server_db::KvChunkPayload::External(reference) = &chunk.chunk.payload else {
                panic!("external fixture");
            };
            let bytes = historical_store.get_object(&reference.object).unwrap();
            historical_store
                .put(&reference.object.digest, &bytes)
                .unwrap();
            after = Some(chunk.key);
        }
    }
    let connection = rusqlite::Connection::open(environment.database_path()).unwrap();
    connection
        .execute_batch(include_str!(
            "../../foks-server-db/tests/fixtures/remove_kv_current.sql"
        ))
        .unwrap();
    connection
        .execute_batch(include_str!(
            "../../foks-server-db/tests/fixtures/remove_chunk_binding.sql"
        ))
        .unwrap();
    connection.pragma_update(None, "user_version", 50).unwrap();
    drop(connection);
    std::fs::remove_file(environment.root().join("chunks/_foks_namespace_v1")).unwrap();
    assert!(environment.start_server().is_err());
    let store = FaultStore {
        puts: AtomicUsize::new(0),
        inner: environment.filesystem_chunk_store().unwrap(),
        mode: AtomicU8::new(0),
    };
    let failed = foks_server::adopt_chunk_store(
        environment.database_path(),
        foks_server_db::Config::default(),
        &store,
        |progress| {
            assert_eq!(progress.verified_objects, 1);
            store.mode.store(2, Ordering::SeqCst);
        },
    );
    assert!(failed.is_err());
    let binding = environment
        .read_database()
        .unwrap()
        .chunk_storage_binding()
        .unwrap()
        .unwrap();
    assert_eq!(binding.verified_objects, 1);
    assert_eq!(
        binding.state,
        foks_server_db::ChunkStorageBindingState::PendingAdoption
    );
    assert!(store.read_namespace_marker().unwrap().is_none());
    assert!(environment.start_server().is_err());
    store.mode.store(0, Ordering::SeqCst);
    let mut callbacks = 0;
    let adopted = foks_server::adopt_chunk_store(
        environment.database_path(),
        foks_server_db::Config::default(),
        &store,
        |_| callbacks += 1,
    )
    .unwrap();
    assert_eq!(adopted.verified_objects, 2);
    assert_eq!(
        callbacks, 1,
        "verified first row is resumed without fetching it again"
    );
    assert_eq!(
        environment
            .read_database()
            .unwrap()
            .chunk_storage_binding()
            .unwrap()
            .unwrap()
            .namespace_id,
        binding.namespace_id
    );
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "legacy-adopt").unwrap();
    assert_files(&client, &host, &account, &[("adopt.bin", &content)]);
    let backup = server.backup_named("adopted").unwrap();
    let addresses = server.addresses();
    server.shutdown().unwrap();
    let restored = TestEnvironment::restore_backup(&backup, addresses).unwrap();
    assert!(restored
        .read_database()
        .unwrap()
        .chunk_storage_binding()
        .unwrap()
        .is_none());
    restored.use_filesystem_chunks().unwrap();
    let server = restored.start_server().unwrap();
    assert_ne!(
        restored
            .read_database()
            .unwrap()
            .chunk_storage_binding()
            .unwrap()
            .unwrap()
            .namespace_id,
        binding.namespace_id
    );
    assert_files(&client, &host, &account, &[("adopt.bin", &content)]);
    server.shutdown().unwrap();
}

#[test]
fn preexisting_unknown_objects_require_inventory_and_remain_charged_without_deletion() {
    let environment = TestEnvironment::new().unwrap();
    environment.use_filesystem_chunks().unwrap();
    let store = environment.filesystem_chunk_store().unwrap();
    let orphan = environment.root().join("chunks/old-temporary-object");
    std::fs::write(&orphan, vec![0x77; 100]).unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "inventory").unwrap();
    let host = client.probe_and_pin().unwrap().pinned;
    let account = client
        .create_account(&host, &TestAccountSpec::new("inventoryuser", 0x92))
        .unwrap();
    assert!(
        !environment
            .read_database()
            .unwrap()
            .chunk_physical_usage()
            .unwrap()
            .inventory_complete
    );
    assert!(put(&client, &host, &account, "blocked.bin", &vec![3; 8192]).is_err());
    assert_eq!(
        environment
            .read_database()
            .unwrap()
            .kv_usage(account.credential.uid.as_bytes())
            .unwrap()
            .large
            .chunks,
        0
    );
    server.shutdown().unwrap();
    let report = foks_server::inventory_chunk_store(
        environment.database_path(),
        Default::default(),
        store.as_ref(),
        false,
        |_| {},
    )
    .unwrap();
    assert!(report.inventory_complete);
    assert_eq!(
        (
            report.present_bytes,
            report.unknown_objects,
            report.orphan_bytes
        ),
        (100, 1, 100)
    );
    assert!(orphan.exists());
    let server = environment.start_server().unwrap();
    put(&client, &host, &account, "ready.bin", &vec![4; 8192]).unwrap();
    let report = environment
        .read_database()
        .unwrap()
        .chunk_physical_usage()
        .unwrap();
    assert_eq!(
        (report.objects, report.unknown_objects, report.orphan_bytes),
        (2, 1, 100)
    );
    assert!(report.present_bytes > 100);
    assert_eq!(report.unresolved_attempts, 0);
    assert_files(&client, &host, &account, &[("ready.bin", &vec![4; 8192])]);
    server.shutdown().unwrap();
}

struct BackupReadGate {
    inner: Arc<dyn ChunkStore>,
    entered: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}
impl ChunkStore for BackupReadGate {
    fn namespace_lease(
        &self,
        exclusive: bool,
    ) -> foks_server::Result<Option<foks_server::chunks::ChunkNamespaceLease>> {
        self.inner.namespace_lease(exclusive)
    }
    fn locator(&self) -> foks_server::Result<String> {
        self.inner.locator()
    }
    fn read_namespace_marker(&self) -> foks_server::Result<Option<Vec<u8>>> {
        self.inner.read_namespace_marker()
    }
    fn create_namespace_marker_if_absent(&self, marker: &[u8]) -> foks_server::Result<()> {
        self.inner.create_namespace_marker_if_absent(marker)
    }
    fn namespace_is_empty(&self) -> foks_server::Result<bool> {
        self.inner.namespace_is_empty()
    }
    fn inventory_page(
        &self,
        cursor: Option<&str>,
        maximum: usize,
    ) -> foks_server::Result<foks_server::chunks::ChunkInventoryPage> {
        self.inner.inventory_page(cursor, maximum)
    }
    fn put_object(
        &self,
        object: &foks_server_db::ChunkObject,
        bytes: &[u8],
    ) -> foks_server::Result<()> {
        self.inner.put_object(object, bytes)
    }
    fn get_object(&self, object: &foks_server_db::ChunkObject) -> foks_server::Result<Vec<u8>> {
        self.entered.send(()).unwrap();
        self.release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(20))
            .map_err(|_| foks_server::Error::Chunk("test backup gate timed out"))?;
        self.inner.get_object(object)
    }
}

#[test]
fn standalone_backup_outlives_server_and_excludes_offline_collection_until_hydrated() {
    let environment = TestEnvironment::new().unwrap();
    environment.use_filesystem_chunks().unwrap();
    let store = environment.filesystem_chunk_store().unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "collection-backup").unwrap();
    let host = client.probe_and_pin().unwrap().pinned;
    let account = client
        .create_account(&host, &TestAccountSpec::new("collectionbackup", 0x93))
        .unwrap();
    let bytes = vec![0x61; 8192];
    put(&client, &host, &account, "saved.bin", &bytes).unwrap();
    assert!(foks_server::collect_chunk_store(
        environment.database_path(),
        Default::default(),
        store.as_ref(),
        128,
        1024 * 1024,
        true
    )
    .is_err());
    let (entered, entry) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let gate = Arc::new(BackupReadGate {
        inner: store.clone(),
        entered,
        release: Mutex::new(released),
    });
    let db = environment.database_path().to_path_buf();
    let keys = environment.root().join("keys");
    let destination = environment.root().join("independent-backup");
    let backup = std::thread::spawn(move || {
        foks_server::backup_standalone_installation_with_chunk_store(
            db,
            keys,
            [0x51; 32],
            destination,
            Default::default(),
            Some(gate.as_ref()),
        )
    });
    entry.recv_timeout(Duration::from_secs(5)).unwrap();
    let addresses = server.addresses();
    server.shutdown().unwrap();
    let error = foks_server::collect_chunk_store(
        environment.database_path(),
        Default::default(),
        store.as_ref(),
        128,
        1024 * 1024,
        false,
    )
    .unwrap_err();
    assert!(error.to_string().contains("storage is in use"));
    release.send(()).unwrap();
    let backup = backup.join().unwrap().unwrap();
    let dry = foks_server::collect_chunk_store(
        environment.database_path(),
        Default::default(),
        store.as_ref(),
        128,
        1024 * 1024,
        true,
    )
    .unwrap();
    assert_eq!(dry.eligible_objects, 0);
    // Simulate metadata retention removal; every retained reference has gone.
    let c = rusqlite::Connection::open(environment.database_path()).unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON; DELETE FROM kv_file_uploads;")
        .unwrap();
    drop(c);
    let dry = foks_server::collect_chunk_store(
        environment.database_path(),
        Default::default(),
        store.as_ref(),
        128,
        1024 * 1024,
        true,
    )
    .unwrap();
    assert_eq!(dry.eligible_objects, 1);
    assert_eq!(dry.deleted_objects, 0);
    let collected = foks_server::collect_chunk_store(
        environment.database_path(),
        Default::default(),
        store.as_ref(),
        128,
        1024 * 1024,
        false,
    )
    .unwrap();
    assert_eq!(
        (
            collected.deleted_objects,
            collected.usage.objects,
            collected.usage.tombstones
        ),
        (1, 0, 1)
    );
    let again = foks_server::collect_chunk_store(
        environment.database_path(),
        Default::default(),
        store.as_ref(),
        128,
        1024 * 1024,
        false,
    )
    .unwrap();
    assert_eq!(again.deleted_objects, 0);
    let restored = TestEnvironment::restore_backup(&backup, addresses).unwrap();
    let server = restored.start_server().unwrap();
    assert_files(&client, &host, &account, &[("saved.bin", &bytes)]);
    server.shutdown().unwrap();
}

#[test]
fn abandoned_filesystem_read_keeps_storage_lease_after_server_shutdown() {
    let environment =
        TestEnvironment::with_profile(foks_server_testkit::TestProfile::StorageShutdown).unwrap();
    environment.use_filesystem_chunks().unwrap();
    let store = environment.filesystem_chunk_store().unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "abandoned-read").unwrap();
    let host = client.probe_and_pin().unwrap().pinned;
    let account = client
        .create_account(&host, &TestAccountSpec::new("abandonedread", 0x94))
        .unwrap();
    let bytes = vec![0x62; 8192];
    put(&client, &host, &account, "held.bin", &bytes).unwrap();
    server.shutdown().unwrap();
    let c = rusqlite::Connection::open(environment.database_path()).unwrap();
    let file: Vec<u8> = c
        .query_row("SELECT file_id FROM kv_file_uploads LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    drop(c);
    let mut node = [0; 17];
    node[0] = 2;
    node[1..].copy_from_slice(&file);
    let node = foks_proto::KvNodeId(node);
    let (entered, entry) = mpsc::channel();
    let (release, released) = mpsc::channel();
    environment
        .set_chunk_store(Arc::new(BackupReadGate {
            inner: store.clone(),
            entered,
            release: Mutex::new(released),
        }))
        .unwrap();
    let server = environment.start_server().unwrap();
    std::thread::scope(|scope| {
        let read = scope.spawn(|| {
            client.foks().read_user_kv_chunk(
                &host,
                &account.credential,
                &account.authenticated.verified,
                &account.authenticated.puks,
                node,
                0,
                bytes.len(),
            )
        });
        entry.recv_timeout(Duration::from_secs(5)).unwrap();
        server.shutdown().unwrap();
        let error = foks_server::collect_chunk_store(
            environment.database_path(),
            Default::default(),
            store.as_ref(),
            128,
            1024 * 1024,
            false,
        )
        .unwrap_err();
        assert!(error.to_string().contains("storage is in use"));
        release.send(()).unwrap();
        let _ = read.join().unwrap();
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match foks_server::collect_chunk_store(
            environment.database_path(),
            Default::default(),
            store.as_ref(),
            128,
            1024 * 1024,
            true,
        ) {
            Ok(report) => {
                assert_eq!(report.eligible_objects, 0);
                break;
            }
            Err(error) if std::time::Instant::now() < deadline => {
                assert!(error.to_string().contains("storage is in use"));
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("storage lease did not release: {error}"),
        }
    }
}

struct LostDeleteReply {
    inner: Arc<dyn ChunkStore>,
    lose: std::sync::atomic::AtomicBool,
}
impl ChunkStore for LostDeleteReply {
    fn namespace_lease(
        &self,
        exclusive: bool,
    ) -> foks_server::Result<Option<foks_server::chunks::ChunkNamespaceLease>> {
        self.inner.namespace_lease(exclusive)
    }
    fn locator(&self) -> foks_server::Result<String> {
        self.inner.locator()
    }
    fn read_namespace_marker(&self) -> foks_server::Result<Option<Vec<u8>>> {
        self.inner.read_namespace_marker()
    }
    fn create_namespace_marker_if_absent(&self, bytes: &[u8]) -> foks_server::Result<()> {
        self.inner.create_namespace_marker_if_absent(bytes)
    }
    fn namespace_is_empty(&self) -> foks_server::Result<bool> {
        self.inner.namespace_is_empty()
    }
    fn inventory_page(
        &self,
        cursor: Option<&str>,
        maximum: usize,
    ) -> foks_server::Result<foks_server::chunks::ChunkInventoryPage> {
        self.inner.inventory_page(cursor, maximum)
    }
    fn put_object(
        &self,
        object: &foks_server_db::ChunkObject,
        bytes: &[u8],
    ) -> foks_server::Result<()> {
        self.inner.put_object(object, bytes)
    }
    fn get_object(&self, object: &foks_server_db::ChunkObject) -> foks_server::Result<Vec<u8>> {
        self.inner.get_object(object)
    }
    fn supports_collection(&self) -> bool {
        self.inner.supports_collection()
    }
    fn delete_object(&self, object: &foks_server_db::ChunkObject) -> foks_server::Result<()> {
        self.inner.delete_object(object)?;
        if self.lose.swap(false, Ordering::SeqCst) {
            Err(foks_server::Error::Chunk("injected lost delete reply"))
        } else {
            Ok(())
        }
    }
}
#[test]
fn interrupted_delete_stays_charged_and_resumes_without_reusing_retired_key() {
    let environment = TestEnvironment::new().unwrap();
    environment.use_filesystem_chunks().unwrap();
    environment.start_server().unwrap().shutdown().unwrap();
    let inner = environment.filesystem_chunk_store().unwrap();
    let bytes = b"unreferenced canonical encrypted bytes";
    let digest = foks_server::chunks::chunk_digest(bytes);
    let mut db =
        foks_server_db::Database::open(environment.database_path(), Default::default()).unwrap();
    let old = db
        .reserve_chunk_object(&digest, bytes.len() as u64, &[1; 16], 1)
        .unwrap();
    inner.put_object(&old.object(), bytes).unwrap();
    db.settle_chunk_publication(&old, true).unwrap();
    drop(db);
    let store = LostDeleteReply {
        inner: inner.clone(),
        lose: std::sync::atomic::AtomicBool::new(true),
    };
    assert!(foks_server::collect_chunk_store(
        environment.database_path(),
        Default::default(),
        &store,
        128,
        1024 * 1024,
        false
    )
    .is_err());
    let usage = environment
        .read_database()
        .unwrap()
        .chunk_physical_usage()
        .unwrap();
    assert_eq!(
        (usage.objects, usage.deleting_bytes),
        (1, bytes.len() as u64)
    );
    let mut db =
        foks_server_db::Database::open(environment.database_path(), Default::default()).unwrap();
    let new = db
        .reserve_chunk_object(&digest, bytes.len() as u64, &[2; 16], 2)
        .unwrap();
    assert_ne!(old.object_key, new.object_key);
    inner.put_object(&new.object(), bytes).unwrap();
    db.settle_chunk_publication(&new, true).unwrap();
    drop(db);
    // A delayed old DELETE cannot touch the newly published incarnation.
    inner.delete_object(&old.object()).unwrap();
    assert_eq!(inner.get_object(&new.object()).unwrap(), bytes);
    // Limit to one object: the persisted deleting intent must be resumed first.
    let report = foks_server::collect_chunk_store(
        environment.database_path(),
        Default::default(),
        &store,
        1,
        1024 * 1024,
        false,
    )
    .unwrap();
    assert_eq!(
        (
            report.deleted_objects,
            report.usage.objects,
            report.usage.tombstones
        ),
        (1, 1, 1)
    );
    assert_eq!(inner.get_object(&new.object()).unwrap(), bytes);
}

#[test]
fn legacy_clone_collection_never_deletes_another_copy_live_or_stopped_reference() {
    let environment = TestEnvironment::new().unwrap();
    // Initialize the genuine v1 marker before attaching the store. Existing
    // namespaces cannot prove that older database copies do not exist.
    let initial = environment.start_server().unwrap();
    initial.shutdown().unwrap();
    let store = environment.filesystem_chunk_store().unwrap();
    let mut database =
        foks_server_db::Database::open(environment.database_path(), Default::default()).unwrap();
    let binding = database
        .prepare_chunk_storage_binding(&[0xa4; 16], &store.locator().unwrap(), false)
        .unwrap();
    let mut marker = b"FOKS-CHUNK-NAMESPACE\0\x01".to_vec();
    marker.extend_from_slice(&binding.namespace_id.unwrap());
    marker.extend_from_slice(&binding.host_id.unwrap());
    store.create_namespace_marker_if_absent(&marker).unwrap();
    database.activate_chunk_storage_binding(&binding).unwrap();
    drop(database);
    environment.set_chunk_store(store.clone()).unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "legacy-clone").unwrap();
    let host = client.probe_and_pin().unwrap().pinned;
    let account = client
        .create_account(&host, &TestAccountSpec::new("legacyclone", 0xb4))
        .unwrap();
    let bytes = vec![0x43; 8192];
    put(&client, &host, &account, "retained.bin", &bytes).unwrap();
    server.shutdown().unwrap();
    let clone = environment.root().join("stale-copy.sqlite");
    std::fs::copy(environment.database_path(), &clone).unwrap();
    let copied = rusqlite::Connection::open(&clone).unwrap();
    copied
        .execute_batch("PRAGMA foreign_keys=ON; DELETE FROM kv_file_uploads;")
        .unwrap();
    drop(copied);
    let server = environment.start_server().unwrap();
    let live = foks_server::collect_chunk_store(
        &clone,
        Default::default(),
        store.as_ref(),
        128,
        1024 * 1024,
        false,
    );
    assert!(
        live.is_err(),
        "physical namespace must exclude even another database's collector"
    );
    server.shutdown().unwrap();
    let report = foks_server::collect_chunk_store(
        &clone,
        Default::default(),
        store.as_ref(),
        128,
        1024 * 1024,
        false,
    )
    .unwrap();
    assert!(report.report_only);
    assert!(!report.backend_qualified);
    assert_eq!(report.deleted_objects, 0);
    assert_eq!(report.eligible_objects, 1);
    let reader = environment.read_database().unwrap();
    assert_eq!(reader.chunk_physical_usage().unwrap().objects, 1);
    drop(reader);
    let server = environment.start_server().unwrap();
    let fresh = TestClient::new(&environment, "legacy-clone-verify").unwrap();
    let fresh_host = fresh.probe_and_pin().unwrap().pinned;
    assert_eq!(fresh_host.host_id(), host.host_id());
    assert_files(&fresh, &fresh_host, &account, &[("retained.bin", &bytes)]);
    server.shutdown().unwrap();
}
