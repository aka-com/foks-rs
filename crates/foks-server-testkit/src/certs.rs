use std::sync::Arc;

use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose, PKCS_ED25519,
};
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::WebPkiClientVerifier;

pub(crate) struct TestTls {
    pub roots: rustls::RootCertStore,
    pub public: Arc<rustls::ServerConfig>,
    pub authenticated: Arc<rustls::ServerConfig>,
}

pub(crate) fn make_tls() -> TestTls {
    let ca_key = KeyPair::generate_for(&PKCS_ED25519).unwrap();
    let mut ca_params = CertificateParams::default();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    let ca = CertifiedIssuer::self_signed(ca_params, ca_key).unwrap();

    let server_key = KeyPair::generate_for(&PKCS_ED25519).unwrap();
    let mut server_params = CertificateParams::new(vec!["localhost".to_owned()]).unwrap();
    server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let server_cert = server_params.signed_by(&server_key, &ca).unwrap();
    let certificate_chain = vec![server_cert.der().clone(), ca.der().clone()];
    let server_key_der = server_key.serialize_der();
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let public = Arc::new(
        rustls::ServerConfig::builder_with_provider(Arc::clone(&provider))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                certificate_chain.clone(),
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(server_key_der.clone())),
            )
            .unwrap(),
    );
    let mut roots = rustls::RootCertStore::empty();
    roots.add(ca.der().clone()).unwrap();
    let verifier =
        WebPkiClientVerifier::builder_with_provider(Arc::new(roots.clone()), Arc::clone(&provider))
            .build()
            .unwrap();
    let authenticated = Arc::new(
        rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_client_cert_verifier(verifier)
            .with_single_cert(
                certificate_chain,
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(server_key_der)),
            )
            .unwrap(),
    );
    TestTls {
        roots,
        public,
        authenticated,
    }
}
