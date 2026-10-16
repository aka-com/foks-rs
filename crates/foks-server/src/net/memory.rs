//! Framed response and transfer allocation ownership. These allowances exclude
//! SQLite page caches, TLS/kernel buffers
//! and allocator overhead; a node still needs a measured RSS limit/headroom.

use std::{
    future::Future,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(crate) const SMALL_HEADROOM: usize = 16 * 1024 * 1024;

thread_local! {
    static CURRENT: std::cell::RefCell<Option<Arc<Request>>> = const { std::cell::RefCell::new(None) };
}

pub(crate) fn current() -> Option<Arc<Request>> {
    CURRENT.with(|current| current.borrow().clone())
}

pub(crate) struct Pools {
    output: Arc<Semaphore>,
    working: Arc<Semaphore>,
    control: Arc<Semaphore>,
    small: Arc<Semaphore>,
    bulk: Arc<Semaphore>,
}

impl Pools {
    pub(crate) fn new(limits: crate::SessionLimits) -> Arc<Self> {
        Arc::new(Self {
            output: Arc::new(Semaphore::new(limits.maximum_response_memory_bytes)),
            working: Arc::new(Semaphore::new(
                limits.maximum_working_memory_bytes - SMALL_HEADROOM,
            )),
            small: Arc::new(Semaphore::new(SMALL_HEADROOM)),
            bulk: Arc::new(Semaphore::new(
                limits.maximum_in_flight_requests.saturating_sub(1).min(4),
            )),
            control: Arc::new(Semaphore::new(limits.maximum_control_memory_bytes)),
        })
    }

    pub(crate) fn request(self: &Arc<Self>, input: Option<OwnedSemaphorePermit>) -> Arc<Request> {
        Arc::new(Request {
            pools: self.clone(),
            _input: input,
            reservations: Mutex::new(PermitSlots::default()),
            failed: AtomicBool::new(false),
            small_request: AtomicBool::new(true),
        })
    }
}

// One slot per concrete pool: repeated charges retain permits without growing
// a bookkeeping allocation. Input ownership stays separate from these slots.
#[derive(Default)]
struct PermitSlots {
    working: Option<OwnedSemaphorePermit>,
    small: Option<OwnedSemaphorePermit>,
    output: Option<OwnedSemaphorePermit>,
    control: Option<OwnedSemaphorePermit>,
    bulk: Option<OwnedSemaphorePermit>,
}

#[derive(Clone, Copy)]
enum PoolId {
    Working,
    Small,
    Output,
    Control,
    Bulk,
}

impl PoolId {
    fn semaphore(self, pools: &Pools) -> &Arc<Semaphore> {
        match self {
            Self::Working => &pools.working,
            Self::Small => &pools.small,
            Self::Output => &pools.output,
            Self::Control => &pools.control,
            Self::Bulk => &pools.bulk,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ReserveFailure {
    Capacity,
    Invariant,
}

fn merged_count(held: usize, added: usize) -> Result<u32, ReserveFailure> {
    held.checked_add(added)
        .and_then(|count| u32::try_from(count).ok())
        .ok_or(ReserveFailure::Invariant)
}

impl PermitSlots {
    fn insert(
        &mut self,
        pool: PoolId,
        semaphore: &Arc<Semaphore>,
        permit: OwnedSemaphorePermit,
    ) -> Result<(), ReserveFailure> {
        let slot = match pool {
            PoolId::Working => &mut self.working,
            PoolId::Small => &mut self.small,
            PoolId::Output => &mut self.output,
            PoolId::Control => &mut self.control,
            PoolId::Bulk => &mut self.bulk,
        };
        if !Arc::ptr_eq(permit.semaphore(), semaphore) {
            return Err(ReserveFailure::Invariant);
        }
        if let Some(held) = slot {
            if !Arc::ptr_eq(held.semaphore(), semaphore) {
                return Err(ReserveFailure::Invariant);
            }
            // Tokio merge asserts pool identity and adds an internal u32.
            // Validate both before mutating either retained count.
            merged_count(held.num_permits(), permit.num_permits())?;
            held.merge(permit);
        } else {
            *slot = Some(permit);
        }
        Ok(())
    }
}

pub(crate) struct Request {
    pools: Arc<Pools>,
    _input: Option<OwnedSemaphorePermit>,
    reservations: Mutex<PermitSlots>,
    failed: AtomicBool,
    small_request: AtomicBool,
}

impl Request {
    pub(crate) fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }

    fn reserve_from(&self, pool: PoolId, bytes: usize) -> Result<(), ReserveFailure> {
        let count = u32::try_from(bytes).map_err(|_| ReserveFailure::Capacity)?;
        let semaphore = pool.semaphore(&self.pools);
        let permit = semaphore
            .clone()
            .try_acquire_many_owned(count)
            .map_err(|_| ReserveFailure::Capacity)?;
        self.retain_permit(pool, permit)
    }

    fn retain_permit(
        &self,
        pool: PoolId,
        permit: OwnedSemaphorePermit,
    ) -> Result<(), ReserveFailure> {
        let result = self
            .reservations
            .lock()
            .map_err(|_| ReserveFailure::Invariant)
            .and_then(|mut reservations| {
                reservations.insert(pool, pool.semaphore(&self.pools), permit)
            });
        if result.is_err() {
            self.failed.store(true, Ordering::Relaxed);
        }
        result
    }

    fn reserve_with_small_fallback(&self, pool: PoolId, bytes: usize, eligible: bool) -> bool {
        match self.reserve_from(pool, bytes) {
            Ok(()) => true,
            Err(ReserveFailure::Capacity) if eligible => {
                self.reserve_from(PoolId::Small, bytes).is_ok()
            }
            Err(_) => false,
        }
    }

    fn reserve_working(&self, bytes: usize) -> bool {
        self.reserve_with_small_fallback(
            PoolId::Working,
            bytes,
            self.small_request.load(Ordering::Relaxed),
        )
    }

    pub(crate) fn classify_input(&self, length: usize) {
        self.small_request.store(length <= 4096, Ordering::Relaxed);
    }

    pub(crate) fn admit_working(&self, input_bytes: usize, chunk_download: bool) -> bool {
        let small = input_bytes <= 4096 && !chunk_download;
        self.small_request.store(small, Ordering::Relaxed);
        // Bulk calls cannot occupy every execution slot, including while an
        // object store is stalled. The permit follows detached work and output.
        if !small && self.reserve_from(PoolId::Bulk, 1).is_err() {
            return false;
        }
        // Generic trees/results are admitted separately by codec/SQL hooks.
        // Reserve raw argument clones plus the object transfer's representations
        // before external I/O or inline payload materialization.
        let download = if chunk_download {
            crate::chunks::MAXIMUM_ENCODED_CHUNK_BYTES.saturating_mul(8)
        } else {
            0
        };
        self.reserve_working(
            input_bytes
                .saturating_mul(2)
                .saturating_add(download)
                .saturating_add(4096),
        )
    }

    pub(crate) fn sync<T>(self: &Arc<Self>, f: impl FnOnce() -> T) -> T {
        struct Restore(Option<Arc<Request>>);
        impl Drop for Restore {
            fn drop(&mut self) {
                CURRENT.with(|current| *current.borrow_mut() = self.0.take());
            }
        }
        let _restore = Restore(CURRENT.with(|current| current.replace(Some(self.clone()))));
        foks_snowpack::with_allocation_budget(self.clone(), || {
            foks_rpc::with_response_budget(self.clone(), f)
        })
    }

    pub(crate) async fn run<T>(self: &Arc<Self>, future: impl Future<Output = T>) -> T {
        let mut future = Box::pin(future);
        std::future::poll_fn(|cx| self.sync(|| future.as_mut().poll(cx))).await
    }
}

impl foks_rpc::ResponseAllocationBudget for Request {
    fn reserve(&self, bytes: usize, control: bool) -> bool {
        let accepted = self.reserve_with_small_fallback(
            if control {
                PoolId::Control
            } else {
                PoolId::Output
            },
            bytes,
            !control && self.small_request.load(Ordering::Relaxed),
        );
        if !accepted {
            self.failed.store(true, Ordering::Relaxed);
        }
        accepted
    }
}

impl foks_snowpack::AllocationBudget for Request {
    fn reserve(&self, bytes: usize) -> bool {
        let accepted = self.reserve_working(bytes);
        if !accepted {
            self.failed.store(true, Ordering::Relaxed);
        }
        accepted
    }
}

// Field order also matters if a task is cancelled before its first poll/run.
// Keep the whole owner in spawned closures instead of separately captured fields.
pub(crate) struct RetainedWork<F> {
    work: F,
    memory: Option<Arc<Request>>,
}
pub(crate) fn retain_work<F>(work: F) -> RetainedWork<F> {
    RetainedWork {
        work,
        memory: current(),
    }
}
pub(crate) struct RetainedValue<T> {
    value: T,
    _memory: Option<Arc<Request>>,
}
impl<T> RetainedValue<T> {
    // The awaiting request still owns its scope when taking the value. If its
    // waiter was cancelled, Tokio drops this whole owner in field order.
    pub(crate) fn into_inner(self) -> T {
        self.value
    }
}
impl<F> RetainedWork<F> {
    pub(crate) fn run<T>(self) -> RetainedValue<T>
    where
        F: FnOnce() -> T,
    {
        let Self { memory, work } = self;
        let value = match memory.as_ref() {
            Some(memory) => memory.sync(work),
            None => work(),
        };
        RetainedValue {
            value,
            _memory: memory,
        }
    }
}
struct RetainedFuture<F> {
    future: std::pin::Pin<Box<F>>,
    _memory: Option<Arc<Request>>,
}
impl<F: Future> Future for RetainedFuture<F> {
    type Output = F::Output;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.get_mut();
        match this._memory.as_ref() {
            Some(memory) => memory.sync(|| this.future.as_mut().poll(cx)),
            None => this.future.as_mut().poll(cx),
        }
    }
}
pub(crate) fn retain_future<F: Future<Output = ()>>(future: F) -> impl Future<Output = ()> {
    RetainedFuture {
        future: Box::pin(future),
        _memory: current(),
    }
}

#[cfg(test)]
mod tests {
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
        assert!(!request.admit_working(8192, false));
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
        assert!(request.admit_working(512, false));
        let check = Check(pools.clone());
        let work = request.sync(|| retain_work(move || drop(check)));
        drop(request);
        drop(work);
        assert_eq!(pools.working.available_permits(), 128 * 1024);
        let request = pools.request(None);
        assert!(request.admit_working(512, false));
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
        assert!(request.admit_working(512, false));
        let check = Check(pools.clone());
        let result = request.sync(|| retain_work(move || check)).run();
        drop(request);
        drop(result);
        assert_eq!(pools.working.available_permits(), 128 * 1024);
    }

    #[test]
    fn bulk_saturation_preserves_small_request_headroom_and_failure_is_sticky() {
        let pools = Pools::new(crate::SessionLimits::default());
        let bulk = pools.request(None);
        assert!(bulk.admit_working(8192, false));
        let working_pressure = pools
            .working
            .clone()
            .try_acquire_many_owned(pools.working.available_permits() as u32)
            .unwrap();
        assert!(bulk
            .reserve_from(PoolId::Output, pools.output.available_permits())
            .is_ok());
        let small = pools.request(None);
        assert!(small.admit_working(100, false));
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
            super::super::session::write_response(
                &mut socket,
                &response,
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
        assert!(request.admit_working(512, false));
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
}
