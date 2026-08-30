use foks_client::{AddLocalTeamMemberRequest, DeviceCredential, NamedTeamSecrets};
use foks_proto::{EntityId, PermissionToken, Role, SecretSeed, ENTITY_USER};
use foks_rpc::TeamChainLoadOptions;
use foks_server_testkit::{TestAccountSpec, TestClient};
use foks_snowpack::Value;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use std::io::Write as _;
use std::sync::Arc;

use crate::support::Fixture;

const MAX_RESPONSE: usize = 8 * 1024 * 1024;

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
pub(crate) fn local_team_view_tokens_require_their_authenticated_member() {
    let fixture = Fixture::start("local-team-view-auth");
    let account = fixture
        .client
        .create_account(
            fixture.host(),
            &TestAccountSpec::new("localteamviewer", 0xa1),
        )
        .unwrap();
    let secrets = NamedTeamSecrets {
        member_min: SecretSeed::new([0xa2; 32]),
        member: SecretSeed::new([0xa3; 32]),
        admin: SecretSeed::new([0xa4; 32]),
        owner: SecretSeed::new([0xa5; 32]),
        removal_key: SecretSeed::new([0xa6; 32]),
        team_name_commitment_key: [0xa7; 16],
    };
    let created = fixture
        .client
        .foks()
        .create_single_owner_named_team(
            fixture.host(),
            &account.credential,
            "authenticatedteam",
            &secrets,
        )
        .unwrap();
    assert_eq!(created.authenticated.verified.chain_seqno(), 1);

    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
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
        .with_no_client_auth();
    let tcp = std::net::TcpStream::connect(fixture.server.addresses().public_services).unwrap();
    let connection = rustls::ClientConnection::new(
        Arc::new(config),
        ServerName::try_from("localhost".to_owned()).unwrap(),
    )
    .unwrap();
    let mut tls = rustls::StreamOwned::new(connection, tcp);
    let request = foks_rpc::encode_load_team_chain_request_with_options(
        &created.team,
        fixture.host().host_id(),
        &created.authenticated.view_token,
        1,
        TeamChainLoadOptions {
            load_removal_key: true,
            load_remote_view_tokens: true,
            ..TeamChainLoadOptions::default()
        },
    )
    .unwrap();
    tls.write_all(&request).unwrap();
    let error = foks_rpc::read_response(&mut tls, 1024 * 1024, 0).unwrap_err();
    assert!(matches!(
        error,
        foks_rpc::Error::RemoteStatus { code: 1013, .. }
    ));

    let other_client = TestClient::new(&fixture.environment, "other-team-viewer").unwrap();
    let other_host = other_client.probe_and_pin().unwrap();
    let other = other_client
        .create_account(
            &other_host.pinned,
            &TestAccountSpec::new("otherteamviewer", 0xb1),
        )
        .unwrap();
    let certificates = other
        .credential
        .certificate_chain
        .iter()
        .cloned()
        .map(CertificateDer::from)
        .collect();
    let key = foks_crypto::device_signing_key_pkcs8(&other.credential.seed).unwrap();
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
    let mut tls = rustls::StreamOwned::new(connection, tcp);
    tls.write_all(&request).unwrap();
    let error = foks_rpc::read_response(&mut tls, 1024 * 1024, 0).unwrap_err();
    assert!(matches!(
        error,
        foks_rpc::Error::RemoteStatus { code: 1013, .. }
    ));
}

#[test]
pub(crate) fn go_chain_load_authorizations_follow_current_local_permissions() {
    let fixture = Fixture::start("go-chain-load-authorizations");
    let owner = fixture
        .client
        .create_account(
            fixture.host(),
            &TestAccountSpec::new("chainauthowner", 0xc1),
        )
        .unwrap();
    let member_client = TestClient::new(&fixture.environment, "chain-auth-member").unwrap();
    let member_host = member_client.probe_and_pin().unwrap();
    let member = member_client
        .create_account(
            &member_host.pinned,
            &TestAccountSpec::new("chainauthmember", 0xd1),
        )
        .unwrap();

    let self_token = PermissionToken::new([0xd3; 17]);
    let self_authorization = Value::Array(vec![
        Value::Unsigned(2),
        Value::Variant(Some((b"2".to_vec(), Box::new(self_token.to_value())))),
    ]);
    let mut public = public_stream(&fixture);
    public
        .write_all(
            &foks_rpc::encode_registration_select_vhost_request(fixture.host().host_id()).unwrap(),
        )
        .unwrap();
    foks_rpc::read_void_response(&mut public, MAX_RESPONSE, 0).unwrap();
    let self_request = foks_rpc::encode_call(
        foks_rpc::REG_PROTOCOL_ID,
        foks_rpc::REG_LOAD_USER_CHAIN_METHOD_POSITION,
        &load_user_chain_argument(member.credential.uid.as_bytes(), self_authorization),
        0,
    )
    .unwrap();
    public
        .write_all(
            &foks_rpc::resequence_call(&self_request, 1, foks_rpc::DEFAULT_MAX_FRAME_LENGTH)
                .unwrap(),
        )
        .unwrap();
    foks_rpc::read_response(&mut public, MAX_RESPONSE, 1).unwrap();

    let open = Value::Array(vec![Value::Unsigned(4), Value::Variant(None)]);
    let mut authenticated = authenticated_stream(&fixture, &owner.credential);
    authenticated
        .write_all(&user_load_request(member.credential.uid.as_bytes(), open))
        .unwrap();
    foks_rpc::read_response(&mut authenticated, MAX_RESPONSE, 0).unwrap();

    let local_user = Value::Array(vec![Value::Unsigned(0), Value::Variant(None)]);
    let mut authenticated = authenticated_stream(&fixture, &owner.credential);
    authenticated
        .write_all(&user_load_request(
            member.credential.uid.as_bytes(),
            local_user,
        ))
        .unwrap();
    assert!(matches!(
        foks_rpc::read_response(&mut authenticated, MAX_RESPONSE, 0),
        Err(foks_rpc::Error::RemoteStatus { code: 1013, .. })
    ));

    let secrets = NamedTeamSecrets {
        member_min: SecretSeed::new([0xe1; 32]),
        member: SecretSeed::new([0xe2; 32]),
        admin: SecretSeed::new([0xe3; 32]),
        owner: SecretSeed::new([0xe4; 32]),
        removal_key: SecretSeed::new([0xe5; 32]),
        team_name_commitment_key: [0xe6; 16],
    };
    let team = fixture
        .client
        .foks()
        .create_single_owner_named_team(
            fixture.host(),
            &owner.credential,
            "chainauthteam",
            &secrets,
        )
        .unwrap();
    let removal_key = SecretSeed::new([0xe7; 32]);
    let added = fixture
        .client
        .foks()
        .add_local_user_to_named_team(
            fixture.host(),
            &owner.credential,
            &team.team,
            &AddLocalTeamMemberRequest {
                target_user: &member.authenticated.verified,
                destination_role: Role::member(0),
                removal_key: &removal_key,
            },
        )
        .unwrap();
    let as_local_team = Value::Array(vec![
        Value::Unsigned(3),
        Value::Variant(Some((
            b"3".to_vec(),
            Box::new(Value::Binary(added.authenticated.view_token.to_vec())),
        ))),
    ]);
    let mut authenticated = authenticated_stream(&fixture, &owner.credential);
    authenticated
        .write_all(&user_load_request(
            member.credential.uid.as_bytes(),
            as_local_team.clone(),
        ))
        .unwrap();
    foks_rpc::read_response(&mut authenticated, MAX_RESPONSE, 0).unwrap();

    let mut unrelated = vec![0x71; 33];
    unrelated[0] = ENTITY_USER;
    let mut authenticated = authenticated_stream(&fixture, &owner.credential);
    authenticated
        .write_all(&user_load_request(&unrelated, as_local_team))
        .unwrap();
    assert!(matches!(
        foks_rpc::read_response(&mut authenticated, MAX_RESPONSE, 0),
        Err(foks_rpc::Error::RemoteStatus { code: 1013, .. })
    ));
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

fn load_user_chain_argument(uid: &[u8], authorization: Value) -> Vec<u8> {
    foks_snowpack::encode(&Value::Array(vec![Value::Array(vec![
        Value::Binary(uid.to_vec()),
        Value::Unsigned(1),
        Value::Null,
        authorization,
    ])]))
    .unwrap()
}

fn user_load_request(uid: &[u8], authorization: Value) -> Vec<u8> {
    foks_rpc::encode_call(
        foks_rpc::USER_PROTOCOL_ID,
        foks_rpc::USER_LOAD_USER_CHAIN_METHOD_POSITION,
        &load_user_chain_argument(uid, authorization),
        0,
    )
    .unwrap()
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
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(root_store(fixture))
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

fn public_stream(
    fixture: &Fixture,
) -> rustls::StreamOwned<rustls::ClientConnection, std::net::TcpStream> {
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(root_store(fixture))
    .with_no_client_auth();
    let tcp = std::net::TcpStream::connect(fixture.server.addresses().public_services).unwrap();
    let connection = rustls::ClientConnection::new(
        Arc::new(config),
        ServerName::try_from("localhost".to_owned()).unwrap(),
    )
    .unwrap();
    rustls::StreamOwned::new(connection, tcp)
}

fn root_store(fixture: &Fixture) -> rustls::RootCertStore {
    let mut roots = rustls::RootCertStore::empty();
    for certificate in fixture.host().tls_ca_certificates() {
        roots
            .add(CertificateDer::from(certificate.clone()))
            .unwrap();
    }
    roots
}
