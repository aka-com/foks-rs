#![forbid(unsafe_code)]

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use clap::Parser as _;
use foks_agent_proto::{
    AccountStoreRef, AccountSummary, AgentStatus,
    BackupEnrollmentSummary as WireBackupEnrollmentSummary,
    CredentialBackend as WireCredentialBackend, DeviceSummary as WireDeviceSummary, ErrorCode,
    ErrorFields, GoProfileCandidate as WireGoProfileCandidate,
    GoProfileDiscovery as WireGoProfileDiscovery, KnownStoreSummary as WireKnownStoreSummary,
    KvChunkResult, KvEntryMetadata, KvPage, KvPrecondition, KvReadResult, KvRole, KvStoreRef,
    KvUploadHeader, Operation, PendingOperationKind as WirePendingOperationKind,
    PendingOperationSummary as WirePendingOperationSummary, ProfileProtocol, ProfileTrust, Request,
    ResetArtifactKind as WireResetArtifactKind, ResetArtifactSummary as WireResetArtifactSummary,
    ResetStatePreview as WireResetStatePreview, Response, ResponseResult,
    ServerStatusSnapshot as WireServerStatusSnapshot, StoredHostStatus as WireStoredHostStatus,
    TeamDetailsSummary, TeamKind, TeamRole, TeamStoreRef, MAXIMUM_MESSAGE_BYTES,
};
use foks_client_app::{
    derive_vault_key, AccountVault, CancellationToken, Capability, CheckedProfileSession,
    ClientCredentials, CredentialBackend, FederationDestinationRole, KexAcceptanceInput,
    KvMutationPrecondition, KvRoleSummary, Passphrase, Profile, ProfileRegistry, ProfileSession,
    ProtocolPolicy, TeamMemberRole, TrustRoot, UnlockedYubiActor, YubiProvisionInput,
    YubiSignupInput,
};
use foks_client_db::{KnownStore, KnownTeamStore, SoftStateStore};
use foks_keystore::EncryptedFileSecretStore;
use foks_yubi::{
    CardId, HardwareYubiProvider, Pin, PinRetryConfiguration, SlotId, YubiProvider as _,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use zeroize::{Zeroize as _, Zeroizing};

const DEFAULT_SOCKET_NAME: &str = "foks-rs.sock";
const AGENT_LOCK_NAME: &str = ".foks-rs.lock";
const MAXIMUM_REQUESTS_PER_CONNECTION: usize = 128;
const CANCELLATION_GRACE: Duration = Duration::from_secs(1);
const MAXIMUM_CANARY_BYTES: usize = 64 * 1024;
const MAXIMUM_CANARY_FETCHES: usize = 4;
const MAXIMUM_CONCURRENT_READS: usize = 4;
const DEVICE_PAIRING_TIMEOUT: Duration = Duration::from_secs(5 * 60);
// Byte vectors are JSON integer arrays on local protocol v2. Keep enough
// headroom for their worst-case textual expansion inside the 1 MiB frame.
const MAXIMUM_LOCAL_KV_CHUNK_BYTES: usize = 128 * 1024;
const MAXIMUM_INLINE_KV_BYTES: usize = 128 * 1024;
const MAXIMUM_STREAM_KV_BYTES: u64 = 1024 * 1024 * 1024;
const MAXIMUM_STREAM_FRAMES: usize = 8193;
const MAXIMUM_CACHED_CATALOGS: usize = 4;
const MAXIMUM_CACHED_CATALOG_BYTES: usize = 128 * 1024 * 1024;
const MAXIMUM_SINGLE_CATALOG_BYTES: usize = 64 * 1024 * 1024;
const CATALOG_CACHE_LIFETIME: Duration = Duration::from_secs(60);
const RESET_TOKEN_LIFETIME: Duration = Duration::from_secs(60);
const MAXIMUM_RESET_TICKETS: usize = 64;

struct ResetTicket {
    state_dir: PathBuf,
    profile: String,
    state_digest: [u8; 32],
    expires_at: Instant,
}

static RESET_TICKETS: OnceLock<Mutex<BTreeMap<[u8; 32], ResetTicket>>> = OnceLock::new();

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
    /// Maximum concurrent blocking reads (1-4).
    #[arg(long, default_value_t = MAXIMUM_CONCURRENT_READS)]
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
fn compatibility_http_client(timeout: Duration) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .https_only(true)
        // A profile pins the exact lease URL. Following a redirect would let
        // that origin turn a status refresh into a blind request elsewhere,
        // even though the eventual artifact still has to verify.
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(timeout)
        .timeout(timeout)
        .user_agent("foks-agent/compatibility-lease-v2")
        .build()
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
        || arguments.blocking_workers > MAXIMUM_CONCURRENT_READS
    {
        return Err("agent limits are outside supported bounds".into());
    }
    drop(ProfileRegistry::open(&arguments.state_dir)?);
    let state_dir = arguments.state_dir.canonicalize()?;
    let initialized = ClientCredentials::is_initialized(&state_dir)?;
    if initialized {
        ClientCredentials::open(&state_dir)?.master_key()?;
    }
    let ready = Arc::new(AtomicBool::new(initialized));
    let socket = arguments
        .socket
        .unwrap_or_else(|| state_dir.join(DEFAULT_SOCKET_NAME));
    let parent = socket.parent().ok_or("agent socket has no parent")?;
    if socket.file_name().is_none() {
        return Err("agent socket path has no file name".into());
    }
    // Ensure parent is a real directory (not a symlink) and is inside the
    // explicit state root. symlink_metadata does not follow the parent
    // symlink; canonicalize then checks containment after resolution.
    let parent_metadata = std::fs::symlink_metadata(parent)
        .map_err(|error| format!("agent socket parent is missing or inaccessible: {error}"))?;
    if !parent_metadata.is_dir() {
        return Err("agent socket parent is not a directory".into());
    }
    if !parent.canonicalize()?.starts_with(&state_dir) {
        return Err("agent socket must be below the explicit state root".into());
    }
    let _agent_lock = AgentLock::acquire(&state_dir)?;
    remove_stale_agent_socket(&socket)?;
    let listener = bind_private_agent_socket(&socket)?;
    let _socket_guard = SocketGuard(socket.clone());
    let active = Arc::new(Semaphore::new(arguments.maximum_connections));
    let blocking = Arc::new(Semaphore::new(arguments.blocking_workers));
    let mutations = Arc::new(Semaphore::new(1));
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
    let compatibility_client = compatibility_http_client(timeout)?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    eprintln!("FOKS agent ready: {}", socket.display());

    loop {
        tokio::select! {
            signal = tokio::signal::ctrl_c() => {
                signal?;
                scheduler_cancellation.cancel();
                break;
            }
            _ = terminate.recv() => {
                scheduler_cancellation.cancel();
                break;
            }
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let cred = match stream.peer_cred() {
                    Ok(cred) => cred,
                    Err(error) => {
                        eprintln!("foks-agent: rejecting connection without peer credentials: {error}");
                        continue;
                    }
                };
                if cred.uid() != rustix::process::geteuid().as_raw() {
                    eprintln!(
                        "foks-agent: rejecting peer uid {} (expected {})",
                        cred.uid(),
                        rustix::process::geteuid().as_raw()
                    );
                    continue;
                }
                let Ok(permit) = active.clone().try_acquire_owned() else {
                    drop(stream);
                    continue;
                };
                let state_dir = state_dir.clone();
                let blocking = blocking.clone();
                let mutations = mutations.clone();
                let ready = ready.clone();
                tokio::spawn(async move {
                    if let Err(error) = handle_connection(
                        stream,
                        state_dir,
                        blocking,
                        mutations,
                        ready,
                        timeout,
                        permit,
                    ).await {
                        eprintln!("foks-agent connection failed: {error}");
                    }
                });
            }
            _ = scheduler.tick() => {
                if !ready.load(Ordering::Acquire) {
                    continue;
                }
                let Ok(scheduler_permit) = scheduler_gate.clone().try_acquire_owned() else {
                    continue;
                };
                let Ok(permit) = blocking.clone().try_acquire_owned() else {
                    continue;
                };
                let Ok(mutation_permit) = mutations.clone().try_acquire_owned() else {
                    continue;
                };
                let state = state_dir.clone();
                let cancellation = scheduler_cancellation.clone();
                tokio::task::spawn_blocking(move || {
                    let _scheduler_permit = scheduler_permit;
                    let _permit = permit;
                    let _mutation_permit = mutation_permit;
                    run_scheduled_profiles(&state, timeout, cancellation);
                });
            }
            _ = compatibility.tick() => {
                if !ready.load(Ordering::Acquire) {
                    continue;
                }
                let Ok(compatibility_permit) = compatibility_gate.clone().try_acquire_owned() else {
                    continue;
                };
                let state = state_dir.clone();
                let client = compatibility_client.clone();
                let cancellation = scheduler_cancellation.clone();
                let mutations = mutations.clone();
                tokio::spawn(async move {
                    let _compatibility_permit = compatibility_permit;
                    refresh_hosted_profiles(&state, client, cancellation, mutations).await;
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
    let credentials = ClientCredentials::open(state_dir)?;
    let now = now_microseconds()?;
    let _ = credentials.try_with_checked_session(&session, |session| {
        let master = credentials.master_key()?;
        let mut store = EncryptedFileSecretStore::open(
            &session.paths().credential_store,
            derive_vault_key(&master),
        )?;
        session.run_due_jobs_with_federation(
            now,
            &mut AccountVault::new(&mut store),
            &registry,
            &credentials,
            &master,
        )?;
        Ok::<_, foks_client_app::Error>(())
    })?;
    Ok(())
}

async fn refresh_hosted_profiles(
    state_dir: &Path,
    client: reqwest::Client,
    cancellation: CancellationToken,
    mutations: Arc<Semaphore>,
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
            Ok((profile, Ok(bytes))) => {
                match apply_hosted_lease_with_gate(state_dir, &profile, &bytes, &mutations) {
                    Ok(Some(true)) => {
                        eprintln!("foks-agent applied a newer compatibility lease for {profile}");
                    }
                    Ok(Some(false)) => {}
                    Ok(None) => {
                        eprintln!(
                            "foks-agent deferred a compatibility lease while a mutation is active"
                        );
                    }
                    Err(_) => {
                        eprintln!("foks-agent rejected the compatibility lease for {profile}");
                    }
                }
            }
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

impl std::error::Error for LeaseFetchError {}

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

fn apply_hosted_lease_with_gate(
    state_dir: &Path,
    profile: &str,
    bytes: &[u8],
    mutations: &Semaphore,
) -> Result<Option<bool>, Box<dyn std::error::Error>> {
    let Ok(_mutation_permit) = mutations.try_acquire() else {
        return Ok(None);
    };
    apply_hosted_lease(state_dir, profile, bytes).map(Some)
}

#[cfg(unix)]
async fn handle_connection(
    mut stream: tokio::net::UnixStream,
    state_dir: PathBuf,
    blocking: Arc<Semaphore>,
    mutations: Arc<Semaphore>,
    ready: Arc<AtomicBool>,
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
                let id = foks_agent_proto::request_id(&frame);
                write_response(
                    &mut stream,
                    &Response::error_with_fields(
                        id,
                        ErrorCode::VersionMismatch,
                        "unsupported protocol version",
                        ErrorFields::default(),
                    ),
                    timeout,
                )
                .await?;
                continue;
            }
            Err(error) => {
                let id = foks_agent_proto::request_id(&frame);
                write_response(
                    &mut stream,
                    &Response::error_with_fields(
                        id,
                        ErrorCode::InvalidRequest,
                        error.to_string(),
                        ErrorFields::default(),
                    ),
                    timeout,
                )
                .await?;
                continue;
            }
        };
        if !operation_allowed(ready.load(Ordering::Acquire), &request.operation) {
            write_response(
                &mut stream,
                &Response::error(
                    request.id,
                    ErrorCode::BootstrapRequired,
                    "initialize the local FOKS state before using this operation",
                ),
                timeout,
            )
            .await?;
            continue;
        }
        if let Operation::PutKvStream { header } = &request.operation {
            handle_streaming_upload(
                &mut stream,
                state_dir.clone(),
                blocking.clone(),
                mutations.clone(),
                timeout,
                request.id,
                header.clone(),
            )
            .await?;
            return Ok(());
        }
        if let Operation::RefreshLease { profile } = &request.operation {
            let result = async {
                let registry = ProfileRegistry::open(&state_dir)?;
                let url = registry
                    .profile(profile)?
                    .compatibility_lease_url()
                    .ok_or(AgentRequestError(
                        "profile does not use a refreshable compatibility lease",
                    ))?
                    .to_owned();
                let client = compatibility_http_client(timeout)?;
                let bytes = fetch_canary(&client, &url).await?;
                let _mutation_permit = mutations
                    .clone()
                    .acquire_owned()
                    .await
                    .map_err(|_| AgentRequestError("agent is shutting down"))?;
                let updated = apply_hosted_lease(&state_dir, profile, &bytes)?;
                Ok::<_, Box<dyn std::error::Error>>(serde_json::json!({
                    "profile": profile,
                    "updated": updated,
                }))
            };
            let response = match tokio::time::timeout(timeout, result).await {
                Ok(Ok(value)) => Response::success(request.id, value),
                Ok(Err(error)) => dispatch_error_response(request.id, error.as_ref()),
                Err(_) => Response::error(
                    request.id,
                    ErrorCode::DeadlineExceeded,
                    "compatibility lease refresh exceeded its deadline",
                ),
            };
            write_response(&mut stream, &response, timeout).await?;
            continue;
        }
        let mutation_permit =
            if request.operation.is_mutation() && !request.operation.is_device_pairing_wait() {
                match tokio::time::timeout(timeout, mutations.clone().acquire_owned()).await {
                    Ok(Ok(permit)) => Some(permit),
                    Ok(Err(_)) => return Err("agent mutation gate closed".into()),
                    Err(_) => {
                        write_response(
                            &mut stream,
                            &Response::error(
                                request.id,
                                ErrorCode::Busy,
                                "another mutation is still in progress",
                            ),
                            timeout,
                        )
                        .await?;
                        continue;
                    }
                }
            } else {
                None
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
        let operation_ready = ready.clone();
        let operation_timeout = if request.operation.is_device_pairing_wait() {
            timeout.max(DEVICE_PAIRING_TIMEOUT)
        } else {
            timeout
        };
        let supervised = supervise_blocking(
            request.id,
            permit,
            mutation_permit,
            operation_timeout,
            move |cancellation| {
                dispatch_controlled(
                    &state,
                    request,
                    operation_timeout,
                    cancellation,
                    operation_ready,
                )
            },
        )
        .await;
        write_response(&mut stream, &supervised.response, timeout).await?;
        let close_connection = supervised.close_connection;
        drop(supervised);
        if close_connection {
            return Ok(());
        }
    }
    Ok(())
}

enum UploadMessage {
    Chunk(Zeroizing<Vec<u8>>),
    Commit,
}

struct UploadReader {
    receiver: tokio::sync::mpsc::Receiver<UploadMessage>,
    current: Zeroizing<Vec<u8>>,
    position: usize,
    total: u64,
    read: u64,
    committed: bool,
}

impl UploadReader {
    fn new(receiver: tokio::sync::mpsc::Receiver<UploadMessage>, total: u64) -> Self {
        Self {
            receiver,
            current: Zeroizing::new(Vec::new()),
            position: 0,
            total,
            read: 0,
            committed: false,
        }
    }
}

impl std::io::Read for UploadReader {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        loop {
            if self.position < self.current.len() {
                let count = output
                    .len()
                    .min(self.current.len().saturating_sub(self.position));
                output[..count]
                    .copy_from_slice(&self.current[self.position..self.position + count]);
                self.position += count;
                self.read = self
                    .read
                    .checked_add(count as u64)
                    .ok_or_else(|| std::io::Error::other("upload length overflow"))?;
                return Ok(count);
            }
            if self.committed {
                return Ok(0);
            }
            match self.receiver.blocking_recv() {
                Some(UploadMessage::Chunk(content)) => {
                    self.current = content;
                    self.position = 0;
                }
                Some(UploadMessage::Commit) if self.read == self.total => {
                    self.committed = true;
                }
                Some(UploadMessage::Commit) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "upload commit did not match its declared length",
                    ));
                }
                None => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "upload disconnected before commit",
                    ));
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_streaming_upload(
    stream: &mut tokio::net::UnixStream,
    state_dir: PathBuf,
    blocking: Arc<Semaphore>,
    mutations: Arc<Semaphore>,
    timeout: Duration,
    request_id: u64,
    header: KvUploadHeader,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if header.total_length > MAXIMUM_STREAM_KV_BYTES {
        write_response(
            stream,
            &Response::error(
                request_id,
                ErrorCode::InvalidRequest,
                "streaming KV upload exceeds the FOKS file size limit",
            ),
            timeout,
        )
        .await?;
        return Ok(());
    }
    let mutation_permit = match tokio::time::timeout(timeout, mutations.acquire_owned()).await {
        Ok(Ok(permit)) => permit,
        Ok(Err(_)) => return Err("agent mutation gate closed".into()),
        Err(_) => {
            write_response(
                stream,
                &Response::error(
                    request_id,
                    ErrorCode::Busy,
                    "another mutation is still in progress",
                ),
                timeout,
            )
            .await?;
            return Ok(());
        }
    };
    let permit = match tokio::time::timeout(timeout, blocking.acquire_owned()).await {
        Ok(Ok(permit)) => permit,
        Ok(Err(_)) => return Err("agent worker pool closed".into()),
        Err(_) => {
            write_response(
                stream,
                &Response::error(
                    request_id,
                    ErrorCode::Busy,
                    "agent worker pool is saturated",
                ),
                timeout,
            )
            .await?;
            return Ok(());
        }
    };
    let (sender, receiver) = tokio::sync::mpsc::channel(4);
    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.clone();
    let worker_header = header.clone();
    let worker = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let result = put_kv_reader(
            &state_dir,
            &ProfileRegistry::open(&state_dir)?,
            timeout,
            worker_cancellation,
            &worker_header.store,
            &worker_header.path,
            UploadReader::new(receiver, worker_header.total_length),
            worker_header.precondition,
            worker_header.read_role,
            worker_header.write_role,
            worker_header.mkdir_p,
        );
        let response = match result {
            Ok(value) => Response::success(request_id, value),
            Err(error) => dispatch_error_response(request_id, error.as_ref()),
        };
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>((response, mutation_permit))
    });
    let _cancel_on_drop = CancelOnDrop(cancellation.clone());
    let mut offset = 0u64;
    let mut committed = false;
    for _ in 0..MAXIMUM_STREAM_FRAMES {
        let frame = match tokio::time::timeout(timeout, read_frame(stream)).await {
            Ok(Ok(Some(frame))) => frame,
            Ok(Ok(None)) => return Ok(()),
            Ok(Err(error)) => return Err(error),
            Err(_) => return Err("streaming upload frame deadline exceeded".into()),
        };
        let frame = foks_agent_proto::decode_upload_frame(&frame)?;
        if frame.id != request_id {
            return Err("streaming upload response binding changed".into());
        }
        match frame.payload {
            foks_agent_proto::KvUploadPayload::Chunk {
                offset: frame_offset,
                content,
            } => {
                let content = Zeroizing::new(content);
                if content.is_empty()
                    || content.len() > MAXIMUM_LOCAL_KV_CHUNK_BYTES
                    || frame_offset != offset
                {
                    return Err("streaming upload chunk is invalid or out of order".into());
                }
                offset = offset
                    .checked_add(content.len() as u64)
                    .ok_or("streaming upload offset overflow")?;
                if offset > header.total_length {
                    return Err("streaming upload exceeds its declared length".into());
                }
                if sender.send(UploadMessage::Chunk(content)).await.is_err() {
                    break;
                }
            }
            foks_agent_proto::KvUploadPayload::Commit => {
                if offset != header.total_length {
                    return Err("streaming upload commit length mismatch".into());
                }
                if sender.send(UploadMessage::Commit).await.is_err() {
                    break;
                }
                committed = true;
                break;
            }
        }
    }
    drop(sender);
    if !committed {
        cancellation.cancel();
    }
    let (response, mutation_permit) = match tokio::time::timeout(
        timeout + CANCELLATION_GRACE,
        worker,
    )
    .await
    {
        Ok(Ok(Ok((response, mutation_permit)))) => (response, Some(mutation_permit)),
        Ok(Ok(Err(error))) => (
            Response::error(
                request_id,
                ErrorCode::OperationFailed,
                format!("streaming upload worker failed: {error}"),
            ),
            None,
        ),
        Ok(Err(error)) => (
            Response::error(
                request_id,
                ErrorCode::OperationFailed,
                format!("streaming upload worker failed: {error}"),
            ),
            None,
        ),
        Err(_) => {
            cancellation.cancel();
            (
                    Response::error(
                        request_id,
                        ErrorCode::DeadlineExceeded,
                        "streaming upload outcome is ambiguous; refresh the store before another mutation",
                    ),
                    None,
                )
        }
    };
    write_response(stream, &response, timeout).await?;
    drop(mutation_permit);
    Ok(())
}

fn operation_allowed(ready: bool, operation: &Operation) -> bool {
    ready
        || matches!(
            operation,
            Operation::AgentStatus | Operation::InitializeState { .. }
        )
}

struct SupervisedResponse {
    response: Response,
    close_connection: bool,
    _mutation_permit: Option<OwnedSemaphorePermit>,
}

struct WorkerCompletion {
    response: Response,
    mutation_permit: Option<OwnedSemaphorePermit>,
}

async fn supervise_blocking(
    request_id: u64,
    permit: OwnedSemaphorePermit,
    mutation_permit: Option<OwnedSemaphorePermit>,
    timeout: Duration,
    operation: impl FnOnce(CancellationToken) -> Response + Send + 'static,
) -> SupervisedResponse {
    let cancellation = CancellationToken::new();
    let _cancel_on_drop = CancelOnDrop(cancellation.clone());
    let worker_cancellation = cancellation.clone();
    let mut task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        WorkerCompletion {
            response: operation(worker_cancellation),
            mutation_permit,
        }
    });
    match tokio::time::timeout(timeout, &mut task).await {
        Ok(Ok(completion)) => SupervisedResponse {
            response: completion.response,
            close_connection: false,
            _mutation_permit: completion.mutation_permit,
        },
        Ok(Err(error)) => SupervisedResponse {
            response: Response::error(
                request_id,
                ErrorCode::OperationFailed,
                format!("agent worker failed: {error}"),
            ),
            close_connection: true,
            _mutation_permit: None,
        },
        Err(_) => {
            cancellation.cancel();
            // Blocking work cannot be forcibly killed safely. Give network
            // operations one poll interval to observe cancellation; if other
            // synchronous work remains stuck, its permit keeps the pool bound.
            let mutation_permit = match tokio::time::timeout(CANCELLATION_GRACE, &mut task).await {
                Ok(Ok(completion)) => completion.mutation_permit,
                Ok(Err(_)) | Err(_) => None,
            };
            SupervisedResponse {
                response: Response::error(
                    request_id,
                    ErrorCode::DeadlineExceeded,
                    "agent operation deadline exceeded; connection closed because completion is ambiguous",
                ),
                close_connection: true,
                _mutation_permit: mutation_permit,
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
    ready: Arc<AtomicBool>,
) -> Response {
    let id = request.id;
    let initializes = matches!(request.operation, Operation::InitializeState { .. });
    let result = dispatch_result(
        state_dir,
        request.operation,
        timeout,
        cancellation,
        ready.load(Ordering::Acquire),
    );
    if initializes && result.is_ok() {
        ready.store(true, Ordering::Release);
    }
    match result {
        Ok(value) => Response::success(id, value),
        Err(error) => dispatch_error_response(id, error.as_ref()),
    }
}

fn dispatch_error_response(id: u64, error: &(dyn std::error::Error + 'static)) -> Response {
    if let Some(error) = error.downcast_ref::<AgentRequestError>() {
        return Response::error(id, ErrorCode::InvalidRequest, error.to_string());
    }
    if error.downcast_ref::<ProfileBusyError>().is_some() {
        return Response::error(id, ErrorCode::ProfileBusy, error.to_string());
    }
    if let Some(error) = error.downcast_ref::<foks_client_app::Error>() {
        match error {
            foks_client_app::Error::CapabilityDenied(capability) => {
                let capability = serde_json::to_value(capability)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned));
                return Response::error_with_fields(
                    Some(id),
                    ErrorCode::CapabilityDenied,
                    error.to_string(),
                    ErrorFields {
                        capability,
                        ..ErrorFields::default()
                    },
                );
            }
            foks_client_app::Error::RollbackDetected(reason) => {
                return Response::error_with_fields(
                    Some(id),
                    ErrorCode::RollbackDetected,
                    error.to_string(),
                    ErrorFields {
                        reason: Some((*reason).to_owned()),
                        ..ErrorFields::default()
                    },
                );
            }
            foks_client_app::Error::CheckpointResetRequired {
                profile,
                state_dir,
                reason,
            } => {
                return Response::error_with_fields(
                    Some(id),
                    ErrorCode::CheckpointResetRequired,
                    error.to_string(),
                    ErrorFields {
                        profile: Some(profile.clone()),
                        state_dir: Some(state_dir.display().to_string()),
                        reason: Some((*reason).to_owned()),
                        ..ErrorFields::default()
                    },
                );
            }
            foks_client_app::Error::Io(io) if io.kind() == std::io::ErrorKind::WouldBlock => {
                return Response::error(id, ErrorCode::ProfileBusy, error.to_string());
            }
            foks_client_app::Error::KvConflict => {
                return Response::error(
                    id,
                    ErrorCode::Conflict,
                    "the KV item changed; refresh before retrying",
                );
            }
            foks_client_app::Error::ResetPreviewChanged => {
                return Response::error(
                    id,
                    ErrorCode::Conflict,
                    "the reset preview changed; review it again before retrying",
                );
            }
            _ => {}
        }
    }
    let mut source = Some(error);
    while let Some(candidate) = source {
        if let Some(foks_rpc::Error::RemoteStatus { code, .. }) =
            candidate.downcast_ref::<foks_rpc::Error>()
        {
            if let Some(response) = remote_status_response(id, *code, candidate.to_string()) {
                return response;
            }
        }
        if matches!(
            candidate.downcast_ref::<foks_client::Error>(),
            Some(foks_client::Error::DeadlineExceeded)
        ) {
            return Response::error(
                id,
                ErrorCode::DeadlineExceeded,
                bounded_error(error.to_string()),
            );
        }
        if candidate
            .downcast_ref::<foks_client_db::Error>()
            .is_some_and(database_error_invalidates_trust)
            || candidate
                .downcast_ref::<foks_verify::Error>()
                .is_some_and(verify_error_invalidates_trust)
        {
            return Response::error_with_fields(
                Some(id),
                ErrorCode::RollbackDetected,
                bounded_error(error.to_string()),
                ErrorFields {
                    reason: Some(bounded_error(candidate.to_string())),
                    ..ErrorFields::default()
                },
            );
        }
        source = candidate.source();
    }
    Response::error(
        id,
        ErrorCode::OperationFailed,
        bounded_error(error.to_string()),
    )
}

fn remote_status_response(id: u64, status: u64, reason: String) -> Option<Response> {
    let (code, message) = match status {
        foks_rpc::STATUS_RATE_LIMIT_ERROR => (
            ErrorCode::RateLimited,
            "The FOKS server is busy or rate-limiting requests. Wait briefly, then retry.",
        ),
        foks_rpc::STATUS_OVER_QUOTA_ERROR => (
            ErrorCode::QuotaExceeded,
            "The FOKS server reached a configured capacity limit. Review its capacity or remove unused data before retrying.",
        ),
        _ => return None,
    };
    Some(Response::error_with_fields(
        Some(id),
        code,
        message,
        ErrorFields {
            reason: Some(bounded_error(reason)),
            ..ErrorFields::default()
        },
    ))
}

fn database_error_invalidates_trust(error: &foks_client_db::Error) -> bool {
    matches!(
        error,
        foks_client_db::Error::HostIdentityChanged { .. }
            | foks_client_db::Error::GenesisChanged
            | foks_client_db::Error::ChainRollback { .. }
            | foks_client_db::Error::ChainFork { .. }
            | foks_client_db::Error::ProjectionChanged { .. }
            | foks_client_db::Error::MerkleRollback { .. }
            | foks_client_db::Error::MerkleFork { .. }
            | foks_client_db::Error::UserRollback { .. }
            | foks_client_db::Error::UserFork { .. }
            | foks_client_db::Error::UserProjectionChanged { .. }
            | foks_client_db::Error::UserGenericRollback { .. }
            | foks_client_db::Error::UserGenericFork { .. }
            | foks_client_db::Error::TeamRollback { .. }
            | foks_client_db::Error::TeamFork { .. }
            | foks_client_db::Error::TeamProjectionChanged { .. }
            | foks_client_db::Error::KvRootRollback { .. }
            | foks_client_db::Error::KvDirectoryRollback { .. }
            | foks_client_db::Error::KvProjectionConflict(_)
    )
}

fn verify_error_invalidates_trust(error: &foks_verify::Error) -> bool {
    matches!(
        error,
        foks_verify::Error::MerkleRollback { .. } | foks_verify::Error::MerkleFork(_)
    )
}

#[derive(Debug)]
struct AgentRequestError(&'static str);

impl std::fmt::Display for AgentRequestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for AgentRequestError {}

#[derive(Debug)]
struct ProfileBusyError;

impl std::fmt::Display for ProfileBusyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Another operation is using this profile.")
    }
}

impl std::error::Error for ProfileBusyError {}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CatalogStoreBinding {
    Account { value: AccountStoreRef },
    Team { value: TeamStoreRef },
}

#[derive(Debug)]
struct CatalogCursor {
    store: CatalogStoreBinding,
    snapshot_version: u64,
    snapshot_digest: [u8; 32],
    offset: usize,
}

struct CachedCatalog {
    store: CatalogStoreBinding,
    digest: [u8; 32],
    report: foks_client_app::KvCatalogReport,
    encoded_bytes: usize,
    expires_at: Instant,
}

#[derive(Default)]
struct CatalogCache {
    entries: VecDeque<CachedCatalog>,
    encoded_bytes: usize,
}

fn catalog_cache() -> &'static Mutex<CatalogCache> {
    static CACHE: OnceLock<Mutex<CatalogCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(CatalogCache::default()))
}

fn paginate_cached_catalog(
    store: &CatalogStoreBinding,
    cursor: &str,
    limit: u32,
) -> Result<Option<KvPage>, Box<dyn std::error::Error>> {
    validate_catalog_request(store, Some(cursor), limit)?;
    let requested = decode_catalog_cursor(cursor)?;
    let now = Instant::now();
    let mut cache = catalog_cache()
        .lock()
        .map_err(|_| AgentRequestError("catalog cache is unavailable"))?;
    while cache
        .entries
        .front()
        .is_some_and(|entry| entry.expires_at <= now)
    {
        if let Some(expired) = cache.entries.pop_front() {
            cache.encoded_bytes = cache.encoded_bytes.saturating_sub(expired.encoded_bytes);
        }
    }
    let Some(index) = cache.entries.iter().position(|entry| {
        entry.store == *store
            && entry.digest == requested.snapshot_digest
            && entry.report.snapshot_version == requested.snapshot_version
    }) else {
        return Ok(None);
    };
    let page = paginate_catalog(
        &cache.entries[index].report,
        store.clone(),
        Some(cursor),
        limit,
    )?;
    if page.next_cursor.is_none() {
        let completed = cache
            .entries
            .remove(index)
            .expect("located catalog cache entry is present");
        cache.encoded_bytes = cache.encoded_bytes.saturating_sub(completed.encoded_bytes);
    }
    Ok(Some(page))
}

fn cache_catalog(
    store: CatalogStoreBinding,
    report: &foks_client_app::KvCatalogReport,
) -> Result<(), Box<dyn std::error::Error>> {
    let (digest, encoded_bytes) = catalog_snapshot_identity(report)?;
    if encoded_bytes > MAXIMUM_SINGLE_CATALOG_BYTES {
        return Ok(());
    }
    let mut cache = catalog_cache()
        .lock()
        .map_err(|_| AgentRequestError("catalog cache is unavailable"))?;
    if let Some(index) = cache.entries.iter().position(|entry| entry.store == store) {
        let previous = cache
            .entries
            .remove(index)
            .expect("located catalog cache entry is present");
        cache.encoded_bytes = cache.encoded_bytes.saturating_sub(previous.encoded_bytes);
    }
    while cache.entries.len() >= MAXIMUM_CACHED_CATALOGS
        || cache
            .encoded_bytes
            .checked_add(encoded_bytes)
            .is_none_or(|total| total > MAXIMUM_CACHED_CATALOG_BYTES)
    {
        let Some(evicted) = cache.entries.pop_front() else {
            break;
        };
        cache.encoded_bytes = cache.encoded_bytes.saturating_sub(evicted.encoded_bytes);
    }
    cache.encoded_bytes += encoded_bytes;
    cache.entries.push_back(CachedCatalog {
        store,
        digest,
        report: report.clone(),
        encoded_bytes,
        expires_at: Instant::now() + CATALOG_CACHE_LIFETIME,
    });
    Ok(())
}

fn paginate_fresh_catalog(
    report: foks_client_app::KvCatalogReport,
    store: CatalogStoreBinding,
    cursor: Option<&str>,
    limit: u32,
) -> Result<KvPage, Box<dyn std::error::Error>> {
    let page = paginate_catalog(&report, store.clone(), cursor, limit)?;
    if page.next_cursor.is_some() {
        cache_catalog(store, &report)?;
    }
    Ok(page)
}

fn paginate_catalog(
    report: &foks_client_app::KvCatalogReport,
    store: CatalogStoreBinding,
    cursor: Option<&str>,
    limit: u32,
) -> Result<KvPage, Box<dyn std::error::Error>> {
    validate_catalog_request(&store, cursor, limit)?;
    let (snapshot_digest, _) = catalog_snapshot_identity(report)?;
    let offset = if let Some(cursor) = cursor {
        let cursor = decode_catalog_cursor(cursor)?;
        if cursor.snapshot_version != report.snapshot_version
            || cursor.snapshot_digest != snapshot_digest
        {
            return Err(Box::new(AgentRequestError(
                "catalog cursor belongs to another store snapshot",
            )));
        }
        cursor.offset
    } else {
        0
    };
    if offset > report.entries.len() {
        return Err(Box::new(AgentRequestError(
            "catalog cursor offset is outside the snapshot",
        )));
    }
    let maximum_end = offset
        .saturating_add(limit as usize)
        .min(report.entries.len());
    // Leave room for the response envelope and the next cursor. Limits are a
    // maximum, so unusually long authenticated paths produce a smaller page
    // instead of failing later in the frame encoder.
    const RESPONSE_ENVELOPE_RESERVE: usize = 32 * 1024;
    let payload_budget = MAXIMUM_MESSAGE_BYTES - RESPONSE_ENVELOPE_RESERVE;
    let mut encoded_entries = 2usize; // JSON array brackets.
    let mut entries = Vec::new();
    let mut end = offset;
    for entry in &report.entries[offset..maximum_end] {
        let entry = KvEntryMetadata {
            path: entry.path.clone(),
            node_type: entry.node_type.clone(),
            version: entry.version,
            size: entry.size,
            read_role: catalog_role(entry.read_role),
            write_role: catalog_role(entry.write_role),
        };
        let entry_bytes = serde_json::to_vec(&entry)?.len();
        let next_size = encoded_entries
            .checked_add(entry_bytes)
            .and_then(|size| size.checked_add(usize::from(!entries.is_empty())))
            .ok_or(AgentRequestError("catalog page size overflow"))?;
        if next_size > payload_budget {
            if entries.is_empty() {
                return Err(Box::new(AgentRequestError(
                    "catalog entry exceeds the local frame limit",
                )));
            }
            break;
        }
        encoded_entries = next_size;
        entries.push(entry);
        end += 1;
    }
    let next_cursor = (end < report.entries.len())
        .then(|| {
            encode_catalog_cursor(&CatalogCursor {
                store,
                snapshot_version: report.snapshot_version,
                snapshot_digest,
                offset: end,
            })
        })
        .transpose()?;
    Ok(KvPage {
        snapshot_version: report.snapshot_version,
        entries,
        next_cursor,
    })
}

fn catalog_snapshot_identity(
    report: &foks_client_app::KvCatalogReport,
) -> Result<([u8; 32], usize), serde_json::Error> {
    use sha2::Digest as _;
    use std::io::Write as _;

    struct DigestWriter {
        digest: sha2::Sha256,
        bytes: usize,
    }

    impl std::io::Write for DigestWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.digest.update(bytes);
            self.bytes = self
                .bytes
                .checked_add(bytes.len())
                .ok_or_else(|| std::io::Error::other("catalog size overflow"))?;
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let mut writer = DigestWriter {
        digest: sha2::Sha256::new(),
        bytes: 0,
    };
    serde_json::to_writer(&mut writer, report)?;
    writer.flush().map_err(serde_json::Error::io)?;
    Ok((writer.digest.finalize().into(), writer.bytes))
}

fn validate_catalog_request(
    store: &CatalogStoreBinding,
    cursor: Option<&str>,
    limit: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    const MAXIMUM_CATALOG_PAGE_ENTRIES: u32 = 500;
    if !(1..=MAXIMUM_CATALOG_PAGE_ENTRIES).contains(&limit) {
        return Err(Box::new(AgentRequestError(
            "catalog page limit must be between 1 and 500",
        )));
    }
    if let Some(cursor) = cursor {
        let cursor = decode_catalog_cursor(cursor)?;
        if cursor.store != store.clone() {
            return Err(Box::new(AgentRequestError(
                "catalog cursor belongs to another store snapshot",
            )));
        }
    }
    Ok(())
}

fn catalog_role(role: foks_client_app::KvRoleSummary) -> KvRole {
    match role {
        foks_client_app::KvRoleSummary::Member { visibility } => KvRole::Member { visibility },
        foks_client_app::KvRoleSummary::Admin => KvRole::Admin,
        foks_client_app::KvRoleSummary::Owner => KvRole::Owner,
    }
}

fn encode_catalog_cursor(cursor: &CatalogCursor) -> Result<String, Box<dyn std::error::Error>> {
    const MAXIMUM_CURSOR_BYTES: usize = 8192;
    let store = match &cursor.store {
        CatalogStoreBinding::Account { value } => serde_json::json!({
            "kind": "account",
            "profile": value.profile,
            "account_alias": value.account_alias,
        }),
        CatalogStoreBinding::Team { value } => serde_json::json!({
            "kind": "team",
            "profile": value.profile,
            "account_alias": value.account_alias,
            "team_alias": value.team_alias,
            "team_id": value.team_id,
        }),
    };
    let bytes = serde_json::to_vec(&serde_json::json!({
        "store": store,
        "snapshot_version": cursor.snapshot_version,
        "snapshot_digest": cursor
            .snapshot_digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        "offset": cursor.offset,
    }))?;
    if bytes.len() > MAXIMUM_CURSOR_BYTES {
        return Err(Box::new(AgentRequestError("catalog cursor is too large")));
    }
    let mut encoded = String::with_capacity(3 + bytes.len() * 2);
    encoded.push_str("v2.");
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}")?;
    }
    Ok(encoded)
}

fn decode_catalog_cursor(cursor: &str) -> Result<CatalogCursor, Box<dyn std::error::Error>> {
    const MAXIMUM_CURSOR_TEXT_BYTES: usize = 3 + 8192 * 2;
    let Some(encoded) = cursor.strip_prefix("v2.") else {
        return Err(Box::new(AgentRequestError("catalog cursor is invalid")));
    };
    if cursor.len() > MAXIMUM_CURSOR_TEXT_BYTES || encoded.len() % 2 != 0 {
        return Err(Box::new(AgentRequestError("catalog cursor is invalid")));
    }
    let mut bytes = Vec::with_capacity(encoded.len() / 2);
    for pair in encoded.as_bytes().chunks_exact(2) {
        let pair = std::str::from_utf8(pair)
            .map_err(|_| AgentRequestError("catalog cursor is invalid"))?;
        bytes.push(
            u8::from_str_radix(pair, 16)
                .map_err(|_| AgentRequestError("catalog cursor is invalid"))?,
        );
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| AgentRequestError("catalog cursor is invalid"))?;
    let object = value
        .as_object()
        .ok_or(AgentRequestError("catalog cursor is invalid"))?;
    let store = object
        .get("store")
        .and_then(serde_json::Value::as_object)
        .ok_or(AgentRequestError("catalog cursor is invalid"))?;
    let text = |name: &str| {
        store
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or(AgentRequestError("catalog cursor is invalid"))
    };
    let binding = match store.get("kind").and_then(serde_json::Value::as_str) {
        Some("account") => CatalogStoreBinding::Account {
            value: AccountStoreRef {
                profile: text("profile")?,
                account_alias: text("account_alias")?,
            },
        },
        Some("team") => CatalogStoreBinding::Team {
            value: TeamStoreRef {
                profile: text("profile")?,
                account_alias: text("account_alias")?,
                team_alias: text("team_alias")?,
                team_id: text("team_id")?,
            },
        },
        _ => return Err(Box::new(AgentRequestError("catalog cursor is invalid"))),
    };
    let snapshot_version = object
        .get("snapshot_version")
        .and_then(serde_json::Value::as_u64)
        .ok_or(AgentRequestError("catalog cursor is invalid"))?;
    let snapshot_digest = object
        .get("snapshot_digest")
        .and_then(serde_json::Value::as_str)
        .and_then(|digest| {
            if digest.len() != 64 {
                return None;
            }
            let mut bytes = [0u8; 32];
            for (output, pair) in bytes.iter_mut().zip(digest.as_bytes().chunks_exact(2)) {
                *output = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
            }
            Some(bytes)
        })
        .ok_or(AgentRequestError("catalog cursor is invalid"))?;
    let offset = object
        .get("offset")
        .and_then(serde_json::Value::as_u64)
        .and_then(|offset| usize::try_from(offset).ok())
        .ok_or(AgentRequestError("catalog cursor is invalid"))?;
    Ok(CatalogCursor {
        store: binding,
        snapshot_version,
        snapshot_digest,
        offset,
    })
}

fn kv_store_profile(store: &KvStoreRef) -> &str {
    match store {
        KvStoreRef::Account(store) => &store.profile,
        KvStoreRef::Team(store) => &store.profile,
    }
}

fn wire_role_to_app(role: KvRole) -> KvRoleSummary {
    match role {
        KvRole::Member { visibility } => KvRoleSummary::Member { visibility },
        KvRole::Admin => KvRoleSummary::Admin,
        KvRole::Owner => KvRoleSummary::Owner,
    }
}

fn app_role_to_wire(role: KvRoleSummary) -> KvRole {
    match role {
        KvRoleSummary::Member { visibility } => KvRole::Member { visibility },
        KvRoleSummary::Admin => KvRole::Admin,
        KvRoleSummary::Owner => KvRole::Owner,
    }
}

fn wire_precondition(precondition: KvPrecondition) -> KvMutationPrecondition {
    match precondition {
        KvPrecondition::Create => KvMutationPrecondition::Create,
        KvPrecondition::ExactVersion { version } => KvMutationPrecondition::ExactVersion(version),
    }
}

#[allow(clippy::too_many_arguments)]
fn put_kv_reader<R: std::io::Read>(
    state_dir: &Path,
    registry: &ProfileRegistry,
    timeout: Duration,
    cancellation: CancellationToken,
    store: &KvStoreRef,
    path: &str,
    mut reader: R,
    precondition: KvPrecondition,
    read_role: KvRole,
    write_role: KvRole,
    mkdir_p: bool,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let session = ProfileSession::open_with_control(
        registry,
        kv_store_profile(store),
        timeout,
        cancellation,
    )?;
    with_vault(state_dir, &session, |session, vault| {
        let credentials = ClientCredentials::open(state_dir)?;
        let master = credentials.master_key()?;
        let report = match store {
            KvStoreRef::Account(store) => session.put_kv_file_checked(
                &store.account_alias,
                path,
                &mut reader,
                wire_precondition(precondition),
                wire_role_to_app(read_role),
                wire_role_to_app(write_role),
                mkdir_p,
                vault,
                &master,
            )?,
            KvStoreRef::Team(store) => session.put_team_kv_file_checked(
                &store.account_alias,
                &store.team_alias,
                &store.team_id,
                path,
                &mut reader,
                wire_precondition(precondition),
                wire_role_to_app(read_role),
                wire_role_to_app(write_role),
                mkdir_p,
                vault,
                &master,
            )?,
        };
        Ok(serde_json::to_value(report)?)
    })
}

fn issue_reset_ticket(
    state_dir: &Path,
    profile: &str,
    state_digest: [u8; 32],
) -> Result<foks_agent_proto::SecretString, Box<dyn std::error::Error>> {
    let now = Instant::now();
    let tickets = RESET_TICKETS.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut tickets = tickets
        .lock()
        .map_err(|_| AgentRequestError("reset authorization state is unavailable"))?;
    tickets.retain(|_, ticket| ticket.expires_at > now);
    while tickets.len() >= MAXIMUM_RESET_TICKETS {
        let Some(oldest) = tickets
            .iter()
            .min_by_key(|(_, ticket)| ticket.expires_at)
            .map(|(token, _)| *token)
        else {
            break;
        };
        tickets.remove(&oldest);
    }
    for _ in 0..8 {
        let mut random = [0_u8; 32];
        getrandom::fill(&mut random)
            .map_err(|_| AgentRequestError("reset authorization randomness failed"))?;
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut token = String::with_capacity(random.len() * 2);
        for byte in random.iter().copied() {
            token.push(char::from(HEX[usize::from(byte >> 4)]));
            token.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        random.zeroize();
        let token_key = reset_token_key(&token);
        if tickets.contains_key(&token_key) {
            continue;
        }
        tickets.insert(
            token_key,
            ResetTicket {
                state_dir: state_dir.to_owned(),
                profile: profile.to_owned(),
                state_digest,
                expires_at: now + RESET_TOKEN_LIFETIME,
            },
        );
        return Ok(foks_agent_proto::SecretString::new(token));
    }
    Err(Box::new(AgentRequestError(
        "reset authorization could not be allocated",
    )))
}

fn consume_reset_ticket(
    state_dir: &Path,
    profile: &str,
    token: &str,
) -> Result<[u8; 32], Box<dyn std::error::Error>> {
    let now = Instant::now();
    let tickets = RESET_TICKETS.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut tickets = tickets
        .lock()
        .map_err(|_| AgentRequestError("reset authorization state is unavailable"))?;
    tickets.retain(|_, ticket| ticket.expires_at > now);
    let ticket = tickets
        .remove(&reset_token_key(token))
        .ok_or(AgentRequestError(
            "reset authorization is invalid or expired",
        ))?;
    if ticket.state_dir != state_dir || ticket.profile != profile {
        return Err(Box::new(AgentRequestError(
            "reset authorization does not match this profile",
        )));
    }
    Ok(ticket.state_digest)
}

fn reset_token_key(token: &str) -> [u8; 32] {
    use sha2::Digest as _;

    let mut digest = sha2::Sha256::new();
    digest.update(b"foks-reset-ticket-v1\0");
    digest.update(token.as_bytes());
    digest.finalize().into()
}

fn wire_pending_operation(
    operation: foks_client_app::PendingOperationSummary,
) -> WirePendingOperationSummary {
    WirePendingOperationSummary {
        kind: match operation.kind {
            foks_client_app::PendingOperationKind::AccountSignup => {
                WirePendingOperationKind::AccountSignup
            }
            foks_client_app::PendingOperationKind::DeviceProvision => {
                WirePendingOperationKind::DeviceProvision
            }
            foks_client_app::PendingOperationKind::PairingOffer => {
                WirePendingOperationKind::PairingOffer
            }
            foks_client_app::PendingOperationKind::PairingAcceptance => {
                WirePendingOperationKind::PairingAcceptance
            }
            foks_client_app::PendingOperationKind::AccountRecovery => {
                WirePendingOperationKind::AccountRecovery
            }
            foks_client_app::PendingOperationKind::YubiEnrollment => {
                WirePendingOperationKind::YubiEnrollment
            }
            foks_client_app::PendingOperationKind::TeamCreation => {
                WirePendingOperationKind::TeamCreation
            }
            foks_client_app::PendingOperationKind::TeamMemberAddition => {
                WirePendingOperationKind::TeamMemberAddition
            }
            foks_client_app::PendingOperationKind::TeamMemberEdit => {
                WirePendingOperationKind::TeamMemberEdit
            }
            foks_client_app::PendingOperationKind::FederationExpulsion => {
                WirePendingOperationKind::FederationExpulsion
            }
            foks_client_app::PendingOperationKind::TeamRekey => WirePendingOperationKind::TeamRekey,
        },
        alias: operation.alias,
        target: operation.target,
    }
}

fn wire_reset_artifact_kind(kind: foks_client_app::ResetArtifactKind) -> WireResetArtifactKind {
    match kind {
        foks_client_app::ResetArtifactKind::HardState => WireResetArtifactKind::HardState,
        foks_client_app::ResetArtifactKind::SoftState => WireResetArtifactKind::SoftState,
        foks_client_app::ResetArtifactKind::ProtectedMutations => {
            WireResetArtifactKind::ProtectedMutations
        }
        foks_client_app::ResetArtifactKind::CredentialsAndResumables => {
            WireResetArtifactKind::CredentialsAndResumables
        }
        foks_client_app::ResetArtifactKind::ExternalRollbackCheckpoint => {
            WireResetArtifactKind::ExternalRollbackCheckpoint
        }
        foks_client_app::ResetArtifactKind::ExternalDatabaseClaim => {
            WireResetArtifactKind::ExternalDatabaseClaim
        }
        foks_client_app::ResetArtifactKind::ExternalPublicationAuthorization => {
            WireResetArtifactKind::ExternalPublicationAuthorization
        }
    }
}

fn dispatch_result(
    state_dir: &Path,
    operation: Operation,
    timeout: Duration,
    cancellation: CancellationToken,
    ready: bool,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let mut registry = ProfileRegistry::open(state_dir)?;
    match operation {
        Operation::Ping => Ok(serde_json::json!({ "ready": true })),
        Operation::AgentStatus => Ok(serde_json::to_value(if ready {
            AgentStatus::Ready
        } else {
            AgentStatus::Bootstrap {
                step: "initialize-state".to_owned(),
            }
        })?),
        Operation::DiscoverGoProfiles => {
            let installation = foks_go_interop::discover_standard()?;
            Ok(serde_json::to_value(WireGoProfileDiscovery {
                installed: installation.installed,
                candidates: installation
                    .candidates
                    .into_iter()
                    .map(|candidate| WireGoProfileCandidate {
                        candidate_id: candidate.id,
                        username: None,
                        server_hint: None,
                        host_id_hex: candidate.host_id_hex,
                        user_id_hex: candidate.user_id_hex,
                        device_id_hex: candidate.device_id_hex,
                        role: candidate.role,
                        storage_kind: candidate.storage.as_str().to_owned(),
                        hidden: candidate.hidden,
                        provisional: candidate.provisional,
                        pairable: candidate.pairable,
                        copyable: candidate.copyable,
                    })
                    .collect(),
            })?)
        }
        Operation::InitializeState { backend } => {
            let backend = match backend {
                WireCredentialBackend::Native => CredentialBackend::Native,
                WireCredentialBackend::PrivateFile => CredentialBackend::PrivateFile,
            };
            // Bootstrap is a durable-state transition, not a one-shot process
            // event. The CLI may have completed it while this resident agent was
            // still in bootstrap mode, or a prior request may have written the
            // state envelope before master-key verification was interrupted.
            // Reopen and verify existing state to ensure the agent transitions
            // from Bootstrap to Ready if initialization already completed.
            let credentials = if ClientCredentials::is_initialized(state_dir)? {
                ClientCredentials::open(state_dir)?
            } else {
                match ClientCredentials::initialize(state_dir, backend) {
                    Ok(credentials) => credentials,
                    Err(_error) if ClientCredentials::is_initialized(state_dir)? => {
                        ClientCredentials::open(state_dir)?
                    }
                    Err(error) => return Err(error.into()),
                }
            };
            credentials.master_key()?;
            drop(ProfileRegistry::open(state_dir)?);
            Ok(serde_json::json!({
                "backend": match credentials.backend() {
                    CredentialBackend::Native => "native",
                    CredentialBackend::PrivateFile => "private-file",
                }
            }))
        }
        Operation::AddProfile {
            name,
            probe,
            protocol,
            trust,
        } => {
            let protocol = match protocol {
                ProfileProtocol::V019 => ProtocolPolicy::V019,
                ProfileProtocol::CurrentProbeOnly {
                    canary_public_key,
                    lease_url,
                } => ProtocolPolicy::CurrentProbeOnly {
                    canary_public_key,
                    lease_url,
                    last_artifact: None,
                },
            };
            let trust = match trust {
                ProfileTrust::WebPki => TrustRoot::WebPki,
                ProfileTrust::CertificateDer { path } => TrustRoot::CertificateDer {
                    path: PathBuf::from(path),
                },
            };
            let profile = Profile {
                name,
                probe,
                protocol,
                trust,
            };
            registry.add(profile.clone())?;
            Ok(serde_json::to_value(profile)?)
        }
        Operation::CheckAndAddProfile {
            name,
            probe,
            protocol,
            trust,
        } => {
            let protocol = match protocol {
                ProfileProtocol::V019 => ProtocolPolicy::V019,
                ProfileProtocol::CurrentProbeOnly {
                    canary_public_key,
                    lease_url,
                } => ProtocolPolicy::CurrentProbeOnly {
                    canary_public_key,
                    lease_url,
                    last_artifact: None,
                },
            };
            let trust = match trust {
                ProfileTrust::WebPki => TrustRoot::WebPki,
                ProfileTrust::CertificateDer { path } => TrustRoot::CertificateDer {
                    path: PathBuf::from(path),
                },
            };
            let credentials = ClientCredentials::open(state_dir)?;
            Ok(serde_json::to_value(
                credentials.check_and_add_profile_with_control(
                    &mut registry,
                    Profile {
                        name,
                        probe,
                        protocol,
                        trust,
                    },
                    timeout,
                    cancellation,
                )?,
            )?)
        }
        Operation::CheckAndAddGoProfile {
            candidate_id,
            name,
            probe,
            protocol,
            trust,
        } => {
            let root = foks_go_interop::standard_root().ok_or("Go FOKS home is unavailable")?;
            let candidate = foks_go_interop::resolve(&root, &candidate_id)?;
            if !candidate.summary.pairable {
                return Err("Go FOKS profile is not pairable".into());
            }
            let protocol = match protocol {
                ProfileProtocol::V019 => ProtocolPolicy::V019,
                ProfileProtocol::CurrentProbeOnly {
                    canary_public_key,
                    lease_url,
                } => ProtocolPolicy::CurrentProbeOnly {
                    canary_public_key,
                    lease_url,
                    last_artifact: None,
                },
            };
            let trust = match trust {
                ProfileTrust::WebPki => TrustRoot::WebPki,
                ProfileTrust::CertificateDer { path } => TrustRoot::CertificateDer {
                    path: PathBuf::from(path),
                },
            };
            let credentials = ClientCredentials::open(state_dir)?;
            Ok(serde_json::to_value(
                credentials.check_and_add_profile_for_host_with_control(
                    &mut registry,
                    Profile {
                        name,
                        probe,
                        protocol,
                        trust,
                    },
                    timeout,
                    cancellation,
                    Some(candidate.host_id()),
                )?,
            )?)
        }
        Operation::RemoveProfile { name } => {
            let credentials = ClientCredentials::open(state_dir)?;
            let removed = credentials.remove_profile(&mut registry, &name)?;
            Ok(serde_json::json!({
                "profile": name,
                "removed": removed,
            }))
        }
        Operation::DescribeResetHardState { profile } => {
            let credentials = ClientCredentials::open(state_dir)?;
            credentials.master_key()?;
            let session = ProfileSession::open(&registry, &profile)?;
            let preview = credentials.describe_reset_state(&session)?;
            let token = issue_reset_ticket(state_dir, &profile, preview.state_digest())?;
            Ok(serde_json::to_value(WireResetStatePreview {
                profile: preview.profile,
                resumables: preview
                    .resumables
                    .into_iter()
                    .map(wire_pending_operation)
                    .collect(),
                artifacts: preview
                    .artifacts
                    .into_iter()
                    .map(|artifact| WireResetArtifactSummary {
                        kind: wire_reset_artifact_kind(artifact.kind),
                        entries: artifact.entries,
                        bytes: artifact.bytes,
                    })
                    .collect(),
                token,
                expires_in_seconds: RESET_TOKEN_LIFETIME.as_secs(),
            })?)
        }
        Operation::ResetHardState { profile, token } => {
            let state_digest = consume_reset_ticket(state_dir, &profile, token.expose())?;
            let credentials = ClientCredentials::open(state_dir)?;
            credentials.master_key()?;
            let session = ProfileSession::open(&registry, &profile)?;
            credentials.reset_hard_state_if_matches(&session, state_digest)?;
            Ok(serde_json::json!({
                "profile": profile,
                "hard_state_reset": true,
            }))
        }
        Operation::ListProfiles => Ok(serde_json::to_value(
            registry.profiles().cloned().collect::<Vec<_>>(),
        )?),
        Operation::Probe { profile } => {
            let credentials = ClientCredentials::open(state_dir)?;
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            let report = checked_session(&credentials, &session, |session| {
                session
                    .probe_and_pin()
                    .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)
            })?;
            Ok(serde_json::to_value(report)?)
        }
        Operation::RefreshLease { .. } => Err(Box::new(AgentRequestError(
            "lease refresh requires the async agent connection path",
        ))),
        Operation::ListKnownStores { profile } => {
            registry.profile(&profile)?;
            let paths = registry.prepare_profile_directory(&profile)?;
            let stores = SoftStateStore::open(&paths.soft_database)?
                .known_stores()?
                .into_iter()
                .map(wire_known_store)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(serde_json::to_value(stores)?)
        }
        Operation::ListAccounts { profile } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                let mut accounts = Vec::new();
                for alias in vault.aliases()? {
                    let loaded = vault.account(&alias)?;
                    accounts.push(AccountSummary {
                        profile: profile.clone(),
                        alias,
                        username: loaded.username,
                    });
                }
                let aliases = accounts
                    .iter()
                    .map(|account| account.alias.clone())
                    .collect::<Vec<_>>();
                retain_known_accounts(session, &profile, &aliases);
                Ok(serde_json::to_value(accounts)?)
            })
        }
        Operation::ListPendingOperations { profile } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |_session, vault| {
                let pending = vault
                    .pending_operations()?
                    .into_iter()
                    .map(wire_pending_operation)
                    .collect::<Vec<_>>();
                Ok(serde_json::to_value(pending)?)
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
            checked_session(&credentials, &session, |session| {
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
                    invite.expose(),
                    passphrase,
                    &mut vault,
                    &master,
                )?)?)
            })
        }
        Operation::ResumeAccount { profile, alias } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            let credentials = ClientCredentials::open(state_dir)?;
            checked_session(&credentials, &session, |session| {
                let master = credentials.master_key()?;
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    derive_vault_key(&master),
                )?;
                Ok(serde_json::to_value(session.resume_account(
                    &alias,
                    &mut AccountVault::new(&mut store),
                    &master,
                )?)?)
            })
        }
        Operation::ListDevices { profile, alias } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                let devices = session
                    .list_devices(&alias, vault)?
                    .into_iter()
                    .map(|device| WireDeviceSummary {
                        id_hex: device.id_hex,
                        name: device.name,
                        role: device.role,
                        current: device.current,
                    })
                    .collect::<Vec<_>>();
                Ok(serde_json::to_value(devices)?)
            })
        }
        Operation::ListBackupEnrollments {
            profile,
            account_alias,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |_session, vault| {
                let summaries = vault
                    .backup_enrollments(&account_alias)?
                    .into_iter()
                    .map(|summary| WireBackupEnrollmentSummary {
                        backup_alias: summary.backup_alias,
                        account_alias: summary.account_alias,
                        backup_id_hex: summary.backup_id_hex,
                    })
                    .collect::<Vec<_>>();
                Ok(serde_json::to_value(summaries)?)
            })
        }
        Operation::DescribeServerStatus { profile } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            let status = if session.has_hard_state_artifacts()? {
                let credentials = ClientCredentials::open(state_dir)?;
                checked_session(&credentials, &session, |session| {
                    session
                        .server_status()
                        .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)
                })?
            } else {
                session.server_status_without_pinned_host()?
            };
            Ok(serde_json::to_value(WireServerStatusSnapshot {
                profile: status.profile,
                configured_probe: status.configured_probe,
                host: status.host.map(|host| WireStoredHostStatus {
                    lookup_name: host.lookup_name,
                    canonical_name: host.canonical_name,
                    host_id_hex: host.host_id_hex,
                    host_chain_sequence: host.host_chain_sequence,
                    merkle_epoch: host.merkle_epoch,
                }),
                lease_required: status.lease_required,
                lease_expires_at: status.lease_expires_at,
            })?)
        }
        Operation::RemoveDevice {
            profile,
            signer_alias,
            device_id,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                let credentials = ClientCredentials::open(state_dir)?;
                let master = credentials.master_key()?;
                Ok(serde_json::to_value(session.remove_software_device(
                    &signer_alias,
                    &device_id,
                    vault,
                    &master,
                )?)?)
            })
        }
        Operation::ProvisionOwnerDevice {
            profile,
            source_alias,
            target_alias,
            device_name,
            serial,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
                Ok(serde_json::to_value(session.provision_owner_device(
                    &source_alias,
                    &target_alias,
                    &device_name,
                    serial,
                    vault,
                    master,
                )?)?)
            })
        }
        Operation::ResumeOwnerDeviceProvision {
            profile,
            target_alias,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
                Ok(serde_json::to_value(
                    session.resume_owner_device_provision(&target_alias, vault, master)?,
                )?)
            })
        }
        Operation::PrepareOwnerBackup {
            profile,
            account_alias,
            backup_alias,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                let phrase = session.prepare_owner_backup(&account_alias, &backup_alias, vault)?;
                let phrase = phrase.expose_joined();
                Ok(serde_json::json!({
                    "backup_alias": backup_alias,
                    "phrase": phrase.as_str(),
                }))
            })
        }
        Operation::CommitOwnerBackup {
            profile,
            account_alias,
            backup_alias,
            phrase,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(session.commit_owner_backup(
                    &account_alias,
                    &backup_alias,
                    Zeroizing::new(phrase.expose().to_owned()),
                    vault,
                )?)?)
            })
        }
        Operation::RevokeOwnerBackup {
            profile,
            account_alias,
            backup_alias,
            backup_id,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
                Ok(serde_json::to_value(session.revoke_owner_backup(
                    &account_alias,
                    &backup_alias,
                    &backup_id,
                    vault,
                    master,
                )?)?)
            })
        }
        Operation::RecoverOwnerAccount {
            profile,
            target_alias,
            phrase,
            device_name,
            serial,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(session.recover_owner_account(
                    &target_alias,
                    Zeroizing::new(phrase.expose().to_owned()),
                    &device_name,
                    serial,
                    vault,
                )?)?)
            })
        }
        Operation::ResumeOwnerRecovery {
            profile,
            target_alias,
            phrase,
            device_name,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(session.resume_owner_recovery(
                    &target_alias,
                    Zeroizing::new(phrase.expose().to_owned()),
                    &device_name,
                    vault,
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
        Operation::SetYubiPassphrase {
            profile,
            alias,
            pin,
            passphrase,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
                Ok(serde_json::to_value(session.set_yubi_passphrase(
                    &alias,
                    Pin::new(pin.expose())?,
                    Passphrase::new(passphrase.expose())?,
                    &HardwareYubiProvider::new(),
                    vault,
                    master,
                )?)?)
            })
        }
        Operation::ChangeYubiPassphrase {
            profile,
            alias,
            pin,
            passphrase,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
                Ok(serde_json::to_value(session.change_yubi_passphrase(
                    &alias,
                    Pin::new(pin.expose())?,
                    Passphrase::new(passphrase.expose())?,
                    &HardwareYubiProvider::new(),
                    vault,
                    master,
                )?)?)
            })
        }
        Operation::VerifyYubiPassphrase {
            profile,
            alias,
            pin,
            passphrase,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
                Ok(serde_json::to_value(session.verify_yubi_passphrase(
                    &alias,
                    Pin::new(pin.expose())?,
                    Passphrase::new(passphrase.expose())?,
                    &HardwareYubiProvider::new(),
                    vault,
                    master,
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
        Operation::StartDevicePairing {
            profile,
            account_alias,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(
                    session.start_owner_device_pairing(&account_alias, vault)?,
                )?)
            })
        }
        Operation::RepublishDevicePairing {
            profile,
            account_alias,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(
                    session.republish_owner_device_pairing(&account_alias, vault)?,
                )?)
            })
        }
        Operation::FinishDevicePairing {
            profile,
            account_alias,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
                Ok(serde_json::to_value(session.finish_owner_device_pairing(
                    &account_alias,
                    vault,
                    master,
                )?)?)
            })
        }
        Operation::AcceptDevicePairing {
            profile,
            target_alias,
            device_name,
            serial,
            phrase,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(session.accept_owner_device_pairing(
                    KexAcceptanceInput {
                        target_alias,
                        device_name,
                        serial,
                        phrase: phrase.expose().to_owned(),
                    },
                    vault,
                )?)?)
            })
        }
        Operation::AcceptGoProfilePairing {
            candidate_id,
            profile,
            target_alias,
            device_name,
            serial,
            phrase,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                let candidate = go_candidate_for_session(&candidate_id, session)?;
                Ok(serde_json::to_value(
                    session.accept_owner_device_pairing_for_user(
                        KexAcceptanceInput {
                            target_alias,
                            device_name,
                            serial,
                            phrase: phrase.expose().to_owned(),
                        },
                        Some(candidate.user_id()),
                        Some(&candidate_id),
                        vault,
                    )?,
                )?)
            })
        }
        Operation::ResumeDevicePairingAcceptance {
            profile,
            target_alias,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(
                    session.resume_owner_device_pairing_acceptance(&target_alias, vault)?,
                )?)
            })
        }
        Operation::ResumeGoProfilePairing {
            candidate_id,
            profile,
            target_alias,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(
                    session.resume_owner_device_pairing_acceptance_for_user(
                        &target_alias,
                        None,
                        Some(&candidate_id),
                        vault,
                    )?,
                )?)
            })
        }
        Operation::CopyGoProfileDevice {
            candidate_id,
            profile,
            target_alias,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                let candidate = go_candidate_for_session(&candidate_id, session)?;
                if !candidate.summary.copyable {
                    return Err("Go FOKS profile cannot copy its device key".into());
                }
                let expected_device: &[u8; 33] = candidate
                    .device_id()
                    .try_into()
                    .map_err(|_| "Go FOKS profile has a non-software device id")?;
                let seed = candidate.copy_device_seed()?;
                Ok(serde_json::to_value(session.import_software_device(
                    &target_alias,
                    candidate.user_id(),
                    expected_device,
                    foks_proto::SecretSeed::new(*seed),
                    vault,
                )?)?)
            })
        }
        Operation::ListYubiCards { profile } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, _vault| {
                Ok(serde_json::to_value(
                    session.list_yubi_cards(&HardwareYubiProvider::new())?,
                )?)
            })
        }
        Operation::ListYubiAccounts { profile } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |_session, vault| {
                Ok(serde_json::to_value(vault.yubi_accounts()?)?)
            })
        }
        Operation::CreateYubiAccount {
            profile,
            alias,
            username,
            device_name,
            email,
            invite,
            passphrase,
            card_serial,
            signing_slot,
            pq_slot,
            pin,
            retry_configuration,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
                let provider = HardwareYubiProvider::new();
                let card = yubi_card(&provider, card_serial)?;
                let passphrase = passphrase
                    .as_ref()
                    .map(|passphrase| Passphrase::new(passphrase.expose()))
                    .transpose()?;
                let retry_configuration = retry_configuration
                    .map(|retry| {
                        PinRetryConfiguration::new(
                            Pin::new(retry.puk.expose())?,
                            retry.pin_attempts,
                            retry.puk_attempts,
                        )
                    })
                    .transpose()?;
                Ok(serde_json::to_value(session.create_yubi_account(
                    YubiSignupInput {
                        alias,
                        username,
                        device_name,
                        email,
                        invite: invite.expose().to_owned(),
                        passphrase,
                        card,
                        signing_slot: SlotId::new(signing_slot)?,
                        pq_slot: SlotId::new(pq_slot)?,
                        retry_configuration,
                    },
                    Pin::new(pin.expose())?,
                    &provider,
                    vault,
                    master,
                )?)?)
            })
        }
        Operation::ResumeYubiAccount {
            profile,
            alias,
            pin,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
                Ok(serde_json::to_value(session.resume_yubi_account(
                    &alias,
                    Pin::new(pin.expose())?,
                    &HardwareYubiProvider::new(),
                    vault,
                    master,
                )?)?)
            })
        }
        Operation::ProvisionYubiDevice {
            profile,
            source_alias,
            target_alias,
            device_name,
            serial,
            card_serial,
            signing_slot,
            pq_slot,
            pin,
            retry_configuration,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
                let provider = HardwareYubiProvider::new();
                let card = yubi_card(&provider, card_serial)?;
                let retry_configuration = retry_configuration
                    .map(|retry| {
                        PinRetryConfiguration::new(
                            Pin::new(retry.puk.expose())?,
                            retry.pin_attempts,
                            retry.puk_attempts,
                        )
                    })
                    .transpose()?;
                Ok(serde_json::to_value(session.provision_yubi_device(
                    YubiProvisionInput {
                        source_alias,
                        target_alias,
                        device_name,
                        serial,
                        card,
                        signing_slot: SlotId::new(signing_slot)?,
                        pq_slot: SlotId::new(pq_slot)?,
                        retry_configuration,
                    },
                    Pin::new(pin.expose())?,
                    &provider,
                    vault,
                    master,
                )?)?)
            })
        }
        Operation::SyncYubiAccount {
            profile,
            alias,
            pin,
            with_federation,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            if !with_federation {
                return with_vault_and_master(state_dir, &session, |session, vault, master| {
                    Ok(serde_json::to_value(session.sync_yubi_account(
                        &alias,
                        Pin::new(pin.expose())?,
                        &HardwareYubiProvider::new(),
                        vault,
                        master,
                    )?)?)
                });
            }
            let credentials = ClientCredentials::open(state_dir)?;
            checked_session(&credentials, &session, |session| {
                let master = credentials.master_key()?;
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    derive_vault_key(&master),
                )?;
                Ok(serde_json::to_value(
                    session.sync_yubi_account_with_federation(
                        &alias,
                        Pin::new(pin.expose())?,
                        &HardwareYubiProvider::new(),
                        &mut AccountVault::new(&mut store),
                        &registry,
                        &credentials,
                        &master,
                    )?,
                )?)
            })
        }
        Operation::YubiPinStatus { profile, alias } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(session.yubi_pin_status(
                    &alias,
                    &HardwareYubiProvider::new(),
                    vault,
                )?)?)
            })
        }
        Operation::ChangeYubiPin {
            profile,
            alias,
            old_pin,
            new_pin,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(session.change_yubi_pin(
                    &alias,
                    Pin::new(old_pin.expose())?,
                    Pin::new(new_pin.expose())?,
                    &HardwareYubiProvider::new(),
                    vault,
                )?)?)
            })
        }
        Operation::ChangeYubiPuk {
            profile,
            alias,
            old_puk,
            new_puk,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                session.change_yubi_puk(
                    &alias,
                    Pin::new(old_puk.expose())?,
                    Pin::new(new_puk.expose())?,
                    &HardwareYubiProvider::new(),
                    vault,
                )?;
                Ok(serde_json::json!({ "alias": alias, "changed": true }))
            })
        }
        Operation::UnblockYubiPin {
            profile,
            alias,
            puk,
            new_pin,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(session.unblock_yubi_pin(
                    &alias,
                    Pin::new(puk.expose())?,
                    Pin::new(new_pin.expose())?,
                    &HardwareYubiProvider::new(),
                    vault,
                )?)?)
            })
        }
        Operation::RotateYubiManagementKey {
            profile,
            alias,
            pin,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
                Ok(serde_json::to_value(session.rotate_yubi_management_key(
                    &alias,
                    Pin::new(pin.expose())?,
                    &HardwareYubiProvider::new(),
                    vault,
                    master,
                )?)?)
            })
        }
        Operation::ResumeYubiManagementKey {
            profile,
            alias,
            pin,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
                let pin = pin.as_ref().map(|pin| Pin::new(pin.expose())).transpose()?;
                Ok(serde_json::to_value(session.resume_yubi_management_key(
                    &alias,
                    pin,
                    &HardwareYubiProvider::new(),
                    vault,
                    master,
                )?)?)
            })
        }
        Operation::RecoverYubiManagementKey {
            profile,
            yubi_alias,
            software_alias,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(session.recover_yubi_management_key(
                    &yubi_alias,
                    &software_alias,
                    vault,
                )?)?)
            })
        }
        Operation::RecoverYubiSubkey {
            profile,
            alias,
            pin,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
                Ok(serde_json::to_value(session.recover_yubi_subkey(
                    &alias,
                    Pin::new(pin.expose())?,
                    &HardwareYubiProvider::new(),
                    vault,
                    master,
                )?)?)
            })
        }
        Operation::RevokeYubiDevice {
            profile,
            yubi_alias,
            software_alias,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
                Ok(serde_json::to_value(session.revoke_yubi_device(
                    &software_alias,
                    &yubi_alias,
                    vault,
                    master,
                )?)?)
            })
        }
        Operation::ListKv {
            store,
            cursor,
            limit,
        } => {
            let binding = CatalogStoreBinding::Account {
                value: store.clone(),
            };
            validate_catalog_request(&binding, cursor.as_deref(), limit)?;
            if let Some(cursor) = cursor.as_deref() {
                if let Some(page) = paginate_cached_catalog(&binding, cursor, limit)? {
                    return Ok(serde_json::to_value(page)?);
                }
            }
            let session = ProfileSession::open_with_control(
                &registry,
                &store.profile,
                timeout,
                cancellation,
            )?;
            with_vault(state_dir, &session, |session, vault| {
                let report = session.list_kv_metadata(&store.account_alias, vault)?;
                Ok(serde_json::to_value(paginate_fresh_catalog(
                    report,
                    binding,
                    cursor.as_deref(),
                    limit,
                )?)?)
            })
        }
        Operation::ListTeamKv {
            store,
            cursor,
            limit,
        } => {
            let binding = CatalogStoreBinding::Team {
                value: store.clone(),
            };
            validate_catalog_request(&binding, cursor.as_deref(), limit)?;
            if let Some(cursor) = cursor.as_deref() {
                if let Some(page) = paginate_cached_catalog(&binding, cursor, limit)? {
                    return Ok(serde_json::to_value(page)?);
                }
            }
            let session = ProfileSession::open_with_control(
                &registry,
                &store.profile,
                timeout,
                cancellation,
            )?;
            with_vault(state_dir, &session, |session, vault| {
                let report = session.list_team_kv_metadata(
                    &store.account_alias,
                    &store.team_alias,
                    &store.team_id,
                    vault,
                )?;
                Ok(serde_json::to_value(paginate_fresh_catalog(
                    report,
                    binding,
                    cursor.as_deref(),
                    limit,
                )?)?)
            })
        }
        Operation::ReadKv {
            store,
            path,
            version,
        } => {
            let profile = kv_store_profile(&store).to_owned();
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                let report = match &store {
                    KvStoreRef::Account(store) => {
                        session.read_kv_entry(&store.account_alias, &path, version, vault)?
                    }
                    KvStoreRef::Team(store) => session.read_team_kv_entry(
                        &store.account_alias,
                        &store.team_alias,
                        &store.team_id,
                        &path,
                        version,
                        vault,
                    )?,
                };
                Ok(serde_json::to_value(KvReadResult {
                    store,
                    path: report.path,
                    version: report.version,
                    node_type: report.node_type,
                    size: report.size,
                    read_role: app_role_to_wire(report.read_role),
                    write_role: app_role_to_wire(report.write_role),
                    content: report.content,
                    symlink_target: report.symlink_target,
                })?)
            })
        }
        Operation::ReadKvChunk {
            store,
            path,
            version,
            offset,
            length,
        } => {
            let length = usize::try_from(length)
                .map_err(|_| AgentRequestError("KV chunk length is invalid"))?;
            if length == 0 || length > MAXIMUM_LOCAL_KV_CHUNK_BYTES {
                return Err(Box::new(AgentRequestError(
                    "KV chunk length is outside supported bounds",
                )));
            }
            let profile = kv_store_profile(&store).to_owned();
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                let report = match &store {
                    KvStoreRef::Account(store) => session.read_kv_chunk(
                        &store.account_alias,
                        &path,
                        version,
                        offset,
                        length,
                        vault,
                    )?,
                    KvStoreRef::Team(store) => session.read_team_kv_chunk(
                        &store.account_alias,
                        &store.team_alias,
                        &store.team_id,
                        &path,
                        version,
                        offset,
                        length,
                        vault,
                    )?,
                };
                Ok(serde_json::to_value(KvChunkResult {
                    store,
                    path: report.path,
                    version: report.version,
                    offset: report.offset,
                    content: report.content,
                    eof: report.eof,
                })?)
            })
        }
        Operation::PutKv {
            store,
            path,
            content,
            read_role,
            write_role,
            precondition,
            mkdir_p,
        } => {
            if content.len() > MAXIMUM_INLINE_KV_BYTES {
                return Err(Box::new(AgentRequestError(
                    "inline KV content exceeds its local protocol limit",
                )));
            }
            let content = Zeroizing::new(content);
            put_kv_reader(
                state_dir,
                &registry,
                timeout,
                cancellation,
                &store,
                &path,
                std::io::Cursor::new(content.as_slice()),
                precondition,
                read_role,
                write_role,
                mkdir_p,
            )
        }
        Operation::PutKvStream { .. } => Err(Box::new(AgentRequestError(
            "streaming KV upload requires the streaming connection path",
        ))),
        Operation::PutKvSymlink {
            store,
            path,
            target,
            read_role,
            write_role,
            precondition,
            mkdir_p,
        } => {
            let target = Zeroizing::new(target);
            let session = ProfileSession::open_with_control(
                &registry,
                kv_store_profile(&store),
                timeout,
                cancellation,
            )?;
            with_vault(state_dir, &session, |session, vault| {
                let credentials = ClientCredentials::open(state_dir)?;
                let master = credentials.master_key()?;
                let report = match &store {
                    KvStoreRef::Account(store) => session.put_kv_symlink_checked(
                        &store.account_alias,
                        &path,
                        &target,
                        wire_precondition(precondition),
                        wire_role_to_app(read_role),
                        wire_role_to_app(write_role),
                        mkdir_p,
                        vault,
                        &master,
                    )?,
                    KvStoreRef::Team(store) => session.put_team_kv_symlink_checked(
                        &store.account_alias,
                        &store.team_alias,
                        &store.team_id,
                        &path,
                        &target,
                        wire_precondition(precondition),
                        wire_role_to_app(read_role),
                        wire_role_to_app(write_role),
                        mkdir_p,
                        vault,
                        &master,
                    )?,
                };
                Ok(serde_json::to_value(report)?)
            })
        }
        Operation::MkdirKv {
            store,
            path,
            read_role,
            write_role,
            precondition,
            mkdir_p,
        } => {
            let session = ProfileSession::open_with_control(
                &registry,
                kv_store_profile(&store),
                timeout,
                cancellation,
            )?;
            with_vault(state_dir, &session, |session, vault| {
                let credentials = ClientCredentials::open(state_dir)?;
                let master = credentials.master_key()?;
                let report = match &store {
                    KvStoreRef::Account(store) => session.mkdir_kv_checked(
                        &store.account_alias,
                        &path,
                        wire_precondition(precondition),
                        wire_role_to_app(read_role),
                        wire_role_to_app(write_role),
                        mkdir_p,
                        vault,
                        &master,
                    )?,
                    KvStoreRef::Team(store) => session.mkdir_team_kv_checked(
                        &store.account_alias,
                        &store.team_alias,
                        &store.team_id,
                        &path,
                        wire_precondition(precondition),
                        wire_role_to_app(read_role),
                        wire_role_to_app(write_role),
                        mkdir_p,
                        vault,
                        &master,
                    )?,
                };
                Ok(serde_json::to_value(report)?)
            })
        }
        Operation::RemoveKv {
            store,
            path,
            recursive,
            precondition,
        } => {
            let KvPrecondition::ExactVersion { version } = precondition else {
                return Err(Box::new(AgentRequestError(
                    "KV removal requires an exact-version precondition",
                )));
            };
            let session = ProfileSession::open_with_control(
                &registry,
                kv_store_profile(&store),
                timeout,
                cancellation,
            )?;
            with_vault(state_dir, &session, |session, vault| {
                let credentials = ClientCredentials::open(state_dir)?;
                let master = credentials.master_key()?;
                let report = match &store {
                    KvStoreRef::Account(store) => session.remove_kv_checked(
                        &store.account_alias,
                        &path,
                        recursive,
                        version,
                        vault,
                        &master,
                    )?,
                    KvStoreRef::Team(store) => session.remove_team_kv_checked(
                        &store.account_alias,
                        &store.team_alias,
                        &store.team_id,
                        &path,
                        recursive,
                        version,
                        vault,
                        &master,
                    )?,
                };
                Ok(serde_json::to_value(report)?)
            })
        }
        Operation::CreateTeam {
            profile,
            account_alias,
            team_alias,
            name,
            kind,
        } => {
            if team_alias.trim().is_empty()
                || account_alias.trim().is_empty()
                || matches!(kind, TeamKind::Named) && name.trim().is_empty()
                || matches!(kind, TeamKind::AdHoc) && !name.is_empty()
            {
                return Err(Box::new(AgentRequestError(
                    "team creation requires account and team aliases; a team name must be provided only for named teams",
                )));
            }
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
                let report = match kind {
                    TeamKind::Named => session.create_named_team(
                        &account_alias,
                        &team_alias,
                        &name,
                        vault,
                        master,
                    )?,
                    TeamKind::AdHoc => {
                        session.create_adhoc_team(&account_alias, &team_alias, vault, master)?
                    }
                };
                Ok(serde_json::to_value(report)?)
            })
        }
        Operation::ResumeTeamCreation {
            profile,
            team_alias,
        } => {
            if team_alias.trim().is_empty() {
                return Err(Box::new(AgentRequestError(
                    "resuming team creation requires a team alias",
                )));
            }
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
                Ok(serde_json::to_value(session.resume_team_creation(
                    &team_alias,
                    vault,
                    master,
                )?)?)
            })
        }
        Operation::ListTeams { profile } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                let teams = session.list_teams(vault)?;
                let known = teams
                    .iter()
                    .map(|team| KnownTeamStore {
                        account_alias: team.account_alias.clone(),
                        team_alias: team.alias.clone(),
                        team_id_hex: team.team_id_hex.clone(),
                        team_kind: team.kind.clone(),
                        display_name: team.name.clone(),
                        active: team.active,
                    })
                    .collect::<Vec<_>>();
                retain_known_teams(session, &profile, &known);
                Ok(serde_json::to_value(teams)?)
            })
        }
        Operation::DiscoverTeams {
            profile,
            account_alias,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(
                    session.discover_teams(&account_alias, vault)?,
                )?)
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
        Operation::ListTeamDetails {
            profile,
            team_alias,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                let members = match session.list_team_members(&team_alias, vault) {
                    Ok(members) => ResponseResult::Success {
                        value: serde_json::to_value(members)?,
                    },
                    Err(error) => dispatch_error_response(0, &error).result,
                };
                let federation = match session.list_federated_memberships(&team_alias, vault) {
                    Ok(federation) => ResponseResult::Success {
                        value: serde_json::to_value(federation)?,
                    },
                    Err(error) => dispatch_error_response(0, &error).result,
                };
                Ok(serde_json::to_value(TeamDetailsSummary {
                    members,
                    federation,
                })?)
            })
        }
        Operation::ListTeamMembers {
            profile,
            team_alias,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(
                    session.list_team_members(&team_alias, vault)?,
                )?)
            })
        }
        Operation::AddTeamMember {
            profile,
            team_alias,
            username,
            role,
            visibility,
        } => {
            let destination = team_destination(role, visibility)?;
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
                Ok(serde_json::to_value(session.add_local_team_member(
                    &team_alias,
                    &username,
                    destination,
                    vault,
                    master,
                )?)?)
            })
        }
        Operation::ResumeTeamMemberAddition {
            profile,
            team_alias,
            username,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
                Ok(serde_json::to_value(
                    session.resume_local_team_member_addition(
                        &team_alias,
                        &username,
                        vault,
                        master,
                    )?,
                )?)
            })
        }
        Operation::DemoteTeamMember {
            profile,
            team_alias,
            party_id_hex,
            role,
            visibility,
        } => {
            let destination = team_destination(role, visibility)?;
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
                let credentials = ClientCredentials::open(state_dir)?;
                Ok(serde_json::to_value(
                    session.demote_local_team_member_in_authenticated_roster(
                        &team_alias,
                        &party_id_hex,
                        destination,
                        vault,
                        &registry,
                        &credentials,
                        master,
                    )?,
                )?)
            })
        }
        Operation::RemoveTeamMember {
            profile,
            team_alias,
            party_id_hex,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
                let credentials = ClientCredentials::open(state_dir)?;
                Ok(serde_json::to_value(
                    session.remove_local_team_member_in_authenticated_roster(
                        &team_alias,
                        &party_id_hex,
                        vault,
                        &registry,
                        &credentials,
                        master,
                    )?,
                )?)
            })
        }
        Operation::ResumeTeamMemberEdit {
            profile,
            team_alias,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
                let credentials = ClientCredentials::open(state_dir)?;
                Ok(serde_json::to_value(
                    session.resume_local_team_member_edit_in_authenticated_roster(
                        &team_alias,
                        vault,
                        &registry,
                        &credentials,
                        master,
                    )?,
                )?)
            })
        }
        Operation::AdmitFederatedTeam {
            local_profile,
            local_team_alias,
            remote_profile,
            remote_team_alias,
            role,
            visibility,
        } => {
            let destination = federation_destination(role, visibility)?;
            let local = ProfileSession::open_with_control(
                &registry,
                &local_profile,
                timeout,
                cancellation.clone(),
            )?;
            let remote = ProfileSession::open_with_control(
                &registry,
                &remote_profile,
                timeout,
                cancellation,
            )?;
            let credentials = ClientCredentials::open(state_dir)?;
            credentials.with_checked_sessions(&local, &remote, |local, remote| {
                let master = credentials.master_key()?;
                let mut local_store = EncryptedFileSecretStore::open(
                    &local.paths().credential_store,
                    derive_vault_key(&master),
                )?;
                let mut remote_store = EncryptedFileSecretStore::open(
                    &remote.paths().credential_store,
                    derive_vault_key(&master),
                )?;
                Ok(serde_json::to_value(local.admit_federated_team(
                    remote,
                    &local_team_alias,
                    &remote_team_alias,
                    destination,
                    &mut AccountVault::new(&mut local_store),
                    &mut AccountVault::new(&mut remote_store),
                    &master,
                )?)?)
            })
        }
        Operation::ListFederatedTeams {
            profile,
            team_alias,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(
                    session.list_federated_memberships(&team_alias, vault)?,
                )?)
            })
        }
        Operation::ExpelFederatedTeam {
            profile,
            team_alias,
            remote_host_id_hex,
            remote_team_id_hex,
        } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
                let credentials = ClientCredentials::open(state_dir)?;
                Ok(serde_json::to_value(session.expel_federated_team(
                    &team_alias,
                    &remote_host_id_hex,
                    &remote_team_id_hex,
                    vault,
                    &registry,
                    &credentials,
                    master,
                )?)?)
            })
        }
        Operation::RefreshFederatedSecurity {
            profile,
            team_alias,
            local_yubi_alias,
            local_pin,
            remote_profile,
            remote_yubi_alias,
            remote_pin,
            unlocks,
        } => {
            let session = ProfileSession::open_with_control(
                &registry,
                &profile,
                timeout,
                cancellation.clone(),
            )?;
            refresh_federated_security(
                state_dir,
                &registry,
                &session,
                &profile,
                RefreshFederatedSecurityArguments {
                    team_alias,
                    local_yubi_alias,
                    local_pin,
                    remote_profile,
                    remote_yubi_alias,
                    remote_pin,
                    unlocks,
                },
                timeout,
                cancellation,
            )
        }
        Operation::RunDueJobs { profile } => {
            let session =
                ProfileSession::open_with_control(&registry, &profile, timeout, cancellation)?;
            let credentials = ClientCredentials::open(state_dir)?;
            checked_session(&credentials, &session, |session| {
                let master = credentials.master_key()?;
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    derive_vault_key(&master),
                )?;
                Ok(serde_json::to_value(
                    session.run_due_jobs_with_federation(
                        now_microseconds()?,
                        &mut AccountVault::new(&mut store),
                        &registry,
                        &credentials,
                        &master,
                    )?,
                )?)
            })
        }
    }
}

fn federation_destination(
    role: foks_agent_proto::FederationRole,
    visibility: i16,
) -> Result<FederationDestinationRole, Box<dyn std::error::Error>> {
    match role {
        foks_agent_proto::FederationRole::Member => {
            Ok(FederationDestinationRole::Member { visibility })
        }
        foks_agent_proto::FederationRole::Admin | foks_agent_proto::FederationRole::Owner => {
            Err("federated teams can only hold member roles".into())
        }
    }
}

fn team_destination(
    role: TeamRole,
    visibility: i16,
) -> Result<TeamMemberRole, Box<dyn std::error::Error>> {
    match role {
        TeamRole::Member => Ok(TeamMemberRole::Member { visibility }),
        TeamRole::Admin if visibility == 0 => Ok(TeamMemberRole::Admin),
        TeamRole::Owner if visibility == 0 => Ok(TeamMemberRole::Owner),
        TeamRole::Admin | TeamRole::Owner => Err("visibility applies only to member roles".into()),
    }
}

struct RefreshFederatedSecurityArguments {
    team_alias: String,
    local_yubi_alias: Option<String>,
    local_pin: Option<foks_agent_proto::SecretString>,
    remote_profile: Option<String>,
    remote_yubi_alias: Option<String>,
    remote_pin: Option<foks_agent_proto::SecretString>,
    unlocks: Vec<foks_agent_proto::YubiFederationUnlockInput>,
}

fn requested_federation_unlocks(
    profile: &str,
    arguments: &RefreshFederatedSecurityArguments,
) -> foks_client_app::Result<Vec<(String, String, Pin)>> {
    let mut requested = Vec::new();
    match (
        arguments.local_yubi_alias.as_deref(),
        arguments.local_pin.as_ref(),
    ) {
        (Some(alias), Some(pin)) => requested.push((
            profile.to_owned(),
            alias.to_owned(),
            Pin::new(pin.expose())?,
        )),
        (None, None) => {}
        _ => {
            return Err(foks_client_app::Error::InvalidConfig(
                "local Yubi alias and PIN must be supplied together",
            ))
        }
    }
    match (
        arguments.remote_profile.as_deref(),
        arguments.remote_yubi_alias.as_deref(),
        arguments.remote_pin.as_ref(),
    ) {
        (Some(remote_profile), Some(alias), Some(pin)) => requested.push((
            remote_profile.to_owned(),
            alias.to_owned(),
            Pin::new(pin.expose())?,
        )),
        (None, None, None) => {}
        _ => {
            return Err(foks_client_app::Error::InvalidConfig(
                "remote profile, Yubi alias, and PIN must be supplied together",
            ))
        }
    }
    for unlock in &arguments.unlocks {
        requested.push((
            unlock.profile.clone(),
            unlock.alias.clone(),
            Pin::new(unlock.pin.expose())?,
        ));
    }
    Ok(requested)
}

/// Runs one federated security responder with a YubiKey unlocked on any
/// number of the profiles the refresh will touch.
///
/// Every profile's durable Yubi record is read under its own short-lived
/// checked session, all of which are released before the refresh takes the
/// local one. The refresh re-acquires the other profiles without waiting, so
/// holding their operation locks across it would report each as permanently
/// busy. The record only names the device; the credential built from it is
/// re-checked against the authenticated user chain at every use. PINs are
/// used to open the devices and are never persisted.
fn refresh_federated_security(
    state_dir: &Path,
    registry: &ProfileRegistry,
    session: &ProfileSession,
    profile: &str,
    arguments: RefreshFederatedSecurityArguments,
    timeout: Duration,
    cancellation: CancellationToken,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let provider = HardwareYubiProvider::new();
    // Check required capabilities before prompting for hardware credentials
    // or consuming PIN attempts.
    session.profile().require(Capability::Teams)?;
    session.profile().require(Capability::Federation)?;
    let credentials = ClientCredentials::open(state_dir)?;
    let master = credentials.master_key()?;

    let requested = requested_federation_unlocks(profile, &arguments)?;

    let mut opened = Vec::new();
    for (unlock_profile, alias, pin) in requested {
        let unlock_session = ProfileSession::open_with_control(
            registry,
            &unlock_profile,
            timeout,
            cancellation.clone(),
        )?;
        unlock_session.profile().require(Capability::Teams)?;
        unlock_session.profile().require(Capability::Federation)?;
        let loaded = checked_session(&credentials, &unlock_session, |checked| {
            let mut store = EncryptedFileSecretStore::open(
                &checked.paths().credential_store,
                derive_vault_key(&master),
            )?;
            Ok(AccountVault::new(&mut store).yubi_account(&alias)?)
        })?;
        let device = provider.open(&loaded.locator, Some(&pin))?;
        opened.push((unlock_profile, alias, loaded, device));
    }
    let unlocked_credentials = opened
        .iter()
        .map(|(_, _, loaded, device)| loaded.credential(device.as_ref()))
        .collect::<Vec<_>>();
    let actors = opened
        .iter()
        .zip(&unlocked_credentials)
        .map(
            |((unlock_profile, alias, _, _), credential)| UnlockedYubiActor {
                profile: unlock_profile,
                alias,
                credential,
            },
        )
        .collect::<Vec<_>>();

    checked_session(&credentials, session, |session| {
        let mut store = EncryptedFileSecretStore::open(
            &session.paths().credential_store,
            derive_vault_key(&master),
        )?;
        let report = session.refresh_federated_security_with_unlocked_yubi(
            &arguments.team_alias,
            &actors,
            &mut AccountVault::new(&mut store),
            registry,
            &credentials,
            &master,
        )?;
        Ok(serde_json::to_value(report)?)
    })
}

fn retain_known_accounts(session: &CheckedProfileSession<'_>, profile: &str, aliases: &[String]) {
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let observed_at = now_microseconds()? / 1_000_000;
        SoftStateStore::open(&session.paths().soft_database)?
            .replace_known_accounts(aliases, observed_at)?;
        Ok(())
    })();
    if let Err(error) = result {
        eprintln!("foks-agent could not retain known account stores for {profile}: {error}");
    }
}

fn retain_known_teams(
    session: &CheckedProfileSession<'_>,
    profile: &str,
    teams: &[KnownTeamStore],
) {
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let observed_at = now_microseconds()? / 1_000_000;
        SoftStateStore::open(&session.paths().soft_database)?
            .replace_known_teams(teams, observed_at)?;
        Ok(())
    })();
    if let Err(error) = result {
        eprintln!("foks-agent could not retain known team stores for {profile}: {error}");
    }
}

fn wire_known_store(
    store: KnownStore,
) -> Result<WireKnownStoreSummary, Box<dyn std::error::Error>> {
    Ok(match store {
        KnownStore::Account {
            account_alias,
            last_seen_at,
        } => WireKnownStoreSummary::Account {
            account_alias,
            last_seen_at,
        },
        KnownStore::Team {
            store,
            last_seen_at,
        } => WireKnownStoreSummary::Team {
            account_alias: store.account_alias,
            team_alias: store.team_alias,
            team_id_hex: store.team_id_hex,
            team_kind: match store.team_kind.as_str() {
                "named" => TeamKind::Named,
                "ad-hoc" => TeamKind::AdHoc,
                _ => return Err(Box::new(AgentRequestError("known team kind is invalid"))),
            },
            name: store.display_name,
            active: store.active,
            last_seen_at,
        },
    })
}

fn go_candidate_for_session(
    candidate_id: &str,
    session: &CheckedProfileSession<'_>,
) -> Result<foks_go_interop::ResolvedCandidate, Box<dyn std::error::Error>> {
    let root = foks_go_interop::standard_root().ok_or("Go FOKS home is unavailable")?;
    let candidate = foks_go_interop::resolve(&root, candidate_id)?;
    if !candidate.summary.pairable {
        return Err("Go FOKS profile is not pairable".into());
    }
    if session.pinned_host()?.host_id().as_bytes() != candidate.host_id() {
        return Err("Go FOKS profile belongs to a different server".into());
    }
    Ok(candidate)
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
    checked_session(&credentials, session, |session| {
        let master = credentials.master_key()?;
        let mut store = EncryptedFileSecretStore::open(
            &session.paths().credential_store,
            derive_vault_key(&master),
        )?;
        operation(session, &mut AccountVault::new(&mut store))
    })
}

fn with_vault_and_master(
    state_dir: &Path,
    session: &ProfileSession,
    operation: impl FnOnce(
        &CheckedProfileSession<'_>,
        &mut AccountVault<'_>,
        &[u8; 32],
    ) -> Result<serde_json::Value, Box<dyn std::error::Error>>,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let credentials = ClientCredentials::open(state_dir)?;
    checked_session(&credentials, session, |session| {
        let master = credentials.master_key()?;
        let mut store = EncryptedFileSecretStore::open(
            &session.paths().credential_store,
            derive_vault_key(&master),
        )?;
        operation(session, &mut AccountVault::new(&mut store), &master)
    })
}

fn checked_session<T>(
    credentials: &ClientCredentials,
    session: &ProfileSession,
    operation: impl FnOnce(&CheckedProfileSession<'_>) -> Result<T, Box<dyn std::error::Error>>,
) -> Result<T, Box<dyn std::error::Error>> {
    credentials
        .try_with_checked_session(session, operation)?
        .ok_or_else(|| Box::new(ProfileBusyError) as Box<dyn std::error::Error>)
}

fn yubi_card(
    provider: &HardwareYubiProvider,
    serial: u32,
) -> Result<CardId, Box<dyn std::error::Error>> {
    let matches = provider
        .cards()?
        .into_iter()
        .filter(|card| card.serial == serial)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [card] => Ok(card.clone()),
        [] => Err(format!("YubiKey serial {serial} is not connected").into()),
        _ => Err(format!("YubiKey serial {serial} is ambiguous").into()),
    }
}

#[cfg(test)]
fn dispatch(state_dir: &Path, request: Request) -> Response {
    dispatch_controlled(
        state_dir,
        request,
        Duration::from_secs(15),
        CancellationToken::new(),
        Arc::new(AtomicBool::new(true)),
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
fn bind_private_agent_socket(path: &Path) -> std::io::Result<tokio::net::UnixListener> {
    use std::os::unix::fs::PermissionsExt as _;

    let staging = path.with_extension("bind");
    let _ = std::fs::remove_file(&staging);
    let listener = tokio::net::UnixListener::bind(&staging)?;
    if let Err(error) = std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o600)) {
        let _ = std::fs::remove_file(&staging);
        return Err(error);
    }
    // rename publishes the already-private socket atomically into place,
    // which works across all UNIX platforms including macOS/Darwin where
    // hard linking domain sockets returns EPERM.
    if let Err(error) = std::fs::rename(&staging, path) {
        let _ = std::fs::remove_file(&staging);
        return Err(error);
    }
    Ok(listener)
}

#[cfg(unix)]
#[derive(Debug)]
struct AgentLock(std::fs::File);

#[cfg(unix)]
impl AgentLock {
    fn acquire(state_dir: &Path) -> std::io::Result<Self> {
        use fs2::FileExt as _;
        use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};

        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(state_dir.join(AGENT_LOCK_NAME))?;
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & 0o077 != 0
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "agent lock is not private and owned by this user",
            ));
        }
        file.try_lock_exclusive().map_err(|error| {
            if error.kind() == std::io::ErrorKind::WouldBlock {
                std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    "another agent owns this state directory",
                )
            } else {
                error
            }
        })?;
        Ok(Self(file))
    }
}

#[cfg(unix)]
impl Drop for AgentLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

#[cfg(unix)]
fn remove_stale_agent_socket(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _};

    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_socket()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o777 != 0o600
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "agent socket path is not a private socket owned by this user",
        ));
    }
    std::fs::remove_file(path)
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
    fn unsupported_soft_schema_recovery_survives_agent_error_mapping() {
        let error =
            foks_client_app::Error::ClientDatabase(foks_client_db::Error::UnsupportedSoftSchema {
                path: "/private/foks/profiles/local/soft.sqlite3".to_owned(),
                found: 2,
                supported: 4,
            });
        let response = dispatch_error_response(7, &error);
        let foks_agent_proto::ResponseResult::Error { code, message, .. } = response.result else {
            panic!("schema failure returned success");
        };
        assert_eq!(code, ErrorCode::OperationFailed);
        assert!(message.contains("unsupported soft-state cache schema version"));
        assert!(message.contains("/private/foks/profiles/local/soft.sqlite3"));
        assert!(message.contains("cache must be recreated"));
    }

    #[test]
    fn remote_capacity_statuses_keep_retry_semantics_and_diagnostics() {
        for (status, expected, message) in [
            (
                foks_rpc::STATUS_RATE_LIMIT_ERROR,
                ErrorCode::RateLimited,
                "Wait briefly",
            ),
            (
                foks_rpc::STATUS_OVER_QUOTA_ERROR,
                ErrorCode::QuotaExceeded,
                "capacity limit",
            ),
        ] {
            let response = remote_status_response(8, status, format!("remote status {status}"))
                .expect("known status maps");
            let foks_agent_proto::ResponseResult::Error {
                code,
                message: mapped,
                fields,
            } = response.result
            else {
                panic!("remote status returned success");
            };
            assert_eq!(code, expected);
            assert!(mapped.contains(message));
            assert_eq!(fields.reason, Some(format!("remote status {status}")));
        }
        assert!(remote_status_response(8, 9999, "unknown".to_owned()).is_none());
    }

    #[test]
    fn known_ad_hoc_store_keeps_its_wire_kind() {
        let wire = wire_known_store(KnownStore::Team {
            store: KnownTeamStore {
                account_alias: "personal".to_owned(),
                team_alias: "share".to_owned(),
                team_id_hex: "14aa".to_owned(),
                team_kind: "ad-hoc".to_owned(),
                display_name: Some("Share".to_owned()),
                active: true,
            },
            last_seen_at: 1,
        })
        .unwrap();
        assert_eq!(serde_json::to_value(wire).unwrap()["team_kind"], "ad-hoc");
    }

    #[test]
    fn agent_lifetime_lock_is_exclusive_and_reusable() {
        let directory = tempfile::tempdir().unwrap();
        let first = AgentLock::acquire(directory.path()).unwrap();
        assert!(directory.path().join(AGENT_LOCK_NAME).is_file());
        assert_eq!(
            AgentLock::acquire(directory.path()).unwrap_err().kind(),
            std::io::ErrorKind::AddrInUse
        );
        drop(first);
        AgentLock::acquire(directory.path()).unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn private_socket_bind_accepts_maximum_length_path() {
        use std::os::unix::ffi::OsStrExt as _;

        #[cfg(target_os = "linux")]
        const UNIX_SOCKET_PATH_MAX: usize = 107;
        #[cfg(not(target_os = "linux"))]
        const UNIX_SOCKET_PATH_MAX: usize = 103;

        let directory = tempfile::Builder::new()
            .prefix("fa")
            .tempdir_in("/tmp")
            .unwrap();
        let suffix = ".sock";
        let stem_length =
            UNIX_SOCKET_PATH_MAX - directory.path().as_os_str().as_bytes().len() - 1 - suffix.len();
        let path = directory
            .path()
            .join(format!("{}{suffix}", "s".repeat(stem_length)));
        assert_eq!(path.as_os_str().as_bytes().len(), UNIX_SOCKET_PATH_MAX);

        let listener = bind_private_agent_socket(&path).unwrap();
        assert!(path.exists());
        drop(listener);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn stale_socket_cleanup_refuses_non_socket_paths() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agent.sock");
        std::fs::write(&path, b"not a socket").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            remove_stale_agent_socket(&path).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        assert!(path.exists());
    }

    #[test]
    fn streaming_reader_requires_an_exact_commit_and_detects_disconnect() {
        use std::io::Read as _;

        let (sender, receiver) = tokio::sync::mpsc::channel(4);
        sender
            .blocking_send(UploadMessage::Chunk(Zeroizing::new(b"abcd".to_vec())))
            .unwrap();
        sender.blocking_send(UploadMessage::Commit).unwrap();
        drop(sender);
        let mut output = Vec::new();
        UploadReader::new(receiver, 4)
            .read_to_end(&mut output)
            .unwrap();
        assert_eq!(output, b"abcd");

        let (sender, receiver) = tokio::sync::mpsc::channel(4);
        sender
            .blocking_send(UploadMessage::Chunk(Zeroizing::new(b"abc".to_vec())))
            .unwrap();
        drop(sender);
        let error = UploadReader::new(receiver, 4)
            .read_to_end(&mut Vec::new())
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);

        let (sender, receiver) = tokio::sync::mpsc::channel(4);
        sender.blocking_send(UploadMessage::Commit).unwrap();
        let error = UploadReader::new(receiver, 1)
            .read_to_end(&mut Vec::new())
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn bootstrap_allows_only_status_and_initialization_then_becomes_ready() {
        assert!(operation_allowed(false, &Operation::AgentStatus));
        assert!(operation_allowed(
            false,
            &Operation::InitializeState {
                backend: WireCredentialBackend::PrivateFile,
            }
        ));
        assert!(!operation_allowed(false, &Operation::Ping));
        assert!(operation_allowed(true, &Operation::Ping));

        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join("state");
        ProfileRegistry::open(&state).unwrap();
        let ready = Arc::new(AtomicBool::new(false));
        let status = dispatch_controlled(
            &state,
            Request::new(1, Operation::AgentStatus),
            Duration::from_secs(15),
            CancellationToken::new(),
            ready.clone(),
        );
        assert!(matches!(
            status.result,
            foks_agent_proto::ResponseResult::Success { value }
                if serde_json::from_value::<AgentStatus>(value.clone()).unwrap()
                    == AgentStatus::Bootstrap { step: "initialize-state".to_owned() }
        ));

        let initialized = dispatch_controlled(
            &state,
            Request::new(
                2,
                Operation::InitializeState {
                    backend: WireCredentialBackend::PrivateFile,
                },
            ),
            Duration::from_secs(15),
            CancellationToken::new(),
            ready.clone(),
        );
        assert!(matches!(
            initialized.result,
            foks_agent_proto::ResponseResult::Success { .. }
        ));
        assert!(ready.load(Ordering::Acquire));
        assert!(ClientCredentials::is_initialized(&state).unwrap());

        ready.store(false, Ordering::Release);
        let resumed = dispatch_controlled(
            &state,
            Request::new(
                3,
                Operation::InitializeState {
                    // Durable state wins over a stale caller preference.
                    backend: WireCredentialBackend::Native,
                },
            ),
            Duration::from_secs(15),
            CancellationToken::new(),
            ready.clone(),
        );
        assert!(matches!(
            resumed.result,
            foks_agent_proto::ResponseResult::Success { ref value }
                if value["backend"] == "private-file"
        ));
        assert!(ready.load(Ordering::Acquire));

        let added = dispatch_controlled(
            &state,
            Request::new(
                4,
                Operation::AddProfile {
                    name: "local".to_owned(),
                    probe: "foks.example:443".to_owned(),
                    protocol: ProfileProtocol::V019,
                    trust: ProfileTrust::WebPki,
                },
            ),
            Duration::from_secs(15),
            CancellationToken::new(),
            ready.clone(),
        );
        assert!(matches!(
            added.result,
            foks_agent_proto::ResponseResult::Success { .. }
        ));
        assert_eq!(
            ProfileRegistry::open(&state)
                .unwrap()
                .profile("local")
                .unwrap()
                .probe,
            "foks.example:443"
        );

        let status = dispatch_controlled(
            &state,
            Request::new(4, Operation::AgentStatus),
            Duration::from_secs(15),
            CancellationToken::new(),
            ready,
        );
        assert!(matches!(
            status.result,
            foks_agent_proto::ResponseResult::Success { value }
                if serde_json::from_value::<AgentStatus>(value.clone()).unwrap() == AgentStatus::Ready
        ));
    }

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
    fn forgetting_a_profile_erases_its_local_state() {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join("state");
        ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&state).unwrap();
        registry
            .add(Profile {
                name: "local".to_owned(),
                probe: "foks.app".to_owned(),
                protocol: ProtocolPolicy::V019,
                trust: TrustRoot::WebPki,
            })
            .unwrap();
        let paths = registry.paths("local").unwrap();
        std::fs::create_dir_all(&paths.credential_store).unwrap();
        std::fs::write(paths.credential_store.join("account.personal"), b"key").unwrap();

        let forgotten = dispatch(
            &state,
            Request::new(
                1,
                Operation::RemoveProfile {
                    name: "local".to_owned(),
                },
            ),
        );
        assert!(matches!(
            forgotten.result,
            foks_agent_proto::ResponseResult::Success { value }
                if value["profile"] == "local" && value["removed"] == true
        ));
        // Verify that profile removal deletes local credential keys from disk.
        assert!(!paths.directory.exists());
        assert!(ProfileRegistry::open(&state)
            .unwrap()
            .profile("local")
            .is_err());
    }

    #[test]
    fn reset_tokens_are_one_use_profile_bound_and_recheck_the_preview() {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join("state");
        ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&state).unwrap();
        registry
            .add(Profile {
                name: "local".to_owned(),
                probe: "foks.app".to_owned(),
                protocol: ProtocolPolicy::V019,
                trust: TrustRoot::WebPki,
            })
            .unwrap();

        let described = dispatch(
            &state,
            Request::new(
                10,
                Operation::DescribeResetHardState {
                    profile: "local".to_owned(),
                },
            ),
        );
        let foks_agent_proto::ResponseResult::Success { value } = described.result else {
            panic!("reset preview failed");
        };
        let preview: WireResetStatePreview = serde_json::from_value(value).unwrap();
        assert_eq!(preview.profile, "local");
        assert_eq!(preview.token.expose().len(), 64);

        // A state change after the preview is a conflict, and presenting the
        // ticket consumed it even though the reset did not run.
        let session = ProfileSession::open(&registry, "local").unwrap();
        foks_client_db::HardStateStore::open(&session.paths().soft_database).unwrap();
        let reset = dispatch(
            &state,
            Request::new(
                11,
                Operation::ResetHardState {
                    profile: "local".to_owned(),
                    token: preview.token.clone(),
                },
            ),
        );
        assert!(matches!(
            reset.result,
            foks_agent_proto::ResponseResult::Error {
                code: ErrorCode::Conflict,
                ..
            }
        ));
        let repeated = dispatch(
            &state,
            Request::new(
                12,
                Operation::ResetHardState {
                    profile: "local".to_owned(),
                    token: preview.token,
                },
            ),
        );
        assert!(matches!(
            repeated.result,
            foks_agent_proto::ResponseResult::Error {
                code: ErrorCode::InvalidRequest,
                ..
            }
        ));

        let described = dispatch(
            &state,
            Request::new(
                13,
                Operation::DescribeResetHardState {
                    profile: "local".to_owned(),
                },
            ),
        );
        let foks_agent_proto::ResponseResult::Success { value } = described.result else {
            panic!("second reset preview failed");
        };
        let preview: WireResetStatePreview = serde_json::from_value(value).unwrap();
        let reset = dispatch(
            &state,
            Request::new(
                14,
                Operation::ResetHardState {
                    profile: "local".to_owned(),
                    token: preview.token,
                },
            ),
        );
        assert!(matches!(
            reset.result,
            foks_agent_proto::ResponseResult::Success { value }
                if value["hard_state_reset"] == true
        ));
        assert!(!session.paths().soft_database.exists());

        let token = issue_reset_ticket(&state, "local", [9; 32]).unwrap();
        assert!(consume_reset_ticket(&state, "other", token.expose()).is_err());
        assert!(consume_reset_ticket(&state, "local", token.expose()).is_err());
    }

    #[test]
    fn simultaneous_reset_ticket_consumers_allow_exactly_one_winner() {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().to_owned();
        let token = Arc::new(issue_reset_ticket(&state, "local", [3; 32]).unwrap());
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let mut workers = Vec::new();
        for _ in 0..2 {
            let state = state.clone();
            let token = Arc::clone(&token);
            let barrier = Arc::clone(&barrier);
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                consume_reset_ticket(&state, "local", token.expose()).is_ok()
            }));
        }
        barrier.wait();
        assert_eq!(
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .filter(|won| *won)
                .count(),
            1
        );
    }

    #[test]
    fn agent_signup_carries_invites_and_passphrases_across_the_product_boundary() {
        use foks_client_app::{ClientCredentials, CredentialBackend};
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
        let probed = dispatch(
            &state,
            Request::new(
                9,
                Operation::CheckAndAddProfile {
                    name: "local".to_owned(),
                    probe: format!("localhost:{}", addresses.probe.port()),
                    protocol: ProfileProtocol::V019,
                    trust: ProfileTrust::CertificateDer {
                        path: root.display().to_string(),
                    },
                },
            ),
        );
        let foks_agent_proto::ResponseResult::Success { value: publication } = &probed.result
        else {
            panic!("unexpected checked profile publication response: {probed:?}");
        };
        assert_eq!(publication["profile"]["name"], "local");
        assert_eq!(publication["probe"]["acceptance"], "inserted");
        assert_eq!(publication["probe"]["lookup_name"], "localhost");
        assert!(publication["probe"]["canonical_name"]
            .as_str()
            .is_some_and(|name| !name.is_empty()));
        assert!(publication["probe"]["host_id_hex"]
            .as_str()
            .is_some_and(|host_id| host_id.len() == 66));

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
                    invite: foks_agent_proto::SecretString::new(invite.code.clone()),
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

        let listed_devices = dispatch(
            &state,
            Request::new(
                18,
                Operation::ListDevices {
                    profile: "local".to_owned(),
                    alias: "personal".to_owned(),
                },
            ),
        );
        let foks_agent_proto::ResponseResult::Success { value } = listed_devices.result else {
            panic!("unexpected device-list response: {listed_devices:?}");
        };
        let devices: Vec<WireDeviceSummary> = serde_json::from_value(value).unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name.as_deref(), Some("agent laptop"));
        assert_eq!(devices[0].role, "owner");
        assert!(devices[0].current);

        let prepared = dispatch(
            &state,
            Request::new(
                19,
                Operation::PrepareOwnerBackup {
                    profile: "local".to_owned(),
                    account_alias: "personal".to_owned(),
                    backup_alias: "paper".to_owned(),
                },
            ),
        );
        let phrase = match &prepared.result {
            foks_agent_proto::ResponseResult::Success { value } => {
                value["phrase"].as_str().unwrap().to_owned()
            }
            _ => panic!("unexpected prepare response: {prepared:?}"),
        };
        assert_eq!(phrase.split_whitespace().count(), 17);
        assert!(!format!("{prepared:?}").contains(&phrase));
        let committed = dispatch(
            &state,
            Request::new(
                20,
                Operation::CommitOwnerBackup {
                    profile: "local".to_owned(),
                    account_alias: "personal".to_owned(),
                    backup_alias: "paper".to_owned(),
                    phrase: foks_agent_proto::SecretString::new(phrase),
                },
            ),
        );
        let backup_id = match &committed.result {
            foks_agent_proto::ResponseResult::Success { value } => {
                value["backup_id_hex"].as_str().unwrap().to_owned()
            }
            _ => panic!("unexpected backup commit response: {committed:?}"),
        };
        let revoked = dispatch(
            &state,
            Request::new(
                21,
                Operation::RevokeOwnerBackup {
                    profile: "local".to_owned(),
                    account_alias: "personal".to_owned(),
                    backup_alias: "paper".to_owned(),
                    backup_id: backup_id.clone(),
                },
            ),
        );
        assert!(matches!(
            revoked.result,
            foks_agent_proto::ResponseResult::Success { value }
                if value["backup_alias"] == "paper"
                    && value["backup_id_hex"] == backup_id
                    && value["removed_local_enrollment"] == true
        ));
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
                if value == serde_json::json!([{
                    "profile": "local",
                    "alias": "personal",
                    "username": "agentinvite"
                }])
        ));
        let known = dispatch(
            &state,
            Request::new(
                20,
                Operation::ListKnownStores {
                    profile: "local".to_owned(),
                },
            ),
        );
        assert!(matches!(
            known.result,
            foks_agent_proto::ResponseResult::Success { value }
                if value.as_array().is_some_and(|stores| stores.iter().any(|store| {
                    store["store_kind"] == "account" && store["account_alias"] == "personal"
                }))
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
    fn federated_refresh_rejects_partial_unlock_tuples() {
        let arguments =
            |local_yubi_alias, local_pin, remote_profile, remote_yubi_alias, remote_pin| {
                RefreshFederatedSecurityArguments {
                    team_alias: "engineering".to_owned(),
                    local_yubi_alias,
                    local_pin,
                    remote_profile,
                    remote_yubi_alias,
                    remote_pin,
                    unlocks: Vec::new(),
                }
            };

        for partial in [
            arguments(Some("local-key".to_owned()), None, None, None, None),
            arguments(
                None,
                Some(foks_agent_proto::SecretString::new("123456")),
                None,
                None,
                None,
            ),
        ] {
            assert!(matches!(
                requested_federation_unlocks("local", &partial),
                Err(foks_client_app::Error::InvalidConfig(
                    "local Yubi alias and PIN must be supplied together"
                ))
            ));
        }

        for partial in [
            arguments(
                None,
                None,
                Some("remote".to_owned()),
                Some("remote-key".to_owned()),
                None,
            ),
            arguments(
                None,
                None,
                None,
                Some("remote-key".to_owned()),
                Some(foks_agent_proto::SecretString::new("123456")),
            ),
        ] {
            assert!(matches!(
                requested_federation_unlocks("local", &partial),
                Err(foks_client_app::Error::InvalidConfig(
                    "remote profile, Yubi alias, and PIN must be supplied together"
                ))
            ));
        }

        let mut complete = arguments(
            Some("local-key".to_owned()),
            Some(foks_agent_proto::SecretString::new("123456")),
            Some("remote".to_owned()),
            Some("remote-key".to_owned()),
            Some(foks_agent_proto::SecretString::new("123456")),
        );
        complete
            .unlocks
            .push(foks_agent_proto::YubiFederationUnlockInput {
                profile: "third".to_owned(),
                alias: "third-key".to_owned(),
                pin: foks_agent_proto::SecretString::new("123456"),
            });
        assert_eq!(
            requested_federation_unlocks("local", &complete)
                .unwrap()
                .len(),
            3
        );
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

    #[tokio::test(flavor = "current_thread")]
    async fn compatibility_client_refuses_non_https_requests() {
        let client = compatibility_http_client(Duration::from_secs(1)).unwrap();
        let error = client
            .get("http://127.0.0.1:9/lease.json")
            .send()
            .await
            .unwrap_err();
        assert!(error.is_builder());
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
        let mutation_gate = Semaphore::new(1);
        let active_mutation = mutation_gate.try_acquire().unwrap();
        assert_eq!(
            apply_hosted_lease_with_gate(directory.path(), "hosted", &bytes, &mutation_gate,)
                .unwrap(),
            None
        );
        drop(active_mutation);
        assert_eq!(
            apply_hosted_lease_with_gate(directory.path(), "hosted", &bytes, &mutation_gate,)
                .unwrap(),
            Some(true)
        );
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
        let result = supervise_blocking(
            7,
            permit,
            None,
            Duration::from_millis(20),
            move |cancellation| {
                while !cancellation.is_cancelled() {
                    std::thread::sleep(Duration::from_millis(1));
                }
                worker_observed.store(true, std::sync::atomic::Ordering::Release);
                Response::success(7, serde_json::json!({ "late": true }))
            },
        )
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

    #[tokio::test(flavor = "current_thread")]
    async fn completed_mutations_hold_the_gate_until_the_response_is_released() {
        let workers = Arc::new(Semaphore::new(1));
        let mutations = Arc::new(Semaphore::new(1));
        let worker_permit = workers.clone().acquire_owned().await.unwrap();
        let mutation_permit = mutations.clone().acquire_owned().await.unwrap();
        let result = supervise_blocking(
            8,
            worker_permit,
            Some(mutation_permit),
            Duration::from_secs(1),
            |_| Response::success(8, serde_json::json!({ "ok": true })),
        )
        .await;

        assert_eq!(mutations.available_permits(), 0);
        drop(result);
        assert_eq!(mutations.available_permits(), 1);
    }

    #[test]
    fn catalog_cursors_bind_store_snapshot_and_offset() {
        let store = CatalogStoreBinding::Account {
            value: AccountStoreRef {
                profile: "local".to_owned(),
                account_alias: "personal".to_owned(),
            },
        };
        let empty = paginate_catalog(
            &foks_client_app::KvCatalogReport {
                snapshot_version: 0,
                entries: Vec::new(),
            },
            store.clone(),
            None,
            2,
        )
        .unwrap();
        assert_eq!(empty.snapshot_version, 0);
        assert!(empty.entries.is_empty());
        assert!(empty.next_cursor.is_none());
        let report = foks_client_app::KvCatalogReport {
            snapshot_version: 7,
            entries: (0..3)
                .map(|index| foks_client_app::KvCatalogEntry {
                    path: format!("/{index}"),
                    node_type: "small-file".to_owned(),
                    version: index + 1,
                    size: None,
                    read_role: foks_client_app::KvRoleSummary::Owner,
                    write_role: foks_client_app::KvRoleSummary::Owner,
                })
                .collect(),
        };
        let first = paginate_catalog(&report, store.clone(), None, 2).unwrap();
        assert_eq!(first.entries.len(), 2);
        let second =
            paginate_catalog(&report, store.clone(), first.next_cursor.as_deref(), 2).unwrap();
        assert_eq!(second.entries[0].path, "/2");
        assert!(second.next_cursor.is_none());
        cache_catalog(store.clone(), &report).unwrap();
        let cached = paginate_cached_catalog(&store, first.next_cursor.as_deref().unwrap(), 2)
            .unwrap()
            .unwrap();
        assert_eq!(cached.entries[0].path, "/2");
        assert!(cached.next_cursor.is_none());
        assert!(
            paginate_cached_catalog(&store, first.next_cursor.as_deref().unwrap(), 2,)
                .unwrap()
                .is_none()
        );

        let another_store = CatalogStoreBinding::Account {
            value: AccountStoreRef {
                profile: "local".to_owned(),
                account_alias: "other".to_owned(),
            },
        };
        assert!(
            paginate_catalog(&report, another_store, first.next_cursor.as_deref(), 2,).is_err()
        );
        let mut changed = report;
        changed.snapshot_version += 1;
        assert!(
            paginate_catalog(&changed, store.clone(), first.next_cursor.as_deref(), 2,).is_err()
        );
        changed.snapshot_version -= 1;
        changed.entries[0].path = "/same-version-different-branch".to_owned();
        assert!(paginate_catalog(&changed, store, first.next_cursor.as_deref(), 2,).is_err());

        let large_store = CatalogStoreBinding::Account {
            value: AccountStoreRef {
                profile: "local".to_owned(),
                account_alias: "large".to_owned(),
            },
        };
        let large = foks_client_app::KvCatalogReport {
            snapshot_version: 8,
            entries: (0..500)
                .map(|index| foks_client_app::KvCatalogEntry {
                    path: format!("/{index}-{}", "x".repeat(4096)),
                    node_type: "small-file".to_owned(),
                    version: index + 1,
                    size: None,
                    read_role: foks_client_app::KvRoleSummary::Owner,
                    write_role: foks_client_app::KvRoleSummary::Owner,
                })
                .collect(),
        };
        let page = paginate_catalog(&large, large_store, None, 500).unwrap();
        assert!(!page.entries.is_empty());
        assert!(page.entries.len() < 500);
        assert!(page.next_cursor.is_some());
        foks_agent_proto::encode(&Response::success(11, serde_json::to_value(page).unwrap()))
            .unwrap();
    }

    #[test]
    fn protocol_errors_preserve_actionable_app_kinds() {
        let denied =
            dispatch_error_response(9, &foks_client_app::Error::CapabilityDenied(Capability::Kv));
        assert!(matches!(
            denied.result,
            foks_agent_proto::ResponseResult::Error {
                code: ErrorCode::CapabilityDenied,
                fields: ErrorFields {
                    capability: Some(ref capability),
                    ..
                },
                ..
            } if capability == "kv"
        ));
        let invalid = dispatch_error_response(10, &AgentRequestError("invalid page"));
        assert!(matches!(
            invalid.result,
            foks_agent_proto::ResponseResult::Error {
                code: ErrorCode::InvalidRequest,
                ..
            }
        ));

        let deadline = dispatch_error_response(
            11,
            &foks_client_app::Error::Client(foks_client::Error::DeadlineExceeded),
        );
        assert!(matches!(
            deadline.result,
            foks_agent_proto::ResponseResult::Error {
                code: ErrorCode::DeadlineExceeded,
                ..
            }
        ));

        let rollback = dispatch_error_response(
            12,
            &foks_client_app::Error::Client(foks_client::Error::Database(
                foks_client_db::Error::MerkleFork { epoch: 7 },
            )),
        );
        assert!(matches!(
            rollback.result,
            foks_agent_proto::ResponseResult::Error {
                code: ErrorCode::RollbackDetected,
                fields: ErrorFields {
                    reason: Some(_),
                    ..
                },
                ..
            }
        ));

        let verified_rollback = dispatch_error_response(
            13,
            &foks_client_app::Error::Client(foks_client::Error::Verify(
                foks_verify::Error::MerkleRollback {
                    stored: 8,
                    received: 7,
                },
            )),
        );
        assert!(matches!(
            verified_rollback.result,
            foks_agent_proto::ResponseResult::Error {
                code: ErrorCode::RollbackDetected,
                ..
            }
        ));
    }
}
