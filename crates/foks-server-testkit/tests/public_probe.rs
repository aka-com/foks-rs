use std::time::Duration;
use std::{io::Write as _, net::TcpStream, sync::Arc};

use foks_client::{FoksClient, ProbeTarget};
use foks_client_db::Acceptance;
use foks_proto::{InviteCode, SecretSeed};
use foks_server_testkit::IsolatedTestServer;

#[test]
fn public_client_verifies_and_pins_the_real_tls_probe() {
    let server = IsolatedTestServer::start().unwrap();
    let mut client = FoksClient::with_roots(server.probe_roots());
    client.set_timeout(Duration::from_secs(5));
    let target =
        ProbeTarget::parse(&format!("localhost:{}", server.addresses().probe.port())).unwrap();
    let database = server.root().join("client-hard-state.sqlite");
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let client_tls = Arc::new(
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(server.probe_roots())
            .with_no_client_auth(),
    );
    let tcp = TcpStream::connect(server.addresses().probe).unwrap();
    let connection = rustls::ClientConnection::new(
        client_tls,
        rustls::pki_types::ServerName::try_from("localhost").unwrap(),
    )
    .unwrap();
    let mut tls = rustls::StreamOwned::new(connection, tcp);
    if let Err(error) =
        tls.write_all(&foks_rpc::encode_probe_request("localhost", 0, None).unwrap())
    {
        std::thread::sleep(Duration::from_millis(100));
        panic!("probe TLS write failed: {error}");
    }
    tls.flush().unwrap();
    let content = foks_rpc::read_frame(&mut tls, foks_rpc::DEFAULT_MAX_FRAME_LENGTH).unwrap();
    foks_rpc::decode_probe_response(&content, 0).unwrap();

    let first = client.probe_and_pin(&target, &database).unwrap();
    let second = client.probe_and_pin(&target, &database).unwrap();
    assert_eq!(first.acceptance, Acceptance::Inserted);
    assert_eq!(second.acceptance, Acceptance::Unchanged);
    assert_eq!(
        first.verified.snapshot.host_id(),
        second.verified.snapshot.host_id()
    );
    assert_eq!(
        first.verified.public_zone.services.probe,
        format!("localhost:{}", server.addresses().probe.port())
    );
    assert_eq!(
        first.verified.public_zone.services.registration,
        format!("localhost:{}", server.addresses().public_services.port())
    );
    assert_eq!(
        first.verified.public_zone.services.user,
        format!("localhost:{}", server.addresses().authenticated.port())
    );
    let reservation = client
        .reserve_username(&first.pinned, "fixtureuser")
        .unwrap();
    assert_eq!(reservation.sequence, 1);
    assert!(client
        .reserve_username(&first.pinned, "fixtureuser")
        .is_err());
    let (acceptance, advanced) = client.advance_merkle_root(&first.pinned).unwrap();
    assert_eq!(acceptance, Acceptance::Unchanged);
    assert_eq!(advanced.root().epoch, 1);

    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let service_tls = Arc::new(
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(server.service_roots())
            .with_no_client_auth(),
    );
    let tcp = TcpStream::connect(server.addresses().public_services).unwrap();
    let connection = rustls::ClientConnection::new(
        service_tls,
        rustls::pki_types::ServerName::try_from("localhost").unwrap(),
    )
    .unwrap();
    let mut tls = rustls::StreamOwned::new(connection, tcp);
    tls.write_all(&foks_rpc::encode_merkle_select_vhost_request(first.pinned.host_id()).unwrap())
        .unwrap();
    tls.flush().unwrap();
    foks_rpc::read_void_response(&mut tls, foks_rpc::DEFAULT_MAX_FRAME_LENGTH, 0).unwrap();
    tls.write_all(
        &foks_rpc::encode_get_historical_merkle_roots_request(
            first.pinned.host_id(),
            &[1],
            &[1],
            1,
        )
        .unwrap(),
    )
    .unwrap();
    tls.flush().unwrap();
    let historical =
        foks_rpc::read_response(&mut tls, foks_rpc::DEFAULT_MAX_FRAME_LENGTH, 1).unwrap();
    let historical = foks_proto::HistoricalMerkleRoots::decode(&historical).unwrap();
    assert_eq!(historical.roots.as_slice(), [advanced.root().clone()]);
    assert_eq!(
        historical.hashes.as_slice(),
        [advanced.snapshot().root_hash()]
    );

    let mut protected = foks_client::EncryptedFileMutationStore::open(
        server.root().join("client-protected"),
        zeroize::Zeroizing::new([0x61; 32]),
    )
    .unwrap();
    let created = client.create_software_account(
        &first.pinned,
        foks_client::SoftwareAccountRequest {
            username_utf8: "signupuser".to_owned(),
            device_name: "signup device".to_owned(),
            invite_code: InviteCode::Empty,
            email: "signup@example.test".to_owned(),
        },
        foks_client::SoftwareAccountSecrets::new(
            SecretSeed::new([0x71; 32]),
            SecretSeed::new([0x72; 32]),
            [0x73; 17],
        ),
        &server.root().join("client-soft-state.sqlite"),
        &mut protected,
    );
    assert!(
        created.is_err(),
        "personal KV root creation is not implemented yet"
    );
    let (acceptance, advanced) = client.advance_merkle_root(&first.pinned).unwrap();
    assert_eq!(acceptance, Acceptance::Unchanged);
    assert_eq!(advanced.root().epoch, 2);
    let puk = foks_crypto::derive_shared_verify_key(
        &SecretSeed::new([0x72; 32]),
        foks_proto::ENTITY_PUK_VERIFY,
    )
    .unwrap();
    let mut uid = puk.as_bytes().to_vec();
    uid[0] = foks_proto::ENTITY_USER;
    let uid_entity = foks_proto::EntityId::from_bytes(uid.clone()).unwrap();
    let device_seed = SecretSeed::new([0x71; 32]);
    let certificate_chain = client
        .fetch_device_certificate_chain(&first.pinned, &uid_entity, &device_seed)
        .unwrap();
    assert_eq!(certificate_chain.len(), 1);
    let credential = foks_client::DeviceCredential {
        uid: uid_entity,
        seed: device_seed,
        certificate_chain,
    };
    let authenticated = client
        .authenticate_and_pin(&first.pinned, &credential)
        .unwrap();
    assert_eq!(authenticated.verified.chain_seqno(), 1);
    assert_eq!(authenticated.verified.username(), b"signupuser");
    assert_eq!(authenticated.puks.len(), 1);
    assert_eq!(authenticated.puks[0].seed.as_slice(), &[0x72; 32]);
    let refreshed = client
        .authenticate_and_pin(&first.pinned, &credential)
        .unwrap();
    assert_eq!(refreshed.verified, authenticated.verified);
    assert_eq!(refreshed.puks[0].seed.as_slice(), &[0x72; 32]);

    assert!(client
        .create_software_account(
            &first.pinned,
            foks_client::SoftwareAccountRequest {
                username_utf8: "otheruser".to_owned(),
                device_name: "other device".to_owned(),
                invite_code: InviteCode::Empty,
                email: "other@example.test".to_owned(),
            },
            foks_client::SoftwareAccountSecrets::new(
                SecretSeed::new([0x81; 32]),
                SecretSeed::new([0x82; 32]),
                [0x83; 17],
            ),
            &server.root().join("other-client-soft-state.sqlite"),
            &mut protected,
        )
        .is_err());
    let other_puk = foks_crypto::derive_shared_verify_key(
        &SecretSeed::new([0x82; 32]),
        foks_proto::ENTITY_PUK_VERIFY,
    )
    .unwrap();
    let mut other_uid = other_puk.as_bytes().to_vec();
    other_uid[0] = foks_proto::ENTITY_USER;
    let other_uid = foks_proto::EntityId::from_bytes(other_uid).unwrap();
    let other_seed = SecretSeed::new([0x81; 32]);
    let other_certificates = client
        .fetch_device_certificate_chain(&first.pinned, &other_uid, &other_seed)
        .unwrap();
    let unbound = foks_client::DeviceCredential {
        uid: credential.uid.clone(),
        seed: other_seed,
        certificate_chain: other_certificates,
    };
    assert!(matches!(
        client.authenticate_and_pin(&first.pinned, &unbound),
        Err(foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
            code: 1013,
            ..
        }))
    ));
    let after_other_signup = client
        .authenticate_and_pin(&first.pinned, &credential)
        .unwrap();
    assert_eq!(after_other_signup.verified.tree_root().epoch, 2);
    assert_eq!(after_other_signup.verified.username(), b"signupuser");
    assert_eq!(after_other_signup.puks[0].seed.as_slice(), &[0x72; 32]);
    let stored = server.identity(&uid).unwrap().unwrap();
    assert_eq!(stored.normalized_name, b"signupuser");
    assert_eq!(stored.uid, uid);
    assert!(stored.exact_link.len() > 128);
    assert!(stored.exact_parcel.len() > 128);
    server.shutdown().unwrap();
}
