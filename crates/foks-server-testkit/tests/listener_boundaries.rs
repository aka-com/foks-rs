use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::sync::Arc;

use foks_rpc::{Error, DEFAULT_MAX_FRAME_LENGTH};
use foks_server_testkit::IsolatedTestServer;

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
