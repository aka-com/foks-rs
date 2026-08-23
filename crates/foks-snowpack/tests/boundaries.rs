use foks_snowpack::{decode, encode, ErrorKind, Value};

#[test]
fn unsigned_integer_boundaries_round_trip() {
    for value in [
        0,
        1,
        127,
        128,
        255,
        256,
        65_535,
        65_536,
        u64::from(u32::MAX),
        u64::from(u32::MAX) + 1,
        u64::MAX,
    ] {
        let value = Value::Unsigned(value);
        assert_eq!(decode(&encode(&value).unwrap()).unwrap(), value);
    }
}

#[test]
fn negative_integer_boundaries_round_trip() {
    for value in [
        -1,
        -32,
        -33,
        i64::from(i8::MIN),
        i64::from(i8::MIN) - 1,
        i64::from(i16::MIN),
        i64::from(i16::MIN) - 1,
        i64::from(i32::MIN),
        i64::from(i32::MIN) - 1,
        i64::MIN,
    ] {
        let value = Value::Negative(value);
        assert_eq!(decode(&encode(&value).unwrap()).unwrap(), value);
    }
}

#[test]
fn binary_length_boundaries_round_trip() {
    for length in [0, 1, 255, 256, 65_535, 65_536] {
        let value = Value::Binary(vec![0x5a; length]);
        assert_eq!(decode(&encode(&value).unwrap()).unwrap(), value);
    }
}

#[test]
fn text_length_boundaries_round_trip() {
    for length in [0, 1, 31, 32, 255, 256, 65_535, 65_536] {
        let value = Value::Text(vec![0xa5; length]);
        assert_eq!(decode(&encode(&value).unwrap()).unwrap(), value);
    }
}

#[test]
fn array_header_boundaries_match_foks_v019() {
    for length in [1, 15, 16, 17, 31, 32, 33, 255] {
        let value = Value::Array(vec![Value::Null; length]);
        assert_eq!(decode(&encode(&value).unwrap()).unwrap(), value);
    }
}

#[test]
fn variants_have_only_zero_or_one_short_tagged_entry() {
    let none = Value::Variant(None);
    assert_eq!(encode(&none).unwrap(), vec![0x80]);
    assert_eq!(decode(&[0x80]).unwrap(), none);

    let tagged = Value::Variant(Some((b"1".to_vec(), Box::new(Value::Bool(true)))));
    assert_eq!(encode(&tagged).unwrap(), vec![0x81, 0xa1, b'1', 0xc3]);
    assert_eq!(decode(&encode(&tagged).unwrap()).unwrap(), tagged);

    let long = Value::Variant(Some((vec![b'x'; 32], Box::new(Value::Null))));
    assert_eq!(
        encode(&long).unwrap_err().kind,
        ErrorKind::InvalidVariantTag
    );
}

#[test]
fn length_headers_must_be_minimal() {
    let cases: &[&[u8]] = &[
        &[0xc5, 0, 0],
        &[0xc6, 0, 0, 0, 0],
        &[0xd9, 0],
        &[0xda, 0, 0],
        &[0xdb, 0, 0, 0, 0],
        &[0xdc, 0, 1, 0xc0],
        &[0xdd, 0, 0, 0, 1, 0xc0],
    ];
    for bytes in cases {
        assert!(decode(bytes).is_err(), "{bytes:02x?}");
    }
}
