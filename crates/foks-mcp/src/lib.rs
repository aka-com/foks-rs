//! MCP tool contracts and a bounded adapter for the authenticated local agent.
#![forbid(unsafe_code)]

pub mod contract;
pub mod session;
pub mod transport;

pub mod agent;

/// Serve one MCP process; dropping it never stops the resident agent.
pub async fn serve_stdio(
    backend: agent::AgentBackend,
    set: contract::ToolSet,
    read_only: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use rmcp::ServiceExt as _;
    let transport = transport::BoundedTransport::new(tokio::io::stdin(), tokio::io::stdout());
    session::Session::new(set, read_only, backend, transport.lifetime())
        .serve(transport)
        .await?
        .waiting()
        .await?;
    Ok(())
}
