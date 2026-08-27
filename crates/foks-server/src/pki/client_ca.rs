use rcgen::{
    Certificate, CertificateParams, CertifiedIssuer, ExtendedKeyUsagePurpose, KeyUsagePurpose,
    PublicKeyData, SerialNumber, SignatureAlgorithm, SigningKey, PKCS_ED25519,
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

pub(crate) fn issue_bounded_device_certificate(
    provider: &dyn crate::keys::HostKeyProvider,
    canonical_name: &str,
    public_key: [u8; 32],
    serial: &[u8],
    not_before_micros: u64,
    not_after_micros: u64,
) -> crate::Result<Vec<u8>> {
    let issuer = super::host_tls::client_identity_ca(provider, canonical_name)?;
    let mut parameters = CertificateParams::default();
    parameters.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    parameters.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    parameters.serial_number = Some(SerialNumber::from_slice(serial));
    parameters.not_before = timestamp(not_before_micros)?;
    parameters.not_after = timestamp(not_after_micros)?;
    Ok(parameters
        .signed_by(&DetachedEd25519PublicKey(public_key), &issuer)?
        .der()
        .to_vec())
}

fn timestamp(microseconds: u64) -> crate::Result<time::OffsetDateTime> {
    let seconds = i64::try_from(microseconds / 1_000_000)
        .map_err(|_| crate::Error::Config("certificate timestamp overflow"))?;
    time::OffsetDateTime::from_unix_timestamp(seconds)
        .map_err(|_| crate::Error::Config("certificate timestamp out of range"))
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
