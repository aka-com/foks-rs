#![cfg(feature = "native")]
#[path = "../../foks-server-db/tests/common/mod.rs"]
mod common;
use foks_metadata_core::{
    directory::{self, DIRECTORY_SLICE_BYTES, MAXIMUM_DIRECTORY_BYTES},
    native::NativeSql,
    page::{Cell, PageBuffer, PageEncoder},
    page_catalog::PageStatement as P,
    personal::PersonalError,
    Error, Sql, Statement, Value,
};
use foks_proto::{KvDirectory, KvDirectoryStatus, Role, RoleAndGeneration};
use foks_server_db::{KvDirectoryMutation, ReadDatabase};
use rusqlite::{types::ValueRef, Connection};
use serde::Serialize;
use std::{
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

const UID: [u8; 33] = [1; 33];
const PARENT: [u8; 16] = [0x51; 16];
fn directory(seed: usize) -> KvDirectory {
    KvDirectory {
        id: [0x99; 16],
        version: 73,
        key: RoleAndGeneration {
            role: Role::ADMIN,
            generation: 7,
        },
        seed_ciphertext: vec![0x77; seed],
        write_role: Role::OWNER,
        status: KvDirectoryStatus::Dead,
    }
}
fn exact_size(length: usize) -> Vec<u8> {
    let mut value = directory(length.saturating_sub(128));
    let first = value.encode().unwrap();
    assert!(first.len() <= length);
    value
        .seed_ciphertext
        .resize(value.seed_ciphertext.len() + length - first.len(), 0x77);
    let exact = value.encode().unwrap();
    assert_eq!(exact.len(), length);
    exact
}
fn fixture(exact: &[u8]) -> common::TestDatabase {
    let mut test = common::TestDatabase::new();
    test.reserve(1_000_000);
    test.commit(None).unwrap();
    // SQL selection and denormalized key deliberately disagree with decoded
    // fields: native List uses the full decoder and only the resulting key.
    test.database
        .put_kv_directory(&KvDirectoryMutation {
            uid: &UID,
            id: &PARENT,
            version: 1,
            key_role: 3,
            key_visibility: 0,
            key_generation: 1,
            status: 0,
            exact,
        })
        .unwrap();
    test
}
fn read(c: &mut Connection, maximum: usize) -> (Result<RoleAndGeneration, PersonalError>, usize) {
    let tx = c.transaction().unwrap();
    let mut host = NativeSql::new(&tx).unwrap();
    let result = directory::read_list_parent_key(&mut host, &UID, &PARENT, maximum);
    let calls = host.calls();
    tx.rollback().unwrap();
    (result, calls)
}

#[test]
fn bounded_slices_match_native_full_decoder_including_maximum_and_mismatched_fields() {
    for length in [
        1024,
        65537,
        DIRECTORY_SLICE_BYTES - 1,
        DIRECTORY_SLICE_BYTES,
        DIRECTORY_SLICE_BYTES + 1,
        MAXIMUM_DIRECTORY_BYTES,
    ] {
        let exact = exact_size(length);
        let test = fixture(&exact);
        let reader = ReadDatabase::open(&test.path, Default::default()).unwrap();
        let stored = reader.kv_directory(&UID, &PARENT).unwrap().unwrap();
        let expected = KvDirectory::decode(&stored.exact).unwrap();
        assert_ne!(expected.id, stored.id);
        assert_ne!(expected.version, stored.version);
        assert_eq!(expected.status, KvDirectoryStatus::Dead);
        let mut c = Connection::open(&test.path).unwrap();
        let (actual, calls) = read(&mut c, MAXIMUM_DIRECTORY_BYTES);
        assert_eq!(actual, Ok(expected.key));
        assert_eq!(calls, 1 + length.div_ceil(DIRECTORY_SLICE_BYTES));
    }
}

#[test]
fn malformed_large_record_is_not_repaired_and_bounds_fail_before_slice_or_decode() {
    let mut exact = exact_size(MAXIMUM_DIRECTORY_BYTES);
    *exact.last_mut().unwrap() = 0xff;
    let test = fixture(&exact);
    assert!(KvDirectory::decode(&exact).is_err());
    let mut c = Connection::open(&test.path).unwrap();
    assert_eq!(
        read(&mut c, MAXIMUM_DIRECTORY_BYTES).0,
        Err(PersonalError::Corrupt)
    );
    let (result, calls) = read(&mut c, MAXIMUM_DIRECTORY_BYTES - 1);
    assert_eq!(result, Err(PersonalError::Corrupt));
    assert_eq!(calls, 1);
    let (result, calls) = read(&mut c, MAXIMUM_DIRECTORY_BYTES + 1);
    assert_eq!(result, Err(PersonalError::UnsupportedProfile));
    assert_eq!(calls, 0);
    c.execute(
        "UPDATE kv_directories SET exact_directory=zeroblob(?1)",
        [i64::try_from(MAXIMUM_DIRECTORY_BYTES + 1).unwrap()],
    )
    .unwrap();
    let (result, calls) = read(&mut c, MAXIMUM_DIRECTORY_BYTES);
    assert_eq!(result, Err(PersonalError::Corrupt));
    assert_eq!(calls, 1);
    c.execute("DELETE FROM kv_directory_heads", []).unwrap();
    assert_eq!(
        read(&mut c, MAXIMUM_DIRECTORY_BYTES).0,
        Err(PersonalError::Missing)
    );
}

struct Forged {
    mode: u8,
    calls: usize,
}
impl Sql for Forged {
    fn query(
        &mut self,
        _: Statement,
        _: &[Value],
    ) -> foks_metadata_core::Result<Option<Vec<Value>>> {
        Err(Error::Storage)
    }
    fn execute(&mut self, _: Statement, _: &[Value]) -> foks_metadata_core::Result<u64> {
        Err(Error::Storage)
    }
    fn query_page(&mut self, s: P, _: &[Value]) -> foks_metadata_core::Result<PageBuffer> {
        self.calls += 1;
        let mut page = PageEncoder::new(s.spec(), 1)?;
        if s == P::DirectoryProbe {
            page.push(&[
                Cell::Integer(1),
                Cell::Text(if self.mode == 0 { "text" } else { "blob" }),
                Cell::Integer(2),
            ])?;
        } else {
            match self.mode {
                1 => {} // disappeared selected head/version
                2 => page.push(&[
                    Cell::Integer(2),
                    Cell::Text("blob"),
                    Cell::Integer(2),
                    Cell::Blob(&[0xc0, 0]),
                ])?,
                3 => page.push(&[
                    Cell::Integer(1),
                    Cell::Text("blob"),
                    Cell::Integer(3),
                    Cell::Blob(&[0xc0, 0]),
                ])?,
                4 => page.push(&[
                    Cell::Integer(1),
                    Cell::Text("blob"),
                    Cell::Integer(2),
                    Cell::Blob(&[0xc0]),
                ])?,
                5 => page.push(&[
                    Cell::Integer(1),
                    Cell::Text("blob"),
                    Cell::Integer(2),
                    Cell::Blob(&[0xc0, 0, 0]),
                ])?,
                _ => page.push(&[
                    Cell::Integer(1),
                    Cell::Text("blob"),
                    Cell::Integer(2),
                    Cell::Null,
                ])?,
            }
        }
        page.finish()
    }
}
#[test]
fn inconsistent_type_version_length_missing_short_extra_and_suppressed_slices_fail() {
    for mode in 0..=6 {
        let mut sql = Forged { mode, calls: 0 };
        assert_eq!(
            directory::read_list_parent_key(&mut sql, &UID, &PARENT, MAXIMUM_DIRECTORY_BYTES),
            Err(PersonalError::Corrupt)
        );
        assert_eq!(sql.calls, if mode == 0 { 1 } else { 2 });
    }
}

#[test]
fn allocation_refusal_before_input_and_during_decode_is_infrastructure_not_corruption() {
    struct RejectAfter {
        remaining: AtomicUsize,
    }
    impl foks_snowpack::AllocationBudget for RejectAfter {
        fn reserve(&self, n: usize) -> bool {
            self.remaining
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |r| r.checked_sub(n))
                .is_ok()
        }
    }
    let exact = exact_size(DIRECTORY_SLICE_BYTES + 1);
    let test = fixture(&exact);
    for budget in [0, 64, exact.len() - 1] {
        let mut c = Connection::open(&test.path).unwrap();
        let result = foks_snowpack::with_allocation_budget(
            Arc::new(RejectAfter {
                remaining: AtomicUsize::new(budget),
            }),
            || read(&mut c, MAXIMUM_DIRECTORY_BYTES).0,
        );
        assert_eq!(
            result,
            Err(PersonalError::Kernel(Error::Bounds)),
            "budget {budget}"
        );
    }
    struct RejectDecoder {
        threshold: usize,
        rejected: AtomicUsize,
    }
    impl foks_snowpack::AllocationBudget for RejectDecoder {
        fn reserve(&self, n: usize) -> bool {
            if n >= self.threshold {
                self.rejected.fetch_add(1, Ordering::Relaxed);
                false
            } else {
                true
            }
        }
    }
    let budget = Arc::new(RejectDecoder {
        threshold: exact.len() * 3,
        rejected: AtomicUsize::new(0),
    });
    let mut c = Connection::open(&test.path).unwrap();
    let (result, calls) = foks_snowpack::with_allocation_budget(budget.clone(), || {
        read(&mut c, MAXIMUM_DIRECTORY_BYTES)
    });
    assert_eq!(
        calls, 3,
        "all bounded slices completed before decoder admission"
    );
    assert_eq!(budget.rejected.load(Ordering::Relaxed), 1);
    assert_eq!(result, Err(PersonalError::Kernel(Error::Bounds)));
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
#[derive(Serialize)]
struct Table {
    name: String,
    columns: Vec<String>,
    rows: Vec<Vec<Value>>,
}
fn snapshot(c: &Connection) -> Vec<Table> {
    let tables=c.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([],|r|r.get::<_,String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    tables
        .into_iter()
        .map(|name| {
            let mut q = c
                .prepare(&format!("SELECT * FROM {}", quote(&name)))
                .unwrap();
            let columns = q
                .column_names()
                .iter()
                .map(|s| (*s).to_owned())
                .collect::<Vec<_>>();
            let count = columns.len();
            let rows = q
                .query_map([], |row| {
                    (0..count)
                        .map(|i| {
                            Ok(match row.get_ref(i)? {
                                ValueRef::Null => Value::Null,
                                ValueRef::Integer(n) => Value::Integer(n),
                                ValueRef::Blob(b) => Value::Blob(b.to_vec()),
                                ValueRef::Text(s) => {
                                    Value::Text(std::str::from_utf8(s).unwrap().to_owned())
                                }
                                _ => panic!("REAL"),
                            })
                        })
                        .collect()
                })
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            Table {
                name,
                columns,
                rows,
            }
        })
        .collect()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
#[test]
fn export_binary_directory_decoder_cases_without_large_json_byte_arrays() {
    let Some(output_directory) = std::env::var_os("FOKS_DIRECTORY_VECTORS") else {
        return;
    };
    let out = Path::new(&output_directory);
    std::fs::create_dir_all(out).unwrap();
    let test = fixture(&directory(1).encode().unwrap());
    let c = Connection::open(&test.path).unwrap();
    let objects=c.prepare("SELECT type,sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' AND sql IS NOT NULL ORDER BY type,name").unwrap().query_map([],|r|Ok(serde_json::json!({"kind":r.get::<_,String>(0)?,"sql":r.get::<_,String>(1)?}))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    let before = snapshot(&c);
    let mut vectors = Vec::new();
    for (name, length, malformed, limit) in [
        ("small", 1024, false, MAXIMUM_DIRECTORY_BYTES),
        ("over64k", 65537, false, MAXIMUM_DIRECTORY_BYTES),
        (
            "slice_minus",
            DIRECTORY_SLICE_BYTES - 1,
            false,
            MAXIMUM_DIRECTORY_BYTES,
        ),
        (
            "slice_exact",
            DIRECTORY_SLICE_BYTES,
            false,
            MAXIMUM_DIRECTORY_BYTES,
        ),
        (
            "slice_plus",
            DIRECTORY_SLICE_BYTES + 1,
            false,
            MAXIMUM_DIRECTORY_BYTES,
        ),
        (
            "maximum",
            MAXIMUM_DIRECTORY_BYTES,
            false,
            MAXIMUM_DIRECTORY_BYTES,
        ),
        (
            "malformed_maximum",
            MAXIMUM_DIRECTORY_BYTES,
            true,
            MAXIMUM_DIRECTORY_BYTES,
        ),
        (
            "configured_limit",
            DIRECTORY_SLICE_BYTES + 1,
            false,
            DIRECTORY_SLICE_BYTES,
        ),
    ] {
        let mut exact = exact_size(length);
        if malformed {
            *exact.last_mut().unwrap() = 0xff;
        }
        let expected = if malformed || length > limit {
            None
        } else {
            Some(hex(&foks_snowpack::encode(
                &KvDirectory::decode(&exact).unwrap().key.to_value(),
            )
            .unwrap()))
        };
        let file = format!("{name}.bin");
        std::fs::write(out.join(&file), &exact).unwrap();
        let command = foks_metadata_core::page::encode_parameters(
            &P::DirectoryProbe.spec(),
            &[
                Value::Blob(UID.to_vec()),
                Value::Blob(PARENT.to_vec()),
                Value::Integer(limit as i64),
            ],
        )
        .unwrap();
        c.execute(
            "UPDATE kv_directories SET exact_directory=?1 WHERE uid=?2 AND directory_id=?3",
            rusqlite::params![exact, UID, PARENT],
        )
        .unwrap();
        let tx = c.unchecked_transaction().unwrap();
        let mut host = NativeSql::new(&tx).unwrap();
        let result = directory::qualify(&mut host, command.clone());
        let calls = host.calls();
        tx.rollback().unwrap();
        assert_eq!(
            calls,
            if length > limit {
                1
            } else {
                1 + length.div_ceil(DIRECTORY_SLICE_BYTES)
            }
        );
        match &result {
            Ok(bytes) => assert_eq!(Some(hex(bytes)), expected),
            Err(error) => {
                assert_eq!(directory::qualification_error(error), "corrupt");
                assert!(expected.is_none());
            }
        }
        vectors.push(serde_json::json!({"name":name,"file":file,"command":hex(&command),"reply":expected,"outcome":if expected.is_some(){"ok"}else{"corrupt"},"calls":if length>limit {1}else{1+length.div_ceil(DIRECTORY_SLICE_BYTES)}}));
    }
    std::fs::write(out.join("directory-vectors.json"),serde_json::to_vec(&serde_json::json!({"schema_version":foks_server_db::SCHEMA_VERSION,"objects":objects,"before":before,"uid":UID.to_vec(),"parent":PARENT.to_vec(),"vectors":vectors})).unwrap()).unwrap();
    std::fs::write(
        out.join("page-catalog.json"),
        foks_metadata_core::page::catalog_artifact().unwrap(),
    )
    .unwrap();
}
