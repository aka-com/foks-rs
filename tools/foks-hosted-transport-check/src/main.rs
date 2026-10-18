//! External public-API/feature consumer. This is not a hosted runtime.
#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use foks_hosted_transport::{
        deadline::{self, Deadline},
        framing,
        memory::{self, PoolLimits, Pools, Request, Workload},
    };
    use std::{sync::Arc, time::Duration};
    fn owner_contract<T: foks_metadata_core::owned_list::ListRetainedOwner + Send + Sync>() {}
    owner_contract::<Request>();
    let pools = Pools::new(PoolLimits::new(1024 * 1024, 32 * 1024 * 1024, 16384, 2).unwrap());
    let request: Arc<Request> = pools.request(None);
    request.classify_input(0);
    assert!(request.admit_working(0, Workload::Ordinary));
    assert!(!request.failed());
    request.sync(|| assert!(Arc::ptr_eq(&memory::current().unwrap(), &request)));
    assert!(memory::current().is_none());
    let deadline = Deadline::after(Duration::from_secs(15)).unwrap();
    deadline::scope(Some(deadline), || deadline::check().unwrap());
    let mut output = Vec::new();
    drop(framing::write_response_until(
        &mut output,
        b"frame",
        deadline,
        Duration::from_secs(15),
    ));
    let mut input = &b"\x01x"[..];
    drop(framing::read_frame_ingress(
        &mut input,
        1024,
        Duration::from_secs(15),
    ));
    #[cfg(feature = "qualification")]
    {
        let owner: foks_metadata_core::Result<Arc<Request>> = pools.admitted_request(None);
        assert!(owner.is_ok());
    }
    #[cfg(feature = "test-support")]
    {
        use memory::test_support::{snapshot, try_hold, PressurePool};
        let before = snapshot(&pools);
        let held = try_hold(&pools, PressurePool::Output, 1).unwrap();
        assert_eq!(snapshot(&pools).output, before.output - 1);
        drop(held);
        assert_eq!(snapshot(&pools), before);
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {
    assert_eq!(foks_metadata_core::MAX_INPUT_BYTES, 2 * 1024 * 1024);
    assert_eq!(foks_metadata_core::Error::Bounds.code(), "bounds");
}
