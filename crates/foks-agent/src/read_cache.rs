//! Process-wide reuse of authenticated users, activated team views, and
//! transports for read operations.
//!
//! Desktop catalog work performs several reads against the same user and team.
//! These caches avoid repeated authentication, user-chain, PUK-parcel, and
//! team-view activation requests within a short bounded interval.
//!
//! Only operations listed by [`operation_serves_reads`] receive cached state.
//! Entries have count and lifetime limits. Operations that may mutate state
//! clear all entries before and after execution, including on failure, so
//! concurrent reads cannot leave stale entries after a mutation.
//!
//! Changes made by another device can make a cached entry stale. The server
//! still validates the device credential on every request, and response data
//! is verified with keys from authenticated state. Stale entries therefore
//! cause a failed read rather than unverified output. [`crate::dispatch_result`]
//! clears the affected profile and retries such failures once with fresh
//! authentication.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use foks_agent_proto::Operation;
use foks_client::{AuthenticatedUserOutcome, TeamViewGrant};
use foks_client_app::{
    AuthCacheKey, AuthenticatedUserCache, CancellationToken, ProfileRegistry, ProfileSession,
    ReadCaches, TeamViewCacheKey, TeamViewTokenCache,
};

/// At or below the desktop's 30-second catalog cadence, so a catalog pass and
/// the reads it triggers share one authentication and the next pass does not.
const AUTHENTICATED_USER_CACHE_LIFETIME: Duration = Duration::from_secs(30);
/// One entry per (profile, host, credential); 64 covers every profile a
/// desktop realistically drives at once and bounds resident PUK seeds.
const MAXIMUM_CACHED_AUTHENTICATED_USERS: usize = 64;
/// Well inside the server's six-hour view lifetime, so an entry that is still
/// held is almost never refused for age alone.
const TEAM_VIEW_TOKEN_CACHE_LIFETIME: Duration = Duration::from_secs(5 * 60 * 60);
/// One entry per (profile, actor, team).
const MAXIMUM_CACHED_TEAM_VIEWS: usize = 64;
/// One base transport per (state root, profile). A desktop holds a handful of
/// profiles; the bound only prevents unbounded growth on a state root that is
/// reconfigured many times within one agent lifetime.
const MAXIMUM_BASE_CLIENTS: usize = 16;

/// One bounded, expiring store. The two caches differ only in what they hold,
/// so the eviction rules live here once and take the current time explicitly.
struct Expiring<K, V> {
    entries: VecDeque<(K, V, Instant)>,
    lifetime: Duration,
    maximum: usize,
}

impl<K: PartialEq, V: Clone> Expiring<K, V> {
    fn new(lifetime: Duration, maximum: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            lifetime,
            maximum,
        }
    }

    fn get_at(&mut self, key: &K, now: Instant) -> Option<V> {
        self.expire(now);
        self.entries
            .iter()
            .find(|(stored, _, _)| stored == key)
            .map(|(_, value, _)| value.clone())
    }

    fn put_at(&mut self, key: K, value: V, now: Instant) {
        self.expire(now);
        self.entries.retain(|(stored, _, _)| *stored != key);
        while self.entries.len() >= self.maximum {
            self.entries.pop_front();
        }
        self.entries.push_back((key, value, now));
    }

    /// Keeps only the entries whose key the predicate accepts. Dropping an
    /// entry drops its value, which is what clears an outcome's PUK seeds once
    /// no read still holds it.
    fn retain(&mut self, keep: impl Fn(&K) -> bool) {
        self.entries.retain(|(key, _, _)| keep(key));
    }

    fn clear(&mut self) {
        self.entries.clear();
    }

    fn expire(&mut self, now: Instant) {
        let lifetime = self.lifetime;
        self.entries
            .retain(|(_, _, stored_at)| now.duration_since(*stored_at) < lifetime);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

struct CacheState {
    users: Expiring<AuthCacheKey, Arc<AuthenticatedUserOutcome>>,
    team_views: Expiring<TeamViewCacheKey, TeamViewGrant>,
}

impl Default for CacheState {
    fn default() -> Self {
        Self {
            users: Expiring::new(
                AUTHENTICATED_USER_CACHE_LIFETIME,
                MAXIMUM_CACHED_AUTHENTICATED_USERS,
            ),
            team_views: Expiring::new(TEAM_VIEW_TOKEN_CACHE_LIFETIME, MAXIMUM_CACHED_TEAM_VIEWS),
        }
    }
}

fn cache_state() -> &'static Mutex<CacheState> {
    static CACHE: OnceLock<Mutex<CacheState>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(CacheState::default()))
}

/// The agent's implementation of both read caches. It holds no state itself;
/// the entries live in one process-wide store so operations that run on
/// different worker threads share them.
struct AgentReadCaches;

fn caches() -> ReadCaches {
    static CACHES: OnceLock<Arc<AgentReadCaches>> = OnceLock::new();
    let caches = CACHES.get_or_init(|| Arc::new(AgentReadCaches));
    ReadCaches {
        authenticated_users: Some(caches.clone()),
        team_view_tokens: Some(caches.clone()),
    }
}

impl AuthenticatedUserCache for AgentReadCaches {
    fn get(&self, key: &AuthCacheKey) -> Option<Arc<AuthenticatedUserOutcome>> {
        let outcome = cache_state()
            .lock()
            .ok()?
            .users
            .get_at(key, Instant::now())?;
        note_cache_hit();
        Some(outcome)
    }

    fn put(&self, key: AuthCacheKey, outcome: Arc<AuthenticatedUserOutcome>) {
        if let Ok(mut state) = cache_state().lock() {
            state.users.put_at(key, outcome, Instant::now());
        }
    }

    fn invalidate_profile(&self, state_root: &Path, profile: &str) {
        if let Ok(mut state) = cache_state().lock() {
            state
                .users
                .retain(|key| key.state_root != state_root || key.profile != profile);
        }
    }
}

impl TeamViewTokenCache for AgentReadCaches {
    fn get(&self, key: &TeamViewCacheKey) -> Option<TeamViewGrant> {
        cache_state()
            .lock()
            .ok()?
            .team_views
            .get_at(key, Instant::now())
    }

    fn put(&self, key: TeamViewCacheKey, grant: TeamViewGrant) {
        if let Ok(mut state) = cache_state().lock() {
            state.team_views.put_at(key, grant, Instant::now());
        }
    }

    fn invalidate(&self, key: &TeamViewCacheKey) {
        if let Ok(mut state) = cache_state().lock() {
            state.team_views.retain(|stored| stored != key);
        }
    }

    fn invalidate_profile(&self, state_root: &Path, profile: &str) {
        if let Ok(mut state) = cache_state().lock() {
            state
                .team_views
                .retain(|key| key.state_root != state_root || key.profile != profile);
        }
    }
}

/// Drops every retained outcome and view for one profile.
pub(crate) fn invalidate_profile(state_root: &Path, profile: &str) {
    AuthenticatedUserCache::invalidate_profile(&AgentReadCaches, state_root, profile);
    TeamViewTokenCache::invalidate_profile(&AgentReadCaches, state_root, profile);
}

/// Drops every retained outcome and view. This is the epilogue of every
/// operation that is not a read: an operation's effective profile set is not
/// always knowable here (a federation cascade reaches profiles it selects
/// while running), and dropping a still-valid entry only costs one
/// authentication on the next read.
pub(crate) fn invalidate_all() {
    let Ok(mut state) = cache_state().lock() else {
        return;
    };
    state.users.clear();
    state.team_views.clear();
}

/// Per-operation state. `dispatch_result` installs one for the duration of a
/// request; work that runs outside a request (the chat poll, the scheduler,
/// connectivity probes) finds none and is never served from a cache.
struct OperationScope {
    reads: bool,
    served_from_cache: bool,
    profiles: Vec<(PathBuf, String)>,
}

thread_local! {
    static SCOPE: RefCell<Option<OperationScope>> = const { RefCell::new(None) };
}

/// Cache usage and profile access recorded for one operation.
pub(crate) struct OperationTrace {
    pub(crate) served_from_cache: bool,
    pub(crate) profiles: Vec<(PathBuf, String)>,
}

/// Runs an operation with cache tracking enabled and returns its trace.
pub(crate) fn with_operation_scope<T>(
    reads: bool,
    work: impl FnOnce() -> T,
) -> (T, OperationTrace) {
    struct Restore(Option<OperationScope>);
    impl Drop for Restore {
        fn drop(&mut self) {
            SCOPE.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(SCOPE.with(|slot| {
        slot.replace(Some(OperationScope {
            reads,
            served_from_cache: false,
            profiles: Vec::new(),
        }))
    }));
    let value = work();
    let trace = SCOPE.with(|slot| {
        let scope = slot.borrow();
        let scope = scope.as_ref();
        OperationTrace {
            served_from_cache: scope.is_some_and(|scope| scope.served_from_cache),
            profiles: scope
                .map(|scope| scope.profiles.clone())
                .unwrap_or_default(),
        }
    });
    (value, trace)
}

fn note_cache_hit() {
    SCOPE.with(|slot| {
        if let Some(scope) = slot.borrow_mut().as_mut() {
            scope.served_from_cache = true;
        }
    });
}

fn note_profile(state_root: &Path, profile: &str) {
    SCOPE.with(|slot| {
        if let Some(scope) = slot.borrow_mut().as_mut() {
            let entry = (state_root.to_path_buf(), profile.to_owned());
            if !scope.profiles.contains(&entry) {
                scope.profiles.push(entry);
            }
        }
    });
}

fn scope_serves_reads() -> bool {
    SCOPE.with(|slot| slot.borrow().as_ref().is_some_and(|scope| scope.reads))
}

struct BaseClient {
    state_root: PathBuf,
    profile: String,
    fingerprint: [u8; 32],
    client: foks_client::FoksClient,
}

fn base_clients() -> &'static Mutex<VecDeque<BaseClient>> {
    static CLIENTS: OnceLock<Mutex<VecDeque<BaseClient>>> = OnceLock::new();
    CLIENTS.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// Drops the retained transports. A profile whose trust changes is already
/// caught by the fingerprint check in [`base_client`]; this is for the
/// operations that add, remove or rewrite profiles outright.
pub(crate) fn invalidate_base_clients() {
    let Ok(mut clients) = base_clients().lock() else {
        return;
    };
    clients.clear();
}

/// The transport every operation on one profile shares.
///
/// A pooled connection carries no authentication state of its own: the pool is
/// keyed by endpoint, host identity, trust and a digest of the presented mTLS
/// material, and `checkout_connection` rebuilds and validates that material on
/// every call. Reads and writes may therefore share one client. Deadlines and
/// cancellation stay per operation because they live on the clone, not on the
/// shared pool.
fn base_client(
    registry: &ProfileRegistry,
    name: &str,
) -> foks_client_app::Result<foks_client::FoksClient> {
    let fingerprint = foks_client_app::profile_transport_fingerprint(registry, name)?;
    let root = registry.root();
    if let Ok(clients) = base_clients().lock() {
        if let Some(entry) = clients.iter().find(|entry| {
            entry.state_root == root && entry.profile == name && entry.fingerprint == fingerprint
        }) {
            return Ok(entry.client.clone());
        }
    }
    let (client, fingerprint) = foks_client_app::base_client_for_profile(registry, name)?;
    if let Ok(mut clients) = base_clients().lock() {
        clients.retain(|entry| entry.state_root != root || entry.profile != name);
        while clients.len() >= MAXIMUM_BASE_CLIENTS {
            clients.pop_front();
        }
        clients.push_back(BaseClient {
            state_root: root.to_path_buf(),
            profile: name.to_owned(),
            fingerprint,
            client: client.clone(),
        });
    }
    Ok(client)
}

/// Opens a profile for one agent operation over the shared transport, with the
/// read caches attached only when the operation is a read.
pub(crate) fn open_profile_session(
    registry: &ProfileRegistry,
    name: &str,
    timeout: Duration,
    cancellation: CancellationToken,
) -> foks_client_app::Result<ProfileSession> {
    note_profile(registry.root(), name);
    let session = match base_client(registry, name) {
        Ok(base) => ProfileSession::open_with_control_and_client(
            registry,
            name,
            timeout,
            cancellation,
            &base,
        )?,
        // Building the base transport reads exactly what opening a session
        // reads, so the fallback surfaces the same failure rather than hiding
        // it.
        Err(_) => ProfileSession::open_with_control(registry, name, timeout, cancellation)?,
    };
    Ok(if scope_serves_reads() {
        session.with_read_caches(caches())
    } else {
        session
    })
}

/// Whether an operation may be served from the read caches.
///
/// This list is deliberately explicit rather than derived from an existing
/// classification: a new operation is not a read here until it is added, and
/// anything that posts a chain link, rotates a key, writes settings, enrolls,
/// revokes or sends a message must never appear in it.
pub(crate) fn operation_serves_reads(operation: &Operation) -> bool {
    use foks_agent_proto::chat::ChatAction;
    use foks_agent_proto::invitations::InvitationAction;
    match operation {
        Operation::ListKv { .. }
        | Operation::ListTeamKv { .. }
        | Operation::ReadKv { .. }
        | Operation::ReadKvChunk { .. }
        | Operation::ReadData { .. }
        | Operation::DiscoverTeams { .. }
        | Operation::ListTeamDetails { .. }
        | Operation::ListTeamMembers { .. }
        | Operation::ListFederatedTeams { .. } => true,
        // PollInbox is excluded: it holds a long poll on its own connection,
        // where an outcome retained for 30 seconds buys nothing.
        Operation::Chat { action, .. } => matches!(
            action,
            ChatAction::Channels
                | ChatAction::History { .. }
                | ChatAction::NotificationHistory { .. }
                | ChatAction::Inbox
                | ChatAction::SyncInbox { .. }
                | ChatAction::OperationBody { .. }
        ),
        Operation::Invitations { action, .. } => matches!(
            action,
            InvitationAction::Inbox { .. }
                | InvitationAction::InboxCount { .. }
                | InvitationAction::PendingApprovals { .. }
                | InvitationAction::Preview { .. }
                | InvitationAction::List
        ),
        _ => false,
    }
}

/// Whether an operation that is not served from the caches may nonetheless
/// leave retained material in place.
///
/// These operations read server or local state but cannot change this user's
/// devices, keys or team membership, so an outcome or view token retained
/// before one of them is still exactly as good afterwards. The desktop issues
/// several of them on a fixed tick for every profile, and treating them as
/// mutations would wipe the caches mid-pass and defeat the 30-second lifetime.
///
/// This list is explicit for the same reason as the read list: a new operation
/// is invalidating until it is added here, and anything that can post a chain
/// link, rotate a key, write settings, enroll, revoke, send a message or renew
/// server-side state beyond a compatibility lease must never appear in it.
pub(crate) fn operation_leaves_retained_material(operation: &Operation) -> bool {
    use foks_agent_proto::chat::ChatAction;
    match operation {
        // Local or server reads of devices, enrollments, accounts, teams,
        // stores and pending work. Each one opens a session and reads; none
        // writes durable credential state.
        Operation::ListProfiles
        | Operation::ListKnownStores { .. }
        | Operation::ListProfileOverview { .. }
        | Operation::ListAccounts { .. }
        | Operation::ListPendingOperations { .. }
        | Operation::ListAccountRenames { .. }
        | Operation::ListDevices { .. }
        | Operation::ListBackupEnrollments { .. }
        | Operation::ListTeams { .. }
        | Operation::ListYubiCards { .. }
        | Operation::ListYubiAccounts { .. }
        | Operation::YubiPinStatus { .. }
        | Operation::PendingDataWrites { .. }
        | Operation::DataWriteStatus { .. }
        | Operation::DescribeServerStatus { .. }
        | Operation::DescribeResetHardState { .. } => true,
        // Reachability and host reconciliation. Both may pin a host on first
        // contact, which is trust material the base-client pool key already
        // covers, and neither touches a credential, a PUK generation or a
        // view. RefreshLease renews only the compatibility lease.
        Operation::Probe { .. }
        | Operation::ReconcileProfile { .. }
        | Operation::RefreshLease { .. } => true,
        // Status reads a local submission record; every other chat action is
        // either a read that is already served from the caches, a long poll,
        // or a mutation.
        Operation::Chat { action, .. } => matches!(action, ChatAction::Status { .. }),
        _ => false,
    }
}

/// Whether a failed read earns its one retry: it must have been served
/// retained material, and the failure must be a refusal of that material. A
/// read that authenticated for itself gets the answer it was given.
pub(crate) fn read_earns_a_retry(
    trace: &OperationTrace,
    error: &(dyn std::error::Error + 'static),
) -> bool {
    trace.served_from_cache && error_rejects_retained_material(error)
}

/// Whether a failure means retained material was refused, rather than the read
/// itself being impossible. Every case here is either a local binding check
/// against the verified state or a server refusal of the credential, the PUK
/// generation or the view token, which is exactly what a superseded outcome
/// produces.
pub(crate) fn error_rejects_retained_material(error: &(dyn std::error::Error + 'static)) -> bool {
    if let Some(error) = error.downcast_ref::<foks_client_app::Error>() {
        return match error {
            foks_client_app::Error::Client(error) => client_error_rejects_material(error),
            _ => false,
        };
    }
    error
        .downcast_ref::<foks_client::Error>()
        .is_some_and(client_error_rejects_material)
}

/// The server refusals a superseded credential, PUK generation or view token
/// produces. Every other status is the read's own answer.
fn status_rejects_material(code: u64) -> bool {
    use foks_rpc::{
        STATUS_EXPIRED_ERROR, STATUS_KEY_NOT_FOUND_ERROR, STATUS_PERMISSION_ERROR,
        STATUS_TEAM_BEARER_TOKEN_STALE_ERROR, STATUS_TEAM_KEY_ERROR, STATUS_TEAM_NO_SRC_ROLE_ERROR,
        STATUS_WRONG_USER_ERROR,
    };
    matches!(
        code,
        STATUS_PERMISSION_ERROR
            | STATUS_KEY_NOT_FOUND_ERROR
            | STATUS_EXPIRED_ERROR
            | STATUS_WRONG_USER_ERROR
            | STATUS_TEAM_BEARER_TOKEN_STALE_ERROR
            | STATUS_TEAM_KEY_ERROR
            | STATUS_TEAM_NO_SRC_ROLE_ERROR
    )
}

fn client_error_rejects_material(error: &foks_client::Error) -> bool {
    match error {
        foks_client::Error::KeyBinding(_)
        | foks_client::Error::CredentialBinding(_)
        | foks_client::Error::UserBinding(_)
        | foks_client::Error::TeamBinding(_) => true,
        foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus { code, .. }) => {
            status_rejects_material(*code)
        }
        // A chat read reports its reuse failures through this wrapper.
        foks_client::Error::ReadReuse(inner) => inner
            .downcast_ref::<foks_client_app::Error>()
            .is_some_and(|error| match error {
                foks_client_app::Error::Client(error) => client_error_rejects_material(error),
                _ => false,
            }),
        _ => false,
    }
}

#[cfg(test)]
mod tests;
