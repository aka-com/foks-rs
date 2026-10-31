use super::*;
pub(super) fn domain() -> Domain {
    Domain(DomainValue::Darwin {
        boot: BootId::parse("12345678-1234-1234-1234-123456789abc").unwrap(),
    })
}
fn envelope() -> DeadlineEnvelope {
    DeadlineEnvelope {
        version: Version,
        domain: domain(),
        expires_ns: Decimal(u64::MAX),
    }
}
#[test]
fn strict_envelope_round_trips_exact_integers() {
    let value = serde_json::to_value(envelope()).unwrap();
    assert_eq!(value["expires_ns"], "18446744073709551615");
    assert_eq!(
        serde_json::from_value::<DeadlineEnvelope>(value).unwrap(),
        envelope()
    );
}
#[test]
fn noncanonical_or_unsupported_wire_context_is_rejected() {
    let original = serde_json::to_value(envelope()).unwrap();
    for bad in [
        serde_json::json!(1),
        serde_json::json!(-1),
        serde_json::json!(1.0),
        serde_json::json!(""),
        serde_json::json!("01"),
        serde_json::json!("+1"),
        serde_json::json!("-1"),
        serde_json::json!(" 1"),
        serde_json::json!("1.0"),
        serde_json::json!("18446744073709551616"),
    ] {
        let mut value = original.clone();
        value["expires_ns"] = bad;
        assert!(serde_json::from_value::<DeadlineEnvelope>(value).is_err());
    }
    for version in [
        serde_json::json!(0),
        serde_json::json!(2),
        serde_json::json!("1"),
    ] {
        let mut value = original.clone();
        value["version"] = version;
        assert!(serde_json::from_value::<DeadlineEnvelope>(value).is_err());
    }
    for field in ["version", "domain", "expires_ns"] {
        let mut value = original.clone();
        value.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<DeadlineEnvelope>(value).is_err());
    }
    let mut value = original.clone();
    value["extra"] = serde_json::json!(true);
    assert!(serde_json::from_value::<DeadlineEnvelope>(value).is_err());
    let mut value = original.clone();
    value["domain"]["extra"] = serde_json::json!(true);
    assert!(serde_json::from_value::<DeadlineEnvelope>(value).is_err());
    let mut value = original.clone();
    value["domain"]["clock"] = serde_json::json!("wallclock");
    assert!(serde_json::from_value::<DeadlineEnvelope>(value).is_err());
    for uuid in [
        "12345678-1234-1234-1234-123456789ABC",
        "bad",
        "12345678_1234-1234-1234-123456789abc",
    ] {
        let mut value = original.clone();
        value["domain"]["boot"] = serde_json::json!(uuid);
        assert!(serde_json::from_value::<DeadlineEnvelope>(value).is_err());
    }
}

#[test]
fn linux_namespace_identity_has_exact_string_integers() {
    let domain = Domain(DomainValue::Linux {
        boot: BootId::parse("12345678-1234-1234-1234-123456789abc").unwrap(),
        device: Decimal(u64::MAX),
        inode: Decimal(0),
    });
    let value = serde_json::to_value(&domain).unwrap();
    assert_eq!(
        serde_json::from_value::<Domain>(value.clone()).unwrap(),
        domain
    );
    for field in ["device", "inode"] {
        for bad in [
            serde_json::json!(1),
            serde_json::json!("01"),
            serde_json::json!("18446744073709551616"),
        ] {
            let mut altered = value.clone();
            altered[field] = bad;
            assert!(serde_json::from_value::<Domain>(altered).is_err());
        }
    }
}
#[test]
fn boot_ids_convert_the_hyphenated_form_to_bytes() {
    let boot = BootId::parse("0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0").unwrap();
    assert_eq!(
        boot.bytes(),
        [
            0x0f, 0x1e, 0x2d, 0x3c, 0x4b, 0x5a, 0x69, 0x78, 0x87, 0x96, 0xa5, 0xb4, 0xc3, 0xd2,
            0xe1, 0xf0
        ]
    );
    for text in [
        "",
        "0F1E2D3C-4B5A-6978-8796-A5B4C3D2E1F0",
        "0f1e2d3c4b5a69788796a5b4c3d2e1f0",
        "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f",
        "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1fg",
        "+f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0",
        "0f1e2d3c-4b5a-6978-8796a-5b4c3d2e1f0",
    ] {
        assert!(BootId::parse(text).is_err(), "{text}");
    }
}
