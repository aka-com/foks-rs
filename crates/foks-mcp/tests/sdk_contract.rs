use foks_mcp::{contract::ToolSet, transport::BoundedTransport};
use rmcp::{
    model::{
        ListToolsResult, PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerInfo,
    },
    service::RequestContext,
    RoleServer, ServerHandler, ServiceExt,
};

struct ContractServer(ToolSet);

impl ServerHandler for ContractServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.protocol_version = ProtocolVersion::V_2025_11_25;
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        Ok(ListToolsResult {
            tools: self.0.tools(false),
            ..Default::default()
        })
    }
}

#[tokio::test]
async fn sdk_negotiates_and_lists_separate_tool_sets_over_bounded_framing() {
    for set in [ToolSet::Kv, ToolSet::Team] {
        let (server_io, client_io) = tokio::io::duplex(32 * 1024);
        let (read, write) = tokio::io::split(server_io);
        let server = tokio::spawn(async move {
            ContractServer(set)
                .serve(BoundedTransport::new(read, write))
                .await
                .unwrap()
                .waiting()
                .await
                .unwrap();
        });
        let client = ().serve(client_io).await.unwrap();
        assert_eq!(
            client.peer().peer_info().unwrap().protocol_version,
            ProtocolVersion::V_2025_11_25
        );
        client
            .peer()
            .send_request(rmcp::model::ClientRequest::PingRequest(Default::default()))
            .await
            .unwrap();
        let tools = client.peer().list_all_tools().await.unwrap();
        assert_eq!(
            tools.iter().map(|t| t.name.as_ref()).collect::<Vec<_>>(),
            set.names(false)
        );
        client.cancel().await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
}

struct WaitingBackend {
    started: std::sync::Arc<std::sync::atomic::AtomicBool>,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl foks_mcp::session::Backend for WaitingBackend {
    fn invoke(
        &self,
        _: foks_mcp::contract::Invocation,
        token: &tokio_util::sync::CancellationToken,
    ) -> Result<rmcp::model::CallToolResult, String> {
        self.started
            .store(true, std::sync::atomic::Ordering::Release);
        while !token.is_cancelled() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        Err("cancelled".into())
    }
}

#[tokio::test]
async fn disconnect_cancels_dispatched_backend_work() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let started = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::new(AtomicBool::new(false));
    let backend = WaitingBackend {
        started: started.clone(),
        cancelled: cancelled.clone(),
    };
    let (server_io, client_io) = tokio::io::duplex(8192);
    let (read, write) = tokio::io::split(server_io);
    let server = tokio::spawn(async move {
        let transport = BoundedTransport::new(read, write);
        foks_mcp::session::Session::new(ToolSet::Kv, true, backend, transport.lifetime())
            .serve(transport)
            .await
            .unwrap()
            .waiting()
            .await
            .unwrap();
    });
    let client = ().serve(client_io).await.unwrap();
    let peer = client.peer().clone();
    let call = tokio::spawn(async move {
        let mut params = rmcp::model::CallToolRequestParams::new("get");
        params.arguments = Some(
            serde_json::json!({"path":"/x"})
                .as_object()
                .unwrap()
                .clone(),
        );
        peer.call_tool(params).await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !started.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    client.cancel().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !cancelled.load(Ordering::Acquire) {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let _ = call.await;
    tokio::time::timeout(std::time::Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
}
