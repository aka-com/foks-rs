use foks_snowpack::{decode, encode, ErrorKind, Value, MAX_DEPTH};

fn nested(depth: usize) -> Value {
    (0..depth).fold(Value::Null, |value, _| Value::Array(vec![value]))
}

#[test]
fn maximum_depth_is_accepted() {
    let value = nested(MAX_DEPTH);
    let encoded = encode(&value).unwrap();
    assert_eq!(decode(&encoded).unwrap(), value);
}

#[test]
fn depth_above_maximum_is_rejected_by_both_directions() {
    let value = nested(MAX_DEPTH + 1);
    assert_eq!(encode(&value).unwrap_err().kind, ErrorKind::DepthLimit);

    let mut encoded = vec![0x91; MAX_DEPTH + 1];
    encoded.push(0xc0);
    assert_eq!(decode(&encoded).unwrap_err().kind, ErrorKind::DepthLimit);
}
