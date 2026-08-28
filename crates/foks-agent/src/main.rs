#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use clap::Parser as _;
use foks_agent_proto::{ErrorCode, Operation, Request, Response, MAXIMUM_MESSAGE_BYTES};
use foks_client_app::{
    derive_vault_key, AccountVault, CancellationToken, CheckedProfileSession, ClientCredentials,
    Passphrase, ProfileRegistry, ProfileSession,
};
use foks_keystore::EncryptedFileSecretStore;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use zeroize::Zeroizing;

const MAXIMUM_REQUESTS_PER_CONNECTION: usize = 128;
const CANCELLATION_GRACE: Duration = Duration::from_secs(1);
const MAXIMUM_CANARY_BYTES: usize = 64 * 1024;
const MAXIMUM_CANARY_FETCHES: usize = 4;

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
    #[arg(long, default_value_t = 300)]
    compatibility_poll_seconds: u64,
}

#[cfg(unix)]
fn main() {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("foks-agent-io")
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("foks-agent: {error}");
            std::process::exit(1);
        }
    };
    let result = runtime.block_on(run(Arguments::parse()));
    runtime.shutdown_timeout(CANCELLATION_GRACE);
    if let Err(error) = result {
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
        || arguments.compatibility_poll_seconds == 0
        || arguments.compatibility_poll_seconds > 24 * 60 * 60
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
    let scheduler_gate = Arc::new(Semaphore::new(1));
    let compatibility_gate = Arc::new(Semaphore::new(1));
    let timeout = Duration::from_secs(arguments.request_timeout_seconds);
    let scheduler_cancellation = CancellationToken::new();
    let _scheduler_cancellation_guard = CancelOnDrop(scheduler_cancellation.clone());
    let mut scheduler =
        tokio::time::interval(Duration::from_secs(arguments.scheduler_poll_seconds));
    scheduler.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut compatibility =
        tokio::time::interval(Duration::from_secs(arguments.compatibility_poll_seconds));
    compatibility.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let compatibility_client = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::limited(3))
        .connect_timeout(timeout)
        .timeout(timeout)
        .user_agent("foks-agent/compatibility-lease-v2")
        .build()?;
    eprintln!("FOKS agent ready: {}", socket.display());

    loop {
        tokio::select! {
            signal = tokio::signal::ctrl_c() => {
                signal?;
                scheduler_cancellation.cancel();
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
                let Ok(scheduler_permit) = scheduler_gate.clone().try_acquire_owned() else {
                    continue;
                };
                let Ok(permit) = blocking.clone().try_acquire_owned() else {
                    continue;
                };
                let state = state_dir.clone();
                let cancellation = scheduler_cancellation.clone();
                tokio::task::spawn_blocking(move || {
                    let _scheduler_permit = scheduler_permit;
                    let _permit = permit;
                    run_scheduled_profiles(&state, timeout, cancellation);
                });
            }
            _ = compatibility.tick() => {
                let Ok(compatibility_permit) = compatibility_gate.clone().try_acquire_owned() else {
                    continue;
                };
                let state = state_dir.clone();
                let client = compatibility_client.clone();
                let cancellation = scheduler_cancellation.clone();
                tokio::spawn(async move {
                    let _compatibility_permit = compatibility_permit;
                    refresh_hosted_profiles(&state, client, cancellation).await;
                });
            }
        }
    }
    Ok(())
}

fn run_scheduled_profiles(state_dir: &Path, timeout: Duration, cancellation: CancellationToken) {
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
        if cancellation.is_cancelled() {
            break;
        }
        if let Err(error) =
            run_scheduled_profile(state_dir, &profile, timeout, cancellation.clone())
        {
            eprintln!("foks-agent scheduled refresh failed: {error}");
        }
    }
}

fn run_scheduled_profile(
    state_dir: &Path,
    profile: &str,
    timeout: Duration,
    cancellation: CancellationToken,
) -> Result<(), Box<dyn std::error::Error>> {
    let registry = ProfileRegistry::open(state_dir)?;
    let session = ProfileSession::open_with_control(&registry, profile, timeout, cancellation)?;
    let _ = try_with_vault(state_dir, &session, |session, vault| {
        Ok(session.try_run_due_jobs(now_microseconds()?, vault)?)
    })?;
    Ok(())
}

async fn refresh_hosted_profiles(
    state_dir: &Path,
    client: reqwest::Client,
    cancellation: CancellationToken,
) {
    let registry = match ProfileRegistry::open(state_dir) {
        Ok(registry) => registry,
        Err(_) => {
            eprintln!("foks-agent compatibility refresh could not open profiles");
            return;
        }
    };
    let profiles = registry
        .profiles()
        .filter_map(|profile| {
            profile
                .compatibility_lease_url()
                .map(|url| (profile.name.clone(), url.to_owned()))
        })
        .collect::<Vec<_>>();
    drop(registry);

    let mut profiles = profiles.into_iter();
    let mut fetches = tokio::task::JoinSet::new();
    for _ in 0..MAXIMUM_CANARY_FETCHES {
        let Some(profile) = profiles.next() else {
            break;
        };
        spawn_canary_fetch(&mut fetches, client.clone(), profile);
    }
    while let Some(fetched) = fetches.join_next().await {
        if cancellation.is_cancelled() {
            fetches.abort_all();
            break;
        }
        match fetched {
            Ok((profile, Ok(bytes))) => match apply_hosted_lease(state_dir, &profile, &bytes) {
                Ok(true) => {
                    eprintln!("foks-agent applied a newer compatibility lease for {profile}");
                }
                Ok(false) => {}
                Err(_) => {
                    eprintln!("foks-agent rejected the compatibility lease for {profile}");
                }
            },
            Ok((profile, Err(error))) => {
                eprintln!("foks-agent compatibility lease fetch failed for {profile}: {error}");
            }
            Err(_) => eprintln!("foks-agent compatibility lease fetch task failed"),
        }
        if let Some(profile) = profiles.next() {
            spawn_canary_fetch(&mut fetches, client.clone(), profile);
        }
    }
}

fn spawn_canary_fetch(
    fetches: &mut tokio::task::JoinSet<(String, Result<Vec<u8>, LeaseFetchError>)>,
    client: reqwest::Client,
    (profile, url): (String, String),
) {
    fetches.spawn(async move {
        let result = fetch_canary(&client, &url).await;
        (profile, result)
    });
}

#[derive(Clone, Copy, Debug)]
enum LeaseFetchError {
    Transport,
    Status(u16),
    TooLarge,
}

impl std::fmt::Display for LeaseFetchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport => formatter.write_str("transport error"),
            Self::Status(status) => write!(formatter, "HTTP status {status}"),
            Self::TooLarge => formatter.write_str("response exceeds 64 KiB"),
        }
    }
}

async fn fetch_canary(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, LeaseFetchError> {
    let mut response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|_| LeaseFetchError::Transport)?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(LeaseFetchError::Status(response.status().as_u16()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAXIMUM_CANARY_BYTES as u64)
    {
        return Err(LeaseFetchError::TooLarge);
    }
    let mut bytes = Vec::with_capacity(
        response
            .content_length()
            .unwrap_or(0)
            .min(MAXIMUM_CANARY_BYTES as u64) as usize,
    );
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| LeaseFetchError::Transport)?
    {
        append_canary_chunk(&mut bytes, &chunk)?;
    }
    Ok(bytes)
}

fn append_canary_chunk(bytes: &mut Vec<u8>, chunk: &[u8]) -> Result<(), LeaseFetchError> {
    if bytes
        .len()
        .checked_add(chunk.len())
        .is_none_or(|length| length > MAXIMUM_CANARY_BYTES)
    {
        return Err(LeaseFetchError::TooLarge);
    }
    bytes.extend_from_slice(chunk);
    Ok(())
}

fn apply_hosted_lease(
    state_dir: &Path,
    profile: &str,
    bytes: &[u8],
) -> Result<bool, Box<dyn std::error::Error>> {
    if bytes.len() > MAXIMUM_CANARY_BYTES {
        return Err("compatibility lease exceeds 64 KiB".into());
    }
    let signed: foks_compat_artifact::SignedCanaryArtifact = serde_json::from_slice(bytes)?;
    let mut registry = ProfileRegistry::open(state_dir)?;
    let previous = registry.profile(profile)?.clone();
    let current = registry.apply_canary(profile, &signed, now_microseconds()? / 1_000_000)?;
    Ok(current != previous)
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
        let state = state_dir.clone();
        let supervised = supervise_blocking(request.id, permit, timeout, move |cancellation| {
            dispatch_controlled(&state, request, timeout, cancellation)
        })
        .await;
        write_response(&mut stream, &supervised.response, timeout).await?;
        if supervised.close_connection {
            return Ok(());
        }
    }
    Ok(())
}

struct SupervisedResponse {
    response: Response,
    close_connection: bool,
}

async fn supervise_blocking(
    request_id: u64,
    permit: OwnedSemaphorePermit,
    timeout: Duration,
    operation: impl FnOnce(CancellationToken) -> Response + Send + 'static,
) -> SupervisedResponse {
    let cancellation = CancellationToken::new();
    let _cancel_on_drop = CancelOnDrop(cancellation.clone());
    let worker_cancellation = cancellation.clone();
    let mut task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        operation(worker_cancellation)
    });
    match tokio::time::timeout(timeout, &mut task).await {
        Ok(Ok(response)) => SupervisedResponse {
            response,
            close_connection: false,
        },
        Ok(Err(error)) => SupervisedResponse {
            response: Response::error(
                request_id,
                ErrorCode::OperationFailed,
                format!("agent worker failed: {error}"),
            ),
            close_connection: true,
        },
        Err(_) => {
            cancellation.cancel();
            // Blocking work cannot be forcibly killed safely. Give network
            // operations one poll interval to observe cancellation; if other
            // synchronous work remains stuck, its permit keeps the pool bound.
            let _ = tokio::time::timeout(CANCELLATION_GRACE, &mut task).await;
            SupervisedResponse {
                response: Response::error(
                    request_id,
                    ErrorCode::DeadlineExceeded,
                    "agent operation deadline exceeded; connection closed because completion is ambiguous",
                ),
                close_connection: true,
            }
        }
    }
}

struct CancelOnDrop(CancellationToken);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

fn dispatch_controlled(
    state_dir: &Path,
    request: Request,
    timeout: Duration,
    cancellation: CancellationToken,
) -> Response {
    let id = request.id;
    let result = dispatch_result(state_dir, request.operation, timeout, cancellation);
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
    timeout: Duration,
    cancellation: CancellationToken,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let registry = ProfileRegistry::open(state_dir)?;
    match operation {
        Operation::Ping => Ok(serde_json::json!({ "ready": true })),
        Operation::ListProfiles => Ok(serde_json::to_value(
            registry.profiles().cloned().collect::<Vec<_>>(),
        )?),
        Operation::Probe { profile } => {
            let credentials = ClientCredentials::open(state_dir)?;
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            let report = credentials.with_checked_session(&session, |session| {
                session
                    .probe_and_pin()
                    .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)
            })?;
            Ok(serde_json::to_value(report)?)
        }
        Operation::ListAccounts { profile } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |_session, vault| {
                Ok(serde_json::to_value(vault.aliases()?)?)
            })
        }
        Operation::CreateAccount {
            profile,
            alias,
            username,
            device_name,
            email,
            invite,
            passphrase,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            let credentials = ClientCredentials::open(state_dir)?;
            credentials.with_checked_session(&session, |session| {
                let master = credentials.master_key()?;
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    derive_vault_key(&master),
                )?;
                let mut vault = AccountVault::new(&mut store);
                let passphrase = passphrase
                    .as_ref()
                    .map(|passphrase| Passphrase::new(passphrase.expose()))
                    .transpose()?;
                Ok(serde_json::to_value(session.create_account(
                    &alias,
                    &username,
                    &device_name,
                    &email,
                    &invite,
                    passphrase,
                    &mut vault,
                    &master,
                )?)?)
            })
        }
        Operation::SetPassphrase {
            profile,
            alias,
            passphrase,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(session.set_passphrase(
                    &alias,
                    Passphrase::new(passphrase.expose())?,
                    vault,
                )?)?)
            })
        }
        Operation::ChangePassphrase {
            profile,
            alias,
            passphrase,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(session.change_passphrase(
                    &alias,
                    Passphrase::new(passphrase.expose())?,
                    vault,
                )?)?)
            })
        }
        Operation::VerifyPassphrase {
            profile,
            alias,
            passphrase,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(session.verify_passphrase(
                    &alias,
                    Passphrase::new(passphrase.expose())?,
                    vault,
                )?)?)
            })
        }
        Operation::SyncAccount { profile, alias } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(session.sync_account(&alias, vault)?)?)
            })
        }
        Operation::ListKv { profile, alias } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(session.list_kv(&alias, vault)?)?)
            })
        }
        Operation::ListTeams { profile } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(session.list_teams(vault)?)?)
            })
        }
        Operation::SyncTeam {
            profile,
            team_alias,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(
                    session.sync_team(&team_alias, vault)?,
                )?)
            })
        }
        Operation::RunDueJobs { profile } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
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
        &CheckedProfileSession<'_>,
        &mut AccountVault<'_>,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error>>,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let credentials = ClientCredentials::open(state_dir)?;
    credentials.with_checked_session(session, |session| {
        let master = credentials.master_key()?;
        let mut store = EncryptedFileSecretStore::open(
            &session.paths().credential_store,
            derive_vault_key(&master),
        )?;
        operation(session, &mut AccountVault::new(&mut store))
    })
}

fn try_with_vault<T>(
    state_dir: &Path,
    session: &ProfileSession,
    operation: impl FnOnce(
        &CheckedProfileSession<'_>,
        &mut AccountVault<'_>,
    ) -> Result<T, Box<dyn std::error::Error>>,
) -> Result<Option<T>, Box<dyn std::error::Error>> {
    let credentials = ClientCredentials::open(state_dir)?;
    credentials.try_with_checked_session(session, |session| {
        let master = credentials.master_key()?;
        let mut store = EncryptedFileSecretStore::open(
            &session.paths().credential_store,
            derive_vault_key(&master),
        )?;
        operation(session, &mut AccountVault::new(&mut store))
    })
}

#[cfg(test)]
fn dispatch(state_dir: &Path, request: Request) -> Response {
    dispatch_controlled(
        state_dir,
        request,
        Duration::from_secs(15),
        CancellationToken::new(),
    )
}

#[cfg(unix)]
async fn read_frame(
    stream: &mut tokio::net::UnixStream,
) -> Result<Option<Zeroizing<Vec<u8>>>, Box<dyn std::error::Error + Send + Sync>> {
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
    let mut frame = Zeroizing::new(Vec::with_capacity(4 + length));
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
    fn agent_signup_carries_invites_and_passphrases_across_the_product_boundary() {
        use foks_client_app::{
            ClientCredentials, CredentialBackend, Profile, ProtocolPolicy, TrustRoot,
        };
        use foks_server_db::InviteRegime;
        use foks_server_testkit::TestEnvironment;

        let environment = TestEnvironment::new().unwrap();
        let _server = environment.start_server().unwrap();
        environment
            .set_invite_regime(InviteRegime::Required)
            .unwrap();
        let invite = environment.issue_standard_invite(None).unwrap();
        let state = environment.client_path("agent-product", "state").unwrap();
        let root = environment
            .client_path("agent-product", "probe-root.der")
            .unwrap();
        environment.write_probe_root(&root).unwrap();
        ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
        let addresses = environment.addresses().unwrap();
        let mut registry = ProfileRegistry::open(&state).unwrap();
        registry
            .add(Profile {
                name: "local".to_owned(),
                probe: format!("localhost:{}", addresses.probe.port()),
                protocol: ProtocolPolicy::V019,
                trust: TrustRoot::CertificateDer { path: root },
            })
            .unwrap();

        let probed = dispatch(
            &state,
            Request::new(
                9,
                Operation::Probe {
                    profile: "local".to_owned(),
                },
            ),
        );
        assert!(
            matches!(
                probed.result,
                foks_agent_proto::ResponseResult::Success { .. }
            ),
            "unexpected probe response: {probed:?}"
        );

        let response = dispatch(
            &state,
            Request::new(
                10,
                Operation::CreateAccount {
                    profile: "local".to_owned(),
                    alias: "personal".to_owned(),
                    username: "agentinvite".to_owned(),
                    device_name: "agent laptop".to_owned(),
                    email: "agent@example.test".to_owned(),
                    invite: invite.code.clone(),
                    passphrase: Some(foks_agent_proto::SecretString::new("agent passphrase one")),
                },
            ),
        );
        assert!(
            matches!(
                response.result,
                foks_agent_proto::ResponseResult::Success { .. }
            ),
            "unexpected signup response: {response:?}"
        );
        let listed = dispatch(
            &state,
            Request::new(
                11,
                Operation::ListAccounts {
                    profile: "local".to_owned(),
                },
            ),
        );
        assert!(matches!(
            listed.result,
            foks_agent_proto::ResponseResult::Success { value }
                if value == serde_json::json!(["personal"])
        ));

        let duplicate_set = dispatch(
            &state,
            Request::new(
                12,
                Operation::SetPassphrase {
                    profile: "local".to_owned(),
                    alias: "personal".to_owned(),
                    passphrase: foks_agent_proto::SecretString::new("unexpected replacement"),
                },
            ),
        );
        assert!(matches!(
            duplicate_set.result,
            foks_agent_proto::ResponseResult::Error { .. }
        ));

        for (id, operation) in [
            (
                13,
                Operation::VerifyPassphrase {
                    profile: "local".to_owned(),
                    alias: "personal".to_owned(),
                    passphrase: foks_agent_proto::SecretString::new("agent passphrase one"),
                },
            ),
            (
                14,
                Operation::ChangePassphrase {
                    profile: "local".to_owned(),
                    alias: "personal".to_owned(),
                    passphrase: foks_agent_proto::SecretString::new("agent passphrase two"),
                },
            ),
            (
                15,
                Operation::VerifyPassphrase {
                    profile: "local".to_owned(),
                    alias: "personal".to_owned(),
                    passphrase: foks_agent_proto::SecretString::new("agent passphrase two"),
                },
            ),
        ] {
            let response = dispatch(&state, Request::new(id, operation));
            assert!(
                matches!(
                    response.result,
                    foks_agent_proto::ResponseResult::Success { .. }
                ),
                "unexpected passphrase response: {response:?}"
            );
        }

        let records = environment.invites().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].use_count, 1);
    }

    #[test]
    fn error_messages_are_utf8_bounded() {
        let error = format!("{}é", "x".repeat(2048));
        let bounded = bounded_error(error);
        assert!(bounded.len() <= 1024);
        assert!(bounded.is_char_boundary(bounded.len()));
    }

    #[test]
    fn compatibility_response_limit_is_enforced_incrementally() {
        let mut bytes = vec![0; MAXIMUM_CANARY_BYTES - 1];
        append_canary_chunk(&mut bytes, &[1]).unwrap();
        assert_eq!(bytes.len(), MAXIMUM_CANARY_BYTES);
        assert!(matches!(
            append_canary_chunk(&mut bytes, &[2]),
            Err(LeaseFetchError::TooLarge)
        ));
    }

    #[test]
    fn downloaded_lease_grants_then_drift_revokes_atomically() {
        use std::collections::BTreeSet;

        let seed = [
            0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec,
            0x2c, 0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03,
            0x1c, 0xae, 0x7f, 0x60,
        ];
        let directory = tempfile::tempdir().unwrap();
        let mut registry = ProfileRegistry::open(directory.path()).unwrap();
        registry
            .add(foks_client_app::Profile {
                name: "hosted".to_owned(),
                probe: "foks.app".to_owned(),
                protocol: foks_client_app::ProtocolPolicy::CurrentProbeOnly {
                    canary_public_key:
                        "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
                            .to_owned(),
                    lease_url: "https://updates.example.test/lease.json".to_owned(),
                    last_artifact: None,
                },
                trust: foks_client_app::TrustRoot::WebPki,
            })
            .unwrap();
        drop(registry);
        let now = now_microseconds().unwrap() / 1_000_000;
        let mut artifact = foks_compat_artifact::CanaryArtifact {
            schema_version: foks_compat_artifact::SCHEMA_VERSION,
            generation: 1,
            target: "foks.app".to_owned(),
            run_id: "agent-test-1".to_owned(),
            generated_at: now,
            expires_at: now + 60,
            protocol_metadata_sha256: foks_client_app::PINNED_PROTOCOL_METADATA_SHA256.to_owned(),
            mutation_digest: "11".repeat(32),
            read_digest: "11".repeat(32),
            outcome: foks_compat_artifact::Outcome::Compatible,
            capabilities: BTreeSet::from(["kv".to_owned()]),
            drift_reason: String::new(),
        };
        let compatible =
            foks_compat_artifact::SignedCanaryArtifact::sign(artifact.clone(), &seed).unwrap();
        let bytes = serde_json::to_vec(&compatible).unwrap();
        assert!(apply_hosted_lease(directory.path(), "hosted", &bytes).unwrap());
        assert!(!apply_hosted_lease(directory.path(), "hosted", &bytes).unwrap());
        assert!(ProfileRegistry::open(directory.path())
            .unwrap()
            .profile("hosted")
            .unwrap()
            .require_at(foks_client_app::Capability::Kv, now)
            .is_ok());

        artifact.generation = 2;
        artifact.run_id = "agent-test-2".to_owned();
        artifact.outcome = foks_compat_artifact::Outcome::Drift;
        artifact.capabilities.clear();
        artifact.drift_reason = "read-back mismatch".to_owned();
        let drift = foks_compat_artifact::SignedCanaryArtifact::sign(artifact, &seed).unwrap();
        assert!(apply_hosted_lease(
            directory.path(),
            "hosted",
            &serde_json::to_vec(&drift).unwrap(),
        )
        .unwrap());
        assert!(ProfileRegistry::open(directory.path())
            .unwrap()
            .profile("hosted")
            .unwrap()
            .require_at(foks_client_app::Capability::Kv, now)
            .is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn timed_out_workers_observe_cancellation_and_release_capacity() {
        let workers = Arc::new(Semaphore::new(1));
        let permit = workers.clone().acquire_owned().await.unwrap();
        let observed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_observed = Arc::clone(&observed);
        let result =
            supervise_blocking(7, permit, Duration::from_millis(20), move |cancellation| {
                while !cancellation.is_cancelled() {
                    std::thread::sleep(Duration::from_millis(1));
                }
                worker_observed.store(true, std::sync::atomic::Ordering::Release);
                Response::success(7, serde_json::json!({ "late": true }))
            })
            .await;

        assert!(result.close_connection);
        assert!(matches!(
            result.response.result,
            foks_agent_proto::ResponseResult::Error {
                code: ErrorCode::DeadlineExceeded,
                ..
            }
        ));
        assert!(observed.load(std::sync::atomic::Ordering::Acquire));
        assert_eq!(workers.available_permits(), 1);
    }
}
