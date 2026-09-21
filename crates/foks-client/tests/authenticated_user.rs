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
        key_kind: crate::SoftwareKeyKind::Device,
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
        key_kind: crate::SoftwareKeyKind::Device,
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
        key_kind: crate::SoftwareKeyKind::Device,
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

/// Serves `responses` on one connection and hands back every request frame it
/// read, so a test can count round trips and inspect what was actually asked.
fn spawn_recording_server(
    listener: TcpListener,
    config: Arc<rustls::ServerConfig>,
    responses: Vec<Vec<u8>>,
) -> thread::JoinHandle<Vec<Vec<u8>>> {
    thread::spawn(move || {
        let (tcp, _) = listener.accept().unwrap();
        let connection = rustls::ServerConnection::new(config).unwrap();
        let mut tls = rustls::StreamOwned::new(connection, tcp);
        let mut requests = Vec::new();
        for response in responses {
            requests.push(read_frame(&mut tls, DEFAULT_MAX_FRAME_LENGTH).unwrap());
            tls.write_all(&response).unwrap();
            tls.flush().unwrap();
        }
        requests
    })
}

struct MarkerFixture {
    pki: TestPki,
    directory: tempfile::TempDir,
    advance: foks_verify::VerifiedMerkleAdvance,
    head: foks_verify::VerifiedUserState,
    absence: Vec<foks_proto::MerklePathCompressed>,
}

/// The pinned v0.1.9 user chain, its authenticated epoch-998 advance, and the
/// two terminal absence proofs its own response carries: the link past the head
/// and the username sequence past the head. Those are exactly the paths an
/// honest server returns for the two change-marker keys.
fn marker_fixture() -> MarkerFixture {
    let pki = test_pki();
    let directory = tempfile::tempdir().unwrap();
    let public = verify_public_host("foks.app", PROBE).unwrap();
    HardStateStore::open(&directory.path().join("hard.sqlite3"))
        .unwrap()
        .accept_verified_host(&public.snapshot)
        .unwrap();
    let advance = foks_verify::verify_merkle_advance(
        public.snapshot.merkle_root(),
        &fixture("merkle-root-998.snowp"),
        &fixture("merkle-historical-response.snowp"),
        &foks_proto::HostchainTail {
            seqno: public.snapshot.chain_seqno(),
            hash: public.snapshot.chain_tail_hash(),
        },
    )
    .unwrap();
    let chain_bytes = fixture("user-chain.snowp");
    let chain = foks_proto::UserChain::decode(&chain_bytes).unwrap();
    let uid = entity("uid.snowp");
    let host_id = chain.links[0].decode_eldest().unwrap().host;
    let head = foks_verify::verify_non_self_user_chain(
        &chain_bytes,
        &uid,
        &host_id,
        advance.authenticated_roots(),
        &advance,
    )
    .unwrap();
    let names = usize::try_from(chain.num_username_links).unwrap();
    let absence = vec![
        chain.merkle.paths().last().unwrap().clone(),
        chain.merkle.paths()[names - 1].clone(),
    ];
    MarkerFixture {
        pki,
        directory,
        advance,
        head,
        absence,
    }
}

fn marker_host(marker: &MarkerFixture, merkle: &TcpListener) -> PinnedHost {
    let database = marker.directory.path().join("hard.sqlite3");
    let client = FoksClient::webpki();
    let mut host = client.pinned_host("foks.app", &database).unwrap();
    host.merkle_query = ProbeTarget::parse(&format!(
        "localhost:{}",
        merkle.local_addr().unwrap().port()
    ))
    .unwrap();
    host.tls_ca_certificates = vec![marker.pki.server_ca.clone()];
    host
}

fn multi_lookup_response(
    root: &foks_proto::MerkleRoot,
    paths: Vec<foks_proto::MerklePathCompressed>,
    sequence: u64,
) -> Vec<u8> {
    encode_success_response_at(
        &foks_proto::MerkleMultiLookupResponse {
            root: root.clone(),
            paths,
        }
        .encoded()
        .unwrap(),
        sequence,
    )
    .unwrap()
}

/// A no-op pass over a roster costs exactly one batched lookup, whatever the
/// roster size. Before the marker each member cost two: `advance_merkle_root`
/// on the Merkle service and the chain load on the user service, both inside
/// `load_and_pin_other_local_user_with_material`. Three members therefore go
/// from six requests to one, and no user-service connection is opened at all.
#[test]
fn a_no_op_refresh_pass_costs_one_batched_merkle_round_trip() {
    const MEMBERS: usize = 3;
    let marker = marker_fixture();
    let merkle = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let user = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    user.set_nonblocking(true).unwrap();
    let host = marker_host(&marker, &merkle);

    // The fixture supplies one user chain, so the roster's pinned tails repeat.
    // What is under test is the request count and the batching, not the keys.
    let pinned = vec![marker.head.clone(); MEMBERS];
    let paths = marker
        .absence
        .iter()
        .cloned()
        .cycle()
        .take(MEMBERS * 2)
        .collect::<Vec<_>>();
    let server = spawn_recording_server(
        merkle,
        Arc::clone(&marker.pki.unauthenticated_server),
        vec![
            fixture("merkle-select-vhost-response.frame"),
            multi_lookup_response(marker.advance.root(), paths, 1),
        ],
    );

    let mut client = FoksClient::with_roots(rustls::RootCertStore::empty());
    client.set_timeout(Duration::from_secs(5));
    let marks = client
        .probe_user_chain_tails(&host, &marker.advance, &pinned)
        .unwrap();
    assert_eq!(marks, vec![foks_verify::UserChainTail::Unchanged; MEMBERS]);

    let requests = server.join().unwrap();
    // One vhost selection on the cold connection, then one lookup covering the
    // whole roster. The selection is per connection, not per member.
    assert_eq!(requests.len(), 2);
    let call = foks_rpc::decode_call(&requests[1]).unwrap();
    let lookup = foks_rpc::arguments::decode_merkle_multi_lookup(call.argument()).unwrap();
    assert_eq!(lookup.keys.len(), MEMBERS * 2);
    assert_eq!(lookup.root, Some(marker.advance.root().epoch));
    assert!(!lookup.signed);
    assert!(
        user.accept().is_err(),
        "a proved-unchanged pass loads no chains"
    );
}

/// Section 11's first risk. The lookup names the pass's epoch, but the server
/// chooses what it answers with, so an absence proof is only evidence once the
/// returned root is checked back against the root this pass authenticated.
/// Epoch 997 is a root this client has independently authenticated and at which
/// the probed keys really were absent; answering there instead of at 998 is
/// exactly how a server would forge an absence for a chain that has since
/// moved, and it must be refused rather than read as "unchanged".
#[test]
fn a_forged_absence_under_another_authenticated_root_is_refused() {
    let marker = marker_fixture();
    let merkle = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let host = marker_host(&marker, &merkle);
    let pinned = vec![marker.head.clone()];

    let stale = foks_proto::MerkleRoot::decode(&fixture("merkle-root-997.snowp")).unwrap();
    assert_ne!(stale.epoch, marker.advance.root().epoch);
    assert!(
        marker
            .advance
            .authenticated_roots()
            .contains_epoch(stale.epoch),
        "the stale root is one this client authenticated, and is still refused"
    );
    let mut substituted = marker.advance.root().clone();
    substituted.root_node[0] ^= 1;

    let server = spawn_recording_server(
        merkle,
        Arc::clone(&marker.pki.unauthenticated_server),
        vec![
            fixture("merkle-select-vhost-response.frame"),
            multi_lookup_response(&stale, marker.absence.clone(), 1),
            multi_lookup_response(&substituted, marker.absence.clone(), 2),
            multi_lookup_response(marker.advance.root(), marker.absence.clone(), 3),
        ],
    );

    let mut client = FoksClient::with_roots(rustls::RootCertStore::empty());
    client.set_timeout(Duration::from_secs(5));

    // A stale but genuinely authenticated root, carrying proofs that verify
    // under the epoch the server picked, is not evidence about this epoch.
    assert!(matches!(
        client.probe_user_chain_tails(&host, &marker.advance, &pinned),
        Err(Error::UserBinding(
            "Merkle change-marker lookup returned an unauthenticated root"
        ))
    ));
    // Nor is a root that claims this pass's epoch but is not this pass's root,
    // so the binding is to the whole root and not merely to its epoch number.
    assert!(matches!(
        client.probe_user_chain_tails(&host, &marker.advance, &pinned),
        Err(Error::UserBinding(
            "Merkle change-marker lookup returned an unauthenticated root"
        ))
    ));
    // The same proofs under the pass's own root are accepted, so the refusals
    // above are about the root and not about the paths.
    assert_eq!(
        client
            .probe_user_chain_tails(&host, &marker.advance, &pinned)
            .unwrap(),
        vec![foks_verify::UserChainTail::Unchanged]
    );
    server.join().unwrap();
}

/// A tail location the caller cannot justify must fall back to a full load
/// rather than being probed at a key derived from something else: a pinned
/// state newer than the pass's own epoch says nothing about that epoch, and a
/// server answering fewer paths than keys is not answering the question asked.
#[test]
fn an_unusable_tail_falls_back_to_a_full_load() {
    let marker = marker_fixture();
    let merkle = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let host = marker_host(&marker, &merkle);
    let server = spawn_recording_server(
        merkle,
        Arc::clone(&marker.pki.unauthenticated_server),
        vec![
            fixture("merkle-select-vhost-response.frame"),
            multi_lookup_response(marker.advance.root(), marker.absence[..1].to_vec(), 1),
        ],
    );
    let mut client = FoksClient::with_roots(rustls::RootCertStore::empty());
    client.set_timeout(Duration::from_secs(5));
    assert!(matches!(
        client.probe_user_chain_tails(&host, &marker.advance, std::slice::from_ref(&marker.head)),
        Err(Error::UserBinding(
            "Merkle change-marker lookup returned the wrong number of paths"
        ))
    ));
    server.join().unwrap();

    // No pinned tail at all, and a tail pinned past this pass's epoch, are both
    // reported as "load" without a key ever being derived for them.
    let database = marker.directory.path().join("hard.sqlite3");
    let offline = FoksClient::webpki();
    let offline_host = offline.pinned_host("foks.app", &database).unwrap();
    assert_eq!(
        offline
            .probe_pinned_user_chains(&offline_host, &marker.advance, &[entity("uid.snowp")])
            .unwrap()
            .len(),
        1
    );
    assert!(offline
        .probe_pinned_user_chains(&offline_host, &marker.advance, &[entity("uid.snowp")])
        .unwrap()[0]
        .is_none());
}
