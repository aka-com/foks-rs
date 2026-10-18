//! Unit fixtures use the real R3 importer. Only the TLS integration test proves
//! peer possession; direct operation tests inject a private retained peer solely
//! to exercise deterministic transaction/freshness boundaries.
use super::*;
use crate::{
    keys::DirectoryKeyProvider,
    restricted_storage::{CaptureLimits, RestrictedScanner, ScanControl},
};
use foks_proto::{
    KvDirectory, KvDirectoryStatus, KvDirent, KvExtendedDirent, KvListResponse, KvNodeId,
    KvSmallFileBox, Role, RoleAndGeneration, SecretBox,
};
use rusqlite::{params, Connection};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

pub(super) const UID: [u8; 33] = [foks_proto::ENTITY_USER; 33];
pub(super) const PARENT: [u8; 16] = [0x51; 16];
pub(super) const NOW: u64 = 1_750_000_000_000_000;
pub(super) struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    pub(super) database: PathBuf,
    keys: PathBuf,
    pub(super) certificate: Vec<u8>,
    pub(super) device: [u8; 33],
    pub(super) client: Arc<rustls::ClientConfig>,
    next: AtomicUsize,
}
impl Fixture {
    pub(super) fn new(head: bool, role: Role, entries: usize) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let database = root.join("source.sqlite");
        let keys = root.join("source-keys");
        let provider = DirectoryKeyProvider::open(&keys, [0x35; 32]).unwrap();
        let mut writer = foks_server_db::Database::open(&database, Default::default()).unwrap();
        crate::host::load_or_bootstrap(
            &mut writer,
            &provider,
            &crate::host::BootstrapInput {
                canonical_name: "localhost".into(),
                endpoints: crate::host::BootstrapEndpoints {
                    probe: "localhost:4430".into(),
                    public_services: "localhost:4431".into(),
                    authenticated: "localhost:4432".into(),
                },
                ttl_seconds: 60,
                now_microseconds: 1000,
            },
        )
        .unwrap();
        let tls = crate::pki::build_host_tls(&provider, "localhost").unwrap();
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
        let public: [u8; 32] = key.public_key_raw().try_into().unwrap();
        let mut device = [0; 33];
        device[0] = foks_proto::ENTITY_DEVICE;
        device[1..].copy_from_slice(&public);
        let certificate = crate::pki::issue_bounded_device_certificate(
            &provider,
            "localhost",
            public,
            &[7],
            1_600_000_000_000_000,
            2_000_000_000_000_000,
        )
        .unwrap();
        let client = Arc::new(
            rustls::ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::aws_lc_rs::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(tls.delegated_ca)
            .with_client_auth_cert(
                vec![rustls::pki_types::CertificateDer::from(certificate.clone())],
                rustls::pki_types::PrivateKeyDer::Pkcs8(
                    rustls::pki_types::PrivatePkcs8KeyDer::from(key.serialize_der()),
                ),
            )
            .unwrap(),
        );
        drop(writer);
        let db = Connection::open(&database).unwrap();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        db.execute_batch("BEGIN IMMEDIATE").unwrap();
        db.execute(
            "INSERT INTO names(normalized_name,reservation_sequence,uid) VALUES(?1,1,?2)",
            params![b"alice".as_slice(), UID.as_slice()],
        )
        .unwrap();
        db.execute("INSERT INTO users(uid,normalized_name,username_utf8,username_sequence,username_commitment_key,created_at) VALUES(?1,?2,?2,1,zeroblob(16),0)",params![UID.as_slice(),b"alice".as_slice()]).unwrap();
        db.execute("INSERT INTO devices(device_id,uid,active,role_type,visibility,hepk_fingerprint,self_token,exact_hepk,exact_name) VALUES(?1,?2,1,?3,?4,zeroblob(32),zeroblob(17),X'00',X'00')",params![device.as_slice(),UID.as_slice(),role.protocol_value() as i64,role.visibility().unwrap_or(0)]).unwrap();
        db.execute("INSERT INTO shared_keys(uid,role_type,visibility,generation,verify_key,exact_hepk) VALUES(?1,?2,?3,1,?4,X'00')",params![UID.as_slice(),role.protocol_value() as i64,role.visibility().unwrap_or(0),[8u8;33].as_slice()]).unwrap();
        db.execute("INSERT INTO issued_certificates(serial,uid,device_id,credential_id,not_before,not_after,exact_certificate) VALUES(X'07',?1,?2,?2,1600000000000000,2000000000000000,?3)",params![UID.as_slice(),device.as_slice(),certificate.as_slice()]).unwrap();
        if head {
            db.execute("INSERT INTO user_chain_links(uid,seqno,link_hash,exact_link,root_epoch) VALUES(?1,1,?2,X'00',1)",params![UID.as_slice(),[9u8;32].as_slice()]).unwrap();
            db.execute(
                "INSERT INTO user_chain_heads(uid,seqno,link_hash) VALUES(?1,1,?2)",
                params![UID.as_slice(), [9u8; 32].as_slice()],
            )
            .unwrap();
            db.execute(
                "INSERT INTO tree_locations(uid,seqno,location) VALUES(?1,1,?2)",
                params![UID.as_slice(), [10u8; 32].as_slice()],
            )
            .unwrap();
        }
        db.execute("INSERT INTO kv_namespaces(namespace_id,party_kind,host_id) SELECT ?1,1,host_id FROM host_metadata WHERE singleton=1",[UID.as_slice()]).unwrap();
        let parent = KvDirectory {
            id: PARENT,
            version: 1,
            key: RoleAndGeneration {
                role,
                generation: 1,
            },
            seed_ciphertext: vec![1; 48],
            write_role: Role::OWNER,
            status: KvDirectoryStatus::Active,
        }
        .encode()
        .unwrap();
        db.execute("INSERT INTO kv_directories(uid,directory_id,version,key_role,key_visibility,key_generation,status,exact_directory) VALUES(?1,?2,1,?3,?4,1,0,?5)",params![UID.as_slice(),PARENT.as_slice(),role.protocol_value() as i64,role.visibility().unwrap_or(0),parent]).unwrap();
        db.execute(
            "INSERT INTO kv_directory_heads VALUES(?1,?2,1)",
            params![UID.as_slice(), PARENT.as_slice()],
        )
        .unwrap();
        for n in 0..entries {
            let mut id = [0; 16];
            id[8..].copy_from_slice(&(n as u64).to_be_bytes());
            let mut node = [3; 17];
            node[1..].copy_from_slice(&id);
            let mut mac = [0; 32];
            mac[16..].copy_from_slice(&id);
            let small = KvSmallFileBox {
                key: RoleAndGeneration {
                    role,
                    generation: 1,
                },
                ciphertext: vec![n as u8; 32],
            }
            .encode()
            .unwrap();
            db.execute("INSERT INTO kv_nodes(uid,node_id,node_type,exact_node,ciphertext_bytes) VALUES(?1,?2,3,?3,32)",params![UID.as_slice(),node.as_slice(),small]).unwrap();
            let entry = KvDirent::new(
                PARENT,
                id,
                KvNodeId(node),
                1,
                1,
                role,
                mac,
                SecretBox {
                    nonce: [7; 16],
                    ciphertext: vec![8; 16],
                },
                KvDirectoryStatus::Active,
                [9; 32],
                n as u64,
            )
            .unwrap();
            db.execute("INSERT INTO kv_dirents(uid,parent_id,dirent_id,version,directory_version,node_id,name_mac,creation_time,exact_dirent) VALUES(?1,?2,?3,1,1,?4,?5,?6,?7)",params![UID.as_slice(),PARENT.as_slice(),id.as_slice(),node.as_slice(),mac.as_slice(),n as i64,entry.encode().unwrap()]).unwrap();
            db.execute(
                "INSERT INTO kv_dirent_heads VALUES(?1,?2,?3,1)",
                params![UID.as_slice(), PARENT.as_slice(), id.as_slice()],
            )
            .unwrap();
        }
        db.execute_batch("COMMIT").unwrap();
        drop(db);
        drop(provider);
        Self {
            _directory: directory,
            root,
            database,
            keys,
            certificate,
            device,
            client,
            next: AtomicUsize::new(0),
        }
    }
    pub(super) fn backup(&self) -> crate::BackupArtifacts {
        crate::backup_standalone_installation(
            &self.database,
            &self.keys,
            [0x35; 32],
            self.root.join(format!(
                "backup-{}",
                self.next.fetch_add(1, Ordering::SeqCst)
            )),
            Default::default(),
        )
        .unwrap()
    }
    pub(super) fn owner(&self, pools: &Arc<Pools>) -> bootstrap::NativeOwner {
        let artifacts = self.backup();
        let job = control();
        let mut scanner =
            RestrictedScanner::capture_finalized_backup(&artifacts, CaptureLimits::default(), &job)
                .unwrap();
        while !scanner.advance(&job).unwrap().complete {}
        let source = scanner.finish(&job).unwrap();
        let imported = super::super::import_personal_list(
            source,
            Zeroizing::new([0x35; 32]),
            Default::default(),
            &job,
        )
        .unwrap();
        bootstrap::consume(imported, &job, pools).unwrap().0
    }
    pub(super) fn peer(&self, pools: &Arc<Pools>) -> Arc<SessionPeer> {
        // Private deterministic-test input, never used by the actual TLS listener.
        let memory = pools.admitted_request(None).unwrap();
        assert!(foks_snowpack::AllocationBudget::reserve(
            memory.as_ref(),
            self.certificate.len() + operation::arc_bytes::<SessionPeer>()
        ));
        Arc::new(SessionPeer {
            der: self.certificate.clone(),
            _memory: memory,
        })
    }
    pub(super) fn oracle(&self, count: usize, load: bool) -> Vec<u8> {
        let reader =
            foks_server_db::ReadDatabase::open(&self.database, Default::default()).unwrap();
        let stored = reader.kv_list(&UID, &PARENT, None, count).unwrap();
        let final_page = stored.len() < count;
        let mut entries = Vec::new();
        let mut extended = Vec::new();
        for (position, row) in stored.into_iter().enumerate() {
            entries.push(KvDirent::decode(&row.exact).unwrap());
            if load {
                let node = reader.kv_node(&UID, &row.node_id).unwrap().unwrap();
                extended.push(KvExtendedDirent {
                    position: position as u64,
                    small_file: KvSmallFileBox::decode(&node.exact).unwrap(),
                });
            }
        }
        KvListResponse::new(entries, final_page, extended)
            .unwrap()
            .encoded()
            .to_vec()
    }
}
pub(super) fn control() -> ScanControl {
    ScanControl::new(
        foks_local_deadline::Deadline::capture(Instant::now() + Duration::from_secs(300)).unwrap(),
        Arc::new(AtomicBool::new(false)),
    )
}
pub(super) fn call(frame: Vec<u8>) -> foks_rpc::DecodedCall {
    foks_rpc::read_call(&mut frame.as_slice(), foks_rpc::DEFAULT_MAX_FRAME_LENGTH).unwrap()
}
pub(super) fn list(parent: [u8; 16], cursor: foks_rpc::KvListCursor) -> foks_rpc::DecodedCall {
    call(
        foks_rpc::encode_kv_list_request(foks_rpc::KvAuth::User, &parent, cursor, 4096, true)
            .unwrap(),
    )
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn genuine_import_and_mutual_tls_match_native_list_bytes_and_reuse_after_status() {
    let fixture = Fixture::new(true, Role::OWNER, 3);
    let expected = fixture.oracle(4096, true);
    let server = start_native_personal_list_qualification(
        &fixture.backup(),
        Zeroizing::new([0x35; 32]),
        Default::default(),
        &control(),
        Default::default(),
    )
    .await
    .unwrap();
    let address = server.address();
    let canonical_name = server.canonical_name().to_owned();
    assert_eq!(canonical_name, "localhost");
    let client = fixture.client.clone();
    let result = tokio::task::spawn_blocking(move || {
        use std::io::Write as _;
        let socket = std::net::TcpStream::connect(address).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        let connection = rustls::ClientConnection::new(
            client,
            rustls::pki_types::ServerName::try_from(canonical_name).unwrap(),
        )
        .unwrap();
        let mut stream = rustls::StreamOwned::new(connection, socket);
        for (sequence, argument) in [
            (
                1,
                foks_snowpack::encode(&foks_snowpack::Value::Null).unwrap(),
            ),
            (
                2,
                list(PARENT, foks_rpc::KvListCursor::None)
                    .argument()
                    .to_vec(),
            ),
        ] {
            let request = foks_rpc::encode_call(0x8ee37b6b, 14, &argument, sequence).unwrap();
            stream.write_all(&request).unwrap();
            stream.flush().unwrap();
            let content =
                foks_rpc::read_frame(&mut stream, foks_rpc::DEFAULT_MAX_FRAME_LENGTH).unwrap();
            if sequence == 1 {
                let expected = foks_rpc::encode_status_response_at(
                    &foks_rpc::RpcStatus::BadArguments("KV argument is not a struct".into()),
                    sequence,
                )
                .unwrap();
                assert_eq!(
                    content,
                    foks_rpc::read_frame(
                        &mut expected.as_slice(),
                        foks_rpc::DEFAULT_MAX_FRAME_LENGTH
                    )
                    .unwrap()
                );
            } else {
                assert_eq!(
                    foks_rpc::decode_response(&content, sequence).unwrap(),
                    expected
                );
            }
        }
    })
    .await;
    assert!(result.is_ok());
    let report = server.shutdown().await;
    assert_eq!(report.completed_requests, 2);
    assert!(!report.stopped_after_failure);
    assert!(report.maximum_sql_charged <= 1492);
    assert!(report.maximum_request_micros <= 15_000_000);
    assert_eq!(report.sqlite_version, "3.53.2");
    assert_eq!(&report.compiled_profile[..4], b"FKNP");
    assert_eq!(report.shared_connection_programs, 5);
}
