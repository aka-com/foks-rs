#![cfg(feature = "native")]
//! Native repository/public codec oracle for the qualification-only List helper.
#[path = "../../foks-server-db/tests/common/mod.rs"]
mod common;
use foks_metadata_core::{directory::MAXIMUM_DIRECTORY_BYTES, native::NativeSql, personal::*, Sql};
use foks_proto::{
    KvDirectory, KvDirectoryStatus, KvDirent, KvExtendedDirent, KvListResponse, KvNodeId,
    KvSmallFileBox, Role, RoleAndGeneration, SecretBox,
};
use foks_server_db::{KvDirectoryMutation, ReadDatabase};
use rusqlite::{params, Connection};
const UID: [u8; 33] = [1; 33];
const DEVICE: [u8; 33] = [4; 33];
const ROOT: [u8; 16] = [0x51; 16];
fn id(n: usize) -> [u8; 16] {
    let mut b = [0; 16];
    b[8..].copy_from_slice(&(n as u64).to_be_bytes());
    b
}
fn node(n: usize) -> [u8; 17] {
    let mut b = [3; 17];
    b[1..].copy_from_slice(&id(n));
    b
}
fn mac(n: usize) -> [u8; 32] {
    let mut b = [0; 32];
    b[16..].copy_from_slice(&id(n));
    b
}
fn peer() -> PeerCertificate {
    PeerCertificate::from_frontend_parts(vec![0x30, 0], vec![7], [4; 32], 1000).unwrap()
}
fn key(role: Role, generation: u64) -> RoleAndGeneration {
    RoleAndGeneration { role, generation }
}
fn parent(role: Role, generation: u64, size: usize) -> Vec<u8> {
    KvDirectory {
        id: ROOT,
        version: 1,
        key: key(role, generation),
        seed_ciphertext: vec![1; size],
        write_role: Role::OWNER,
        status: KvDirectoryStatus::Active,
    }
    .encode()
    .unwrap()
}
fn fixture(count: usize, distinct_roles: bool) -> common::TestDatabase {
    let mut t = common::TestDatabase::new();
    t.reserve(1_000_000);
    t.commit(None).unwrap();
    t.database
        .record_certificate(&[7], &UID, &DEVICE, &DEVICE, 100, 1000, peer().exact_der())
        .unwrap();
    t.database
        .put_kv_directory(&KvDirectoryMutation {
            uid: &UID,
            id: &ROOT,
            version: 1,
            key_role: 3,
            key_visibility: 0,
            key_generation: 1,
            status: 0,
            exact: &parent(Role::OWNER, 1, 48),
        })
        .unwrap();
    let mut c = Connection::open(&t.path).unwrap();
    c.pragma_update(None, "foreign_keys", true).unwrap();
    let tx = c.transaction().unwrap();
    for n in 0..count {
        let role = if distinct_roles {
            Role::member(i16::MIN + n as i16)
        } else {
            Role::OWNER
        };
        if distinct_roles {
            tx.execute("INSERT INTO shared_keys(uid,role_type,visibility,generation,verify_key,exact_hepk) VALUES(?1,1,?2,1,zeroblob(33),X'01')",params![UID,role.visibility().unwrap()]).unwrap();
        }
        let b = KvSmallFileBox {
            key: key(role, 1),
            ciphertext: vec![n as u8; 32],
        }
        .encode()
        .unwrap();
        tx.execute("INSERT INTO kv_nodes(uid,node_id,node_type,exact_node,ciphertext_bytes) VALUES(?1,?2,3,?3,32)",params![UID,node(n),b]).unwrap();
        let d = KvDirent::new(
            ROOT,
            id(n),
            KvNodeId(node(n)),
            1,
            1,
            Role::OWNER,
            mac(n / 2),
            SecretBox {
                nonce: [7; 16],
                ciphertext: vec![8; 16],
            },
            KvDirectoryStatus::Active,
            [9; 32],
            (n / 3) as u64,
        )
        .unwrap();
        tx.execute("INSERT INTO kv_dirents(uid,parent_id,dirent_id,version,directory_version,node_id,name_mac,creation_time,exact_dirent) VALUES(?1,?2,?3,1,1,?4,?5,?6,?7)",params![UID,ROOT,id(n),node(n),mac(n/2),(n/3) as i64,d.encode().unwrap()]).unwrap();
        tx.execute(
            "INSERT INTO kv_dirent_heads VALUES(?1,?2,?3,1)",
            params![UID, ROOT, id(n)],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    t
}
fn run(
    t: &common::TestDatabase,
    cursor: ListCursor,
    count: u64,
    load: bool,
) -> (Result<Vec<u8>>, usize) {
    let c = Connection::open(&t.path).unwrap();
    let tx = c.unchecked_transaction().unwrap();
    let mut sql = NativeSql::new(&tx).unwrap();
    let current = authenticate_peer(&mut sql, &peer(), 200).unwrap();
    let before = sql.calls();
    let result = read_personal_list(
        &mut sql,
        &current,
        &ListRequest::new(ROOT, cursor, count, load),
        200,
        MAXIMUM_DIRECTORY_BYTES,
    );
    let calls = sql.calls() - before;
    tx.rollback().unwrap();
    (result, calls)
}
fn oracle(t: &common::TestDatabase, cursor: ListCursor, count: u64, load: bool) -> Vec<u8> {
    let count = if count == 0 || count > 4096 {
        4096
    } else {
        count as usize
    };
    let reader = ReadDatabase::open(&t.path, foks_server_db::Config::default()).unwrap();
    let cursor = match cursor {
        ListCursor::Initial => None,
        ListCursor::Mac(m) => Some(foks_server_db::KvListCursor::Mac(m)),
        ListCursor::Time(t) => Some(foks_server_db::KvListCursor::Time(t)),
    };
    let stored = reader.kv_list(&UID, &ROOT, cursor, count).unwrap();
    let final_page = stored.len() < count;
    let mut entries = Vec::new();
    let mut extended = Vec::new();
    let c = Connection::open(&t.path).unwrap();
    for (position, s) in stored.into_iter().enumerate() {
        let entry = KvDirent::decode(&s.exact).unwrap();
        if load && s.node_id[0] == 3 {
            if let Some(n) = reader.kv_node(&UID, &s.node_id).unwrap() {
                let small = KvSmallFileBox::decode(&n.exact).unwrap();
                let generation:Option<i64>=c.query_row("SELECT max(generation) FROM shared_keys WHERE uid=?1 AND role_type=?2 AND visibility=?3",params![UID,small.key.role.protocol_value() as i64,small.key.role.visibility().unwrap_or(0)],|r|r.get(0)).unwrap();
                if small.key.role != Role::NONE
                    && small.key.role <= Role::OWNER
                    && small.key.generation > 0
                    && generation.is_some_and(|g| small.key.generation <= g as u64)
                {
                    extended.push(KvExtendedDirent {
                        position: position as u64,
                        small_file: small,
                    });
                }
            }
        }
        entries.push(entry);
    }
    KvListResponse::new(entries, final_page, extended)
        .unwrap()
        .encoded()
        .to_vec()
}
#[test]
fn exact_repository_and_public_codec_parity_across_pages_and_cursors() {
    for n in [0, 1, 14, 15, 16, 30, 64, 65] {
        let t = fixture(n, false);
        for count in [0, 1, 14, 15, 16, 64, 4096, u64::MAX] {
            for cursor in [
                ListCursor::Initial,
                ListCursor::Mac(mac(1)),
                ListCursor::Time(2),
            ] {
                for load in [false, true] {
                    let expected = oracle(&t, cursor.clone(), count, load);
                    assert_eq!(
                        run(&t, cursor.clone(), count, load).0.unwrap(),
                        expected,
                        "n={n}, count={count}, cursor={cursor:?}, load={load}"
                    );
                }
            }
        }
    }
}
#[test]
fn every_distinct_role_fits_the_fixed_call_budget_without_cache() {
    let t = fixture(4096, true);
    let (actual, calls) = run(&t, ListCursor::Initial, 4096, true);
    assert_eq!(actual.unwrap(), oracle(&t, ListCursor::Initial, 4096, true));
    // Parent fits one slice:3 entry checks+2 parent queries+1 parent generation,
    // 274 pages,273*3+1 generation batches for the final single entry.
    assert_eq!(calls, 1100);
    assert!(calls <= 1133);
}
#[test]
fn absent_unreadable_and_zero_generation_bodies_are_omitted_but_decoded_first() {
    let t = fixture(16, false);
    let c = Connection::open(&t.path).unwrap();
    c.execute(
        "DELETE FROM kv_nodes WHERE uid=?1 AND node_id=?2",
        params![UID, node(0)],
    )
    .unwrap();
    for (n, generation) in [(1, 0), (2, 2)] {
        let b = KvSmallFileBox {
            key: key(Role::OWNER, generation),
            ciphertext: vec![2; 32],
        }
        .encode()
        .unwrap();
        c.execute(
            "UPDATE kv_nodes SET exact_node=?1,ciphertext_bytes=NULL WHERE uid=?2 AND node_id=?3",
            params![b, UID, node(n)],
        )
        .unwrap();
    }
    assert_eq!(
        run(&t, ListCursor::Initial, 16, true).0.unwrap(),
        oracle(&t, ListCursor::Initial, 16, true)
    );
    c.execute(
        "UPDATE kv_nodes SET exact_node=X'C1',ciphertext_bytes=NULL WHERE uid=?1 AND node_id=?2",
        params![UID, node(1)],
    )
    .unwrap();
    assert!(matches!(
        run(&t, ListCursor::Initial, 16, true).0,
        Err(PersonalError::Corrupt)
    ));
    assert!(run(&t, ListCursor::Initial, 16, false).0.is_ok());
}
#[test]
fn parent_semantics_and_unsigned_time_error_precedence_are_preserved() {
    let t = fixture(1, false);
    let c = Connection::open(&t.path).unwrap();
    let expected = run(&t, ListCursor::Initial, 1, true).0.unwrap();
    let mut p = KvDirectory::decode(&parent(Role::OWNER, 1, 48)).unwrap();
    p.id = [99; 16];
    p.version = 999;
    c.execute(
        "UPDATE kv_directories SET exact_directory=?1 WHERE uid=?2",
        params![p.encode().unwrap(), UID],
    )
    .unwrap();
    assert_eq!(run(&t, ListCursor::Initial, 1, true).0.unwrap(), expected);
    c.execute(
        "UPDATE kv_directories SET exact_directory=?1 WHERE uid=?2",
        params![parent(Role::OWNER, 2, 48), UID],
    )
    .unwrap();
    assert!(matches!(
        run(&t, ListCursor::Time(u64::MAX), 1, true).0,
        Err(PersonalError::Permission)
    ));
    c.execute(
        "UPDATE kv_directories SET exact_directory=X'C1' WHERE uid=?1",
        params![UID],
    )
    .unwrap();
    assert!(matches!(
        run(&t, ListCursor::Time(u64::MAX), 1, true).0,
        Err(PersonalError::Corrupt)
    ));
    c.execute(
        "UPDATE kv_directories SET exact_directory=?1 WHERE uid=?2",
        params![parent(Role::OWNER, 1, 48), UID],
    )
    .unwrap();
    assert!(matches!(
        run(&t, ListCursor::Time(u64::MAX), 1, true).0,
        Err(PersonalError::Corrupt)
    ));
}
#[test]
fn revoked_or_changed_credential_cannot_use_previously_prepared_current() {
    let t = fixture(1, false);
    let c = Connection::open(&t.path).unwrap();
    let tx = c.unchecked_transaction().unwrap();
    let mut sql = NativeSql::new(&tx).unwrap();
    let credential = authenticate_peer(&mut sql, &peer(), 200).unwrap();
    tx.execute(
        "UPDATE devices SET role_type=2 WHERE uid=?1 AND device_id=?2",
        params![UID, DEVICE],
    )
    .unwrap();
    assert!(matches!(
        read_personal_list(
            &mut sql,
            &credential,
            &ListRequest::new(ROOT, ListCursor::Initial, 1, true),
            200,
            MAXIMUM_DIRECTORY_BYTES
        ),
        Err(PersonalError::StalePreparation)
    ));
    tx.rollback().unwrap();
}
struct Fault<'a> {
    inner: NativeSql<'a>,
    at: usize,
}
impl Sql for Fault<'_> {
    fn query(
        &mut self,
        s: foks_metadata_core::Statement,
        p: &[foks_metadata_core::Value],
    ) -> foks_metadata_core::Result<Option<Vec<foks_metadata_core::Value>>> {
        self.inner.query(s, p)
    }
    fn execute(
        &mut self,
        s: foks_metadata_core::Statement,
        p: &[foks_metadata_core::Value],
    ) -> foks_metadata_core::Result<u64> {
        self.inner.execute(s, p)
    }
    fn query_page(
        &mut self,
        s: foks_metadata_core::page_catalog::PageStatement,
        p: &[foks_metadata_core::Value],
    ) -> foks_metadata_core::Result<foks_metadata_core::page::PageBuffer> {
        if s == foks_metadata_core::page_catalog::PageStatement::RoleGenerations {
            self.at -= 1;
            if self.at == 0 {
                return Err(foks_metadata_core::Error::Storage);
            }
        }
        self.inner.query_page(s, p)
    }
}
#[test]
fn prior_permission_lookup_infrastructure_error_is_not_hidden_by_later_bad_body() {
    let t = fixture(3, false);
    let c = Connection::open(&t.path).unwrap();
    c.execute(
        "UPDATE kv_nodes SET exact_node=X'C1',ciphertext_bytes=NULL WHERE uid=?1 AND node_id=?2",
        params![UID, node(2)],
    )
    .unwrap();
    let tx = c.unchecked_transaction().unwrap();
    let mut sql = Fault {
        inner: NativeSql::new(&tx).unwrap(),
        at: 1,
    };
    let credential = authenticate_peer(&mut sql, &peer(), 200).unwrap();
    assert!(matches!(
        read_personal_list(
            &mut sql,
            &credential,
            &ListRequest::new(ROOT, ListCursor::Initial, 3, true),
            200,
            MAXIMUM_DIRECTORY_BYTES
        ),
        Err(PersonalError::Kernel(foks_metadata_core::Error::Storage))
    ));
}

#[test]
fn every_admission_failure_aborts_without_success_or_metadata_mutation() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    struct Budget {
        at: usize,
        calls: AtomicUsize,
    }
    impl foks_snowpack::AllocationBudget for Budget {
        fn reserve(&self, _: usize) -> bool {
            self.calls.fetch_add(1, Ordering::SeqCst) + 1 != self.at
        }
    }
    let t = fixture(2, false);
    let c = Connection::open(&t.path).unwrap();
    let tx = c.unchecked_transaction().unwrap();
    let mut sql = NativeSql::new(&tx).unwrap();
    let current = authenticate_peer(&mut sql, &peer(), 200).unwrap();
    let request = ListRequest::new(ROOT, ListCursor::Initial, 2, true);
    let baseline = Arc::new(Budget {
        at: usize::MAX,
        calls: AtomicUsize::new(0),
    });
    let expected = foks_snowpack::with_allocation_budget(baseline.clone(), || {
        read_personal_list(&mut sql, &current, &request, 200, MAXIMUM_DIRECTORY_BYTES)
    })
    .unwrap();
    let calls = baseline.calls.load(Ordering::SeqCst);
    assert!(calls > 20);
    for at in 1..=calls {
        let mut sql = NativeSql::new(&tx).unwrap();
        let budget = Arc::new(Budget {
            at,
            calls: AtomicUsize::new(0),
        });
        let result = foks_snowpack::with_allocation_budget(budget.clone(), || {
            read_personal_list(&mut sql, &current, &request, 200, MAXIMUM_DIRECTORY_BYTES)
        });
        assert!(
            matches!(
                result,
                Err(PersonalError::Kernel(foks_metadata_core::Error::Bounds))
            ),
            "admission {at}: {result:?}"
        );
        assert!(budget.calls.load(Ordering::SeqCst) >= at);
    }
    let mut sql = NativeSql::new(&tx).unwrap();
    assert_eq!(
        read_personal_list(&mut sql, &current, &request, 200, MAXIMUM_DIRECTORY_BYTES).unwrap(),
        expected
    );
    assert_eq!(tx.total_changes(), 0);
}

#[test]
fn malformed_storage_types_are_suppressed_before_payload_copy() {
    use foks_metadata_core::{page::Cell, page_catalog::PageStatement, Value};
    use rusqlite::types::Value as Stored;
    let t = fixture(1, false);
    for kind in [
        Stored::Null,
        Stored::Text("3".into()),
        Stored::Real(3.0),
        Stored::Blob(vec![3; 1024 * 1024]),
    ] {
        let c = Connection::open(&t.path).unwrap();
        // Projection-only malformed storage fixture. A production schema guard
        // must reject this temporary shadow; this is not an admitted database.
        c.execute_batch(
            "CREATE TEMP TABLE kv_nodes(uid BLOB,node_id BLOB,node_type,exact_node BLOB)",
        )
        .unwrap();
        let bytes = KvSmallFileBox {
            key: key(Role::OWNER, 1),
            ciphertext: vec![1; 32],
        }
        .encode()
        .unwrap();
        c.execute(
            "INSERT INTO temp.kv_nodes VALUES(?1,?2,?3,?4)",
            params![UID, node(0), kind, bytes],
        )
        .unwrap();
        assert!(c
            .query_row("SELECT node_type FROM kv_nodes", [], |r| r.get::<_, i64>(0))
            .is_err());
        let tx = c.unchecked_transaction().unwrap();
        let mut sql = NativeSql::new(&tx).unwrap();
        let page = sql
            .query_page(
                PageStatement::ListInitial,
                &[
                    Value::Blob(UID.to_vec()),
                    Value::Blob(ROOT.to_vec()),
                    Value::Integer(1),
                    Value::Integer(1),
                ],
            )
            .unwrap();
        let row = page.rows().next().unwrap();
        let cells: Vec<_> = row.cells().collect();
        assert_eq!(cells[7], Cell::Integer(1));
        assert_eq!(cells[8], Cell::Text("blob"));
        assert_eq!(cells[10], Cell::Null);
    }
}
