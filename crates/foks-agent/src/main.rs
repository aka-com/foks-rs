#![forbid(unsafe_code)]
mod account;
mod bot_token;
mod chat;
mod chat_poll;
mod connectivity;
mod data;
mod invitations;
mod profile_work;
mod read_cache;
mod retention;
mod sso;
mod timers;
#[cfg(test)]
use chat_poll::ActiveChatPollGuard;
use chat_poll::{handle_chat_poll, ChatPollKey};

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
    PendingOperationSummary as WirePendingOperationSummary, ProfileOverview, ProfileProtocol,
    ProfileTrust, Request, ResetArtifactKind as WireResetArtifactKind,
    ResetArtifactSummary as WireResetArtifactSummary, ResetStatePreview as WireResetStatePreview,
    Response, ResponseResult, ResponseTiming, ServerStatusSnapshot as WireServerStatusSnapshot,
    StoredHostStatus as WireStoredHostStatus, TeamDetailsSummary, TeamKind, TeamRole, TeamStoreRef,
    TeamSummary as WireTeamSummary, MAXIMUM_MESSAGE_BYTES,
};
use foks_client_app::{
    derive_vault_key, AccountVault, CancellationToken, Capability, CheckedProfileSession,
    ClientCredentials, CredentialBackend, FederationDestinationRole, KexAcceptanceInput,
    KvMutationPrecondition, KvRoleSummary, Passphrase, Profile, ProfileRegistry, ProfileSession,
    ProtocolPolicy, SharedSessionOutcome, TeamMemberRole, TrustRoot, UnlockedYubiActor,
    YubiProvisionInput, YubiSignupInput,
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
/// How much of the request budget admission leaves for its reply to travel.
/// A client waits its own budget from before it connects, and the agent's
/// begins once the request has been read, so admission that waited the whole
/// budget answered after every client with the same budget had given up: the
/// definite "not started" arrived as an ambiguous timeout, and a mutation the
/// agent never began was reported as one that may have applied. Admission
/// and the wait for a worker end this much earlier; a budget too short to
/// spare it keeps a quarter for the work.
const ADMISSION_REPLY_MARGIN: Duration = Duration::from_secs(2);
const MAXIMUM_CANARY_BYTES: usize = 64 * 1024;
const MAXIMUM_CANARY_FETCHES: usize = 4;
const MAXIMUM_CONCURRENT_READS: usize = 4;
const MAXIMUM_CONCURRENT_CHAT_POLLS: usize = 32;
const CHAT_POLL_TIMEOUT: Duration = Duration::from_secs(60);
const DEVICE_PAIRING_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// Work an interactive pairing still has to do after its last relay receive:
/// the authenticated chain load, the identity mutation and the wait for the
/// user chain to carry the new device. Reserved out of the pairing cap so the
/// client's relay waits end before the agent cancels the operation, which
/// would report a pairing that is merely slow as an ambiguous deadline error.
const PAIRING_COMPLETION_MARGIN: Duration = Duration::from_secs(45);
// Byte payloads are base64 on the wire, so the frame cost of a chunk is
// `ceil(n/3)*4` and the bound that keeps it inside the 1 MiB frame, with the
// envelope reserve held back, is the protocol's own. Both limits are that
// bound: a longer chunk request or a larger inline put is refused as an
// invalid request, never answered with a shortened payload.
const MAXIMUM_LOCAL_KV_CHUNK_BYTES: usize = foks_agent_proto::MAXIMUM_KV_PAYLOAD_BYTES;
const MAXIMUM_INLINE_KV_BYTES: usize = foks_agent_proto::MAXIMUM_KV_PAYLOAD_BYTES;
const MAXIMUM_STREAM_KV_BYTES: u64 = 1024 * 1024 * 1024;
/// A ceiling on frames, not on bytes. The declared total length and the
/// running offset check are what bound an upload's size; this only stops a
/// peer from holding the connection open with an unbounded frame count. It is
/// deliberately not derived from the chunk bound, because a sender is free to
/// send frames smaller than that bound and must not be cut off for it.
const MAXIMUM_STREAM_FRAMES: usize = 8193;
/// Bounds the retained catalog reports. The desktop walks four profiles at a
/// time and each profile holds several stores, so a bound near the old four
/// evicted one profile's entries while another profile was still paginating.
const MAXIMUM_CACHED_CATALOGS: usize = 32;
const MAXIMUM_CACHED_CATALOG_BYTES: usize = 128 * 1024 * 1024;
const MAXIMUM_SINGLE_CATALOG_BYTES: usize = 64 * 1024 * 1024;
const CATALOG_CACHE_LIFETIME: Duration = Duration::from_secs(60);
/// The background loops whose period is fixed rather than configured.
const RETENTION_PERIOD: Duration = Duration::from_secs(60);
const OWNERSHIP_PERIOD: Duration = Duration::from_secs(5);
/// The soonest the scheduler wakes after a pass. A job that is due but could
/// not run — its profile was busy, or its own backoff moved it — must not
/// turn the loop into a spin.
const SCHEDULER_MINIMUM_DELAY: Duration = Duration::from_secs(1);
const RESET_TOKEN_LIFETIME: Duration = Duration::from_secs(60);
const MAXIMUM_RESET_TICKETS: usize = 64;

struct ResetTicket {
    state_dir: PathBuf,
    profile: String,
    state_digest: [u8; 32],
    expires_at: Instant,
}

static RESET_TICKETS: OnceLock<Mutex<BTreeMap<[u8; 32], ResetTicket>>> = OnceLock::new();

#[derive(Clone)]
struct ConnectionCapacity {
    recovery: Arc<Semaphore>,
    blocking: Arc<Semaphore>,
    local: Arc<Semaphore>,
    chat_polling: Arc<Semaphore>,
    active_chat_polls: Arc<Mutex<std::collections::HashSet<ChatPollKey>>>,
}

impl ConnectionCapacity {
    fn worker_pool(&self, operation: &Operation) -> Arc<Semaphore> {
        match operation {
            Operation::SetLocalAccountAlias { .. }
            | Operation::ListProfiles
            | Operation::Ping
            | Operation::AgentStatus
            | Operation::RetentionStatus => self.local.clone(),
            Operation::DataWriteStatus { .. } | Operation::PendingDataWrites { .. } => {
                self.recovery.clone()
            }
            _ => self.blocking.clone(),
        }
    }
}

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
    #[arg(long, default_value_t = 4)]
    chat_poll_workers: usize,
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
        || arguments.chat_poll_workers == 0
        || arguments.chat_poll_workers > MAXIMUM_CONCURRENT_CHAT_POLLS
        || arguments.request_timeout_seconds == 0
        || arguments.scheduler_poll_seconds == 0
        || arguments.compatibility_poll_seconds == 0
        || arguments.compatibility_poll_seconds > 24 * 60 * 60
        || arguments.maximum_connections > 4096
        || arguments.blocking_workers > MAXIMUM_CONCURRENT_READS
    {
        return Err("agent limits are outside supported bounds".into());
    }
    let root_lease = foks_client_app::ClientStateLease::acquire(&arguments.state_dir)?;
    drop(ProfileRegistry::open(&arguments.state_dir)?);
    let state_dir = arguments.state_dir.canonicalize()?;
    let initialized = ClientCredentials::is_initialized(&state_dir)?;
    if initialized {
        vault_master_key(&open_credentials(&state_dir)?)?;
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
    let agent_lock = AgentLock::acquire(&state_dir)?;
    remove_stale_agent_socket(&socket)?;
    let listener = bind_private_agent_socket(&socket)?;
    let socket_guard = SocketGuard::new(socket.clone())?;
    let active = Arc::new(Semaphore::new(arguments.maximum_connections));
    let recovery = Arc::new(Semaphore::new(1));
    let blocking = Arc::new(Semaphore::new(arguments.blocking_workers));
    let local = Arc::new(Semaphore::new(1));
    let chat_polling = Arc::new(Semaphore::new(arguments.chat_poll_workers));
    let active_chat_polls = Arc::new(Mutex::new(std::collections::HashSet::new()));
    let retention_gate = Arc::new(Semaphore::new(1));
    let mut retention_timer = tokio::time::interval(RETENTION_PERIOD);
    retention_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let scheduler_gate = Arc::new(Semaphore::new(1));
    let compatibility_gate = Arc::new(Semaphore::new(1));
    let timeout = Duration::from_secs(arguments.request_timeout_seconds);
    let scheduler_cancellation = CancellationToken::new();
    let _scheduler_cancellation_guard = CancelOnDrop(scheduler_cancellation.clone());
    let scheduler_period = Duration::from_secs(arguments.scheduler_poll_seconds);
    // The poll is an upper bound, not a cadence: each pass reports when the
    // earliest job it saw is next due, and the loop wakes then when that is
    // sooner. A pass that reports nothing leaves the fixed poll in place, and
    // the first pass is due at once, as the fixed poll's first tick was.
    let mut scheduler_due_at = tokio::time::Instant::now();
    let (scheduler_reported, mut scheduler_reports) = tokio::sync::mpsc::channel::<Duration>(1);
    let compatibility_period = Duration::from_secs(arguments.compatibility_poll_seconds);
    let mut compatibility = tokio::time::interval(compatibility_period);
    compatibility.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut ownership = tokio::time::interval(OWNERSHIP_PERIOD);
    ownership.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
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
            _ = ownership.tick() => {
                let now_ms = timers::now_milliseconds();
                timers::timers().tick(timers::OWNERSHIP, now_ms, OWNERSHIP_PERIOD);
                let run = timers::timers().begin(timers::OWNERSHIP, now_ms);
                if !agent_ownership_is_current(&root_lease, &agent_lock, &socket_guard, &state_dir) {
                    run.finish(timers::Outcome::Error);
                    eprintln!("foks-agent ownership was displaced; exiting");
                    scheduler_cancellation.cancel();
                    break;
                }
                run.finish(timers::Outcome::Ok);
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
                let capacity = ConnectionCapacity {
                    recovery: recovery.clone(),
                    blocking: blocking.clone(),
                    local: local.clone(),
                    chat_polling: chat_polling.clone(),
                    active_chat_polls: active_chat_polls.clone(),
                };
                let ready = ready.clone();
                tokio::spawn(async move {
                    if let Err(error) = handle_connection(
                        stream,
                        state_dir,
                        capacity,
                        ready,
                        timeout,
                        permit,
                    ).await {
                        eprintln!("foks-agent connection failed: {error}");
                    }
                });
            }
            _ = retention_timer.tick() => {
                let now_ms = timers::now_milliseconds();
                timers::timers().tick(timers::RETENTION, now_ms, RETENTION_PERIOD);
                if !agent_ownership_is_current(&root_lease, &agent_lock, &socket_guard, &state_dir) {
                    eprintln!("foks-agent ownership was displaced; exiting");
                    scheduler_cancellation.cancel();
                    break;
                }
                if !ready.load(Ordering::Acquire) {
                    timers::timers().skipped(timers::RETENTION);
                    continue;
                }
                let Ok(permit) = retention_gate.clone().try_acquire_owned() else {
                    timers::timers().skipped(timers::RETENTION);
                    continue;
                };
                let state=state_dir.clone();
                let run = timers::timers().begin(timers::RETENTION, now_ms);
                tokio::task::spawn_blocking(move || {
                    let _permit=permit;
                    retention::run_one_profile(&state);
                    run.finish(timers::Outcome::Ok);
                });
            }
            Some(delay) = scheduler_reports.recv() => {
                let delay = delay.max(SCHEDULER_MINIMUM_DELAY);
                let wake = tokio::time::Instant::now() + delay;
                if wake < scheduler_due_at {
                    scheduler_due_at = wake;
                    timers::timers().due_in(
                        timers::SCHEDULER,
                        timers::now_milliseconds(),
                        delay,
                    );
                }
            }
            _ = tokio::time::sleep_until(scheduler_due_at) => {
                let now_ms = timers::now_milliseconds();
                scheduler_due_at = tokio::time::Instant::now() + scheduler_period;
                timers::timers().due_in(timers::SCHEDULER, now_ms, scheduler_period);
                if !agent_ownership_is_current(&root_lease, &agent_lock, &socket_guard, &state_dir) {
                    eprintln!("foks-agent ownership was displaced; exiting");
                    scheduler_cancellation.cancel();
                    break;
                }
                if !ready.load(Ordering::Acquire) {
                    timers::timers().skipped(timers::SCHEDULER);
                    continue;
                }
                let Ok(scheduler_permit) = scheduler_gate.clone().try_acquire_owned() else {
                    timers::timers().skipped(timers::SCHEDULER);
                    continue;
                };
                let state = state_dir.clone();
                let cancellation = scheduler_cancellation.clone();
                let workers = blocking.clone();
                let run = timers::timers().begin(timers::SCHEDULER, now_ms);
                let reported = scheduler_reported.clone();
                tokio::spawn(async move {
                    let _scheduler_permit = scheduler_permit;
                    let (outcome, next_due) =
                        run_scheduled_profiles(state, timeout, cancellation, workers).await;
                    run.finish(outcome);
                    if let Some(next_due) = next_due {
                        // A report the loop has not read yet stands; this one
                        // is dropped rather than blocking the pass, and the
                        // next pass reads the due times again anyway.
                        let _ = reported.try_send(next_due);
                    }
                });
            }
            _ = compatibility.tick() => {
                let now_ms = timers::now_milliseconds();
                timers::timers().tick(timers::COMPATIBILITY, now_ms, compatibility_period);
                if !agent_ownership_is_current(&root_lease, &agent_lock, &socket_guard, &state_dir) {
                    eprintln!("foks-agent ownership was displaced; exiting");
                    scheduler_cancellation.cancel();
                    break;
                }
                if !ready.load(Ordering::Acquire) {
                    timers::timers().skipped(timers::COMPATIBILITY);
                    continue;
                }
                let Ok(compatibility_permit) = compatibility_gate.clone().try_acquire_owned() else {
                    timers::timers().skipped(timers::COMPATIBILITY);
                    continue;
                };
                let state = state_dir.clone();
                let client = compatibility_client.clone();
                let cancellation = scheduler_cancellation.clone();
                let run = timers::timers().begin(timers::COMPATIBILITY, now_ms);
                tokio::spawn(async move {
                    let _compatibility_permit = compatibility_permit;
                    let outcome = refresh_hosted_profiles(&state, client, cancellation, timeout).await;
                    run.finish(outcome);
                });
            }
        }
    }
    Ok(())
}

/// One scheduler pass over every profile. Reports how the pass ended, so the
/// loop's own record says whether the work it held admission for succeeded,
/// and how long it is until the earliest job the pass saw is due, so the loop
/// can wake for it rather than on its fixed poll.
async fn run_scheduled_profiles(
    state_dir: PathBuf,
    timeout: Duration,
    cancellation: CancellationToken,
    workers: Arc<Semaphore>,
) -> (timers::Outcome, Option<Duration>) {
    let registry = match ProfileRegistry::open(&state_dir) {
        Ok(registry) => registry,
        Err(error) => {
            eprintln!("foks-agent scheduler could not open profiles: {error}");
            return (timers::Outcome::Error, None);
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
    let hard_databases = profiles
        .iter()
        .filter_map(|profile| {
            registry
                .paths(profile)
                .ok()
                .map(|paths| (profile.clone(), paths.hard_database))
        })
        .collect::<BTreeMap<_, _>>();
    drop(registry);
    // A fixed cutoff keeps completed or failed jobs from becoming due again
    // within this pass, even if a different job takes a long time.
    let now = match now_microseconds() {
        Ok(now) => now,
        Err(error) => {
            eprintln!("foks-agent scheduler could not read time: {error}");
            return (timers::Outcome::Error, None);
        }
    };
    let mut outcome = timers::Outcome::Ok;
    let mut earliest = None;
    for profile in profiles {
        if cancellation.is_cancelled() {
            break;
        }
        let hard_database = hard_databases.get(&profile);
        let next_run_at = hard_database.and_then(|path| profile_next_run(path));
        // Opening the profile and taking its admission for a profile whose
        // jobs are all in the future competes with foreground requests for
        // nothing. The durable due time answers that without either.
        if !scheduled_profile_is_due(next_run_at, now) {
            earliest = earlier_run(earliest, next_run_at);
            continue;
        }
        let state = state_dir.clone();
        let scheduled = profile.clone();
        let probed = hard_database.cloned();
        let probed_profile = profile.clone();
        let result = run_scheduled_batches(
            &state_dir,
            timeout,
            cancellation.clone(),
            workers.clone(),
            move || scheduled_slice_admission(probed.as_deref(), &probed_profile, now),
            move |remaining, control, admitted| {
                run_scheduled_profile(&state, &scheduled, now, remaining, control, admitted)
                    .map_err(|error| error.to_string())
            },
        )
        .await;
        if let Err(error) = result {
            outcome = timers::Outcome::Error;
            eprintln!("foks-agent scheduled refresh failed: {error}");
        }
        // The pass rescheduled whatever it ran, so this is the profile's next
        // due time rather than the one read before the batch.
        earliest = earlier_run(
            earliest,
            hard_database.and_then(|path| profile_next_run(path)),
        );
    }
    (
        outcome,
        scheduled_delay(earliest, now_microseconds().unwrap_or(now)),
    )
}

/// The next time a profile has scheduled work, read from its durable job
/// state. `None` means the profile has no jobs, no hard state yet, or state
/// that could not be read; each of those runs the profile as before rather
/// than skipping work on a guess.
///
/// This read takes no profile admission and no profile lock. It is a
/// consistent SQLite read of one column: a value that changes while it is
/// read costs at most one further pass, never a missed job, because the
/// claim itself still happens under the scheduler lock.
fn profile_next_run(hard_database: &Path) -> Option<u64> {
    if !hard_database.exists() {
        return None;
    }
    foks_client::FoksScheduler::new(hard_database, foks_client::SchedulerConfig::default())
        .ok()?
        .next_run_at()
        .ok()?
}

/// Whether a profile's scheduled work is due. A profile whose due time could
/// not be read is due: the pass itself registers a profile's default jobs.
fn scheduled_profile_is_due(next_run_at: Option<u64>, now: u64) -> bool {
    next_run_at.is_none_or(|due| due <= now)
}

/// What one scheduled slice reserved. A slice runs one job, so it reserves
/// what that job needs rather than what any job might need.
#[derive(Clone, Debug, Eq, PartialEq)]
enum SliceAdmission {
    /// The state root: the only scope under which a job whose work can reach
    /// a second profile may run, because the profiles it reaches are known
    /// only while it executes.
    SecurityRoot,
    /// This profile alone. Every other profile's work, including its shared
    /// reads, runs beside the slice.
    Profile(String),
}

impl SliceAdmission {
    fn scope(&self) -> profile_work::Scope {
        match self {
            Self::SecurityRoot => profile_work::Scope::SecurityRoot,
            Self::Profile(profile) => profile_work::Scope::profile(profile),
        }
    }
    fn is_single_profile(&self) -> bool {
        matches!(self, Self::Profile(_))
    }
}

/// How one scheduled slice ended, as the batch loop needs it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScheduledSlice {
    /// A job was claimed and run.
    Ran,
    /// Nothing was due. The batch ends.
    Idle,
    /// A due job needs wider admission than the slice reserved, and was left
    /// unclaimed. Nothing about that job changed, so this is not a failure:
    /// it asks for one re-run under the wider scope.
    Deferred,
}

/// The admission one scheduled slice should reserve, read from the durable
/// job state alongside the due time the pass already reads there.
///
/// Only the kind of the job a claim would take decides this, and a job's kind
/// is immutable for its identifier: `register_scheduled_job` and
/// `register_scheduled_job_if_missing` both refuse a registration that would
/// rebind an existing identifier to another kind, host or scope. The row can
/// still change between this read and the claim, which is why the claim
/// itself, not this probe, enforces what a single-profile slice may run.
///
/// Durable state that cannot be read at all takes the wide scope, as every
/// slice did before this probe existed. A profile with no hard state yet, and
/// one with nothing due, take the profile scope: the run registers a
/// profile's default jobs, and those are single-profile kinds.
fn scheduled_slice_admission(
    hard_database: Option<&Path>,
    profile: &str,
    now: u64,
) -> SliceAdmission {
    let wide = match hard_database {
        // Opening a store that does not exist would create one outside the
        // profile's locks; a profile with no jobs has no wide job either.
        Some(path) if !path.exists() => Some(false),
        Some(path) => due_scheduled_job_reaches_other_profiles(path, now),
        None => None,
    };
    match wide {
        Some(false) => SliceAdmission::Profile(profile.to_owned()),
        Some(true) | None => SliceAdmission::SecurityRoot,
    }
}

/// Whether the job a claim would take next can reach a second profile.
/// `None` when the durable state could not be read.
///
/// This read takes no profile admission and no profile lock, like the due-time
/// read beside it.
fn due_scheduled_job_reaches_other_profiles(hard_database: &Path, now: u64) -> Option<bool> {
    Some(
        foks_client_db::HardStateStore::open(hard_database)
            .ok()?
            .next_due_scheduled_job(now)
            .ok()?
            .is_some_and(|job| job.kind.reaches_other_profiles()),
    )
}

fn earlier_run(earliest: Option<u64>, candidate: Option<u64>) -> Option<u64> {
    match (earliest, candidate) {
        (Some(earliest), Some(candidate)) => Some(earliest.min(candidate)),
        (earliest, candidate) => earliest.or(candidate),
    }
}

/// How long until the earliest job a pass saw is due, for the loop's next
/// wake. Microseconds durably, milliseconds and coarser in the loop.
fn scheduled_delay(earliest: Option<u64>, now: u64) -> Option<Duration> {
    earliest.map(|due| Duration::from_micros(due.saturating_sub(now)))
}

const SCHEDULED_JOBS_PER_PROFILE: usize = 16;

/// How long one scheduled job may spend on the network. A job that can reach
/// a second profile runs under `SecurityRoot` admission, which every profile's
/// foreground requests wait behind, and its remote calls each get the whole
/// budget it is handed as their deadline. A federated server that accepts a
/// connection and never answers would otherwise hold every profile for the
/// request timeout, and the foreground requests queued behind it would expire
/// at that same deadline without starting. A single-profile job holds only its
/// own profile, where the same bound keeps one slow host from holding that
/// profile's foreground requests for the whole request timeout. The bound
/// matches the identity check's
/// (`connectivity::IDENTITY_NETWORK_BUDGET`) and the default request timeout
/// a command-line agent already runs scheduled jobs under.
const SCHEDULED_NETWORK_BUDGET: Duration = Duration::from_secs(20);

/// The budget handed to one scheduled job: the remaining request budget,
/// bounded by what background work may hold every profile for.
fn scheduled_job_budget(remaining: Duration) -> Duration {
    remaining.min(SCHEDULED_NETWORK_BUDGET)
}

/// How long a request may wait for admission and a worker before the agent
/// answers that it did not start: the budget less the margin its reply needs
/// to reach a client that waits that same budget.
fn admission_budget(timeout: Duration) -> Duration {
    timeout.saturating_sub(ADMISSION_REPLY_MARGIN.min(timeout / 4))
}

// Each worker runs one job through checkpoint publication. No protected state
// or lock survives into the next reservation, and no unstarted job is leased.
//
// `slice_admission` is consulted once per slice rather than once per batch: a
// slice runs one job, and the next due job may be of another kind.
async fn run_scheduled_batches(
    state_dir: &Path,
    timeout: Duration,
    cancellation: CancellationToken,
    workers: Arc<Semaphore>,
    slice_admission: impl Fn() -> SliceAdmission + Send + Sync + 'static,
    run_one: impl Fn(Duration, CancellationToken, SliceAdmission) -> Result<ScheduledSlice, String>
        + Send
        + Sync
        + 'static,
) -> Result<(), String> {
    let run_one = Arc::new(run_one);
    'slices: for _ in 0..SCHEDULED_JOBS_PER_PROFILE {
        if cancellation.is_cancelled() {
            break;
        }
        let mut admitted = slice_admission();
        let outcome = loop {
            // Federation can discover additional profiles during execution, so
            // a slice that may run one reserves the state root. Either way the
            // fair queue is rejoined after every completed job.
            let started = Instant::now();
            let admission = profile_work::coordinator()
                .acquire(state_dir, admitted.scope(), timeout)
                .await
                .map_err(|error| error.to_string())?;
            let permit = tokio::time::timeout(
                timeout.saturating_sub(started.elapsed()),
                workers.clone().acquire_owned(),
            )
            .await
            .map_err(|_| "scheduled worker capacity timed out".to_owned())?
            .map_err(|error| error.to_string())?;
            if cancellation.is_cancelled() {
                break 'slices;
            }
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                break 'slices;
            }
            let control = cancellation.clone();
            let run_one = run_one.clone();
            let budget = scheduled_job_budget(remaining);
            let reserved = admitted.clone();
            let outcome = tokio::task::spawn_blocking(move || {
                let _admission = admission;
                let _permit = permit;
                run_one(budget, control, reserved)
            })
            .await
            .map_err(|error| error.to_string())??;
            match outcome {
                // A due job needed a wider scope than this slice reserved and
                // was left untouched. Release what was reserved and rejoin the
                // queue for the state root. The re-run takes that scope
                // unconditionally, so it cannot defer in turn: a slice passes
                // through admission at most twice, and the job it deferred is
                // claimed by the re-run rather than by a later pass.
                ScheduledSlice::Deferred if admitted.is_single_profile() => {
                    admitted = SliceAdmission::SecurityRoot;
                    // Requests that queued behind the deferral run before the
                    // wider re-run reserves, which a re-run that rejoined the
                    // queue without yielding could otherwise precede.
                    tokio::task::yield_now().await;
                }
                outcome => break outcome,
            }
        };
        match outcome {
            ScheduledSlice::Ran => {}
            // Nothing is due, or a slice already holding the state root has
            // nothing wider to re-run under. Either way the pass is done with
            // this profile; the next pass probes again from current state.
            ScheduledSlice::Idle | ScheduledSlice::Deferred => break,
        }
        // Give ready requests a chance to enqueue before reserving again.
        tokio::task::yield_now().await;
    }
    Ok(())
}

/// Runs one scheduled job for `profile` under the admission the slice
/// reserved.
///
/// A slice that reserved the profile alone runs its claim restricted to jobs
/// whose work cannot leave the profile. The restriction is what makes the
/// narrow reservation sound: it holds for the claim itself, so a job
/// registered after the slice's probe, or between the probe and the claim,
/// still cannot run here. Such a job is left due, unleased and with its
/// failure count untouched, and the slice reports a deferral for its caller to
/// answer with one re-run under the state root.
fn run_scheduled_profile(
    state_dir: &Path,
    profile: &str,
    now: u64,
    timeout: Duration,
    cancellation: CancellationToken,
    admitted: SliceAdmission,
) -> Result<ScheduledSlice, Box<dyn std::error::Error>> {
    let registry = ProfileRegistry::open(state_dir)?;
    let session =
        read_cache::open_profile_session(&registry, profile, timeout, cancellation.clone())?;
    let credentials = open_credentials(state_dir)?;
    profile_work::with_control(timeout, cancellation, || {
        checked_session(&credentials, &session, |session| {
            let master = vault_master_key(&credentials)?;
            let mut store = EncryptedFileSecretStore::open(
                &session.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut run = || {
                session.run_next_due_job_with_federation(
                    now,
                    &mut AccountVault::new(&mut store),
                    &registry,
                    &credentials,
                    &master,
                )
            };
            let (report, deferred) = if admitted.is_single_profile() {
                foks_client_db::with_single_profile_claims(run)
            } else {
                (run(), false)
            };
            let report = report?;
            Ok(if deferred {
                ScheduledSlice::Deferred
            } else if report.runs.is_empty() {
                ScheduledSlice::Idle
            } else {
                ScheduledSlice::Ran
            })
        })
    })
}

async fn refresh_hosted_profiles(
    state_dir: &Path,
    client: reqwest::Client,
    cancellation: CancellationToken,
    timeout: Duration,
) -> timers::Outcome {
    let registry = match ProfileRegistry::open(state_dir) {
        Ok(registry) => registry,
        Err(_) => {
            eprintln!("foks-agent compatibility refresh could not open profiles");
            return timers::Outcome::Error;
        }
    };
    // The poll interval bounds how quickly a profile that still needs a lease
    // acquires one, not how often a profile that already holds a long-lived
    // one refetches it. A profile whose stored lease is validated and not yet
    // near its expiry is skipped: its capability checks already answer from
    // the stored artifact, so the fetch would apply an artifact that reports
    // "unchanged" while competing for that profile's admission.
    let now = match now_microseconds() {
        Ok(now) => now / 1_000_000,
        Err(_) => {
            eprintln!("foks-agent compatibility refresh could not read the system clock");
            return timers::Outcome::Error;
        }
    };
    let profiles = registry
        .profiles()
        .filter(|profile| profile.protocol.needs_lease_renewal_at(now))
        .map(|profile| profile.name.clone())
        .collect::<Vec<_>>();
    drop(registry);

    let state_dir = state_dir.to_owned();
    let failed = Arc::new(AtomicBool::new(false));
    let renewal_failed = failed.clone();
    let outcome = run_hosted_refreshes(profiles, cancellation, move |profile| {
        let state_dir = state_dir.clone();
        let client = client.clone();
        let failed = renewal_failed.clone();
        async move {
            let result = connectivity::renew(&state_dir, &profile, client, timeout).await;
            if let ResponseResult::Error { code, .. } = result {
                failed.store(true, Ordering::Relaxed);
                eprintln!("foks-agent compatibility renewal failed: {code:?}");
            }
        }
    })
    .await;
    if outcome == timers::Outcome::Ok && failed.load(Ordering::Relaxed) {
        timers::Outcome::Error
    } else {
        outcome
    }
}

async fn run_hosted_refreshes<F, Fut>(
    profiles: Vec<String>,
    cancellation: CancellationToken,
    renew: F,
) -> timers::Outcome
where
    F: Fn(String) -> Fut + Send,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let mut profiles = profiles.into_iter();
    let mut pending = tokio::task::JoinSet::new();
    let mut outcome = timers::Outcome::Ok;
    loop {
        if cancellation.is_cancelled() {
            pending.abort_all();
            return timers::Outcome::Interrupted;
        }
        while pending.len() < MAXIMUM_CANARY_FETCHES {
            let Some(profile) = profiles.next() else {
                break;
            };
            pending.spawn(renew(profile));
        }
        if pending.is_empty() {
            break;
        }
        tokio::select! {
            result = pending.join_next() => {
                if let Some(Err(_)) = result {
                    outcome = timers::Outcome::Error;
                }
            },
            _ = tokio::time::sleep(Duration::from_millis(20)) => {},
        }
    }
    outcome
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

#[cfg(test)]
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

#[cfg(test)]
fn apply_hosted_lease_if_idle(
    state_dir: &Path,
    profile: &str,
    bytes: &[u8],
) -> Result<Option<bool>, Box<dyn std::error::Error>> {
    let Some(_profile_permit) =
        profile_work::coordinator().try_acquire(state_dir, profile_work::Scope::Root)?
    else {
        return Ok(None);
    };
    apply_hosted_lease(state_dir, profile, bytes).map(Some)
}

#[cfg(unix)]
async fn handle_connection(
    mut stream: tokio::net::UnixStream,
    state_dir: PathBuf,
    capacity: ConnectionCapacity,
    ready: Arc<AtomicBool>,
    timeout: Duration,
    _active_permit: OwnedSemaphorePermit,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    'requests: for _ in 0..MAXIMUM_REQUESTS_PER_CONNECTION {
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
                capacity.blocking.clone(),
                timeout,
                request.id,
                header.clone(),
            )
            .await?;
            return Ok(());
        }
        if let Operation::ReconcileProfile { profile } | Operation::RefreshLease { profile } =
            &request.operation
        {
            if profile.is_empty()
                || profile.len() > 64
                || !profile
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            {
                write_response(
                    &mut stream,
                    &Response::error(
                        request.id,
                        ErrorCode::InvalidRequest,
                        "invalid profile name",
                    ),
                    timeout,
                )
                .await?;
                continue;
            }
        }
        if let Operation::ReconcileProfile { profile } = &request.operation {
            let (value, timing) =
                connectivity::reconcile(&state_dir, profile, timeout, capacity.blocking.clone())
                    .await;
            write_response(
                &mut stream,
                &Response::success(request.id, value).with_timing(timing),
                timeout,
            )
            .await?;
            continue;
        }
        if let Operation::RefreshLease { profile } = &request.operation {
            let result = match compatibility_http_client(timeout) {
                Ok(client) => connectivity::renew(&state_dir, profile, client, timeout).await,
                Err(error) => dispatch_error_response(request.id, &error).result,
            };
            let response = match result {
                ResponseResult::Success { value } if value["status"] == "not-required" => {
                    Response::error(
                        request.id,
                        ErrorCode::InvalidRequest,
                        "profile does not use a refreshable compatibility lease",
                    )
                }
                ResponseResult::Success { value } => Response::success(
                    request.id,
                    serde_json::json!({
                        "profile": profile,
                        "updated": value["status"] == "renewed",
                    }),
                ),
                ResponseResult::Error {
                    code,
                    message,
                    fields,
                } => Response::error_with_fields(Some(request.id), code, message, fields),
            };
            write_response(&mut stream, &response, timeout).await?;
            continue;
        }
        if request.operation.is_chat_poll_wait() {
            if handle_chat_poll(
                &mut stream,
                state_dir.clone(),
                capacity.chat_polling.clone(),
                capacity.active_chat_polls.clone(),
                timeout,
                request,
            )
            .await?
            {
                return Ok(());
            }
            continue;
        }
        // A read that shares its profile is admitted in shared mode. One
        // that finds the profile in a state only an exclusive session
        // settles is run again under exclusive admission, once; the answer
        // it did not get is never written.
        let request_started = Instant::now();
        let mut scope = profile_work::admission_scope(&request.operation);
        let supervised = loop {
            let remaining_timeout = timeout.saturating_sub(request_started.elapsed());
            if remaining_timeout.is_zero() {
                write_response(
                    &mut stream,
                    &dispatch_error_response(request.id, &profile_work::AdmissionError::Deadline),
                    timeout,
                )
                .await?;
                continue 'requests;
            }
            let request = request.clone();
            let exclusive_scope = profile_work::operation_scope(&request.operation);
            let needs_exclusive = Arc::new(AtomicBool::new(false));
            // Admission precedes worker allocation: same-profile waiters must not
            // occupy every worker and prevent an unrelated profile from proceeding.
            let admitted_at = Instant::now();
            let profile_permit = match profile_work::coordinator()
                .acquire(
                    &state_dir,
                    scope.clone(),
                    admission_budget(remaining_timeout),
                )
                .await
            {
                Ok(permit) => permit,
                Err(error) => {
                    write_response(
                        &mut stream,
                        &dispatch_error_response(request.id, &error),
                        timeout,
                    )
                    .await?;
                    continue 'requests;
                }
            };
            let queue = admitted_at.elapsed();
            let waited_behind = profile_permit.waited_behind;
            let remaining = admission_budget(remaining_timeout).saturating_sub(queue);
            let worker_pool = capacity.worker_pool(&request.operation);
            let pool_started = Instant::now();
            let permit = match tokio::time::timeout(remaining, worker_pool.acquire_owned()).await {
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
                    continue 'requests;
                }
            };
            let state = state_dir.clone();
            let operation_ready = ready.clone();
            let operation_timeout = if request.operation.is_device_pairing_wait() {
                timeout.max(DEVICE_PAIRING_TIMEOUT)
            } else {
                remaining_timeout.saturating_sub(admitted_at.elapsed())
            };
            if operation_timeout.is_zero() {
                write_response(
                    &mut stream,
                    &dispatch_error_response(request.id, &profile_work::AdmissionError::Deadline),
                    timeout,
                )
                .await?;
                continue 'requests;
            }
            let request_timing = RequestTiming {
                queue,
                pool: pool_started.elapsed(),
                spawned_at: Some(Instant::now()),
                waited_behind,
            };
            // Monitor client disconnection for cancellable read operations. Until
            // disconnection is detected, the request retains its profile admission
            // and session. Mutations continue because they may change persistent state.
            let abandonment =
                read_cache::operation_is_abandonable(&request.operation).then_some(&mut stream);
            let shared = matches!(scope, profile_work::Scope::SharedProfile(_));
            let worker_flag = Arc::clone(&needs_exclusive);
            let supervised = supervise_blocking(
                request.id,
                permit,
                profile_permit,
                operation_timeout,
                abandonment,
                move |cancellation| {
                    dispatch_timed(
                        &state,
                        request,
                        operation_timeout,
                        cancellation,
                        operation_ready,
                        request_timing,
                        Admission {
                            shared,
                            needs_exclusive: worker_flag,
                        },
                    )
                },
            )
            .await;
            if !supervised.abandoned
                && !supervised.close_connection
                && needs_exclusive.load(Ordering::Acquire)
                && matches!(scope, profile_work::Scope::SharedProfile(_))
            {
                scope = exclusive_scope;
                continue;
            }
            break supervised;
        };
        if supervised.abandoned {
            return Ok(());
        }
        write_response(&mut stream, &supervised.response, timeout).await?;
        let close_connection = supervised.close_connection;
        drop(supervised);
        if close_connection {
            return Ok(());
        }
    }
    Ok(())
}

/// The relay budget a pairing operation may spend under `operation_timeout`.
fn pairing_relay_budget(operation_timeout: Duration) -> Duration {
    match operation_timeout.checked_sub(PAIRING_COMPLETION_MARGIN) {
        Some(budget) if !budget.is_zero() => budget,
        // A cap at or below the margin leaves nothing to reserve. Halving it
        // still ends the relay waits before the cap does.
        _ => operation_timeout / 2,
    }
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
    timeout: Duration,
    request_id: u64,
    header: KvUploadHeader,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if header.total_length > MAXIMUM_STREAM_KV_BYTES
        || (header.adapter.is_some() && header.total_length > 4 * 1024 * 1024)
    {
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
    let profile_permit = match profile_work::coordinator()
        .acquire(
            &state_dir,
            profile_work::operation_scope(&Operation::PutKvStream {
                header: header.clone(),
            }),
            timeout,
        )
        .await
    {
        Ok(permit) => permit,
        Err(error) => {
            write_response(
                stream,
                &dispatch_error_response(request_id, &error),
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
        let result = profile_work::with_control(timeout, worker_cancellation.clone(), || {
            if let Some(submission) = worker_header.adapter {
                data::upload(
                    &state_dir,
                    &ProfileRegistry::open(&state_dir)?,
                    submission,
                    &mut UploadReader::new(receiver, worker_header.total_length),
                    timeout,
                    worker_cancellation,
                )
            } else {
                put_kv_reader(
                    &state_dir,
                    &ProfileRegistry::open(&state_dir)?,
                    timeout,
                    worker_cancellation,
                    &worker_header.store,
                    &worker_header.path,
                    UploadReader::new(receiver, worker_header.total_length),
                    Some(worker_header.total_length),
                    worker_header.precondition,
                    worker_header.read_role,
                    worker_header.write_role,
                    worker_header.mkdir_p,
                )
            }
        });
        let response = match result {
            Ok(value) => Response::success(request_id, value),
            Err(error) => dispatch_error_response(request_id, error.as_ref()),
        };
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>((response, profile_permit))
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
    let (response, profile_permit) = match tokio::time::timeout(
        timeout + CANCELLATION_GRACE,
        worker,
    )
    .await
    {
        Ok(Ok(Ok((response, profile_permit)))) => (response, Some(profile_permit)),
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
    drop(profile_permit);
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
    /// True when the client disconnected before the operation completed, so no
    /// response should be written.
    abandoned: bool,
    _profile_permit: Option<profile_work::Permit>,
}

struct WorkerCompletion {
    response: Response,
    profile_permit: profile_work::Permit,
}

/// Waits for the peer to close `stream`. Peeking distinguishes a closed
/// connection from pipelined request data without consuming that data. If
/// pipelined data is present, this future remains pending and the operation uses
/// its normal deadline.
async fn connection_closed(stream: &mut tokio::net::UnixStream) {
    loop {
        match stream.ready(tokio::io::Interest::READABLE).await {
            Ok(ready) if ready.is_read_closed() => return,
            Ok(_) => {}
            Err(_) => return,
        }
        let peeked = stream.try_io(tokio::io::Interest::READABLE, || {
            let mut byte = [0_u8];
            rustix::net::recv(&*stream, &mut byte[..], rustix::net::RecvFlags::PEEK)
                .map(|(_, length)| length)
                .map_err(std::io::Error::from)
        });
        match peeked {
            Ok(0) => return,
            // try_io clears cached readiness on WouldBlock, so a later close
            // will wake the next wait instead of disabling this watcher.
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return,
            Ok(_) => break,
        }
    }
    std::future::pending().await
}

async fn supervise_blocking(
    request_id: u64,
    permit: OwnedSemaphorePermit,
    profile_permit: profile_work::Permit,
    timeout: Duration,
    // For cancellable reads, monitors client disconnection while the operation
    // runs. `None` applies only the operation deadline.
    abandonment: Option<&mut tokio::net::UnixStream>,
    operation: impl FnOnce(CancellationToken) -> Response + Send + 'static,
) -> SupervisedResponse {
    let cancellation = CancellationToken::new();
    let _cancel_on_drop = CancelOnDrop(cancellation.clone());
    let worker_cancellation = cancellation.clone();
    let mut task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        WorkerCompletion {
            response: operation(worker_cancellation),
            profile_permit,
        }
    });
    let completed = async {
        match abandonment {
            Some(stream) => {
                tokio::select! {
                    biased;
                    result = tokio::time::timeout(timeout, &mut task) => Some(result),
                    () = connection_closed(stream) => None,
                }
            }
            None => Some(tokio::time::timeout(timeout, &mut task).await),
        }
    }
    .await;
    let Some(completed) = completed else {
        cancellation.cancel();
        // Allow the standard cancellation grace period for the worker to release
        // profile admission and session resources. The timeout prevents an
        // unresponsive worker from blocking the request loop.
        let profile_permit = match tokio::time::timeout(CANCELLATION_GRACE, &mut task).await {
            Ok(Ok(completion)) => Some(completion.profile_permit),
            Ok(Err(_)) | Err(_) => None,
        };
        return SupervisedResponse {
            // Construct a response to preserve the common result type; the caller
            // does not write it because the socket is closed.
            response: Response::error(
                request_id,
                ErrorCode::OperationFailed,
                "agent request abandoned by its client",
            ),
            close_connection: true,
            abandoned: true,
            _profile_permit: profile_permit,
        };
    };
    match completed {
        Ok(Ok(completion)) => SupervisedResponse {
            response: completion.response,
            close_connection: false,
            abandoned: false,
            _profile_permit: Some(completion.profile_permit),
        },
        Ok(Err(error)) => SupervisedResponse {
            response: Response::error(
                request_id,
                ErrorCode::OperationFailed,
                format!("agent worker failed: {error}"),
            ),
            close_connection: true,
            abandoned: false,
            _profile_permit: None,
        },
        Err(_) => {
            cancellation.cancel();
            // Blocking work cannot be forcibly killed safely. Give network
            // operations one poll interval to observe cancellation; if other
            // synchronous work remains stuck, its permit keeps the pool bound.
            let profile_permit = match tokio::time::timeout(CANCELLATION_GRACE, &mut task).await {
                Ok(Ok(completion)) => Some(completion.profile_permit),
                Ok(Err(_)) | Err(_) => None,
            };
            SupervisedResponse {
                response: Response::error(
                    request_id,
                    ErrorCode::DeadlineExceeded,
                    "agent operation deadline exceeded; connection closed because completion is ambiguous",
                ),
                close_connection: true,
                abandoned: false,
                _profile_permit: profile_permit,
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

/// What the request loop measured before the worker ran: admission and
/// pool waits, when the worker was spawned, and what admission waited behind.
#[derive(Clone, Copy, Debug, Default)]
struct RequestTiming {
    queue: Duration,
    pool: Duration,
    spawned_at: Option<Instant>,
    waited_behind: Option<&'static str>,
}

/// `dispatch_timed` without the request loop's measurements, for tests that
/// dispatch directly.
#[cfg(test)]
fn dispatch_controlled(
    state_dir: &Path,
    request: Request,
    timeout: Duration,
    cancellation: CancellationToken,
    ready: Arc<AtomicBool>,
) -> Response {
    dispatch_timed(
        state_dir,
        request,
        timeout,
        cancellation,
        ready,
        RequestTiming::default(),
        Admission {
            shared: false,
            needs_exclusive: Arc::new(AtomicBool::new(false)),
        },
    )
}

/// `dispatch_controlled` with the request's phases measured and attached to
/// the response: the loop's waits, the worker's start latency, session opens,
/// lock retries and the operation body, plus which caches served it.
fn dispatch_timed(
    state_dir: &Path,
    request: Request,
    timeout: Duration,
    cancellation: CancellationToken,
    ready: Arc<AtomicBool>,
    request_timing: RequestTiming,
    admission: Admission,
) -> Response {
    let id = request.id;
    let initializes = matches!(request.operation, Operation::InitializeState { .. });
    let start = request_timing
        .spawned_at
        .map(|spawned| spawned.elapsed())
        .unwrap_or_default();
    let body_started = Instant::now();
    let (result, phases) = profile_work::with_phase_timing(|| {
        profile_work::with_control(timeout, cancellation.clone(), || {
            profile_work::with_shared_session(admission.shared, || {
                dispatch_result(
                    state_dir,
                    request.operation,
                    timeout,
                    cancellation,
                    ready.load(Ordering::Acquire),
                )
            })
        })
    });
    let total = body_started.elapsed();
    if result
        .as_ref()
        .is_err_and(|error| error.downcast_ref::<NeedsExclusiveSession>().is_some())
    {
        admission.needs_exclusive.store(true, Ordering::Release);
    }
    if initializes && result.is_ok() {
        ready.store(true, Ordering::Release);
    }
    let ms = ResponseTiming::millis;
    let timing = ResponseTiming {
        queue_ms: ms(request_timing.queue),
        pool_ms: ms(request_timing.pool),
        start_ms: ms(start),
        session_ms: ms(phases.session),
        lock_ms: ms(phases.lock),
        lock_retries: phases.lock_retries,
        body_ms: ms(total
            .saturating_sub(phases.session)
            .saturating_sub(phases.lock)),
        auth_cached: phases.auth_cached,
        report_cached: phases.report_cached,
        waited_behind: request_timing.waited_behind.map(str::to_owned),
        ..ResponseTiming::default()
    };
    match result {
        Ok(value) => Response::success(id, value),
        Err(error) => dispatch_error_response(id, error.as_ref()),
    }
    .with_timing(timing)
}

fn dispatch_error_response(id: u64, error: &(dyn std::error::Error + 'static)) -> Response {
    if let Some(error) = error.downcast_ref::<profile_work::AdmissionError>() {
        // DeadlineExceeded means a dispatched operation may have committed.
        // Admission failure is known not to have entered an operation body.
        return Response::error_with_fields(
            Some(id),
            ErrorCode::Busy,
            error.to_string(),
            ErrorFields {
                reason: Some("admission-not-started".to_owned()),
                ..ErrorFields::default()
            },
        );
    }
    if error
        .downcast_ref::<CatalogSnapshotChangedError>()
        .is_some()
    {
        return Response::error(
            id,
            ErrorCode::CatalogSnapshotChanged,
            "The vault changed while it was being listed.",
        );
    }
    if let Some(error) = error.downcast_ref::<AgentRequestError>() {
        return Response::error(id, ErrorCode::InvalidRequest, error.to_string());
    }
    if error.downcast_ref::<ProfileBusyError>().is_some()
        || error.downcast_ref::<NeedsExclusiveSession>().is_some()
    {
        return Response::error(id, ErrorCode::ProfileBusy, error.to_string());
    }
    if let Some(error) = error.downcast_ref::<foks_client_app::Error>() {
        match error {
            foks_client_app::Error::WebAdmin(error) => {
                use foks_client::WebAdminError as W;
                let code = match error {
                    W::Unsupported => ErrorCode::WebAdminUnsupported,
                    W::Expired => ErrorCode::WebAdminExpired,
                    W::WrongAccount => ErrorCode::WebAdminWrongAccount,
                    W::DestinationRejected => ErrorCode::WebAdminDestinationRejected,
                    W::Unavailable => ErrorCode::WebAdminUnavailable,
                    W::ReauthenticationRequired => ErrorCode::ReauthenticationRequired,
                };
                return Response::error(id, code, error.to_string());
            }
            foks_client_app::Error::ImportVerificationRequired => return Response::error(id, ErrorCode::ImportVerificationRequired, "Imported profile requires online verification. Use state verify-online or the native state verification control."),
            foks_client_app::Error::BotTokenLocked => {
                return Response::error(
                    id,
                    ErrorCode::BotTokenLocked,
                    "Bot token is locked; load the original token into this agent session.",
                )
            }
            foks_client_app::Error::BotToken => {
                return Response::error(id, ErrorCode::BotToken, "Invalid bot token.")
            }
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
            foks_client_app::Error::AccountExists => {
                return Response::error(
                    id,
                    ErrorCode::Conflict,
                    "an account with this username already exists",
                );
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
            foks_client_app::Error::CurrentPassphraseRejected => {
                return Response::error(
                    id,
                    ErrorCode::CurrentPassphraseRejected,
                    "that current passphrase is not correct; nothing was changed",
                );
            }
            foks_client_app::Error::Client(foks_client::Error::PassphraseConflict { .. }) => {
                return Response::error(
                    id,
                    // The generation moved, which covers another device's
                    // change and this one's own submission landing after the
                    // connection dropped. Neither is named here, because the
                    // observation cannot tell them apart.
                    ErrorCode::Conflict,
                    "the account's passphrase changed since this was prepared; refresh before retrying",
                );
            }
            _ => {}
        }
    }
    let mut source = Some(error);
    while let Some(candidate) = source {
        if matches!(
            candidate.downcast_ref::<foks_keystore::Error>(),
            Some(foks_keystore::Error::CredentialsRequired)
        ) {
            return Response::error(
                id,
                ErrorCode::CredentialsRequired,
                "Native credentials require user interaction before this operation can continue.",
            );
        }
        if matches!(
            candidate.downcast_ref::<foks_rpc::Error>(),
            Some(foks_rpc::Error::RemoteStatus { code: 1069, .. })
        ) {
            return Response::error(id, ErrorCode::ReauthenticationRequired, "Organization sign-in is required. Sign in again for this account, then review the interrupted operation before retrying.");
        }
        if let Some(chat) = candidate.downcast_ref::<foks_client::Error>() {
            let code = match chat {
                foks_client::Error::ChatInvalidInput(..) => Some(ErrorCode::ChatInvalidInput),
                foks_client::Error::ChatUnsupported(..) => Some(ErrorCode::ChatUnsupported),
                foks_client::Error::ChatAccessDenied(..) => Some(ErrorCode::ChatAccessDenied),
                foks_client::Error::ChatRefreshRequired(..) => Some(ErrorCode::ChatRefreshRequired),
                foks_client::Error::ChatReprepareRequired(..) => {
                    Some(ErrorCode::ChatReprepareRequired)
                }
                foks_client::Error::ChatNotFound(..) => Some(ErrorCode::ChatNotFound),
                foks_client::Error::ChatKeyUnavailable(..) => Some(ErrorCode::ChatKeyUnavailable),
                foks_client::Error::ChatLimit(..) => Some(ErrorCode::ChatLimit),
                foks_client::Error::ChatOperationState(..) => Some(ErrorCode::ChatOperationState),
                foks_client::Error::ChatNameConflict(..) => Some(ErrorCode::ChatNameConflict),
                foks_client::Error::ChatRandomness(..) => Some(ErrorCode::ChatRandomness),
                foks_client::Error::ChatChannelIntegrity(..) => {
                    Some(ErrorCode::ChatChannelIntegrity)
                }
                foks_client::Error::ChatIntegrity(..) => Some(ErrorCode::ChatIntegrity),
                _ => None,
            };
            if let Some(code) = code {
                return Response::error(id, code, chat.to_string());
            }
        }
        if let Some(retention) = candidate.downcast_ref::<foks_client_db::Error>() {
            if let foks_client_db::Error::UnsupportedSchema { found, supported } = retention {
                return Response::error_with_fields(
                    Some(id),
                    ErrorCode::UnsupportedSchema,
                    bounded_error(retention.to_string()),
                    ErrorFields {
                        found_schema: Some(*found),
                        supported_schema: Some(*supported),
                        ..ErrorFields::default()
                    },
                );
            }
            let code = match retention {
                foks_client_db::Error::AdapterRetentionFull => Some(ErrorCode::RetentionFull),
                foks_client_db::Error::AdapterClockUntrusted => Some(ErrorCode::ClockUntrusted),
                foks_client_db::Error::AdapterActiveFull => Some(ErrorCode::SubmissionActiveFull),
                foks_client_db::Error::AdapterIdentityConflict => {
                    Some(ErrorCode::SubmissionIdentityConflict)
                }
                foks_client_db::Error::AdapterFutureHandle => Some(ErrorCode::SubmissionFuture),
                _ => None,
            };
            if let Some(code) = code {
                retention::rejected(code);
                return Response::error(id, code, retention.to_string());
            }
        }
        if let Some(chat) = candidate.downcast_ref::<foks_client_db::Error>() {
            let code = match chat {
                foks_client_db::Error::InvalidChatInbox(..) => Some(ErrorCode::ChatIntegrity),
                foks_client_db::Error::ChatConflict(..) => Some(ErrorCode::Conflict),
                foks_client_db::Error::ChatOperationState(..) => {
                    Some(ErrorCode::ChatOperationState)
                }
                foks_client_db::Error::ChatLimit(..) => Some(ErrorCode::ChatLimit),
                foks_client_db::Error::ChatNotFound(..) => Some(ErrorCode::ChatNotFound),
                _ => None,
            };
            if let Some(code) = code {
                return Response::error(id, code, chat.to_string());
            }
        }

        if let Some(response) = client_error_response(id, candidate, error) {
            return response;
        }
        if let Some(foks_rpc::Error::RemoteStatus { code, .. }) =
            candidate.downcast_ref::<foks_rpc::Error>()
        {
            if let Some(response) = remote_status_response(id, *code, error.to_string()) {
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
    // Nothing above named this failure. The innermost error is the one that
    // says what went wrong; the crate chain wrapping it is kept in `reason`
    // for logs and support rather than shown as the message.
    Response::error_with_fields(
        Some(id),
        ErrorCode::OperationFailed,
        bounded_error(sentence(leaf_message(error))),
        ErrorFields {
            reason: Some(bounded_error(error.to_string())),
            ..ErrorFields::default()
        },
    )
}

/// The message of the innermost error in a chain.
fn leaf_message(error: &(dyn std::error::Error + 'static)) -> String {
    let mut leaf = error;
    while let Some(next) = leaf.source() {
        leaf = next;
    }
    leaf.to_string()
}

/// Capitalizes the first letter so that the error fragment is formatted as a complete sentence.
fn sentence(message: String) -> String {
    let mut chars = message.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => message,
    }
}

/// A sentence for the client-side failures a person can cause, with the crate
/// chain in `reason`. Anything not listed falls through to the status table
/// and the leaf-message fallback.
fn client_error_response(
    id: u64,
    candidate: &(dyn std::error::Error + 'static),
    chain: &(dyn std::error::Error + 'static),
) -> Option<Response> {
    use foks_client::Error as C;
    let (code, message): (ErrorCode, String) = if let Some(error) =
        candidate.downcast_ref::<foks_client::Error>()
    {
        match error {
                C::NoAddress(host) => (
                    ErrorCode::OperationFailed,
                    format!("The server address {host} could not be resolved. Check the name."),
                ),
                C::Connect(_) => (
                    ErrorCode::OperationFailed,
                    "Could not reach the server. Check the address and port, and that the server is running.".to_owned(),
                ),
                C::Tls(_) | C::ServerName | C::HostTlsRoots => (
                    ErrorCode::OperationFailed,
                    "The server's TLS certificate could not be verified.".to_owned(),
                ),
                C::Kex(_) => (
                    ErrorCode::OperationFailed,
                    "Pairing did not finish. Start pairing again on both devices.".to_owned(),
                ),
                C::KexPhrase(_) => (
                    ErrorCode::InvalidRequest,
                    "Enter a valid device-pairing phrase.".to_owned(),
                ),
                C::Backup(reason) => (ErrorCode::InvalidRequest, backup_phrase_sentence(reason)),
                C::AccountRequest(reason) if *reason == "username is not valid after normalization" => (
                    ErrorCode::InvalidRequest,
                    "Usernames use 3 to 25 letters, numbers, and single underscores.".to_owned(),
                ),
                C::AccountRequest(reason) if *reason == "device name is not valid after normalization" => (
                    ErrorCode::InvalidRequest,
                    "Device names use 2 to 200 letters, numbers, spaces, and . _ + ' -, and start with a letter or number.".to_owned(),
                ),
                C::TeamRequest(reason) if *reason == "team name is not valid under FOKS v0.1.9 normalization" => (
                    ErrorCode::InvalidRequest,
                    "Group names use 3 to 25 letters, numbers, spaces, dots, dashes, or underscores.".to_owned(),
                ),
                C::TeamRequest(reason) if *reason == "target user is already a team member" => (
                    ErrorCode::Conflict,
                    "That user is already a member of this group.".to_owned(),
                ),
                C::KvRequest(reason) if *reason == "upload exceeds FOKS file size limit" => (
                    ErrorCode::InvalidRequest,
                    "That file is too large to upload.".to_owned(),
                ),
                _ => return None,
            }
    } else if let Some(error) = candidate.downcast_ref::<foks_client_app::Error>() {
        match error {
                foks_client_app::Error::InvalidAccount(reason)
                    if reason.starts_with("group names use") =>
                {
                    (
                        ErrorCode::InvalidRequest,
                        "Group names use 3 to 25 letters, numbers, spaces, dots, dashes, or underscores.".to_owned(),
                    )
                }
                _ => return None,
            }
    } else {
        return None;
    };
    Some(Response::error_with_fields(
        Some(id),
        code,
        message,
        ErrorFields {
            reason: Some(bounded_error(chain.to_string())),
            ..ErrorFields::default()
        },
    ))
}

fn backup_phrase_sentence(error: &foks_crypto::BackupPhraseError) -> String {
    use foks_crypto::BackupPhraseError as E;
    match error {
        E::TokenCount { found } => format!(
            "A paper key has {} words and numbers; this one has {found}.",
            foks_crypto::BACKUP_PHRASE_TOKENS
        ),
        E::Word { index } => format!(
            "Word {} of the paper key is not a recognized word. Check its spelling.",
            index + 1
        ),
        E::Number { index } | E::NumberRange { index } => format!(
            "Number {} of the paper key should be a whole number from 0 to 8191.",
            index + 1
        ),
        _ => "That paper key is not valid.".to_owned(),
    }
}

/// Formatted error descriptions and numeric codes for user-actionable FOKS
/// status codes. The server's raw error message is preserved in `reason`.
fn remote_status_response(id: u64, status: u64, reason: String) -> Option<Response> {
    let (code, message) = match status {
        foks_rpc::STATUS_RATE_LIMIT_ERROR => (
            ErrorCode::RateLimited,
            "The FOKS server is busy or rate-limiting requests. Retry after a short delay.",
        ),
        foks_rpc::STATUS_OVER_QUOTA_ERROR => (
            ErrorCode::QuotaExceeded,
            "The FOKS server reached a configured capacity limit. Review its capacity or remove unused data before retrying.",
        ),
        foks_rpc::STATUS_USERNAME_IN_USE_ERROR => (
            ErrorCode::Conflict,
            "That name is already taken on this server. Choose another.",
        ),
        foks_rpc::STATUS_DUPLICATE_ERROR => (
            ErrorCode::Conflict,
            "The server already has this record.",
        ),
        foks_rpc::STATUS_DEVICE_ALREADY_PROVISIONED_ERROR => (
            ErrorCode::Conflict,
            "This device is already set up on the account.",
        ),
        foks_rpc::STATUS_TEAM_INVITE_ALREADY_ACCEPTED_ERROR => (
            ErrorCode::Conflict,
            "This invitation was already accepted.",
        ),
        foks_rpc::STATUS_TEAM_ADHOC_DUPLICATE_ERROR => (
            ErrorCode::Conflict,
            "An identical share already exists.",
        ),
        foks_rpc::STATUS_BAD_ARGS_ERROR => (
            ErrorCode::InvalidRequest,
            "The server rejected the request as malformed.",
        ),
        foks_rpc::STATUS_BAD_INVITE_CODE_ERROR => (
            ErrorCode::InvalidRequest,
            "That invitation code is not valid.",
        ),
        foks_rpc::STATUS_BAD_PASSPHRASE_ERROR => (
            ErrorCode::InvalidRequest,
            "That passphrase is not correct.",
        ),
        foks_rpc::STATUS_KEX_BAD_SECRET => (
            ErrorCode::InvalidRequest,
            "The pairing phrase did not match. Check it and try again.",
        ),
        foks_rpc::STATUS_PASSPHRASE_NOT_FOUND_ERROR => (
            ErrorCode::OperationFailed,
            "No passphrase is set for this account.",
        ),
        foks_rpc::STATUS_USER_NOT_FOUND_ERROR => (
            ErrorCode::OperationFailed,
            "No user by that name exists on this server.",
        ),
        foks_rpc::STATUS_WRONG_USER_ERROR => (
            ErrorCode::OperationFailed,
            "These credentials belong to a different user.",
        ),
        foks_rpc::STATUS_KEY_NOT_FOUND_ERROR => (
            ErrorCode::OperationFailed,
            "The server does not recognize this key.",
        ),
        foks_rpc::STATUS_PERMISSION_ERROR => (
            ErrorCode::OperationFailed,
            "The server refused permission for this action.",
        ),
        foks_rpc::STATUS_EXPIRED_ERROR => (
            ErrorCode::OperationFailed,
            "This request expired. Start it again.",
        ),
        foks_rpc::STATUS_TIMEOUT_ERROR => (
            ErrorCode::OperationFailed,
            "The server timed out. Try again.",
        ),
        foks_rpc::STATUS_NOT_IMPLEMENTED => (
            ErrorCode::OperationFailed,
            "The server does not support this operation.",
        ),
        foks_rpc::STATUS_GENERIC_NOT_FOUND_ERROR => (
            ErrorCode::OperationFailed,
            "The server has no such record.",
        ),
        foks_rpc::STATUS_TEAM_NOT_FOUND_ERROR => (
            ErrorCode::OperationFailed,
            "The server has no such team.",
        ),
        foks_rpc::STATUS_KV_PERM_ERROR => (
            ErrorCode::OperationFailed,
            "You do not have permission for this item.",
        ),
        foks_rpc::STATUS_KV_NOENT_ERROR => (
            ErrorCode::OperationFailed,
            "This item no longer exists on the server.",
        ),
        foks_rpc::STATUS_KV_LOCK_TIMEOUT_ERROR | foks_rpc::STATUS_KV_LOCK_ALREADY_HELD_ERROR => (
            ErrorCode::Busy,
            "Another change to this item is in progress. Try again.",
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
struct CatalogSnapshotChangedError;

impl std::fmt::Display for CatalogSnapshotChangedError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("catalog changed while it was being listed")
    }
}

impl std::error::Error for CatalogSnapshotChangedError {}

#[derive(Debug)]
struct ProfileBusyError;

impl std::fmt::Display for ProfileBusyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Another operation is using this profile.")
    }
}

impl std::error::Error for ProfileBusyError {}

/// How a request was admitted, as its dispatch needs to know it.
struct Admission {
    /// Admitted beside other reads: the sessions it opens share the
    /// profile's locks.
    shared: bool,
    /// Set by the dispatch when a shared session was refused for a state
    /// only an exclusive one settles, so the connection loop runs the request
    /// again under exclusive admission.
    needs_exclusive: Arc<AtomicBool>,
}

/// A read admitted in shared mode found its profile in a state only an
/// exclusive session settles. The connection loop runs it again under
/// exclusive admission; the error is answered only if that cannot happen.
#[derive(Debug)]
struct NeedsExclusiveSession;

impl std::fmt::Display for NeedsExclusiveSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("This profile needs an exclusive session; retry.")
    }
}

impl std::error::Error for NeedsExclusiveSession {}

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
    stored_at: Instant,
}

/// One bounded, expiring store of catalog reports, in the shape of
/// [`read_cache::Expiring`]: the current time is passed in, every entry
/// expires, and the bounds evict the oldest entry first. It differs only in
/// carrying the encoded size of each report, because a catalog is bounded by
/// bytes as well as by count.
///
/// The lifetime is the whole freshness bound: a hit is served without any
/// server round trip, and nothing here validates an entry against the server.
/// It therefore bounds staleness against writes made elsewhere only. A write
/// made through this agent drops every entry ([`invalidate_cached_catalogs`]),
/// and the desktop additionally asks for the first page with `fresh` set
/// after a mutation and on a refresh the user started.
#[derive(Default)]
struct CatalogCache {
    entries: VecDeque<CachedCatalog>,
    encoded_bytes: usize,
}

impl CatalogCache {
    /// The entry for a store, optionally pinned to the snapshot a cursor
    /// names. A request with no cursor takes whatever snapshot is retained;
    /// a request with one is answered only from the snapshot it holds, so a
    /// later page can never come from a different listing than its cursor.
    fn get_at(
        &mut self,
        store: &CatalogStoreBinding,
        snapshot: Option<(u64, [u8; 32])>,
        now: Instant,
    ) -> Option<&CachedCatalog> {
        self.expire(now);
        self.entries.iter().find(|entry| {
            entry.store == *store
                && snapshot.is_none_or(|(version, digest)| {
                    entry.digest == digest && entry.report.snapshot_version == version
                })
        })
    }

    fn put_at(&mut self, entry: CachedCatalog, now: Instant) {
        self.expire(now);
        let store = entry.store.clone();
        self.remove(|candidate| candidate.store == store);
        while self.entries.len() >= MAXIMUM_CACHED_CATALOGS
            || self
                .encoded_bytes
                .checked_add(entry.encoded_bytes)
                .is_none_or(|total| total > MAXIMUM_CACHED_CATALOG_BYTES)
        {
            let Some(evicted) = self.entries.pop_front() else {
                break;
            };
            self.encoded_bytes = self.encoded_bytes.saturating_sub(evicted.encoded_bytes);
        }
        self.encoded_bytes += entry.encoded_bytes;
        self.entries.push_back(entry);
    }

    fn expire(&mut self, now: Instant) {
        self.remove(|entry| now.duration_since(entry.stored_at) >= CATALOG_CACHE_LIFETIME);
    }

    /// Drops the entries the predicate selects, keeping the byte total and
    /// the insertion order (so the front stays the oldest entry) correct.
    fn remove(&mut self, drop: impl Fn(&CachedCatalog) -> bool) {
        let mut freed = 0usize;
        self.entries.retain(|entry| {
            if drop(entry) {
                freed += entry.encoded_bytes;
                return false;
            }
            true
        });
        self.encoded_bytes = self.encoded_bytes.saturating_sub(freed);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

fn catalog_cache() -> &'static Mutex<CatalogCache> {
    static CACHE: OnceLock<Mutex<CatalogCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(CatalogCache::default()))
}

/// Drops every retained catalog. This runs with the read caches' own
/// invalidation around any operation that may have written, so the lifetime
/// bounds staleness against writes made elsewhere only: a write made through
/// this agent, by any client, is never followed by a page served from a
/// snapshot that predates it. Dropping other stores' entries too costs one
/// listing each and removes the need to know which stores an operation
/// reached while it ran.
fn invalidate_cached_catalogs() {
    invalidate_catalog_cache(catalog_cache());
}

fn invalidate_catalog_cache(cache: &Mutex<CatalogCache>) {
    if let Ok(mut cache) = cache.lock() {
        cache.remove(|_| true);
    }
}

/// Whether a request may be answered from a retained report. `fresh` is
/// honored on the first page only: a request that carries a cursor is pinned
/// to the snapshot that cursor names, and re-listing for it would answer from
/// a snapshot the cursor does not describe.
fn may_serve_cached_catalog(cursor: Option<&str>, fresh: bool) -> bool {
    cursor.is_some() || !fresh
}

fn paginate_cached_catalog(
    store: &CatalogStoreBinding,
    cursor: Option<&str>,
    limit: u32,
) -> Result<Option<KvPage>, Box<dyn std::error::Error>> {
    paginate_cached_catalog_in(catalog_cache(), store, cursor, limit)
}

fn paginate_cached_catalog_in(
    cache: &Mutex<CatalogCache>,
    store: &CatalogStoreBinding,
    cursor: Option<&str>,
    limit: u32,
) -> Result<Option<KvPage>, Box<dyn std::error::Error>> {
    validate_catalog_request(store, cursor, limit)?;
    let snapshot = cursor
        .map(decode_catalog_cursor)
        .transpose()?
        .map(|cursor| (cursor.snapshot_version, cursor.snapshot_digest));
    let mut cache = cache
        .lock()
        .map_err(|_| AgentRequestError("catalog cache is unavailable"))?;
    let Some(entry) = cache.get_at(store, snapshot, Instant::now()) else {
        return Ok(None);
    };
    // The digest was computed when the report was listed; a page never
    // re-serializes and re-hashes the snapshot to rediscover it.
    Ok(Some(paginate_catalog(
        &entry.report,
        store.clone(),
        entry.digest,
        cursor,
        limit,
    )?))
}

fn cache_catalog(
    cache: &Mutex<CatalogCache>,
    store: CatalogStoreBinding,
    report: &foks_client_app::KvCatalogReport,
    digest: [u8; 32],
    encoded_bytes: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    if encoded_bytes > MAXIMUM_SINGLE_CATALOG_BYTES {
        return Ok(());
    }
    let now = Instant::now();
    cache
        .lock()
        .map_err(|_| AgentRequestError("catalog cache is unavailable"))?
        .put_at(
            CachedCatalog {
                store,
                digest,
                report: report.clone(),
                encoded_bytes,
                stored_at: now,
            },
            now,
        );
    Ok(())
}

fn paginate_fresh_catalog(
    report: foks_client_app::KvCatalogReport,
    store: CatalogStoreBinding,
    cursor: Option<&str>,
    limit: u32,
) -> Result<KvPage, Box<dyn std::error::Error>> {
    paginate_fresh_catalog_in(catalog_cache(), report, store, cursor, limit)
}

fn paginate_fresh_catalog_in(
    cache: &Mutex<CatalogCache>,
    report: foks_client_app::KvCatalogReport,
    store: CatalogStoreBinding,
    cursor: Option<&str>,
    limit: u32,
) -> Result<KvPage, Box<dyn std::error::Error>> {
    // Once per fresh report, for both the cache key and the cursors of every
    // page served from it.
    let (digest, encoded_bytes) = catalog_snapshot_identity(&report)?;
    let page = paginate_catalog(&report, store.clone(), digest, cursor, limit)?;
    // A single-page store is retained too: the desktop re-reads every store
    // on its catalog pass, and the pass after this one is inside the
    // lifetime.
    cache_catalog(cache, store, &report, digest, encoded_bytes)?;
    Ok(page)
}

fn paginate_catalog(
    report: &foks_client_app::KvCatalogReport,
    store: CatalogStoreBinding,
    snapshot_digest: [u8; 32],
    cursor: Option<&str>,
    limit: u32,
) -> Result<KvPage, Box<dyn std::error::Error>> {
    validate_catalog_request(&store, cursor, limit)?;
    let offset = if let Some(cursor) = cursor {
        let cursor = decode_catalog_cursor(cursor)?;
        if cursor.snapshot_version != report.snapshot_version
            || cursor.snapshot_digest != snapshot_digest
        {
            return Err(Box::new(CatalogSnapshotChangedError));
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
    // Measuring an entry by serializing it into a buffer that is then thrown
    // away costs one allocation per entry. A counting sink measures exactly
    // the same bytes and allocates nothing.
    struct CountingWriter(usize);

    impl std::io::Write for CountingWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .ok_or_else(|| std::io::Error::other("catalog entry size overflow"))?;
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

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
        let mut measured = CountingWriter(0);
        serde_json::to_writer(&mut measured, &entry)?;
        let entry_bytes = measured.0;
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

// Counts serializations on this thread, independently of parallel fixtures.
#[cfg(test)]
thread_local! {
    static CATALOG_IDENTITY_COMPUTATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn catalog_snapshot_identity(
    report: &foks_client_app::KvCatalogReport,
) -> Result<([u8; 32], usize), serde_json::Error> {
    use sha2::Digest as _;
    use std::io::Write as _;

    #[cfg(test)]
    CATALOG_IDENTITY_COMPUTATIONS.set(CATALOG_IDENTITY_COMPUTATIONS.get() + 1);

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
    // The encoded-byte budget in `paginate_catalog` is the binding bound on a
    // page; this only keeps a caller from asking for an unbounded row slice.
    const MAXIMUM_CATALOG_PAGE_ENTRIES: u32 = 4096;
    if !(1..=MAXIMUM_CATALOG_PAGE_ENTRIES).contains(&limit) {
        return Err(Box::new(AgentRequestError(
            "catalog page limit must be between 1 and 4096",
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
    expected_size: Option<u64>,
    precondition: KvPrecondition,
    read_role: KvRole,
    write_role: KvRole,
    mkdir_p: bool,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let session =
        read_cache::open_profile_session(registry, kv_store_profile(store), timeout, cancellation)?;
    with_vault_and_master(state_dir, &session, |session, vault, master| {
        let report = match store {
            KvStoreRef::Account(store) => session.put_kv_file_checked_with_size(
                &store.account_alias,
                path,
                &mut reader,
                expected_size,
                wire_precondition(precondition),
                wire_role_to_app(read_role),
                wire_role_to_app(write_role),
                mkdir_p,
                vault,
                master,
            )?,
            KvStoreRef::Team(store) => session.put_team_kv_file_checked_with_size(
                &store.account_alias,
                &store.team_alias,
                &store.team_id,
                path,
                &mut reader,
                expected_size,
                wire_precondition(precondition),
                wire_role_to_app(read_role),
                wire_role_to_app(write_role),
                mkdir_p,
                vault,
                master,
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
        foks_client_app::ResetArtifactKind::ExternalImportReadiness => {
            WireResetArtifactKind::ExternalImportReadiness
        }
        foks_client_app::ResetArtifactKind::ExternalPublicationAuthorization => {
            WireResetArtifactKind::ExternalPublicationAuthorization
        }
    }
}

fn operation_is_noninteractive(operation: &Operation) -> bool {
    match operation {
        Operation::ListProfiles
        | Operation::ListKnownStores { .. }
        | Operation::ListProfileOverview { .. }
        | Operation::ListAccounts { .. }
        | Operation::ListPendingOperations { .. }
        | Operation::ListDevices { .. }
        | Operation::ListBackupEnrollments { .. }
        | Operation::DescribeServerStatus { .. }
        | Operation::ListYubiAccounts { .. }
        | Operation::ListKv { .. }
        | Operation::ListTeamKv { .. }
        | Operation::ReadKv { .. }
        | Operation::ReadKvChunk { .. }
        | Operation::ReadData { .. }
        | Operation::BindDataAccount { .. }
        | Operation::ListTeams { .. }
        | Operation::ListTeamDetails { .. }
        | Operation::ListTeamMembers { .. }
        | Operation::ListFederatedTeams { .. }
        | Operation::ListAccountRenames { .. }
        | Operation::DataWriteStatus { .. }
        | Operation::PendingDataWrites { .. }
        | Operation::BotAccount {
            action: foks_agent_proto::bot::BotAction::List,
            ..
        }
        | Operation::Sso {
            action: foks_agent_proto::sso::SsoAction::Status { .. },
            ..
        } => true,
        Operation::Chat { action, .. } => matches!(
            action,
            foks_agent_proto::chat::ChatAction::LoadIntent { .. }
                | foks_agent_proto::chat::ChatAction::SaveIntent { .. }
                | foks_agent_proto::chat::ChatAction::ClearIntent { .. }
                | foks_agent_proto::chat::ChatAction::ImportIntent { .. }
                | foks_agent_proto::chat::ChatAction::Channels
                | foks_agent_proto::chat::ChatAction::History { .. }
                | foks_agent_proto::chat::ChatAction::NotificationHistory { .. }
                | foks_agent_proto::chat::ChatAction::Inbox
                | foks_agent_proto::chat::ChatAction::SyncInbox { .. }
                | foks_agent_proto::chat::ChatAction::PollInbox { .. }
                | foks_agent_proto::chat::ChatAction::Pending
                | foks_agent_proto::chat::ChatAction::CleanupPending
                | foks_agent_proto::chat::ChatAction::Status { .. }
                | foks_agent_proto::chat::ChatAction::OperationBody { .. }
        ),
        _ => false,
    }
}

/// Runs an operation, invalidates retained read state around operations that may
/// mutate, and retries eligible cached reads once with fresh authentication.
fn dispatch_result(
    state_dir: &Path,
    operation: Operation,
    timeout: Duration,
    cancellation: CancellationToken,
    ready: bool,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let started = Instant::now();
    let noninteractive = operation_is_noninteractive(&operation);
    let reads = read_cache::operation_serves_reads(&operation);
    let scope = profile_work::operation_scope(&operation);
    let retry = reads.then(|| operation.clone());
    let run = |operation, timeout| {
        let dispatch =
            || dispatch_result_inner(state_dir, operation, timeout, cancellation.clone(), ready);
        if noninteractive {
            foks_keystore::without_user_interaction(dispatch)
        } else {
            dispatch()
        }
    };
    // Treat operations outside the read and noninvalidating lists as mutations
    // and clear all retained read state.
    let invalidates = !reads
        && !read_cache::operation_leaves_retained_material(&operation)
        && !matches!(
            scope,
            profile_work::Scope::None | profile_work::Scope::RegistryRead
        );
    // Invalidate before dispatch because a timed-out operation may still commit
    // without reaching post-dispatch cleanup.
    if invalidates {
        read_cache::invalidate_all();
        invalidate_cached_catalogs();
    }
    let (result, trace) = read_cache::with_operation_scope(reads, || run(operation, timeout));
    if !reads {
        // Invalidate again after dispatch, including on failure, to remove entries
        // cached concurrently while the mutation was running.
        if invalidates {
            read_cache::invalidate_all();
            invalidate_cached_catalogs();
        }
        if matches!(scope, profile_work::Scope::Root) {
            read_cache::invalidate_base_clients();
        }
        return result;
    }
    let Err(error) = result else {
        return result;
    };
    if !read_cache::read_earns_a_retry(&trace, error.as_ref()) {
        return Err(error);
    }
    for (state_root, profile) in &trace.profiles {
        read_cache::invalidate_profile(state_root, profile);
    }
    let Some(operation) = retry else {
        return Err(error);
    };
    // Limit the retry to the remaining request deadline. If no time remains,
    // return the original error instead of producing a zero-timeout error or
    // retaining profile admission after the caller has abandoned the request.
    let remaining = timeout.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        return Err(error);
    }
    // Retry once with fresh authentication and return the retry result.
    read_cache::with_operation_scope(true, || run(operation, remaining)).0
}

fn dispatch_result_inner(
    state_dir: &Path,
    operation: Operation,
    timeout: Duration,
    cancellation: CancellationToken,
    ready: bool,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let mut registry = ProfileRegistry::open(state_dir)?;
    match operation {
        operation @ (Operation::PrepareDataWrite { .. }
        | Operation::ExecuteDataWrite { .. }
        | Operation::DataWriteStatus { .. }
        | Operation::PendingDataWrites { .. }) => {
            let scope = match &operation {
                Operation::PrepareDataWrite { scope, .. }
                | Operation::PendingDataWrites { scope } => scope.clone(),
                Operation::ExecuteDataWrite { submission }
                | Operation::DataWriteStatus { submission } => submission.scope.clone(),
                _ => unreachable!(),
            };
            data::write_control(
                state_dir,
                &registry,
                scope,
                operation,
                timeout,
                cancellation,
            )
        }
        Operation::BindDataAccount {
            profile,
            account_alias,
        } => data::bind_account(
            state_dir,
            &registry,
            profile,
            account_alias,
            timeout,
            cancellation,
        ),
        Operation::ReadData { scope, query } => {
            data::read(state_dir, &registry, scope, query, timeout, cancellation)
        }
        Operation::RetentionStatus => Ok(retention::snapshot()),
        Operation::Ping => Ok(serde_json::json!({ "ready": true })),
        Operation::AgentStatus => Ok(serde_json::to_value(if ready {
            // Include background-loop timings in the existing agent-status response
            // for diagnostic correlation with requests.
            AgentStatus::Ready {
                history_after: Some(true),
                timers: timers::timers().snapshot(timers::now_milliseconds()),
            }
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
                        username: candidate.username,
                        server_hint: candidate.server_hint,
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
                open_credentials(state_dir)?
            } else {
                match ClientCredentials::initialize(state_dir, backend) {
                    Ok(credentials) => credentials,
                    Err(_error) if ClientCredentials::is_initialized(state_dir)? => {
                        open_credentials(state_dir)?
                    }
                    Err(error) => return Err(error.into()),
                }
            };
            vault_master_key(&credentials)?;
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
                label: None,
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
            let credentials = open_credentials(state_dir)?;
            Ok(serde_json::to_value(
                credentials.check_and_add_profile_with_control(
                    &mut registry,
                    Profile {
                        name,
                        label: None,
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
            let credentials = open_credentials(state_dir)?;
            Ok(serde_json::to_value(
                credentials.check_and_add_profile_for_host_with_control(
                    &mut registry,
                    Profile {
                        name,
                        label: None,
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
            let credentials = open_credentials(state_dir)?;
            let removed = credentials.remove_profile(&mut registry, &name)?;
            Ok(serde_json::json!({
                "profile": name,
                "removed": removed,
            }))
        }
        Operation::SetProfileLabel { profile, label } => {
            let label = foks_client_app::normalize_profile_label(&profile, label)?;
            let changed = registry.set_label(&profile, label.clone())?;
            Ok(serde_json::json!({
                "profile": profile,
                "label": label,
                "changed": changed,
            }))
        }
        Operation::DescribeResetHardState { profile } => {
            // A reset is the way out of credentials this Mac cannot read, so
            // the preview goes as far as it can without them and says so.
            let credentials = open_credentials(state_dir)?;
            let session = ProfileSession::open(&registry, &profile)?;
            let preview = credentials.describe_reset_state_best_effort(&session)?;
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
                credentials_unavailable: preview.credentials_unavailable,
            })?)
        }
        Operation::ResetHardState { profile, token } => {
            let state_digest = consume_reset_ticket(state_dir, &profile, token.expose())?;
            let credentials = open_credentials(state_dir)?;
            let session = ProfileSession::open(&registry, &profile)?;
            let outcome =
                credentials.reset_hard_state_best_effort_if_matches(&session, state_digest)?;
            Ok(serde_json::json!({
                "profile": profile,
                "hard_state_reset": true,
                "credential_records_retained": outcome.credential_records_retained,
            }))
        }
        Operation::ListProfiles => Ok(serde_json::to_value(
            registry.profiles().cloned().collect::<Vec<_>>(),
        )?),
        Operation::Probe { profile } => {
            let credentials = open_credentials(state_dir)?;
            let session =
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
            let report = checked_session(&credentials, &session, |session| {
                session
                    .probe_and_pin()
                    .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)
            })?;
            Ok(serde_json::to_value(report)?)
        }
        Operation::RefreshLease { .. } | Operation::ReconcileProfile { .. } => Err(Box::new(
            AgentRequestError("lease refresh requires the async agent connection path"),
        )),
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
        Operation::ListProfileOverview { profile } => {
            let session =
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
            let credentials = open_credentials(state_dir)?;
            if credentials.requires_import_verification(&session)? {
                let catalog =
                    foks_client_app::portability::imported_local_catalog(&credentials, &session)?;
                let accounts = catalog
                    .accounts
                    .into_iter()
                    .map(|(alias, username)| AccountSummary {
                        local_alias: None,
                        profile: profile.clone(),
                        alias,
                        username,
                    })
                    .collect::<Vec<_>>();
                let blocked = || {
                    wire_read_result(Err(Box::new(
                        foks_client_app::Error::ImportVerificationRequired,
                    )))
                };
                return Ok(serde_json::to_value(ProfileOverview {
                    profile,
                    accounts: wire_read_result(Ok(serde_json::to_value(accounts)?)),
                    teams: blocked(),
                    server_status: blocked(),
                })?);
            }
            with_vault_in(&credentials, &session, |session, vault, _master| {
                let accounts = (|| -> Result<_, Box<dyn std::error::Error>> {
                    let mut accounts = Vec::new();
                    for alias in vault.aliases()? {
                        let username = vault.account_display_name(&alias)?;
                        accounts.push(AccountSummary {
                            local_alias: vault.local_account_alias(&alias)?,
                            profile: profile.clone(),
                            alias,
                            username,
                        });
                    }
                    let aliases = accounts
                        .iter()
                        .map(|account| account.alias.clone())
                        .collect::<Vec<_>>();
                    retain_known_accounts(session, &profile, &aliases);
                    Ok(accounts)
                })();
                let teams = (|| -> Result<_, Box<dyn std::error::Error>> {
                    let teams = session.list_local_teams(vault)?;
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
                    Ok(teams
                        .into_iter()
                        .map(|team| WireTeamSummary {
                            alias: team.alias,
                            account_alias: team.account_alias,
                            team_id_hex: team.team_id_hex,
                            kind: team.kind,
                            name: team.name,
                            active: team.active,
                            creation_phase: team.creation_phase,
                            chain_seqno: team.chain_seqno,
                        })
                        .collect::<Vec<_>>())
                })();
                let server_status = session
                    .server_status()
                    .map(wire_server_status)
                    .map_err(|error| Box::new(error) as Box<dyn std::error::Error>);
                Ok(serde_json::to_value(ProfileOverview {
                    profile,
                    accounts: wire_read_result(
                        accounts.and_then(|value| Ok(serde_json::to_value(value)?)),
                    ),
                    teams: wire_read_result(
                        teams.and_then(|value| Ok(serde_json::to_value(value)?)),
                    ),
                    server_status: wire_read_result(
                        server_status.and_then(|value| Ok(serde_json::to_value(value)?)),
                    ),
                })?)
            })
        }
        Operation::SetLocalAccountAlias {
            profile,
            account_alias,
            label,
        } => {
            registry.profile(&profile)?;
            let paths = registry.prepare_profile_directory(&profile)?;
            let credentials = open_credentials(state_dir)?;
            let master = vault_master_key(&credentials)?;
            let mut store =
                EncryptedFileSecretStore::open(&paths.credential_store, derive_vault_key(&master))?;
            AccountVault::new(&mut store).set_local_account_alias(&account_alias, &label)?;
            Ok(
                serde_json::json!({"profile": profile, "account_alias": account_alias, "label": label}),
            )
        }
        Operation::ListAccounts { profile } => {
            let session =
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
            let credentials = open_credentials(state_dir)?;
            if credentials.requires_import_verification(&session)? {
                let catalog =
                    foks_client_app::portability::imported_local_catalog(&credentials, &session)?;
                return Ok(serde_json::to_value(
                    catalog
                        .accounts
                        .into_iter()
                        .map(|(alias, username)| AccountSummary {
                            local_alias: None,
                            profile: profile.clone(),
                            alias,
                            username,
                        })
                        .collect::<Vec<_>>(),
                )?);
            }
            with_vault_in(&credentials, &session, |session, vault, _master| {
                let mut accounts = Vec::new();
                for alias in vault.aliases()? {
                    let username = vault.account_display_name(&alias)?;
                    accounts.push(AccountSummary {
                        local_alias: vault.local_account_alias(&alias)?,
                        profile: profile.clone(),
                        alias,
                        username,
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |_session, vault| {
                let pending = vault
                    .pending_operations()?
                    .into_iter()
                    .map(wire_pending_operation)
                    .collect::<Vec<_>>();
                Ok(serde_json::to_value(pending)?)
            })
        }
        Operation::ListAccountRenames {
            profile,
            account_alias,
        } => {
            let session =
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(
                    session.account_renames(&account_alias, vault)?,
                )?)
            })
        }
        Operation::Invitations {
            profile,
            account_alias,
            action,
            pin,
        } => invitations::run(
            state_dir,
            &registry,
            &profile,
            &account_alias,
            action,
            pin,
            timeout,
            cancellation,
        ),
        Operation::RenameAccount {
            profile,
            account_alias,
            action,
        } => account::rename(
            state_dir,
            &registry,
            &profile,
            &account_alias,
            action,
            timeout,
            cancellation,
        ),
        Operation::WebAdmin {
            profile,
            account_alias,
            action,
        } => account::web_admin(
            state_dir,
            &registry,
            &profile,
            &account_alias,
            action,
            timeout,
            cancellation,
        ),
        Operation::BotAccount {
            profile,
            account_alias,
            action,
        } => bot_token::handle(
            state_dir,
            &registry,
            &profile,
            &account_alias,
            action,
            timeout,
            cancellation,
        ),
        Operation::Sso {
            profile,
            account_alias,
            action,
        } => sso::handle(
            state_dir,
            &registry,
            &profile,
            &account_alias,
            action,
            timeout,
            cancellation,
        ),
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
            let credentials = open_credentials(state_dir)?;
            checked_session(&credentials, &session, |session| {
                let master = vault_master_key(&credentials)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
            let credentials = open_credentials(state_dir)?;
            checked_session(&credentials, &session, |session| {
                let master = vault_master_key(&credentials)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
            let status = if session.has_hard_state_artifacts()? {
                let credentials = open_credentials(state_dir)?;
                checked_session(&credentials, &session, |session| {
                    session
                        .server_status()
                        .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)
                })?
            } else {
                session.server_status_without_pinned_host()?
            };
            Ok(serde_json::to_value(wire_server_status(status))?)
        }
        Operation::RemoveDevice {
            profile,
            signer_alias,
            device_id,
        } => {
            let session =
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
                Ok(serde_json::to_value(session.remove_software_device(
                    &signer_alias,
                    &device_id,
                    vault,
                    master,
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(session.set_passphrase(
                    &alias,
                    Passphrase::new(passphrase.expose())?,
                    vault,
                )?)?)
            })
        }
        Operation::PassphraseStatus { profile, alias } => {
            let session =
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(
                    session.passphrase_status(&alias, vault)?,
                )?)
            })
        }
        Operation::ChangePassphrase {
            profile,
            alias,
            current,
            passphrase,
        } => {
            let session =
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                let current = current
                    .as_ref()
                    .map(|current| Passphrase::new(current.expose()))
                    .transpose()?;
                Ok(serde_json::to_value(session.change_passphrase(
                    &alias,
                    current,
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(session.verify_yubi_passphrase(
                    &alias,
                    Pin::new(pin.expose())?,
                    Passphrase::new(passphrase.expose())?,
                    &HardwareYubiProvider::new(),
                    vault,
                )?)?)
            })
        }
        Operation::SyncAccount { profile, alias } => {
            let session =
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(session.sync_account(&alias, vault)?)?)
            })
        }
        Operation::StartDevicePairing {
            profile,
            account_alias,
        } => {
            let session =
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?
                    .with_pairing_budget(pairing_relay_budget(timeout));
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?
                    .with_pairing_budget(pairing_relay_budget(timeout));
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?
                    .with_pairing_budget(pairing_relay_budget(timeout));
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?
                    .with_pairing_budget(pairing_relay_budget(timeout));
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?
                    .with_pairing_budget(pairing_relay_budget(timeout));
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, _vault| {
                Ok(serde_json::to_value(
                    session.list_yubi_cards(&HardwareYubiProvider::new())?,
                )?)
            })
        }
        Operation::ListYubiAccounts { profile } => {
            let session =
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
            let credentials = open_credentials(state_dir)?;
            if credentials.requires_import_verification(&session)? {
                return Ok(serde_json::to_value(
                    foks_client_app::portability::imported_local_catalog(&credentials, &session)?
                        .yubi,
                )?);
            }
            with_vault_in(&credentials, &session, |_session, vault, _master| {
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
            let credentials = open_credentials(state_dir)?;
            checked_session(&credentials, &session, |session| {
                let master = vault_master_key(&credentials)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
            fresh,
        } => {
            let binding = CatalogStoreBinding::Account {
                value: store.clone(),
            };
            validate_catalog_request(&binding, cursor.as_deref(), limit)?;
            if may_serve_cached_catalog(cursor.as_deref(), fresh) {
                if let Some(page) = paginate_cached_catalog(&binding, cursor.as_deref(), limit)? {
                    profile_work::note_report_cached();
                    return Ok(serde_json::to_value(page)?);
                }
            }
            let session =
                read_cache::open_profile_session(&registry, &store.profile, timeout, cancellation)?;
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
            fresh,
        } => {
            let binding = CatalogStoreBinding::Team {
                value: store.clone(),
            };
            validate_catalog_request(&binding, cursor.as_deref(), limit)?;
            if may_serve_cached_catalog(cursor.as_deref(), fresh) {
                if let Some(page) = paginate_cached_catalog(&binding, cursor.as_deref(), limit)? {
                    profile_work::note_report_cached();
                    return Ok(serde_json::to_value(page)?);
                }
            }
            let session =
                read_cache::open_profile_session(&registry, &store.profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
            let content_size = u64::try_from(content.len())?;
            put_kv_reader(
                state_dir,
                &registry,
                timeout,
                cancellation,
                &store,
                &path,
                std::io::Cursor::new(content.as_slice()),
                Some(content_size),
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
            let session = read_cache::open_profile_session(
                &registry,
                kv_store_profile(&store),
                timeout,
                cancellation,
            )?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
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
                        master,
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
                        master,
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
            let session = read_cache::open_profile_session(
                &registry,
                kv_store_profile(&store),
                timeout,
                cancellation,
            )?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
                let report = match &store {
                    KvStoreRef::Account(store) => session.mkdir_kv_checked(
                        &store.account_alias,
                        &path,
                        wire_precondition(precondition),
                        wire_role_to_app(read_role),
                        wire_role_to_app(write_role),
                        mkdir_p,
                        vault,
                        master,
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
                        master,
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
            let session = read_cache::open_profile_session(
                &registry,
                kv_store_profile(&store),
                timeout,
                cancellation,
            )?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
                let report = match &store {
                    KvStoreRef::Account(store) => session.remove_kv_checked(
                        &store.account_alias,
                        &path,
                        recursive,
                        version,
                        vault,
                        master,
                    )?,
                    KvStoreRef::Team(store) => session.remove_team_kv_checked(
                        &store.account_alias,
                        &store.team_alias,
                        &store.team_id,
                        &path,
                        recursive,
                        version,
                        vault,
                        master,
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
            with_vault_and_master(state_dir, &session, |session, vault, master| {
                Ok(serde_json::to_value(session.resume_team_creation(
                    &team_alias,
                    vault,
                    master,
                )?)?)
            })
        }
        Operation::AbandonTeamCreation {
            profile,
            team_alias,
        } => {
            if team_alias.trim().is_empty() {
                return Err(Box::new(AgentRequestError(
                    "abandoning team creation requires a team alias",
                )));
            }
            let session =
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
            with_vault(state_dir, &session, |session, vault| {
                Ok(serde_json::to_value(
                    session.abandon_team_creation(&team_alias, vault)?,
                )?)
            })
        }
        Operation::Chat { store, action } => {
            let session =
                read_cache::open_profile_session(&registry, &store.profile, timeout, cancellation)?;
            // The send and attempt arms need the credentials handle, so this
            // opens it once here rather than letting them open a second one
            // from inside the session.
            let credentials = open_credentials(state_dir)?;
            with_vault_in(&credentials, &session, |session, vault, master| {
                chat::dispatch(
                    state_dir,
                    &credentials,
                    session,
                    vault,
                    master,
                    store,
                    action,
                )
                .map_err(chat::contextual_error)
            })
        }
        Operation::ListTeams { profile } => {
            let session =
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
            let credentials = open_credentials(state_dir)?;
            with_vault_in(&credentials, &session, |session, vault, master| {
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
            let credentials = open_credentials(state_dir)?;
            with_vault_in(&credentials, &session, |session, vault, master| {
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
            let credentials = open_credentials(state_dir)?;
            with_vault_in(&credentials, &session, |session, vault, master| {
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
            let local = read_cache::open_profile_session(
                &registry,
                &local_profile,
                timeout,
                cancellation.clone(),
            )?;
            let remote = read_cache::open_profile_session(
                &registry,
                &remote_profile,
                timeout,
                cancellation,
            )?;
            let credentials = open_credentials(state_dir)?;
            checked_sessions(&credentials, &local, &remote, |local, remote| {
                let master = vault_master_key(&credentials)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
            let credentials = open_credentials(state_dir)?;
            with_vault_in(&credentials, &session, |session, vault, master| {
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
            let session = read_cache::open_profile_session(
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
                read_cache::open_profile_session(&registry, &profile, timeout, cancellation)?;
            let credentials = open_credentials(state_dir)?;
            checked_session(&credentials, &session, |session| {
                let master = vault_master_key(&credentials)?;
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
    let credentials = open_credentials(state_dir)?;
    let master = vault_master_key(&credentials)?;

    let requested = requested_federation_unlocks(profile, &arguments)?;

    let mut opened = Vec::new();
    for (unlock_profile, alias, pin) in requested {
        let unlock_session = read_cache::open_profile_session(
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

fn wire_read_result(
    result: Result<serde_json::Value, Box<dyn std::error::Error>>,
) -> ResponseResult {
    match result {
        Ok(value) => ResponseResult::Success { value },
        Err(error) => dispatch_error_response(0, error.as_ref()).result,
    }
}

fn wire_server_status(status: foks_client_app::ServerStatusSnapshot) -> WireServerStatusSnapshot {
    WireServerStatusSnapshot {
        profile: status.profile,
        configured_probe: status.configured_probe,
        host: status.host.map(|host| WireStoredHostStatus {
            lookup_name: host.lookup_name,
            canonical_name: host.canonical_name,
            host_id_hex: host.host_id_hex,
            host_chain_sequence: host.host_chain_sequence,
            merkle_epoch: host.merkle_epoch,
        }),
        chat_supported: status.chat_supported,
        compatibility: match status.compatibility {
            foks_client_app::CompatibilityStatus::NotRequired => {
                foks_agent_proto::CompatibilityStatus::NotRequired
            }
            foks_client_app::CompatibilityStatus::Missing => {
                foks_agent_proto::CompatibilityStatus::Missing
            }
            foks_client_app::CompatibilityStatus::Validated {
                expires_at,
                capabilities,
            } => foks_agent_proto::CompatibilityStatus::Validated {
                expires_at,
                capabilities: capabilities
                    .into_iter()
                    .map(|capability| capability.as_str().to_owned())
                    .collect(),
            },
            foks_client_app::CompatibilityStatus::Incompatible { expires_at, reason } => {
                foks_agent_proto::CompatibilityStatus::Incompatible {
                    expires_at,
                    reason: match reason {
                        foks_client_app::CompatibilityFailure::Drift => {
                            foks_agent_proto::CompatibilityFailure::Drift
                        }
                        foks_client_app::CompatibilityFailure::ProtocolMismatch => {
                            foks_agent_proto::CompatibilityFailure::ProtocolMismatch
                        }
                        foks_client_app::CompatibilityFailure::UnknownCapability => {
                            foks_agent_proto::CompatibilityFailure::UnknownCapability
                        }
                    },
                }
            }
        },
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

/// Test-only tally of the credential-service work one dispatched operation
/// performs.
///
/// The fixtures run on the file backend, where these reads are ordinary file
/// reads, and the one native-backend test in the tree is ignored; counting
/// the calls the agent makes is therefore what an ordinary test run can
/// observe. The mapping to native cost is fixed: an open is two manifest
/// reads and a master-key read is one.
///
/// `nested_opens` counts opens taken while a checked session is held. Such an
/// open re-enters the manifest file lock, which is not reentrant and is
/// otherwise always the innermost lock, from inside a span that already holds
/// the profile and database locks. It is the count that must stay at zero.
///
/// Dispatch runs synchronously on the calling thread, so thread-local
/// counters keep parallel tests from observing each other.
#[cfg(test)]
mod credential_reads {
    use std::cell::Cell;

    thread_local! {
        static OPENS: Cell<usize> = const { Cell::new(0) };
        static NESTED_OPENS: Cell<usize> = const { Cell::new(0) };
        static MASTER_KEY_READS: Cell<usize> = const { Cell::new(0) };
        static CHECKED_DEPTH: Cell<usize> = const { Cell::new(0) };
    }

    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub(super) struct Counts {
        pub(super) opens: usize,
        pub(super) nested_opens: usize,
        pub(super) master_key_reads: usize,
    }

    /// Marks the closure body of a checked session, so an open taken inside
    /// one is distinguishable from an open taken before it.
    pub(super) struct CheckedSpan;

    impl CheckedSpan {
        pub(super) fn enter() -> Self {
            CHECKED_DEPTH.with(|depth| depth.set(depth.get() + 1));
            Self
        }
    }

    impl Drop for CheckedSpan {
        fn drop(&mut self) {
            CHECKED_DEPTH.with(|depth| depth.set(depth.get() - 1));
        }
    }

    pub(super) fn note_open() {
        OPENS.with(|count| count.set(count.get() + 1));
        if CHECKED_DEPTH.with(Cell::get) > 0 {
            NESTED_OPENS.with(|count| count.set(count.get() + 1));
        }
    }

    pub(super) fn note_master_key_read() {
        MASTER_KEY_READS.with(|count| count.set(count.get() + 1));
    }

    pub(super) fn reset() {
        OPENS.with(|count| count.set(0));
        NESTED_OPENS.with(|count| count.set(0));
        MASTER_KEY_READS.with(|count| count.set(0));
    }

    pub(super) fn counts() -> Counts {
        Counts {
            opens: OPENS.with(Cell::get),
            nested_opens: NESTED_OPENS.with(Cell::get),
            master_key_reads: MASTER_KEY_READS.with(Cell::get),
        }
    }
}

/// Opens the client credentials for `state_dir`.
///
/// Every open is charged twice against the native credential service: the
/// state lease reads the manifest to confirm the namespace is not mid
/// relocation or import, and `open` reads it again to verify the state-root
/// binding. Routing every open through here keeps that cost visible and lets
/// the tests below assert how many an operation pays for.
fn open_credentials(state_dir: &Path) -> foks_client_app::Result<ClientCredentials> {
    #[cfg(test)]
    credential_reads::note_open();
    ClientCredentials::open(state_dir)
}

/// Reads the vault wrapping key, one further manifest read on the native
/// backend. Counted for the same reason as [`open_credentials`].
fn vault_master_key(
    credentials: &ClientCredentials,
) -> foks_client_app::Result<Zeroizing<[u8; 32]>> {
    #[cfg(test)]
    credential_reads::note_master_key_read();
    credentials.master_key()
}

/// Runs `operation` under a checked session with the profile's account vault
/// open, using credentials the caller already holds.
///
/// Callers that needed `ClientCredentials` before the session — to answer an
/// import-verification question, or to hand the handle to a federated
/// operation — pass theirs rather than opening a second one. The second open
/// would repeat reads whose answers cannot have changed (the profile
/// operation lock excludes every writer, and the state-root binding is
/// immutable outside maintenance, which the held lease excludes), and it
/// would take the manifest file lock again from inside the span where the
/// profile and database locks are held.
fn with_vault_in<T>(
    credentials: &ClientCredentials,
    session: &ProfileSession,
    operation: impl FnOnce(
        &CheckedProfileSession<'_>,
        &mut AccountVault<'_>,
        &[u8; 32],
    ) -> Result<T, Box<dyn std::error::Error>>,
) -> Result<T, Box<dyn std::error::Error>> {
    checked_session(credentials, session, |session| {
        let master = vault_master_key(credentials)?;
        let mut store = EncryptedFileSecretStore::open(
            &session.paths().credential_store,
            derive_vault_key(&master),
        )?;
        {
            let mut vault = AccountVault::new(&mut store);
            bot_token::attach(session, &mut vault)?;
            operation(session, &mut vault, &master)
        }
    })
}

fn with_vault<T>(
    state_dir: &Path,
    session: &ProfileSession,
    operation: impl FnOnce(
        &CheckedProfileSession<'_>,
        &mut AccountVault<'_>,
    ) -> Result<T, Box<dyn std::error::Error>>,
) -> Result<T, Box<dyn std::error::Error>> {
    let credentials = open_credentials(state_dir)?;
    with_vault_in(&credentials, session, |session, vault, _master| {
        operation(session, vault)
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
    let credentials = open_credentials(state_dir)?;
    with_vault_in(&credentials, session, operation)
}

fn checked_session<T>(
    credentials: &ClientCredentials,
    session: &ProfileSession,
    operation: impl FnOnce(&CheckedProfileSession<'_>) -> Result<T, Box<dyn std::error::Error>>,
) -> Result<T, Box<dyn std::error::Error>> {
    let mut operation = Some(operation);
    if profile_work::session_is_shared() {
        // Admitted beside other reads: the session shares the profile's
        // locks. A profile only an exclusive session settles sends the
        // request back to be admitted that way.
        loop {
            profile_work::check_control()
                .map_err(|_| Box::new(ProfileBusyError) as Box<dyn std::error::Error>)?;
            match credentials.try_with_shared_checked_session(session, |checked| {
                #[cfg(test)]
                let _span = credential_reads::CheckedSpan::enter();
                operation.take().expect("checked operation runs once")(checked)
            })? {
                SharedSessionOutcome::Ran(value) => return Ok(value),
                SharedSessionOutcome::NeedsExclusive => {
                    return Err(Box::new(NeedsExclusiveSession));
                }
                SharedSessionOutcome::Contended => {}
            }
            profile_work::wait_for_external_lock()
                .map_err(|_| Box::new(ProfileBusyError) as Box<dyn std::error::Error>)?;
        }
    }
    loop {
        profile_work::check_control()
            .map_err(|_| Box::new(ProfileBusyError) as Box<dyn std::error::Error>)?;
        let result = credentials.try_with_checked_session(session, |checked| {
            #[cfg(test)]
            let _span = credential_reads::CheckedSpan::enter();
            operation.take().expect("checked operation runs once")(checked)
        })?;
        if let Some(value) = result {
            return Ok(value);
        }
        // None proves the closure was not entered. Never retry an error from
        // the closure, checkpoint publication, or response decoding.
        profile_work::wait_for_external_lock()
            .map_err(|_| Box::new(ProfileBusyError) as Box<dyn std::error::Error>)?;
    }
}

fn checked_sessions<T>(
    credentials: &ClientCredentials,
    left: &ProfileSession,
    right: &ProfileSession,
    operation: impl FnOnce(
        &CheckedProfileSession<'_>,
        &CheckedProfileSession<'_>,
    ) -> Result<T, Box<dyn std::error::Error>>,
) -> Result<T, Box<dyn std::error::Error>> {
    let mut operation = Some(operation);
    loop {
        profile_work::check_control()
            .map_err(|_| Box::new(ProfileBusyError) as Box<dyn std::error::Error>)?;
        let result = credentials.try_with_checked_sessions(left, right, |left, right| {
            #[cfg(test)]
            let _span = credential_reads::CheckedSpan::enter();
            operation.take().expect("checked operation runs once")(left, right)
        })?;
        if let Some(value) = result {
            return Ok(value);
        }
        profile_work::wait_for_external_lock()
            .map_err(|_| Box::new(ProfileBusyError) as Box<dyn std::error::Error>)?;
    }
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
        [] => Err(format!("hardware key serial {serial} is not connected").into()),
        _ => Err(format!("hardware key serial {serial} is ambiguous").into()),
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
fn agent_ownership_is_current(
    root: &foks_client_app::ClientStateLease,
    lock: &AgentLock,
    socket: &SocketGuard,
    state_dir: &Path,
) -> bool {
    root.validate().is_ok()
        && lock.owns_named_file(state_dir).unwrap_or(false)
        && socket.owns_named_socket().unwrap_or(false)
}

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

    fn owns_named_file(&self, state_dir: &Path) -> std::io::Result<bool> {
        use std::os::unix::fs::MetadataExt as _;

        let held = self.0.metadata()?;
        match std::fs::symlink_metadata(state_dir.join(AGENT_LOCK_NAME)) {
            Ok(named) => Ok(held.dev() == named.dev() && held.ino() == named.ino()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
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
    // A socket belonging to another live agent is not stale, even when that
    // agent predates our state-directory lock or bound after desktop takeover.
    match std::os::unix::net::UnixStream::connect(path) {
        Ok(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                "another process owns the agent socket",
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {}
        Err(error) => return Err(error),
    }
    let current = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if current.dev() != metadata.dev() || current.ino() != metadata.ino() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AddrInUse,
            "the agent socket changed during stale-socket inspection",
        ));
    }
    std::fs::remove_file(path)
}

#[cfg(unix)]
struct SocketGuard {
    path: PathBuf,
    device: u64,
    inode: u64,
}

#[cfg(unix)]
impl SocketGuard {
    fn new(path: PathBuf) -> std::io::Result<Self> {
        use std::os::unix::fs::MetadataExt as _;
        let metadata = std::fs::symlink_metadata(&path)?;
        Ok(Self {
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    fn owns_named_socket(&self) -> std::io::Result<bool> {
        use std::os::unix::fs::MetadataExt as _;

        match std::fs::symlink_metadata(&self.path) {
            Ok(metadata) => Ok(metadata.dev() == self.device && metadata.ino() == self.inode),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }
}

#[cfg(unix)]
impl Drop for SocketGuard {
    fn drop(&mut self) {
        use std::os::unix::fs::MetadataExt as _;
        let Ok(metadata) = std::fs::symlink_metadata(&self.path) else {
            return;
        };
        // An exiting agent must never remove its successor's socket.
        if metadata.dev() != self.device || metadata.ino() != self.inode {
            return;
        }
        if let Err(error) = std::fs::remove_file(&self.path) {
            eprintln!("foks-agent could not remove socket: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEDULER_PROBE: &[u8] = include_bytes!(
        "../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
    );

    #[test]
    fn a_profile_with_no_job_due_is_not_opened_and_reports_its_next_run() {
        let directory = tempfile::tempdir().unwrap();
        let hard_database = directory.path().join("hard.sqlite");
        // No hard state at all: the pass runs the profile as it always did,
        // and opening it is what registers its default jobs.
        assert_eq!(profile_next_run(&hard_database), None);
        assert!(!hard_database.exists());
        assert!(scheduled_profile_is_due(None, 100));

        let host = foks_verify::verify_public_host("foks.app", SCHEDULER_PROBE).unwrap();
        foks_client_db::HardStateStore::open(&hard_database)
            .unwrap()
            .accept_verified_host(&host.snapshot)
            .unwrap();
        // Hard state without jobs is the same unknown: the profile is opened
        // so its default refresh jobs are registered.
        assert_eq!(profile_next_run(&hard_database), None);

        let scheduler = foks_client::FoksScheduler::new(
            &hard_database,
            foks_client::SchedulerConfig::default(),
        )
        .unwrap();
        scheduler
            .register(foks_client::ScheduledJobRegistration {
                job_id: [7; 16],
                kind: foks_client_db::ScheduledJobKind::UserRefresh,
                host_id: host.snapshot.host_id().to_vec(),
                scope_id: vec![9; 33],
                interval_micros: 1_000,
                first_run_at: 5_000,
                registered_at: 100,
            })
            .unwrap();
        assert_eq!(profile_next_run(&hard_database), Some(5_000));
        assert!(!scheduled_profile_is_due(Some(5_000), 4_999));
        assert!(scheduled_profile_is_due(Some(5_000), 5_000));
    }

    #[test]
    fn a_pass_reports_the_earliest_due_time_it_saw() {
        assert_eq!(earlier_run(None, None), None);
        assert_eq!(earlier_run(None, Some(9)), Some(9));
        assert_eq!(earlier_run(Some(9), None), Some(9));
        assert_eq!(earlier_run(Some(9), Some(4)), Some(4));
        // A profile whose state could not be read contributes nothing, so
        // the loop keeps its fixed poll rather than waking for a guess.
        assert_eq!(scheduled_delay(None, 1_000), None);
        assert_eq!(
            scheduled_delay(Some(2_500_000), 500_000),
            Some(Duration::from_secs(2))
        );
        // An overdue job asks for the soonest wake, not a negative one.
        assert_eq!(scheduled_delay(Some(1), 1_000), Some(Duration::ZERO));
    }

    #[test]
    fn pairing_relay_budget_ends_before_the_agent_cancels_the_operation() {
        let budget = pairing_relay_budget(DEVICE_PAIRING_TIMEOUT);
        assert!(budget < DEVICE_PAIRING_TIMEOUT);
        assert_eq!(budget + PAIRING_COMPLETION_MARGIN, DEVICE_PAIRING_TIMEOUT);
        // A peer that answers just inside the budget still leaves the agent
        // the margin it needs to finish, so the caller sees the pairing's own
        // result instead of the cancellation's deadline error.
        assert!(budget > DEVICE_PAIRING_TIMEOUT / 2);
    }

    #[test]
    fn pairing_relay_budget_stays_inside_a_cap_shorter_than_the_margin() {
        for cap in [
            Duration::from_secs(1),
            PAIRING_COMPLETION_MARGIN,
            PAIRING_COMPLETION_MARGIN + Duration::from_secs(1),
            Duration::from_secs(60 * 60),
        ] {
            let budget = pairing_relay_budget(cap);
            assert!(budget < cap, "budget {budget:?} did not fit cap {cap:?}");
            assert!(!budget.is_zero(), "cap {cap:?} left no relay budget");
        }
    }

    #[test]
    fn every_pairing_wait_operation_runs_under_the_pairing_cap() {
        // The relay budget is derived from the operation timeout the agent
        // applies, so the two must be driven by the same classification.
        let operation = foks_agent_proto::Operation::FinishDevicePairing {
            profile: "local".to_owned(),
            account_alias: "owner".to_owned(),
        };
        assert!(operation.is_device_pairing_wait());
        let timeout = Duration::from_secs(30).max(DEVICE_PAIRING_TIMEOUT);
        assert_eq!(timeout, DEVICE_PAIRING_TIMEOUT);
        assert!(pairing_relay_budget(timeout) < timeout);
    }

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
        assert!(message.contains("soft-state cache schema version"));
        assert!(message.contains("/private/foks/profiles/local/soft.sqlite3"));
        assert!(message.contains("cache must be recreated"));
    }

    #[test]
    fn duplicate_account_alias_maps_to_conflict() {
        let error = foks_client_app::Error::AccountExists;
        let response = dispatch_error_response(11, &error);
        let foks_agent_proto::ResponseResult::Error { code, message, .. } = response.result else {
            panic!("duplicate alias returned success");
        };
        assert_eq!(code, ErrorCode::Conflict);
        assert!(message.contains("already exists"));
    }

    #[test]
    fn hard_state_schema_mapping_is_structured_and_wording_independent() {
        let error =
            foks_client_app::Error::ClientDatabase(foks_client_db::Error::UnsupportedSchema {
                found: 23,
                supported: 27,
            });
        let response = dispatch_error_response(9, &error);
        let foks_agent_proto::ResponseResult::Error {
            code,
            message: _,
            fields,
        } = response.result
        else {
            panic!("schema failure returned success");
        };
        assert_eq!(code, ErrorCode::UnsupportedSchema);
        assert_eq!(fields.found_schema, Some(23));
        assert_eq!(fields.supported_schema, Some(27));
    }

    #[test]
    fn remote_capacity_statuses_keep_retry_semantics_and_diagnostics() {
        for (status, expected, message) in [
            (
                foks_rpc::STATUS_RATE_LIMIT_ERROR,
                ErrorCode::RateLimited,
                "Retry after a short delay.",
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
        let path = directory.path().join(AGENT_LOCK_NAME);
        assert!(path.is_file());
        assert!(first.owns_named_file(directory.path()).unwrap());
        assert_eq!(
            AgentLock::acquire(directory.path()).unwrap_err().kind(),
            std::io::ErrorKind::AddrInUse
        );
        std::fs::remove_file(&path).unwrap();
        assert!(!first.owns_named_file(directory.path()).unwrap());
        drop(first);
        AgentLock::acquire(directory.path()).unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ownership_check_rejects_a_replaced_lock() {
        let directory = tempfile::tempdir().unwrap();
        let lease = foks_client_app::ClientStateLease::acquire(directory.path()).unwrap();
        let lock = AgentLock::acquire(directory.path()).unwrap();
        let socket = directory.path().join("agent.sock");
        let listener = bind_private_agent_socket(&socket).unwrap();
        let guard = SocketGuard::new(socket).unwrap();
        assert!(agent_ownership_is_current(
            &lease,
            &lock,
            &guard,
            directory.path()
        ));
        std::fs::remove_file(directory.path().join(AGENT_LOCK_NAME)).unwrap();
        assert!(!agent_ownership_is_current(
            &lease,
            &lock,
            &guard,
            directory.path()
        ));
        drop(listener);
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
    fn stale_socket_cleanup_preserves_live_listener_and_removes_dead_socket() {
        use std::os::unix::fs::PermissionsExt as _;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agent.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            remove_stale_agent_socket(&path).unwrap_err().kind(),
            std::io::ErrorKind::AddrInUse
        );
        assert!(path.exists());
        drop(listener);
        remove_stale_agent_socket(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn socket_guard_preserves_replacement_listener() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agent.sock");
        let original = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let guard = SocketGuard::new(path.clone()).unwrap();
        assert!(guard.owns_named_socket().unwrap());
        std::fs::remove_file(&path).unwrap();
        let replacement = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert!(!guard.owns_named_socket().unwrap());
        drop(guard);
        assert!(path.exists());
        assert!(std::os::unix::net::UnixStream::connect(&path).is_ok());
        drop(original);
        let guard = SocketGuard::new(path.clone()).unwrap();
        drop(replacement);
        drop(guard);
        assert!(!path.exists());
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
                if serde_json::from_value::<AgentStatus>(value.clone()).unwrap().is_ready()
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
    fn profile_labels_are_changed_locally_and_returned_by_listing() {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join("state");
        ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&state).unwrap();
        registry
            .add(Profile {
                name: "offline".to_owned(),
                label: None,
                probe: "127.0.0.1:1".to_owned(),
                protocol: ProtocolPolicy::V019,
                trust: TrustRoot::WebPki,
            })
            .unwrap();
        drop(registry);

        let response = dispatch(
            &state,
            Request::new(
                1,
                Operation::SetProfileLabel {
                    profile: "offline".to_owned(),
                    label: Some("  FOKS  ".to_owned()),
                },
            ),
        );
        assert!(matches!(
            response.result,
            ResponseResult::Success { value }
                if value == serde_json::json!({
                    "profile": "offline",
                    "label": "FOKS",
                    "changed": true
                })
        ));
        let profiles = dispatch(&state, Request::new(2, Operation::ListProfiles));
        assert!(matches!(
            profiles.result,
            ResponseResult::Success { value }
                if value[0]["name"] == "offline" && value[0]["label"] == "FOKS"
        ));

        let unchanged = dispatch(
            &state,
            Request::new(
                3,
                Operation::SetProfileLabel {
                    profile: "offline".to_owned(),
                    label: Some("FOKS".to_owned()),
                },
            ),
        );
        assert!(matches!(
            unchanged.result,
            ResponseResult::Success { value } if value["changed"] == false
        ));
    }

    #[test]
    fn profile_overview_combines_startup_reads() {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join("state");
        ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&state).unwrap();
        registry
            .add(Profile {
                name: "local".to_owned(),
                label: None,
                probe: "foks.app".to_owned(),
                protocol: ProtocolPolicy::V019,
                trust: TrustRoot::WebPki,
            })
            .unwrap();
        let response = dispatch(
            &state,
            Request::new(
                3,
                Operation::ListProfileOverview {
                    profile: "local".to_owned(),
                },
            ),
        );
        let ResponseResult::Success { value } = response.result else {
            panic!("profile overview failed")
        };
        let overview: ProfileOverview = serde_json::from_value(value).unwrap();
        assert_eq!(overview.profile, "local");
        assert!(matches!(
            overview.accounts,
            ResponseResult::Success { value } if value == serde_json::json!([])
        ));
        assert!(matches!(
            overview.teams,
            ResponseResult::Success { value } if value == serde_json::json!([])
        ));
        assert!(matches!(
            overview.server_status,
            ResponseResult::Success { value }
                if value["profile"] == "local" && value["host"].is_null()
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
                label: None,
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
                label: None,
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
    fn reset_goes_ahead_when_the_credentials_cannot_be_read() {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join("state");
        ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&state).unwrap();
        registry
            .add(Profile {
                name: "local".to_owned(),
                label: None,
                probe: "foks.app".to_owned(),
                protocol: ProtocolPolicy::V019,
                trust: TrustRoot::WebPki,
            })
            .unwrap();
        let session = ProfileSession::open(&registry, "local").unwrap();
        foks_client_db::HardStateStore::open(&session.paths().soft_database).unwrap();
        // The master key is damaged: a reset is the way out of that, so the
        // preview answers with what it can see and says what it could not.
        std::fs::write(state.join("master.key"), b"short").unwrap();

        let described = dispatch(
            &state,
            Request::new(
                20,
                Operation::DescribeResetHardState {
                    profile: "local".to_owned(),
                },
            ),
        );
        let foks_agent_proto::ResponseResult::Success { value } = described.result else {
            panic!("best-effort reset preview failed");
        };
        let preview: WireResetStatePreview = serde_json::from_value(value).unwrap();
        assert!(preview
            .credentials_unavailable
            .as_deref()
            .is_some_and(|reason| reason.contains("master key")));
        let reset = dispatch(
            &state,
            Request::new(
                21,
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
        let missing_profile = dispatch(
            &state,
            Request::new(
                110,
                Operation::SetLocalAccountAlias {
                    profile: "missing".into(),
                    account_alias: "personal".into(),
                    label: "Private account".into(),
                },
            ),
        );
        assert!(matches!(
            missing_profile.result,
            foks_agent_proto::ResponseResult::Error { .. }
        ));
        assert!(!state.join("profiles/missing").exists());
        let renamed = dispatch(
            &state,
            Request::new(
                111,
                Operation::SetLocalAccountAlias {
                    profile: "local".into(),
                    account_alias: "personal".into(),
                    label: "Private account".into(),
                },
            ),
        );
        assert!(
            matches!(renamed.result, foks_agent_proto::ResponseResult::Success { value }
            if value == serde_json::json!({"profile":"local", "account_alias":"personal", "label":"Private account"}))
        );
        let renamed_list = dispatch(
            &state,
            Request::new(
                112,
                Operation::ListAccounts {
                    profile: "local".into(),
                },
            ),
        );
        assert!(
            matches!(renamed_list.result, foks_agent_proto::ResponseResult::Success { value }
            if value == serde_json::json!([{"profile":"local", "alias":"personal", "username":"agentinvite", "local_alias":"Private account"}]))
        );
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
                    // The rotation is guarded by the passphrase set above, so
                    // this also covers the check the agent runs before it.
                    current: Some(foks_agent_proto::SecretString::new("agent passphrase one")),
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
    fn metadata_reads_are_noninteractive_without_changing_explicit_workflows() {
        let profile = "saved".to_owned();
        let account = AccountStoreRef {
            profile: profile.clone(),
            account_alias: "owner".into(),
        };
        let data_scope = foks_agent_proto::data::DataScope {
            profile: profile.clone(),
            account_alias: "owner".into(),
            host_id: "host".into(),
            user_id: "user".into(),
            team_id: None,
        };
        for operation in [
            Operation::BotAccount {
                profile: profile.clone(),
                account_alias: "owner".into(),
                action: foks_agent_proto::bot::BotAction::List,
            },
            Operation::Sso {
                profile: profile.clone(),
                account_alias: "owner".into(),
                action: foks_agent_proto::sso::SsoAction::Status {
                    operation_id: "0".repeat(32),
                },
            },
            Operation::ListProfiles,
            Operation::ListKnownStores {
                profile: profile.clone(),
            },
            Operation::ListProfileOverview {
                profile: profile.clone(),
            },
            Operation::ListAccounts {
                profile: profile.clone(),
            },
            Operation::ListPendingOperations {
                profile: profile.clone(),
            },
            Operation::DescribeServerStatus {
                profile: profile.clone(),
            },
            Operation::ListYubiAccounts {
                profile: profile.clone(),
            },
            Operation::ListDevices {
                profile: profile.clone(),
                alias: "owner".into(),
            },
            Operation::ListBackupEnrollments {
                profile: profile.clone(),
                account_alias: "owner".into(),
            },
            Operation::ListTeams {
                profile: profile.clone(),
            },
            Operation::ListTeamDetails {
                profile: profile.clone(),
                team_alias: "team".into(),
            },
            Operation::ListTeamMembers {
                profile: profile.clone(),
                team_alias: "team".into(),
            },
            Operation::ListFederatedTeams {
                profile: profile.clone(),
                team_alias: "team".into(),
            },
            Operation::ListAccountRenames {
                profile: profile.clone(),
                account_alias: "owner".into(),
            },
            Operation::ListKv {
                store: account.clone(),
                cursor: None,
                limit: 10,
                fresh: false,
            },
            Operation::ReadKv {
                store: KvStoreRef::Account(account.clone()),
                path: "/entry".into(),
                version: 1,
            },
            Operation::ReadKvChunk {
                store: KvStoreRef::Account(account),
                path: "/entry".into(),
                version: 1,
                offset: 0,
                length: 10,
            },
            Operation::BindDataAccount {
                profile: profile.clone(),
                account_alias: "owner".into(),
            },
            Operation::ReadData {
                scope: data_scope.clone(),
                query: foks_agent_proto::data::DataRead::Catalog,
            },
            Operation::DataWriteStatus {
                submission: foks_agent_proto::data::DataSubmission {
                    scope: data_scope.clone(),
                    submission_id: "pending".into(),
                },
            },
            Operation::PendingDataWrites { scope: data_scope },
            Operation::ListTeamKv {
                store: TeamStoreRef {
                    profile: profile.clone(),
                    account_alias: "owner".into(),
                    team_alias: "team".into(),
                    team_id: "team-id".into(),
                },
                cursor: None,
                limit: 10,
                fresh: false,
            },
        ] {
            assert!(operation_is_noninteractive(&operation), "{operation:?}");
        }
        for operation in [
            Operation::RunDueJobs {
                profile: profile.clone(),
            },
            Operation::ListYubiCards {
                profile: profile.clone(),
            },
            Operation::YubiPinStatus {
                profile: profile.clone(),
                alias: "owner".into(),
            },
            Operation::Probe {
                profile: profile.clone(),
            },
            Operation::CreateAccount {
                profile: profile.clone(),
                alias: "owner".into(),
                username: "owner".into(),
                device_name: "device".into(),
                email: String::new(),
                invite: foks_agent_proto::SecretString::new(""),
                passphrase: None,
            },
            Operation::SyncYubiAccount {
                profile: profile.clone(),
                alias: "owner".into(),
                pin: foks_agent_proto::SecretString::new("123456"),
                with_federation: false,
            },
            Operation::ResetHardState {
                profile,
                token: foks_agent_proto::SecretString::new("token"),
            },
        ] {
            assert!(!operation_is_noninteractive(&operation), "{operation:?}");
        }
    }

    #[test]
    fn pending_intent_io_is_noninteractive_and_only_load_is_abandonable() {
        use foks_agent_proto::chat::ChatAction;
        let host = format!("02{}", "ab".repeat(32));
        let actor = format!("01{}", "cd".repeat(32));
        let channel = "12".repeat(16);
        let submission = "34".repeat(16);
        let store = TeamStoreRef {
            profile: "local".into(),
            account_alias: "owner".into(),
            team_alias: "team".into(),
            team_id: format!("03{}", "ef".repeat(32)),
        };
        for action in [
            ChatAction::LoadIntent {
                host: host.clone(),
                actor: actor.clone(),
                channel: channel.clone(),
            },
            ChatAction::SaveIntent {
                host: host.clone(),
                actor: actor.clone(),
                channel: channel.clone(),
                submission: submission.clone(),
                text: foks_agent_proto::SecretString::new("private intent"),
            },
            ChatAction::ClearIntent {
                host: host.clone(),
                actor: actor.clone(),
                channel: channel.clone(),
                submission: submission.clone(),
            },
            ChatAction::ImportIntent {
                host,
                actor,
                channel,
                submission,
                text: foks_agent_proto::SecretString::new("private intent"),
                source: "ab".repeat(32),
            },
        ] {
            assert!(action.validate());
            let mutation = action.is_mutation();
            let operation = Operation::Chat {
                store: store.clone(),
                action,
            };
            assert!(operation_is_noninteractive(&operation));
            assert!(read_cache::operation_leaves_retained_material(&operation));
            assert_eq!(read_cache::operation_is_abandonable(&operation), !mutation);
            assert!(!format!("{operation:?}").contains("private intent"));
        }
    }

    #[test]
    fn chat_background_reads_are_noninteractive_but_submissions_are_not() {
        use foks_agent_proto::chat::ChatAction;
        let store = TeamStoreRef {
            profile: "saved".into(),
            account_alias: "owner".into(),
            team_alias: "team".into(),
            team_id: "team-id".into(),
        };
        for action in [
            ChatAction::Channels,
            ChatAction::History {
                after: None,
                channel: "channel".into(),
                before: None,
            },
            ChatAction::NotificationHistory {
                channel: "channel".into(),
                before: None,
            },
            ChatAction::Inbox,
            ChatAction::SyncInbox {
                blocked_channels: Vec::new(),
            },
            ChatAction::PollInbox {
                since: "0".into(),
                timeout_milliseconds: 100,
            },
            ChatAction::Pending,
            ChatAction::CleanupPending,
            ChatAction::Status {
                operation: "pending".into(),
            },
            ChatAction::OperationBody {
                operation: "pending".into(),
                channel: "channel".into(),
            },
        ] {
            assert!(operation_is_noninteractive(&Operation::Chat {
                store: store.clone(),
                action
            }));
        }
        for action in [
            ChatAction::PrepareMessage {
                submission: "pending".into(),
                channel: "channel".into(),
                text: foks_agent_proto::SecretString::new("message"),
            },
            ChatAction::SubmitMessage {
                submission: "pending".into(),
                channel: "channel".into(),
                text: foks_agent_proto::SecretString::new("message"),
            },
            ChatAction::Attempt {
                operation: "pending".into(),
            },
            ChatAction::Reconcile {
                operation: "pending".into(),
            },
            ChatAction::MarkRead {
                channel: "channel".into(),
                sequence: "1".into(),
            },
        ] {
            assert!(!operation_is_noninteractive(&Operation::Chat {
                store: store.clone(),
                action
            }));
        }
    }

    #[test]
    fn interaction_required_credentials_have_a_distinct_typed_response() {
        let wrapped = foks_client_app::Error::Keystore(foks_keystore::Error::CredentialsRequired);
        for error in [
            &wrapped as &dyn std::error::Error,
            &foks_keystore::Error::CredentialsRequired,
        ] {
            assert!(matches!(
                dispatch_error_response(17, error).result,
                ResponseResult::Error {
                    code: ErrorCode::CredentialsRequired,
                    ..
                }
            ));
        }
        let unrelated =
            foks_keystore::Error::Native("native credentials require user interaction".into());
        assert!(matches!(
            dispatch_error_response(17, &unrelated).result,
            ResponseResult::Error {
                code: ErrorCode::OperationFailed,
                ..
            }
        ));
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
                label: None,
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
        let active_mutation = profile_work::coordinator()
            .try_acquire(directory.path(), profile_work::Scope::profile("hosted"))
            .unwrap()
            .unwrap();
        assert_eq!(
            apply_hosted_lease_if_idle(directory.path(), "hosted", &bytes).unwrap(),
            None
        );
        drop(active_mutation);
        assert_eq!(
            apply_hosted_lease_if_idle(directory.path(), "hosted", &bytes).unwrap(),
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

    #[test]
    fn local_worker_capacity_is_reserved_from_remote_and_recovery_work() {
        let capacity = ConnectionCapacity {
            recovery: Arc::new(Semaphore::new(1)),
            blocking: Arc::new(Semaphore::new(1)),
            local: Arc::new(Semaphore::new(1)),
            chat_polling: Arc::new(Semaphore::new(1)),
            active_chat_polls: Arc::new(Mutex::new(Default::default())),
        };
        let _remote = capacity.blocking.clone().try_acquire_owned().unwrap();
        let _recovery = capacity.recovery.clone().try_acquire_owned().unwrap();
        let alias = Operation::SetLocalAccountAlias {
            profile: "local".into(),
            account_alias: "owner".into(),
            label: "Personal".into(),
        };
        let local = capacity.worker_pool(&alias).try_acquire_owned().unwrap();
        assert!(capacity
            .worker_pool(&Operation::ListProfiles)
            .try_acquire_owned()
            .is_err());
        drop(local);
        assert!(capacity
            .worker_pool(&Operation::ListProfiles)
            .try_acquire_owned()
            .is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn timed_out_workers_observe_cancellation_and_release_capacity() {
        let workers = Arc::new(Semaphore::new(1));
        let permit = workers.clone().acquire_owned().await.unwrap();
        let observed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_observed = Arc::clone(&observed);
        let coordinator = Arc::new(profile_work::Coordinator::default());
        let profile_permit = coordinator
            .try_acquire(Path::new("/worker-test"), profile_work::Scope::profile("a"))
            .unwrap()
            .unwrap();
        let result = supervise_blocking(
            7,
            permit,
            profile_permit,
            Duration::from_millis(20),
            None,
            move |cancellation| {
                while !cancellation.is_cancelled() {
                    std::thread::sleep(Duration::from_millis(1));
                }
                worker_observed.store(true, std::sync::atomic::Ordering::Release);
                Response::success(7, serde_json::json!({ "late": true }))
            },
        )
        .await;

        assert!(!result.abandoned);
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

    #[test]
    fn chat_poll_ownership_is_one_per_profile_account() {
        let first = TeamStoreRef {
            profile: "local".into(),
            account_alias: "personal".into(),
            team_alias: "one".into(),
            team_id: format!("03{}", "11".repeat(32)),
        };
        let second = TeamStoreRef {
            team_alias: "two".into(),
            team_id: format!("03{}", "22".repeat(32)),
            ..first.clone()
        };
        let first_key = ChatPollKey::from(&first);
        let second_key = ChatPollKey::from(&second);
        assert_eq!(first_key, second_key);
        let polls = Arc::new(Mutex::new(std::collections::HashSet::new()));
        assert!(polls.lock().unwrap().insert(first_key.clone()));
        assert!(!polls.lock().unwrap().insert(second_key));
        let guard = ActiveChatPollGuard {
            polls: polls.clone(),
            key: first_key,
        };
        drop(guard);
        assert!(polls.lock().unwrap().is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn completed_workers_hold_profile_admission_until_the_response_is_released() {
        let workers = Arc::new(Semaphore::new(1));
        let coordinator = Arc::new(profile_work::Coordinator::default());
        let root = Path::new("/worker-test");
        let worker_permit = workers.clone().acquire_owned().await.unwrap();
        let profile_permit = coordinator
            .try_acquire(root, profile_work::Scope::profile("a"))
            .unwrap()
            .unwrap();
        let result = supervise_blocking(
            8,
            worker_permit,
            profile_permit,
            Duration::from_secs(1),
            None,
            |_| Response::success(8, serde_json::json!({ "ok": true })),
        )
        .await;

        assert!(coordinator
            .try_acquire(root, profile_work::Scope::profile("a"))
            .unwrap()
            .is_none());
        drop(result);
        assert!(coordinator
            .try_acquire(root, profile_work::Scope::profile("a"))
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn scheduled_batches_release_admission_for_queued_foreground_work() {
        use std::future::Future as _;
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().to_path_buf();
        let workers = Arc::new(Semaphore::new(1));
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (started_tx, mut started_rx) = tokio::sync::mpsc::unbounded_channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = Mutex::new(release_rx);
        let counts = calls.clone();
        let task_root = root.clone();
        let task_workers = workers.clone();
        let scheduled = tokio::spawn(async move {
            run_scheduled_batches(
                &task_root,
                Duration::from_secs(5),
                CancellationToken::new(),
                task_workers,
                || SliceAdmission::SecurityRoot,
                move |_, _, _| {
                    let n = counts.fetch_add(1, Ordering::SeqCst);
                    if n == 0 {
                        started_tx.send(()).unwrap();
                        release_rx
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(5))
                            .unwrap();
                    }
                    Ok(if n == 0 {
                        ScheduledSlice::Ran
                    } else {
                        ScheduledSlice::Idle
                    })
                },
            )
            .await
        });
        started_rx.recv().await.unwrap();
        let mut foreground = Box::pin(profile_work::coordinator().acquire(
            &root,
            profile_work::Scope::profile("a"),
            Duration::from_secs(5),
        ));
        std::future::poll_fn(|cx| {
            assert!(foreground.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        assert_eq!(workers.available_permits(), 0);
        release_tx.send(()).unwrap();
        let foreground = foreground.await.unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "foreground runs before the next job"
        );
        assert_eq!(workers.available_permits(), 1);
        drop(foreground);
        scheduled.await.unwrap().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn scheduled_job_budget_is_bounded_below_the_request_timeout() {
        assert_eq!(
            scheduled_job_budget(Duration::from_secs(60)),
            SCHEDULED_NETWORK_BUDGET
        );
        assert_eq!(
            scheduled_job_budget(Duration::from_secs(5)),
            Duration::from_secs(5)
        );
        assert_eq!(scheduled_job_budget(Duration::ZERO), Duration::ZERO);
        assert!(SCHEDULED_NETWORK_BUDGET < Duration::from_secs(60));
    }

    #[tokio::test]
    async fn scheduled_batches_hand_each_job_the_bounded_network_budget() {
        let temporary = tempfile::tempdir().unwrap();
        let budgets = Arc::new(Mutex::new(Vec::new()));
        let seen = budgets.clone();
        run_scheduled_batches(
            temporary.path(),
            Duration::from_secs(60),
            CancellationToken::new(),
            Arc::new(Semaphore::new(1)),
            || SliceAdmission::SecurityRoot,
            move |remaining, _, _| {
                seen.lock().unwrap().push(remaining);
                Ok(ScheduledSlice::Idle)
            },
        )
        .await
        .unwrap();
        let budgets = budgets.lock().unwrap();
        assert_eq!(budgets.len(), 1);
        assert!(
            budgets[0] <= SCHEDULED_NETWORK_BUDGET,
            "a job under a 60 s request budget got {:?}",
            budgets[0]
        );
    }

    /// A profile's durable job state holding exactly `jobs`, given as
    /// `(kind, next_run_at)`. Identifiers ascend with the list, so the job a
    /// claim would take is the earliest due one.
    fn scheduled_job_state(path: &Path, jobs: &[(foks_client_db::ScheduledJobKind, u64)]) {
        let host = foks_verify::verify_public_host("foks.app", SCHEDULER_PROBE).unwrap();
        foks_client_db::HardStateStore::open(path)
            .unwrap()
            .accept_verified_host(&host.snapshot)
            .unwrap();
        let scheduler =
            foks_client::FoksScheduler::new(path, foks_client::SchedulerConfig::default()).unwrap();
        for (index, (kind, first_run_at)) in jobs.iter().enumerate() {
            scheduler
                .register(foks_client::ScheduledJobRegistration {
                    job_id: [u8::try_from(index + 1).unwrap(); 16],
                    kind: *kind,
                    host_id: host.snapshot.host_id().to_vec(),
                    scope_id: vec![9; 33],
                    interval_micros: 1_000_000,
                    first_run_at: *first_run_at,
                    registered_at: 1,
                })
                .unwrap();
        }
    }

    #[tokio::test]
    async fn a_single_profile_scheduled_job_admits_beside_another_profiles_read() {
        use foks_client_db::ScheduledJobKind;

        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().to_path_buf();
        let hard_database = root.join("hard.sqlite");
        scheduled_job_state(&hard_database, &[(ScheduledJobKind::UserRefresh, 5_000)]);
        assert_eq!(
            scheduled_slice_admission(Some(&hard_database), "a", 10_000),
            SliceAdmission::Profile("a".into())
        );

        let workers = Arc::new(Semaphore::new(1));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let started_tx = Mutex::new(Some(started_tx));
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = Mutex::new(release_rx);
        let task_root = root.clone();
        let task_workers = workers.clone();
        let probed = hard_database.clone();
        let scheduled = tokio::spawn(async move {
            run_scheduled_batches(
                &task_root,
                Duration::from_secs(5),
                CancellationToken::new(),
                task_workers,
                move || scheduled_slice_admission(Some(&probed), "a", 10_000),
                move |_, _, admitted| {
                    assert_eq!(admitted, SliceAdmission::Profile("a".into()));
                    started_tx.lock().unwrap().take().unwrap().send(()).unwrap();
                    release_rx
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap();
                    Ok(ScheduledSlice::Idle)
                },
            )
            .await
        });
        started_rx.await.unwrap();

        // The read this job used to block: another profile's, which shares
        // nothing with the job's profile.
        let other = profile_work::coordinator()
            .try_acquire(&root, profile_work::Scope::SharedProfile("b".into()))
            .unwrap();
        assert!(
            other.is_some(),
            "another profile's read waited behind a single-profile job"
        );
        // The job's own profile stays exclusive while it runs.
        assert!(profile_work::coordinator()
            .try_acquire(&root, profile_work::Scope::profile("a"))
            .unwrap()
            .is_none());
        drop(other);
        release_tx.send(()).unwrap();
        scheduled.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn a_federation_refresh_job_still_takes_the_wide_scope() {
        use foks_client_db::ScheduledJobKind;

        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().to_path_buf();
        let hard_database = root.join("hard.sqlite");
        scheduled_job_state(
            &hard_database,
            &[
                (ScheduledJobKind::FederationRefresh, 5_000),
                (ScheduledJobKind::UserRefresh, 6_000),
            ],
        );
        assert_eq!(
            scheduled_slice_admission(Some(&hard_database), "a", 10_000),
            SliceAdmission::SecurityRoot
        );
        // Durable state that could not be read keeps the wide scope the pass
        // took before the probe existed.
        assert_eq!(
            scheduled_slice_admission(None, "a", 10_000),
            SliceAdmission::SecurityRoot
        );
        assert_eq!(
            scheduled_slice_admission(Some(&hard_database), "a", 5_999),
            SliceAdmission::SecurityRoot
        );
        // Only the job a claim would take decides the scope: a federation job
        // that is not yet due leaves the slice reserving its own profile.
        let later = root.join("later.sqlite");
        scheduled_job_state(
            &later,
            &[
                (ScheduledJobKind::UserRefresh, 5_000),
                (ScheduledJobKind::FederationRefresh, 6_000),
            ],
        );
        assert_eq!(
            scheduled_slice_admission(Some(&later), "a", 5_999),
            SliceAdmission::Profile("a".into())
        );

        let workers = Arc::new(Semaphore::new(1));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let started_tx = Mutex::new(Some(started_tx));
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = Mutex::new(release_rx);
        let task_root = root.clone();
        let task_workers = workers.clone();
        let probed = hard_database.clone();
        let scheduled = tokio::spawn(async move {
            run_scheduled_batches(
                &task_root,
                Duration::from_secs(5),
                CancellationToken::new(),
                task_workers,
                move || scheduled_slice_admission(Some(&probed), "a", 10_000),
                move |_, _, admitted| {
                    assert_eq!(admitted, SliceAdmission::SecurityRoot);
                    started_tx.lock().unwrap().take().unwrap().send(()).unwrap();
                    release_rx
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap();
                    Ok(ScheduledSlice::Idle)
                },
            )
            .await
        });
        started_rx.await.unwrap();
        // A federation refresh can reach a second profile, so every other
        // profile's work, including its reads, still waits behind it.
        assert!(profile_work::coordinator()
            .try_acquire(&root, profile_work::Scope::SharedProfile("b".into()))
            .unwrap()
            .is_none());
        release_tx.send(()).unwrap();
        scheduled.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn a_claim_race_defers_the_slice_and_re_runs_it_under_the_wide_scope() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().to_path_buf();
        let admissions = Arc::new(Mutex::new(Vec::new()));
        let seen = admissions.clone();
        // The probe reads a single-profile job every time; the first claim
        // lands on one that reaches another profile and leaves it unclaimed.
        let result = run_scheduled_batches(
            &root,
            Duration::from_secs(5),
            CancellationToken::new(),
            Arc::new(Semaphore::new(1)),
            || SliceAdmission::Profile("a".into()),
            move |_, _, admitted| {
                let mut seen = seen.lock().unwrap();
                seen.push(admitted);
                Ok(match seen.len() {
                    1 => ScheduledSlice::Deferred,
                    2 => ScheduledSlice::Ran,
                    _ => ScheduledSlice::Idle,
                })
            },
        )
        .await;

        assert!(result.is_ok(), "a deferral was reported as a failure");
        let admissions = admissions.lock().unwrap();
        assert_eq!(
            *admissions,
            vec![
                SliceAdmission::Profile("a".into()),
                // One re-run, under the scope the deferred job needs.
                SliceAdmission::SecurityRoot,
                // The re-run ran a job, so the pass continues from a fresh
                // probe rather than ending on the deferral.
                SliceAdmission::Profile("a".into()),
            ]
        );
        assert!(profile_work::coordinator()
            .try_acquire(&root, profile_work::Scope::Root)
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn a_deferral_under_the_wide_scope_ends_the_pass_instead_of_re_running() {
        let temporary = tempfile::tempdir().unwrap();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counts = calls.clone();
        // Nothing is wider than the state root, so a slice that reports a
        // deferral while holding it must not re-run: the next pass probes
        // again from current state.
        run_scheduled_batches(
            temporary.path(),
            Duration::from_secs(5),
            CancellationToken::new(),
            Arc::new(Semaphore::new(1)),
            || SliceAdmission::SecurityRoot,
            move |_, _, _| {
                counts.fetch_add(1, Ordering::SeqCst);
                Ok(ScheduledSlice::Deferred)
            },
        )
        .await
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn scheduled_batches_stop_at_limit_empty_error_or_cancellation() {
        for case in ["limit", "empty", "error", "cancel"] {
            let temporary = tempfile::tempdir().unwrap();
            let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let counts = calls.clone();
            let workers = Arc::new(Semaphore::new(1));
            let result = run_scheduled_batches(
                temporary.path(),
                Duration::from_secs(5),
                CancellationToken::new(),
                workers.clone(),
                || SliceAdmission::SecurityRoot,
                move |_, control, _| {
                    counts.fetch_add(1, Ordering::SeqCst);
                    match case {
                        "empty" => Ok(ScheduledSlice::Idle),
                        "error" => Err("storage failed".to_owned()),
                        "cancel" => {
                            control.cancel();
                            Ok(ScheduledSlice::Ran)
                        }
                        _ => Ok(ScheduledSlice::Ran),
                    }
                },
            )
            .await;
            assert_eq!(result.is_err(), case == "error");
            assert_eq!(
                calls.load(Ordering::SeqCst),
                if case == "limit" {
                    SCHEDULED_JOBS_PER_PROFILE
                } else {
                    1
                }
            );
            assert_eq!(workers.available_permits(), 1);
            assert!(profile_work::coordinator()
                .try_acquire(temporary.path(), profile_work::Scope::Root)
                .unwrap()
                .is_some());
        }
    }

    #[tokio::test]
    async fn cancelled_scheduler_keeps_admission_until_its_worker_exits() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().to_path_buf();
        let workers = Arc::new(Semaphore::new(1));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let started_tx = Mutex::new(Some(started_tx));
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = Mutex::new(release_rx);
        let task_root = root.clone();
        let task_workers = workers.clone();
        let scheduled = tokio::spawn(async move {
            run_scheduled_batches(
                &task_root,
                Duration::from_secs(5),
                CancellationToken::new(),
                task_workers,
                || SliceAdmission::SecurityRoot,
                move |_, _, _| {
                    started_tx.lock().unwrap().take().unwrap().send(()).unwrap();
                    release_rx
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap();
                    Ok(ScheduledSlice::Ran)
                },
            )
            .await
        });
        started_rx.await.unwrap();
        scheduled.abort();
        assert!(scheduled.await.unwrap_err().is_cancelled());
        assert!(profile_work::coordinator()
            .try_acquire(&root, profile_work::Scope::Root)
            .unwrap()
            .is_none());
        assert_eq!(workers.available_permits(), 0);
        release_tx.send(()).unwrap();
        let _foreground = profile_work::coordinator()
            .acquire(&root, profile_work::Scope::Root, Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(workers.available_permits(), 1);
    }

    #[tokio::test]
    async fn scheduled_admission_does_not_hold_worker_capacity_while_waiting() {
        use std::future::Future as _;
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        ClientCredentials::initialize(&root, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&root).unwrap();
        registry
            .add(Profile {
                name: "a".into(),
                label: None,
                probe: "example.test".into(),
                protocol: ProtocolPolicy::V019,
                trust: TrustRoot::WebPki,
            })
            .unwrap();
        drop(registry);
        let workers = Arc::new(Semaphore::new(1));
        let foreground = profile_work::coordinator()
            .try_acquire(&root, profile_work::Scope::profile("a"))
            .unwrap()
            .unwrap();
        let mut scheduled = Box::pin(run_scheduled_profiles(
            root.clone(),
            Duration::from_secs(2),
            CancellationToken::new(),
            workers.clone(),
        ));
        std::future::poll_fn(|cx| {
            assert!(scheduled.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        assert_eq!(
            workers.available_permits(),
            1,
            "a queued root job cannot consume the foreground worker"
        );
        drop(scheduled);
        drop(foreground);
        assert!(profile_work::coordinator()
            .try_acquire(&root, profile_work::Scope::Root)
            .unwrap()
            .is_some());
    }

    #[test]
    fn checked_lock_wait_never_replays_an_entered_body_and_respects_cancellation() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let credentials =
            ClientCredentials::initialize(&root, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&root).unwrap();
        for name in ["a", "b"] {
            registry
                .add(Profile {
                    name: name.into(),
                    label: None,
                    probe: "example.test".into(),
                    protocol: ProtocolPolicy::V019,
                    trust: TrustRoot::WebPki,
                })
                .unwrap();
        }
        let first = ProfileSession::open(&registry, "a").unwrap();
        let second = ProfileSession::open(&registry, "b").unwrap();
        let mut entries = 0;
        let result =
            profile_work::with_control(Duration::from_secs(1), CancellationToken::new(), || {
                checked_session(&credentials, &first, |_| {
                    entries += 1;
                    Err::<(), Box<dyn std::error::Error>>(Box::new(ProfileBusyError))
                })
            });
        assert!(result.is_err());
        assert_eq!(
            entries, 1,
            "an error after entering the callback must not replay it"
        );
        let result =
            profile_work::with_control(Duration::from_secs(1), CancellationToken::new(), || {
                checked_sessions(&credentials, &first, &second, |_, _| {
                    entries += 1;
                    Err::<(), Box<dyn std::error::Error>>(Box::new(ProfileBusyError))
                })
            });
        assert!(result.is_err());
        assert_eq!(entries, 2);
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let result = profile_work::with_control(Duration::from_secs(1), cancellation, || {
            checked_session(&credentials, &first, |_| {
                entries += 1;
                Ok(())
            })
        });
        assert!(result.is_err());
        assert_eq!(
            entries, 2,
            "cancellation before admission must not enter the callback"
        );
    }

    #[tokio::test]
    async fn timed_out_worker_retains_profile_until_its_actual_exit() {
        let workers = Arc::new(Semaphore::new(1));
        let coordinator = Arc::new(profile_work::Coordinator::default());
        let root = Path::new("/stuck-worker-test");
        let profile_permit = coordinator
            .try_acquire(root, profile_work::Scope::profile("a"))
            .unwrap()
            .unwrap();
        let permit = workers.clone().acquire_owned().await.unwrap();
        let (entered, entry) = tokio::sync::oneshot::channel();
        let (release, held) = std::sync::mpsc::channel();
        let task = tokio::spawn(supervise_blocking(
            10,
            permit,
            profile_permit,
            Duration::from_millis(20),
            None,
            move |_| {
                entered.send(()).unwrap();
                held.recv().unwrap();
                Response::success(10, serde_json::json!({"ok":true}))
            },
        ));
        entry.await.unwrap();
        let response = task.await.unwrap();
        assert!(response.close_connection);
        drop(response);
        assert_eq!(workers.available_permits(), 0);
        assert!(coordinator
            .try_acquire(root, profile_work::Scope::profile("a"))
            .unwrap()
            .is_none());
        release.send(()).unwrap();
        // Wait on actual permit release, without a polling delay or wider timeout.
        drop(
            coordinator
                .acquire(
                    root,
                    profile_work::Scope::profile("a"),
                    Duration::from_secs(2),
                )
                .await
                .unwrap(),
        );
        assert_eq!(workers.available_permits(), 1);
    }

    /// A client that retires a read drops its connection. Until the agent
    /// observes that, the request holds this profile's admission and the
    /// request behind it waits for a reply nobody will read.
    #[tokio::test]
    async fn an_abandoned_read_releases_its_profile_before_its_deadline() {
        let workers = Arc::new(Semaphore::new(1));
        let coordinator = Arc::new(profile_work::Coordinator::default());
        let root = Path::new("/abandoned-read-test");
        let profile_permit = coordinator
            .try_acquire(root, profile_work::Scope::profile("a"))
            .unwrap()
            .unwrap();
        let permit = workers.clone().acquire_owned().await.unwrap();
        let (mut agent_side, client_side) = tokio::net::UnixStream::pair().unwrap();
        let observed = Arc::new(AtomicBool::new(false));
        let worker_observed = Arc::clone(&observed);
        let supervised = tokio::spawn(async move {
            supervise_blocking(
                11,
                permit,
                profile_permit,
                // Far past what this test waits for: the client's departure
                // is what ends the request, not the deadline.
                Duration::from_secs(120),
                Some(&mut agent_side),
                move |cancellation| {
                    while !cancellation.is_cancelled() {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    worker_observed.store(true, Ordering::Release);
                    Response::success(11, serde_json::json!({ "late": true }))
                },
            )
            .await
        });
        drop(client_side);
        let response = supervised.await.unwrap();

        assert!(response.abandoned);
        assert!(response.close_connection);
        assert!(observed.load(Ordering::Acquire));
        drop(response);
        // The profile and the worker are free for the request that was
        // waiting behind this one.
        assert!(coordinator
            .try_acquire(root, profile_work::Scope::profile("a"))
            .unwrap()
            .is_some());
        assert_eq!(workers.available_permits(), 1);
    }

    /// A monitored read completes normally if the client remains connected.
    #[tokio::test]
    async fn a_watched_read_whose_client_stays_answers_normally() {
        let workers = Arc::new(Semaphore::new(1));
        let coordinator = Arc::new(profile_work::Coordinator::default());
        let root = Path::new("/watched-read-test");
        let profile_permit = coordinator
            .try_acquire(root, profile_work::Scope::profile("a"))
            .unwrap()
            .unwrap();
        let permit = workers.clone().acquire_owned().await.unwrap();
        let (mut agent_side, client_side) = tokio::net::UnixStream::pair().unwrap();
        let response = supervise_blocking(
            12,
            permit,
            profile_permit,
            Duration::from_secs(5),
            Some(&mut agent_side),
            |_| Response::success(12, serde_json::json!({ "ok": true })),
        )
        .await;

        assert!(!response.abandoned);
        assert!(!response.close_connection);
        assert!(matches!(
            response.response.result,
            foks_agent_proto::ResponseResult::Success { .. }
        ));
        drop(client_side);
    }

    /// Only an operation that commits nothing is ended when its client
    /// leaves. A mutation runs to its own conclusion, so its outcome is never
    /// unknown to the agent that performed it.
    #[test]
    fn only_operations_that_commit_nothing_are_abandoned_with_their_client() {
        use foks_agent_proto::chat::ChatAction;
        let team = TeamStoreRef {
            profile: "local".into(),
            account_alias: "owner".into(),
            team_alias: "team".into(),
            team_id: format!("03{}", "11".repeat(32)),
        };
        for operation in [
            Operation::ListKnownStores {
                profile: "local".into(),
            },
            Operation::ListProfileOverview {
                profile: "local".into(),
            },
            Operation::ListTeamMembers {
                profile: "local".into(),
                team_alias: "team".into(),
            },
            Operation::ListKv {
                store: foks_agent_proto::AccountStoreRef {
                    profile: "local".into(),
                    account_alias: "owner".into(),
                },
                cursor: None,
                limit: 1,
                fresh: false,
            },
            Operation::Chat {
                store: team.clone(),
                action: ChatAction::Pending,
            },
        ] {
            assert!(
                read_cache::operation_is_abandonable(&operation),
                "expected an abandonable read: {operation:?}"
            );
        }
        for operation in [
            // Writes, of this user's material or of the server's.
            Operation::RemoveDevice {
                profile: "local".into(),
                signer_alias: "owner".into(),
                device_id: "00".into(),
            },
            Operation::Chat {
                store: team,
                action: ChatAction::MarkRead {
                    channel: "00".repeat(16),
                    sequence: "1".into(),
                },
            },
            // Host contact that pins trust or renews a lease, which is not
            // abandoned half-done even though it retains no material.
            Operation::Probe {
                profile: "local".into(),
            },
            Operation::ReconcileProfile {
                profile: "local".into(),
            },
            Operation::RefreshLease {
                profile: "local".into(),
            },
        ] {
            assert!(
                !read_cache::operation_is_abandonable(&operation),
                "expected an operation that runs to its conclusion: {operation:?}"
            );
        }
    }

    #[test]
    fn admission_budget_leaves_the_reply_room_before_the_client_deadline() {
        assert_eq!(
            admission_budget(Duration::from_secs(60)),
            Duration::from_secs(58)
        );
        assert_eq!(
            admission_budget(Duration::from_secs(15)),
            Duration::from_secs(13)
        );
        assert_eq!(
            admission_budget(Duration::from_secs(1)),
            Duration::from_millis(750)
        );
        assert_eq!(admission_budget(Duration::ZERO), Duration::ZERO);
    }

    /// The desktop launches the agent with the budget its client waits, and
    /// the agent's clock starts after the request is read. An admission that
    /// runs out must still answer "not started" with time to spare before that
    /// client gives up, or the client reports an ambiguous timeout for a
    /// request the agent never began.
    #[tokio::test]
    async fn admission_deadline_answers_before_a_client_with_the_same_budget_gives_up() {
        let temporary = tempfile::tempdir().unwrap();
        let state_dir = temporary.path().to_path_buf();
        let held = profile_work::coordinator()
            .try_acquire(&state_dir, profile_work::Scope::profile("a"))
            .unwrap()
            .unwrap();
        let timeout = Duration::from_secs(1);
        let capacity = ConnectionCapacity {
            recovery: Arc::new(Semaphore::new(1)),
            blocking: Arc::new(Semaphore::new(1)),
            local: Arc::new(Semaphore::new(1)),
            chat_polling: Arc::new(Semaphore::new(1)),
            active_chat_polls: Arc::new(Mutex::new(std::collections::HashSet::new())),
        };
        let active = Arc::new(Semaphore::new(1));
        let permit = active.clone().acquire_owned().await.unwrap();
        let (mut client, server_stream) = tokio::net::UnixStream::pair().unwrap();
        let server = tokio::spawn(handle_connection(
            server_stream,
            state_dir.clone(),
            capacity,
            Arc::new(AtomicBool::new(true)),
            timeout,
            permit,
        ));
        let frame = foks_agent_proto::encode(&foks_agent_proto::Request {
            version: foks_agent_proto::PROTOCOL_VERSION,
            id: 7,
            operation: Operation::ListPendingOperations {
                profile: "a".to_owned(),
            },
        })
        .unwrap();
        // A client's deadline runs from before it writes the request.
        let started = Instant::now();
        let answered = tokio::time::timeout(timeout * 2, async {
            client.write_all(&frame).await.unwrap();
            read_frame(&mut client).await.unwrap()
        })
        .await;
        let elapsed = started.elapsed();
        drop(held);
        server.abort();
        let frame = answered
            .expect("the agent answered within twice the budget")
            .expect("a response frame");
        let response = foks_agent_proto::decode_response(&frame).unwrap();
        assert!(
            matches!(
                response.result,
                ResponseResult::Error { code: ErrorCode::Busy, ref fields, .. }
                if fields.reason.as_deref() == Some("admission-not-started")
            ),
            "the client saw {:?}",
            response.result
        );
        assert!(
            elapsed + Duration::from_millis(100) <= timeout,
            "the answer left at {elapsed:?}, too late for a client that waits {timeout:?}"
        );
    }

    #[test]
    fn admission_errors_are_definite_non_execution_and_never_ambiguous_timeouts() {
        for error in [
            profile_work::AdmissionError::Full,
            profile_work::AdmissionError::Deadline,
            profile_work::AdmissionError::Cancelled,
        ] {
            let response = dispatch_error_response(9, &error);
            assert!(
                matches!(response.result, ResponseResult::Error { code: ErrorCode::Busy, fields, .. }
                if fields.reason.as_deref() == Some("admission-not-started"))
            );
        }
    }

    fn catalog_store(alias: &str) -> CatalogStoreBinding {
        CatalogStoreBinding::Account {
            value: AccountStoreRef {
                profile: "local".to_owned(),
                account_alias: alias.to_owned(),
            },
        }
    }

    fn catalog_report(snapshot_version: u64, entries: u64) -> foks_client_app::KvCatalogReport {
        foks_client_app::KvCatalogReport {
            snapshot_version,
            entries: (0..entries)
                .map(|index| foks_client_app::KvCatalogEntry {
                    path: format!("/{index}"),
                    node_type: "small-file".to_owned(),
                    version: index + 1,
                    size: None,
                    read_role: foks_client_app::KvRoleSummary::Owner,
                    write_role: foks_client_app::KvRoleSummary::Owner,
                })
                .collect(),
        }
    }

    fn cache_entry(store: &CatalogStoreBinding, version: u64, stored_at: Instant) -> CachedCatalog {
        let report = catalog_report(version, 1);
        let (digest, encoded_bytes) = catalog_snapshot_identity(&report).unwrap();
        CachedCatalog {
            store: store.clone(),
            digest,
            report,
            encoded_bytes,
            stored_at,
        }
    }

    #[test]
    fn a_first_page_is_served_from_the_retained_report_until_it_expires() {
        let store = catalog_store("personal");
        let mut cache = CatalogCache::default();
        let start = Instant::now();
        cache.put_at(cache_entry(&store, 7, start), start);
        assert!(cache
            .get_at(
                &store,
                None,
                start + CATALOG_CACHE_LIFETIME - Duration::from_secs(1)
            )
            .is_some());
        assert!(cache
            .get_at(&store, None, start + CATALOG_CACHE_LIFETIME)
            .is_none());
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.encoded_bytes, 0);
    }

    #[test]
    fn a_fresh_first_page_is_never_served_from_the_retained_report() {
        // A request after a local write asks for the first page with `fresh`
        // set, and only the first page: a later page is pinned by its cursor
        // to the snapshot the walk started from.
        assert!(!may_serve_cached_catalog(None, true));
        assert!(may_serve_cached_catalog(None, false));
        assert!(may_serve_cached_catalog(Some("v2.cursor"), true));
        assert!(may_serve_cached_catalog(Some("v2.cursor"), false));
    }

    #[test]
    fn a_later_page_is_served_only_from_the_snapshot_its_cursor_names() {
        let cache = Mutex::new(CatalogCache::default());
        let store = catalog_store("personal");
        let report = catalog_report(7, 3);
        let first =
            paginate_fresh_catalog_in(&cache, report.clone(), store.clone(), None, 2).unwrap();
        let cursor = first.next_cursor.clone().unwrap();
        assert!(paginate_cached_catalog_in(&cache, &store, Some(&cursor), 2)
            .unwrap()
            .is_some());
        // A write replaces the retained report. The cursor the desktop still
        // holds names the previous snapshot, which is no longer retained, so
        // it is not answered from the new one.
        let rewritten = catalog_report(8, 3);
        paginate_fresh_catalog_in(&cache, rewritten, store.clone(), None, 2).unwrap();
        assert!(paginate_cached_catalog_in(&cache, &store, Some(&cursor), 2)
            .unwrap()
            .is_none());
        assert!(paginate_cached_catalog_in(&cache, &store, None, 2)
            .unwrap()
            .is_some_and(|page| page.snapshot_version == 8));
    }

    #[test]
    fn retained_catalogs_are_bounded_and_evicted_oldest_first() {
        let mut cache = CatalogCache::default();
        let start = Instant::now();
        let stores = (0..MAXIMUM_CACHED_CATALOGS + 1)
            .map(|index| catalog_store(&format!("store-{index}")))
            .collect::<Vec<_>>();
        for (index, store) in stores.iter().enumerate() {
            // Distinct, increasing storage times: the entry evicted must be
            // the oldest one, not whichever entry a sweep reached first.
            cache.put_at(
                cache_entry(store, 1, start + Duration::from_millis(index as u64)),
                start + Duration::from_millis(index as u64),
            );
        }
        assert_eq!(cache.len(), MAXIMUM_CACHED_CATALOGS);
        assert!(cache.get_at(&stores[0], None, start).is_none());
        for store in &stores[1..] {
            assert!(cache.get_at(store, None, start).is_some());
        }
        assert_eq!(
            cache.encoded_bytes,
            cache
                .entries
                .iter()
                .map(|entry| entry.encoded_bytes)
                .sum::<usize>()
        );
    }

    #[test]
    fn storing_a_store_again_replaces_its_retained_report() {
        let store = catalog_store("personal");
        let mut cache = CatalogCache::default();
        let start = Instant::now();
        cache.put_at(cache_entry(&store, 1, start), start);
        cache.put_at(cache_entry(&store, 2, start), start);
        assert_eq!(cache.len(), 1);
        assert_eq!(
            cache
                .get_at(&store, None, start)
                .unwrap()
                .report
                .snapshot_version,
            2
        );
        assert_eq!(cache.encoded_bytes, cache.entries[0].encoded_bytes);
    }

    #[test]
    fn a_fresh_report_is_serialized_and_hashed_once_for_every_page() {
        let cache = Mutex::new(CatalogCache::default());
        let store = catalog_store("personal");
        let report = catalog_report(7, 5);
        let before = CATALOG_IDENTITY_COMPUTATIONS.get();
        let mut page = paginate_fresh_catalog_in(&cache, report, store.clone(), None, 2).unwrap();
        let mut pages = 1;
        while let Some(cursor) = page.next_cursor.clone() {
            page = paginate_cached_catalog_in(&cache, &store, Some(&cursor), 2)
                .unwrap()
                .expect("the retained report serves every later page");
            pages += 1;
        }
        assert_eq!(pages, 3);
        assert_eq!(CATALOG_IDENTITY_COMPUTATIONS.get() - before, 1);
    }

    #[test]
    fn a_write_through_this_agent_drops_every_retained_catalog() {
        let cache = Mutex::new(CatalogCache::default());
        let store = catalog_store("personal");
        paginate_fresh_catalog_in(&cache, catalog_report(7, 3), store.clone(), None, 2).unwrap();
        assert!(paginate_cached_catalog_in(&cache, &store, None, 2)
            .unwrap()
            .is_some());
        // The lifetime bounds staleness against writes made on other devices.
        // A write made through this agent, by any client and whether or not
        // it asked for a fresh listing, is not one of those.
        invalidate_catalog_cache(&cache);
        assert!(paginate_cached_catalog_in(&cache, &store, None, 2)
            .unwrap()
            .is_none());
    }

    #[test]
    fn catalog_cursors_bind_store_snapshot_and_offset() {
        let cache = Mutex::new(CatalogCache::default());
        let store = CatalogStoreBinding::Account {
            value: AccountStoreRef {
                profile: "local".to_owned(),
                account_alias: "personal".to_owned(),
            },
        };
        let empty_report = foks_client_app::KvCatalogReport {
            snapshot_version: 0,
            entries: Vec::new(),
        };
        let empty = paginate_catalog(
            &empty_report,
            store.clone(),
            catalog_snapshot_identity(&empty_report).unwrap().0,
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
        let (digest, encoded_bytes) = catalog_snapshot_identity(&report).unwrap();
        let first = paginate_catalog(&report, store.clone(), digest, None, 2).unwrap();
        assert_eq!(first.entries.len(), 2);
        let second = paginate_catalog(
            &report,
            store.clone(),
            digest,
            first.next_cursor.as_deref(),
            2,
        )
        .unwrap();
        assert_eq!(second.entries[0].path, "/2");
        assert!(second.next_cursor.is_none());
        cache_catalog(&cache, store.clone(), &report, digest, encoded_bytes).unwrap();
        let cached = paginate_cached_catalog_in(&cache, &store, first.next_cursor.as_deref(), 2)
            .unwrap()
            .unwrap();
        assert_eq!(cached.entries[0].path, "/2");
        assert!(cached.next_cursor.is_none());
        // A completed pagination no longer drops the entry: only expiry and
        // eviction do, so the next pass over the same store is served from it.
        assert!(
            paginate_cached_catalog_in(&cache, &store, first.next_cursor.as_deref(), 2)
                .unwrap()
                .is_some()
        );

        let another_store = CatalogStoreBinding::Account {
            value: AccountStoreRef {
                profile: "local".to_owned(),
                account_alias: "other".to_owned(),
            },
        };
        assert!(paginate_catalog(
            &report,
            another_store,
            digest,
            first.next_cursor.as_deref(),
            2,
        )
        .is_err());
        let mut changed = report;
        changed.snapshot_version += 1;
        assert!(paginate_catalog(
            &changed,
            store.clone(),
            catalog_snapshot_identity(&changed).unwrap().0,
            first.next_cursor.as_deref(),
            2,
        )
        .is_err());
        changed.snapshot_version -= 1;
        changed.entries[0].path = "/same-version-different-branch".to_owned();
        assert!(paginate_catalog(
            &changed,
            store,
            catalog_snapshot_identity(&changed).unwrap().0,
            first.next_cursor.as_deref(),
            2,
        )
        .is_err());

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
        let page = paginate_catalog(
            &large,
            large_store,
            catalog_snapshot_identity(&large).unwrap().0,
            None,
            500,
        )
        .unwrap();
        assert!(!page.entries.is_empty());
        assert!(page.entries.len() < 500);
        assert!(page.next_cursor.is_some());
        foks_agent_proto::encode(&Response::success(11, serde_json::to_value(page).unwrap()))
            .unwrap();
    }

    /// The KV payload bound is a refusal, not a truncation.
    ///
    /// Both bounds are checked before the request reaches a profile, so this
    /// runs against an empty registry: a request at the bound gets as far as
    /// the profile lookup, and one byte past it is refused for its size.
    #[test]
    fn a_kv_payload_one_byte_past_the_bound_is_refused_for_its_size() {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join("state");
        ProfileRegistry::open(&state).unwrap();
        let store = KvStoreRef::Account(AccountStoreRef {
            profile: "local".to_owned(),
            account_alias: "personal".to_owned(),
        });
        let dispatch = |operation| {
            dispatch_result(
                &state,
                operation,
                Duration::from_secs(5),
                CancellationToken::new(),
                true,
            )
            .expect_err("no profile is configured, so nothing here can succeed")
            .to_string()
        };
        let chunk = |length: u32| Operation::ReadKvChunk {
            store: store.clone(),
            path: "/large".to_owned(),
            version: 1,
            offset: 0,
            length,
        };
        let bound = u32::try_from(MAXIMUM_LOCAL_KV_CHUNK_BYTES).unwrap();
        assert!(!dispatch(chunk(bound)).contains("outside supported bounds"));
        assert!(dispatch(chunk(bound + 1)).contains("outside supported bounds"));
        assert!(dispatch(chunk(0)).contains("outside supported bounds"));

        let put = |length: usize| Operation::PutKv {
            store: store.clone(),
            path: "/inline".to_owned(),
            content: vec![0x5a; length],
            read_role: foks_agent_proto::KvRole::Owner,
            write_role: foks_agent_proto::KvRole::Owner,
            precondition: foks_agent_proto::KvPrecondition::Create,
            mkdir_p: false,
        };
        assert!(!dispatch(put(MAXIMUM_INLINE_KV_BYTES)).contains("exceeds its local protocol"));
        assert!(dispatch(put(MAXIMUM_INLINE_KV_BYTES + 1)).contains("exceeds its local protocol"));
    }

    /// Walks a store larger than one page at the largest row limit this agent
    /// accepts, and asserts the walk sees every entry once, in snapshot order.
    #[test]
    fn a_store_larger_than_one_page_paginates_through_every_entry() {
        const LIMIT: u32 = 4096;
        let store = catalog_store("personal");
        let report = catalog_report(9, u64::from(LIMIT) + 404);
        let digest = catalog_snapshot_identity(&report).unwrap().0;
        let mut collected = Vec::new();
        let mut cursor: Option<String> = None;
        let mut pages = 0usize;
        loop {
            let page =
                paginate_catalog(&report, store.clone(), digest, cursor.as_deref(), LIMIT).unwrap();
            pages += 1;
            assert_eq!(page.snapshot_version, report.snapshot_version);
            assert!(!page.entries.is_empty());
            if pages == 1 {
                // The point of the raised cap: a page is bounded by its
                // encoded size, not by a row count a caller cannot raise.
                assert!(page.entries.len() > 500, "{} rows", page.entries.len());
            }
            collected.extend(page.entries.into_iter().map(|entry| entry.path));
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        assert!(pages > 1);
        assert_eq!(collected.len(), report.entries.len());
        assert!(collected
            .iter()
            .zip(&report.entries)
            .all(|(path, entry)| *path == entry.path));
    }

    /// Long authenticated paths make the encoded-byte budget bind before the
    /// row limit does. The short page must still carry a cursor that resumes
    /// at exactly the first entry it dropped.
    #[test]
    fn a_page_truncated_at_the_byte_budget_returns_a_usable_cursor() {
        const LIMIT: u32 = 4096;
        let store = catalog_store("wide");
        let report = foks_client_app::KvCatalogReport {
            snapshot_version: 12,
            entries: (0..u64::from(LIMIT))
                .map(|index| foks_client_app::KvCatalogEntry {
                    path: format!("/{index}-{}", "p".repeat(2048)),
                    node_type: "small-file".to_owned(),
                    version: index + 1,
                    size: None,
                    read_role: foks_client_app::KvRoleSummary::Owner,
                    write_role: foks_client_app::KvRoleSummary::Owner,
                })
                .collect(),
        };
        let digest = catalog_snapshot_identity(&report).unwrap().0;
        let first = paginate_catalog(&report, store.clone(), digest, None, LIMIT).unwrap();
        assert!(!first.entries.is_empty());
        assert!(first.entries.len() < report.entries.len());
        let cursor = first
            .next_cursor
            .clone()
            .expect("a truncated page names where it stopped");
        let kept = first.entries.len();
        // A truncated page still has to fit the frame it is answered in.
        let frame =
            foks_agent_proto::encode(&Response::success(12, serde_json::to_value(first).unwrap()))
                .unwrap();
        assert!(frame.len() <= MAXIMUM_MESSAGE_BYTES);

        let mut collected = kept;
        let mut cursor = Some(cursor);
        while let Some(next) = cursor {
            let page =
                paginate_catalog(&report, store.clone(), digest, Some(&next), LIMIT).unwrap();
            assert!(!page.entries.is_empty());
            assert_eq!(page.entries[0].path, report.entries[collected].path);
            collected += page.entries.len();
            cursor = page.next_cursor;
        }
        assert_eq!(collected, report.entries.len());
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

    /// A state root with one offline profile, enough for every arm below to
    /// reach its account vault before the operation itself fails for want of
    /// an account, a team or a server.
    fn offline_state(directory: &tempfile::TempDir) -> PathBuf {
        let state = directory.path().join("state");
        ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&state).unwrap();
        registry
            .add(Profile {
                name: "local".to_owned(),
                label: None,
                probe: "foks.app".to_owned(),
                protocol: ProtocolPolicy::V019,
                trust: TrustRoot::WebPki,
            })
            .unwrap();
        state
    }

    fn credential_reads_for(
        state: &Path,
        id: u64,
        operation: Operation,
    ) -> credential_reads::Counts {
        credential_reads::reset();
        let _ = dispatch(state, Request::new(id, operation));
        credential_reads::counts()
    }

    fn account_store() -> KvStoreRef {
        KvStoreRef::Account(AccountStoreRef {
            profile: "local".to_owned(),
            account_alias: "personal".to_owned(),
        })
    }

    /// One dispatched operation opens the client credentials once and reads
    /// the vault wrapping key once, and never opens a second handle from
    /// inside its checked session.
    ///
    /// The count is what the native backend pays: an open is two manifest
    /// reads — the namespace-readiness read the state lease takes and the
    /// root-binding verification — and the key is a third. Each arm listed
    /// here used to open a second handle inside the session closure, so each
    /// paid two or three of those reads again. The nested count is the one
    /// that matters beyond cost: the manifest lock is a non-reentrant file
    /// lock and is otherwise always innermost, and a nested open takes it
    /// from inside the span that already holds the profile and database
    /// locks.
    ///
    /// The claim is per listed operation, not agent-wide. The submission
    /// upload in `data.rs` is the one arm left out that still takes the
    /// shared-handle path: it needs a live upload channel, so it cannot be
    /// dispatched here.
    #[test]
    fn one_operation_opens_client_credentials_once_outside_its_checked_session() {
        let directory = tempfile::tempdir().unwrap();
        let state = offline_state(&directory);
        let expected = credential_reads::Counts {
            opens: 1,
            nested_opens: 0,
            master_key_reads: 1,
        };
        let mut observed = Vec::new();
        for (id, operation) in [
            (
                1,
                Operation::ListProfileOverview {
                    profile: "local".to_owned(),
                },
            ),
            (
                2,
                Operation::ListAccounts {
                    profile: "local".to_owned(),
                },
            ),
            (
                3,
                Operation::ListYubiAccounts {
                    profile: "local".to_owned(),
                },
            ),
            (
                4,
                Operation::RemoveDevice {
                    profile: "local".to_owned(),
                    signer_alias: "personal".to_owned(),
                    device_id: "0".repeat(66),
                },
            ),
            (
                5,
                Operation::MkdirKv {
                    store: account_store(),
                    path: "/directory".to_owned(),
                    read_role: KvRole::Admin,
                    write_role: KvRole::Admin,
                    precondition: KvPrecondition::Create,
                    mkdir_p: false,
                },
            ),
            (
                6,
                Operation::PutKvSymlink {
                    store: account_store(),
                    path: "/link".to_owned(),
                    target: "/directory".to_owned(),
                    read_role: KvRole::Admin,
                    write_role: KvRole::Admin,
                    precondition: KvPrecondition::Create,
                    mkdir_p: false,
                },
            ),
            (
                7,
                Operation::RemoveKv {
                    store: account_store(),
                    path: "/directory".to_owned(),
                    recursive: false,
                    precondition: KvPrecondition::ExactVersion { version: 1 },
                },
            ),
            (
                8,
                Operation::DemoteTeamMember {
                    profile: "local".to_owned(),
                    team_alias: "team".to_owned(),
                    party_id_hex: "0".repeat(66),
                    role: TeamRole::Member,
                    visibility: 0,
                },
            ),
            (
                9,
                Operation::RemoveTeamMember {
                    profile: "local".to_owned(),
                    team_alias: "team".to_owned(),
                    party_id_hex: "0".repeat(66),
                },
            ),
            (
                10,
                Operation::ResumeTeamMemberEdit {
                    profile: "local".to_owned(),
                    team_alias: "team".to_owned(),
                },
            ),
            (
                11,
                Operation::ExpelFederatedTeam {
                    profile: "local".to_owned(),
                    team_alias: "team".to_owned(),
                    remote_host_id_hex: "0".repeat(66),
                    remote_team_id_hex: "0".repeat(66),
                },
            ),
            // The chat dispatch takes the handle its wrapper holds rather
            // than opening one in its send and attempt arms.
            (
                12,
                Operation::Chat {
                    store: TeamStoreRef {
                        profile: "local".to_owned(),
                        account_alias: "personal".to_owned(),
                        team_alias: "team".to_owned(),
                        team_id: "0".repeat(66),
                    },
                    action: foks_agent_proto::chat::ChatAction::Inbox,
                },
            ),
            // The submission control arm in `data.rs`, for the same reason.
            (
                13,
                Operation::PrepareDataWrite {
                    scope: foks_agent_proto::data::DataScope {
                        profile: "local".to_owned(),
                        account_alias: "personal".to_owned(),
                        host_id: "0".repeat(66),
                        user_id: "0".repeat(66),
                        team_id: None,
                    },
                    submission_id: format!("v1-{}-{}", "0".repeat(16), "0".repeat(32)),
                    spec: foks_agent_proto::data::DataWriteSpec {
                        kind: foks_agent_proto::data::DataWriteKind::Mkdir,
                        path: "/directory".to_owned(),
                        destination: None,
                        team_selector: None,
                        overwrite: false,
                        mkdir_p: false,
                        recursive: false,
                        body_length: 0,
                        body_hash: [0; 32],
                    },
                },
            ),
        ] {
            let name = operation.name();
            observed.push((name, credential_reads_for(&state, id, operation)));
        }
        assert!(
            observed.iter().all(|(_, counts)| *counts == expected),
            "expected {expected:?} per operation, observed {observed:?}"
        );
    }

    /// Sharing one credentials handle across an operation does not weaken the
    /// import gate: it is enforced inside the checked session, on every entry,
    /// by the profile check the session wrapper runs, not by the open.
    #[test]
    fn a_shared_credentials_handle_still_refuses_a_profile_awaiting_import_verification() {
        let directory = tempfile::tempdir().unwrap();
        let state = offline_state(&directory);
        let paths = ProfileRegistry::open(&state)
            .unwrap()
            .prepare_profile_directory("local")
            .unwrap();

        let listed = dispatch(
            &state,
            Request::new(
                1,
                Operation::ListPendingOperations {
                    profile: "local".to_owned(),
                },
            ),
        );
        assert!(
            matches!(listed.result, ResponseResult::Success { .. }),
            "unexpected pending-operation listing: {listed:?}"
        );

        foks_client_db::HardStateStore::open(&paths.hard_database)
            .unwrap()
            .install_import_readiness(
                [4; 16],
                [5; 32],
                &[foks_client_db::ImportAccount {
                    alias: "personal".to_owned(),
                    kind: foks_client_db::ImportAccountKind::Software,
                }],
            )
            .unwrap();

        for (id, operation) in [
            (
                2,
                Operation::ListPendingOperations {
                    profile: "local".to_owned(),
                },
            ),
            (
                3,
                Operation::MkdirKv {
                    store: account_store(),
                    path: "/directory".to_owned(),
                    read_role: KvRole::Admin,
                    write_role: KvRole::Admin,
                    precondition: KvPrecondition::Create,
                    mkdir_p: false,
                },
            ),
        ] {
            let name = operation.name();
            let response = dispatch(&state, Request::new(id, operation));
            assert!(
                matches!(
                    response.result,
                    ResponseResult::Error {
                        code: ErrorCode::ImportVerificationRequired,
                        ..
                    }
                ),
                "{name} was not refused by the import gate: {response:?}"
            );
        }
    }
}
