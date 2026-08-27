use rcgen::{
    Certificate, CertificateParams, CertifiedIssuer, ExtendedKeyUsagePurpose, KeyUsagePurpose,
    PublicKeyData, SignatureAlgorithm, SigningKey, PKCS_ED25519,
};

struct DetachedEd25519PublicKey([u8; 32]);

impl PublicKeyData for DetachedEd25519PublicKey {
    fn der_bytes(&self) -> &[u8] {
        &self.0
    }

    fn algorithm(&self) -> &'static SignatureAlgorithm {
        &PKCS_ED25519
    }
}

/// Issues a client certificate for an already-held Ed25519 device key.
///
/// The issuer receives only the public key. Proof of private-key possession is
/// performed later by the authenticated TLS listener.
pub fn issue_ed25519_client_certificate<S: SigningKey>(
    public_key: [u8; 32],
    issuer: &CertifiedIssuer<'_, S>,
) -> Result<Certificate, rcgen::Error> {
    let mut parameters = CertificateParams::default();
    parameters.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    parameters.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    parameters.signed_by(&DetachedEd25519PublicKey(public_key), issuer)
}
