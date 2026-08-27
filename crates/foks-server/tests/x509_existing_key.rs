use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose, PKCS_ED25519,
};
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use rustls::server::WebPkiClientVerifier;
use x509_parser::prelude::parse_x509_certificate;

#[test]
fn ca_issues_for_detached_ed25519_public_key_and_mtls_proves_possession() {
    let ca_key = KeyPair::generate_for(&PKCS_ED25519).expect("CA key");
    let mut ca_params = CertificateParams::default();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::CrlSign,
    ];
    let ca = CertifiedIssuer::self_signed(ca_params, ca_key).expect("CA certificate");

    let server_key = KeyPair::generate_for(&PKCS_ED25519).expect("server key");
    let mut server_params = CertificateParams::new(vec!["localhost".to_owned()]).unwrap();
    server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let server_cert = server_params
        .signed_by(&server_key, &ca)
        .expect("server certificate");

    let device_key = KeyPair::generate_for(&PKCS_ED25519).expect("device key");
    let detached: [u8; 32] = device_key
        .public_key_raw()
        .try_into()
        .expect("Ed25519 public key length");

    // The certificate issuer receives only the detached public-key value. The
    // device private key remains available solely to the later TLS client.
    let client_cert = foks_server::pki::issue_ed25519_client_certificate(detached, &ca)
        .expect("client certificate for detached public key");
    let (_, parsed) = parse_x509_certificate(client_cert.der()).expect("parse client cert");
    assert_eq!(
        parsed.public_key().subject_public_key.data.as_ref(),
        detached
    );

    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut client_roots = rustls::RootCertStore::empty();
    client_roots.add(ca.der().clone()).unwrap();
    let client_verifier = WebPkiClientVerifier::builder_with_provider(
        Arc::new(client_roots.clone()),
        Arc::clone(&provider),
    )
    .build()
    .unwrap();
    let server_config = Arc::new(
        rustls::ServerConfig::builder_with_provider(Arc::clone(&provider))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_client_cert_verifier(client_verifier)
            .with_single_cert(
                vec![server_cert.der().clone(), ca.der().clone()],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(server_key.serialize_der())),
            )
            .unwrap(),
    );
    let client_config = Arc::new(
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(client_roots)
            .with_client_auth_cert(
                vec![client_cert.der().clone(), ca.der().clone()],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(device_key.serialize_der())),
            )
            .unwrap(),
    );

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    assert!(address.ip().is_loopback());
    let server = thread::spawn(move || {
        let (tcp, peer) = listener.accept().unwrap();
        assert!(peer.ip().is_loopback());
        let connection = rustls::ServerConnection::new(server_config).unwrap();
        let mut tls = rustls::StreamOwned::new(connection, tcp);
        let mut request = [0_u8; 4];
        tls.read_exact(&mut request).unwrap();
        assert_eq!(&request, b"ping");
        assert_eq!(tls.conn.peer_certificates().map(<[_]>::len), Some(2));
        tls.write_all(b"pong").unwrap();
        tls.flush().unwrap();
    });

    let tcp = TcpStream::connect(address).unwrap();
    let server_name = ServerName::try_from("localhost").unwrap();
    let connection = rustls::ClientConnection::new(client_config, server_name).unwrap();
    let mut tls = rustls::StreamOwned::new(connection, tcp);
    tls.write_all(b"ping").unwrap();
    tls.flush().unwrap();
    let mut response = [0_u8; 4];
    tls.read_exact(&mut response).unwrap();
    assert_eq!(&response, b"pong");
    server.join().unwrap();
}
