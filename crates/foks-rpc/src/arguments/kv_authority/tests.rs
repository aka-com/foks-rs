use super::*;
use foks_snowpack::encode;

fn personal() -> Value {
    Value::Array(vec![Value::Unsigned(0), Value::Variant(None)])
}
fn team(token: Value) -> Value {
    Value::Array(vec![
        Value::Unsigned(1),
        Value::Variant(Some((b"1".to_vec(), Box::new(token)))),
    ])
}
fn argument(auth: Value) -> Vec<u8> {
    encode(&Value::Array(vec![auth, Value::Null, Value::Unsigned(17)])).unwrap()
}
fn bad(message: &str) -> RpcStatus {
    RpcStatus::BadArguments(message.to_owned())
}

#[test]
fn direct_and_wrapped_selections_keep_original_decoded_owner() {
    for wrapped in [false, true] {
        for auth in [personal(), team(Value::Binary(vec![0x47; 16]))] {
            let team_expected = matches!(&auth, Value::Array(v) if v[0] == Value::Unsigned(1));
            let auth = if wrapped {
                Value::Array(vec![auth, Value::Bool(true)])
            } else {
                auth
            };
            let input = argument(auth);
            let parsed = KvAuthorityArgument::decode(&input).unwrap();
            let Value::Array(fields) = &parsed.root else {
                panic!("root")
            };
            let root_pointer = fields.as_ptr();
            let selection = parsed.classify().unwrap();
            drop(input);
            if team_expected {
                assert!(
                    matches!(selection, KvAuthoritySelection::Team(token) if token == [0x47; 16])
                );
            } else {
                assert!(matches!(selection, KvAuthoritySelection::Personal));
            }
            let Value::Array(fields) = &parsed.root else {
                panic!("root")
            };
            assert_eq!(fields.as_ptr(), root_pointer);
            assert_eq!(fields[2], Value::Unsigned(17));
            // Repeated classification neither consumes the AST nor exposes it.
            assert!(parsed.classify().unwrap() == selection);
        }
    }
}

#[test]
fn pre_clock_shape_failures_and_full_decode_are_preserved() {
    for (value, message) in [
        (Value::Null, "KV argument is not a struct"),
        (
            Value::Array(vec![Value::Null]),
            "KV authentication is not a struct",
        ),
        (
            Value::Array(vec![Value::Array(vec![Value::Unsigned(2)])]),
            "KV authentication is not a struct",
        ),
        (
            Value::Array(vec![Value::Array(vec![Value::Null])]),
            "KV authentication is not a struct",
        ),
    ] {
        assert_eq!(
            KvAuthorityArgument::decode(&encode(&value).unwrap())
                .err()
                .unwrap(),
            bad(message)
        );
    }
    let mut encoded = argument(personal());
    let length = encoded.len();
    encoded.push(0);
    assert_eq!(
        KvAuthorityArgument::decode(&encoded).err().unwrap(),
        bad(&format!("Snowpack error at byte {length}: trailing bytes"))
    );
    // Empty arrays are rejected by canonical decoding before semantic checks.
    for encoded in [&[0x90][..], &[0x91, 0x90][..]] {
        assert!(
            matches!(KvAuthorityArgument::decode(encoded), Err(RpcStatus::BadArguments(message)) if message.contains("empty arrays must be null"))
        );
    }
}

#[test]
fn token_and_variant_errors_remain_after_the_clock_boundary() {
    let cases = [
        (
            Value::Array(vec![Value::Unsigned(0), Value::Null]),
            "unsupported KV authentication",
        ),
        (
            Value::Array(vec![Value::Unsigned(0)]),
            "unsupported KV authentication",
        ),
        (
            Value::Array(vec![Value::Unsigned(1), Value::Variant(None)]),
            "unsupported KV authentication",
        ),
        (
            Value::Array(vec![
                Value::Unsigned(1),
                Value::Variant(Some((b"2".to_vec(), Box::new(Value::Binary(vec![0; 16]))))),
            ]),
            "unsupported KV authentication",
        ),
        (team(Value::Null), "team KV token is not binary"),
        (
            team(Value::Binary(vec![0; 15])),
            "team KV token has the wrong width",
        ),
        (
            team(Value::Binary(vec![0; 17])),
            "team KV token has the wrong width",
        ),
    ];
    for (auth, message) in cases {
        for wrapped in [false, true] {
            let auth = if wrapped {
                Value::Array(vec![auth.clone(), Value::Null])
            } else {
                auth.clone()
            };
            let parsed = KvAuthorityArgument::decode(&argument(auth)).unwrap();
            assert_eq!(parsed.classify().err().unwrap(), bad(message));
        }
    }
    // Unknown types inside the selected auth array reach the post-clock match;
    // a direct leading unknown type instead selects the old header-shape path.
    let wrapped = Value::Array(vec![Value::Array(vec![
        Value::Unsigned(9),
        Value::Variant(None),
    ])]);
    assert_eq!(
        KvAuthorityArgument::decode(&argument(wrapped))
            .unwrap()
            .classify()
            .err()
            .unwrap(),
        bad("unsupported KV authentication")
    );
}
