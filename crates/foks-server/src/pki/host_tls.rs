use std::sync::Arc;

use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose, PKCS_ED25519,
};
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::WebPkiClientVerifier;
use zeroize::Zeroizing;

use crate::keys::{HostKeyProvider, KeyPurpose, SecretKey};
use crate::{Error, Result};

pub struct HostTls {
    pub public: Arc<rustls::ServerConfig>,
    pub authenticated: Arc<rustls::ServerConfig>,
    pub delegated_ca: rustls::RootCertStore,
    pub client_ca: rustls::RootCertStore,
}

pub fn build_host_tls(provider: &dyn HostKeyProvider, canonical_name: &str) -> Result<HostTls> {
    let delegated_key = provider.load_or_create(KeyPurpose::DelegatedTls)?;
    let delegated = ed25519_ca(&delegated_key, canonical_name, "FOKS delegated TLS CA")?;
    let client_key = provider.load_or_create(KeyPurpose::ClientCa)?;
    let client = ed25519_ca(&client_key, canonical_name, "FOKS client identity CA")?;

    let server_key = KeyPair::generate_for(&PKCS_ED25519)?;
    let mut server_parameters = CertificateParams::new(vec![canonical_name.to_owned()])?;
    server_parameters.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    server_parameters.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    let server_certificate = server_parameters.signed_by(&server_key, &delegated)?;
    let certificate_chain = vec![server_certificate.der().clone(), delegated.der().clone()];
    let private_key = PrivatePkcs8KeyDer::from(server_key.serialize_der());
    let crypto_provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let public = Arc::new(
        rustls::ServerConfig::builder_with_provider(Arc::clone(&crypto_provider))
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(
                certificate_chain.clone(),
                PrivateKeyDer::Pkcs8(private_key.clone_key()),
            )?,
    );

    let mut client_ca = rustls::RootCertStore::empty();
    client_ca.add(client.der().clone())?;
    let verifier = WebPkiClientVerifier::builder_with_provider(
        Arc::new(client_ca.clone()),
        Arc::clone(&crypto_provider),
    )
    .build()
    .map_err(|_| Error::Config("invalid client CA verifier"))?;
    let authenticated = Arc::new(
        rustls::ServerConfig::builder_with_provider(crypto_provider)
            .with_safe_default_protocol_versions()?
            .with_client_cert_verifier(verifier)
            .with_single_cert(certificate_chain, PrivateKeyDer::Pkcs8(private_key))?,
    );
    let mut delegated_ca = rustls::RootCertStore::empty();
    delegated_ca.add(delegated.der().clone())?;
    Ok(HostTls {
        public,
        authenticated,
        delegated_ca,
        client_ca,
    })
}

pub(crate) fn delegated_tls_ca_der(key: &SecretKey, canonical_name: &str) -> Result<Vec<u8>> {
    Ok(ed25519_ca(key, canonical_name, "FOKS delegated TLS CA")?
        .der()
        .to_vec())
}

pub(crate) fn client_identity_ca(
    provider: &dyn HostKeyProvider,
    canonical_name: &str,
) -> Result<CertifiedIssuer<'static, KeyPair>> {
    let key = provider.load_or_create(KeyPurpose::ClientCa)?;
    ed25519_ca(&key, canonical_name, "FOKS client identity CA")
}

fn ed25519_ca(
    key: &SecretKey,
    canonical_name: &str,
    common_name: &str,
) -> Result<CertifiedIssuer<'static, KeyPair>> {
    let signing_key = key_pair(key)?;
    let mut parameters = CertificateParams::default();
    parameters.distinguished_name = DistinguishedName::new();
    parameters.distinguished_name.push(
        DnType::CommonName,
        format!("{canonical_name} {common_name}"),
    );
    parameters.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    parameters.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    Ok(CertifiedIssuer::self_signed(parameters, signing_key)?)
}

fn key_pair(key: &SecretKey) -> Result<KeyPair> {
    let mut pkcs8 = Zeroizing::new(Vec::with_capacity(48));
    pkcs8.extend_from_slice(&[
        0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04,
        0x20,
    ]);
    pkcs8.extend_from_slice(key.expose());
    let private_key = PrivatePkcs8KeyDer::from(pkcs8.as_slice());
    Ok(KeyPair::from_pkcs8_der_and_sign_algo(
        &private_key,
        &PKCS_ED25519,
    )?)
}
