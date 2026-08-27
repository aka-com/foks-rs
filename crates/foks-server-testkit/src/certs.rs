use std::sync::Arc;

use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose, PKCS_ED25519,
};
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};

pub(crate) struct TestTls {
    pub roots: rustls::RootCertStore,
    pub public: Arc<rustls::ServerConfig>,
    pub certificate_chain: Vec<Vec<u8>>,
    pub private_key: zeroize::Zeroizing<Vec<u8>>,
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
    let retained_certificates = certificate_chain
        .iter()
        .map(|certificate| certificate.as_ref().to_vec())
        .collect();
    let server_key_der = server_key.serialize_der();
    let retained_private_key = zeroize::Zeroizing::new(server_key_der.clone());
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let public = Arc::new(
        rustls::ServerConfig::builder_with_provider(Arc::clone(&provider))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                certificate_chain,
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(server_key_der)),
            )
            .unwrap(),
    );
    let mut roots = rustls::RootCertStore::empty();
    roots.add(ca.der().clone()).unwrap();
    TestTls {
        roots,
        public,
        certificate_chain: retained_certificates,
        private_key: retained_private_key,
    }
}
