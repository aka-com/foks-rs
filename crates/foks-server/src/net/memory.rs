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
            reservations: Mutex::new(Vec::new()),
            failed: AtomicBool::new(false),
            small_request: AtomicBool::new(true),
        })
    }
}

pub(crate) struct Request {
    pools: Arc<Pools>,
    _input: Option<OwnedSemaphorePermit>,
    reservations: Mutex<Vec<OwnedSemaphorePermit>>,
    failed: AtomicBool,
    small_request: AtomicBool,
}

impl Request {
    pub(crate) fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }

    fn reserve_from(&self, pool: &Arc<Semaphore>, bytes: usize) -> bool {
        let Ok(count) = u32::try_from(bytes) else {
            return false;
        };
        let Ok(permit) = pool.clone().try_acquire_many_owned(count) else {
            return false;
        };
        let Ok(mut reservations) = self.reservations.lock() else {
            return false;
        };
        reservations.push(permit);
        true
    }

    fn reserve_working(&self, bytes: usize) -> bool {
        self.reserve_from(&self.pools.working, bytes)
            || (self.small_request.load(Ordering::Relaxed)
                && self.reserve_from(&self.pools.small, bytes))
    }

    pub(crate) fn classify_input(&self, length: usize) {
        self.small_request.store(length <= 4096, Ordering::Relaxed);
    }

    pub(crate) fn admit_working(&self, input_bytes: usize, chunk_download: bool) -> bool {
        let small = input_bytes <= 4096 && !chunk_download;
        self.small_request.store(small, Ordering::Relaxed);
        // Bulk calls cannot occupy every execution slot, including while an
        // object store is stalled. The permit follows detached work and output.
        if !small && !self.reserve_from(&self.pools.bulk, 1) {
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
        let accepted = self.reserve_from(
            if control {
                &self.pools.control
            } else {
                &self.pools.output
            },
            bytes,
        ) || (!control
            && self.small_request.load(Ordering::Relaxed)
            && self.reserve_from(&self.pools.small, bytes));
        if !accepted {
            self.failed.store(true, Ordering::Relaxed);
        }
        accepted
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
        assert!(bulk.reserve_from(&pools.output, pools.output.available_permits()));
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
