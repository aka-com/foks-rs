use super::*;
use crate::{catalog::Kind, Statement};
use foks_snowpack::{with_allocation_budget, AllocationBudget};
use std::{
    mem::size_of,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Budget {
    charges: Mutex<Vec<usize>>,
    reject: Option<usize>,
}
impl AllocationBudget for Budget {
    fn reserve(&self, bytes: usize) -> bool {
        let mut charges = self.charges.lock().unwrap();
        let index = charges.len();
        charges.push(bytes);
        self.reject != Some(index)
    }
}
fn budget(reject: Option<usize>) -> Arc<Budget> {
    Arc::new(Budget {
        reject,
        ..Budget::default()
    })
}
fn spec(columns: &'static [Kind]) -> Spec {
    Spec {
        id: 0,
        sql: "",
        parameters: &[],
        columns,
        write: false,
        maximum_blob_bytes: 0,
    }
}
fn parser_charge(bytes: &[u8]) -> usize {
    3 * bytes.len() + 2344
}

#[test]
fn exact_writer_matches_unchanged_serde_for_all_bytes_unicode_and_i64() {
    let text = format!(
        "{}é漢🦀/\\\"",
        (0u8..=127).map(char::from).collect::<String>()
    );
    for n in [i64::MIN, -100, -10, -1, 0, 1, 9, 10, 99, 100, i64::MAX] {
        let values = [
            Value::Integer(n),
            Value::Blob((0u8..=255).collect()),
            Value::Null,
            Value::Text(text.clone()),
        ];
        let expected = serde_json::to_vec(&values).unwrap();
        let recorder = budget(None);
        let encoded = with_allocation_budget(recorder.clone(), || encode(&values)).unwrap();
        assert_eq!(encoded, expected);
        assert_eq!(*recorder.charges.lock().unwrap(), [expected.len()]);
    }
    assert_eq!(encode(&[]).unwrap(), b"[]");
    assert_eq!(
        encode(&[Value::Blob(vec![]), Value::Text(String::new())]).unwrap(),
        br#"[{"blob":[]},{"text":""}]"#
    );
}

#[test]
fn writer_bounds_and_refusal_precede_storage_and_never_grow_past_plan() {
    let refusal = budget(Some(0));
    assert_eq!(
        with_allocation_budget(refusal, || encode(&[Value::Integer(1)])),
        Err(Error::Bounds)
    );
    let large = vec![Value::Blob(vec![255; 4096]); 5];
    let recorder = budget(None);
    assert_eq!(
        with_allocation_budget(recorder.clone(), || encode(&large)),
        Err(Error::Bounds)
    );
    assert!(recorder.charges.lock().unwrap().is_empty());
    let mut writer = Writer::new(1).unwrap();
    assert_eq!(writer.push(b"aa"), Err(Error::Bounds));
    assert!(writer.bytes.is_empty());
    assert_eq!(writer.finish(), Err(Error::Bounds));
    assert_eq!(add(usize::MAX, 1), Err(Error::Bounds));
}

#[test]
fn query_and_changes_preserve_exact_values_and_escaped_text() {
    let values = vec![
        Value::Integer(i64::MIN),
        Value::Blob((0u8..=255).collect()),
        Value::Null,
        Value::Text("é\0\"\\🦀".into()),
    ];
    let bytes = serde_json::to_vec(&values).unwrap();
    assert_eq!(
        decode_query(&Statement::ProbeTypes.spec(), &bytes),
        Ok(Some(values))
    );
    let escaped = br#" [ {"te\u0078t":"\u00e9\u0000\ud83e\udd80\/"} ] "#;
    assert_eq!(
        decode_query(&spec(&[Kind::Text]), escaped),
        Ok(Some(vec![Value::Text("é\0🦀/".into())]))
    );
    assert_eq!(decode_changes(br#"{"inte\u0067er":"9\u0030"}"#), Ok(90));
    assert_eq!(
        decode_changes(br#"{"integer":"9223372036854775807"}"#),
        Ok(i64::MAX as u64)
    );
    assert_eq!(
        decode_changes(br#"{"integer":"-1"}"#),
        Err(Error::IntegerRange)
    );
}

#[test]
fn row_and_payload_admission_have_explicit_refusal_provenance() {
    let bytes = br#"[{"blob":[1,2]},{"text":"abc"}]"#;
    for refused in 0..4 {
        let recorder = budget(Some(refused));
        assert_eq!(
            with_allocation_budget(recorder.clone(), || decode_query(
                &spec(&[Kind::Blob, Kind::Text]),
                bytes
            )),
            Err(Error::Bounds)
        );
        assert_eq!(recorder.charges.lock().unwrap().len(), refused + 1);
    }
    let recorder = budget(None);
    let result = with_allocation_budget(recorder.clone(), || {
        decode_query(&spec(&[Kind::Blob, Kind::Text]), bytes)
    })
    .unwrap();
    assert_eq!(
        result,
        Some(vec![Value::Blob(vec![1, 2]), Value::Text("abc".into())])
    );
    assert_eq!(
        *recorder.charges.lock().unwrap(),
        [parser_charge(bytes), 2 * size_of::<Value>(), 4096, 3]
    );
    let none = budget(None);
    assert_eq!(
        with_allocation_budget(none.clone(), || decode_query(&spec(&[Kind::Text]), b"null")),
        Ok(None)
    );
    assert_eq!(*none.charges.lock().unwrap(), [parser_charge(b"null")]);
}

#[test]
fn row_shape_types_and_canonical_integer_errors_are_host_storage_failures() {
    for bytes in [
        br#"[]"#.as_slice(),
        br#"[null]"#,
        br#"["null"]"#,
        br#"[{"text":"1"}]"#,
        br#"[{"integer":"01"}]"#,
        br#"[{"integer":"+1"}]"#,
        br#"[{"integer":"-0"}]"#,
        br#"[{"integer":"9223372036854775808"}]"#,
        br#"[{"integer":1}]"#,
        br#"[{"integer":"1","integer":"1"}]"#,
        br#"[{"integer":"1"},{"integer":"2"}]"#,
        br#"[{"integer":"1"}] garbage"#,
    ] {
        assert_eq!(
            decode_query(&spec(&[Kind::Integer]), bytes),
            Err(Error::Storage),
            "{bytes:?}"
        );
    }
    for bytes in [
        br#"null"#.as_slice(),
        br#"{"integer":"01"}"#,
        br#"{"integer":1}"#,
        br#"{"integer":"1","x":"y"}"#,
    ] {
        assert_eq!(decode_changes(bytes), Err(Error::Storage));
    }
    for bytes in [
        br#"[{"blob":[-1]}]"#.as_slice(),
        br#"[{"blob":[1.0]}]"#,
        br#"[{"blob":[1e0]}]"#,
        br#"[{"blob":[256]}]"#,
        br#"[{"blob":[01]}]"#,
        br#"[{"blob":[null]}]"#,
    ] {
        assert_eq!(
            decode_query(&spec(&[Kind::Blob]), bytes),
            Err(Error::Storage)
        );
    }
}

#[test]
fn global_cell_bounds_and_oversize_envelope_remain_explicit() {
    for kind in [Kind::Text, Kind::Blob] {
        for length in [4096, 4097] {
            let value = if kind == Kind::Text {
                Value::Text("a".repeat(length))
            } else {
                Value::Blob(vec![255; length])
            };
            let bytes = serde_json::to_vec(std::slice::from_ref(&value)).unwrap();
            let columns = if kind == Kind::Text {
                &[Kind::Text][..]
            } else {
                &[Kind::Blob][..]
            };
            let result = decode_query(&spec(columns), &bytes);
            if length == 4096 {
                assert_eq!(result, Ok(Some(vec![value])));
            } else {
                assert_eq!(result, Err(Error::Storage));
            }
        }
    }
    let recorder = budget(None);
    assert_eq!(
        with_allocation_budget(recorder.clone(), || decode_changes(&vec![
            b' ';
            MAX_BYTES + 1
        ])),
        Err(Error::Bounds)
    );
    assert!(recorder.charges.lock().unwrap().is_empty());
}

#[test]
fn malformed_escaped_and_feature_sensitive_tokens_are_bounded_without_echo() {
    for bytes in [
        br#"[{"text":"\uD800"}]"#.as_slice(),
        br#"[{"text":"\uDC00"}]"#,
        br#"[{"text":"\q"}]"#,
        b"[{\"text\":\"\xff\"}]",
        br#"[[[[]]]]"#,
    ] {
        assert_eq!(
            decode_query(&spec(&[Kind::Text]), bytes),
            Err(Error::Storage)
        );
    }
    let mut bytes = b"[{\"".to_vec();
    bytes.extend_from_slice(br#"\u0078"#);
    bytes.extend(std::iter::repeat_n(b'x', MAX_BYTES - 21));
    bytes.extend_from_slice(br#"":"z"}]"#);
    assert!(bytes.len() <= MAX_BYTES);
    assert_eq!(
        decode_query(&spec(&[Kind::Text]), &bytes),
        Err(Error::Storage)
    );
    assert!(!decode::diagnostic(&bytes).contains("xxxxxxxx"));
    let raw = vec![b'9'; MAX_BYTES];
    assert_eq!(
        decode_query(&spec(&[Kind::Text]), &raw),
        Err(Error::Storage)
    );
    let long_decimal = format!("[{{\"integer\":\"{}\"}}]", "1".repeat(60_000));
    assert_eq!(
        decode_query(&spec(&[Kind::Integer]), long_decimal.as_bytes()),
        Err(Error::Storage)
    );
}

#[test]
fn parser_and_growth_assumptions_are_source_pinned() {
    let lock = include_str!("../../../../../Cargo.lock");
    let package = lock
        .split("[[package]]")
        .find(|s| s.contains("name = \"serde_json\"\n"))
        .unwrap();
    assert!(package.contains("version = \"1.0.151\""));
    assert!(package.contains("c841b55ecdae098c80dcae9cf767f6f8a0c2cdb3416bbef72181df4d0fe73f14"));
    assert!(include_str!("../../../../../rust-toolchain.toml").contains("channel = \"1.95.0\""));
    #[repr(C)]
    struct ErrorRecord {
        tag: usize,
        payload: [usize; 2],
        line: usize,
        column: usize,
    }
    assert!(size_of::<ErrorRecord>() <= 64);
    assert!(size_of::<std::io::Error>() <= 2 * size_of::<usize>());
    assert_eq!(parser_charge(&vec![0; MAX_BYTES]), 198_952);
}
