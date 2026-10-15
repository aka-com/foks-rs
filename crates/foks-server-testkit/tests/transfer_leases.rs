//! Object-store latency must not monopolize a tenant's sole database reader.
use foks_server::chunks::ChunkStore;
use foks_server_testkit::{TestAccountSpec, TestClient, TestEnvironment, TestProfile};
use std::{
    io::Cursor,
    sync::{
        atomic::{AtomicU8, Ordering},
        mpsc, Arc, Condvar, Mutex,
    },
    time::Duration,
};

struct Gated {
    inner: Arc<dyn ChunkStore>,
    mode: AtomicU8,
    entered: mpsc::Sender<()>,
    released: Mutex<bool>,
    release: Condvar,
}
impl Gated {
    fn wait(&self, mode: u8) -> foks_server::Result<()> {
        if self.mode.load(Ordering::SeqCst) != mode {
            return Ok(());
        }
        self.entered.send(()).unwrap();
        let (_guard, timeout) = self
            .release
            .wait_timeout_while(
                self.released.lock().unwrap(),
                Duration::from_secs(5),
                |done| !*done,
            )
            .unwrap();
        if timeout.timed_out() {
            return Err(foks_server::Error::Chunk("test transfer gate timed out"));
        }
        Ok(())
    }
}
impl ChunkStore for Gated {
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
        self.wait(1)?;
        self.inner.put_object(object, bytes)
    }
    fn get_object(&self, object: &foks_server_db::ChunkObject) -> foks_server::Result<Vec<u8>> {
        self.wait(2)?;
        self.inner.get_object(object)
    }
}
#[test]
fn object_upload_and_download_release_the_only_reader() {
    let environment = TestEnvironment::with_profile(TestProfile::RealtimeSingleReader).unwrap();
    let (entered, entry) = mpsc::channel();
    let store = Arc::new(Gated {
        inner: environment.filesystem_chunk_store().unwrap(),
        mode: AtomicU8::new(0),
        entered,
        released: Mutex::new(false),
        release: Condvar::new(),
    });
    environment.set_chunk_store(store.clone()).unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "lease-writer").unwrap();
    let host = client.probe_and_pin().unwrap().pinned;
    let account = client
        .create_account(&host, &TestAccountSpec::new("transferlease", 0x87))
        .unwrap();
    let other = TestClient::new(&environment, "lease-reader").unwrap();
    for mode in [1, 2] {
        store.mode.store(mode, Ordering::SeqCst);
        *store.released.lock().unwrap() = false;
        std::thread::scope(|scope| {
            let transfer = scope.spawn(|| {
                if mode == 1 {
                    let mut protected = client.open_protected_store().unwrap();
                    let mut session = client
                        .foks()
                        .user_kv_write_session(
                            &host,
                            &account.credential,
                            &account.authenticated.verified,
                            &account.authenticated.puks,
                            client.soft_state_path(),
                            &mut protected,
                        )
                        .unwrap();
                    session
                        .put_file(
                            account.kv_projection[0].root_directory_id,
                            "gated.bin",
                            &mut Cursor::new(vec![0x34; 8192]),
                            foks_client::KvWriteOptions {
                                read_role: foks_proto::Role::OWNER,
                                write_role: foks_proto::Role::OWNER,
                                overwrite: false,
                                expected_version: None,
                            },
                        )
                        .unwrap();
                } else {
                    other
                        .foks()
                        .sync_user_kv(
                            &host,
                            &account.credential,
                            &account.authenticated.verified,
                            &account.authenticated.puks,
                            other.soft_state_path(),
                        )
                        .unwrap();
                }
            });
            entry.recv_timeout(Duration::from_secs(3)).unwrap();
            // Uses mTLS authentication and an authenticated KV read while the
            // sole connection would be unavailable if the transfer kept its lease.
            let usage = client.foks().kv_usage(&host, &account.credential, None);
            *store.released.lock().unwrap() = true;
            store.release.notify_all();
            assert!(
                usage.is_ok(),
                "credential request was blocked by object transfer: {usage:?}"
            );
            transfer.join().unwrap();
        });
    }
    server.shutdown().unwrap();
}

#[test]
fn maximum_size_upload_fits_minimum_working_allowance() {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
    use std::io::Write;
    // Default is the smallest accepted working allowance. Exercise actual TLS,
    // parser, authorization, external-store preflight and metadata publication.
    let environment = TestEnvironment::new().unwrap();
    environment.use_filesystem_chunks().unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "maxchunk").unwrap();
    let host = client.probe_and_pin().unwrap().pinned;
    let account = client
        .create_account(&host, &TestAccountSpec::new("maximumchunk", 0x88))
        .unwrap();
    let certificates = account
        .credential
        .certificate_chain
        .iter()
        .cloned()
        .map(CertificateDer::from)
        .collect();
    let key = foks_crypto::device_signing_key_pkcs8(&account.credential.seed).unwrap();
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.to_vec()));
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
    .with_client_auth_cert(certificates, key)
    .unwrap();
    let tcp = std::net::TcpStream::connect(server.addresses().authenticated).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    tcp.set_write_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let connection = rustls::ClientConnection::new(
        Arc::new(config),
        ServerName::try_from("localhost".to_owned()).unwrap(),
    )
    .unwrap();
    let mut tls = rustls::StreamOwned::new(connection, tcp);
    tls.write_all(&foks_rpc::encode_kv_select_vhost_request(host.host_id()).unwrap())
        .unwrap();
    foks_rpc::read_void_response(&mut tls, foks_rpc::DEFAULT_MAX_FRAME_LENGTH, 0).unwrap();
    let chunk = foks_proto::KvUploadChunk {
        ciphertext: vec![0x79; 9 * 1024 * 1024],
        offset: 0,
        final_upload: None,
    };
    let metadata = foks_proto::KvLargeFileMetadata {
        key: foks_proto::RoleAndGeneration {
            role: foks_proto::Role::OWNER,
            generation: 1,
        },
        key_seed: foks_proto::SecretBox {
            nonce: [0x61; 16],
            ciphertext: vec![0x62; 48],
        },
        version: 1,
        custom_metadata: None,
    };
    tls.write_all(
        &foks_rpc::encode_kv_file_upload_init_request_at(
            foks_rpc::KvAuth::User,
            [0x19; 16],
            &metadata,
            &chunk,
            1,
        )
        .unwrap(),
    )
    .unwrap();
    foks_rpc::read_void_response(&mut tls, foks_rpc::DEFAULT_MAX_FRAME_LENGTH, 1).unwrap();
    assert!(client
        .foks()
        .kv_usage(&host, &account.credential, None)
        .is_ok());
    drop(tls);
    server.shutdown().unwrap();
}
