//! Real Request pools and native SQL, without a factory/route or readiness claim.
use super::*;
use foks_hosted_transport::memory::test_support::{
    self, snapshot, try_hold, Capacities, PressurePool,
};
#[path = "../../../../foks-server-db/tests/common/mod.rs"]
mod common;
use foks_metadata_core::{
    native::NativeSql,
    owned_list::{frame_owned_list, read_personal_list_owned, ListRetainedOwner, OwnedListBytes},
    page::PageBuffer,
    page_catalog::PageStatement,
    personal::{authenticate_peer, ListCursor, ListRequest, PeerCertificate},
    Sql, Statement, Value,
};
use foks_proto::{
    KvDirectory, KvDirectoryStatus, KvDirent, KvNodeId, Role, RoleAndGeneration, SecretBox,
};
use foks_server_db::KvDirectoryMutation;
use rusqlite::{params, Connection};
const CAPACITY: usize = 64 * 1024 * 1024;
const UID: [u8; 33] = [1; 33];
const DEVICE: [u8; 33] = [4; 33];
const ROOT: [u8; 16] = [0x51; 16];
fn pools() -> Arc<Pools> {
    test_support::pools(Capacities {
        working: CAPACITY,
        output: CAPACITY,
        small: 0,
        control: 4096,
        bulk: 4,
    })
    .unwrap()
}

fn peer() -> PeerCertificate {
    PeerCertificate::from_frontend_parts(vec![0x30, 0], vec![7], [4; 32], 1000).unwrap()
}
fn fixture(count: usize) -> common::TestDatabase {
    let mut fixture = common::TestDatabase::new();
    fixture.reserve(1_000_000);
    fixture.commit(None).unwrap();
    fixture
        .database
        .record_certificate(&[7], &UID, &DEVICE, &DEVICE, 100, 1000, peer().exact_der())
        .unwrap();
    let parent = KvDirectory {
        id: ROOT,
        version: 1,
        key: RoleAndGeneration {
            role: Role::OWNER,
            generation: 1,
        },
        seed_ciphertext: vec![1; 48],
        write_role: Role::OWNER,
        status: KvDirectoryStatus::Active,
    }
    .encode()
    .unwrap();
    fixture
        .database
        .put_kv_directory(&KvDirectoryMutation {
            uid: &UID,
            id: &ROOT,
            version: 1,
            key_role: 3,
            key_visibility: 0,
            key_generation: 1,
            status: 0,
            exact: &parent,
        })
        .unwrap();
    let mut connection = Connection::open(&fixture.path).unwrap();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    let transaction = connection.transaction().unwrap();
    for n in 0..count {
        let mut id = [0; 16];
        id[8..].copy_from_slice(&(n as u64).to_be_bytes());
        let mut mac = [0; 32];
        mac[16..].copy_from_slice(&id);
        let mut node = [3; 17];
        node[1..].copy_from_slice(&id);
        let entry = KvDirent::new(
            ROOT,
            id,
            KvNodeId(node),
            1,
            1,
            Role::OWNER,
            mac,
            SecretBox {
                nonce: [7; 16],
                ciphertext: vec![8; 16],
            },
            KvDirectoryStatus::Active,
            [9; 32],
            n as u64,
        )
        .unwrap();
        transaction.execute("INSERT INTO kv_dirents(uid,parent_id,dirent_id,version,directory_version,node_id,name_mac,creation_time,exact_dirent) VALUES(?1,?2,?3,1,1,?4,?5,?6,?7)",params![UID,ROOT,id,node,mac,n as i64,entry.encode().unwrap()]).unwrap();
        transaction
            .execute(
                "INSERT INTO kv_dirent_heads VALUES(?1,?2,?3,1)",
                params![UID, ROOT, id],
            )
            .unwrap();
    }
    transaction.commit().unwrap();
    fixture
}
struct Observe<'a> {
    sql: NativeSql<'a>,
    pools: Arc<Pools>,
    list_pages: usize,
    before: [usize; 32],
}
impl Sql for Observe<'_> {
    fn query(
        &mut self,
        s: Statement,
        p: &[Value],
    ) -> foks_metadata_core::Result<Option<Vec<Value>>> {
        self.sql.query(s, p)
    }
    fn execute(&mut self, _: Statement, _: &[Value]) -> foks_metadata_core::Result<u64> {
        panic!("List mutation");
    }
    fn query_page(
        &mut self,
        s: PageStatement,
        p: &[Value],
    ) -> foks_metadata_core::Result<PageBuffer> {
        let list = matches!(
            s,
            PageStatement::ListInitial | PageStatement::ListMacContinue
        );
        let before = snapshot(&self.pools).working;
        let result = self.sql.query_page(s, p);
        if list {
            assert_eq!(
                snapshot(&self.pools).working,
                before,
                "page SQL consumes local credits only"
            );
            self.before[self.list_pages] = before;
            self.list_pages += 1;
        }
        result
    }
}
fn read(
    fixture: &common::TestDatabase,
    owner: &Arc<Request>,
    pools: &Arc<Pools>,
) -> (OwnedListBytes, usize, [usize; 32]) {
    let connection = Connection::open(&fixture.path).unwrap();
    let transaction = connection.unchecked_transaction().unwrap();
    let mut sql = Observe {
        sql: NativeSql::new(&transaction).unwrap(),
        pools: pools.clone(),
        list_pages: 0,
        before: [0; 32],
    };
    let current = owner
        .sync(|| authenticate_peer(&mut sql, &peer(), 200))
        .unwrap();
    // Deliberately omit owner.sync here. The public owned helper must install A
    // even when its caller has an unrelated ambient Request B.
    let result = read_personal_list_owned(
        &mut sql,
        &current,
        &ListRequest::new(ROOT, ListCursor::Initial, 4096, false),
        200,
        1 << 20,
        owner.clone(),
    )
    .unwrap();
    let pages = sql.list_pages;
    let before = sql.before;
    drop(sql);
    transaction.rollback().unwrap();
    (result, pages, before)
}
#[test]
fn actual_multi_page_list_and_frame_use_original_pools_and_restore_ambient() {
    let fixture = fixture(65);
    let a = pools();
    let b = pools();
    let owner = a.request(None);
    let ambient = b.request(None);
    ambient.fail();
    let (raw, pages, before) = ambient.sync(|| read(&fixture, &owner, &a));
    assert_eq!(pages, 5);
    assert!(CAPACITY - before[0] > 10 * 1024 * 1024);
    for pair in before[..pages].windows(2) {
        assert!(pair[0] >= pair[1]);
        assert!(
            pair[0] - pair[1] < 128 * 1024,
            "no second page workspace reservation"
        );
    }
    assert_eq!(snapshot(&b).working, CAPACITY);
    assert_eq!(snapshot(&b).output, CAPACITY);
    let expected = foks_rpc::encode_success_response_at(raw.as_bytes(), 256).unwrap();
    let length = raw.len();
    let frame = ambient.sync(|| {
        let frame = frame_owned_list(raw, 256).unwrap();
        foks_snowpack::reserve_allocation(11).unwrap();
        let _ = foks_rpc::encode_success_response_at(&[0xc0], 0).unwrap();
        frame
    });
    assert_eq!(frame.as_bytes(), expected);
    assert!(!frame.is_empty());
    assert_eq!(snapshot(&a).output, CAPACITY - 2 * (length + 128));
    assert_eq!(snapshot(&b).working, CAPACITY - 11);
    assert_eq!(snapshot(&b).output, CAPACITY - 2 * (1 + 128));
    let weak = Arc::downgrade(&owner);
    drop(owner);
    assert!(weak.upgrade().is_some());
    drop(frame);
    assert!(weak.upgrade().is_none());
    assert_eq!(snapshot(&a).working, CAPACITY);
    assert_eq!(snapshot(&a).output, CAPACITY);
}
#[test]
fn output_refusal_and_sticky_owner_preserve_working_and_control_policy() {
    let fixture = fixture(0);
    let a = pools();
    let owner = a.request(None);
    let (raw, pages, _) = read(&fixture, &owner, &a);
    assert_eq!(pages, 1);
    assert!(!raw.is_empty());
    let held = snapshot(&a).working;
    let exhausted = try_hold(&a, PressurePool::Output, CAPACITY).unwrap();
    assert!(
        matches!(frame_owned_list(raw,0),Err(foks_rpc::Error::Io(ref error)) if error.kind()==std::io::ErrorKind::WouldBlock)
    );
    assert!(owner.failed());
    assert_eq!(snapshot(&a).working, held);
    assert!(!owner.reserve_retained(1));
    assert!(foks_rpc::ResponseAllocationBudget::reserve(
        &*owner, 7, true
    ));
    assert!(owner.failed());
    assert_eq!(snapshot(&a).control, 4096 - 7);
    drop(exhausted);
    drop(owner);
    assert_eq!(snapshot(&a).working, CAPACITY);
    assert_eq!(snapshot(&a).control, 4096);
}
#[test]
fn workspace_refusal_precedes_list_sql_and_preserves_domain_error() {
    let fixture = fixture(1);
    let a = pools();
    let owner = a.request(None);
    let connection = Connection::open(&fixture.path).unwrap();
    let transaction = connection.unchecked_transaction().unwrap();
    let mut sql = Observe {
        sql: NativeSql::new(&transaction).unwrap(),
        pools: a.clone(),
        list_pages: 0,
        before: [0; 32],
    };
    let current = owner
        .sync(|| authenticate_peer(&mut sql, &peer(), 200))
        .unwrap();
    // Leave room for parent/entry work, but not the reviewed >10MiB workspace.
    let held = try_hold(
        &a,
        PressurePool::Working,
        snapshot(&a).working - 1024 * 1024,
    )
    .unwrap();
    let result = read_personal_list_owned(
        &mut sql,
        &current,
        &ListRequest::new(ROOT, ListCursor::Initial, 1, false),
        200,
        1 << 20,
        owner.clone(),
    );
    assert!(matches!(
        result,
        Err(foks_metadata_core::personal::PersonalError::Kernel(
            foks_metadata_core::Error::Bounds
        ))
    ));
    assert_eq!(sql.list_pages, 0);
    assert!(owner.failed());
    drop(held);
    drop(sql);
    transaction.rollback().unwrap();
    // Earlier missing-parent error remains its original domain result.
    let b = pools();
    let other = b.request(None);
    let tx = connection.unchecked_transaction().unwrap();
    let mut sql = NativeSql::new(&tx).unwrap();
    let result = read_personal_list_owned(
        &mut sql,
        &current,
        &ListRequest::new([0x52; 16], ListCursor::Initial, 1, false),
        200,
        1 << 20,
        other.clone(),
    );
    assert!(matches!(
        result,
        Err(foks_metadata_core::personal::PersonalError::Missing)
    ));
    assert!(!other.failed());
}
#[test]
fn detached_worker_and_unwrapped_result_retain_exact_request_until_final_bytes_drop() {
    let fixture = fixture(0);
    let a = pools();
    let owner = a.request(None);
    let weak = Arc::downgrade(&owner);
    let (raw, _, _) = read(&fixture, &owner, &a);
    let (arrived, ready) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let work = owner.sync(|| {
        retain_work(move || {
            arrived.send(()).unwrap();
            wait.recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            raw
        })
    });
    let thread = std::thread::spawn(move || work.run());
    ready
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    drop(owner);
    assert!(weak.upgrade().is_some());
    assert!(snapshot(&a).working < CAPACITY);
    release.send(()).unwrap();
    let raw = thread.join().unwrap().into_inner();
    assert!(weak.upgrade().is_some());
    let frame = frame_owned_list(raw, 0).unwrap();
    assert!(snapshot(&a).output < CAPACITY);
    drop(frame);
    assert!(weak.upgrade().is_none());
    assert_eq!(snapshot(&a).working, CAPACITY);
    assert_eq!(snapshot(&a).output, CAPACITY);
}

#[tokio::test]
async fn blocked_physical_writer_keeps_frame_owner_after_waiter_is_gone() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let fixture = fixture(0);
    let a = pools();
    let owner = a.request(None);
    let weak = Arc::downgrade(&owner);
    let (raw, _, _) = read(&fixture, &owner, &a);
    let frame = frame_owned_list(raw, 0).unwrap();
    let expected = frame.as_bytes().to_vec();
    let (mut writer, mut reader) = tokio::io::duplex(1);
    let task = tokio::spawn(async move {
        writer.write_all(frame.as_bytes()).await.unwrap();
        writer.shutdown().await.unwrap();
        drop(frame);
    });
    drop(owner);
    let mut first = [0; 1];
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        reader.read_exact(&mut first),
    )
    .await
    .unwrap()
    .unwrap();
    // A one-byte pipe cannot accept the remaining frame without this reader.
    assert!(!task.is_finished());
    assert!(weak.upgrade().is_some());
    assert!(snapshot(&a).working < CAPACITY);
    assert!(snapshot(&a).output < CAPACITY);
    let mut actual = first.to_vec();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        reader.read_to_end(&mut actual),
    )
    .await
    .unwrap()
    .unwrap();
    task.await.unwrap();
    assert_eq!(actual, expected);
    assert!(weak.upgrade().is_none());
    assert_eq!(snapshot(&a).working, CAPACITY);
    assert_eq!(snapshot(&a).output, CAPACITY);
}

#[test]
fn pre_page_error_and_panic_restore_both_hooks_with_original_failure_provenance() {
    struct FaultSql(bool);
    impl Sql for FaultSql {
        fn query(
            &mut self,
            _: Statement,
            _: &[Value],
        ) -> foks_metadata_core::Result<Option<Vec<Value>>> {
            foks_snowpack::reserve_allocation(17).unwrap();
            if self.0 {
                panic!("pre-page SQL injection");
            }
            Err(foks_metadata_core::Error::Expired)
        }
        fn execute(&mut self, _: Statement, _: &[Value]) -> foks_metadata_core::Result<u64> {
            panic!("mutation");
        }
    }
    let fixture = fixture(0);
    let connection = Connection::open(&fixture.path).unwrap();
    let transaction = connection.unchecked_transaction().unwrap();
    let mut sql = NativeSql::new(&transaction).unwrap();
    let current = authenticate_peer(&mut sql, &peer(), 200).unwrap();
    for panic in [false, true] {
        let a = pools();
        let b = pools();
        let owner = a.request(None);
        let ambient = b.request(None);
        ambient.sync(|| {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                read_personal_list_owned(
                    &mut FaultSql(panic),
                    &current,
                    &ListRequest::new(ROOT, ListCursor::Initial, 1, false),
                    200,
                    1 << 20,
                    owner.clone(),
                )
            }));
            if panic {
                assert!(result.is_err());
            } else {
                assert!(matches!(
                    result.unwrap(),
                    Err(foks_metadata_core::personal::PersonalError::Kernel(
                        foks_metadata_core::Error::Expired
                    ))
                ));
            }
            assert_eq!(owner.failed(), panic);
            foks_snowpack::reserve_allocation(11).unwrap();
            foks_rpc::encode_success_response_at(&[0xc0], 0).unwrap();
        });
        // CredentialCurrent's three S3a-owned parameter copies precede the
        // injected SQL charge and must also belong to the captured owner A.
        let entry_copies =
            current.owner_id().len() + current.uid().len() + current.credential_id().len();
        assert_eq!(snapshot(&a).working, CAPACITY - entry_copies - 17);
        assert_eq!(snapshot(&a).output, CAPACITY);
        assert_eq!(snapshot(&b).working, CAPACITY - 11);
        assert_eq!(snapshot(&b).output, CAPACITY - 2 * (1 + 128));
        assert!(!ambient.failed());
    }
}
