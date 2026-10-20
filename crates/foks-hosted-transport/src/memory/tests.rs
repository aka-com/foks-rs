use super::*;
use tokio::io::AsyncReadExt;

fn pools() -> Arc<Pools> {
    Arc::new(Pools {
        output: Arc::new(Semaphore::new(12 * 1024)),
        working: Arc::new(Semaphore::new(128 * 1024)),
        control: Arc::new(Semaphore::new(16 * 1024)),
        small: Arc::new(Semaphore::new(0)),
        bulk: Arc::new(Semaphore::new(4)),
    })
}

fn balanced_pools(count: usize) -> Arc<Pools> {
    Arc::new(Pools {
        output: Arc::new(Semaphore::new(count)),
        working: Arc::new(Semaphore::new(count)),
        control: Arc::new(Semaphore::new(count)),
        small: Arc::new(Semaphore::new(count)),
        bulk: Arc::new(Semaphore::new(count)),
    })
}

const ALL_POOLS: [PoolId; 5] = [
    PoolId::Working,
    PoolId::Small,
    PoolId::Output,
    PoolId::Control,
    PoolId::Bulk,
];

#[test]
fn repeated_charges_merge_into_five_fixed_slots_and_keep_input_separate() {
    let pools = balanced_pools(100);
    let input_pool = Arc::new(Semaphore::new(3));
    let input = input_pool.clone().try_acquire_many_owned(3).unwrap();
    let request = pools.request(Some(input));
    for pool in ALL_POOLS {
        assert_eq!(request.reserve_from(pool, 0), Ok(()));
        for _ in 0..20 {
            assert_eq!(request.reserve_from(pool, 2), Ok(()));
        }
        assert_eq!(pool.semaphore(&pools).available_permits(), 60);
    }
    let slots = request.reservations.lock().unwrap();
    for slot in [
        &slots.working,
        &slots.small,
        &slots.output,
        &slots.control,
        &slots.bulk,
    ] {
        assert_eq!(slot.as_ref().unwrap().num_permits(), 40);
    }
    assert_eq!(
        std::mem::size_of::<PermitSlots>(),
        5 * std::mem::size_of::<Option<OwnedSemaphorePermit>>()
    );
    // Exact preceding field layout, for target-local size qualification only.
    #[allow(dead_code)]
    struct PriorRequest {
        pools: Arc<Pools>,
        _input: Option<OwnedSemaphorePermit>,
        reservations: Mutex<Vec<OwnedSemaphorePermit>>,
        failed: AtomicBool,
        small_request: AtomicBool,
    }
    // The fixed owner may be larger; the removed Vec capacity was unbounded.
    eprintln!(
        "Request bytes: prior={}, fixed={}, fixed slots={}, old Vec header={}",
        std::mem::size_of::<PriorRequest>(),
        std::mem::size_of::<Request>(),
        std::mem::size_of::<PermitSlots>(),
        std::mem::size_of::<Vec<OwnedSemaphorePermit>>()
    );
    drop(slots);
    assert_eq!(input_pool.available_permits(), 0);
    drop(request);
    assert_eq!(input_pool.available_permits(), 3);
    for pool in ALL_POOLS {
        assert_eq!(pool.semaphore(&pools).available_permits(), 100);
    }
}

#[test]
fn capacity_fallback_is_eligible_only_and_control_never_uses_small() {
    let pools = balanced_pools(10);
    let request = pools.request(None);
    assert!(request.reserve_from(PoolId::Working, 10).is_ok());
    assert!(request.reserve_from(PoolId::Output, 10).is_ok());
    assert!(request.reserve_from(PoolId::Control, 10).is_ok());
    assert!(request.reserve_working(2));
    assert!(foks_rpc::ResponseAllocationBudget::reserve(
        &*request, 3, false
    ));
    assert_eq!(pools.small.available_permits(), 5);
    assert!(!foks_rpc::ResponseAllocationBudget::reserve(
        &*request, 1, true
    ));
    assert_eq!(pools.small.available_permits(), 5);
    request.classify_input(4097);
    assert!(!request.reserve_working(1));
    assert!(!foks_rpc::ResponseAllocationBudget::reserve(
        &*request, 1, false
    ));
    assert_eq!(pools.small.available_permits(), 5);
}

#[test]
fn wrong_pool_and_poison_release_only_new_attempt_without_fallback() {
    let pools = balanced_pools(10);
    let request = pools.request(None);
    assert!(request.reserve_from(PoolId::Working, 3).is_ok());
    let wrong = pools.output.clone().try_acquire_many_owned(2).unwrap();
    assert_eq!(
        request.retain_permit(PoolId::Working, wrong),
        Err(ReserveFailure::Invariant)
    );
    assert!(request.failed());
    assert_eq!(pools.output.available_permits(), 10);
    assert_eq!(pools.working.available_permits(), 7);
    let poisoned = request.clone();
    assert!(std::thread::spawn(move || {
        let _guard = poisoned.reservations.lock().unwrap();
        panic!("fixture poisons bookkeeping");
    })
    .join()
    .is_err());
    assert!(!request.reserve_working(2));
    assert_eq!(pools.working.available_permits(), 7);
    assert_eq!(pools.small.available_permits(), 10);
    drop(request);
    assert_eq!(pools.working.available_permits(), 10);
}

#[test]
fn checked_merge_count_never_wraps() {
    assert_eq!(merged_count(u32::MAX as usize, 0), Ok(u32::MAX));
    assert_eq!(
        merged_count(u32::MAX as usize, 1),
        Err(ReserveFailure::Invariant)
    );
    assert_eq!(merged_count(usize::MAX, 1), Err(ReserveFailure::Invariant));
}

#[cfg(target_pointer_width = "64")]
#[test]
fn overflow_drops_new_permit_without_mutating_held_slot() {
    let semaphore = Arc::new(Semaphore::new(u32::MAX as usize + 1));
    let mut slots = PermitSlots::default();
    let held = semaphore.clone().try_acquire_many_owned(u32::MAX).unwrap();
    slots.insert(PoolId::Working, &semaphore, held).unwrap();
    let added = semaphore.clone().try_acquire_owned().unwrap();
    assert_eq!(
        slots.insert(PoolId::Working, &semaphore, added),
        Err(ReserveFailure::Invariant)
    );
    assert_eq!(
        slots.working.as_ref().unwrap().num_permits(),
        u32::MAX as usize
    );
    assert_eq!(semaphore.available_permits(), 1);
    drop(slots);
    assert_eq!(semaphore.available_permits(), u32::MAX as usize + 1);
}

#[test]
fn working_failure_keeps_previously_acquired_bulk_until_drop() {
    let pools = balanced_pools(10);
    let request = pools.request(None);
    assert!(!request.admit_working(8192, Workload::Ordinary));
    assert_eq!(pools.bulk.available_permits(), 9);
    assert_eq!(pools.working.available_permits(), 10);
    assert_eq!(pools.small.available_permits(), 10);
    drop(request);
    assert_eq!(pools.bulk.available_permits(), 10);
}

#[test]
fn detached_physical_output_retains_all_five_slots() {
    let pools = balanced_pools(10);
    let request = pools.request(None);
    for pool in ALL_POOLS {
        assert!(request.reserve_from(pool, 2).is_ok());
    }
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (output_tx, output_rx) = std::sync::mpsc::channel();
    let work = request.sync(|| {
        retain_work(move || {
            entered_tx.send(()).unwrap();
            release_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            7u8
        })
    });
    let worker = std::thread::spawn(move || assert!(output_tx.send(work.run()).is_ok()));
    entered_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    drop(worker);
    drop(request);
    for pool in ALL_POOLS {
        assert_eq!(pool.semaphore(&pools).available_permits(), 8);
    }
    release_tx.send(()).unwrap();
    let output = output_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert_eq!(output.value, 7);
    for pool in ALL_POOLS {
        assert_eq!(pool.semaphore(&pools).available_permits(), 8);
    }
    drop(output);
    for pool in ALL_POOLS {
        assert_eq!(pool.semaphore(&pools).available_permits(), 10);
    }
}

#[test]
fn unstarted_work_and_future_drop_payloads_before_allowances() {
    struct Check(Arc<Pools>);
    impl Drop for Check {
        fn drop(&mut self) {
            assert!(self.0.working.available_permits() < 128 * 1024);
        }
    }
    let pools = pools();
    let request = pools.request(None);
    assert!(request.admit_working(512, Workload::Ordinary));
    let check = Check(pools.clone());
    let work = request.sync(|| retain_work(move || drop(check)));
    drop(request);
    drop(work);
    assert_eq!(pools.working.available_permits(), 128 * 1024);
    let request = pools.request(None);
    assert!(request.admit_working(512, Workload::Ordinary));
    let check = Check(pools.clone());
    let future = request.sync(|| {
        retain_future(async move {
            let _payload = check;
            std::future::pending::<()>().await
        })
    });
    drop(request);
    drop(future);
    assert_eq!(pools.working.available_permits(), 128 * 1024);
    let request = pools.request(None);
    assert!(request.admit_working(512, Workload::Ordinary));
    let check = Check(pools.clone());
    let result = request.sync(|| retain_work(move || check)).run();
    drop(request);
    drop(result);
    assert_eq!(pools.working.available_permits(), 128 * 1024);
}

#[test]
fn bulk_saturation_preserves_small_request_headroom_and_failure_is_sticky() {
    let pools =
        Pools::new(PoolLimits::new(128 * 1024 * 1024, 512 * 1024 * 1024, 1024 * 1024, 64).unwrap());
    let bulk = pools.request(None);
    assert!(bulk.admit_working(8192, Workload::Ordinary));
    let working_pressure = pools
        .working
        .clone()
        .try_acquire_many_owned(pools.working.available_permits() as u32)
        .unwrap();
    assert!(bulk
        .reserve_from(PoolId::Output, pools.output.available_permits())
        .is_ok());
    let small = pools.request(None);
    assert!(small.admit_working(100, Workload::Ordinary));
    assert!(small
        .sync(|| foks_rpc::encode_success_response_at(&[0xc0], 1))
        .is_ok());
    assert!(!foks_rpc::ResponseAllocationBudget::reserve(
        &*bulk, 1, false
    ));
    assert!(bulk.failed());
    // A status can be encoded once unrelated working pressure subsides;
    // its success must not erase the prior ambiguous-output failure.
    drop(working_pressure);
    assert!(bulk
        .sync(|| foks_rpc::encode_status_response_at(&foks_rpc::RpcStatus::Unsupported, 1))
        .is_ok());
    assert!(
        bulk.failed(),
        "a fallback status must not hide an ambiguous allocation failure"
    );
}

#[tokio::test]
async fn slow_socket_retains_response_capacity_and_control_errors_still_fit() {
    let pools = pools();
    let request = pools.request(None);
    let data = foks_snowpack::encode(&foks_snowpack::Value::Binary(vec![7; 4096])).unwrap();
    let response = request
        .sync(|| foks_rpc::encode_success_response_at(&data, 9))
        .unwrap();
    let expected = response.clone();
    let (mut socket, mut receiver) = tokio::io::duplex(1);
    let write = tokio::spawn(async move {
        let _request = request;
        crate::framing::write_response_until(
            &mut socket,
            &response,
            crate::deadline::Deadline::after(std::time::Duration::from_secs(5)).unwrap(),
            std::time::Duration::from_secs(5),
        )
        .await
        .unwrap();
        drop(response);
    });
    tokio::task::yield_now().await;
    assert!(!write.is_finished());
    let second = pools.request(None);
    assert!(second
        .sync(|| foks_rpc::encode_success_response_at(&data, 10))
        .is_err());
    assert!(second
        .sync(|| foks_rpc::encode_status_response_at(&foks_rpc::RpcStatus::RateLimited, 10))
        .is_ok());
    let mut received = Vec::new();
    receiver.read_to_end(&mut received).await.unwrap();
    write.await.unwrap();
    assert_eq!(received, expected);
    assert_eq!(pools.output.available_permits(), 12 * 1024);
    assert!(second
        .sync(|| foks_rpc::encode_success_response_at(&data, 10))
        .is_ok());
}

#[tokio::test]
async fn cancelling_awaiter_does_not_release_blocking_work_or_its_output() {
    let pools = pools();
    let (started, ready) = std::sync::mpsc::channel();
    let (release, gate) = std::sync::mpsc::channel();
    let (done, finished) = tokio::sync::oneshot::channel();
    let request = pools.request(None);
    assert!(request.admit_working(512, Workload::Ordinary));
    let data = foks_snowpack::encode(&foks_snowpack::Value::Binary(vec![7; 4096])).unwrap();
    let task = tokio::task::spawn_blocking(move || {
        request.sync(|| {
            let bytes = foks_rpc::encode_success_response_at(&data, 1).unwrap();
            started.send(()).unwrap();
            gate.recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            drop(bytes);
        });
        drop(request);
        let _ = done.send(());
    });
    ready
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    drop(task);
    assert!(pools.working.available_permits() < 128 * 1024);
    assert!(pools.output.available_permits() < 12 * 1024);
    release.send(()).unwrap();
    finished.await.unwrap();
    assert_eq!(pools.working.available_permits(), 128 * 1024);
    assert_eq!(pools.output.available_permits(), 12 * 1024);
}

#[tokio::test(flavor = "current_thread")]
async fn async_budget_scope_is_restored_between_polls() {
    let a = pools();
    let b = pools();
    let request_a = a.request(None);
    let request_b = b.request(None);
    let ((), ()) = tokio::join!(
        request_a.run(async {
            tokio::task::yield_now().await;
            foks_rpc::encode_void_success_response_at(1).unwrap();
        }),
        request_b.run(async {
            tokio::task::yield_now().await;
            foks_rpc::encode_void_success_response_at(2).unwrap();
        }),
    );
    assert!(current().is_none());
    assert_eq!(a.control.available_permits(), 16 * 1024 - 256);
    assert_eq!(b.control.available_permits(), 16 * 1024 - 256);
}

#[test]
fn validated_geometry_rejects_underflow_and_unreservable_capacities() {
    let maximum = maximum_reservable();
    for (response, working, control, execution) in [
        (0, SMALL_HEADROOM, 1, 2),
        (1, SMALL_HEADROOM - 1, 1, 2),
        (1, SMALL_HEADROOM, 0, 2),
        (1, SMALL_HEADROOM, 1, 1),
        (maximum + 1, SMALL_HEADROOM, 1, 2),
        (1, maximum + 1, 1, 2),
        (1, SMALL_HEADROOM, maximum + 1, 2),
        (1, SMALL_HEADROOM, 1, Semaphore::MAX_PERMITS + 1),
    ] {
        assert!(matches!(
            PoolLimits::new(response, working, control, execution),
            Err(crate::Error::Config("invalid memory pool limits"))
        ));
    }
    for (execution, bulk) in [(2, 1), (3, 2), (5, 4), (64, 4)] {
        let pools = Pools::new(PoolLimits::new(1, SMALL_HEADROOM, 1, execution).unwrap());
        assert_eq!(pools.working.available_permits(), 0);
        assert_eq!(pools.small.available_permits(), SMALL_HEADROOM);
        assert_eq!(pools.bulk.available_permits(), bulk);
    }
}

#[test]
fn supplied_chunk_limit_preserves_charge_and_bulk_classification() {
    let pools = pools();
    let request = pools.request(None);
    assert!(request.admit_working(
        512,
        Workload::ChunkDownload {
            maximum_encoded_bytes: 1000
        }
    ));
    assert_eq!(
        pools.working.available_permits(),
        128 * 1024 - (512 * 2 + 1000 * 8 + 4096)
    );
    assert_eq!(pools.bulk.available_permits(), 3);
    assert!(!request.small_request.load(Ordering::Relaxed));
    drop(request);
    let request = pools.request(None);
    assert!(!request.admit_working(
        512,
        Workload::ChunkDownload {
            maximum_encoded_bytes: usize::MAX
        }
    ));
    assert_eq!(pools.working.available_permits(), 128 * 1024);
    assert_eq!(pools.bulk.available_permits(), 3);
    drop(request);
    assert_eq!(pools.bulk.available_permits(), 4);
}

#[test]
fn qualified_arc_is_charged_before_construction_and_failure_releases_input() {
    #[repr(C)]
    struct ArcLayout {
        strong: std::sync::atomic::AtomicUsize,
        weak: std::sync::atomic::AtomicUsize,
        request: Request,
    }
    let bytes = std::mem::size_of::<ArcLayout>();
    let pools = balanced_pools(bytes);
    let request = pools.admitted_request(None).unwrap();
    assert_eq!(pools.working.available_permits(), 0);
    let input_pool = Arc::new(Semaphore::new(1));
    let input = input_pool.clone().try_acquire_owned().unwrap();
    assert!(matches!(
        pools.admitted_request(Some(input)),
        Err(foks_metadata_core::Error::Bounds)
    ));
    assert_eq!(input_pool.available_permits(), 1);
    drop(request);
    assert_eq!(pools.working.available_permits(), bytes);
}

#[test]
fn public_test_pressure_is_bounded_opaque_and_releases_on_drop() {
    use test_support::{Capacities, PressurePool};
    let pools = test_support::pools(Capacities {
        working: 3,
        small: 0,
        output: 2,
        control: 0,
        bulk: 0,
    })
    .unwrap();
    let initial = test_support::snapshot(&pools);
    let working = test_support::try_hold(&pools, PressurePool::Working, 3).unwrap();
    let output = test_support::try_hold(&pools, PressurePool::Output, 2).unwrap();
    assert_eq!(test_support::snapshot(&pools).working, 0);
    assert_eq!(test_support::snapshot(&pools).output, 0);
    assert!(
        matches!(test_support::try_hold(&pools, PressurePool::Working, 1), Err(crate::Error::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    assert!(matches!(
        test_support::try_hold(&pools, PressurePool::Working, maximum_reservable() + 1),
        Err(crate::Error::Config(_))
    ));
    assert!(test_support::pools(Capacities {
        working: maximum_reservable() + 1,
        small: 0,
        output: 0,
        control: 0,
        bulk: 0
    })
    .is_err());
    pools.output.close();
    assert!(
        matches!(test_support::try_hold(&pools, PressurePool::Output, 0), Err(crate::Error::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    drop(working);
    drop(output);
    assert_eq!(test_support::snapshot(&pools), initial);
}
