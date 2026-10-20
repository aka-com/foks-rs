use super::support::{account_ref, phase_four_state};
use crate::agent::AgentError;
use crate::commands::context::AppState;
use crate::commands::store_key::store_id;
use crate::commands::vault::change_file_upload_in;
use foks_agent_proto::{Operation, UploadHandle};
use foks_desktop::CatalogStoreRef;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

type Hook = Box<dyn Fn() + Send + Sync>;

/// Answers the upload operations with `reply`, running `during` while the
/// agent call is in progress, and records each operation.
struct UploadOperations {
    calls: Mutex<Vec<Operation>>,
    reply: fn() -> Result<serde_json::Value, foks_desktop::AgentError>,
    during: Hook,
}

impl UploadOperations {
    fn new(reply: fn() -> Result<serde_json::Value, foks_desktop::AgentError>) -> Arc<Self> {
        Self::with_hook(reply, Box::new(|| {}))
    }
    fn with_hook(
        reply: fn() -> Result<serde_json::Value, foks_desktop::AgentError>,
        during: Hook,
    ) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            reply,
            during,
        })
    }
}

impl foks_desktop::AgentTransport for UploadOperations {
    fn call_with_deadline(
        &self,
        operation: Operation,
        _deadline: &foks_local_deadline::Deadline,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<serde_json::Value, foks_desktop::AgentError> {
        self.call_cancellable(operation, cancelled)
    }
    fn put_kv_stream_with_deadline(
        &self,
        _header: foks_agent_proto::KvUploadHeader,
        _reader: &mut dyn std::io::Read,
        _deadline: &foks_local_deadline::Deadline,
    ) -> Result<serde_json::Value, foks_desktop::AgentError> {
        unreachable!("the upload commands do not stream")
    }
    fn call(&self, operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        self.calls.lock().unwrap().push(operation);
        (self.during)();
        (self.reply)()
    }
    fn put_kv_stream(
        &self,
        _header: foks_agent_proto::KvUploadHeader,
        _reader: &mut dyn std::io::Read,
    ) -> Result<serde_json::Value, foks_desktop::AgentError> {
        unreachable!("the upload commands do not stream")
    }
}

fn report() -> Result<serde_json::Value, foks_desktop::AgentError> {
    Ok(serde_json::json!({"upload_id": handle(), "state": "completed"}))
}

fn handle() -> String {
    UploadHandle::new([1; 16], 2, [3; 16]).to_string()
}

fn store() -> (CatalogStoreRef, String) {
    let store = CatalogStoreRef::Account(account_ref("work.example", "personal"));
    let id = store_id(&store);
    (store, id)
}

/// The application lock as the commands see it: unlocked until `lock`.
#[derive(Clone, Default)]
struct Lock(Arc<AtomicBool>);

impl Lock {
    fn lock(&self) {
        self.0.store(true, Ordering::Release);
    }
    fn check(&self) -> Result<(), AgentError> {
        if self.0.load(Ordering::Acquire) {
            Err(AgentError::new(
                "app-locked",
                "The application locked before the change was submitted.",
                false,
            ))
        } else {
            Ok(())
        }
    }
}

fn change(
    state: &AppState,
    transport: &Arc<UploadOperations>,
    upload_id: &str,
    resume: bool,
    lock: &Lock,
) -> Result<serde_json::Value, AgentError> {
    let (_, id) = store();
    let transport: Arc<dyn foks_desktop::AgentTransport> = transport.clone();
    tauri::async_runtime::block_on(change_file_upload_in(
        state,
        move |_| Ok(transport),
        &id,
        upload_id,
        resume,
        || lock.check(),
    ))
}

#[test]
fn a_lock_during_a_resume_or_cancel_does_not_discard_the_applied_report() {
    for resume in [true, false] {
        let state = phase_four_state(Vec::new());
        let lock = Lock::default();
        let during = lock.clone();
        let transport = UploadOperations::with_hook(report, Box::new(move || during.lock()));
        let value = change(&state, &transport, &handle(), resume, &lock).unwrap();
        assert_eq!(value["state"], "completed");
        assert_eq!(transport.calls.lock().unwrap().len(), 1);
    }
    // A lock before the call still refuses it before the agent is asked.
    let state = phase_four_state(Vec::new());
    let lock = Lock::default();
    lock.lock();
    let transport = UploadOperations::new(report);
    let error = change(&state, &transport, &handle(), true, &lock).unwrap_err();
    assert_eq!(error.code, "app-locked");
    assert!(transport.calls.lock().unwrap().is_empty());
}
