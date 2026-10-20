//! One full local-clock budget, captured before admission and carried into a worker.
use foks_client_app::CancellationToken;
use foks_local_deadline::Deadline;
use std::{
    cell::RefCell,
    marker::PhantomData,
    rc::Rc,
    time::{Duration, Instant},
};

#[derive(Clone)]
pub(super) struct RequestControl {
    pub(super) deadline: Deadline,
    pub(super) cancellation: CancellationToken,
    inherited: Vec<CancellationToken>,
}
impl RequestControl {
    pub(super) fn receive(
        request: &foks_agent_proto::Request,
        ordinary_cap: Duration,
    ) -> Result<Self, foks_client::Error> {
        Ok(Self {
            deadline: Deadline::receive(
                &request.deadline,
                configured_cap(request.operation.deadline_class(), ordinary_cap),
            )?,
            cancellation: CancellationToken::new(),
            inherited: Vec::new(),
        })
    }
    pub(super) fn background(
        budget: Duration,
        cancellation: CancellationToken,
    ) -> Result<Self, foks_client::Error> {
        let until = Instant::now()
            .checked_add(budget)
            .ok_or(foks_local_deadline::Error::Range)?;
        Ok(Self {
            deadline: Deadline::capture(until)?,
            cancellation,
            inherited: Vec::new(),
        })
    }
    pub(super) fn child(&self) -> Self {
        let mut child = self.clone();
        child.inherited.push(child.cancellation.clone());
        child.cancellation = CancellationToken::new();
        child
    }
    pub(super) fn check(&self) -> Result<(), foks_client::Error> {
        if self.cancellation.is_cancelled()
            || self.inherited.iter().any(CancellationToken::is_cancelled)
        {
            return Err(foks_client::Error::Cancelled);
        }
        self.deadline.check()?;
        Ok(())
    }
    pub(super) fn remaining(&self) -> Result<Duration, foks_client::Error> {
        self.check()?;
        let remaining = self
            .deadline
            .instant()
            .saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(foks_client::Error::DeadlineExceeded);
        }
        Ok(remaining)
    }
    pub(super) fn enter(&self) -> Result<Scope, foks_client_app::Error> {
        self.check()?;
        let mut next = self.clone();
        if let Some(parent) = CURRENT.with(|slot| slot.borrow().clone()) {
            parent.check()?;
            if parent.inherited.len() >= 64 {
                return Err(foks_client_app::Error::InvalidConfig(
                    "request nesting limit",
                ));
            }
            next.deadline = next
                .deadline
                .intersect(&parent.deadline)
                .map_err(foks_client::Error::from)?;
            next.inherited.extend(parent.inherited);
            next.inherited.push(parent.cancellation);
        }
        let app = foks_client_app::ProfileOperationScope::new(
            next.deadline.clone(),
            next.cancellation.clone(),
        )?;
        Ok(Scope {
            previous: CURRENT.with(|slot| slot.replace(Some(next))),
            _app: app,
            _thread: PhantomData,
        })
    }
}
// The configured ordinary ceiling never reduces explicit poll/job policies.
fn configured_cap(class: foks_agent_proto::DeadlineClass, ordinary: Duration) -> Duration {
    use foks_agent_proto::DeadlineClass;
    match class {
        DeadlineClass::Ordinary => class.cap().min(ordinary),
        DeadlineClass::ChatPoll | DeadlineClass::Pairing | DeadlineClass::Bulk => class.cap(),
    }
}
thread_local! { static CURRENT: RefCell<Option<RequestControl>> = const { RefCell::new(None) }; }
pub(super) struct Scope {
    previous: Option<RequestControl>,
    _app: foks_client_app::ProfileOperationScope,
    _thread: PhantomData<Rc<()>>,
}
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|slot| *slot.borrow_mut() = self.previous.take());
    }
}
pub(super) fn current() -> Result<RequestControl, foks_client_app::Error> {
    let control =
        CURRENT
            .with(|slot| slot.borrow().clone())
            .ok_or(foks_client_app::Error::InvalidConfig(
                "request deadline context is required",
            ))?;
    control.check()?;
    Ok(control)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordinary_server_cap_shortens_without_reducing_named_job_classes() {
        use foks_agent_proto::{DeadlineClass, Operation, Request};
        let caller = Deadline::capture(Instant::now() + Duration::from_secs(15)).unwrap();
        let request = Request::new(1, Operation::Ping, caller.envelope().clone());
        let control = RequestControl::receive(&request, Duration::from_secs(1)).unwrap();
        assert!(control.remaining().unwrap() <= Duration::from_secs(1));
        assert!(control.deadline.envelope().expires_ns() <= caller.envelope().expires_ns());
        let capped = RequestControl::receive(&request, Duration::from_secs(60)).unwrap();
        assert!(capped.remaining().unwrap() <= Duration::from_secs(15));
        for class in [
            DeadlineClass::ChatPoll,
            DeadlineClass::Pairing,
            DeadlineClass::Bulk,
        ] {
            assert_eq!(configured_cap(class, Duration::from_secs(1)), class.cap());
        }
    }

    #[test]
    fn absent_control_refuses_profile_work_without_creating_state() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("never-created");
        assert!(current().is_err());
        let result = (|| -> Result<(), foks_client_app::Error> {
            current()?.check()?;
            let _ = foks_client_app::ProfileRegistry::open(&root)?;
            Ok(())
        })();
        assert!(result.is_err());
        assert!(!root.exists());
    }
    #[test]
    fn nested_scopes_intersect_restore_and_do_not_leak_after_unwind() {
        let parent =
            RequestControl::background(Duration::from_secs(10), CancellationToken::new()).unwrap();
        let child =
            RequestControl::background(Duration::from_secs(2), CancellationToken::new()).unwrap();
        {
            let _parent = parent.enter().unwrap();
            assert_eq!(
                current().unwrap().deadline.envelope(),
                parent.deadline.envelope()
            );
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _child = child.enter().unwrap();
                assert!(current().unwrap().deadline.instant() <= child.deadline.instant());
                panic!("intentional scope unwind");
            }));
            assert!(result.is_err());
            assert_eq!(
                current().unwrap().deadline.envelope(),
                parent.deadline.envelope()
            );
        }
        assert!(current().is_err());
        parent.cancellation.cancel();
        let replacement =
            RequestControl::background(Duration::from_secs(1), CancellationToken::new()).unwrap();
        let _replacement = replacement.enter().unwrap();
        assert!(current().unwrap().check().is_ok());
    }
    #[test]
    fn cancelling_an_attempt_does_not_cancel_the_retry_owner() {
        let parent =
            RequestControl::background(Duration::from_secs(1), CancellationToken::new()).unwrap();
        let first = parent.child();
        first.cancellation.cancel();
        assert!(first.check().is_err());
        assert!(parent.check().is_ok());
        let next = parent.child();
        assert_eq!(next.deadline.envelope(), parent.deadline.envelope());
        assert!(next.check().is_ok());
        parent.cancellation.cancel();
        assert!(next.check().is_err());
    }

    #[test]
    fn cancellation_of_parent_is_not_hidden_by_a_nested_token() {
        let parent =
            RequestControl::background(Duration::from_secs(10), CancellationToken::new()).unwrap();
        let child =
            RequestControl::background(Duration::from_secs(10), CancellationToken::new()).unwrap();
        let _parent = parent.enter().unwrap();
        let _child = child.enter().unwrap();
        parent.cancellation.cancel();
        assert!(current().is_err());
    }
    #[tokio::test]
    async fn queued_expired_request_never_acquires_profile_or_worker() {
        let dir = tempfile::tempdir().unwrap();
        let coordinator = Arc::new(crate::profile_work::Coordinator::default());
        let held = coordinator
            .acquire(
                dir.path(),
                crate::profile_work::Scope::profile("p"),
                Duration::from_secs(1),
            )
            .await
            .unwrap();
        let control =
            RequestControl::background(Duration::from_millis(10), CancellationToken::new())
                .unwrap();
        let error = coordinator
            .acquire_control(
                dir.path(),
                crate::profile_work::Scope::profile("p"),
                &control,
                Duration::from_secs(1),
            )
            .await
            .err()
            .unwrap();
        assert_eq!(error, crate::profile_work::AdmissionError::Deadline);
        drop(held);
        let next =
            RequestControl::background(Duration::from_secs(1), CancellationToken::new()).unwrap();
        assert!(coordinator
            .acquire_control(
                dir.path(),
                crate::profile_work::Scope::profile("p"),
                &next,
                Duration::from_secs(1)
            )
            .await
            .is_ok());
    }
    use std::sync::Arc;
}
