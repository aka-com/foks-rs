use std::io::Write as _;
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use super::{CancellationToken, DeviceCredential, Error, FoksClient, PinnedHost, ProbeTarget};
use foks_client_db::HardStateStore;
use foks_crypto::device_signing_key_pkcs8;
use foks_proto::{EntityId, SecretSeed};
use foks_rpc::{encode_success_response_at, read_frame, DEFAULT_MAX_FRAME_LENGTH};
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
    server_ca: Vec<u8>,
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
    let server_ca = ca.der().to_vec();

    TestPki {
        seed,
        server_ca,
        unauthenticated_server,
        authenticated_server,
        certificate_result,
    }
}

fn spawn_server(
    listener: TcpListener,
    config: Arc<rustls::ServerConfig>,
    connections: Vec<Vec<Vec<u8>>>,
    require_client_certificate: bool,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        for responses in connections {
            let (tcp, _) = listener.accept().unwrap();
            let connection = rustls::ServerConnection::new(Arc::clone(&config)).unwrap();
            let mut tls = rustls::StreamOwned::new(connection, tcp);
            for response in responses {
                read_frame(&mut tls, DEFAULT_MAX_FRAME_LENGTH).unwrap();
                if require_client_certificate {
                    assert!(tls.conn.peer_certificates().is_some());
                }
                tls.write_all(&response).unwrap();
                tls.flush().unwrap();
            }
        }
    })
}

#[test]
fn unsigned_newer_merkle_root_is_rejected_before_user_state_is_used() {
    let pki = test_pki();

    let registration = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let registration_port = registration.local_addr().unwrap().port();
    let user = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let user_port = user.local_addr().unwrap().port();
    let merkle = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let merkle_port = merkle.local_addr().unwrap().port();
    let cert_response = encode_success_response_at(&pki.certificate_result, 1).unwrap();
    let root_response = encode_success_response_at(&fixture("merkle-root-998.snowp"), 1).unwrap();
    let select_response = fixture("kv-select-vhost-response.frame");

    let reg_thread = spawn_server(
        registration,
        Arc::clone(&pki.unauthenticated_server),
        vec![vec![select_response.clone(), cert_response]],
        false,
    );
    let user_thread = spawn_server(user, pki.authenticated_server, Vec::new(), true);
    let merkle_thread = spawn_server(
        merkle,
        pki.unauthenticated_server,
        vec![vec![select_response, root_response]],
        false,
    );

    // Delegated services must use the hostchain CA, not bootstrap roots.
    let mut client = FoksClient::with_roots(rustls::RootCertStore::empty());
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
        probe: registration_target.clone(),
        registration: registration_target,
        user: user_target.clone(),
        merkle_query: merkle_target,
        realtime: Some(user_target.clone()),
        kv_store: user_target,
        tls_ca_certificates: vec![pki.server_ca.clone()],
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

    assert!(matches!(
        client.authenticate_and_pin(&pinned, &credential),
        Err(Error::HostBinding("current Merkle root is not signed"))
    ));

    reg_thread.join().unwrap();
    user_thread.join().unwrap();
    merkle_thread.join().unwrap();
}

#[test]
fn stalled_tls_is_cancelled_before_the_socket_timeout() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let cancellation = CancellationToken::new();
    let mut client = FoksClient::with_roots(rustls::RootCertStore::empty())
        .with_cancellation_token(cancellation.clone());
    client.set_timeout(Duration::from_secs(5));
    let target = ProbeTarget::parse(&format!("127.0.0.1:{port}")).unwrap();
    let started = Instant::now();
    let request = thread::spawn(move || client.probe(&target));
    let (stalled_peer, _) = listener.accept().unwrap();
    thread::sleep(Duration::from_millis(50));
    cancellation.cancel();
    let error = request.join().unwrap().unwrap_err();
    assert!(matches!(error, Error::Cancelled));
    assert!(started.elapsed() < Duration::from_secs(2));
    drop(stalled_peer);
}

#[test]
fn stalled_tls_obeys_the_overall_deadline() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut client = FoksClient::with_roots(rustls::RootCertStore::empty());
    client.set_timeout(Duration::from_millis(100));
    let target = ProbeTarget::parse(&format!("127.0.0.1:{port}")).unwrap();
    let started = Instant::now();
    let request = thread::spawn(move || client.probe(&target));
    let (stalled_peer, _) = listener.accept().unwrap();
    let error = request.join().unwrap().unwrap_err();
    assert!(matches!(error, Error::DeadlineExceeded));
    assert!(started.elapsed() < Duration::from_secs(2));
    drop(stalled_peer);
}

#[test]
fn realtime_selects_pinned_host_with_mtls_and_preserves_pooled_sequence() {
    realtime_sequence_scenario(false);
}

#[test]
fn realtime_status_error_consumes_sequence_and_preserves_connection() {
    realtime_sequence_scenario(true);
}

fn realtime_sequence_scenario(status_error: bool) {
    use foks_proto::{
        RealtimeWire, RtChannelSet, RtHostId, RtListChannelsArgument, RtSelectVhostArgument,
    };
    use foks_rpc::{read_call, RealtimeRequest, RealtimeResponse, REAL_TIME_PROTOCOL_ID};
    let pki = test_pki();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let target = ProbeTarget::parse(&format!(
        "localhost:{}",
        listener.local_addr().unwrap().port()
    ))
    .unwrap();
    let mut client = FoksClient::with_roots(rustls::RootCertStore::empty());
    client.set_timeout(Duration::from_secs(5));
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("hard.sqlite3");
    let public = verify_public_host("foks.app", PROBE).unwrap();
    HardStateStore::open(&database)
        .unwrap()
        .accept_verified_host(&public.snapshot)
        .unwrap();
    let mut host = client.pinned_host("foks.app", &database).unwrap();
    host.realtime = Some(target);
    host.tls_ca_certificates = vec![pki.server_ca];
    let Value::Array(certs) = decode(&pki.certificate_result).unwrap() else {
        panic!()
    };
    let credential = DeviceCredential {
        uid: entity("uid.snowp"),
        seed: pki.seed,
        certificate_chain: certs
            .into_iter()
            .map(|v| {
                let Value::Binary(b) = v else { panic!() };
                b
            })
            .collect(),
    };
    let select = RealtimeRequest::SelectVhost(RtSelectVhostArgument {
        host: RtHostId::new(host.host_id.clone()).unwrap(),
    });
    let expected_select = select.argument().unwrap();
    let server = thread::spawn(move || {
        let (tcp, _) = listener.accept().unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let conn = rustls::ServerConnection::new(pki.authenticated_server).unwrap();
        let mut tls = rustls::StreamOwned::new(conn, tcp);
        for (sequence, position) in [(0, 9), (1, 2), (2, 2)] {
            let call = read_call(&mut tls, DEFAULT_MAX_FRAME_LENGTH).unwrap();
            assert!(tls.conn.peer_certificates().is_some());
            assert_eq!(call.protocol_id(), REAL_TIME_PROTOCOL_ID);
            assert_eq!(call.method_position(), position);
            assert_eq!(call.sequence(), sequence);
            if sequence == 0 {
                assert_eq!(call.argument(), expected_select);
            }
            if sequence == 1 && status_error {
                tls.write_all(
                    &foks_rpc::encode_status_response_at(
                        &foks_rpc::RpcStatus::RtRace("retry after refresh".into()),
                        sequence,
                    )
                    .unwrap(),
                )
                .unwrap();
                tls.flush().unwrap();
                continue;
            }
            let result = if sequence == 0 {
                vec![0xc0]
            } else {
                RtChannelSet {
                    version: 7,
                    channels: vec![],
                    mtime: 8,
                }
                .encoded()
                .unwrap()
            };
            tls.write_all(&encode_success_response_at(&result, sequence).unwrap())
                .unwrap();
            tls.flush().unwrap();
        }
    });
    let fixture = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../foks-snowpack/tests/fixtures/foks-v0.1.9/realtime/request-2.frame"
    ))
    .unwrap();
    let call = read_call(&mut fixture.as_slice(), DEFAULT_MAX_FRAME_LENGTH).unwrap();
    let request =
        RealtimeRequest::ListChannels(RtListChannelsArgument::decode(call.argument()).unwrap());
    for attempt in 0..2 {
        let mut connection = client.realtime_connection(&host, &credential).unwrap();
        assert!(connection.call(&select).is_err());
        if attempt == 0 && status_error {
            assert!(matches!(
                connection.call(&request),
                Err(Error::Rpc(foks_rpc::Error::RemoteStatus {
                    code: 12003,
                    ..
                }))
            ));
        } else {
            assert!(
                matches!(connection.call(&request).unwrap(), RealtimeResponse::Channels(v) if v.version == 7)
            );
        }
    }
    server.join().unwrap();
    host.realtime = None;
    assert!(matches!(
        client.realtime_connection(&host, &credential),
        Err(Error::PinnedService("realtime"))
    ));
}

#[test]
fn realtime_timeout_closes_stream_and_rejects_subsequent_calls() {
    use foks_proto::{
        RealtimeWire, RtChannelSet, RtHostId, RtListChannelsArgument, RtSelectVhostArgument,
    };
    use foks_rpc::{read_call, RealtimeRequest, REAL_TIME_PROTOCOL_ID};
    let pki = test_pki();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let target = ProbeTarget::parse(&format!(
        "localhost:{}",
        listener.local_addr().unwrap().port()
    ))
    .unwrap();
    let mut client = FoksClient::with_roots(rustls::RootCertStore::empty());
    client.set_timeout(Duration::from_millis(250));
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("hard.sqlite3");
    let public = verify_public_host("foks.app", PROBE).unwrap();
    HardStateStore::open(&database)
        .unwrap()
        .accept_verified_host(&public.snapshot)
        .unwrap();
    let mut host = client.pinned_host("foks.app", &database).unwrap();
    host.realtime = Some(target);
    host.tls_ca_certificates = vec![pki.server_ca];
    let Value::Array(certs) = decode(&pki.certificate_result).unwrap() else {
        panic!()
    };
    let credential = DeviceCredential {
        uid: entity("uid.snowp"),
        seed: pki.seed,
        certificate_chain: certs
            .into_iter()
            .map(|v| {
                let Value::Binary(b) = v else { panic!() };
                b
            })
            .collect(),
    };
    let select = RealtimeRequest::SelectVhost(RtSelectVhostArgument {
        host: RtHostId::new(host.host_id.clone()).unwrap(),
    });
    let expected_select = select.argument().unwrap();
    let server = thread::spawn(move || {
        let (tcp, _) = listener.accept().unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let conn = rustls::ServerConnection::new(pki.authenticated_server).unwrap();
        let mut tls = rustls::StreamOwned::new(conn, tcp);
        for (sequence, position) in [(0, 9), (1, 2), (2, 2)] {
            if sequence == 2 {
                // After the timeout, the peer must close instead of issuing a
                // new request that could consume the delayed response.
                assert!(read_call(&mut tls, DEFAULT_MAX_FRAME_LENGTH).is_err());
                break;
            }
            let call = read_call(&mut tls, DEFAULT_MAX_FRAME_LENGTH).unwrap();
            assert!(tls.conn.peer_certificates().is_some());
            assert_eq!(call.protocol_id(), REAL_TIME_PROTOCOL_ID);
            assert_eq!(call.method_position(), position);
            assert_eq!(call.sequence(), sequence);
            if sequence == 0 {
                assert_eq!(call.argument(), expected_select);
            }
            if sequence == 1 {
                continue;
            }
            let result = if sequence == 0 {
                vec![0xc0]
            } else {
                RtChannelSet {
                    version: 7,
                    channels: vec![],
                    mtime: 8,
                }
                .encoded()
                .unwrap()
            };
            tls.write_all(&encode_success_response_at(&result, sequence).unwrap())
                .unwrap();
            tls.flush().unwrap();
        }
    });
    let fixture = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../foks-snowpack/tests/fixtures/foks-v0.1.9/realtime/request-2.frame"
    ))
    .unwrap();
    let call = read_call(&mut fixture.as_slice(), DEFAULT_MAX_FRAME_LENGTH).unwrap();
    let request =
        RealtimeRequest::ListChannels(RtListChannelsArgument::decode(call.argument()).unwrap());
    let mut connection = client.realtime_connection(&host, &credential).unwrap();
    assert!(connection.call(&request).is_err());
    let RealtimeRequest::ListChannels(mut next) = request else {
        panic!()
    };
    next.last = 999;
    assert!(matches!(
        connection.call(&RealtimeRequest::ListChannels(next)),
        Err(Error::Transport("pooled connection is absent"))
    ));
    server.join().unwrap();
    host.realtime = None;
    assert!(matches!(
        client.realtime_connection(&host, &credential),
        Err(Error::PinnedService("realtime"))
    ));
}
