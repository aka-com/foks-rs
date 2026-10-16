use super::*;
fn workspace() -> CertificateWorkspace {
    CertificateWorkspace::new(
        crate::net::memory::Pools::new(Default::default())
            .admitted_request(None)
            .unwrap(),
        MAX_DER,
    )
    .unwrap()
}
fn certificate(serial: &[u8]) -> Vec<u8> {
    crate::pki::issue_bounded_device_certificate(
        &crate::keys::MemoryKeyProvider::default(),
        "localhost",
        foks_crypto::ed25519_public_key(&[0x42; 32]),
        serial,
        1_700_000_000_000_000,
        1_800_000_000_000_000,
    )
    .unwrap()
}
#[test]
fn shallow_extraction_matches_native_public_key_and_raw_expiry() {
    let mut parser = workspace();
    for serial in [&[0][..], &[1], &[0, 0, 0x80], &[0x7f; 20], &[0x80; 20]] {
        let der = certificate(serial);
        let fields = parser.parse(&der).unwrap();
        let (_, native) = x509_parser::parse_x509_certificate(&der).unwrap();
        assert_eq!(
            fields.public_key,
            native.public_key().subject_public_key.data.as_ref()
        );
        assert_eq!(
            fields.not_after_seconds,
            native.validity().not_after.timestamp()
        );
        assert_eq!(
            fields.native_expiry_micros().unwrap(),
            1_800_000_000_000_000
        );
        let first = serial
            .iter()
            .position(|b| *b != 0)
            .unwrap_or(serial.len() - 1);
        assert_eq!(fields.serial(), &serial[first..]);
    }
}
#[test]
fn bounds_trailing_and_malformed_rejection_return_fixed_errors() {
    let mut parser = workspace();
    assert_eq!(parser.parse(&[]), Err(CertificateError::Bounds));
    assert_eq!(
        parser.parse(&vec![0; MAX_DER + 1]),
        Err(CertificateError::Bounds)
    );
    assert_eq!(parser.parse(&[0x30, 0]), Err(CertificateError::Malformed));
    let mut valid = certificate(&[7]);
    valid.push(0);
    assert_eq!(parser.parse(&valid), Err(CertificateError::Malformed));
    let mut valid = certificate(&[7]);
    // SubjectPublicKeyInfo's Ed25519 OID occurs alongside signature OIDs. A
    // changed recognized signature algorithm is not a signature-validation claim;
    // only the SPKI key identity is this extractor's policy.
    let (_, native) = x509_parser::parse_x509_certificate(&valid).unwrap();
    let original_key = native
        .public_key()
        .subject_public_key
        .data
        .as_ref()
        .to_vec();
    let offset = valid
        .windows(original_key.len())
        .position(|w| w == original_key)
        .unwrap();
    valid[offset] ^= 1;
    assert_ne!(
        parser.parse(&valid).unwrap().public_key,
        original_key.as_slice()
    );
}
#[test]
fn raw_negative_expiry_is_not_an_import_restriction() {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
    let mut params = rcgen::CertificateParams::default();
    params.serial_number = Some(rcgen::SerialNumber::from_slice(&[0]));
    params.not_before = time::OffsetDateTime::from_unix_timestamp(-100).unwrap();
    params.not_after = time::OffsetDateTime::from_unix_timestamp(-1).unwrap();
    let cert = params.self_signed(&key).unwrap();
    let fields = workspace().parse(cert.der()).unwrap();
    assert_eq!(fields.not_after_seconds, -1);
    assert_eq!(fields.serial(), &[0]);
    assert_eq!(fields.native_expiry_micros(), Err(CertificateError::Expiry));
}
#[test]
fn reviewed_parser_dependencies_and_layout_formula_are_pinned() {
    let lock = include_str!("../../../../../Cargo.lock");
    for (name, version, checksum) in [
        (
            "x509-parser",
            "0.18.1",
            "d43b0f71ce057da06bc0851b23ee24f3f86190b07203dd8f567d0b706a185202",
        ),
        (
            "asn1-rs",
            "0.7.2",
            "b7f43a50ac4fdca5df8e885c21b835997f0a1cdee65494a6847694a98652d9d8",
        ),
        (
            "der-parser",
            "10.0.0",
            "07da5016415d5a3c4dd39b11ed26f915f52fc4e0dc197d87908bc916e51bc1a6",
        ),
        (
            "nom",
            "7.1.3",
            "d273983c5a657a70a3e8f2a01329822f3b8c8172b73826411a55751e404a0a4a",
        ),
        (
            "num-bigint",
            "0.4.8",
            "c89e69e7e0f03bea5ef08013795c25018e101932225a656383bd384495ecc367",
        ),
    ] {
        let entry = lock
            .split("[[package]]")
            .find(|entry| entry.contains(&format!("name = {name:?}\nversion = {version:?}")))
            .expect("reviewed dependency version");
        assert!(entry.contains(&format!("checksum = {checksum:?}")));
    }
    for n in [1usize, 6, 8, 31, 32, 255, 256, 4095, 4096] {
        assert_eq!(
            allowance(n),
            Some(
                4 * (n / 6) * size_of::<AttributeTypeAndValue>()
                    + 4 * (n / 8 + 2) * size_of::<RelativeDistinguishedName>()
                    + 4 * (n / 6 + 1) * size_of::<X509Extension>()
                    + n
                    + 2 * ((n + 7) & !7)
                    + 106
            )
        );
    }
    assert_eq!(allowance(0), None);
    assert_eq!(allowance(4097), None);
    let policy: serde_json::Value =
        serde_json::from_slice(include_bytes!("../profile/certificate-policy.json")).unwrap();
    assert_eq!(
        policy["asn1_features"],
        serde_json::json!(["datetime", "default", "std", "time"])
    );
    assert_eq!(
        policy["forbidden_asn1_features"],
        serde_json::json!(["bigint", "debug", "trace"])
    );
}

fn tlv(tag: u8, value: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    if value.len() < 128 {
        out.push(value.len() as u8);
    } else if value.len() < 256 {
        out.extend([0x81, value.len() as u8]);
    } else {
        out.push(0x82);
        out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    }
    out.extend_from_slice(value);
    out
}
fn manual(spki_oid: &[u8], extension: bool) -> Vec<u8> {
    let algorithm = tlv(0x30, &tlv(0x06, &[0x2b, 0x65, 0x70]));
    let mut time = tlv(0x17, b"230101000000Z");
    time.extend(tlv(0x17, b"330101000000Z"));
    let mut spki = tlv(0x30, &tlv(0x06, spki_oid));
    let mut key = vec![0];
    key.extend([4; 32]);
    spki.extend(tlv(0x03, &key));
    let mut tbs = tlv(0xa0, &tlv(0x02, &[2]));
    tbs.extend(tlv(0x02, &[0]));
    tbs.extend(&algorithm);
    tbs.extend(tlv(0x30, &[]));
    tbs.extend(tlv(0x30, &time));
    tbs.extend(tlv(0x30, &[]));
    tbs.extend(tlv(0x30, &spki));
    if extension {
        let mut ext = tlv(0x06, &[0x55, 0x1d, 0x13]);
        ext.extend(tlv(0x04, &[0xff]));
        tbs.extend(tlv(0xa3, &tlv(0x30, &tlv(0x30, &ext))));
    }
    let mut body = tlv(0x30, &tbs);
    body.extend(algorithm);
    body.extend(tlv(0x03, &[0, 1]));
    tlv(0x30, &body)
}
#[test]
fn borrowed_numeric_oid_preserves_native_nonminimal_arcs_and_invalid_extension_behavior() {
    let mut long = vec![0x2b];
    long.extend([0x80; 2048]);
    long.extend([0x65, 0x70]);
    for oid in [
        vec![0x2b, 0x65, 0x70],
        vec![0x2b, 0x80, 0x65, 0x70],
        vec![0x2b, 0x65, 0x80],
        vec![0x2b, 0x65, 0x71],
        long,
    ] {
        for extension in [false, true] {
            let der = manual(&oid, extension);
            let native = x509_parser::parse_x509_certificate(&der);
            let expected = native.as_ref().is_ok_and(|(_, cert)| {
                cert.public_key().algorithm.algorithm.to_id_string() == "1.3.101.112"
                    && cert.public_key().subject_public_key.unused_bits == 0
                    && cert.public_key().subject_public_key.data.len() == 32
            });
            let actual = workspace().parse(&der);
            assert_eq!(
                actual.is_ok(),
                expected,
                "OID length {}, extension {extension}",
                oid.len()
            );
            if oid == [0x2b, 0x80, 0x65, 0x70] {
                assert!(actual.is_ok(), "native accepts a nonminimal numeric OID");
            }
        }
    }
}
