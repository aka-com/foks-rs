use std::io::{Cursor, Write as _};
use std::sync::Arc;

use foks_client::{DeviceCredential, KvWriteOptions};
use foks_proto::{KvDirectoryVersion, KvGetResponse, KvListResponse, KvNode, KvUsage, Role};
use foks_rpc::{KvAuth, KvListCursor};
use foks_server_testkit::TestAccountSpec;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};

use crate::support::Fixture;

#[test]
pub(crate) fn go_style_kv_get_usage_directory_nodes_and_time_lists_work() {
    let fixture = Fixture::start("go-client-kv");
    let created = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("goclientkv", 0x5a))
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
    for (name, contents) in [
        ("first.txt", b"first".as_slice()),
        ("second.txt", b"second"),
    ] {
        session
            .put_file(
                root,
                name,
                &mut Cursor::new(contents),
                KvWriteOptions {
                    read_role: Role::OWNER,
                    write_role: Role::OWNER,
                    overwrite: false,
                    expected_version: None,
                },
            )
            .unwrap();
    }
    drop(session);

    let reader = fixture.server.read_database().unwrap();
    let stored = reader
        .kv_list(created.credential.uid.as_bytes(), &root, None, 10)
        .unwrap();
    let target = foks_proto::KvDirent::decode(&stored[0].exact).unwrap();
    assert!(target.creation_time > 0);
    let root_version = reader
        .kv_root(created.credential.uid.as_bytes())
        .unwrap()
        .unwrap()
        .version;
    let directory_version = reader
        .kv_directory(created.credential.uid.as_bytes(), &root)
        .unwrap()
        .unwrap()
        .version;
    let partial = foks_proto::KvPathVersionVector {
        root_version,
        directories: vec![KvDirectoryVersion {
            id: root,
            version: directory_version,
            entries: Vec::new(),
        }],
    };

    let mut tls = authenticated_stream(&fixture, &created.credential);
    tls.write_all(&foks_rpc::encode_kv_select_vhost_request(fixture.host().host_id()).unwrap())
        .unwrap();
    foks_rpc::read_void_response(&mut tls, foks_rpc::DEFAULT_MAX_FRAME_LENGTH, 0).unwrap();

    tls.write_all(
        &foks_rpc::encode_kv_get_request_at(
            KvAuth::User,
            Some(&partial),
            &root,
            &[(target.directory_version, target.name_mac)],
            2,
            1,
        )
        .unwrap(),
    )
    .unwrap();
    let response =
        foks_rpc::read_response(&mut tls, foks_rpc::DEFAULT_MAX_FRAME_LENGTH, 1).unwrap();
    let response = KvGetResponse::decode(&response).unwrap();
    assert_eq!(response.dirent.id, target.id);
    assert_eq!(response.dirent.creation_time, 0);
    assert!(matches!(response.node, Some(KvNode::SmallFile(_))));

    tls.write_all(&foks_rpc::encode_kv_usage_request_at(KvAuth::User, 2).unwrap())
        .unwrap();
    let response =
        foks_rpc::read_response(&mut tls, foks_rpc::DEFAULT_MAX_FRAME_LENGTH, 2).unwrap();
    let usage = KvUsage::decode(&response).unwrap();
    assert_eq!(usage.small.number, 2);
    assert!(usage.small.bytes > 0);

    let mut root_node = [0; 17];
    root_node[0] = 1;
    root_node[1..].copy_from_slice(&root);
    tls.write_all(
        &foks_rpc::encode_kv_get_node_request_at(KvAuth::User, foks_proto::KvNodeId(root_node), 3)
            .unwrap(),
    )
    .unwrap();
    let response =
        foks_rpc::read_response(&mut tls, foks_rpc::DEFAULT_MAX_FRAME_LENGTH, 3).unwrap();
    assert!(matches!(
        KvNode::decode(&response).unwrap(),
        KvNode::Directory(_)
    ));

    tls.write_all(
        &foks_rpc::encode_kv_list_request_at(
            KvAuth::User,
            &root,
            KvListCursor::Time(target.creation_time),
            0,
            false,
            4,
        )
        .unwrap(),
    )
    .unwrap();
    let response =
        foks_rpc::read_response(&mut tls, foks_rpc::DEFAULT_MAX_FRAME_LENGTH, 4).unwrap();
    let listing = KvListResponse::decode(&response).unwrap();
    assert!(!listing.entries.is_empty());
    assert!(listing
        .entries
        .iter()
        .all(|entry| entry.creation_time >= target.creation_time));

    // Server-side ctime assignment must not make an otherwise exact replay
    // conflict after a response is lost.
    fixture.environment.advance_clock(1);
    tls.write_all(&foks_rpc::encode_kv_put_request_at(KvAuth::User, None, &[target], 5).unwrap())
        .unwrap();
    foks_rpc::read_void_response(&mut tls, foks_rpc::DEFAULT_MAX_FRAME_LENGTH, 5).unwrap();
}

/// go-foks main reports an identical replay of `kvMkdir` and
/// `kvPutSmallFileOrSymlink` with `wasReplay` and refuses a reused ID with
/// `KV_RACE_ERROR`. Each response must match upstream's frame byte for byte,
/// and a replayed small file must not be charged against usage again.
#[test]
pub(crate) fn go_main_kv_creation_replays_match_upstream_frames() {
    let fixture = Fixture::start("go-main-kv-replay");
    let created = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("gomainreplay", 0x5b))
        .unwrap();
    let uid = created.credential.uid.as_bytes().to_vec();
    let root = created.kv_projection[0].root_directory_id;
    let key = foks_proto::KvDirectory::decode(
        &fixture
            .server
            .read_database()
            .unwrap()
            .kv_directory(&uid, &root)
            .unwrap()
            .unwrap()
            .exact,
    )
    .unwrap()
    .key;

    // Every exchange runs on a fresh connection at sequence 1, the sequence
    // the upstream fixtures were generated with.
    let exchange = |request: &[u8]| -> Vec<u8> {
        let mut tls = authenticated_stream(&fixture, &created.credential);
        tls.write_all(&foks_rpc::encode_kv_select_vhost_request(fixture.host().host_id()).unwrap())
            .unwrap();
        foks_rpc::read_void_response(&mut tls, foks_rpc::DEFAULT_MAX_FRAME_LENGTH, 0).unwrap();
        tls.write_all(request).unwrap();
        foks_rpc::read_frame(&mut tls, foks_rpc::DEFAULT_MAX_FRAME_LENGTH).unwrap()
    };
    let upstream = |name: &str| -> Vec<u8> {
        let bytes = std::fs::read(format!(
            "{}/../foks-snowpack/tests/fixtures/foks-main/kv/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        foks_rpc::read_frame(&mut bytes.as_slice(), foks_rpc::DEFAULT_MAX_FRAME_LENGTH).unwrap()
    };
    let usage = || -> KvUsage {
        let mut tls = authenticated_stream(&fixture, &created.credential);
        tls.write_all(&foks_rpc::encode_kv_select_vhost_request(fixture.host().host_id()).unwrap())
            .unwrap();
        foks_rpc::read_void_response(&mut tls, foks_rpc::DEFAULT_MAX_FRAME_LENGTH, 0).unwrap();
        tls.write_all(&foks_rpc::encode_kv_usage_request_at(KvAuth::User, 1).unwrap())
            .unwrap();
        KvUsage::decode(
            &foks_rpc::read_response(&mut tls, foks_rpc::DEFAULT_MAX_FRAME_LENGTH, 1).unwrap(),
        )
        .unwrap()
    };

    let directory = foks_proto::KvDirectory {
        id: [0x6d; 16],
        version: 1,
        key,
        seed_ciphertext: vec![0x11; 48],
        write_role: Role::OWNER,
        status: foks_proto::KvDirectoryStatus::Active,
    };
    let mkdir = |directory: &foks_proto::KvDirectory| {
        foks_rpc::encode_kv_mkdir_request_at(KvAuth::User, None, directory, 1).unwrap()
    };
    assert_eq!(
        exchange(&mkdir(&directory)),
        upstream("mkdir-response-fresh.frame")
    );
    assert_eq!(
        exchange(&mkdir(&directory)),
        upstream("mkdir-response-replay.frame")
    );
    let reused = foks_proto::KvDirectory {
        seed_ciphertext: vec![0x12; 48],
        ..directory.clone()
    };
    assert_eq!(
        exchange(&mkdir(&reused)),
        upstream("mkdir-race-response.frame")
    );

    let mut node = [0x7a; 17];
    node[0] = foks_proto::KvNodeType::SmallFile as u8;
    let node = foks_proto::KvNodeId(node);
    let boxed = foks_proto::KvSmallFileBox {
        key,
        ciphertext: vec![0x21; 40],
    };
    let put_small = |boxed: &foks_proto::KvSmallFileBox| {
        foks_rpc::encode_kv_put_small_file_or_symlink_request_at(KvAuth::User, node, boxed, 1)
            .unwrap()
    };
    let before = usage();
    assert_eq!(
        exchange(&put_small(&boxed)),
        upstream("put-small-response-fresh.frame")
    );
    let charged = usage();
    assert_eq!(charged.small.number, before.small.number + 1);
    assert_eq!(charged.small.bytes, before.small.bytes + 40);
    assert_eq!(
        exchange(&put_small(&boxed)),
        upstream("put-small-response-replay.frame")
    );
    assert_eq!(usage(), charged);
    let reused = foks_proto::KvSmallFileBox {
        ciphertext: vec![0x22; 40],
        ..boxed.clone()
    };
    assert_eq!(
        exchange(&put_small(&reused)),
        upstream("put-small-race-response.frame")
    );
    assert_eq!(usage(), charged);

    // The Rust client reads the struct results and reports fresh creations.
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
    let options = KvWriteOptions {
        read_role: Role::OWNER,
        write_role: Role::OWNER,
        overwrite: false,
        expected_version: None,
    };
    assert!(
        !session
            .mkdir(root, "dir", options)
            .unwrap()
            .content_was_replay
    );
    assert!(
        !session
            .put_symlink(root, "link", "dir", options)
            .unwrap()
            .content_was_replay
    );
    assert!(
        !session
            .put_file(root, "file", &mut Cursor::new(b"small"), options)
            .unwrap()
            .content_was_replay
    );
}

fn authenticated_stream(
    fixture: &Fixture,
    credential: &DeviceCredential,
) -> rustls::StreamOwned<rustls::ClientConnection, std::net::TcpStream> {
    let certificates = credential
        .certificate_chain
        .iter()
        .cloned()
        .map(CertificateDer::from)
        .collect();
    let key = foks_crypto::device_signing_key_pkcs8(&credential.seed).unwrap();
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.as_slice()));
    let mut roots = rustls::RootCertStore::empty();
    for certificate in fixture.host().tls_ca_certificates() {
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
    .with_client_auth_cert(certificates, key.clone_key())
    .unwrap();
    let tcp = std::net::TcpStream::connect(fixture.server.addresses().authenticated).unwrap();
    let connection = rustls::ClientConnection::new(
        Arc::new(config),
        ServerName::try_from("localhost".to_owned()).unwrap(),
    )
    .unwrap();
    rustls::StreamOwned::new(connection, tcp)
}
