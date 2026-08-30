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
