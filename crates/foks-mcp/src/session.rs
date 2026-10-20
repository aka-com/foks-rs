//! SDK dispatch is separated from synchronous, cancellable agent work.
use crate::contract::{ContractError, Invocation, ToolSet, ACTIVE_CALLS, MAX_OUTPUT_BYTES};
use foks_local_deadline::Deadline;
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
        PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerInfo, Tool,
    },
    service::RequestContext,
    ErrorData, RoleServer, ServerHandler,
};
use std::{borrow::Cow, sync::Arc};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroize;

pub trait Backend: Send + Sync + 'static {
    fn invoke(
        &self,
        invocation: Invocation,
        cancelled: &CancellationToken,
        deadline: &Deadline,
    ) -> Result<CallToolResult, String>;
}

pub struct Session<B> {
    set: ToolSet,
    read_only: bool,
    backend: Arc<B>,
    active: Arc<Semaphore>,
    lifetime: CancellationToken,
}

impl<B: Backend> Session<B> {
    pub fn new(set: ToolSet, read_only: bool, backend: B, lifetime: CancellationToken) -> Self {
        Self {
            set,
            read_only,
            backend: Arc::new(backend),
            active: Arc::new(Semaphore::new(ACTIVE_CALLS)),
            lifetime,
        }
    }
}

impl<B: Backend> ServerHandler for Session<B> {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.protocol_version = ProtocolVersion::V_2025_11_25;
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info.name = match self.set {
            ToolSet::Kv => "foks-kv",
            ToolSet::Team => "foks-team",
        }
        .into();
        info.server_info.version = env!("CARGO_PKG_VERSION").into();
        info
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(&[
            ProtocolVersion::V_2025_11_25,
            ProtocolVersion::V_2025_06_18,
            ProtocolVersion::V_2025_03_26,
            ProtocolVersion::V_2024_11_05,
        ])
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.set
            .tools(self.read_only)
            .into_iter()
            .find(|tool| tool.name == name)
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        if request.is_some_and(|r| r.cursor.is_some()) {
            return Err(ErrorData::invalid_params(
                "tool list has no continuation",
                None,
            ));
        }
        Ok(ListToolsResult {
            tools: self.set.tools(self.read_only),
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        // Waiting for an active-call slot is bounded on its own. The call's
        // deadline starts once it holds a slot, so queueing behind other
        // calls does not consume the budget of its IPC.
        let admission_until =
            tokio::time::Instant::now() + foks_agent_proto::DeadlineClass::Ordinary.cap();
        let invocation = match self.set.parse(
            self.read_only,
            &request.name,
            request.arguments.unwrap_or_default(),
        ) {
            Ok(invocation) => invocation,
            Err(ContractError::UnknownTool) => {
                return Err(ErrorData::invalid_params(
                    "unknown or unavailable tool",
                    None,
                ))
            }
            Err(error) => return Ok(tool_error(error.to_string()).into()),
        };
        let permit = tokio::select! {
            _ = tokio::time::sleep_until(admission_until) => return Ok(tool_error("request deadline exceeded before dispatch").into()),
            _ = self.lifetime.cancelled() => return Ok(tool_error("session closed").into()),
            _ = context.ct.cancelled() => return Ok(tool_error("request cancelled").into()),
            permit = self.active.clone().acquire_owned() => permit.map_err(|_| ErrorData::internal_error("server closing", None))?,
        };
        // The same deadline covers scope resolution and every IPC call.
        let deadline = invocation_deadline(&invocation)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let backend = self.backend.clone();
        let cancellation = self.lifetime.child_token();
        let request_cancel = cancellation.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = context.ct.cancelled() => request_cancel.cancel(),
                _ = request_cancel.cancelled() => (),
            }
        });
        // Dropping the SDK future on EOF/cancel also interrupts blocking IPC reads.
        let _cancel_on_drop = cancellation.clone().drop_guard();
        deadline
            .check()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let mut worker = spawn_worker(backend, invocation, cancellation, deadline.clone(), permit);
        let mut guarded =
            match tokio::time::timeout_at(deadline.instant().into(), &mut worker).await {
                Ok(result) => {
                    result.map_err(|_| ErrorData::internal_error("agent worker failed", None))?
                }
                Err(_) => {
                    // Dropping the join handle detaches the worker. Its result guard
                    // erases late plaintext before releasing physical admission.
                    return Ok(tool_error(
                        "request deadline exceeded; check mutation status before retrying",
                    )
                    .into());
                }
            };
        let result = guarded.result.as_mut().expect("owned worker result");
        if let Err(error) = deadline.check() {
            discard_result(result);
            return Ok(tool_error(error.to_string()).into());
        }
        // Leave room for the outer response, including the caller's bounded request ID.
        let encoded = zeroize::Zeroizing::new(
            serde_json::to_vec(&*result)
                .map_err(|_| ErrorData::internal_error("invalid tool result", None))?,
        );
        if encoded.len() > MAX_OUTPUT_BYTES - 1024 {
            discard_result(result);
            return Ok(tool_error(ContractError::TooLarge.to_string()).into());
        }
        if let Err(error) = deadline.check() {
            discard_result(result);
            return Ok(tool_error(error.to_string()).into());
        }
        Ok(guarded.result.take().expect("owned worker result").into())
    }
}

/// The deadline class of a tool call. A `put` streams up to 4 MiB to the
/// agent, which is a bulk transfer; every other tool is an ordinary request.
fn invocation_class(invocation: &Invocation) -> foks_agent_proto::DeadlineClass {
    match invocation {
        Invocation::Put(_) => foks_agent_proto::DeadlineClass::Bulk,
        _ => foks_agent_proto::DeadlineClass::Ordinary,
    }
}

fn invocation_deadline(invocation: &Invocation) -> Result<Deadline, foks_local_deadline::Error> {
    Deadline::capture(std::time::Instant::now() + invocation_class(invocation).cap())
}

fn spawn_worker<B: Backend>(
    backend: Arc<B>,
    invocation: Invocation,
    cancellation: CancellationToken,
    worker_deadline: Deadline,
    permit: tokio::sync::OwnedSemaphorePermit,
) -> tokio::task::JoinHandle<WorkerResult> {
    tokio::task::spawn_blocking(move || {
        if cancellation.is_cancelled() {
            return WorkerResult::new(tool_error("request cancelled"), permit);
        }
        if let Err(error) = worker_deadline.check() {
            return WorkerResult::new(tool_error(error.to_string()), permit);
        }
        let mut result = backend
            .invoke(invocation, &cancellation, &worker_deadline)
            .unwrap_or_else(tool_error);
        if cancellation.is_cancelled() || worker_deadline.check().is_err() {
            discard_result(&mut result);
            return WorkerResult::new(
                tool_error("request interrupted; check mutation status before retrying"),
                permit,
            );
        }
        WorkerResult::new(result, permit)
    })
}

pub fn tool_error(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.into())])
}

pub fn text_result(text: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(text.into())])
}

// Production tools return text. Clear its owned plaintext before abandoning a late result.
fn discard_result(result: &mut CallToolResult) {
    for block in &mut result.content {
        match block {
            ContentBlock::Text(text) => text.text.zeroize(),
            ContentBlock::Image(image) => image.data.zeroize(),
            ContentBlock::Audio(audio) => audio.data.zeroize(),
            _ => {}
        }
    }
    fn clear(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::String(text) => text.zeroize(),
            serde_json::Value::Array(values) => values.iter_mut().for_each(clear),
            serde_json::Value::Object(values) => {
                for (mut key, mut value) in std::mem::take(values) {
                    key.zeroize();
                    clear(&mut value);
                }
            }
            value => *value = serde_json::Value::Null,
        }
    }
    if let Some(value) = &mut result.structured_content {
        clear(value);
    }
    result.content.clear();
}

struct WorkerResult {
    result: Option<CallToolResult>,
    _permit: tokio::sync::OwnedSemaphorePermit,
}
impl WorkerResult {
    fn new(result: CallToolResult, permit: tokio::sync::OwnedSemaphorePermit) -> Self {
        Self {
            result: Some(result),
            _permit: permit,
        }
    }
}
impl Drop for WorkerResult {
    fn drop(&mut self) {
        if let Some(result) = &mut self.result {
            discard_result(result);
        }
    }
}

#[cfg(test)]
mod deadline_tests {
    use super::*;
    use std::{
        sync::{mpsc, Mutex},
        time::{Duration, Instant},
    };

    struct PausedBackend {
        entered: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        release: Mutex<mpsc::Receiver<()>>,
    }
    impl Backend for PausedBackend {
        fn invoke(
            &self,
            _: Invocation,
            _: &CancellationToken,
            _: &Deadline,
        ) -> Result<CallToolResult, String> {
            self.entered
                .lock()
                .unwrap()
                .take()
                .unwrap()
                .send(())
                .unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
            Ok(text_result("late plaintext"))
        }
    }

    #[tokio::test]
    async fn deadline_and_cancel_retain_worker_admission_until_late_result_is_discarded() {
        for expire in [false, true] {
            let admission = Arc::new(Semaphore::new(1));
            let permit = admission.clone().acquire_owned().await.unwrap();
            let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = mpsc::channel();
            let deadline = Deadline::capture(
                Instant::now() + Duration::from_secs(if expire { 1 } else { 30 }),
            )
            .unwrap();
            let cancellation = CancellationToken::new();
            let mut worker = spawn_worker(
                Arc::new(PausedBackend {
                    entered: Mutex::new(Some(entered_tx)),
                    release: Mutex::new(release_rx),
                }),
                Invocation::Pending(crate::contract::EmptyArgs {}),
                cancellation.clone(),
                deadline.clone(),
                permit,
            );
            entered_rx.await.unwrap();
            if expire {
                assert!(
                    tokio::time::timeout_at(deadline.instant().into(), &mut worker)
                        .await
                        .is_err()
                );
            } else {
                cancellation.cancel();
            }
            assert_eq!(admission.available_permits(), 0);
            assert!(admission.clone().try_acquire_owned().is_err());
            release_tx.send(()).unwrap();
            let guarded = worker.await.unwrap();
            let encoded = serde_json::to_string(guarded.result.as_ref().unwrap()).unwrap();
            assert!(!encoded.contains("late plaintext"));
            assert_eq!(admission.available_permits(), 0);
            drop(guarded);
            assert_eq!(admission.available_permits(), 1);
        }
    }

    /// A `put` streams its body under the bulk budget; the ordinary
    /// 15-second cap would cancel a 4 MiB upload on a slow link.
    #[test]
    fn put_calls_get_the_bulk_budget_and_reads_the_ordinary_one() {
        let parse = |name: &str, arguments: serde_json::Value| {
            let serde_json::Value::Object(arguments) = arguments else {
                unreachable!()
            };
            ToolSet::Kv.parse(false, name, arguments).ok().unwrap()
        };
        let ordinary = foks_agent_proto::DeadlineClass::Ordinary.cap();
        let put = parse("put", serde_json::json!({"path": "/a", "content": "body"}));
        let started = Instant::now();
        let deadline = invocation_deadline(&put).unwrap();
        assert!(deadline.instant() > started + ordinary);
        assert!(
            deadline.instant()
                >= started + foks_agent_proto::DeadlineClass::Bulk.cap() - Duration::from_secs(1)
        );
        let get = parse("get", serde_json::json!({"path": "/a"}));
        assert!(invocation_deadline(&get).unwrap().instant() <= Instant::now() + ordinary);
    }

    #[test]
    fn abandoned_result_clears_text_and_nested_structured_plaintext() {
        let mut result = text_result("secret");
        result.structured_content = Some(serde_json::json!({"private-key": ["secret", 42]}));
        discard_result(&mut result);
        assert!(result.content.is_empty());
        assert_eq!(result.structured_content, Some(serde_json::json!({})));
    }
}
