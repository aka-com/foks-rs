use std::io::Write as _;
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use super::{DeviceCredential, PinnedHost, ProbeTarget, PublicClient};
use foks_client_db::{Acceptance, HardStateStore};
use foks_crypto::device_signing_key_pkcs8;
use foks_proto::{EntityId, SecretSeed};
use foks_rpc::{encode_probe_success_response, read_frame, DEFAULT_MAX_FRAME_LENGTH};
use foks_snowpack::{decode, encode, Value};
use foks_verify::verify_public_host;
use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose, PKCS_ED25519,
};
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::WebPkiClientVerifier;

const DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../foks-snowpack/tests/fixtures/foks-v0.1.9/user"
);
const PROBE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
));

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{DIR}/{name}")).unwrap()
}

fn entity(name: &str) -> EntityId {
    let Value::Binary(bytes) = decode(&fixture(name)).unwrap() else {
        panic!("entity fixture is not binary")
    };
    EntityId::from_bytes(bytes).unwrap()
}

struct TestPki {
    seed: SecretSeed,
    client_roots: rustls::RootCertStore,
    unauthenticated_server: Arc<rustls::ServerConfig>,
    authenticated_server: Arc<rustls::ServerConfig>,
    certificate_result: Vec<u8>,
}

fn test_pki() -> TestPki {
    let ca_key = KeyPair::generate_for(&PKCS_ED25519).unwrap();
    let mut ca_params = CertificateParams::default();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::CrlSign,
    ];
    let ca = CertifiedIssuer::self_signed(ca_params, ca_key).unwrap();

    let server_key = KeyPair::generate_for(&PKCS_ED25519).unwrap();
    let mut server_params = CertificateParams::new(vec!["localhost".to_owned()]).unwrap();
    server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let server_cert = server_params.signed_by(&server_key, &ca).unwrap();

    let seed = SecretSeed::new(fixture("device-seed.bin").try_into().unwrap());
    let client_pkcs8 = device_signing_key_pkcs8(&seed).unwrap();
    let client_key = KeyPair::from_pkcs8_der_and_sign_algo(
        &PrivatePkcs8KeyDer::from(client_pkcs8.to_vec()),
        &PKCS_ED25519,
    )
    .unwrap();
    let mut client_params = CertificateParams::default();
    client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    client_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    let client_cert = client_params.signed_by(&client_key, &ca).unwrap();

    let mut client_roots = rustls::RootCertStore::empty();
    client_roots.add(ca.der().clone()).unwrap();
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let unauthenticated_server = Arc::new(
        rustls::ServerConfig::builder_with_provider(Arc::clone(&provider))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![server_cert.der().clone(), ca.der().clone()],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(server_key.serialize_der())),
            )
            .unwrap(),
    );
    let mut server_client_roots = rustls::RootCertStore::empty();
    server_client_roots.add(ca.der().clone()).unwrap();
    let verifier = WebPkiClientVerifier::builder_with_provider(
        Arc::new(server_client_roots),
        Arc::clone(&provider),
    )
    .build()
    .unwrap();
    let authenticated_server = Arc::new(
        rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_client_cert_verifier(verifier)
            .with_single_cert(
                vec![server_cert.der().clone(), ca.der().clone()],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(server_key.serialize_der())),
            )
            .unwrap(),
    );
    let certificate_result = encode(&Value::Array(vec![
        Value::Binary(client_cert.der().to_vec()),
        Value::Binary(ca.der().to_vec()),
    ]))
    .unwrap();

    TestPki {
        seed,
        client_roots,
        unauthenticated_server,
        authenticated_server,
        certificate_result,
    }
}

fn spawn_server(
    listener: TcpListener,
    config: Arc<rustls::ServerConfig>,
    responses: Vec<Vec<u8>>,
    require_client_certificate: bool,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        for response in responses {
            let (tcp, _) = listener.accept().unwrap();
            let connection = rustls::ServerConnection::new(Arc::clone(&config)).unwrap();
            let mut tls = rustls::StreamOwned::new(connection, tcp);
            read_frame(&mut tls, DEFAULT_MAX_FRAME_LENGTH).unwrap();
            if require_client_certificate {
                assert!(tls.conn.peer_certificates().is_some());
            }
            tls.write_all(&response).unwrap();
            tls.flush().unwrap();
        }
    })
}

#[test]
fn registration_then_mtls_user_chain_and_puk_complete_without_interactivity() {
    let pki = test_pki();

    let registration = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let registration_port = registration.local_addr().unwrap().port();
    let user = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let user_port = user.local_addr().unwrap().port();
    let merkle = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let merkle_port = merkle.local_addr().unwrap().port();
    let cert_response = encode_probe_success_response(&pki.certificate_result).unwrap();
    let chain_response = encode_probe_success_response(&fixture("user-chain.snowp")).unwrap();
    let puk_response = encode_probe_success_response(&fixture("puk-parcel.snowp")).unwrap();
    let root_response = encode_probe_success_response(&fixture("merkle-root-998.snowp")).unwrap();
    let history_response =
        encode_probe_success_response(&fixture("merkle-historical-response.snowp")).unwrap();

    let reg_thread = spawn_server(
        registration,
        Arc::clone(&pki.unauthenticated_server),
        vec![cert_response],
        false,
    );
    let user_thread = spawn_server(
        user,
        pki.authenticated_server,
        vec![chain_response, puk_response],
        true,
    );
    let merkle_thread = spawn_server(
        merkle,
        pki.unauthenticated_server,
        vec![root_response, history_response],
        false,
    );

    let mut client = PublicClient::with_roots(pki.client_roots);
    client.set_timeout(Duration::from_secs(5));
    let registration_target =
        ProbeTarget::parse(&format!("localhost:{registration_port}")).unwrap();
    let user_target = ProbeTarget::parse(&format!("localhost:{user_port}")).unwrap();
    let merkle_target = ProbeTarget::parse(&format!("localhost:{merkle_port}")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("hard.sqlite3");
    let public = verify_public_host("foks.app", PROBE).unwrap();
    HardStateStore::open(&database)
        .unwrap()
        .accept_verified_host(&public.snapshot)
        .unwrap();
    let pinned = PinnedHost {
        lookup_name: "foks.app".to_owned(),
        host_id: foks_proto::ProbeResponse::decode(PROBE).unwrap().hostchain[0]
            .decode_change()
            .unwrap()
            .host,
        database_path: database.clone(),
        registration: registration_target,
        user: user_target,
        merkle_query: merkle_target,
    };
    let uid = entity("uid.snowp");
    let certificates = client
        .fetch_device_certificate_chain(&pinned, &uid, &pki.seed)
        .unwrap();
    let credential = DeviceCredential {
        uid: uid.clone(),
        seed: pki.seed,
        certificate_chain: certificates,
    };

    let result = client.authenticate_and_pin(&pinned, &credential).unwrap();
    assert_eq!(result.merkle_acceptance, Acceptance::Advanced);
    assert_eq!(result.acceptance, Acceptance::Inserted);
    assert_eq!(result.puk_seed.as_slice(), fixture("puk-seed.bin"));
    assert_eq!(
        client
            .pinned_user(&pinned, &uid)
            .unwrap()
            .unwrap()
            .chain_seqno(),
        3
    );

    reg_thread.join().unwrap();
    user_thread.join().unwrap();
    merkle_thread.join().unwrap();
}
