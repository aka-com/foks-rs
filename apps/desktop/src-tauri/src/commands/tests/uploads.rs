use super::support::{account_ref, phase_four_state};
use crate::agent::AgentError;
use crate::commands::context::AppState;
use crate::commands::store_key::store_id;
use crate::commands::vault::{change_file_upload_in, list_file_uploads_in};
use foks_agent_proto::{KvStoreRef, Operation, UploadHandle};
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

fn list(
    state: &AppState,
    transport: &Arc<UploadOperations>,
    after: Option<[u8; 32]>,
    lock: &Lock,
) -> Result<serde_json::Value, AgentError> {
    let (_, id) = store();
    let transport: Arc<dyn foks_desktop::AgentTransport> = transport.clone();
    tauri::async_runtime::block_on(list_file_uploads_in(
        state,
        move |_| Ok(transport),
        &id,
        after,
        || lock.check(),
    ))
}

#[test]
fn the_listing_passes_the_store_and_cursor_and_answers_only_while_unlocked() {
    let state = phase_four_state(Vec::new());
    let transport = UploadOperations::new(|| Ok(serde_json::json!({"uploads": []})));
    let lock = Lock::default();
    let value = list(&state, &transport, Some([5; 32]), &lock).unwrap();
    assert_eq!(value["uploads"], serde_json::json!([]));
    assert!(matches!(
        transport.calls.lock().unwrap().as_slice(),
        [Operation::ListKvUploads {
            store: KvStoreRef::Account(account),
            after: Some(after),
        }] if account.account_alias == "personal" && *after == [5; 32]
    ));
    // A lock while the agent answered withholds the listing.
    let during = lock.clone();
    let transport = UploadOperations::with_hook(
        || Ok(serde_json::json!({"uploads": []})),
        Box::new(move || during.lock()),
    );
    assert_eq!(
        list(&state, &transport, None, &lock).unwrap_err().code,
        "app-locked"
    );
}

#[test]
fn a_change_takes_the_reservation_before_it_reads_the_handle() {
    let state = phase_four_state(Vec::new());
    let lock = Lock::default();
    let transport = UploadOperations::new(report);
    // Another change holds the reservation: refused before the handle is parsed.
    let held = state
        .for_profile("work.example")
        .unwrap()
        .begin_mutation()
        .unwrap();
    let error = change(
        &state.for_profile("work.example").unwrap(),
        &transport,
        "not a handle",
        true,
        &lock,
    )
    .unwrap_err();
    assert_eq!(error.code, "mutation-in-flight");
    drop(held);
    let error = change(&state, &transport, "not a handle", true, &lock).unwrap_err();
    assert_eq!(error.code, "invalid-request");
    assert!(transport.calls.lock().unwrap().is_empty());
}

#[test]
fn a_change_discards_the_store_items_before_the_agent_is_asked() {
    let state = phase_four_state(Vec::new());
    let (store, _) = store();
    let mut item = super::vault::download_item(1);
    item.store = store.clone();
    state
        .catalog
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .items
        .push(item);
    let observed = state.clone();
    let listed = Arc::new(Mutex::new(None));
    let seen = listed.clone();
    let transport = UploadOperations::with_hook(
        report,
        Box::new(move || {
            let catalog = observed.catalog.lock().unwrap();
            *seen.lock().unwrap() = Some(
                catalog
                    .as_ref()
                    .unwrap()
                    .items
                    .iter()
                    .filter(|item| item.store == store)
                    .count(),
            );
        }),
    );
    change(&state, &transport, &handle(), false, &Lock::default()).unwrap();
    assert!(matches!(
        transport.calls.lock().unwrap().as_slice(),
        [Operation::CancelKvUpload { upload_id, .. }] if upload_id.to_string() == handle()
    ));
    assert_eq!(*listed.lock().unwrap(), Some(0));
}

#[test]
fn only_an_ambiguous_change_requires_a_refresh() {
    let state = phase_four_state(Vec::new());
    let lock = Lock::default();
    let refused = UploadOperations::new(|| {
        Err(foks_desktop::AgentError::Protocol {
            code: foks_agent_proto::ErrorCode::UploadExpired,
            message: "expired".to_owned(),
            fields: Box::default(),
        })
    });
    assert_eq!(
        change(&state, &refused, &handle(), true, &lock)
            .unwrap_err()
            .code,
        "upload-expired"
    );
    assert!(!state.mutation_requires_refresh().load(Ordering::Acquire));
    let lost =
        UploadOperations::new(|| Err(foks_desktop::AgentError::Ambiguous("reply lost".to_owned())));
    assert!(
        change(&state, &lost, &handle(), true, &lock)
            .unwrap_err()
            .ambiguous
    );
    assert!(state.mutation_requires_refresh().load(Ordering::Acquire));
    // The flag does not stop another upload change: it acts on a handle only.
    assert!(change(
        &state,
        &UploadOperations::new(report),
        &handle(),
        false,
        &lock
    )
    .is_ok());
}
