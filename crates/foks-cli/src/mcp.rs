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
    let available = || -> Result<bool, Box<dyn std::error::Error>> {
        startup.check()?;
        let response = client.call(Operation::Ping);
        startup.check()?;
        startup_available(response)
    };
    if available()? {
        return Ok(());
    }
    // Use the packaged sibling, never a shell command or a caller-selected executable.
    let executable = std::env::current_exe()?.with_file_name("foks-agent");
    startup.check()?;
    let mut child = Command::new(executable)
        .arg("--state-dir")
        .arg(state)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "cannot start packaged foks-agent; install it next to foks-rs")?;
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        loop {
            if available()? {
                return Ok(());
            }
            if child.try_wait()?.is_some() {
                if available()? {
                    return Ok(());
                }
                return Err(
                    "agent did not start; initialize and unlock the selected state first".into(),
                );
            }
            std::thread::sleep(
                Duration::from_millis(25)
                    .min(startup.instant().saturating_duration_since(Instant::now())),
            );
        }
    })();
    if result.is_ok() {
        // Retain process ownership while it remains resident after MCP EOF.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    } else {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

fn startup_available(
    response: Result<foks_agent_proto::Response, foks_agent_client::Error>,
) -> Result<bool, Box<dyn std::error::Error>> {
    match response {
        Ok(response) => match response.result {
            ResponseResult::Success { .. } => Ok(true),
            ResponseResult::Error { code, message, .. } => {
                Err(format!("agent startup refused: {code:?}: {message}").into())
            }
        },
        Err(foks_agent_client::Error::Io(error))
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            Ok(false)
        }
        // Protocol/domain/permission/deadline errors describe an existing
        // endpoint or unusable caller context, not permission to replace it.
        Err(error) => Err(error.into()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;

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
            assert!(!startup_available(Err(Error::Io(std::io::Error::from(kind)))).unwrap());
        }
        for error in [
            Error::DeadlineExceeded,
            Error::Cancelled,
            Error::LocalDeadline(foks_local_deadline::Error::DomainMismatch),
            Error::LocalDeadline(foks_local_deadline::Error::Unavailable("clock".into())),
            Error::Protocol(foks_agent_proto::Error::Version),
            Error::UnsafeSocket,
        ] {
            assert!(startup_available(Err(error)).is_err());
        }
        for code in [
            foks_agent_proto::ErrorCode::VersionMismatch,
            foks_agent_proto::ErrorCode::DeadlineClockUnavailable,
            foks_agent_proto::ErrorCode::DeadlineDomainMismatch,
            foks_agent_proto::ErrorCode::BootstrapRequired,
        ] {
            assert!(startup_available(Ok(foks_agent_proto::Response::error(
                1,
                code,
                "test rejection"
            )))
            .is_err());
        }
    }
}
