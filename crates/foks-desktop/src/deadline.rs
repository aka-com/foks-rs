//! One owned context across a command's several transport calls.
use crate::{AgentError, AgentTransport, IpcErrorCode};
use foks_agent_proto::{KvUploadHeader, Operation};
use foks_local_deadline::Deadline;
use serde_json::Value;
use std::sync::Arc;
use zeroize::Zeroize;

pub fn local_deadline_error(error: foks_local_deadline::Error) -> AgentError {
    if matches!(error, foks_local_deadline::Error::Expired) {
        AgentError::DeadlineExceeded
    } else {
        AgentError::Ipc {
            code: IpcErrorCode::DeadlineClock,
            message: error.to_string(),
            ambiguous: false,
            connection_lost: false,
        }
    }
}
pub(super) fn unsupported() -> AgentError {
    AgentError::Ipc {
        code: IpcErrorCode::Unsupported,
        message: "This agent transport cannot preserve an operation deadline.".into(),
        ambiguous: false,
        connection_lost: false,
    }
}

pub fn bind_transport_deadline(
    inner: Arc<dyn AgentTransport>,
    deadline: Deadline,
) -> Result<Arc<dyn AgentTransport>, AgentError> {
    deadline.check().map_err(local_deadline_error)?;
    Ok(Arc::new(BoundTransport { inner, deadline }))
}
struct BoundTransport {
    inner: Arc<dyn AgentTransport>,
    deadline: Deadline,
}
impl BoundTransport {
    fn call_bound(
        &self,
        mut operation: Operation,
        deadline: &Deadline,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Value, AgentError> {
        if let Err(error) = deadline.check().map_err(local_deadline_error) {
            operation.zeroize_plaintext();
            return Err(error);
        }
        if cancelled() {
            operation.zeroize_plaintext();
            return Err(AgentError::Cancelled);
        }
        let mutation = operation.is_mutation();
        let mut result = self
            .inner
            .call_with_deadline(operation, deadline, cancelled);
        if let Err(error) = deadline.check().map_err(local_deadline_error) {
            if preserve_terminal_transport_error(&mut result) {
                return result;
            }
            discard_transport_result(&mut result);
            return Err(if mutation {
                AgentError::Ambiguous(error.to_string())
            } else {
                error
            });
        }
        if cancelled() {
            if preserve_terminal_transport_error(&mut result) {
                return result;
            }
            discard_transport_result(&mut result);
            return Err(if mutation {
                AgentError::Ambiguous("Request cancelled after dispatch.".into())
            } else {
                AgentError::Cancelled
            });
        }
        result
    }
    fn stream_bound(
        &self,
        header: KvUploadHeader,
        reader: &mut dyn std::io::Read,
        deadline: &Deadline,
    ) -> Result<Value, AgentError> {
        deadline.check().map_err(local_deadline_error)?;
        let mut result = self
            .inner
            .put_kv_stream_with_deadline(header, reader, deadline);
        if let Err(error) = deadline.check() {
            if preserve_terminal_transport_error(&mut result) {
                return result;
            }
            discard_transport_result(&mut result);
            return Err(AgentError::Ambiguous(error.to_string()));
        }
        result
    }
}
impl AgentTransport for BoundTransport {
    fn call(&self, operation: Operation) -> Result<Value, AgentError> {
        self.call_bound(operation, &self.deadline, &|| false)
    }
    fn call_cancellable(
        &self,
        operation: Operation,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Value, AgentError> {
        self.call_bound(operation, &self.deadline, cancelled)
    }
    fn call_with_deadline(
        &self,
        operation: Operation,
        deadline: &Deadline,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Value, AgentError> {
        let mut operation = PendingOperation::new(operation);
        let deadline = self
            .deadline
            .intersect(deadline)
            .map_err(local_deadline_error)?;
        self.call_bound(operation.take(), &deadline, cancelled)
    }
    fn put_kv_stream(
        &self,
        header: KvUploadHeader,
        reader: &mut dyn std::io::Read,
    ) -> Result<Value, AgentError> {
        self.stream_bound(header, reader, &self.deadline)
    }
    fn put_kv_stream_with_deadline(
        &self,
        header: KvUploadHeader,
        reader: &mut dyn std::io::Read,
        deadline: &Deadline,
    ) -> Result<Value, AgentError> {
        let deadline = self
            .deadline
            .intersect(deadline)
            .map_err(local_deadline_error)?;
        self.stream_bound(header, reader, &deadline)
    }
}

fn clear(value: &mut Value) {
    match value {
        Value::String(s) => s.zeroize(),
        Value::Array(items) => items.iter_mut().for_each(clear),
        Value::Object(map) => {
            for (mut key, mut value) in std::mem::take(map) {
                key.zeroize();
                clear(&mut value);
            }
        }
        value => *value = Value::Null,
    }
}

/// An interruption must erase late text without turning a security/clock failure
/// into a retryable cancellation. Keep its typed classification and ambiguity.
pub(crate) fn preserve_terminal_transport_error(result: &mut Result<Value, AgentError>) -> bool {
    let preserve = result.as_ref().err().is_some_and(|error| {
        error.fatal()
            || matches!(
                error,
                AgentError::Ipc {
                    code: IpcErrorCode::DeadlineClock,
                    ..
                }
            )
    });
    if !preserve {
        return false;
    }
    discard_transport_result(result);
    match result {
        Err(AgentError::Protocol { message, .. }) => {
            *message = "Desktop and agent protocol versions do not match.".into();
        }
        Err(AgentError::Ipc { code, message, .. }) => {
            *message = format!(
                "Agent transport rejected the operation ({}).",
                code.as_str()
            );
        }
        _ => unreachable!("classified terminal transport error"),
    }
    true
}

pub fn discard_transport_result(result: &mut Result<Value, AgentError>) {
    match result {
        Ok(value) => clear(value),
        Err(AgentError::Protocol {
            message, fields, ..
        }) => {
            message.zeroize();
            fields.capability.zeroize();
            fields.profile.zeroize();
            fields.state_dir.zeroize();
            fields.reason.zeroize();
        }
        Err(
            AgentError::Transport(s)
            | AgentError::Ambiguous(s)
            | AgentError::Ipc { message: s, .. },
        ) => s.zeroize(),
        Err(_) => {}
    }
}

/// Erase an operation abandoned before the IPC layer takes ownership.
pub struct PendingOperation(Option<Operation>);
impl PendingOperation {
    pub fn new(operation: Operation) -> Self {
        Self(Some(operation))
    }
    pub fn take(&mut self) -> Operation {
        self.0.take().expect("owned operation")
    }
}
impl Drop for PendingOperation {
    fn drop(&mut self) {
        if let Some(operation) = &mut self.0 {
            operation.zeroize_plaintext();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::Mutex,
        time::{Duration, Instant},
    };
    struct Recording(Mutex<Vec<foks_local_deadline::DeadlineEnvelope>>);
    impl AgentTransport for Recording {
        fn call(&self, _: Operation) -> Result<Value, AgentError> {
            panic!("bound transport must not fall back to a fresh unbound call")
        }
        fn call_with_deadline(
            &self,
            _: Operation,
            deadline: &Deadline,
            cancelled: &dyn Fn() -> bool,
        ) -> Result<Value, AgentError> {
            assert!(!cancelled());
            self.0.lock().unwrap().push(deadline.envelope().clone());
            Ok(Value::Null)
        }
    }
    #[test]
    fn helper_calls_and_nested_wrappers_forward_one_original_envelope() {
        let recorder = Arc::new(Recording(Mutex::new(vec![])));
        let deadline = Deadline::capture(Instant::now() + Duration::from_secs(15)).unwrap();
        let inner: Arc<dyn AgentTransport> = recorder.clone();
        let bound = bind_transport_deadline(inner, deadline.clone()).unwrap();
        let later = Deadline::capture(Instant::now() + Duration::from_secs(60)).unwrap();
        let nested = bind_transport_deadline(bound, later).unwrap();
        nested.call(Operation::Ping).unwrap();
        nested
            .call_cancellable(Operation::AgentStatus, &|| false)
            .unwrap();
        assert_eq!(
            *recorder.0.lock().unwrap(),
            vec![deadline.envelope().clone(); 2]
        );
    }
    #[test]
    fn an_adapter_missing_context_support_fails_closed() {
        struct Missing;
        impl AgentTransport for Missing {
            fn call(&self, _: Operation) -> Result<Value, AgentError> {
                panic!("unsupported deadline must not dispatch")
            }
        }
        let deadline = Deadline::capture(Instant::now() + Duration::from_secs(15)).unwrap();
        let bound = bind_transport_deadline(Arc::new(Missing), deadline).unwrap();
        assert!(matches!(
            bound.call(Operation::Ping),
            Err(AgentError::Ipc {
                code: IpcErrorCode::Unsupported,
                ..
            })
        ));
    }
    #[test]
    fn abandoned_transport_plaintext_is_cleared_recursively() {
        let mut result = Ok(serde_json::json!({"secret-key":["secret",{"nested":"value"}]}));
        discard_transport_result(&mut result);
        assert_eq!(result.unwrap(), serde_json::json!({}));
    }

    #[test]
    fn bound_interruption_preserves_security_and_clock_classification_without_late_text() {
        use std::sync::atomic::{AtomicBool, Ordering};
        struct Failed {
            cancelled: Arc<AtomicBool>,
            code: IpcErrorCode,
            expire: bool,
        }
        impl AgentTransport for Failed {
            fn call(&self, _: Operation) -> Result<Value, AgentError> {
                unreachable!("bound adapter uses the context-aware method")
            }
            fn call_with_deadline(
                &self,
                _: Operation,
                deadline: &Deadline,
                _: &dyn Fn() -> bool,
            ) -> Result<Value, AgentError> {
                if self.expire {
                    // Wait on this exact deadline, not a guessed scheduling delay.
                    while let Some(remaining) =
                        deadline.instant().checked_duration_since(Instant::now())
                    {
                        std::thread::park_timeout(remaining);
                    }
                }
                self.cancelled.store(true, Ordering::Release);
                Err(AgentError::Ipc {
                    code: self.code,
                    message: "late private diagnostic".into(),
                    ambiguous: true,
                    connection_lost: false,
                })
            }
        }
        for (code, expire) in [
            (IpcErrorCode::UnsafeSocket, false),
            (IpcErrorCode::Protocol, false),
            (IpcErrorCode::ResponseBinding, false),
            (IpcErrorCode::VersionMismatch, false),
            (IpcErrorCode::DeadlineClock, false),
            (IpcErrorCode::UnsafeSocket, true),
        ] {
            let cancelled = Arc::new(AtomicBool::new(false));
            let deadline = Deadline::capture(Instant::now() + Duration::from_secs(1)).unwrap();
            let bound = bind_transport_deadline(
                Arc::new(Failed {
                    cancelled: cancelled.clone(),
                    code,
                    expire,
                }),
                deadline,
            )
            .unwrap();
            let error = bound
                .call_cancellable(Operation::Ping, &|| cancelled.load(Ordering::Acquire))
                .unwrap_err();
            assert!(
                matches!(error, AgentError::Ipc { code: actual, ambiguous: true, connection_lost: false, .. } if actual == code)
            );
            assert!(!error.user_message().contains("late private"));
            assert!(error.user_message().contains(code.as_str()));
        }
    }

    #[test]
    fn terminal_protocol_error_erases_owned_fields_but_retains_its_code() {
        let mut result = Err(AgentError::Protocol {
            code: foks_agent_proto::ErrorCode::VersionMismatch,
            message: "private diagnostic".into(),
            fields: Box::new(foks_agent_proto::ErrorFields {
                profile: Some("private profile".into()),
                reason: Some("private reason".into()),
                ..Default::default()
            }),
        });
        assert!(preserve_terminal_transport_error(&mut result));
        let Err(AgentError::Protocol {
            code,
            message,
            fields,
        }) = result
        else {
            unreachable!()
        };
        assert_eq!(code, foks_agent_proto::ErrorCode::VersionMismatch);
        assert!(!message.contains("private"));
        assert!(fields.profile.as_deref().is_none_or(str::is_empty));
        assert!(fields.reason.as_deref().is_none_or(str::is_empty));
    }
}
