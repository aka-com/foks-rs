#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use clap::Parser as _;
use foks_agent_proto::{ErrorCode, Operation, Request, Response, MAXIMUM_MESSAGE_BYTES};
use foks_client_app::{
    derive_vault_key, AccountVault, ClientCredentials, ProfileRegistry, ProfileSession,
};
use foks_keystore::EncryptedFileSecretStore;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const MAXIMUM_REQUESTS_PER_CONNECTION: usize = 128;

#[derive(clap::Parser)]
#[command(
    name = "foks-agent",
    about = "Bounded local agent for standalone Rust FOKS clients"
)]
struct Arguments {
    /// Explicit standalone FOKS state root.
    #[arg(long)]
    state_dir: PathBuf,
    /// Unix socket path; defaults inside the explicit state root.
    #[arg(long)]
    socket: Option<PathBuf>,
    #[arg(long, default_value_t = 32)]
    maximum_connections: usize,
    #[arg(long, default_value_t = 4)]
    blocking_workers: usize,
    #[arg(long, default_value_t = 15)]
    request_timeout_seconds: u64,
    #[arg(long, default_value_t = 30)]
    scheduler_poll_seconds: u64,
}

#[cfg(unix)]
#[tokio::main]
async fn main() {
    if let Err(error) = run(Arguments::parse()).await {
        eprintln!("foks-agent: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(unix))]
fn main() {
    let _ = Arguments::parse();
    eprintln!("foks-agent: this build does not yet provide authenticated local IPC on Windows");
    std::process::exit(1);
}

#[cfg(unix)]
async fn run(arguments: Arguments) -> Result<(), Box<dyn std::error::Error>> {
    if arguments.maximum_connections == 0
        || arguments.blocking_workers == 0
        || arguments.request_timeout_seconds == 0
        || arguments.scheduler_poll_seconds == 0
        || arguments.maximum_connections > 4096
        || arguments.blocking_workers > 256
    {
        return Err("agent limits are outside supported bounds".into());
    }
    let registry = ProfileRegistry::open(&arguments.state_dir)?;
    drop(registry);
    let state_dir = arguments.state_dir.canonicalize()?;
    ClientCredentials::open(&state_dir)?.master_key()?;
    let socket = arguments
        .socket
        .unwrap_or_else(|| state_dir.join("agent.sock"));
    if socket.exists() {
        return Err("agent socket path already exists; refusing to replace it".into());
    }
    let parent = socket.parent().ok_or("agent socket has no parent")?;
    if !parent.canonicalize()?.starts_with(&state_dir) {
        return Err("agent socket must be below the explicit state root".into());
    }
    let listener = tokio::net::UnixListener::bind(&socket)?;
    set_socket_permissions(&socket)?;
    let _socket_guard = SocketGuard(socket.clone());
    let active = Arc::new(Semaphore::new(arguments.maximum_connections));
    let blocking = Arc::new(Semaphore::new(arguments.blocking_workers));
    let timeout = Duration::from_secs(arguments.request_timeout_seconds);
    let mut scheduler =
        tokio::time::interval(Duration::from_secs(arguments.scheduler_poll_seconds));
    scheduler.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    eprintln!("FOKS agent ready: {}", socket.display());

    loop {
        tokio::select! {
            signal = tokio::signal::ctrl_c() => {
                signal?;
                break;
            }
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let Ok(permit) = active.clone().try_acquire_owned() else {
                    drop(stream);
                    continue;
                };
                let state_dir = state_dir.clone();
                let blocking = blocking.clone();
                tokio::spawn(async move {
                    if let Err(error) = handle_connection(
                        stream,
                        state_dir,
                        blocking,
                        timeout,
                        permit,
                    ).await {
                        eprintln!("foks-agent connection failed: {error}");
                    }
                });
            }
            _ = scheduler.tick() => {
                let Ok(permit) = blocking.clone().try_acquire_owned() else {
                    continue;
                };
                let state = state_dir.clone();
                tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    run_scheduled_profiles(&state);
                });
            }
        }
    }
    Ok(())
}

fn run_scheduled_profiles(state_dir: &Path) {
    let registry = match ProfileRegistry::open(state_dir) {
        Ok(registry) => registry,
        Err(error) => {
            eprintln!("foks-agent scheduler could not open profiles: {error}");
            return;
        }
    };
    let profiles = registry
        .profiles()
        .filter(|profile| {
            profile
                .require(foks_client_app::Capability::UserSync)
                .is_ok()
        })
        .map(|profile| profile.name.clone())
        .collect::<Vec<_>>();
    drop(registry);
    for profile in profiles {
        let response = dispatch(
            state_dir,
            Request::new(0, Operation::RunDueJobs { profile }),
        );
        if let foks_agent_proto::ResponseResult::Error { message, .. } = response.result {
            eprintln!("foks-agent scheduled refresh failed: {message}");
        }
    }
}

#[cfg(unix)]
async fn handle_connection(
    mut stream: tokio::net::UnixStream,
    state_dir: PathBuf,
    blocking: Arc<Semaphore>,
    timeout: Duration,
    _active_permit: OwnedSemaphorePermit,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    for _ in 0..MAXIMUM_REQUESTS_PER_CONNECTION {
        let frame = match tokio::time::timeout(timeout, read_frame(&mut stream)).await {
            Ok(Ok(Some(frame))) => frame,
            Ok(Ok(None)) => return Ok(()),
            Ok(Err(error)) => return Err(error),
            Err(_) => return Err("agent read deadline exceeded".into()),
        };
        let request = match foks_agent_proto::decode_request(&frame) {
            Ok(request) => request,
            Err(foks_agent_proto::Error::Version) => {
                write_response(
                    &mut stream,
                    &Response::error(
                        0,
                        ErrorCode::VersionMismatch,
                        "unsupported protocol version",
                    ),
                    timeout,
                )
                .await?;
                continue;
            }
            Err(error) => {
                write_response(
                    &mut stream,
                    &Response::error(0, ErrorCode::InvalidRequest, error.to_string()),
                    timeout,
                )
                .await?;
                continue;
            }
        };
        let permit = match tokio::time::timeout(timeout, blocking.clone().acquire_owned()).await {
            Ok(Ok(permit)) => permit,
            Ok(Err(_)) => return Err("agent worker pool closed".into()),
            Err(_) => {
                write_response(
                    &mut stream,
                    &Response::error(
                        request.id,
                        ErrorCode::Busy,
                        "agent worker pool is saturated",
                    ),
                    timeout,
                )
                .await?;
                continue;
            }
        };
        let request_id = request.id;
        let state = state_dir.clone();
        let task = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            dispatch(&state, request)
        });
        let response = match tokio::time::timeout(timeout, task).await {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => Response::error(
                request_id,
                ErrorCode::OperationFailed,
                format!("agent worker failed: {error}"),
            ),
            Err(_) => Response::error(
                request_id,
                ErrorCode::DeadlineExceeded,
                "agent operation deadline exceeded",
            ),
        };
        write_response(&mut stream, &response, timeout).await?;
    }
    Ok(())
}

fn dispatch(state_dir: &Path, request: Request) -> Response {
    let id = request.id;
    let result = dispatch_result(state_dir, request.operation);
    match result {
        Ok(value) => Response::success(id, value),
        Err(error) => Response::error(
            id,
            ErrorCode::OperationFailed,
            bounded_error(error.to_string()),
        ),
    }
}

fn dispatch_result(
    state_dir: &Path,
    operation: Operation,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let registry = ProfileRegistry::open(state_dir)?;
    match operation {
        Operation::Ping => Ok(serde_json::json!({ "ready": true })),
        Operation::ListProfiles => Ok(serde_json::to_value(
            registry.profiles().cloned().collect::<Vec<_>>(),
        )?),
        Operation::Probe { profile } => {
            let credentials = ClientCredentials::open(state_dir)?;
            let session = ProfileSession::open(&registry, &profile)?;
            let report = credentials.with_checkpoint(&session, || {
                session
                    .probe_and_pin()
                    .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)
            })?;
            Ok(serde_json::to_value(report)?)
        }
        Operation::ListAccounts { profile } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |vault| {
                Ok(serde_json::to_value(vault.aliases()?)?)
            })
        }
        Operation::SyncAccount { profile, alias } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |vault| {
                Ok(serde_json::to_value(session.sync_account(&alias, vault)?)?)
            })
        }
        Operation::ListKv { profile, alias } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |vault| {
                Ok(serde_json::to_value(session.list_kv(&alias, vault)?)?)
            })
        }
        Operation::ListTeams { profile } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |vault| {
                Ok(serde_json::to_value(session.list_teams(vault)?)?)
            })
        }
        Operation::SyncTeam {
            profile,
            team_alias,
        } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |vault| {
                Ok(serde_json::to_value(
                    session.sync_team(&team_alias, vault)?,
                )?)
            })
        }
        Operation::RunDueJobs { profile } => {
            let session = ProfileSession::open(&registry, &profile)?;
            with_vault(state_dir, &session, |vault| {
                Ok(serde_json::to_value(
                    session.run_due_jobs(now_microseconds()?, vault)?,
                )?)
            })
        }
    }
}

fn with_vault(
    state_dir: &Path,
    session: &ProfileSession,
    operation: impl FnOnce(
        &mut AccountVault<'_>,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error>>,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let credentials = ClientCredentials::open(state_dir)?;
    credentials.with_checkpoint(session, || {
        let master = credentials.master_key()?;
        let mut store = EncryptedFileSecretStore::open(
            &session.paths().credential_store,
            derive_vault_key(&master),
        )?;
        operation(&mut AccountVault::new(&mut store))
    })
}

#[cfg(unix)]
async fn read_frame(
    stream: &mut tokio::net::UnixStream,
) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
    let mut prefix = [0u8; 4];
    match stream.read_exact(&mut prefix).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let length = u32::from_be_bytes(prefix) as usize;
    if length > MAXIMUM_MESSAGE_BYTES {
        return Err("agent frame exceeds the message limit".into());
    }
    let mut frame = Vec::with_capacity(4 + length);
    frame.extend_from_slice(&prefix);
    frame.resize(4 + length, 0);
    stream.read_exact(&mut frame[4..]).await?;
    Ok(Some(frame))
}

#[cfg(unix)]
async fn write_response(
    stream: &mut tokio::net::UnixStream,
    response: &Response,
    timeout: Duration,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let frame = foks_agent_proto::encode(response)?;
    tokio::time::timeout(timeout, stream.write_all(&frame))
        .await
        .map_err(|_| "agent write deadline exceeded")??;
    Ok(())
}

fn bounded_error(mut error: String) -> String {
    if error.len() <= 1024 {
        return error;
    }
    let mut boundary = 1024;
    while !error.is_char_boundary(boundary) {
        boundary -= 1;
    }
    error.truncate(boundary);
    error
}

fn now_microseconds() -> Result<u64, Box<dyn std::error::Error>> {
    use std::time::{SystemTime, UNIX_EPOCH};

    Ok(u64::try_from(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros(),
    )?)
}

#[cfg(unix)]
fn set_socket_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(unix)]
struct SocketGuard(PathBuf);

#[cfg(unix)]
impl Drop for SocketGuard {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.0) {
            eprintln!("foks-agent could not remove socket: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_and_profile_listing_are_isolated_to_explicit_state() {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join("state");
        ProfileRegistry::open(&state).unwrap();
        let ping = dispatch(&state, Request::new(1, Operation::Ping));
        assert!(matches!(
            ping.result,
            foks_agent_proto::ResponseResult::Success { .. }
        ));
        let profiles = dispatch(&state, Request::new(2, Operation::ListProfiles));
        assert!(matches!(
            profiles.result,
            foks_agent_proto::ResponseResult::Success { .. }
        ));
    }

    #[test]
    fn error_messages_are_utf8_bounded() {
        let error = format!("{}é", "x".repeat(2048));
        let bounded = bounded_error(error);
        assert!(bounded.len() <= 1024);
        assert!(bounded.is_char_boundary(bounded.len()));
    }
}
