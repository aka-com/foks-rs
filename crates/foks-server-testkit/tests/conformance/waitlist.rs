use std::io::Write as _;
use std::sync::Arc;

use foks_snowpack::{decode, Value};
use rustls::pki_types::{CertificateDer, ServerName};

use crate::support::Fixture;

#[test]
pub(crate) fn go_client_waitlist_works() {
    let fixture = Fixture::start("go-client-waitlist");
    let mut tls = public_stream(&fixture);

    tls.write_all(&foks_rpc::encode_join_waitlist_request_at(b"go-client@example.com", 0).unwrap())
        .unwrap();
    let response =
        foks_rpc::read_response(&mut tls, foks_rpc::DEFAULT_MAX_FRAME_LENGTH, 0).unwrap();
    let Value::Binary(waitlist_id) = decode(&response).unwrap() else {
        panic!("waitlist response was not a binary identifier");
    };
    assert_eq!(waitlist_id.len(), 13);
    assert_eq!(waitlist_id[0], 1);
}

fn public_stream(
    fixture: &Fixture,
) -> rustls::StreamOwned<rustls::ClientConnection, std::net::TcpStream> {
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
    .with_no_client_auth();
    let tcp = std::net::TcpStream::connect(fixture.server.addresses().public_services).unwrap();
    let connection = rustls::ClientConnection::new(
        Arc::new(config),
        ServerName::try_from("localhost".to_owned()).unwrap(),
    )
    .unwrap();
    rustls::StreamOwned::new(connection, tcp)
}
