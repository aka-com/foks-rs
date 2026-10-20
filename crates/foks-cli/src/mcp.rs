use foks_agent_client::AgentClient;
use foks_agent_proto::{AgentStatus, ErrorCode, Operation, Peer, Response, ResponseResult};
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
    let backend = start_then_bind(
        &super::deadline::current()?,
        |startup| ensure_agent_with_deadline(state, startup),
        |binding| {
            AgentBackend::connect_with_deadline(
                &state.join("foks-rs.sock"),
                args.profile,
                args.account_alias,
                binding,
            )
            .map_err(Into::into)
        },
    )?;
    super::deadline::pause()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(4)
        .enable_all()
        .build()?;
    let result = runtime.block_on(foks_mcp::serve_stdio(backend, set, args.read_only));
    runtime.shutdown_timeout(Duration::from_secs(2));
    result
}

/// How long agent startup (the running-agent probe, then a launch) may take,
/// and separately how long account binding may then take, both inside the
/// command's own deadline. Binding waits behind another front end's profile
/// work, so it must not inherit only what a slow startup left.
const STARTUP_WINDOW: Duration = Duration::from_secs(5);
const BIND_WINDOW: Duration = Duration::from_secs(5);

/// Runs agent startup under its own window, then account binding under a
/// window taken when binding begins.
fn start_then_bind<T>(
    command: &foks_local_deadline::Deadline,
    start: impl FnOnce(&foks_local_deadline::Deadline) -> Result<(), Box<dyn std::error::Error>>,
    bind: impl FnOnce(&foks_local_deadline::Deadline) -> Result<T, Box<dyn std::error::Error>>,
) -> Result<T, Box<dyn std::error::Error>> {
    start(&command.shorten(Instant::now() + STARTUP_WINDOW)?)?;
    bind(&command.shorten(Instant::now() + BIND_WINDOW)?)
}

/// What a Hello finds at the agent socket.
enum AgentProbe {
    Ready,
    /// An agent answers but its state is not initialized.
    Bootstrap,
    /// An agent accepted the connection but has not reported its status
    /// yet: the Hello or the status request timed out, the agent answered
    /// Busy, or the status connection was lost. A freshly started agent, or
    /// one whose local worker serves another front end, does this, so the
    /// probe is repeated.
    NotReady(String),
    /// An agent answers but uses another protocol version, refuses the
    /// Hello, or answers the status request with something other than a
    /// status. It holds the state directory's lock, so a second agent cannot
    /// start beside it.
    Unusable(String),
    /// The probe could not run in this command's context: the local
    /// deadline's clock is unavailable or changed domain, the socket is
    /// unsafe, or the request was cancelled. None of these shows that no
    /// agent is running, so nothing is started.
    Failed(foks_agent_client::Error),
    /// Connecting found no socket, or a socket nothing listens on.
    Absent,
}

/// Classifies a Hello that got no usable reply. Only a missing or refusing
/// socket shows that no agent is running and authorizes starting one.
fn hello_failure(error: foks_agent_client::Error) -> AgentProbe {
    match error {
        // Something answered the Hello, so a second agent could not take the
        // state directory's lock.
        error @ (foks_agent_client::Error::Protocol(_)
        | foks_agent_client::Error::ResponseBinding) => {
            AgentProbe::Unusable(incompatible_agent_message(&error))
        }
        error if no_listener(&error) => AgentProbe::Absent,
        // A listener accepted the connection but did not answer in time, or
        // closed it unanswered, as an agent at its connection limit does.
        error
            if matches!(error, foks_agent_client::Error::DeadlineExceeded)
                || error.is_connection_loss() =>
        {
            AgentProbe::NotReady(error.to_string())
        }
        error => AgentProbe::Failed(error),
    }
}

fn probe_agent(client: &AgentClient) -> AgentProbe {
    match client.hello(Peer::new("foks-rs", env!("CARGO_PKG_VERSION"))) {
        Ok(_) => {}
        Err(error) => return hello_failure(error),
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
        // AgentStatus runs on the agent's single local worker, which other
        // front ends' requests share; the agent answers Busy when that
        // worker stays occupied.
        Ok(Response {
            result:
                ResponseResult::Error {
                    code: ErrorCode::Busy,
                    message,
                    ..
                },
            ..
        }) => return AgentProbe::NotReady(message),
        Ok(Response {
            result: ResponseResult::Error { message, .. },
            ..
        }) => message,
        Err(error) if error.is_version_mismatch() => {
            return AgentProbe::Unusable(incompatible_agent_message(&error))
        }
        Err(error)
            if matches!(error, foks_agent_client::Error::DeadlineExceeded)
                || error.is_connection_loss() =>
        {
            return AgentProbe::NotReady(error.to_string())
        }
        Err(error) => error.to_string(),
    };
    AgentProbe::Unusable(format!(
        "the running foks-agent did not report its status: {detail}"
    ))
}

/// Whether connecting found no socket, or a socket nothing listens on.
fn no_listener(error: &foks_agent_client::Error) -> bool {
    matches!(
        error,
        foks_agent_client::Error::Io(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            )
    )
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

/// How long a running agent may take to report its status before the CLI
/// gives up on it. Nothing is started or stopped while it is polled.
const RUNNING_AGENT_STATUS_WAIT: Duration = Duration::from_secs(2);

/// How long a started agent may take to report a ready status.
const STARTUP_WAIT: Duration = Duration::from_secs(5);

const PROBE_INTERVAL: Duration = Duration::from_millis(25);

pub(super) fn ensure_agent(state: &Path) -> Result<(), Box<dyn std::error::Error>> {
    ensure_agent_with_deadline(state, &super::deadline::current()?)
}
fn ensure_agent_with_deadline(
    state: &Path,
    original: &foks_local_deadline::Deadline,
) -> Result<(), Box<dyn std::error::Error>> {
    let startup = original.shorten(Instant::now() + Duration::from_secs(5))?;
    let mut client = super::deadline::agent_client(state)?.with_local_deadline(startup.clone())?;
    client.set_timeout(Duration::from_millis(250))?;
    let deadline = (Instant::now() + RUNNING_AGENT_STATUS_WAIT).min(startup.instant());
    loop {
        startup_context(&startup)?;
        match probe_agent(&client) {
            AgentProbe::Ready => return Ok(()),
            AgentProbe::Bootstrap => return Err(UNINITIALIZED_STATE.into()),
            AgentProbe::Unusable(message) => return Err(message.into()),
            AgentProbe::Failed(error) => return Err(error.into()),
            AgentProbe::NotReady(detail) if Instant::now() >= deadline => {
                return Err(format!(
                    "the running foks-agent did not report its status in time: {detail}; retry"
                )
                .into())
            }
            AgentProbe::NotReady(_) => std::thread::sleep(
                PROBE_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
            ),
            AgentProbe::Absent => break,
        }
    }
    // Use the packaged sibling, never a shell command or a caller-selected executable.
    let executable = std::env::current_exe()?.with_file_name("foks-agent");
    startup_context(&startup)?;
    let child = Command::new(executable)
        .arg("--state-dir")
        .arg(state)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "cannot start packaged foks-agent; install it next to foks-rs")?;
    let mut child = Some(child);
    let deadline = (Instant::now() + STARTUP_WAIT).min(startup.instant());
    loop {
        if let Err(error) = startup_context(&startup) {
            stop_child(child);
            return Err(error);
        }
        let detail = match probe_agent(&client) {
            AgentProbe::Ready => {
                if let Some(mut child) = child {
                    // Reap this child if it later exits; it remains resident after MCP EOF.
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                }
                return Ok(());
            }
            // An agent reports bootstrap only while the state is not
            // initialized. Another launcher may also have won the lock with
            // an agent of another version.
            AgentProbe::Bootstrap => {
                stop_child(child);
                return Err(UNINITIALIZED_STATE.into());
            }
            AgentProbe::Unusable(message) => {
                stop_child(child);
                return Err(message.into());
            }
            AgentProbe::Failed(error) => {
                stop_child(child);
                return Err(error.into());
            }
            AgentProbe::NotReady(detail) => Some(detail),
            AgentProbe::Absent if child.is_none() => return Err(UNINITIALIZED_STATE.into()),
            AgentProbe::Absent => None,
        };
        if Instant::now() >= deadline {
            stop_child(child);
            return Err(match detail {
                Some(detail) => format!("authenticated agent startup timed out: {detail}").into(),
                None => "authenticated agent startup timed out".into(),
            });
        }
        if let Some(running) = &mut child {
            if running.try_wait()?.is_some() {
                // Another concurrent launcher may have won the agent's
                // existing lock; its agent is probed from here on.
                child = None;
                continue;
            }
        }
        std::thread::sleep(PROBE_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
    }
}

/// Refuses to continue when the command's deadline context is unusable: its
/// clock domain changed or cannot be read. Expiry is not an error here; each
/// wait ends at the startup deadline with its own diagnosis.
fn startup_context(
    startup: &foks_local_deadline::Deadline,
) -> Result<(), Box<dyn std::error::Error>> {
    match startup.check() {
        Ok(()) | Err(foks_local_deadline::Error::Expired) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn stop_child(child: Option<std::process::Child>) {
    if let Some(mut child) = child {
        let _ = child.kill();
        let _ = child.wait();
    }
}

const UNINITIALIZED_STATE: &str =
    "agent did not start; initialize and unlock the selected state first";

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};
    use std::os::unix::fs::PermissionsExt as _;

    fn scope() -> crate::deadline::CommandScope {
        crate::deadline::CommandScope::enter(Duration::from_secs(15)).unwrap()
    }

    /// Binding gets its full window after a slow startup instead of what
    /// startup left of a shared one.
    #[test]
    fn account_binding_window_starts_after_agent_startup() {
        let _scope = scope();
        let command = crate::deadline::current().unwrap();
        let slow_startup = Duration::from_millis(1500);
        let remaining = start_then_bind(
            &command,
            |startup| {
                assert!(startup.instant() <= Instant::now() + STARTUP_WINDOW);
                std::thread::sleep(slow_startup);
                Ok(())
            },
            |binding| Ok(binding.instant().saturating_duration_since(Instant::now())),
        )
        .unwrap();
        assert!(
            remaining > BIND_WINDOW - Duration::from_millis(500),
            "binding had {remaining:?}"
        );
        assert!(remaining <= BIND_WINDOW);
    }

    #[test]
    fn only_missing_or_refused_endpoints_authorize_agent_startup() {
        use foks_agent_client::Error;
        for kind in [
            std::io::ErrorKind::NotFound,
            std::io::ErrorKind::ConnectionRefused,
        ] {
            assert!(matches!(
                hello_failure(Error::Io(std::io::Error::from(kind))),
                AgentProbe::Absent
            ));
        }
        for error in [
            Error::Cancelled,
            Error::LocalDeadline(foks_local_deadline::Error::DomainMismatch),
            Error::LocalDeadline(foks_local_deadline::Error::Unavailable("clock".into())),
            Error::UnsafeSocket,
        ] {
            assert!(matches!(hello_failure(error), AgentProbe::Failed(_)));
        }
        assert!(matches!(
            hello_failure(Error::Protocol(foks_agent_proto::Error::Version)),
            AgentProbe::Unusable(_)
        ));
        assert!(matches!(
            hello_failure(Error::DeadlineExceeded),
            AgentProbe::NotReady(_)
        ));
    }

    /// An agent at the socket that answers every request with `reply`.
    fn serve_agent(
        socket: &Path,
        reply: fn(Option<u64>) -> serde_json::Value,
    ) -> std::thread::JoinHandle<usize> {
        serve_agent_sequence(socket, Duration::from_millis(500), move |_, id| {
            Answer::Reply(reply(id))
        })
    }

    /// What the test agent does with one request.
    enum Answer {
        Reply(serde_json::Value),
        /// Holds the connection open unanswered until the client closes it.
        Hold,
        /// Closes the connection unanswered.
        Close,
    }

    type Answers = fn(usize, Option<u64>) -> Answer;

    /// An agent at the socket that handles the request at each position as
    /// `answer(position, id)` says.
    fn serve_agent_sequence(
        socket: &Path,
        duration: Duration,
        mut answer: impl FnMut(usize, Option<u64>) -> Answer + Send + 'static,
    ) -> std::thread::JoinHandle<usize> {
        let listener = std::os::unix::net::UnixListener::bind(socket).unwrap();
        std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        listener.set_nonblocking(true).unwrap();
        std::thread::spawn(move || {
            let mut served = 0;
            let deadline = Instant::now() + duration;
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
                let answer = answer(served, foks_agent_proto::request_id(&frame));
                served += 1;
                let reply = match answer {
                    Answer::Reply(reply) => reply,
                    Answer::Hold => {
                        let _ = stream.read(&mut [0; 1]);
                        continue;
                    }
                    Answer::Close => continue,
                };
                stream
                    .write_all(&foks_agent_proto::encode(&reply).unwrap())
                    .unwrap();
            }
            served
        })
    }

    /// A current agent's answer to a Hello.
    fn hello_reply(id: Option<u64>) -> serde_json::Value {
        let reply = foks_agent_proto::HelloReply::new(Peer::new("foks-agent", "test"), &[]);
        serde_json::to_value(foks_agent_proto::Response::success(
            id.unwrap(),
            serde_json::to_value(reply).unwrap(),
        ))
        .unwrap()
    }

    fn ready_status(id: Option<u64>) -> serde_json::Value {
        serde_json::to_value(foks_agent_proto::Response::success(
            id.unwrap(),
            serde_json::to_value(AgentStatus::ready()).unwrap(),
        ))
        .unwrap()
    }

    /// An agent's answer when its local worker stays occupied.
    fn busy_reply(id: Option<u64>) -> serde_json::Value {
        serde_json::to_value(foks_agent_proto::Response::error(
            id.unwrap(),
            ErrorCode::Busy,
            "local worker is busy",
        ))
        .unwrap()
    }

    /// A running agent whose local worker is occupied answers the status
    /// request with Busy or not at all, a loaded agent can miss the Hello's
    /// deadline, and an agent at its connection limit closes a connection
    /// unanswered; the probe repeats instead of diagnosing the agent as
    /// unusable or starting another one.
    #[test]
    fn a_busy_or_slow_agent_is_probed_again() {
        let cases: [(&str, Answers, usize); 4] = [
            (
                "busy status",
                |position, id| match position {
                    0 | 2 => Answer::Reply(hello_reply(id)),
                    1 => Answer::Reply(busy_reply(id)),
                    _ => Answer::Reply(ready_status(id)),
                },
                4,
            ),
            (
                "slow status",
                |position, id| match position {
                    0 | 2 => Answer::Reply(hello_reply(id)),
                    1 => Answer::Hold,
                    _ => Answer::Reply(ready_status(id)),
                },
                4,
            ),
            (
                "slow hello",
                |position, id| match position {
                    0 => Answer::Hold,
                    1 => Answer::Reply(hello_reply(id)),
                    _ => Answer::Reply(ready_status(id)),
                },
                3,
            ),
            (
                "closed hello",
                |position, id| match position {
                    0 => Answer::Close,
                    1 => Answer::Reply(hello_reply(id)),
                    _ => Answer::Reply(ready_status(id)),
                },
                3,
            ),
        ];
        for (case, answers, requests) in cases {
            let directory = tempfile::tempdir().unwrap();
            let server = serve_agent_sequence(
                &directory.path().join("foks-rs.sock"),
                Duration::from_millis(1_000),
                answers,
            );
            // The packaged agent is absent next to the test binary, so a
            // start attempt would fail instead of returning Ok.
            let _scope = scope();
            ensure_agent(directory.path()).unwrap();
            assert_eq!(server.join().unwrap(), requests, "{case}");
        }
    }

    /// An agent that answers every status request with Busy is reported as
    /// busy once the wait ends, without starting a second agent.
    #[test]
    fn a_running_agent_that_stays_busy_is_reported_as_busy() {
        let directory = tempfile::tempdir().unwrap();
        let _server = serve_agent_sequence(
            &directory.path().join("foks-rs.sock"),
            // Outlasts the wait; the server is not joined.
            RUNNING_AGENT_STATUS_WAIT + Duration::from_secs(5),
            |position, id| {
                Answer::Reply(if position % 2 == 0 {
                    hello_reply(id)
                } else {
                    busy_reply(id)
                })
            },
        );
        let _scope = scope();
        let error = ensure_agent(directory.path()).unwrap_err().to_string();
        assert_eq!(
            error,
            "the running foks-agent did not report its status in time: local worker is busy; retry"
        );
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
        let _scope = scope();
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
        let _scope = scope();
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
