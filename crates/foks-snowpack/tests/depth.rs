use foks_snowpack::{decode, encode, encode_ref, ErrorKind, Value, ValueRef, MAX_DEPTH};

fn nested(depth: usize) -> Value {
    (0..depth).fold(Value::Null, |value, _| Value::Array(vec![value]))
}

fn nested_ref(depth: usize) -> ValueRef<'static> {
    (0..depth).fold(ValueRef::Null, |value, _| ValueRef::Array(vec![value]))
}

#[test]
fn maximum_depth_is_accepted() {
    let value = nested(MAX_DEPTH);
    let encoded = encode(&value).unwrap();
    assert_eq!(decode(&encoded).unwrap(), value);
    assert_eq!(encode_ref(&nested_ref(MAX_DEPTH)).unwrap(), encoded);
}

#[test]
fn depth_above_maximum_is_rejected_by_both_directions() {
    let value = nested(MAX_DEPTH + 1);
    assert_eq!(encode(&value).unwrap_err().kind, ErrorKind::DepthLimit);
    assert_eq!(
        encode_ref(&nested_ref(MAX_DEPTH + 1)).unwrap_err().kind,
        ErrorKind::DepthLimit
    );

    let mut encoded = vec![0x91; MAX_DEPTH + 1];
    encoded.push(0xc0);
    assert_eq!(decode(&encoded).unwrap_err().kind, ErrorKind::DepthLimit);
}
