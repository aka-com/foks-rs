use super::super::{
    output::Frame,
    tests::{call, list, Fixture, NOW, PARENT, UID},
};
use super::*;
use foks_proto::Role;
use std::{
    cell::RefCell,
    sync::{atomic::AtomicUsize, mpsc},
    time::Duration,
};

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Stage {
    MainBeforeCommit,
    MainCommitted,
    FreshEntered,
    FreshBeforeCommit,
}
type Action = Box<dyn FnOnce(&rusqlite::Connection, &OperationControl) -> Result<()>>;
thread_local! {static FAULT:RefCell<Option<(Stage,Action)>>=const{RefCell::new(None)};}
fn hook(
    stage: Stage,
    action: impl FnOnce(&rusqlite::Connection, &OperationControl) -> Result<()> + 'static,
) {
    FAULT.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some((stage, Box::new(action)));
    });
}
pub(super) fn stage(
    stage: Stage,
    connection: &rusqlite::Connection,
    control: &OperationControl,
) -> Result<()> {
    let action = FAULT.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot
            .as_ref()
            .is_some_and(|(expected, _)| *expected == stage)
        {
            slot.take().map(|(_, action)| action)
        } else {
            None
        }
    });
    match action {
        Some(action) => action(connection, control),
        None => Ok(()),
    }
}
struct FixedClock(u64);
impl Clock for FixedClock {
    fn now_micros(&self) -> foks_server_db::Result<u64> {
        Ok(self.0)
    }
}
struct FailClock {
    at: usize,
    calls: AtomicUsize,
}
impl Clock for FailClock {
    fn now_micros(&self) -> foks_server_db::Result<u64> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == self.at {
            Err(foks_server_db::Error::Invalid("injected clock"))
        } else {
            Ok(NOW)
        }
    }
}
fn run(fixture: &Fixture, call: DecodedCall, clock: &dyn Clock) -> Result<Pending> {
    let pools = crate::net::memory::pools(Default::default()).unwrap();
    let owner = fixture.owner(&pools);
    let peer = fixture.peer(&pools);
    let memory = pools.admitted_request(None).unwrap();
    let control = OperationControl::test(Duration::from_secs(60), memory);
    let permit = Arc::new(tokio::sync::Semaphore::new(1))
        .try_acquire_owned()
        .unwrap();
    execute(
        Work {
            owner,
            call,
            peer,
            control,
            execution: permit,
        },
        clock,
    )
}
fn expect_status(pending: &Pending, status: RpcStatus) {
    let expected = foks_rpc::encode_status_response_at(&status, pending.call.sequence()).unwrap();
    let Frame::Status(actual) = &pending.frame else {
        panic!("expected status")
    };
    assert_eq!(actual, &expected);
}
fn bad_outer() -> DecodedCall {
    call(
        foks_rpc::encode_call(
            foks_rpc::KV_STORE_PROTOCOL_ID,
            foks_rpc::KV_LIST_METHOD_POSITION,
            &foks_snowpack::encode(&foks_snowpack::Value::Null).unwrap(),
            17,
        )
        .unwrap(),
    )
}
#[test]
fn earlier_syntax_status_publishes_without_a_never_observed_anchor_and_commits_floor_once() {
    let fixture = Fixture::new(false, Role::OWNER, 0);
    let mut pending = run(&fixture, bad_outer(), &FixedClock(NOW)).unwrap();
    assert_eq!(pending.journal, Journal::OuterSyntax);
    expect_status(
        &pending,
        RpcStatus::BadArguments("KV argument is not a struct".into()),
    );
    assert_eq!(pending.owner.expected.floor, NOW / 1000);
    let dependency = pending.observed;
    fresh(&mut pending, &FixedClock(NOW + 1)).unwrap();
    assert_eq!(pending.owner.expected.floor, NOW / 1000);
    assert_eq!(pending.observed, dependency);
    assert!(pending.budget.charged() <= 1492);
    assert_eq!(pending.budget.charged() - pending.budget.attempts(), 2);
}
#[test]
fn maximum_admitted_list_matches_native_bytes_under_one_shared_sql_budget() {
    let fixture = Fixture::new(true, Role::OWNER, 4096);
    let expected = fixture.oracle(4096, true);
    let mut pending = run(
        &fixture,
        list(PARENT, foks_rpc::KvListCursor::None),
        &FixedClock(NOW),
    )
    .unwrap();
    fresh(&mut pending, &FixedClock(NOW + 1)).unwrap();
    let Frame::List(frame) = &pending.frame else {
        panic!("maximum admitted List should complete")
    };
    let content =
        foks_rpc::read_frame(&mut frame.as_bytes(), foks_rpc::DEFAULT_MAX_FRAME_LENGTH).unwrap();
    assert_eq!(
        foks_rpc::decode_response(&content, pending.call.sequence()).unwrap(),
        expected
    );
    assert!(pending.budget.charged() <= 1492);
    assert_eq!(pending.budget.charged() - pending.budget.attempts(), 2);
}
#[test]
fn missing_owner_missing_anchor_and_invalid_role_preserve_native_permission_prefix() {
    for choice in 0..3 {
        let fixture = Fixture::new(choice != 1, Role::OWNER, 0);
        let source = rusqlite::Connection::open(&fixture.database).unwrap();
        if choice == 0 {
            let mut subkey = fixture.device;
            subkey[0] = foks_proto::ENTITY_SUBKEY;
            source
                .execute(
                    "UPDATE issued_certificates SET credential_id=?1",
                    [subkey.as_slice()],
                )
                .unwrap();
        } else if choice == 2 {
            source
                .execute("UPDATE devices SET visibility=1", [])
                .unwrap();
        }
        drop(source);
        let mut pending = run(
            &fixture,
            list(PARENT, foks_rpc::KvListCursor::None),
            &FixedClock(NOW),
        )
        .unwrap();
        assert!(match choice {
            0 => matches!(pending.journal, Journal::NoOwner),
            1 => matches!(pending.journal, Journal::NoAnchor(_)),
            _ => matches!(pending.journal, Journal::InvalidSelectedRole { .. }),
        });
        expect_status(
            &pending,
            RpcStatus::PermissionDenied(
                "active device does not authorize this KV namespace".into(),
            ),
        );
        fresh(&mut pending, &FixedClock(NOW + 1)).unwrap();
    }
}
#[test]
fn actual_native_route_and_clock_error_order_is_retained() {
    let fixture = Fixture::new(false, Role::OWNER, 0);
    let clock = FailClock {
        at: 1,
        calls: AtomicUsize::new(0),
    };
    let unknown = call(foks_rpc::encode_call(0xdeadbeef, 7, &[0xc0], 1).unwrap());
    let mut pending = run(&fixture, unknown, &clock).unwrap();
    expect_status(&pending, RpcStatus::Unsupported);
    assert_eq!(clock.calls.load(Ordering::SeqCst), 1);
    fresh(&mut pending, &FixedClock(NOW + 1)).unwrap();
    for failed in [1, 2] {
        let clock = FailClock {
            at: failed,
            calls: AtomicUsize::new(0),
        };
        let mut pending =
            run(&fixture, list(PARENT, foks_rpc::KvListCursor::None), &clock).unwrap();
        expect_status(&pending, RpcStatus::TransactionRetry);
        assert_eq!(clock.calls.load(Ordering::SeqCst), failed + 1);
        assert_eq!(
            pending.journal,
            if failed == 1 {
                Journal::InitialRoute
            } else {
                Journal::AuthorityClock
            }
        );
        fresh(&mut pending, &FixedClock(NOW + 1)).unwrap();
    }
    assert!(run(
        &fixture,
        bad_outer(),
        &FailClock {
            at: 0,
            calls: AtomicUsize::new(0)
        }
    )
    .is_err());
}
#[test]
fn empty_imported_team_ledger_returns_native_expired_without_personal_authority() {
    let fixture = Fixture::new(false, Role::OWNER, 0);
    let request = call(
        foks_rpc::encode_kv_list_request(
            foks_rpc::KvAuth::Team(&[8; 16]),
            &PARENT,
            foks_rpc::KvListCursor::None,
            1,
            false,
        )
        .unwrap(),
    );
    let mut pending = run(&fixture, request, &FixedClock(NOW)).unwrap();
    assert_eq!(pending.journal, Journal::AuthorityClock);
    expect_status(&pending, RpcStatus::Expired);
    fresh(&mut pending, &FixedClock(NOW + 1)).unwrap();
}
#[test]
fn cursor_overflow_preserves_missing_and_denied_parent_precedence() {
    for choice in 0..3 {
        let fixture = Fixture::new(true, Role::OWNER, 0);
        if choice == 1 {
            let parent = foks_proto::KvDirectory {
                id: PARENT,
                version: 1,
                key: foks_proto::RoleAndGeneration {
                    role: Role::OWNER,
                    generation: 2,
                },
                seed_ciphertext: vec![1; 48],
                write_role: Role::OWNER,
                status: foks_proto::KvDirectoryStatus::Active,
            }
            .encode()
            .unwrap();
            rusqlite::Connection::open(&fixture.database)
                .unwrap()
                .execute(
                    "UPDATE kv_directories SET key_generation=2,exact_directory=?1",
                    [parent],
                )
                .unwrap();
        }
        let parent = if choice == 0 { [0xee; 16] } else { PARENT };
        let reader =
            foks_server_db::ReadDatabase::open(&fixture.database, Default::default()).unwrap();
        if choice == 2 {
            assert!(reader
                .kv_list(
                    &UID,
                    &parent,
                    Some(foks_server_db::KvListCursor::Time(u64::MAX)),
                    4096
                )
                .is_err());
        }
        drop(reader);
        let mut pending = run(
            &fixture,
            list(parent, foks_rpc::KvListCursor::Time(u64::MAX)),
            &FixedClock(NOW),
        )
        .unwrap();
        let expected = match choice {
            0 => RpcStatus::KvNoEnt,
            1 => RpcStatus::PermissionDenied(
                "active device does not authorize this KV namespace".into(),
            ),
            _ => RpcStatus::TransactionRetry,
        };
        expect_status(&pending, expected);
        assert!(!pending.control.memory().failed());
        fresh(&mut pending, &FixedClock(NOW + 1)).unwrap();
        assert!(!pending.control.memory().failed());
    }
}
#[test]
fn malformed_admitted_list_blobs_return_native_retry_but_no_partial_success() {
    for table in ["kv_directories", "kv_dirents", "kv_nodes"] {
        let fixture = Fixture::new(true, Role::OWNER, 1);
        let column = match table {
            "kv_directories" => "exact_directory",
            "kv_dirents" => "exact_dirent",
            _ => "exact_node",
        };
        // Mark the modified node's derived usage unvalidated; schema guards stay active.
        let reset_usage = if table == "kv_nodes" {
            ",ciphertext_bytes=NULL"
        } else {
            ""
        };
        rusqlite::Connection::open(&fixture.database)
            .unwrap()
            .execute(
                &format!("UPDATE {table} SET {column}=X'00'{reset_usage}"),
                [],
            )
            .unwrap();
        let mut pending = run(
            &fixture,
            list(PARENT, foks_rpc::KvListCursor::None),
            &FixedClock(NOW),
        )
        .unwrap();
        expect_status(&pending, RpcStatus::TransactionRetry);
        assert!(!pending.control.memory().failed());
        fresh(&mut pending, &FixedClock(NOW + 1)).unwrap();
        assert!(!pending.control.memory().failed());
    }
}
#[test]
fn corrupt_later_row_discards_a_valid_prefix_and_retains_retry_freshness() {
    // The helper must finish its first 15-row page and retain that encoded
    // prefix before encountering corruption on its second page.
    let fixture = Fixture::new(true, Role::OWNER, 17);
    let mut last = [0u8; 16];
    last[8..].copy_from_slice(&16u64.to_be_bytes());
    assert_eq!(
        rusqlite::Connection::open(&fixture.database)
            .unwrap()
            .execute(
                "UPDATE kv_dirents SET exact_dirent=X'00' WHERE dirent_id=?1",
                [last.as_slice()],
            )
            .unwrap(),
        1
    );
    let reader = foks_server_db::ReadDatabase::open(&fixture.database, Default::default()).unwrap();
    let native = reader.kv_list(&UID, &PARENT, None, 4096).unwrap();
    assert_eq!(native.len(), 17);
    assert!(native[..16]
        .iter()
        .all(|entry| foks_proto::KvDirent::decode(&entry.exact).is_ok()));
    assert!(foks_proto::KvDirent::decode(&native[16].exact).is_err());
    drop(reader);
    let mut pending = run(
        &fixture,
        list(PARENT, foks_rpc::KvListCursor::None),
        &FixedClock(NOW),
    )
    .unwrap();
    expect_status(&pending, RpcStatus::TransactionRetry);
    assert!(!pending.control.memory().failed());
    fresh(&mut pending, &FixedClock(NOW + 1)).unwrap();
    assert!(!pending.control.memory().failed());
}
#[test]
fn retry_candidate_cannot_publish_after_original_request_allocation_refusal() {
    let fixture = Fixture::new(true, Role::OWNER, 1);
    let reached = Arc::new(AtomicBool::new(false));
    let observed = reached.clone();
    hook(Stage::MainBeforeCommit, move |_, control| {
        assert!(!control.memory().failed());
        assert!(!foks_snowpack::AllocationBudget::reserve(
            control.memory().as_ref(),
            usize::MAX,
        ));
        assert!(control.memory().failed());
        observed.store(true, Ordering::SeqCst);
        Ok(())
    });
    assert!(matches!(
        run(
            &fixture,
            list(PARENT, foks_rpc::KvListCursor::Time(u64::MAX)),
            &FixedClock(NOW),
        ),
        Err(Error::Bounds)
    ));
    assert!(reached.load(Ordering::SeqCst));
}
#[test]
fn fresh_expiry_clock_rewind_revision_or_activation_change_discards_candidate() {
    let fixture = Fixture::new(true, Role::OWNER, 1);
    for choice in 0..4 {
        let mut pending = run(
            &fixture,
            list(PARENT, foks_rpc::KvListCursor::None),
            &FixedClock(NOW),
        )
        .unwrap();
        let now = match choice {
            0 => 2_000_000_000_000_000,
            1 => NOW - 1000,
            _ => NOW + 1,
        };
        if choice == 2 {
            pending
                .owner
                .connection
                .execute("UPDATE devices SET active=0", [])
                .unwrap();
        }
        if choice == 3 {
            pending
                .owner
                .connection
                .execute("UPDATE metadata_authority SET owner=zeroblob(16)", [])
                .unwrap();
        }
        assert!(fresh(&mut pending, &FixedClock(now)).is_err());
    }
}
#[test]
fn cancellation_at_each_transaction_commit_boundary_never_publishes() {
    let fixture = Fixture::new(true, Role::OWNER, 1);
    hook(Stage::MainBeforeCommit, |_, control| {
        control.cancel();
        Ok(())
    });
    assert!(run(
        &fixture,
        list(PARENT, foks_rpc::KvListCursor::None),
        &FixedClock(NOW)
    )
    .is_err());
    let mut pending = run(
        &fixture,
        list(PARENT, foks_rpc::KvListCursor::None),
        &FixedClock(NOW),
    )
    .unwrap();
    hook(Stage::FreshBeforeCommit, |_, control| {
        control.cancel();
        Ok(())
    });
    assert_eq!(
        fresh(&mut pending, &FixedClock(NOW + 1)),
        Err(Error::Expired)
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aborted_waiter_retains_original_memory_execution_and_physical_owner_until_worker_drops() {
    struct Completion {
        value: Option<Result<Pending>>,
        done: Option<mpsc::Sender<()>>,
    }
    impl Drop for Completion {
        fn drop(&mut self) {
            drop(self.value.take());
            if let Some(done) = self.done.take() {
                let _ = done.send(());
            }
        }
    }
    let fixture = Fixture::new(true, Role::OWNER, 2);
    let pools = crate::net::memory::pools(Default::default()).unwrap();
    let owner = fixture.owner(&pools);
    let path = std::path::PathBuf::from(owner.connection.path().unwrap());
    let peer = fixture.peer(&pools);
    let memory = pools.admitted_request(None).unwrap();
    let weak = Arc::downgrade(&memory);
    let control = OperationControl::test(Duration::from_secs(60), memory);
    let execution = Arc::new(tokio::sync::Semaphore::new(1));
    let permit = execution.clone().try_acquire_owned().unwrap();
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let (done_tx, done) = mpsc::channel();
    let work = Work {
        owner,
        call: list(PARENT, foks_rpc::KvListCursor::None),
        peer,
        control,
        execution: permit,
    };
    let task = tokio::task::spawn_blocking(move || {
        hook(Stage::MainCommitted, move |_, _| {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(())
        });
        Completion {
            value: Some(execute(work, &FixedClock(NOW))),
            done: Some(done_tx),
        }
    });
    entered.recv_timeout(Duration::from_secs(30)).unwrap();
    task.abort();
    drop(task);
    assert!(weak.upgrade().is_some());
    assert_eq!(execution.available_permits(), 0);
    assert!(crate::DatabaseWriterGuard::acquire(&path).is_err());
    assert!(path.exists());
    release.send(()).unwrap();
    done.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(weak.upgrade().is_none());
    assert_eq!(execution.available_permits(), 1);
    assert!(!path.exists());
}

#[derive(Default)]
struct WriteGate {
    write: std::cell::Cell<bool>,
    flush: std::cell::Cell<bool>,
    bytes: std::cell::Cell<usize>,
    flush_polls: std::cell::Cell<usize>,
}
struct GateWriter(std::rc::Rc<WriteGate>);
impl tokio::io::AsyncWrite for GateWriter {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        if !self.0.write.get() {
            return std::task::Poll::Pending;
        }
        self.0.bytes.set(self.0.bytes.get() + bytes.len());
        std::task::Poll::Ready(Ok(bytes.len()))
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.0.flush_polls.set(self.0.flush_polls.get() + 1);
        if self.0.flush.get() {
            std::task::Poll::Ready(Ok(()))
        } else {
            std::task::Poll::Pending
        }
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.poll_flush(context)
    }
}
enum OutputFinish {
    Flush,
    OriginalTimer,
    Abandon,
}
async fn physical_output_case(fixture: &Fixture, at_flush: bool, finish: OutputFinish) {
    use std::{future::Future as _, task::Poll};
    let pools = crate::net::memory::pools(Default::default()).unwrap();
    let owner = fixture.owner(&pools);
    let path = std::path::PathBuf::from(owner.connection.path().unwrap());
    let peer = fixture.peer(&pools);
    let memory = pools.admitted_request(None).unwrap();
    let weak = Arc::downgrade(&memory);
    let control = OperationControl::test(Duration::from_secs(60), memory);
    let deadline = control.deadline();
    let execution = Arc::new(tokio::sync::Semaphore::new(1));
    let permit = execution.clone().try_acquire_owned().unwrap();
    let mut pending = execute(
        Work {
            owner,
            call: list(PARENT, foks_rpc::KvListCursor::None),
            peer,
            control,
            execution: permit,
        },
        &FixedClock(NOW),
    )
    .unwrap();
    fresh(&mut pending, &FixedClock(NOW + 1)).unwrap();
    let expected_length = match &pending.frame {
        Frame::List(frame) => frame.as_bytes().len(),
        Frame::Status(frame) => frame.len(),
    };
    let gate = std::rc::Rc::new(WriteGate::default());
    gate.write.set(at_flush);
    let mut writer = GateWriter(gate.clone());
    let limits = crate::SessionLimits {
        // The I/O phase cap exceeds the original request budget. Advancing
        // beyond the original deadline below must still abandon the response.
        io_timeout: Duration::from_secs(120),
        ..Default::default()
    };
    tokio::time::pause();
    let mut transmit = Box::pin(pending.transmit(&mut writer, limits));
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(transmit.as_mut().poll(&mut context).is_pending());
    assert_eq!(gate.bytes.get(), if at_flush { expected_length } else { 0 });
    assert_eq!(gate.flush_polls.get() > 0, at_flush);
    assert!(weak.upgrade().is_some());
    assert_eq!(execution.available_permits(), 0);
    assert!(crate::DatabaseWriterGuard::acquire(&path).is_err());
    assert!(path.exists());
    let returned = match finish {
        OutputFinish::Flush => {
            // If first blocked on write, independently pause again at flush.
            gate.write.set(true);
            assert!(transmit.as_mut().poll(&mut context).is_pending());
            assert_eq!(gate.bytes.get(), expected_length);
            assert!(gate.flush_polls.get() > 0);
            assert!(weak.upgrade().is_some());
            assert_eq!(execution.available_permits(), 0);
            assert!(crate::DatabaseWriterGuard::acquire(&path).is_err());
            gate.flush.set(true);
            let Poll::Ready(Some(owner)) = transmit.as_mut().poll(&mut context) else {
                panic!("completed physical flush must return the original owner")
            };
            Some(owner)
        }
        OutputFinish::OriginalTimer => {
            // Paused Tokio time exercises the real timeout_at timer. It does
            // not claim that std::Instant's independent wall clock advanced.
            let remaining = tokio::time::Instant::from_std(deadline.instant())
                .saturating_duration_since(tokio::time::Instant::now());
            tokio::time::advance(remaining + Duration::from_millis(1)).await;
            let advanced = tokio::time::Instant::now();
            // advance may return before every timer has been processed. Poll
            // with the real task waker until this already-due timer completes;
            // no further virtual time may elapse toward the longer I/O cap.
            assert!(transmit.as_mut().await.is_none());
            assert_eq!(tokio::time::Instant::now(), advanced);
            None
        }
        OutputFinish::Abandon => None,
    };
    // Includes cancellation while the future is Pending at either physical
    // boundary. No bare frame or execution permit can outlive this owner.
    drop(transmit);
    tokio::time::resume();
    assert!(weak.upgrade().is_none());
    assert_eq!(execution.available_permits(), 1);
    if returned.is_some() {
        assert!(path.exists());
        assert!(crate::DatabaseWriterGuard::acquire(&path).is_err());
    }
    drop(returned);
    assert!(!path.exists());
}
#[tokio::test]
async fn physical_response_holds_ownership_through_write_and_flush_then_reuses_database() {
    let fixture = Fixture::new(true, Role::OWNER, 2);
    for at_flush in [false, true] {
        physical_output_case(&fixture, at_flush, OutputFinish::Flush).await;
    }
}
#[tokio::test]
async fn original_output_timer_abandons_at_write_without_extending_budget() {
    let fixture = Fixture::new(true, Role::OWNER, 2);
    physical_output_case(&fixture, false, OutputFinish::OriginalTimer).await;
}
#[tokio::test]
async fn original_output_timer_abandons_at_flush_without_extending_budget() {
    // Each timer-expiry case has its own runtime: resume preserves Tokio's
    // advanced offset, whereas the next OperationControl uses std::Instant.
    let fixture = Fixture::new(true, Role::OWNER, 2);
    physical_output_case(&fixture, true, OutputFinish::OriginalTimer).await;
}
#[tokio::test]
async fn cancelling_physical_output_at_write_or_flush_releases_its_whole_owner() {
    let fixture = Fixture::new(true, Role::OWNER, 2);
    for at_flush in [false, true] {
        physical_output_case(&fixture, at_flush, OutputFinish::Abandon).await;
    }
}

#[test]
fn source_control_layouts_name_actual_nonadditive_owners_and_target_bounds() {
    let layouts = control_layouts().unwrap();
    assert_eq!(layouts.len(), 20);
    let mut names = std::collections::BTreeSet::new();
    for (name, bytes, alignment, phase) in layouts {
        assert!(names.insert(name));
        assert!(bytes <= MAX_TASK_PAYLOAD);
        assert!(alignment.is_power_of_two() && alignment <= 16);
        assert!(!phase.is_empty());
    }
    assert_eq!(layouts[0].1, std::mem::size_of::<Work>());
    assert_eq!(layouts[1].1, std::mem::size_of::<Pending>());
    assert_eq!(layouts[2].1, std::mem::size_of::<Result<Pending>>());
    assert_eq!(layouts[4].1, 3 * std::mem::size_of::<usize>());
    assert_eq!((layouts[5].1, layouts[5].2), (6, 2));
    let counts = std::alloc::Layout::array::<AtomicUsize>(2).unwrap();
    let (request, _) = counts.extend(std::alloc::Layout::new::<Request>()).unwrap();
    assert_eq!(layouts[8].1, request.pad_to_align().size());
    let (control, _) = counts
        .extend(std::alloc::Layout::new::<OperationControl>())
        .unwrap();
    assert_eq!(layouts[10].1, control.pad_to_align().size());
    assert_eq!(layouts[11].1, std::mem::size_of::<Arc<OperationControl>>());
    let entry: serde_json::Value =
        serde_json::from_slice(include_bytes!("../profile/entry-contract.json")).unwrap();
    assert_eq!(entry["task_payload"]["maximum_bytes"], MAX_TASK_PAYLOAD);
    assert_eq!(entry["task_payload"]["maximum_alignment"], 16);
}
#[test]
fn refused_unstarted_task_keeps_whole_work_owned_until_its_capture_is_dropped() {
    let fixture = Fixture::new(true, Role::OWNER, 1);
    let pools = crate::net::memory::pools(Default::default()).unwrap();
    let owner = fixture.owner(&pools);
    let path = std::path::PathBuf::from(owner.connection.path().unwrap());
    let peer = fixture.peer(&pools);
    let memory = pools.admitted_request(None).unwrap();
    let weak = Arc::downgrade(&memory);
    let control = OperationControl::test(Duration::from_secs(60), memory);
    let execution = Arc::new(tokio::sync::Semaphore::new(1));
    let work = Work {
        owner,
        call: list(PARENT, foks_rpc::KvListCursor::None),
        peer,
        control: control.clone(),
        execution: execution.clone().try_acquire_owned().unwrap(),
    };
    let padding = [0u8; MAX_TASK_PAYLOAD + 1];
    let task = move || {
        std::hint::black_box(padding);
        execute(work, &FixedClock(NOW))
    };
    assert_eq!(admit_task(&control, &task), Err(Error::Bounds));
    assert!(control.memory().failed());
    drop(control);
    assert!(weak.upgrade().is_some());
    assert_eq!(execution.available_permits(), 0);
    assert!(crate::DatabaseWriterGuard::acquire(&path).is_err());
    // There is no spawn and no new SQL interval after refusal. The capture's
    // one aggregate still owns every resource, then destroys it in field order.
    drop(task);
    assert!(weak.upgrade().is_none());
    assert_eq!(execution.available_permits(), 1);
    assert!(!path.exists());
}
