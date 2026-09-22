use foks_agent_client::AgentClient;
use foks_agent_proto::{Operation, ResponseResult};
use foks_mcp::{agent::AgentBackend, contract::ToolSet};
use std::{
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(clap::Subcommand)]
pub enum McpCommand {
    /// Serve encrypted KV tools over stdin/stdout.
    Kv(McpArguments),
    /// Serve verified team reads over stdin/stdout.
    Team(McpArguments),
}

#[derive(clap::Args)]
pub struct McpArguments {
    #[arg(long)]
    profile: String,
    #[arg(long)]
    account_alias: String,
    /// Omit and reject all write tools.
    #[arg(long)]
    read_only: bool,
}

pub fn run(state: &Path, command: McpCommand) -> Result<(), Box<dyn std::error::Error>> {
    let (set, args) = match command {
        McpCommand::Kv(args) => (ToolSet::Kv, args),
        McpCommand::Team(args) => (ToolSet::Team, args),
    };
    // Starting an agent is entry-point policy; the SDK adapter only uses authenticated IPC.
    ensure_agent(state)?;
    let backend = AgentBackend::connect(
        &state.join("foks-rs.sock"),
        args.profile,
        args.account_alias,
    )?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(4)
        .enable_all()
        .build()?;
    let result = runtime.block_on(foks_mcp::serve_stdio(backend, set, args.read_only));
    runtime.shutdown_timeout(Duration::from_secs(2));
    result
}

pub(super) fn ensure_agent(state: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut client = AgentClient::new(state.join("foks-rs.sock"));
    client.set_timeout(Duration::from_millis(250))?;
    let available = || matches!(client.call(Operation::Ping), Ok(response) if matches!(response.result, ResponseResult::Success { .. }));
    if available() {
        return Ok(());
    }
    // Use the packaged sibling, never a shell command or an executable named by a tool call.
    let executable = std::env::current_exe()?.with_file_name("foks-agent");
    let mut child = Command::new(executable)
        .arg("--state-dir")
        .arg(state)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "cannot start packaged foks-agent; install it next to foks-rs")?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if available() {
            // Reap this child if it later exits; it remains resident after MCP EOF.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            return Ok(());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("authenticated agent startup timed out".into());
        }
        if child.try_wait()?.is_some() {
            // Another concurrent launcher may have won the agent's existing lock.
            if available() {
                return Ok(());
            }
            return Err(
                "agent did not start; initialize and unlock the selected state first".into(),
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
