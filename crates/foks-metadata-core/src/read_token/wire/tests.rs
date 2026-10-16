use super::*;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

const GOLDEN: &[u8] = include_bytes!("../binding.json");
fn value() -> Value {
    serde_json::from_slice(GOLDEN).unwrap()
}
fn decode(value: Value) -> Result<ListBinding> {
    ListBinding::decode_wire(&serde_json::to_vec(&value).unwrap())
}
fn invalid(value: Value) {
    assert_eq!(decode(value).unwrap_err(), Error::InvalidBinding);
}
fn at<'a>(value: &'a mut Value, path: &[&str]) -> &'a mut Value {
    let mut selected = value;
    for field in path {
        selected = selected.get_mut(*field).unwrap();
    }
    selected
}
fn duplicate(path: &[&str], field: &str, escaped: bool) -> Vec<u8> {
    let mut input = value();
    let selected = at(&mut input, path);
    let old = serde_json::to_string(&*selected).unwrap();
    let repeated = serde_json::to_string(&selected[field]).unwrap();
    *selected = json!("REPLACE_OBJECT");
    let key = if escaped {
        let first = field.as_bytes()[0];
        format!("\"\\u{first:04x}{}\"", &field[1..])
    } else {
        serde_json::to_string(field).unwrap()
    };
    serde_json::to_string(&input)
        .unwrap()
        .replace(
            "\"REPLACE_OBJECT\"",
            &format!("{{{key}:{repeated},{}", &old[1..]),
        )
        .into_bytes()
}

#[test]
fn golden_and_reordered_objects_preserve_exact_canonical_binding() {
    let decoded = ListBinding::decode_wire(GOLDEN).unwrap();
    let canonical = decoded.canonical_bytes().unwrap();
    assert_eq!(
        std::str::from_utf8(&canonical).unwrap(),
        std::str::from_utf8(GOLDEN).unwrap().trim()
    );
    let digest: String = decoded
        .digest()
        .unwrap()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(digest, include_str!("../binding.sha256").trim());
    // serde_json's ordered map serializes keys in a different order from the DTO.
    assert_eq!(
        decode(value()).unwrap().canonical_bytes().unwrap(),
        canonical
    );
    let escaped =
        std::str::from_utf8(GOLDEN)
            .unwrap()
            .replacen("\"domain\"", "\"\\u0064omain\"", 1);
    assert_eq!(
        ListBinding::decode_wire(escaped.as_bytes())
            .unwrap()
            .canonical_bytes()
            .unwrap(),
        canonical
    );
}

#[test]
fn every_nested_object_rejects_duplicate_and_unknown_fields() {
    for (path, field) in [
        (vec![], "count"),
        (vec!["fence"], "epoch"),
        (vec!["fence", "identity"], "host"),
        (vec!["fence", "activation"], "schema"),
        (vec!["fence", "activation", "limits"], "dirents"),
        (vec!["dependency"], "revision"),
        (vec!["peer"], "exact_der"),
        (vec!["cursor"], "kind"),
        (vec!["cursor"], "value"),
    ] {
        for escaped in [false, true] {
            assert_eq!(
                ListBinding::decode_wire(&duplicate(&path, field, escaped)).unwrap_err(),
                Error::InvalidBinding,
                "{path:?}/{field}/{escaped}"
            );
        }
        let mut input = value();
        at(&mut input, &path)
            .as_object_mut()
            .unwrap()
            .insert("unknown".into(), json!(0));
        invalid(input);
    }
}

#[test]
fn cursor_shape_is_strict_in_every_field_order() {
    for cursor in [
        json!({"kind":"none"}),
        json!({"kind":"mac","value":vec![7u8;32]}),
        json!({"kind":"time","value":"0"}),
        json!({"kind":"time","value":i64::MAX.to_string()}),
    ] {
        let mut input = value();
        input["cursor"] = cursor;
        assert!(decode(input).is_ok());
    }
    for cursor in [
        json!({"kind":"none","value":null}),
        json!({"kind":"none","value":"0"}),
        json!({"kind":"mac"}),
        json!({"kind":"time"}),
        json!({"kind":"time","value":null}),
        json!({"kind":"time","value":0}),
        json!({"kind":"time","value":"-1"}),
        json!({"kind":"time","value":"00"}),
        json!({"kind":"time","value":"+1"}),
        json!({"kind":"time","value":"9223372036854775808"}),
        json!({"kind":"mac","value":vec![7u8;31]}),
        json!({"kind":"mac","value":vec![7u8;33]}),
        json!({"kind":"mac","value":"1"}),
        json!({"kind":"time","value":vec![7u8;32]}),
        json!({"kind":"other"}),
        json!({"kind":{"none":null}}),
        json!({"kind":["none"]}),
        json!({"kind":null}),
        json!({"kind":0}),
        json!({"kind":"none","unknown":false}),
        json!({"kind":"time","value":{"kind":"time","value":"1"}}),
    ] {
        let mut input = value();
        input["cursor"] = cursor;
        invalid(input);
    }
}

#[test]
fn domain_count_widths_and_structural_invariants_are_never_normalized() {
    for (path, replacement) in [
        (vec!["domain"], json!("wrong")),
        (vec!["route"], json!("other")),
        (vec!["count"], json!("0")),
        (vec!["count"], json!("4097")),
        (vec!["count"], json!("-1")),
        (vec!["count"], json!("01")),
        (vec!["count"], json!(1)),
        (vec!["count"], json!("1.0")),
        (vec!["revision"], json!("0")),
        (vec!["fence", "epoch"], json!("0")),
        (vec!["fence", "activation", "version"], json!("0")),
        (vec!["fence", "activation", "schema"], json!("0")),
        (vec!["fence", "activation", "key_generation"], json!("0")),
        (vec!["deadline"], json!("-1")),
        (vec!["dependency", "revision"], json!("0")),
        (vec!["dependency", "revision"], json!("-1")),
        (vec!["dependency", "incarnation"], json!(vec![0u8; 32])),
        (vec!["dependency", "incarnation"], json!(vec![1u8; 31])),
        (vec!["namespace"], json!(vec![1u8; 32])),
        (vec!["parent"], json!(vec![1u8; 15])),
        (vec!["fence", "identity", "host"], json!(vec![1u8; 32])),
        (vec!["peer", "serial_magnitude"], json!([0, 1])),
        (vec!["peer", "exact_der"], json!([])),
        (
            vec!["fence", "activation", "limits", "blob_bytes"],
            json!(u64::from(u32::MAX) + 1),
        ),
        (
            vec!["fence", "activation", "limits", "dirent_bytes"],
            json!(u64::MAX),
        ),
    ] {
        let mut input = value();
        *at(&mut input, &path) = replacement;
        invalid(input);
    }
    let mut input = value();
    input.as_object_mut().unwrap().remove("load_small");
    invalid(input);
}

#[test]
fn exact_integer_limits_are_preserved_without_js_rounding() {
    for number in [9_007_199_254_740_991i64, 9_007_199_254_740_993, i64::MAX] {
        let mut input = value();
        input["dependency"]["revision"] = json!(number.to_string());
        input["cursor"]["value"] = json!(number.to_string());
        let decoded = decode(input).unwrap();
        assert_eq!(decoded.dependency.revision(), number as u64);
        assert_eq!(
            decoded.cursor,
            InitialCursor::Time {
                value: Number(number)
            }
        );
    }
    let mut input = value();
    input["fence"]["activation"]["limits"]["namespace_bytes"] = json!(u64::MAX);
    input["fence"]["activation"]["limits"]["blob_bytes"] = json!(u32::MAX);
    let decoded = decode(input).unwrap();
    assert_eq!(decoded.fence.activation.limits.namespace_bytes, u64::MAX);
    assert_eq!(
        decoded.fence.activation.limits.blob_bytes as u64,
        u64::from(u32::MAX)
    );
}

#[test]
fn bounds_trailing_and_admission_fail_before_parse_work() {
    for input in [b"".as_slice(), &[b' '; MAX_BINDING_BYTES + 1]] {
        assert_eq!(ListBinding::decode_wire(input).unwrap_err(), Error::Bounds);
    }
    for input in [b"{".as_slice(), b"null", b"[]", b"{}", b"{}{}"] {
        assert_eq!(
            ListBinding::decode_wire(input).unwrap_err(),
            Error::InvalidBinding
        );
    }
    let mut trailing = GOLDEN.to_vec();
    trailing.extend_from_slice(b"{}");
    assert_eq!(
        ListBinding::decode_wire(&trailing).unwrap_err(),
        Error::InvalidBinding
    );
    struct Refuse(AtomicUsize);
    impl foks_snowpack::AllocationBudget for Refuse {
        fn reserve(&self, bytes: usize) -> bool {
            self.0.store(bytes, Ordering::SeqCst);
            false
        }
    }
    let budget = Arc::new(Refuse(AtomicUsize::new(0)));
    let rejected = foks_snowpack::with_allocation_budget(budget.clone(), || {
        ListBinding::decode_wire(b"malformed")
    });
    assert_eq!(rejected.unwrap_err(), Error::Bounds);
    assert_eq!(budget.0.load(Ordering::SeqCst), 4 * b"malformed".len());
}
