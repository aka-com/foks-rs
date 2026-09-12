use super::*;

fn positional(tag: &[u8], fields: Vec<Value>) -> Vec<u8> {
    encode(&Value::Array(vec![
        Value::Unsigned(211),
        Value::Variant(Some((tag.to_vec(), Box::new(Value::Array(fields))))),
    ]))
    .unwrap()
}

fn missing_method_fields() -> Vec<Value> {
    vec![
        Value::Unsigned(REAL_TIME_PROTOCOL_ID),
        Value::Unsigned(RT_CHAT_CAPABILITIES_METHOD_POSITION),
        Value::Text(b"RealTime".to_vec()),
    ]
}

fn named(tag: &[u8], keys: [&[u8]; 3]) -> Vec<u8> {
    let mut bytes = vec![0x82];
    encode_text(b"Sc", &mut bytes);
    encode_unsigned(211, &mut bytes);
    encode_text(tag, &mut bytes);
    bytes.push(0x83);
    for (key, value) in keys.into_iter().zip(missing_method_fields()) {
        encode_text(key, &mut bytes);
        bytes.extend(encode(&value).unwrap());
    }
    bytes
}

#[test]
fn missing_method_status_preserves_exact_identity_in_both_encodings() {
    for bytes in [
        positional(b"3", missing_method_fields()),
        named(b"f3", [b"Proto", b"Method", b"Name"]),
    ] {
        assert!(matches!(
            check_status(&bytes),
            Err(Error::MethodNotFound {
                protocol_id: REAL_TIME_PROTOCOL_ID,
                position: RT_CHAT_CAPABILITIES_METHOD_POSITION
            })
        ));
    }
}

#[test]
fn malformed_missing_method_never_authorizes_fallback() {
    for bytes in [
        positional(b"4", missing_method_fields()),
        positional(b"3", missing_method_fields()[..2].to_vec()),
        positional(
            b"3",
            vec![
                Value::Text(b"1".to_vec()),
                Value::Unsigned(1),
                Value::Text(b"RealTime".to_vec()),
            ],
        ),
        named(b"f4", [b"Proto", b"Method", b"Name"]),
        named(b"f3", [b"Proto", b"Proto", b"Name"]),
        named(b"f3", [b"Proto", b"Method", b"Other"]),
    ] {
        assert!(matches!(check_status(&bytes), Err(Error::Envelope { .. })));
    }
    let malformed_unsupported = encode(&Value::Array(vec![
        Value::Unsigned(STATUS_NOT_IMPLEMENTED),
        Value::Variant(Some((b"0".to_vec(), Box::new(Value::Unsigned(0))))),
    ]))
    .unwrap();
    assert!(matches!(
        check_status(&malformed_unsupported),
        Err(Error::Envelope { .. })
    ));
}
