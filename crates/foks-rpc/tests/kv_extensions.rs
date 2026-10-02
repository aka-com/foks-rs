use foks_proto::KvEmptyDirectoryAssertion;
use foks_rpc::{
    decode_kv_empty_directory_capability_response, encode_kv_capabilities_request_at,
    encode_kv_put_request_at, encode_kv_put_with_empty_directories_request_at, read_call, KvAuth,
    KV_CAPABILITIES_POSITION, KV_EXTENSIONS_PROTOCOL_ID,
};
use foks_snowpack::{decode, encode, Value};

#[test]
fn capability_is_explicit_versioned_and_fails_closed() {
    let request = encode_kv_capabilities_request_at(7).unwrap();
    let call = read_call(&mut request.as_slice(), 4096).unwrap();
    assert_eq!(call.protocol_id(), KV_EXTENSIONS_PROTOCOL_ID);
    assert_eq!(call.method_position(), KV_CAPABILITIES_POSITION);
    assert_eq!(call.sequence(), 7);
    assert_eq!(decode(call.argument()).unwrap(), Value::Null);
    for supported in [true, false] {
        assert_eq!(
            decode_kv_empty_directory_capability_response(
                &encode(&Value::Array(vec![
                    Value::Unsigned(1),
                    Value::Bool(supported),
                ]))
                .unwrap()
            )
            .unwrap(),
            supported
        );
    }
    for value in [
        Value::Null,
        Value::Unsigned(1),
        Value::Array(vec![Value::Unsigned(1)]),
        Value::Array(vec![Value::Unsigned(2), Value::Bool(true)]),
        Value::Array(vec![Value::Unsigned(1), Value::Unsigned(1)]),
        Value::Array(vec![Value::Unsigned(1), Value::Bool(true), Value::Null]),
    ] {
        assert!(decode_kv_empty_directory_capability_response(&encode(&value).unwrap()).is_err());
    }
}

#[test]
fn asserted_put_is_distinct_bounded_and_preserves_legacy_encoding() {
    let assertion = KvEmptyDirectoryAssertion {
        parent: [1; 16],
        dirent: [2; 16],
        version: 3,
        directory: [4; 16],
    };
    let listing = foks_proto::KvListResponse::decode(include_bytes!(
        "../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/kv-list.snowp"
    ))
    .unwrap();
    let dirents = &listing.entries[..1];
    let legacy = encode_kv_put_request_at(KvAuth::User, None, dirents, 5).unwrap();
    assert_eq!(
        legacy,
        encode_kv_put_with_empty_directories_request_at(KvAuth::User, None, dirents, &[], 5)
            .unwrap()
    );
    let request = encode_kv_put_with_empty_directories_request_at(
        KvAuth::User,
        None,
        dirents,
        &[assertion],
        5,
    )
    .unwrap();
    let call = read_call(&mut request.as_slice(), 4096).unwrap();
    assert_eq!(call.protocol_id(), KV_EXTENSIONS_PROTOCOL_ID);
    assert_eq!(
        call.method_position(),
        foks_rpc::KV_PUT_EMPTY_DIRECTORIES_POSITION
    );
    let Value::Array(fields) = decode(call.argument()).unwrap() else {
        panic!("not tuple")
    };
    assert_eq!(fields.len(), 3);
    assert_eq!(fields[2], Value::Array(vec![assertion.to_value()]));
    assert_eq!(
        KvEmptyDirectoryAssertion::from_value(&assertion.to_value()).unwrap(),
        assertion
    );
    assert!(KvEmptyDirectoryAssertion::from_value(&Value::Array(vec![Value::Null; 4])).is_err());
    assert!(encode_kv_put_with_empty_directories_request_at(
        KvAuth::User,
        None,
        &[],
        &[assertion; 65],
        5
    )
    .is_err());
}
