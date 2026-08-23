use std::io::{Cursor, Write as _};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use foks_client::{FoksClient, ProbeTarget};
use foks_client_db::Acceptance;
use foks_rpc::{
    encode_probe_request, encode_probe_success_response, read_frame, DEFAULT_MAX_FRAME_LENGTH,
};
use rcgen::{generate_simple_self_signed, CertifiedKey};
use rustls::pki_types::PrivatePkcs8KeyDer;

const PROBE_RESPONSE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
));

#[test]
fn local_tls_probe_verifies_and_atomically_pins() {
    let CertifiedKey { cert, signing_key } =
        generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let certificate = cert.der().clone();
    let private_key = PrivatePkcs8KeyDer::from(signing_key.serialize_der());
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let server_config = Arc::new(
        rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![certificate.clone()], private_key.into())
            .unwrap(),
    );
    let response = encode_probe_success_response(PROBE_RESPONSE).unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();

    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for _ in 0..2 {
            let (tcp, _) = listener.accept().unwrap();
            tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            tcp.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
            let connection = rustls::ServerConnection::new(Arc::clone(&server_config)).unwrap();
            let mut tls = rustls::StreamOwned::new(connection, tcp);
            requests.push(read_frame(&mut tls, DEFAULT_MAX_FRAME_LENGTH).unwrap());
            tls.write_all(&response).unwrap();
            tls.flush().unwrap();
        }
        requests
    });

    let mut roots = rustls::RootCertStore::empty();
    roots.add(certificate).unwrap();
    let mut client = FoksClient::with_roots(roots);
    client.set_timeout(Duration::from_secs(5));
    let target = ProbeTarget::parse(&format!("localhost:{port}")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("hard.sqlite3");

    let first = client.probe_and_pin(&target, &database).unwrap();
    let second = client.probe_and_pin(&target, &database).unwrap();
    assert_eq!(first.acceptance, Acceptance::Inserted);
    assert_eq!(second.acceptance, Acceptance::Unchanged);
    assert_eq!(
        first.verified.snapshot.host_id(),
        second.verified.snapshot.host_id()
    );

    let requests = server.join().unwrap();
    let expected_frame = encode_probe_request("localhost", 0, None).unwrap();
    let expected = read_frame(&mut Cursor::new(expected_frame), DEFAULT_MAX_FRAME_LENGTH).unwrap();
    assert_eq!(requests, vec![expected.clone(), expected]);
}
