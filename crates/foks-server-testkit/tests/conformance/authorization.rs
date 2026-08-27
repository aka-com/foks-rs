use foks_proto::{EntityId, SecretSeed, ENTITY_USER};
use foks_server_testkit::{TestAccountSpec, TestClient};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use std::io::Write as _;
use std::sync::Arc;

use crate::support::Fixture;

#[test]
pub(crate) fn authorization_and_unsupported_success() {
    let fixture = Fixture::start("authorization-client");
    let created = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("authuser", 0x81))
        .unwrap();
    let other_client = TestClient::new(&fixture.environment, "other-auth-client").unwrap();
    let other_probe = other_client.probe_and_pin().unwrap();
    let other = other_client
        .create_account(
            &other_probe.pinned,
            &TestAccountSpec::new("otherauth", 0x91),
        )
        .unwrap();
    let refreshed = fixture
        .client
        .foks()
        .authenticate_and_pin(fixture.host(), &created.credential)
        .unwrap();
    assert_eq!(
        refreshed.merkle_acceptance,
        foks_client_db::Acceptance::Advanced
    );
    assert_eq!(refreshed.verified.username(), b"authuser");
    assert_eq!(refreshed.puks[0].seed.as_slice(), &[0x82; 32]);

    let foks_client::DeviceCredential {
        seed,
        certificate_chain,
        ..
    } = other.credential;
    let unbound = foks_client::DeviceCredential {
        uid: created.credential.uid.clone(),
        seed,
        certificate_chain,
    };
    let unbound_error = fixture
        .client
        .foks()
        .authenticate_and_pin(fixture.host(), &unbound)
        .unwrap_err();
    assert!(
        matches!(
            unbound_error,
            foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus { code: 1013, .. })
        ),
        "unexpected unbound-device error: {unbound_error:?}"
    );

    let mut unknown_uid = vec![0x22; 33];
    unknown_uid[0] = ENTITY_USER;
    let unknown_uid = EntityId::from_bytes(unknown_uid).unwrap();
    let unknown_error = fixture
        .client
        .foks()
        .fetch_device_certificate_chain(fixture.host(), &unknown_uid, &SecretSeed::new([0x23; 32]))
        .unwrap_err();
    assert!(matches!(
        unknown_error,
        foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus { code: 1049, .. })
    ));

    let config = fixture
        .client
        .foks()
        .host_config(fixture.host(), &created.credential)
        .unwrap();
    assert_eq!(config.user_viewership, foks_proto::ViewershipMode::Open);
    assert_eq!(config.team_viewership, foks_proto::ViewershipMode::Open);
    assert!(!config.meter_users && !config.meter_vhosts && !config.meter_per_vhost_disk);
}

#[test]
pub(crate) fn unsupported_team_routes_return_typed_status() {
    let fixture = Fixture::start("unsupported-team-routes");
    let created = fixture
        .client
        .create_account(
            fixture.host(),
            &TestAccountSpec::new("unsupportedteam", 0x71),
        )
        .unwrap();
    for route in foks_server::rpc::ROUTES
        .iter()
        .filter(|route| route.protocol.starts_with("Team") && !route.supported)
    {
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let certificates = created
            .credential
            .certificate_chain
            .iter()
            .cloned()
            .map(CertificateDer::from)
            .collect();
        let key = foks_crypto::device_signing_key_pkcs8(&created.credential.seed).unwrap();
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.as_slice()));
        let mut roots = rustls::RootCertStore::empty();
        for certificate in fixture.host().tls_ca_certificates() {
            roots
                .add(CertificateDer::from(certificate.clone()))
                .unwrap();
        }
        let config = rustls::ClientConfig::builder_with_provider(provider)
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
        let mut tls = rustls::StreamOwned::new(connection, tcp);
        let argument = foks_snowpack::encode(&foks_snowpack::Value::Null).unwrap();
        let request =
            foks_rpc::encode_call(route.protocol_id, route.position, &argument, 0).unwrap();
        tls.write_all(&request).unwrap();
        let error = foks_rpc::read_response(&mut tls, 4096, 0).unwrap_err();
        assert!(
            matches!(error, foks_rpc::Error::RemoteStatus { code: 1020, .. }),
            "{}::{} returned {error:?}",
            route.protocol,
            route.method
        );
    }
}
