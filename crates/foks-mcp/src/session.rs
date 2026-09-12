//! SDK dispatch is separated from synchronous, cancellable agent work.
use crate::contract::{ContractError, Invocation, ToolSet, ACTIVE_CALLS, MAX_OUTPUT_BYTES};
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

pub trait Backend: Send + Sync + 'static {
    fn invoke(
        &self,
        invocation: Invocation,
        cancelled: &CancellationToken,
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
            ToolSet::Kv => "fennec-kv",
            ToolSet::Team => "fennec-team",
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
            _ = self.lifetime.cancelled() => return Ok(tool_error("session closed").into()),
            _ = context.ct.cancelled() => return Ok(tool_error("request cancelled").into()),
            permit = self.active.clone().acquire_owned() => permit.map_err(|_| ErrorData::internal_error("server closing", None))?,
        };
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
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            if cancellation.is_cancelled() {
                return tool_error("request cancelled");
            }
            let result = backend.invoke(invocation, &cancellation);
            if cancellation.is_cancelled() {
                return tool_error("request cancelled; check mutation status before retrying");
            }
            result.unwrap_or_else(tool_error)
        })
        .await
        .map_err(|_| ErrorData::internal_error("agent worker failed", None))?;
        // Leave room for the outer response, including the caller's bounded request ID.
        let size = serde_json::to_vec(&result)
            .map_err(|_| ErrorData::internal_error("invalid tool result", None))?
            .len();
        if size > MAX_OUTPUT_BYTES - 1024 {
            return Ok(tool_error(ContractError::TooLarge.to_string()).into());
        }
        Ok(result.into())
    }
}

pub fn tool_error(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.into())])
}

pub fn text_result(text: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(text.into())])
}
