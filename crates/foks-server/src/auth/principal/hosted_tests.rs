use super::*;
use foks_metadata_core::personal::MAX_PEER_CERTIFICATE_BYTES;

fn issued(serial: &[u8]) -> (CertificateDer<'static>, [u8; 32]) {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
    let public = key.public_key_raw().try_into().unwrap();
    let der = crate::pki::issue_bounded_device_certificate(
        &crate::keys::MemoryKeyProvider::default(),
        "localhost",
        public,
        serial,
        1_700_000_000_000_000,
        1_800_000_000_000_000,
    )
    .unwrap();
    (CertificateDer::from(der), public)
}

#[test]
fn actual_issuer_serial_normalization_preserves_exact_certificate_and_native_key() {
    for width in 1..=20 {
        for leading in [0x01, 0x80, 0xff] {
            let mut serial = vec![0; 20 - width];
            serial.push(leading);
            serial.extend(std::iter::repeat_n(0x5a, width - 1));
            let (der, public) = issued(&serial);
            let peer = parse_hosted_peer_certificate(&der).unwrap();
            assert_eq!(peer.exact_der(), der.as_ref());
            assert_eq!(peer.serial_magnitude(), &serial[20 - width..]);
            assert_eq!(*peer.public_key(), public);
            assert_eq!(certificate_public_key(&der).unwrap(), public);
            assert_eq!(peer.expires_at_micros(), 1_800_000_000_000_000);
        }
    }
    let (zero, _) = issued(&[0; 20]);
    assert_eq!(
        parse_hosted_peer_certificate(&zero)
            .unwrap()
            .serial_magnitude(),
        &[0]
    );
}

#[test]
fn serial_boundaries_reject_negative_and_oversized_magnitudes_without_truncation() {
    assert!(hosted_serial_magnitude(&[]).is_err());
    assert!(hosted_serial_magnitude(&[0x80]).is_err());
    assert!(hosted_serial_magnitude(&[1; 21]).is_err());
    let mut sign_padded = vec![0];
    sign_padded.extend([0xff; 20]);
    assert_eq!(hosted_serial_magnitude(&sign_padded).unwrap(), &[0xff; 20]);
    assert_eq!(hosted_serial_magnitude(&[0, 0, 0]).unwrap(), &[0]);
    let (oversized, _) = issued(&[1; 21]);
    assert!(parse_hosted_peer_certificate(&oversized).is_err());
}

#[test]
fn malformed_oversized_and_trailing_certificates_are_rejected() {
    for bytes in [
        vec![],
        vec![1, 2, 3],
        vec![0; MAX_PEER_CERTIFICATE_BYTES + 1],
    ] {
        assert!(parse_hosted_peer_certificate(&CertificateDer::from(bytes)).is_err());
    }
    let (der, _) = issued(&[1]);
    let mut trailing = der.to_vec();
    trailing.push(0);
    assert!(parse_hosted_peer_certificate(&CertificateDer::from(trailing)).is_err());
    let mut truncated = der.to_vec();
    truncated.pop();
    assert!(parse_hosted_peer_certificate(&CertificateDer::from(truncated)).is_err());
}

#[test]
fn unsupported_public_key_and_unused_bits_are_rejected() {
    let ec_key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let ec = rcgen::CertificateParams::default()
        .self_signed(&ec_key)
        .unwrap();
    assert!(parse_hosted_peer_certificate(ec.der()).is_err());

    let (der, public) = issued(&[1]);
    let mut malformed = der.to_vec();
    let encoded = [&[0x03, 0x21, 0][..], &public].concat();
    let positions: Vec<_> = malformed
        .windows(encoded.len())
        .enumerate()
        .filter_map(|(i, bytes)| (bytes == encoded).then_some(i))
        .collect();
    assert_eq!(positions.len(), 1);
    malformed[positions[0] + 2] = 1;
    assert!(parse_hosted_peer_certificate(&CertificateDer::from(malformed)).is_err());
}

#[test]
fn negative_expiry_is_rejected_instead_of_wrapping() {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
    let mut params = rcgen::CertificateParams::default();
    params.not_before = time::OffsetDateTime::from_unix_timestamp(-2).unwrap();
    params.not_after = time::OffsetDateTime::from_unix_timestamp(-1).unwrap();
    params.serial_number = Some(rcgen::SerialNumber::from_slice(&[1]));
    let cert = params.self_signed(&key).unwrap();
    assert!(parse_hosted_peer_certificate(cert.der()).is_err());
}
