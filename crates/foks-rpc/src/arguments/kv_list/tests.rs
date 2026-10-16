use super::*;
use crate::{encode_kv_list_request, read_call, KvAuth, DEFAULT_MAX_FRAME_LENGTH};
use foks_snowpack::encode;
use std::io::Cursor;

fn none() -> Value {
    Value::Array(vec![Value::Unsigned(0), Value::Variant(None)])
}
fn variant(discriminant: u64, tag: &[u8], value: Value) -> Value {
    Value::Array(vec![
        Value::Unsigned(discriminant),
        Value::Variant(Some((tag.to_vec(), Box::new(value)))),
    ])
}
fn pagination(cursor: Value, count: Value, load: Value) -> Value {
    Value::Array(vec![cursor, count, load])
}
fn argument(parent: Value, page: Value) -> Value {
    // Opaque on purpose: this decoder is not authentication.
    Value::Array(vec![Value::Null, parent, page])
}
fn parse(value: &Value) -> Result<KvListArguments, RpcStatus> {
    decode_kv_list_arguments(&encode(value).unwrap())
}
fn rejected(value: Value, message: &str) {
    assert_eq!(parse(&value), Err(RpcStatus::BadArguments(message.into())));
}

#[test]
fn request_encoder_parity_normalizes_count_without_narrowing_time() {
    let parent = [7; 16];
    let team = [9; 16];
    for auth in [KvAuth::User, KvAuth::Team(&team)] {
        for cursor in [
            KvListCursor::None,
            KvListCursor::Mac([8; 32]),
            KvListCursor::Time(0),
            KvListCursor::Time(i64::MAX as u64),
            KvListCursor::Time(i64::MAX as u64 + 1),
            KvListCursor::Time(u64::MAX),
        ] {
            for (count, expected) in [
                (0, 4096),
                (1, 1),
                (4096, 4096),
                (4097, 4096),
                (u64::MAX, 4096),
            ] {
                for load_small in [false, true] {
                    let request =
                        encode_kv_list_request(auth, &parent, cursor, count, load_small).unwrap();
                    let call =
                        read_call(&mut Cursor::new(request), DEFAULT_MAX_FRAME_LENGTH).unwrap();
                    assert_eq!(
                        decode_kv_list_arguments(call.argument()).unwrap(),
                        KvListArguments {
                            parent,
                            cursor,
                            number: expected,
                            load_small,
                        }
                    );
                }
            }
        }
    }
}

#[test]
fn field_errors_keep_native_messages_and_structural_precedence() {
    let valid = || pagination(none(), Value::Unsigned(1), Value::Bool(false));
    rejected(Value::Null, "KV argument is not a struct");
    for length in [1, 2, 4] {
        rejected(
            Value::Array(vec![Value::Null; length]),
            "KV argument has the wrong shape",
        );
    }
    // A malformed parent wins even when pagination is also invalid.
    rejected(
        argument(Value::Unsigned(1), Value::Null),
        "KV directory ID is not binary",
    );
    for length in [0, 15, 17] {
        rejected(
            argument(Value::Binary(vec![0; length]), Value::Null),
            "KV directory ID has the wrong width",
        );
    }
    rejected(
        argument(Value::Binary(vec![0; 16]), Value::Null),
        "KV pagination is not a struct",
    );
    for page in [
        Value::Array(vec![Value::Null]),
        Value::Array(vec![Value::Null; 4]),
        pagination(Value::Null, Value::Negative(-1), Value::Bool(false)),
        pagination(Value::Null, Value::Unsigned(1), Value::Unsigned(0)),
    ] {
        rejected(
            argument(Value::Binary(vec![0; 16]), page),
            "KV pagination has the wrong shape",
        );
    }
    let parsed = parse(&argument(Value::Binary(vec![0; 16]), valid())).unwrap();
    assert_eq!(parsed.number, 1);
    assert_eq!(parsed.cursor, KvListCursor::None);
}

#[test]
fn cursor_errors_preserve_union_tags_types_and_exact_widths() {
    let wrap = |cursor| {
        argument(
            Value::Binary(vec![0; 16]),
            pagination(cursor, Value::Unsigned(1), Value::Bool(false)),
        )
    };
    rejected(wrap(Value::Null), "KV list cursor is not a struct");
    for cursor in [
        Value::Array(vec![Value::Unsigned(0)]),
        Value::Array(vec![Value::Unsigned(0), Value::Null]),
        variant(0, b"0", Value::Null),
        variant(1, b"2", Value::Binary(vec![0; 32])),
        variant(2, b"1", Value::Unsigned(0)),
        variant(3, b"3", Value::Unsigned(0)),
        Value::Array(vec![Value::Negative(-1), Value::Variant(None)]),
    ] {
        rejected(wrap(cursor), "unsupported KV list cursor");
    }
    rejected(
        wrap(variant(1, b"1", Value::Unsigned(0))),
        "KV list MAC cursor is not binary",
    );
    for length in [0, 31, 33] {
        rejected(
            wrap(variant(1, b"1", Value::Binary(vec![0; length]))),
            "KV list MAC cursor has the wrong width",
        );
    }
    for value in [Value::Negative(-1), Value::Binary(vec![0; 8]), Value::Null] {
        rejected(
            wrap(variant(2, b"2", value)),
            "KV list time cursor is not unsigned",
        );
    }
}

#[test]
fn malformed_snowpack_keeps_the_existing_decoder_error_text() {
    for bytes in [&[0xc1][..], &[0x93, 0xc0][..], &[0xc0, 0xc0][..]] {
        let expected = foks_snowpack::decode(bytes).unwrap_err().to_string();
        assert_eq!(
            decode_kv_list_arguments(bytes),
            Err(RpcStatus::BadArguments(expected))
        );
    }
}
