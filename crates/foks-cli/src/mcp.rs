use foks_agent_client::AgentClient;
use foks_agent_proto::{AgentStatus, Operation, Peer, Response, ResponseResult};
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

/// What a Hello finds at the agent socket.
enum AgentProbe {
    Ready,
    /// An agent answers but its state is not initialized.
    Bootstrap,
    /// An agent answers but uses another protocol version, refuses the
    /// Hello, or does not report its status. It holds the state directory's
    /// lock, so a second agent cannot start beside it.
    Unusable(String),
    Absent,
}

fn probe_agent(client: &AgentClient) -> AgentProbe {
    match client.hello(Peer::new("foks-rs", env!("CARGO_PKG_VERSION"))) {
        Ok(_) => {}
        // Something answered the Hello, so a second agent could not take the
        // state directory's lock.
        Err(
            error @ (foks_agent_client::Error::Protocol(_)
            | foks_agent_client::Error::ResponseBinding),
        ) => return AgentProbe::Unusable(incompatible_agent_message(&error)),
        Err(_) => return AgentProbe::Absent,
    }
    let detail = match client.call(Operation::AgentStatus) {
        Ok(Response {
            result: ResponseResult::Success { value },
            ..
        }) => match serde_json::from_value::<AgentStatus>(value) {
            Ok(status) if status.is_ready() => return AgentProbe::Ready,
            Ok(_) => return AgentProbe::Bootstrap,
            Err(error) => format!("invalid agent status: {error}"),
        },
        Ok(Response {
            result: ResponseResult::Error { message, .. },
            ..
        }) => message,
        Err(error) if error.is_version_mismatch() => {
            return AgentProbe::Unusable(incompatible_agent_message(&error))
        }
        Err(error) => error.to_string(),
    };
    AgentProbe::Unusable(format!(
        "the running foks-agent did not report its status: {detail}"
    ))
}

/// The diagnosis for an agent of another protocol version, which holds the
/// state directory's lock, so a second agent cannot start beside it.
fn incompatible_agent_message(error: &foks_agent_client::Error) -> String {
    let detail = error
        .version_mismatch()
        .map_or_else(|| error.to_string(), ToString::to_string);
    format!(
        "the running foks-agent is incompatible: {detail}; stop it (quit the FOKS app or end the foks-agent process) and retry"
    )
}

pub(super) fn ensure_agent(state: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut client = AgentClient::new(state.join("foks-rs.sock"));
    client.set_timeout(Duration::from_millis(250))?;
    match probe_agent(&client) {
        AgentProbe::Ready => return Ok(()),
        AgentProbe::Bootstrap => return Err(UNINITIALIZED_STATE.into()),
        AgentProbe::Unusable(message) => return Err(message.into()),
        AgentProbe::Absent => {}
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
        match probe_agent(&client) {
            AgentProbe::Ready => {
                // Reap this child if it later exits; it remains resident after MCP EOF.
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Ok(());
            }
            // An agent reports bootstrap only while the state is not
            // initialized. Another launcher may also have won the lock with
            // an agent of another version.
            AgentProbe::Bootstrap => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(UNINITIALIZED_STATE.into());
            }
            AgentProbe::Unusable(message) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(message.into());
            }
            AgentProbe::Absent => {}
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("authenticated agent startup timed out".into());
        }
        if child.try_wait()?.is_some() {
            // Another concurrent launcher may have won the agent's existing lock.
            return match probe_agent(&client) {
                AgentProbe::Ready => Ok(()),
                AgentProbe::Unusable(message) => Err(message.into()),
                AgentProbe::Bootstrap | AgentProbe::Absent => Err(UNINITIALIZED_STATE.into()),
            };
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

const UNINITIALIZED_STATE: &str =
    "agent did not start; initialize and unlock the selected state first";

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};
    use std::os::unix::fs::PermissionsExt as _;

    /// An agent at the socket that answers every request with `reply`.
    fn serve_agent(
        socket: &Path,
        reply: fn(Option<u64>) -> serde_json::Value,
    ) -> std::thread::JoinHandle<usize> {
        let listener = std::os::unix::net::UnixListener::bind(socket).unwrap();
        std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        listener.set_nonblocking(true).unwrap();
        std::thread::spawn(move || {
            let mut served = 0;
            let deadline = Instant::now() + Duration::from_millis(500);
            while Instant::now() < deadline {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                stream.set_nonblocking(false).unwrap();
                let mut prefix = [0u8; 4];
                stream.read_exact(&mut prefix).unwrap();
                let mut frame = vec![0; 4 + u32::from_be_bytes(prefix) as usize];
                frame[..4].copy_from_slice(&prefix);
                stream.read_exact(&mut frame[4..]).unwrap();
                let reply = reply(foks_agent_proto::request_id(&frame));
                stream
                    .write_all(&foks_agent_proto::encode(&reply).unwrap())
                    .unwrap();
                served += 1;
            }
            served
        })
    }

    /// An agent left running from an older build refuses every request's
    /// version.
    fn older_agent_refusal(id: Option<u64>) -> serde_json::Value {
        serde_json::json!({
            "version": 34,
            "id": id,
            "status": "error",
            "code": "version-mismatch",
            "message": "unsupported protocol version",
        })
    }

    /// The running agent holds the state directory's lock, so the remedy
    /// for a version mismatch is to stop it, not to start a second agent.
    #[test]
    fn an_incompatible_running_agent_is_diagnosed_without_starting_another() {
        let directory = tempfile::tempdir().unwrap();
        let server = serve_agent(&directory.path().join("foks-rs.sock"), older_agent_refusal);
        let error = ensure_agent(directory.path()).unwrap_err().to_string();
        assert_eq!(server.join().unwrap(), 1, "only the Hello was sent");
        assert_eq!(
            error,
            format!(
                "the running foks-agent is incompatible: agent protocol version 34 does not \
                 match client protocol version {}; stop it (quit the FOKS app or end the \
                 foks-agent process) and retry",
                foks_agent_proto::PROTOCOL_VERSION
            )
        );
    }

    /// Any answer to the Hello shows that an agent holds the socket and the
    /// lock, even one that refuses the Hello itself.
    #[test]
    fn a_refused_hello_is_reported_without_starting_another_agent() {
        let directory = tempfile::tempdir().unwrap();
        let server = serve_agent(&directory.path().join("foks-rs.sock"), |id| {
            serde_json::json!({
                "version": foks_agent_proto::PROTOCOL_VERSION,
                "id": id,
                "status": "error",
                "code": "invalid-request",
                "message": "unknown operation",
            })
        });
        let error = ensure_agent(directory.path()).unwrap_err().to_string();
        assert_eq!(server.join().unwrap(), 1, "only the Hello was sent");
        assert!(
            error.starts_with(
                "the running foks-agent is incompatible: local agent protocol failed: agent \
                 refused the handshake: invalid-request: unknown operation"
            ),
            "{error}"
        );
    }
}
