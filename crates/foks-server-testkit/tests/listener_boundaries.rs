use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::sync::Arc;

use foks_rpc::{Error, DEFAULT_MAX_FRAME_LENGTH};
use foks_server_testkit::{IsolatedTestServer, TestClient, TestEnvironment, TestProfile};

#[test]
fn public_listener_returns_unsupported_for_private_methods() {
    let server = IsolatedTestServer::start().unwrap();
    let mut tls = connect_without_client_certificate(
        server.addresses().public_services,
        server.service_roots(),
    );
    let request = foks_rpc::encode_load_user_chain_request(&[1; 33], 0).unwrap();
    tls.write_all(&request).unwrap();
    tls.flush().unwrap();
    assert!(matches!(
        foks_rpc::read_void_response(&mut tls, DEFAULT_MAX_FRAME_LENGTH, 0),
        Err(Error::RemoteStatus { code: 1020, .. })
    ));
    server.shutdown().unwrap();
}

#[test]
fn authenticated_listener_rejects_a_client_without_a_certificate() {
    let server = IsolatedTestServer::start().unwrap();
    let mut tls = connect_without_client_certificate(
        server.addresses().authenticated,
        server.service_roots(),
    );
    let request = foks_rpc::encode_load_user_chain_request(&[1; 33], 0).unwrap();
    let rejected = tls
        .write_all(&request)
        .and_then(|()| tls.flush())
        .and_then(|()| {
            let mut byte = [0];
            tls.read_exact(&mut byte)
        });
    assert!(rejected.is_err());
    server.shutdown().unwrap();
}

#[test]
fn oversized_frame_is_rejected_from_its_prefix_and_server_stays_available() {
    let environment = TestEnvironment::with_profile(TestProfile::TightIo).unwrap();
    let server = environment.start_server().unwrap();
    let mut tls = connect_without_client_certificate(
        server.addresses().public_services,
        server.service_roots(),
    );
    tls.sock
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .unwrap();
    // Canonical uint16 length 1025, one byte above the profile limit. No
    // payload is sent: rejection must happen before payload allocation/read.
    tls.write_all(&[0xcd, 0x04, 0x01]).unwrap();
    tls.flush().unwrap();
    let mut byte = [0];
    assert!(tls.read_exact(&mut byte).is_err());

    let client = TestClient::new(&environment, "post-oversize-client").unwrap();
    client.probe_and_pin().unwrap();
    server.shutdown().unwrap();
}

#[test]
fn partial_slow_frame_times_out_without_monopolizing_other_workers() {
    let environment = TestEnvironment::with_profile(TestProfile::TightIo).unwrap();
    let server = environment.start_server().unwrap();
    let mut slow = connect_without_client_certificate(
        server.addresses().public_services,
        server.service_roots(),
    );
    slow.sock
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .unwrap();
    // A 64-byte frame with only one payload byte forces a bounded read wait.
    slow.write_all(&[0x40, 0x90]).unwrap();
    slow.flush().unwrap();

    let client = TestClient::new(&environment, "parallel-to-slow-client").unwrap();
    client.probe_and_pin().unwrap();
    let started = std::time::Instant::now();
    let mut byte = [0];
    assert!(slow.read_exact(&mut byte).is_err());
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    server.shutdown().unwrap();
}

fn connect_without_client_certificate(
    address: std::net::SocketAddr,
    roots: rustls::RootCertStore,
) -> rustls::StreamOwned<rustls::ClientConnection, TcpStream> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = Arc::new(
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    );
    let tcp = TcpStream::connect(address).unwrap();
    let connection = rustls::ClientConnection::new(
        config,
        rustls::pki_types::ServerName::try_from("localhost").unwrap(),
    )
    .unwrap();
    rustls::StreamOwned::new(connection, tcp)
}
