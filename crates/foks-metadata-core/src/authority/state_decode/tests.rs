use super::*;
use crate::authority::{Caller, Fence, Invocation, Number, Reply, Request};
use crate::{Limits, Sql, Statement, Value};
use foks_snowpack::{with_allocation_budget, AllocationBudget};
use std::sync::{Arc, Mutex};

fn identity_value() -> Identity {
    Identity {
        installation: [1; 16],
        tenant: [2; 16],
        host: vec![3; 33],
        cell: [4; 32],
    }
}
fn activation_value() -> Activation {
    Activation {
        version: Number(1),
        schema: Number(59),
        schema_hash: [5; 32],
        kernel_hash: [6; 32],
        catalog_hash: [7; 32],
        key_generation: Number(1),
        key_digest: [8; 32],
        storage_namespace: [9; 16],
        storage_placement: [10; 32],
        limits: Limits {
            blob_bytes: usize::MAX,
            dirent_bytes: 4096,
            namespace_bytes: u64::MAX,
            namespace_objects: 100,
            directories: 10,
            dirents: 1000,
        },
    }
}
#[derive(Default)]
struct Budget {
    calls: Mutex<Vec<usize>>,
    refuse: usize,
}
impl AllocationBudget for Budget {
    fn reserve(&self, bytes: usize) -> bool {
        let mut calls = self.calls.lock().unwrap();
        calls.push(bytes);
        calls.len() != self.refuse
    }
}
fn parity<T>(input: &str, actual: Result<T>)
where
    T: DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let expected = serde_json::from_str::<T>(input).map_err(|_| Error::Storage);
    assert_eq!(actual, expected, "input bytes {}", input.len());
}

#[test]
fn preserved_codec_accepts_escaped_reordered_and_positional_forms() {
    let mut id = identity_value();
    for length in [0, 1, 33, 255, 1000, 1800] {
        id.host = vec![0; length];
        let text = serde_json::to_string(&id).unwrap();
        parity(&text, identity(&text));
        let sequence =
            serde_json::to_string(&(id.installation, id.tenant, &id.host, id.cell)).unwrap();
        parity(&sequence, identity(&sequence));
        let escaped = text.replace("installation", "instal\\u006cation");
        parity(&escaped, identity(&escaped));
        let reordered = format!(
            "{{\"cell\":{},\"host\":{},\"tenant\":{},\"installation\":{}}}",
            serde_json::to_string(&id.cell).unwrap(),
            serde_json::to_string(&id.host).unwrap(),
            serde_json::to_string(&id.tenant).unwrap(),
            serde_json::to_string(&id.installation).unwrap(),
        );
        parity(&reordered, identity(&reordered));
    }
    for number in [i64::MIN, -1, 0, 1, i64::MAX] {
        let mut active = activation_value();
        active.version = Number(number);
        let text = serde_json::to_string(&active).unwrap();
        parity(&text, activation(&text));
        let escaped = text
            .replace("version", "ver\\u0073ion")
            .replace("\"59\"", "\"\\u00359\"");
        parity(&escaped, activation(&escaped));
        let sequence = serde_json::to_string(&(
            active.version,
            active.schema,
            active.schema_hash,
            active.kernel_hash,
            active.catalog_hash,
            active.key_generation,
            active.key_digest,
            active.storage_namespace,
            active.storage_placement,
            (
                active.limits.blob_bytes,
                active.limits.dirent_bytes,
                active.limits.namespace_bytes,
                active.limits.namespace_objects,
                active.limits.directories,
                active.limits.dirents,
            ),
        ))
        .unwrap();
        parity(&sequence, activation(&sequence));
    }
}

#[test]
fn guard_excludes_only_invalid_raw_numeric_paths_and_handles_escapes() {
    for token in ["0", "1", "255", "4294967295", "18446744073709551615"] {
        assert!(serde_json::from_str::<u64>(token).is_ok());
        assert_eq!(numeric_guard(token.as_bytes()), Ok(()));
    }
    for token in [
        "-0",
        "-1",
        "1.0",
        "1e0",
        "1E+0",
        "1e-0",
        "18446744073709551616",
        "999999999999999999999",
    ] {
        assert!(serde_json::from_str::<u64>(token).is_err());
        assert!(serde_json::from_str::<u8>(token).is_err());
        assert!(serde_json::from_str::<usize>(token).is_err());
        assert_eq!(numeric_guard(token.as_bytes()), Err(Error::Storage));
    }
    for text in [
        r#""-9223372036854775808""#,
        r#""\u0031e999""#,
        r#""a\"999999999999999999999""#,
        r#""\\""#,
    ] {
        assert_eq!(numeric_guard(text.as_bytes()), Ok(()));
    }
    for text in [
        r#""\\"1e999"#,
        r#""a\"b"1e999"#,
        r#""\u0022"999999999999999999999"#,
    ] {
        assert_eq!(numeric_guard(text.as_bytes()), Err(Error::Storage));
    }
    assert_eq!(numeric_guard(b"\"unterminated\\"), Err(Error::Storage));
    // Target width is left to the original usize visitor, not the guard.
    let text = "18446744073709551615";
    assert_eq!(
        serde_json::from_str::<usize>(text).is_ok(),
        usize::BITS == 64
    );
}

#[test]
fn malformed_fields_and_late_errors_keep_storage_and_bounded_diagnostics() {
    let valid = serde_json::to_string(&identity_value()).unwrap();
    let active = serde_json::to_string(&activation_value()).unwrap();
    let mut cases = vec![
        String::new(),
        "null".into(),
        "[]".into(),
        "{}".into(),
        "{".into(),
        format!("{valid}null"),
        format!("{{\"unknown\":0,{}", &valid[1..]),
        valid.replace("\"host\":[3", "\"host\":[256"),
        valid.replace("\"host\":[", "\"host\":null,\"duplicate\":["),
    ];
    cases.push(format!("{{\"{}\":0}}", "x".repeat(4000)));
    cases.push(format!("\"{}\"", "\\u0001".repeat(600)));
    cases.push(format!("{{\"installation\":\"{}\"}}", "🦀".repeat(900)));
    cases.push("[".repeat(4000));
    for text in cases {
        assert!(text.len() <= MAX_BLOB_BYTES);
        parity::<Identity>(&text, identity(&text));
        let message = serde_json::from_str::<Identity>(&text)
            .unwrap_err()
            .to_string();
        assert!(message.len() <= 10 * text.len() + STATIC_ERROR_BYTES);
    }
    for text in [
        active.replace("\"version\":\"1\"", "\"version\":\"01\""),
        active.replace("\"schema\":\"59\"", "\"schema\":\"-0\""),
        active.replace("\"schema\":\"59\"", "\"schema\":\"18446744073709551615\""),
        active.replace("\"dirent_bytes\":4096", "\"dirent_bytes\":1e0"),
        format!("{{\"version\":\"1\",{}", &active[1..]),
    ] {
        parity(&text, activation(&text));
        assert!(activation(&text).is_err());
    }
}

struct Mock {
    row: Option<Vec<Value>>,
    writes: usize,
}
impl Sql for Mock {
    fn query(&mut self, statement: Statement, parameters: &[Value]) -> Result<Option<Vec<Value>>> {
        assert_eq!(statement, Statement::AuthorityState);
        assert!(parameters.is_empty());
        Ok(self.row.take())
    }
    fn execute(&mut self, statement: Statement, _: &[Value]) -> Result<u64> {
        assert_eq!(statement, Statement::AuthorityClock);
        self.writes += 1;
        Ok(1)
    }
}
fn row(identity: String, activation: String) -> Vec<Value> {
    let mut r = vec![
        Value::Text(identity),
        Value::Text(activation),
        Value::Blob(vec![1; 16]),
    ];
    r.extend([1, 1, 0, 0, 0, 256, 1_048_576, 0].map(Value::Integer));
    r
}
#[test]
fn load_admits_each_parser_before_clock_and_preserves_column_order() {
    let id = serde_json::to_string(&identity_value()).unwrap();
    let active = serde_json::to_string(&activation_value()).unwrap();
    for refuse in 0..=2 {
        let budget = Arc::new(Budget {
            refuse,
            ..Default::default()
        });
        let mut sql = Mock {
            row: Some(row(id.clone(), active.clone())),
            writes: 0,
        };
        let result = with_allocation_budget(budget.clone(), || {
            crate::authority::run(
                &mut sql,
                Invocation {
                    caller: Caller::Controller {},
                    now: Number(1),
                    deadline: Number(2),
                    request: Request::Inspect {
                        fence: Fence {
                            identity: identity_value(),
                            activation: activation_value(),
                            owner: [1; 16],
                            epoch: Number(1),
                        },
                    },
                },
            )
        });
        if refuse == 0 {
            assert!(matches!(result, Ok(Reply::State { .. })));
        } else {
            assert_eq!(result, Err(Error::Bounds));
        }
        let expected = [
            allowance(id.len()).unwrap(),
            allowance(active.len()).unwrap(),
        ];
        assert_eq!(
            *budget.calls.lock().unwrap(),
            expected[..if refuse == 0 { 2 } else { refuse }]
        );
        assert_eq!(sql.writes, usize::from(refuse == 0));
    }
    // The first field fails before checking or charging the later malformed type.
    let budget = Arc::new(Budget::default());
    let mut r = row("{".into(), active);
    r[1] = Value::Null;
    let mut sql = Mock {
        row: Some(r),
        writes: 0,
    };
    let result = with_allocation_budget(budget.clone(), || super::super::load(&mut sql));
    assert!(matches!(result, Err(Error::Storage)));
    assert_eq!(*budget.calls.lock().unwrap(), [allowance(1).unwrap()]);
    assert_eq!(sql.writes, 0);
    let active = serde_json::to_string(&activation_value()).unwrap();
    for column in [2, 3, 10] {
        let mut r = row(id.clone(), active.clone());
        r[column] = Value::Null;
        let mut sql = Mock {
            row: Some(r),
            writes: 0,
        };
        let budget = Arc::new(Budget::default());
        let result = with_allocation_budget(budget.clone(), || super::super::load(&mut sql));
        assert!(matches!(result, Err(Error::Storage)));
        assert_eq!(
            *budget.calls.lock().unwrap(),
            [
                allowance(id.len()).unwrap(),
                allowance(active.len()).unwrap()
            ]
        );
        assert_eq!(sql.writes, 0);
    }
    let padded = format!("{}{}", id, " ".repeat(MAX_BLOB_BYTES - id.len()));
    assert_eq!(identity(&padded), Ok(identity_value()));
    let over = " ".repeat(MAX_BLOB_BYTES + 1);
    let budget = Arc::new(Budget::default());
    assert_eq!(
        with_allocation_budget(budget.clone(), || identity(&over)),
        Err(Error::Bounds)
    );
    assert!(budget.calls.lock().unwrap().is_empty());
}

#[test]
fn pinned_parser_error_layout_static_messages_and_allowance_terms() {
    let lock = include_str!("../../../../../Cargo.lock");
    for (name, version, checksum) in [
        (
            "serde_json",
            "1.0.151",
            "c841b55ecdae098c80dcae9cf767f6f8a0c2cdb3416bbef72181df4d0fe73f14",
        ),
        (
            "serde",
            "1.0.229",
            "4148590afebada386688f18773da617792bf2ef03ffc1e4cbd2b1d45b023e0ba",
        ),
        (
            "serde_core",
            "1.0.229",
            "67dca2c9c51e58a4791a4b1ed58308b39c64224d349a935ab5039aa360942a48",
        ),
        (
            "serde_derive",
            "1.0.229",
            "e7a5d71263a5a7d47b41f6b3f06ba276f10cc18b0931f1799f710578e2309348",
        ),
    ] {
        let package = lock
            .split("[[package]]")
            .find(|s| s.contains(&format!("name = \"{name}\"\n")))
            .unwrap();
        assert!(package.contains(&format!("version = \"{version}\"")));
        assert!(package.contains(checksum));
    }
    assert!(include_str!("../../../../../rust-toolchain.toml").contains("channel = \"1.95.0\""));
    #[repr(C)]
    struct ErrorRecord {
        tag: usize,
        payload: [usize; 2],
        line: usize,
        column: usize,
    }
    assert!(std::mem::size_of::<ErrorRecord>() <= 64);
    assert!(std::mem::size_of::<std::io::Error>() <= 2 * std::mem::size_of::<usize>());
    assert!(std::mem::size_of::<Box<str>>() <= 2 * std::mem::size_of::<usize>());
    for error in [
        serde_json::from_str::<Identity>(r#"{"":0}"#).unwrap_err(),
        serde_json::from_str::<Activation>(r#"{"":0}"#).unwrap_err(),
        serde_json::from_str::<Limits>(r#"{"":0}"#).unwrap_err(),
        serde_json::from_str::<[u8; 32]>("[]").unwrap_err(),
        serde_json::from_str::<Activation>("[]").unwrap_err(),
        serde_json::from_str::<u8>("256").unwrap_err(),
    ] {
        assert!(error.to_string().len() <= STATIC_ERROR_BYTES);
    }
    assert_eq!(allowance(4096), Ok(277_908));
    assert_eq!(allowance(0), Ok(3476));
    assert_eq!(allowance(usize::MAX), Err(Error::Bounds));
    for n in 0..=MAX_BLOB_BYTES {
        assert_eq!(allowance(n), Ok(67 * n + 3476));
    }
}
