//! Physical worker admission survives caller timeout and cancellation.
use super::AgentError;
use foks_local_deadline::Deadline;
use std::sync::{Arc, OnceLock};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use zeroize::Zeroize;

pub(crate) fn clock_error(error: foks_local_deadline::Error) -> AgentError {
    AgentError::from_desktop(foks_desktop::local_deadline_error(error))
}

fn workers() -> &'static Arc<Semaphore> {
    static WORKERS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    WORKERS.get_or_init(|| Arc::new(Semaphore::new(32)))
}

struct Outcome<T> {
    result: Option<Result<T, AgentError>>,
    erase: fn(&mut T),
    _permit: OwnedSemaphorePermit,
}
impl<T> Drop for Outcome<T> {
    fn drop(&mut self) {
        match &mut self.result {
            Some(Ok(value)) => (self.erase)(value),
            Some(Err(error)) => {
                erase_error_text(error);
            }
            None => {}
        }
    }
}

fn erase_error_text(error: &mut AgentError) {
    error.message.zeroize();
    if let Some(details) = &mut error.details {
        details.capability.zeroize();
        details.profile.zeroize();
        details.state_dir.zeroize();
        details.reason.zeroize();
    }
}

impl<T> Outcome<T> {
    fn finish(
        mut self,
        validity: Result<(), foks_local_deadline::Error>,
        mutation: bool,
    ) -> Result<T, AgentError> {
        if let Err(expiry) = validity {
            if let Some(Err(error)) = &mut self.result {
                // Preserve an already observed terminal failure, but erase its
                // late diagnostic text. Connection loss alone is retryable for
                // reads and must not be confused with a security failure.
                let terminal = (error.fatal && error.code != "agent-lost")
                    || matches!(
                        error.code.as_str(),
                        "deadline-clock"
                            | "deadline-clock-unavailable"
                            | "deadline-domain-mismatch"
                    );
                if terminal {
                    erase_error_text(error);
                    error.message =
                        "The operation failed a security, compatibility, or clock check.".into();
                    return self.result.take().expect("owned native error");
                }
            }
            let mut error = clock_error(expiry);
            error.ambiguous = mutation;
            error.retryable &= !mutation;
            return Err(error);
        }
        self.result.take().expect("owned native result")
    }
}

pub(crate) async fn run<T: Send + 'static>(
    deadline: &Deadline,
    mutation: bool,
    work: impl FnOnce() -> Result<T, AgentError> + Send + 'static,
    erase: fn(&mut T),
) -> Result<T, AgentError> {
    run_with_admission(deadline, mutation, work, erase, workers().clone()).await
}

async fn run_with_admission<T: Send + 'static>(
    deadline: &Deadline,
    mutation: bool,
    work: impl FnOnce() -> Result<T, AgentError> + Send + 'static,
    erase: fn(&mut T),
    admission: Arc<Semaphore>,
) -> Result<T, AgentError> {
    deadline.check().map_err(clock_error)?;
    let permit = tokio::time::timeout_at(deadline.instant().into(), admission.acquire_owned())
        .await
        .map_err(|_| clock_error(foks_local_deadline::Error::Expired))?
        .map_err(|_| {
            AgentError::new(
                "worker-unavailable",
                "Native worker admission is closed.",
                false,
            )
        })?;
    deadline.check().map_err(clock_error)?;
    let worker_deadline = deadline.clone();
    let worker = tauri::async_runtime::spawn_blocking(move || {
        let result = worker_deadline
            .check()
            .map_err(clock_error)
            .and_then(|()| work());
        Outcome {
            result: Some(result),
            erase,
            _permit: permit,
        }
    });
    let outcome = match tokio::time::timeout_at(deadline.instant().into(), worker).await {
        Ok(result) => result.map_err(|_| {
            let mut error = AgentError::new(
                "worker-failed",
                "The native worker stopped before reporting its outcome.",
                false,
            );
            error.ambiguous = mutation;
            error
        })?,
        Err(_) => {
            // A detached task owns both permit and erasing result guard. Dropping
            // this request never frees admission before that task has finished.
            let mut error = clock_error(foks_local_deadline::Error::Expired);
            error.ambiguous = mutation;
            error.retryable &= !mutation;
            return Err(error);
        }
    };
    outcome.finish(deadline.check(), mutation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{mpsc, Mutex},
        time::{Duration, Instant},
    };

    struct Secret {
        text: String,
        admission: Arc<Semaphore>,
        erased: Option<tokio::sync::oneshot::Sender<()>>,
    }
    fn erase(secret: &mut Secret) {
        assert_eq!(secret.admission.available_permits(), 0);
        secret.text.zeroize();
        secret.erased.take().unwrap().send(()).unwrap();
    }

    #[tokio::test]
    async fn timeout_keeps_physical_admission_until_late_plaintext_is_erased() {
        let admission = Arc::new(Semaphore::new(1));
        let deadline = Deadline::capture(Instant::now() + Duration::from_millis(100)).unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (erased_tx, erased_rx) = tokio::sync::oneshot::channel();
        let physical = admission.clone();
        let task_admission = admission.clone();
        let request = tokio::spawn(async move {
            run_with_admission(
                &deadline,
                false,
                move || {
                    started_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                    Ok(Secret {
                        text: "late secret".into(),
                        admission: physical,
                        erased: Some(erased_tx),
                    })
                },
                erase,
                task_admission,
            )
            .await
            .map(|_| ())
        });
        started_rx.await.unwrap();
        assert_eq!(
            request.await.unwrap().unwrap_err().code,
            "deadline-exceeded"
        );
        assert_eq!(admission.available_permits(), 0);
        release_tx.send(()).unwrap();
        erased_rx.await.unwrap();
        let permit = admission.acquire().await.unwrap();
        drop(permit);
        assert_eq!(admission.available_permits(), 1);
    }

    #[tokio::test]
    async fn cancelled_receiver_still_erases_result_before_releasing_slot() {
        let admission = Arc::new(Semaphore::new(1));
        let deadline = Deadline::capture(Instant::now() + Duration::from_secs(10)).unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (erased_tx, erased_rx) = tokio::sync::oneshot::channel();
        let physical = admission.clone();
        let task_admission = admission.clone();
        let request = tokio::spawn(async move {
            run_with_admission(
                &deadline,
                false,
                move || {
                    started_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                    Ok(Secret {
                        text: "cancelled secret".into(),
                        admission: physical,
                        erased: Some(erased_tx),
                    })
                },
                erase,
                task_admission,
            )
            .await
            .map(|_| ())
        });
        started_rx.await.unwrap();
        request.abort();
        assert!(request.await.unwrap_err().is_cancelled());
        assert_eq!(admission.available_permits(), 0);
        release_tx.send(()).unwrap();
        erased_rx.await.unwrap();
        drop(admission.acquire().await.unwrap());
    }

    #[tokio::test]
    async fn expired_queue_never_enters_worker() {
        let admission = Arc::new(Semaphore::new(1));
        let occupied = admission.clone().acquire_owned().await.unwrap();
        let deadline = Deadline::capture(Instant::now() + Duration::from_millis(20)).unwrap();
        let called = Arc::new(Mutex::new(false));
        let worker_called = called.clone();
        let result = run_with_admission(
            &deadline,
            true,
            move || {
                *worker_called.lock().unwrap() = true;
                Ok(())
            },
            |_| {},
            admission,
        )
        .await;
        let error = result.unwrap_err();
        assert_eq!(error.code, "deadline-exceeded");
        assert!(!error.ambiguous);
        assert!(!*called.lock().unwrap());
        drop(occupied);
    }
    #[tokio::test]
    async fn completed_terminal_errors_survive_expiry_without_late_diagnostics() {
        for code in [
            "unsafe-socket",
            "protocol",
            "response-binding",
            "version-mismatch",
            "deadline-clock",
        ] {
            let admission = Arc::new(Semaphore::new(1));
            let mut error = AgentError::new(code, "late diagnostic secret", false);
            error.fatal = code != "deadline-clock";
            error.ambiguous = true;
            error.details = Some(Box::new(super::super::AgentErrorDetails {
                capability: Some("secret capability".into()),
                profile: Some("secret profile".into()),
                state_dir: Some("secret path".into()),
                reason: Some("secret reason".into()),
                found_schema: None,
                supported_schema: None,
            }));
            let outcome = Outcome::<()> {
                result: Some(Err(error)),
                erase: |_| {},
                _permit: admission.clone().acquire_owned().await.unwrap(),
            };
            let error = outcome
                .finish(Err(foks_local_deadline::Error::Expired), false)
                .unwrap_err();
            assert_eq!(error.code, code);
            assert!(error.ambiguous);
            assert!(!error.retryable);
            assert!(!error.message.contains("secret"));
            let details = error.details.unwrap();
            for value in [
                details.capability,
                details.profile,
                details.state_dir,
                details.reason,
            ] {
                assert!(value.is_none_or(|value| value.is_empty()));
            }
            assert_eq!(admission.available_permits(), 1);
        }
    }

    #[tokio::test]
    async fn completed_connection_loss_does_not_override_expiry() {
        let admission = Arc::new(Semaphore::new(1));
        let mut error = AgentError::new("agent-lost", "late diagnostic secret", true);
        error.fatal = true;
        let outcome = Outcome::<()> {
            result: Some(Err(error)),
            erase: |_| {},
            _permit: admission.clone().acquire_owned().await.unwrap(),
        };
        let error = outcome
            .finish(Err(foks_local_deadline::Error::Expired), true)
            .unwrap_err();
        assert_eq!(error.code, "deadline-exceeded");
        assert!(error.ambiguous);
        assert!(!error.retryable);
        assert_eq!(admission.available_permits(), 1);
    }
}
