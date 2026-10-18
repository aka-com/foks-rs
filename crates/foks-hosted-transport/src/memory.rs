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

pub const SMALL_HEADROOM: usize = 16 * 1024 * 1024;

/// Validated semaphore geometry; the caller retains its stricter deployment policy.
#[derive(Clone, Copy, Debug)]
pub struct PoolLimits {
    response_bytes: usize,
    working_bytes: usize,
    control_bytes: usize,
    maximum_in_flight_requests: usize,
}

fn maximum_reservable() -> usize {
    usize::try_from(u32::MAX)
        .unwrap_or(usize::MAX)
        .min(Semaphore::MAX_PERMITS)
}

impl PoolLimits {
    pub fn new(
        response_bytes: usize,
        working_bytes: usize,
        control_bytes: usize,
        maximum_in_flight_requests: usize,
    ) -> crate::Result<Self> {
        let maximum = maximum_reservable();
        if response_bytes == 0
            || response_bytes > maximum
            || working_bytes < SMALL_HEADROOM
            || working_bytes > maximum
            || control_bytes == 0
            || control_bytes > maximum
            || !(2..=Semaphore::MAX_PERMITS).contains(&maximum_in_flight_requests)
        {
            return Err(crate::Error::Config("invalid memory pool limits"));
        }
        Ok(Self {
            response_bytes,
            working_bytes,
            control_bytes,
            maximum_in_flight_requests,
        })
    }
}

/// The server supplies its existing encoded-chunk bound; admission stays here.
#[derive(Clone, Copy, Debug)]
pub enum Workload {
    Ordinary,
    ChunkDownload { maximum_encoded_bytes: usize },
}

thread_local! {
    static CURRENT: std::cell::RefCell<Option<Arc<Request>>> = const { std::cell::RefCell::new(None) };
}

pub fn current() -> Option<Arc<Request>> {
    CURRENT.with(|current| current.borrow().clone())
}

pub struct Pools {
    output: Arc<Semaphore>,
    working: Arc<Semaphore>,
    control: Arc<Semaphore>,
    small: Arc<Semaphore>,
    bulk: Arc<Semaphore>,
}

impl Pools {
    pub fn new(limits: PoolLimits) -> Arc<Self> {
        Arc::new(Self {
            output: Arc::new(Semaphore::new(limits.response_bytes)),
            working: Arc::new(Semaphore::new(limits.working_bytes - SMALL_HEADROOM)),
            small: Arc::new(Semaphore::new(SMALL_HEADROOM)),
            bulk: Arc::new(Semaphore::new(
                limits.maximum_in_flight_requests.saturating_sub(1).min(4),
            )),
            control: Arc::new(Semaphore::new(limits.control_bytes)),
        })
    }

    /// Restricted qualification construction: reserve the real Arc allocation
    /// before creating it, then retain that permit in the same fixed slots.
    #[cfg(any(test, feature = "qualification"))]
    pub fn admitted_request(
        self: &Arc<Self>,
        input: Option<OwnedSemaphorePermit>,
    ) -> foks_metadata_core::Result<Arc<Request>> {
        #[repr(C)]
        struct Layout {
            strong: std::sync::atomic::AtomicUsize,
            weak: std::sync::atomic::AtomicUsize,
            value: Request,
        }
        let bytes = u32::try_from(std::mem::size_of::<Layout>())
            .map_err(|_| foks_metadata_core::Error::Bounds)?;
        let permit = self
            .working
            .clone()
            .try_acquire_many_owned(bytes)
            .map_err(|_| foks_metadata_core::Error::Bounds)?;
        let reservations = PermitSlots {
            working: Some(permit),
            ..PermitSlots::default()
        };
        Ok(Arc::new(Request {
            pools: self.clone(),
            _input: input,
            reservations: Mutex::new(reservations),
            failed: AtomicBool::new(false),
            small_request: AtomicBool::new(true),
        }))
    }

    pub fn request(self: &Arc<Self>, input: Option<OwnedSemaphorePermit>) -> Arc<Request> {
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

pub struct Request {
    pools: Arc<Pools>,
    _input: Option<OwnedSemaphorePermit>,
    reservations: Mutex<PermitSlots>,
    failed: AtomicBool,
    small_request: AtomicBool,
}

impl Request {
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }

    /// Reserve a qualification frame using the existing response budget.
    /// Small-input fallback keeps its ordinary semantics; guarded invocation
    /// owners classify themselves as non-small before using this method.
    #[cfg(any(test, feature = "qualification"))]
    pub fn reserve_output(&self, bytes: usize) -> bool {
        !self.failed()
            && foks_rpc::ResponseAllocationBudget::reserve(self, bytes, false)
            && !self.failed()
    }

    /// Reserve a qualification control reply independently of response-pool
    /// saturation. The existing control allocator never falls back to Small.
    #[cfg(any(test, feature = "qualification"))]
    pub fn reserve_control(&self, bytes: usize) -> bool {
        !self.failed()
            && foks_rpc::ResponseAllocationBudget::reserve(self, bytes, true)
            && !self.failed()
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

    pub fn classify_input(&self, length: usize) {
        self.small_request.store(length <= 4096, Ordering::Relaxed);
    }

    pub fn admit_working(&self, input_bytes: usize, workload: Workload) -> bool {
        let small = input_bytes <= 4096 && matches!(workload, Workload::Ordinary);
        self.small_request.store(small, Ordering::Relaxed);
        // Bulk calls cannot occupy every execution slot, including while an
        // object store is stalled. The permit follows detached work and output.
        if !small && self.reserve_from(PoolId::Bulk, 1).is_err() {
            return false;
        }
        // Generic trees/results are admitted separately by codec/SQL hooks.
        // Reserve raw argument clones plus the object transfer's representations
        // before external I/O or inline payload materialization.
        let download = match workload {
            Workload::Ordinary => 0,
            Workload::ChunkDownload {
                maximum_encoded_bytes,
            } => maximum_encoded_bytes.saturating_mul(8),
        };
        self.reserve_working(
            input_bytes
                .saturating_mul(2)
                .saturating_add(download)
                .saturating_add(4096),
        )
    }

    pub fn sync<T>(self: &Arc<Self>, f: impl FnOnce() -> T) -> T {
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

    pub async fn run<T>(self: &Arc<Self>, future: impl Future<Output = T>) -> T {
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

// Reuse the original request and fixed permit slots. Page-local credits never
// release these permits; retained bytes and physical work keep this Arc alive.
impl foks_metadata_core::owned_list::ListRetainedOwner for Request {
    fn reserve_retained(&self, bytes: usize) -> bool {
        !self.failed() && foks_snowpack::AllocationBudget::reserve(self, bytes) && !self.failed()
    }
    fn reserve_page_workspace(&self, bytes: usize) -> bool {
        !self.failed() && foks_snowpack::AllocationBudget::reserve(self, bytes) && !self.failed()
    }
    fn failed(&self) -> bool {
        Request::failed(self)
    }
    fn fail(&self) {
        self.failed.store(true, Ordering::Relaxed);
    }
}

// Field order also matters if a task is cancelled before its first poll/run.
// Keep the whole owner in spawned closures instead of separately captured fields.
pub struct RetainedWork<F> {
    work: F,
    memory: Option<Arc<Request>>,
}
pub fn retain_work<F>(work: F) -> RetainedWork<F> {
    RetainedWork {
        work,
        memory: current(),
    }
}
pub struct RetainedValue<T> {
    value: T,
    _memory: Option<Arc<Request>>,
}
impl<T> RetainedValue<T> {
    // The awaiting request still owns its scope when taking the value. If its
    // waiter was cancelled, Tokio drops this whole owner in field order.
    pub fn into_inner(self) -> T {
        self.value
    }
}
impl<F> RetainedWork<F> {
    pub fn run<T>(self) -> RetainedValue<T>
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
pub fn retain_future<F: Future<Output = ()>>(future: F) -> impl Future<Output = ()> {
    RetainedFuture {
        future: Box::pin(future),
        _memory: current(),
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
#[cfg(test)]
mod tests;
