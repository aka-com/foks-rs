use super::*;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

fn value() -> Value {
    json!({
        "version":1,
        "peer":{"exact_der":[1],"serial_magnitude":[1],"public_key":vec![2u8;32],"expires_at_micros":"9000000"},
        "fence":{
            "identity":{"installation":vec![3u8;16],"tenant":vec![4u8;16],"host":vec![5u8;33],"cell":vec![6u8;32]},
            "activation":{"version":"1","schema":"59","schema_hash":vec![7u8;32],"kernel_hash":vec![8u8;32],"catalog_hash":vec![9u8;32],"key_generation":"1","key_digest":vec![10u8;32],"storage_namespace":vec![11u8;16],"storage_placement":vec![12u8;32],"limits":{"blob_bytes":"4096","dirent_bytes":"4096","namespace_bytes":"1000000","namespace_objects":"1000","directories":"100","dirents":"1000"}},
            "owner":vec![13u8;16],"epoch":"2"
        },
        "now_micros":"1000999","execution_deadline_ms":"1100"
    })
}
fn encoded(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}
fn decode_value(value: &Value) -> HostListContext {
    HostListContext::decode(&encoded(value)).unwrap_or_else(|error| panic!("{}", error.code()))
}
fn invalid(value: &Value) {
    assert!(matches!(
        HostListContext::decode(&encoded(value)),
        Err(ContextError::Invalid)
    ));
}
fn changed(path: &str, replacement: Value) -> Value {
    let mut value = value();
    *value.pointer_mut(path).unwrap() = replacement;
    value
}

struct Budget {
    allow: bool,
    calls: Mutex<Vec<usize>>,
}
impl foks_snowpack::AllocationBudget for Budget {
    fn reserve(&self, bytes: usize) -> bool {
        self.calls.lock().unwrap().push(bytes);
        self.allow
    }
}
fn recorded(bytes: &[u8], allow: bool) -> (Result<HostListContext, ContextError>, Vec<usize>) {
    let budget = Arc::new(Budget {
        allow,
        calls: Mutex::new(Vec::with_capacity(2)),
    });
    let result =
        foks_snowpack::with_allocation_budget(budget.clone(), || HostListContext::decode(bytes));
    let calls = budget.calls.lock().unwrap().clone();
    (result, calls)
}

#[test]
fn exact_values_and_microseconds_survive_without_becoming_authority() {
    let decoded = decode_value(&value());
    assert_eq!(decoded.now_micros(), 1_000_999);
    assert_eq!(decoded.now_ms(), 1000);
    assert_eq!(decoded.execution_deadline_ms(), 1100);
    assert_eq!(decoded.peer().exact_der(), [1]);
    assert_eq!(decoded.peer().public_key(), &[2; 32]);
    assert_eq!(decoded.fence().identity.host, vec![5; 33]);
    assert_eq!(decoded.fence().activation.schema.0, 59);
    assert_eq!(decoded.fence().epoch.0, 2);
    // Structural decoding deliberately does not authenticate this synthetic DER
    // or grant validity to an expired certificate.
    for expiry in [0, 1_000_998, 1_000_999, 1_001_000, i64::MAX as u64] {
        let decoded = decode_value(&changed(
            "/peer/expires_at_micros",
            json!(expiry.to_string()),
        ));
        assert_eq!(decoded.peer().expires_at_micros(), expiry);
        assert_eq!(decoded.now_micros(), 1_000_999);
    }
    for now in [999_999u64, 1_000_000, 1_000_001] {
        let decoded = decode_value(&changed("/now_micros", json!(now.to_string())));
        assert_eq!(decoded.now_micros(), now);
        assert_eq!(decoded.now_ms(), (now / 1000) as i64);
    }
}

#[test]
fn scalar_and_vector_widths_are_target_independent() {
    for path in [
        "/fence/activation/limits/blob_bytes",
        "/fence/activation/limits/dirent_bytes",
    ] {
        decode_value(&changed(path, json!(u32::MAX.to_string())));
        invalid(&changed(path, json!((u64::from(u32::MAX) + 1).to_string())));
    }
    for path in [
        "/fence/activation/limits/namespace_bytes",
        "/fence/activation/limits/namespace_objects",
        "/fence/activation/limits/directories",
        "/fence/activation/limits/dirents",
    ] {
        decode_value(&changed(path, json!(u64::MAX.to_string())));
        invalid(&changed(path, json!("18446744073709551616")));
    }
    for path in [
        "/fence/epoch",
        "/fence/activation/version",
        "/fence/activation/schema",
        "/fence/activation/key_generation",
    ] {
        decode_value(&changed(path, json!(i64::MAX.to_string())));
        invalid(&changed(path, json!("0")));
        invalid(&changed(path, json!((i64::MAX as u64 + 1).to_string())));
    }
    for (path, length) in [
        ("/peer/public_key", 32),
        ("/fence/identity/installation", 16),
        ("/fence/identity/tenant", 16),
        ("/fence/identity/host", 33),
        ("/fence/identity/cell", 32),
        ("/fence/owner", 16),
        ("/fence/activation/schema_hash", 32),
        ("/fence/activation/kernel_hash", 32),
        ("/fence/activation/catalog_hash", 32),
        ("/fence/activation/key_digest", 32),
        ("/fence/activation/storage_namespace", 16),
        ("/fence/activation/storage_placement", 32),
    ] {
        decode_value(&changed(path, json!(vec![255u8; length])));
        invalid(&changed(path, json!(vec![255u8; length - 1])));
        invalid(&changed(path, json!(vec![255u8; length + 1])));
        invalid(&changed(path, json!(vec![256u16; length])));
    }
    for (path, maximum) in [("/peer/exact_der", 4096), ("/peer/serial_magnitude", 20)] {
        decode_value(&changed(path, json!(vec![255u8; maximum])));
        invalid(&changed(path, json!(Vec::<u8>::new())));
        invalid(&changed(path, json!(vec![1u8; maximum + 1])));
    }
    decode_value(&changed("/peer/serial_magnitude", json!([0])));
    invalid(&changed("/peer/serial_magnitude", json!([0, 1])));
    invalid(&changed(
        "/peer/expires_at_micros",
        json!((i64::MAX as u64 + 1).to_string()),
    ));
}

#[test]
fn time_bounds_are_checked_without_rounding_the_authentication_sample() {
    for deadline in [1001u64, 16_000] {
        decode_value(&changed(
            "/execution_deadline_ms",
            json!(deadline.to_string()),
        ));
    }
    for deadline in [0u64, 999, 1000, 16_001, u64::MAX] {
        invalid(&changed(
            "/execution_deadline_ms",
            json!(deadline.to_string()),
        ));
    }
    invalid(&changed(
        "/now_micros",
        json!((i64::MAX as u64 + 1).to_string()),
    ));
    let mut large = value();
    large["now_micros"] = json!(i64::MAX.to_string());
    large["execution_deadline_ms"] = json!((i64::MAX / 1000 + 15_000).to_string());
    assert_eq!(decode_value(&large).now_micros(), i64::MAX as u64);
    large["execution_deadline_ms"] = json!((i64::MAX / 1000 + 15_001).to_string());
    invalid(&large);
}

#[test]
fn all_objects_deny_missing_unknown_and_duplicate_fields() {
    for path in [
        "",
        "/peer",
        "/fence",
        "/fence/identity",
        "/fence/activation",
        "/fence/activation/limits",
    ] {
        let original = value();
        let keys: Vec<String> = original
            .pointer(path)
            .unwrap()
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        for key in keys {
            let mut missing = original.clone();
            missing
                .pointer_mut(path)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove(&key);
            invalid(&missing);
            let mut extra = original.clone();
            extra
                .pointer_mut(path)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("unknown".into(), json!(0));
            invalid(&extra);
            let object = original.pointer(path).unwrap();
            let raw = serde_json::to_string(object).unwrap();
            let member = format!(
                "{}:{}",
                serde_json::to_string(&key).unwrap(),
                serde_json::to_string(&object[key.as_str()]).unwrap()
            );
            let duplicate = format!("{{{member},{}", &raw[1..]);
            let full = serde_json::to_string(&original).unwrap();
            assert_eq!(full.matches(raw.as_str()).count(), 1);
            let full = full.replacen(raw.as_str(), duplicate.as_str(), 1);
            assert!(matches!(
                HostListContext::decode(full.as_bytes()),
                Err(ContextError::Invalid)
            ));
        }
    }
}

#[test]
fn malformed_types_decimals_and_lexical_input_never_normalize() {
    for path in [
        "/now_micros",
        "/execution_deadline_ms",
        "/peer/expires_at_micros",
        "/fence/epoch",
        "/fence/activation/limits/blob_bytes",
    ] {
        for bad in [
            json!(0),
            json!(true),
            Value::Null,
            json!([]),
            json!({"value":"1"}),
            json!(""),
            json!("01"),
            json!("+1"),
            json!("-0"),
            json!(" 1"),
            json!("1.0"),
            json!("1e3"),
        ] {
            invalid(&changed(path, bad));
        }
    }
    for path in [
        "/peer",
        "/fence",
        "/fence/identity",
        "/fence/activation",
        "/fence/activation/limits",
        "/peer/exact_der",
        "/peer/public_key",
    ] {
        for bad in [
            json!("large untrusted string"),
            json!(true),
            Value::Null,
            json!(1),
            json!(1.5),
        ] {
            invalid(&changed(path, bad));
        }
    }
    let raw = serde_json::to_string(&value()).unwrap();
    assert_eq!(raw.matches("\"version\":1").count(), 1);
    for replacement in ["-1", "1.0", "1e0", "0001", "1000", "true", "null"] {
        let raw = raw.replacen("\"version\":1", &format!("\"version\":{replacement}"), 1);
        assert!(matches!(
            HostListContext::decode(raw.as_bytes()),
            Err(ContextError::Invalid)
        ));
    }
    for bad in [
        b"[[[[[]]]]]".as_slice(),
        b"{]",
        b"\"unfinished",
        b"{}x",
        b"{\"a\":\"\\u0031\"}",
        b"\"\xff\"",
    ] {
        assert!(matches!(
            HostListContext::decode(bad),
            Err(ContextError::Invalid)
        ));
    }
}

fn maximal(time_valid: bool) -> Value {
    let mut value = value();
    value["peer"]["exact_der"] = json!(vec![255u8; 4096]);
    value["peer"]["serial_magnitude"] = json!(vec![255u8; 20]);
    for path in [
        "/peer/public_key",
        "/fence/identity/installation",
        "/fence/identity/tenant",
        "/fence/identity/host",
        "/fence/identity/cell",
        "/fence/owner",
        "/fence/activation/schema_hash",
        "/fence/activation/kernel_hash",
        "/fence/activation/catalog_hash",
        "/fence/activation/key_digest",
        "/fence/activation/storage_namespace",
        "/fence/activation/storage_placement",
    ] {
        let target = value.pointer_mut(path).unwrap();
        let length = target.as_array().unwrap().len();
        *target = json!(vec![255u8; length]);
    }
    for path in [
        "/peer/expires_at_micros",
        "/now_micros",
        "/execution_deadline_ms",
        "/fence/epoch",
        "/fence/activation/version",
        "/fence/activation/schema",
        "/fence/activation/key_generation",
    ] {
        *value.pointer_mut(path).unwrap() = json!(i64::MAX.to_string());
    }
    for path in [
        "blob_bytes",
        "dirent_bytes",
        "namespace_bytes",
        "namespace_objects",
        "directories",
        "dirents",
    ] {
        value["fence"]["activation"]["limits"][path] =
            json!(if matches!(path, "blob_bytes" | "dirent_bytes") {
                u64::from(u32::MAX)
            } else {
                u64::MAX
            }
            .to_string());
    }
    if time_valid {
        value["execution_deadline_ms"] = json!((i64::MAX / 1000 + 15_000).to_string());
    }
    value
}

#[test]
fn independent_maximum_and_one_reservation_cover_success_and_errors() {
    // Independent field-by-field Python json.dumps(separators=(',',':')) audit:
    // peer16685 + fence1700 + root111 =18496; joint deadline reduces3 bytes.
    let maximal = maximal(false);
    assert_eq!(encoded(&maximal["peer"]).len(), 16_685);
    assert_eq!(encoded(&maximal["fence"]).len(), 1700);
    let bytes = encoded(&maximal);
    assert_eq!(bytes.len(), MAX_CONTEXT_BYTES);
    let (result, calls) = recorded(&bytes, true);
    assert!(matches!(result, Err(ContextError::Invalid)));
    assert_eq!(calls, [27_029]);
    let valid = encoded(&self::maximal(true));
    assert_eq!(valid.len(), 18_493);
    let (result, calls) = recorded(&valid, true);
    assert!(result.is_ok());
    assert_eq!(calls, [27_026]);
    let (result, calls) = recorded(&valid, false);
    assert!(matches!(result, Err(ContextError::Admission)));
    assert_eq!(calls, [27_026]);
    for bytes in [Vec::new(), vec![b' '; MAX_CONTEXT_BYTES + 1]] {
        let (result, calls) = recorded(&bytes, true);
        assert!(matches!(result, Err(ContextError::InputBound)));
        assert!(calls.is_empty());
    }
}

#[test]
fn near_limit_malformed_values_use_fixed_errors_and_fixed_admission() {
    let long = "x".repeat(MAX_CONTEXT_BYTES - 8);
    let mut unknown = format!("{{\"{long}\":0}}").into_bytes();
    unknown.resize(MAX_CONTEXT_BYTES, b' ');
    let root_string = format!("\"{}\"", "x".repeat(MAX_CONTEXT_BYTES - 2)).into_bytes();
    let original = encoded(&value());
    let mut decimal = value();
    decimal["now_micros"] = json!("x".repeat(MAX_CONTEXT_BYTES - original.len() + 7));
    let decimal = encoded(&decimal);
    assert_eq!(decimal.len(), MAX_CONTEXT_BYTES);
    for bytes in [unknown, root_string, decimal] {
        let (result, calls) = recorded(&bytes, true);
        assert!(matches!(result, Err(ContextError::Invalid)));
        assert_eq!(calls, [27_029]);
        let message = decode::parser_error(&bytes).unwrap();
        assert!(message.starts_with("invalid host context"));
        assert!(message.len() < 100);
        assert!(!message.contains("xxxxxxxx"));
    }
    let raw_number = vec![b'9'; MAX_CONTEXT_BYTES];
    let (result, calls) = recorded(&raw_number, true);
    assert!(matches!(result, Err(ContextError::Invalid)));
    assert_eq!(calls, [27_029]);
}

#[test]
fn fixed_visitors_do_not_echo_wrong_kind_strings_even_without_preflight() {
    for raw in [
        "null",
        "true",
        "false",
        "1",
        "-1",
        "1.25",
        "[]",
        "\"untrusted-marker\"",
    ] {
        let message = decode::parser_error(raw.as_bytes()).unwrap();
        assert!(message.starts_with("invalid host context"));
        assert!(!message.contains("untrusted-marker"));
    }
    for path in [
        "/peer/exact_der",
        "/peer/public_key",
        "/fence/identity",
        "/fence/activation",
        "/fence/activation/limits",
        "/now_micros",
    ] {
        let bytes = encoded(&changed(path, json!("untrusted-marker")));
        let message = decode::parser_error(&bytes).unwrap();
        assert!(message.starts_with("invalid host context"));
        assert!(!message.contains("untrusted-marker"));
    }
}

#[test]
fn conversion_moves_bounded_vectors_without_cloning() {
    let (pointers, value) = decode::vector_addresses(&encoded(&value()));
    assert_eq!(
        pointers,
        [
            value.peer().exact_der().as_ptr() as usize,
            value.peer().serial_magnitude().as_ptr() as usize,
            value.fence().identity.host.as_ptr() as usize
        ]
    );
}

#[test]
fn parser_source_pin_and_conservative_error_layout_are_explicit() {
    let lock = include_str!("../../../../Cargo.lock");
    let package = lock
        .split("[[package]]")
        .find(|part| part.contains("name = \"serde_json\"\n"))
        .unwrap();
    assert!(package.contains("version = \"1.0.151\"\n"));
    assert!(package.contains(
        "checksum = \"c841b55ecdae098c80dcae9cf767f6f8a0c2cdb3416bbef72181df4d0fe73f14\""
    ));
    // This is an audited upper-bound model of private ErrorImpl, not the size of
    // serde_json::Error's outer Box pointer. Source pin catches a version change.
    #[repr(C)]
    struct ErrorRecordBound {
        tag: usize,
        payload: [usize; 2],
        line: usize,
        column: usize,
    }
    assert!(std::mem::size_of::<std::io::Error>() <= 2 * std::mem::size_of::<usize>());
    assert!(std::mem::size_of::<Box<str>>() <= 2 * std::mem::size_of::<usize>());
    assert!(std::mem::size_of::<ErrorRecordBound>() <= 64);
    assert_eq!(ERROR_BYTES, 2 * 64 + 2 * 64);
    assert_eq!(NUMBER_TEMPORARY_BYTES, 16 + 16);
}

#[test]
fn bounded_parser_capacity_failure_keeps_static_invalid_diagnostic() {
    let message = decode::rejected_capacity_message();
    assert!(message.starts_with("invalid host context"));
    assert!(message.len() < 100);
    assert_eq!(ContextError::Invalid.code(), "invalid_host_context");
    assert_eq!(ContextError::Admission.code(), "context_admission");
}
