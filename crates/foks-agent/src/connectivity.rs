use super::*;
use std::future::Future;
use tokio::sync::watch;

type RegistryRevision = (u64, u64, i64, i64);
type FlightKey = (PathBuf, String, bool, Option<RegistryRevision>);

fn flight_key(root: &Path, profile: &str, identity: bool) -> FlightKey {
    #[cfg(unix)]
    let revision = {
        use std::os::unix::fs::MetadataExt as _;
        std::fs::symlink_metadata(root.join("profiles.toml"))
            .ok()
            .map(|metadata| {
                (
                    metadata.dev(),
                    metadata.ino(),
                    metadata.ctime(),
                    metadata.ctime_nsec(),
                )
            })
    };
    #[cfg(not(unix))]
    let revision = None;
    (root.to_owned(), profile.to_owned(), identity, revision)
}
/// The named steps of one connectivity observation, in the order they ran,
/// in milliseconds. Carried back with the observation so a readout states
/// where a reconcile spent its time rather than only how long it took.
type Phases = Vec<(String, u32)>;

/// Measures the consecutive steps of one observation. A step ends where the
/// previous one ended, so the steps partition the observation's time; a step
/// measured inside a worker is added with the duration that worker reported.
struct Steps {
    marker: Instant,
    phases: Phases,
}

impl Steps {
    fn new() -> Self {
        Self {
            marker: Instant::now(),
            phases: Phases::new(),
        }
    }

    /// Ends the step that began where the previous step ended.
    fn step(&mut self, name: &str) {
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(self.marker);
        self.marker = now;
        self.add(name, elapsed);
    }

    /// Records a step measured elsewhere, leaving the current one running.
    fn add(&mut self, name: &str, elapsed: Duration) {
        self.phases
            .push((name.to_owned(), ResponseTiming::millis(elapsed)));
    }

    fn into_phases(self) -> Phases {
        self.phases
    }
}

type FlightResult = watch::Receiver<Option<(ResponseResult, Phases)>>;
static FLIGHTS: OnceLock<Mutex<BTreeMap<FlightKey, FlightResult>>> = OnceLock::new();
static FETCH_CAPACITY: OnceLock<Arc<Semaphore>> = OnceLock::new();
const MAX_FLIGHTS: usize = 64;

struct FlightGuard(FlightKey);

impl Drop for FlightGuard {
    fn drop(&mut self) {
        if let Some(flights) = FLIGHTS.get() {
            flights
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&self.0);
        }
    }
}

fn failure(profile: &str, code: ErrorCode, message: &str) -> ResponseResult {
    Response::error_with_fields(
        Some(0),
        code,
        message,
        ErrorFields {
            profile: Some(profile.to_owned()),
            ..ErrorFields::default()
        },
    )
    .result
}

fn success(status: &str) -> ResponseResult {
    Response::success(0, serde_json::json!({"status": status})).result
}

fn read_registry_snapshot<T>(
    started: Instant,
    timeout: Duration,
    mut read: impl FnMut() -> Result<T, foks_client_app::Error>,
) -> Result<T, foks_client_app::Error> {
    loop {
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err(foks_client_app::Error::Io(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "registry snapshot acquisition deadline exceeded",
            )));
        }
        match read() {
            Err(foks_client_app::Error::Io(error))
                if error.kind() == std::io::ErrorKind::WouldBlock =>
            {
                std::thread::sleep(
                    timeout
                        .saturating_sub(started.elapsed())
                        .min(Duration::from_millis(20)),
                );
            }
            result => return result,
        }
    }
}

fn scoped_error(profile: &str, error: &(dyn std::error::Error + 'static)) -> ResponseResult {
    let code = match error.downcast_ref::<foks_client_app::Error>() {
        Some(foks_client_app::Error::SavedTrustMissing) => Some(ErrorCode::SavedTrustMissing),
        Some(
            foks_client_app::Error::ProfileMissing
            | foks_client_app::Error::ProfileRegistryChanged
            | foks_client_app::Error::StatePathChanged,
        ) => Some(ErrorCode::ProfileConfigurationChanged),
        _ => None,
    };
    let mut result = if let Some(code) = code {
        failure(profile, code, &error.to_string())
    } else {
        dispatch_error_response(0, error).result
    };
    if let ResponseResult::Error { fields, .. } = &mut result {
        fields.profile = Some(profile.to_owned());
    }
    result
}

/// Runs one observation per flight key and answers every caller waiting on
/// it with that observation and the steps it took. A caller that joined a
/// flight is answered with the leading observation's steps: they are the
/// steps of the observation it is given.
async fn coalesce<F, Fut>(key: FlightKey, timeout: Duration, work: F) -> (ResponseResult, Phases)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = (ResponseResult, Phases)> + Send + 'static,
{
    let mut receiver = {
        let mut flights = FLIGHTS
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(receiver) = flights.get(&key) {
            receiver.clone()
        } else {
            if flights.len() >= MAX_FLIGHTS {
                return (
                    failure(&key.1, ErrorCode::Busy, "connectivity admission is full"),
                    Phases::new(),
                );
            }
            let (sender, receiver) = watch::channel(None);
            flights.insert(key.clone(), receiver.clone());
            let key = key.clone();
            tokio::spawn(async move {
                let _flight = FlightGuard(key);
                let result = work().await;
                sender.send_replace(Some(result));
            });
            receiver
        }
    };
    let profile = key.1;
    let wait = async {
        loop {
            if let Some(result) = receiver.borrow_and_update().clone() {
                return result;
            }
            if receiver.changed().await.is_err() {
                return (
                    failure(
                        &profile,
                        ErrorCode::OperationFailed,
                        "connectivity worker stopped",
                    ),
                    Phases::new(),
                );
            }
        }
    };
    tokio::time::timeout(timeout, wait)
        .await
        .unwrap_or_else(|_| {
            (
                failure(
                    &profile,
                    ErrorCode::DeadlineExceeded,
                    "connectivity observation exceeded its deadline",
                ),
                Phases::new(),
            )
        })
}

pub(super) async fn renew(
    state_dir: &Path,
    profile: &str,
    client: reqwest::Client,
    timeout: Duration,
) -> ResponseResult {
    renew_observed(state_dir, profile, client, timeout).await.0
}

/// The compatibility renewal with the steps it took: reading the profile's
/// lease snapshot, fetching the signed canary, and applying it.
async fn renew_observed(
    state_dir: &Path,
    profile: &str,
    client: reqwest::Client,
    timeout: Duration,
) -> (ResponseResult, Phases) {
    let root = state_dir.to_owned();
    let profile = profile.to_owned();
    coalesce(
        flight_key(&root, &profile, false),
        timeout,
        move || async move {
            let mut steps = Steps::new();
            let result = renew_one(&root, &profile, &client, timeout, &mut steps).await;
            (result, steps.into_phases())
        },
    )
    .await
}

async fn renew_one(
    root: &Path,
    profile: &str,
    client: &reqwest::Client,
    timeout: Duration,
    steps: &mut Steps,
) -> ResponseResult {
    let started = Instant::now();
    let capacity = FETCH_CAPACITY.get_or_init(|| Arc::new(Semaphore::new(MAXIMUM_CANARY_FETCHES)));
    let Ok(Ok(worker)) = tokio::time::timeout(timeout, capacity.clone().acquire_owned()).await
    else {
        steps.step("lease");
        return failure(
            profile,
            ErrorCode::Busy,
            "compatibility workers unavailable",
        );
    };
    let admission = match profile_work::coordinator()
        .acquire(
            root,
            profile_work::Scope::RegistryRead,
            timeout.saturating_sub(started.elapsed()),
        )
        .await
    {
        Ok(permit) => permit,
        Err(error) => {
            steps.step("lease");
            return scoped_error(profile, &error);
        }
    };
    let snapshot_root = root.to_owned();
    let snapshot_profile = profile.to_owned();
    let snapshot = tokio::task::spawn_blocking(move || {
        let _admission = admission;
        let snapshot = read_registry_snapshot(started, timeout, || {
            ProfileRegistry::try_open(&snapshot_root)
                .and_then(|registry| registry.hosted_lease_renewal(&snapshot_profile))
        });
        (snapshot, worker)
    })
    .await;
    // Waiting for a fetch worker and for registry admission, then reading
    // the profile's renewal snapshot. The half states this step however it
    // ends, so what it waited on is never left out of the observation.
    steps.step("lease");
    let (snapshot, worker) = match snapshot {
        Ok((Ok(Some(snapshot)), worker)) => (snapshot, worker),
        Ok((Ok(None), _)) => return success("not-required"),
        Ok((Err(error), _)) => return scoped_error(profile, &error),
        Err(_) => {
            return failure(
                profile,
                ErrorCode::OperationFailed,
                "compatibility snapshot worker stopped",
            )
        }
    };
    let fetched = tokio::time::timeout(
        timeout.saturating_sub(started.elapsed()),
        fetch_canary(client, snapshot.url()),
    )
    .await;
    steps.step("fetch");
    let bytes = match fetched {
        Ok(Ok(bytes)) => bytes,
        Err(_)
        | Ok(Err(LeaseFetchError::Transport | LeaseFetchError::Status(408 | 429 | 500..=599))) => {
            return failure(
                profile,
                ErrorCode::ServerUnavailable,
                "compatibility service is unavailable",
            );
        }
        Ok(Err(error)) => {
            return failure(
                profile,
                ErrorCode::CompatibilityRejected,
                &error.to_string(),
            )
        }
    };
    let signed = match serde_json::from_slice::<foks_compat_artifact::SignedCanaryArtifact>(&bytes)
    {
        Ok(signed) => signed,
        Err(_) => {
            return failure(
                profile,
                ErrorCode::CompatibilityRejected,
                "compatibility artifact is invalid",
            )
        }
    };
    let now = match now_microseconds() {
        Ok(now) => now / 1_000_000,
        Err(error) => return scoped_error(profile, error.as_ref()),
    };
    if snapshot.matches_stored_validated_artifact(&signed, now) {
        // Applying the artifact the profile already stores writes nothing and
        // reports "unchanged". Taking the profile-scoped admission to reach
        // that conclusion would block the user's own work on this profile for
        // no state change, so the steady state answers without it.
        drop(worker);
        steps.step("apply");
        return success("unchanged");
    }
    let admission = match profile_work::coordinator()
        .acquire(
            root,
            profile_work::Scope::profile(profile),
            timeout.saturating_sub(started.elapsed()),
        )
        .await
    {
        Ok(permit) => permit,
        Err(error) => return scoped_error(profile, &error),
    };
    let applied = tokio::task::spawn_blocking(move || {
        let _admission = admission;
        let _worker = worker;
        if started.elapsed() >= timeout {
            return Err(foks_client_app::Error::Client(
                foks_client::Error::DeadlineExceeded,
            ));
        }
        snapshot.apply(&signed, now)
    })
    .await;
    // Parsing the signed artifact, waiting for the profile's admission and
    // applying the artifact to the profile's local state.
    steps.step("apply");
    let applied = match applied {
        Ok(result) => result,
        Err(_) => {
            return failure(
                profile,
                ErrorCode::OperationFailed,
                "compatibility application worker stopped",
            )
        }
    };
    match applied {
        Ok((_, foks_client_app::CompatibilityStatus::Incompatible { reason, .. })) => {
            let mut result = failure(
                profile,
                ErrorCode::CompatibilityRejected,
                "signed compatibility facts do not permit this client",
            );
            if let ResponseResult::Error { fields, .. } = &mut result {
                fields.reason = serde_json::to_value(reason)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned));
            }
            result
        }
        Ok((updated, _)) => success(if updated { "renewed" } else { "unchanged" }),
        Err(error @ foks_client_app::Error::InvalidProfile(_)) => failure(
            profile,
            ErrorCode::CompatibilityRejected,
            &bounded_error(error.to_string()),
        ),
        Err(error) => scoped_error(profile, &error),
    }
}

fn identity_error(profile: &str, error: &(dyn std::error::Error + 'static)) -> ResponseResult {
    if let Some(foks_client_app::Error::Client(error)) =
        error.downcast_ref::<foks_client_app::Error>()
    {
        let code = match error {
            foks_client::Error::NoAddress(_)
            | foks_client::Error::Connect(_)
            | foks_client::Error::DeadlineExceeded => ErrorCode::ServerUnavailable,
            foks_client::Error::Rpc(foks_rpc::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::ConnectionRefused
                        | std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::ConnectionAborted
                        | std::io::ErrorKind::NotConnected
                        | std::io::ErrorKind::BrokenPipe
                        | std::io::ErrorKind::UnexpectedEof
                ) =>
            {
                ErrorCode::ServerUnavailable
            }
            _ => ErrorCode::ServerIdentityRejected,
        };
        return failure(profile, code, &bounded_error(error.to_string()));
    }
    scoped_error(profile, error)
}

/// How long the identity check may spend on the network. It runs under the
/// profile's admission, and `reconcile_saved_host` updates authenticated
/// hard state under that same admission, so the check cannot release the
/// profile while it waits on the server. Every other request for the
/// profile waits behind it, and a server that accepts a connection and
/// never answers would otherwise hold them for the whole request timeout.
const IDENTITY_NETWORK_BUDGET: Duration = Duration::from_secs(20);

/// The network deadline for one identity check: the request's remaining
/// budget, bounded by what a periodic background check may hold the profile
/// for.
fn identity_network_budget(remaining: Duration) -> Duration {
    remaining.min(IDENTITY_NETWORK_BUDGET)
}

/// The identity check with the steps it took: waiting for the profile's
/// admission and a worker, opening the profile's session, and the round trip
/// that reconciles the saved host.
async fn identity(
    root: &Path,
    profile: &str,
    timeout: Duration,
    network_budget: Duration,
    workers: Arc<Semaphore>,
) -> (ResponseResult, Phases) {
    let root = root.to_owned();
    let profile = profile.to_owned();
    coalesce(
        flight_key(&root, &profile, true),
        timeout,
        move || async move {
            let mut steps = Steps::new();
            let started = Instant::now();
            let admission = match profile_work::coordinator()
                .acquire(&root, profile_work::Scope::profile(&profile), timeout)
                .await
            {
                Ok(permit) => permit,
                Err(error) => {
                    steps.step("admit");
                    return (scoped_error(&profile, &error), steps.into_phases());
                }
            };
            let remaining = timeout.saturating_sub(started.elapsed());
            let worker = match tokio::time::timeout(remaining, workers.acquire_owned()).await {
                Ok(Ok(worker)) => worker,
                _ => {
                    steps.step("admit");
                    return (
                        failure(
                            &profile,
                            ErrorCode::Busy,
                            "connectivity worker capacity timed out",
                        ),
                        steps.into_phases(),
                    );
                }
            };
            steps.step("admit");
            let fallback = profile.clone();
            let observed = tokio::task::spawn_blocking(move || {
                let _admission = admission;
                let _worker = worker;
                let remaining = timeout.saturating_sub(started.elapsed());
                let cancellation = CancellationToken::new();
                let body = Instant::now();
                let (result, phases) = profile_work::with_phase_timing(|| {
                    foks_keystore::without_user_interaction(|| {
                        profile_work::with_control(remaining, cancellation.clone(), || {
                            let registry = read_registry_snapshot(started, timeout, || {
                                ProfileRegistry::try_open(&root)
                            })?;
                            // The shared base client's pool validates an idle
                            // connection at checkout by peeking the socket, so a
                            // peer that has gone away is discarded rather than
                            // reported here as a reachable server.
                            let session = crate::read_cache::open_profile_session(
                                &registry,
                                &profile,
                                timeout.saturating_sub(started.elapsed()),
                                cancellation,
                            )?;
                            let credentials = ClientCredentials::open(&root)?;
                            checked_session(&credentials, &session, |session| {
                                session
                                    .reconcile_saved_host_with_timeout(
                                        timeout
                                            .saturating_sub(started.elapsed())
                                            .min(network_budget),
                                    )
                                    .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)
                            })
                        })
                    })
                });
                // Opening the session, and any wait for the profile's file
                // lock against another process, are what the request's own
                // phase accumulator counted; the rest of the worker is the
                // round trip to the server.
                let elapsed = body.elapsed();
                let result = match result {
                    Ok((host_id, configured_probe)) => {
                        Response::success(
                            0,
                            serde_json::json!({
                                "status": "connected",
                                "host_id": host_id,
                                "configured_probe": configured_probe,
                            }),
                        )
                        .result
                    }
                    Err(error) => identity_error(&profile, error.as_ref()),
                };
                (
                    result,
                    phases.session,
                    phases.lock,
                    elapsed
                        .saturating_sub(phases.session)
                        .saturating_sub(phases.lock),
                )
            })
            .await;
            match observed {
                Ok((result, opened, locked, host)) => {
                    steps.add("open", opened);
                    // Only a contended profile waits for the file lock, and
                    // a step that did not happen is not worth a name.
                    if !locked.is_zero() {
                        steps.add("lock", locked);
                    }
                    steps.add("host", host);
                    (result, steps.into_phases())
                }
                Err(_) => (
                    failure(
                        &fallback,
                        ErrorCode::OperationFailed,
                        "identity worker stopped",
                    ),
                    steps.into_phases(),
                ),
            }
        },
    )
    .await
}

/// Observes one profile's server: its identity and its compatibility lease,
/// concurrently. The response carries the steps both halves took, so a
/// readout states where an observation that took seconds spent them. The
/// halves run at the same time, so their steps do not add up to the
/// observation's own duration.
pub(super) async fn reconcile(
    root: &Path,
    profile: &str,
    timeout: Duration,
    workers: Arc<Semaphore>,
) -> (serde_json::Value, ResponseTiming) {
    let started = Instant::now();
    let compatibility = async {
        match compatibility_http_client(timeout) {
            Ok(client) => renew_observed(root, profile, client, timeout).await,
            Err(_) => (
                failure(
                    profile,
                    ErrorCode::OperationFailed,
                    "compatibility transport could not be initialized",
                ),
                Phases::new(),
            ),
        }
    };
    let ((identity, identity_phases), (compatibility, compatibility_phases)) = tokio::join!(
        identity(
            root,
            profile,
            timeout,
            identity_network_budget(timeout),
            workers
        ),
        compatibility
    );
    let mut phases = identity_phases;
    phases.extend(compatibility_phases);
    let timing = ResponseTiming {
        body_ms: ResponseTiming::millis(started.elapsed()),
        phases,
        ..ResponseTiming::default()
    };
    (
        serde_json::json!({
            "profile": profile,
            "identity": identity,
            "compatibility": compatibility,
        }),
        timing,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_network_budget_is_bounded_below_the_request_timeout() {
        assert_eq!(
            identity_network_budget(Duration::from_secs(60)),
            IDENTITY_NETWORK_BUDGET
        );
        assert_eq!(
            identity_network_budget(Duration::from_secs(5)),
            Duration::from_secs(5)
        );
        assert_eq!(identity_network_budget(Duration::ZERO), Duration::ZERO);
        assert!(IDENTITY_NETWORK_BUDGET < Duration::from_secs(60));
    }

    fn saved_identity_profile() -> (
        foks_server_testkit::TestEnvironment,
        foks_server_testkit::InProcessServer,
        PathBuf,
    ) {
        let environment = foks_server_testkit::TestEnvironment::new().unwrap();
        let server = environment.start_server().unwrap();
        let root = environment.client_path("identity", "state").unwrap();
        let certificate = environment
            .client_path("identity", "probe-root.der")
            .unwrap();
        environment.write_probe_root(&certificate).unwrap();
        let credentials =
            ClientCredentials::initialize(&root, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&root).unwrap();
        registry
            .add(Profile {
                name: "saved".into(),
                label: None,
                probe: format!(
                    "localhost:{}",
                    environment.addresses().unwrap().probe.port()
                ),
                protocol: ProtocolPolicy::V019,
                trust: TrustRoot::CertificateDer { path: certificate },
            })
            .unwrap();
        let session = ProfileSession::open(&registry, "saved").unwrap();
        credentials
            .with_checked_session(&session, |checked| checked.probe_and_pin())
            .unwrap();
        (environment, server, root)
    }

    #[tokio::test]
    async fn identity_network_timeout_releases_profile_admission_and_worker() {
        let (environment, _server, root) = saved_identity_profile();
        let listener = environment.reserve_loopback_listener().unwrap();
        let mut registry = ProfileRegistry::open(&root).unwrap();
        let mut profile = registry.profile("saved").unwrap().clone();
        profile.probe = format!("localhost:{}", listener.local_addr().unwrap().port());
        registry.replace(profile).unwrap();
        drop(registry);
        listener.set_nonblocking(true).unwrap();
        let listener = tokio::net::TcpListener::from_std(listener).unwrap();
        let workers = Arc::new(Semaphore::new(1));
        let request = identity(
            &root,
            "saved",
            Duration::from_secs(5),
            Duration::from_millis(500),
            workers.clone(),
        );
        tokio::pin!(request);
        let _connection = tokio::select! {
            accepted = listener.accept() => accepted.unwrap().0,
            result = &mut request => panic!("identity never reached the stalled server: {result:?}"),
        };
        let (result, phases) = tokio::time::timeout(Duration::from_secs(2), request)
            .await
            .expect("identity exceeded its network budget");
        assert!(matches!(
            result,
            ResponseResult::Error {
                code: ErrorCode::ServerUnavailable,
                ..
            }
        ));
        // The steps say where the check stopped: waiting on the server, not
        // on admission or on opening the profile.
        assert_eq!(
            phases
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["admit", "open", "host"]
        );
        let step = |name: &str| {
            phases
                .iter()
                .find(|(step, _)| step == name)
                .map(|(_, ms)| *ms)
                .unwrap()
        };
        assert!(step("host") >= 400, "identity phases: {phases:?}");
        assert!(step("admit") < 400, "identity phases: {phases:?}");
        assert_eq!(workers.available_permits(), 1);
        let permit = profile_work::coordinator()
            .acquire(
                &root,
                profile_work::Scope::profile("saved"),
                Duration::from_secs(1),
            )
            .await
            .expect("identity retained profile admission after its network timeout");
        drop(permit);
    }

    #[tokio::test]
    async fn identity_lock_acquisition_outlasts_the_network_budget() {
        let (_environment, _server, root) = saved_identity_profile();
        let lock =
            std::fs::File::open(root.join("profiles/saved/.profile-operation.lock")).unwrap();
        fs2::FileExt::lock_exclusive(&lock).unwrap();
        let workers = Arc::new(Semaphore::new(1));
        let request = identity(
            &root,
            "saved",
            Duration::from_secs(5),
            Duration::from_secs(1),
            workers.clone(),
        );
        tokio::pin!(request);
        let waiting = tokio::time::timeout(Duration::from_secs(2), &mut request).await;
        let available_workers = workers.available_permits();
        fs2::FileExt::unlock(&lock).unwrap();
        assert!(
            waiting.is_err(),
            "identity stopped while waiting for the local lock: {waiting:?}"
        );
        assert_eq!(available_workers, 0);
        let (result, phases) = tokio::time::timeout(Duration::from_secs(2), request)
            .await
            .expect("identity did not resume after the local lock was released");
        let ResponseResult::Success { value } = result else {
            panic!("identity failed after waiting for the local lock: {result:?}");
        };
        assert_eq!(value["status"], "connected");
        // A wait on another process's lock is its own step, so a readout
        // does not read it as time spent talking to the server.
        let locked = phases
            .iter()
            .find(|(name, _)| name == "lock")
            .map(|(_, ms)| *ms)
            .unwrap_or_default();
        assert!(locked >= 500, "identity phases: {phases:?}");
        assert_eq!(workers.available_permits(), 1);
    }

    #[tokio::test]
    async fn a_reconcile_states_the_steps_of_both_halves() {
        let (_environment, _server, root) = saved_identity_profile();
        let (value, timing) = reconcile(
            &root,
            "saved",
            Duration::from_secs(10),
            Arc::new(Semaphore::new(1)),
        )
        .await;
        assert_eq!(value["identity"]["value"]["status"], "connected");
        let names = timing
            .phases
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>();
        // The identity check's steps first, then the compatibility half's.
        // This profile carries no hosted lease, so that half ends at the
        // step that read its renewal snapshot, whatever that read answered.
        assert_eq!(names.first().copied(), Some("admit"));
        assert_eq!(names.last().copied(), Some("lease"));
        for step in ["open", "host"] {
            assert!(names.contains(&step), "reconcile phases: {names:?}");
        }
        let total: u32 = timing.phases.iter().map(|(_, ms)| *ms).sum();
        // The two halves run at the same time, so the steps together cover
        // at least what the observation took.
        assert!(
            timing.body_ms <= total + 500,
            "reconcile phases {:?} do not account for {}ms",
            timing.phases,
            timing.body_ms
        );
    }

    #[test]
    fn registry_snapshot_waits_for_the_registry_lock_then_succeeds() {
        for hosted in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let mut registry = ProfileRegistry::open(directory.path()).unwrap();
            registry
                .add(Profile {
                    name: "saved".into(),
                    label: None,
                    probe: "foks.app".into(),
                    protocol: ProtocolPolicy::CurrentProbeOnly {
                        canary_public_key:
                            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
                                .into(),
                        lease_url: "https://updates.example.test/lease.json".into(),
                        last_artifact: None,
                    },
                    trust: TrustRoot::WebPki,
                })
                .unwrap();
            drop(registry);
            let lock = std::fs::File::open(directory.path().join(".profiles.lock")).unwrap();
            fs2::FileExt::lock_exclusive(&lock).unwrap();
            let root = directory.path().to_owned();
            let (attempted, observed) = std::sync::mpsc::sync_channel(1);
            let (finished, completed) = std::sync::mpsc::channel();
            let worker = std::thread::spawn(move || {
                let result = read_registry_snapshot(Instant::now(), Duration::from_secs(5), || {
                    let snapshot = ProfileRegistry::try_open(&root).and_then(|registry| {
                        if hosted {
                            registry
                                .hosted_lease_renewal("saved")
                                .map(|snapshot| assert!(snapshot.is_some()))
                        } else {
                            registry.profile("saved").map(|_| ())
                        }
                    });
                    if matches!(&snapshot, Err(foks_client_app::Error::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock)
                    {
                        let _ = attempted.try_send(());
                    }
                    snapshot
                });
                finished.send(result).unwrap();
            });
            observed.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(matches!(
                completed.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ));
            fs2::FileExt::unlock(&lock).unwrap();
            completed
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap();
            worker.join().unwrap();
        }
    }

    #[test]
    fn registry_snapshot_stops_at_the_deadline_without_replaying_other_errors() {
        let directory = tempfile::tempdir().unwrap();
        drop(ProfileRegistry::open(directory.path()).unwrap());
        let lock = std::fs::File::open(directory.path().join(".profiles.lock")).unwrap();
        fs2::FileExt::lock_exclusive(&lock).unwrap();
        let started = Instant::now();
        let timeout = Duration::from_millis(50);
        let mut attempts = 0;
        let result = read_registry_snapshot(started, timeout, || {
            attempts += 1;
            ProfileRegistry::try_open(directory.path())
        });
        assert!(
            matches!(result, Err(foks_client_app::Error::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
        assert!(started.elapsed() >= timeout);
        assert!(attempts > 0);
        fs2::FileExt::unlock(&lock).unwrap();
        let mut attempts = 0;
        let result = read_registry_snapshot::<()>(Instant::now(), Duration::from_secs(5), || {
            attempts += 1;
            Err(foks_client_app::Error::ProfileMissing)
        });
        assert!(matches!(
            result,
            Err(foks_client_app::Error::ProfileMissing)
        ));
        assert_eq!(attempts, 1);
        let mut calls_after_deadline = 0;
        let _ = read_registry_snapshot(Instant::now(), Duration::ZERO, || {
            calls_after_deadline += 1;
            Ok(())
        });
        assert_eq!(calls_after_deadline, 0);
    }

    #[tokio::test]
    async fn hosted_pipeline_bounds_parallelism_without_waiting_for_an_unhealthy_profile() {
        let release_slow = Arc::new(Semaphore::new(0));
        let release_fast = Arc::new(Semaphore::new(0));
        let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let peak = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (started, mut observed) = tokio::sync::mpsc::unbounded_channel();
        let worker_slow = release_slow.clone();
        let worker_fast = release_fast.clone();
        let worker_peak = peak.clone();
        let task = tokio::spawn(run_hosted_refreshes(
            (0..8).map(|index| index.to_string()).collect(),
            CancellationToken::new(),
            move |profile| {
                let release = if profile == "0" {
                    worker_slow.clone()
                } else {
                    worker_fast.clone()
                };
                let active = active.clone();
                let peak = worker_peak.clone();
                let started = started.clone();
                async move {
                    let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(count, Ordering::SeqCst);
                    started.send(profile).unwrap();
                    release.acquire().await.unwrap().forget();
                    active.fetch_sub(1, Ordering::SeqCst);
                }
            },
        ));
        let mut first = Vec::new();
        for _ in 0..MAXIMUM_CANARY_FETCHES {
            first.push(
                tokio::time::timeout(Duration::from_secs(5), observed.recv())
                    .await
                    .unwrap()
                    .unwrap(),
            );
        }
        first.sort();
        assert_eq!(first, ["0", "1", "2", "3"]);
        assert!(observed.try_recv().is_err());
        release_fast.add_permits(1);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), observed.recv())
                .await
                .unwrap()
                .unwrap(),
            "4"
        );
        release_fast.add_permits(8);
        let mut remaining = Vec::new();
        for _ in 0..3 {
            remaining.push(
                tokio::time::timeout(Duration::from_secs(5), observed.recv())
                    .await
                    .unwrap()
                    .unwrap(),
            );
        }
        remaining.sort();
        assert_eq!(remaining, ["5", "6", "7"]);
        assert!(!task.is_finished());
        release_slow.add_permits(1);
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(peak.load(Ordering::SeqCst), MAXIMUM_CANARY_FETCHES);
    }

    #[tokio::test]
    async fn cancelling_the_hosted_pipeline_retains_active_common_worker_permits() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_owned();
        let workers = Arc::new(Semaphore::new(MAXIMUM_CANARY_FETCHES));
        let release = Arc::new(Semaphore::new(0));
        let cancellation = CancellationToken::new();
        let (started, mut observed) = tokio::sync::mpsc::unbounded_channel();
        let (finished, mut completed) = tokio::sync::mpsc::unbounded_channel();
        let worker_slots = workers.clone();
        let worker_release = release.clone();
        let task = tokio::spawn(run_hosted_refreshes(
            (0..8).map(|index| index.to_string()).collect(),
            cancellation.clone(),
            move |profile| {
                let key = flight_key(&root, &profile, false);
                let workers = worker_slots.clone();
                let release = worker_release.clone();
                let started = started.clone();
                let finished = finished.clone();
                async move {
                    coalesce(key, Duration::from_secs(10), move || async move {
                        let permit = workers.acquire_owned().await.unwrap();
                        started.send(()).unwrap();
                        release.acquire().await.unwrap().forget();
                        drop(permit);
                        finished.send(()).unwrap();
                        (success("unchanged"), Phases::new())
                    })
                    .await;
                }
            },
        ));
        for _ in 0..MAXIMUM_CANARY_FETCHES {
            tokio::time::timeout(Duration::from_secs(5), observed.recv())
                .await
                .unwrap()
                .unwrap();
        }
        cancellation.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(workers.available_permits(), 0);
        assert!(observed.try_recv().is_err());
        release.add_permits(MAXIMUM_CANARY_FETCHES);
        for _ in 0..MAXIMUM_CANARY_FETCHES {
            tokio::time::timeout(Duration::from_secs(5), completed.recv())
                .await
                .unwrap()
                .unwrap();
        }
        assert_eq!(workers.available_permits(), MAXIMUM_CANARY_FETCHES);
    }

    #[cfg(unix)]
    #[test]
    fn changed_registry_cannot_join_an_old_observation_flight() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("profiles.toml");
        std::fs::write(&file, b"before").unwrap();
        let before = flight_key(directory.path(), "saved", true);
        let replacement = directory.path().join("replacement.toml");
        std::fs::write(&replacement, b"after").unwrap();
        std::fs::rename(replacement, file).unwrap();
        assert_ne!(before, flight_key(directory.path(), "saved", true));
    }

    #[tokio::test]
    async fn concurrent_renewal_callers_share_one_result() {
        let directory = tempfile::tempdir().unwrap();
        let key = flight_key(directory.path(), "saved", false);
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let first_started = started.clone();
        let first_release = release.clone();
        let first_key = key.clone();
        let first = tokio::spawn(async move {
            coalesce(first_key, Duration::from_secs(5), move || async move {
                first_started.notify_one();
                first_release.notified().await;
                (success("renewed"), vec![("fetch".to_owned(), 7)])
            })
            .await
        });
        started.notified().await;
        let second = coalesce(key, Duration::from_secs(5), || async {
            panic!("duplicate renewal was dispatched")
        });
        let releaser = async {
            tokio::task::yield_now().await;
            release.notify_one();
        };
        let (second, _) = tokio::join!(second, releaser);
        assert_eq!(first.await.unwrap(), second);
        // The caller that joined the flight is answered with the steps of
        // the observation it is given, not with none of its own.
        assert_eq!(second.1, [("fetch".to_owned(), 7)]);
    }

    #[tokio::test]
    async fn observation_timeout_does_not_release_an_active_flight() {
        let directory = tempfile::tempdir().unwrap();
        let key = flight_key(directory.path(), "saved", false);
        let release = Arc::new(tokio::sync::Notify::new());
        let worker_release = release.clone();
        let first = coalesce(key.clone(), Duration::from_millis(10), move || async move {
            worker_release.notified().await;
            (success("renewed"), Phases::new())
        })
        .await;
        assert!(matches!(
            first.0,
            ResponseResult::Error {
                code: ErrorCode::DeadlineExceeded,
                ..
            }
        ));
        // A caller that gave up on the flight measured no steps of its own.
        assert!(first.1.is_empty());
        let second = coalesce(key, Duration::from_secs(5), || async {
            panic!("active flight was replaced")
        });
        let releaser = async {
            tokio::task::yield_now().await;
            release.notify_one();
        };
        let (result, _) = tokio::join!(second, releaser);
        assert_eq!(result.0, success("renewed"));
    }

    #[tokio::test]
    async fn missing_saved_host_is_scoped_and_lease_free() {
        let directory = tempfile::tempdir().unwrap();
        ClientCredentials::initialize(directory.path(), CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(directory.path()).unwrap();
        registry
            .add(Profile {
                name: "saved".into(),
                label: None,
                probe: "localhost:1".into(),
                protocol: ProtocolPolicy::V019,
                trust: TrustRoot::WebPki,
            })
            .unwrap();
        let (result, timing) = reconcile(
            directory.path(),
            "saved",
            Duration::from_secs(5),
            Arc::new(Semaphore::new(1)),
        )
        .await;
        assert_eq!(result["identity"]["code"], "saved-trust-missing");
        assert_eq!(result["identity"]["fields"]["profile"], "saved");
        assert_eq!(result["compatibility"]["value"]["status"], "not-required");
        assert_eq!(result["profile"], "saved");
        // An observation states the steps of both halves even when it fails:
        // the identity check reached the saved host, and the compatibility
        // half stopped at the profile's renewal snapshot.
        assert_eq!(
            timing
                .phases
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["admit", "open", "host", "lease"]
        );
    }

    #[test]
    fn remote_failure_is_not_an_ipc_failure() {
        let error = foks_client_app::Error::Client(foks_client::Error::Connect(
            std::io::Error::from(std::io::ErrorKind::ConnectionRefused),
        ));
        assert!(matches!(
            identity_error("saved", &error),
            ResponseResult::Error {
                code: ErrorCode::ServerUnavailable,
                ..
            }
        ));
    }

    #[test]
    fn renewal_snapshot_recognizes_the_artifact_it_already_stores() {
        let directory = tempfile::tempdir().unwrap();
        let mut registry = ProfileRegistry::open(directory.path()).unwrap();
        let seed = [
            0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec,
            0x2c, 0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03,
            0x1c, 0xae, 0x7f, 0x60,
        ];
        let mut artifact = foks_compat_artifact::CanaryArtifact {
            schema_version: foks_compat_artifact::SCHEMA_VERSION,
            generation: 1,
            target: "foks.app".into(),
            run_id: "reconcile-1".into(),
            generated_at: 100,
            expires_at: 200,
            protocol_metadata_sha256: foks_client_app::PINNED_PROTOCOL_METADATA_SHA256.into(),
            mutation_digest: "11".repeat(32),
            read_digest: "11".repeat(32),
            outcome: foks_compat_artifact::Outcome::Compatible,
            capabilities: ["kv".to_owned()].into_iter().collect(),
            drift_reason: String::new(),
        };
        let signed =
            foks_compat_artifact::SignedCanaryArtifact::sign(artifact.clone(), &seed).unwrap();
        registry
            .add(Profile {
                name: "saved".into(),
                label: None,
                probe: "foks.app".into(),
                protocol: ProtocolPolicy::CurrentProbeOnly {
                    canary_public_key:
                        "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a".into(),
                    lease_url: "https://updates.example.test/lease.json".into(),
                    last_artifact: None,
                },
                trust: TrustRoot::WebPki,
            })
            .unwrap();

        // Nothing is stored yet, so the fetched artifact still has to be applied.
        assert!(!registry
            .hosted_lease_renewal("saved")
            .unwrap()
            .unwrap()
            .matches_stored_validated_artifact(&signed, 110));
        assert!(
            registry
                .hosted_lease_renewal("saved")
                .unwrap()
                .unwrap()
                .apply(&signed, 110)
                .unwrap()
                .0
        );

        // Refetching the same artifact is a no-op that needs no admission,
        // and `apply` agrees that it changes nothing.
        let snapshot = registry.hosted_lease_renewal("saved").unwrap().unwrap();
        assert!(snapshot.matches_stored_validated_artifact(&signed, 110));
        assert!(!snapshot.apply(&signed, 110).unwrap().0);

        // Once the stored artifact has lapsed, `apply` rejects it rather than
        // reporting "unchanged", so the short circuit must not fire.
        let snapshot = registry.hosted_lease_renewal("saved").unwrap().unwrap();
        assert!(!snapshot.matches_stored_validated_artifact(&signed, 200));
        assert!(snapshot.apply(&signed, 200).is_err());

        // A different artifact is never short-circuited.
        artifact.generation = 2;
        artifact.run_id = "reconcile-2".into();
        let next =
            foks_compat_artifact::SignedCanaryArtifact::sign(artifact.clone(), &seed).unwrap();
        assert!(!registry
            .hosted_lease_renewal("saved")
            .unwrap()
            .unwrap()
            .matches_stored_validated_artifact(&next, 110));

        // A stored artifact that does not grant this client leaves the
        // profile probe-only, where `apply` reports the rejection; that
        // outcome must keep reaching the caller.
        artifact.generation = 3;
        artifact.outcome = foks_compat_artifact::Outcome::Drift;
        artifact.capabilities.clear();
        artifact.drift_reason = "protocol drift".into();
        let drift = foks_compat_artifact::SignedCanaryArtifact::sign(artifact, &seed).unwrap();
        registry
            .hosted_lease_renewal("saved")
            .unwrap()
            .unwrap()
            .apply(&drift, 110)
            .unwrap();
        assert!(!registry
            .hosted_lease_renewal("saved")
            .unwrap()
            .unwrap()
            .matches_stored_validated_artifact(&drift, 110));
    }

    #[test]
    fn renewal_snapshot_rejects_removal_recreation_and_invalid_artifacts() {
        let directory = tempfile::tempdir().unwrap();
        let mut registry = ProfileRegistry::open(directory.path()).unwrap();
        let seed = [7; 32];
        let mut artifact = foks_compat_artifact::CanaryArtifact {
            schema_version: foks_compat_artifact::SCHEMA_VERSION,
            generation: 1,
            target: "foks.app".into(),
            run_id: "reconcile-1".into(),
            generated_at: 100,
            expires_at: 200,
            protocol_metadata_sha256: foks_client_app::PINNED_PROTOCOL_METADATA_SHA256.into(),
            mutation_digest: "11".repeat(32),
            read_digest: "11".repeat(32),
            outcome: foks_compat_artifact::Outcome::Compatible,
            capabilities: ["kv".to_owned()].into_iter().collect(),
            drift_reason: String::new(),
        };
        let signed =
            foks_compat_artifact::SignedCanaryArtifact::sign(artifact.clone(), &seed).unwrap();
        let profile = Profile {
            name: "saved".into(),
            label: None,
            probe: "foks.app".into(),
            protocol: ProtocolPolicy::CurrentProbeOnly {
                canary_public_key:
                    "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a".into(),
                lease_url: "https://updates.example.test/lease.json".into(),
                last_artifact: None,
            },
            trust: TrustRoot::WebPki,
        };
        registry.add(profile.clone()).unwrap();
        assert!(registry
            .hosted_lease_renewal("saved")
            .unwrap()
            .unwrap()
            .apply(&signed, 110)
            .is_err());
        let seed = [
            0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec,
            0x2c, 0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03,
            0x1c, 0xae, 0x7f, 0x60,
        ];
        let signed =
            foks_compat_artifact::SignedCanaryArtifact::sign(artifact.clone(), &seed).unwrap();
        let obsolete = registry.hosted_lease_renewal("saved").unwrap().unwrap();
        registry.remove("saved").unwrap();
        registry.add(profile).unwrap();
        assert!(matches!(
            obsolete.apply(&signed, 110),
            Err(foks_client_app::Error::ProfileRegistryChanged)
        ));
        assert!(registry
            .hosted_lease_renewal("saved")
            .unwrap()
            .unwrap()
            .apply(&signed, 200)
            .is_err());
        assert!(
            registry
                .hosted_lease_renewal("saved")
                .unwrap()
                .unwrap()
                .apply(&signed, 110)
                .unwrap()
                .0
        );
        assert!(
            !registry
                .hosted_lease_renewal("saved")
                .unwrap()
                .unwrap()
                .apply(&signed, 110)
                .unwrap()
                .0
        );
        artifact.generation = 2;
        artifact.outcome = foks_compat_artifact::Outcome::Drift;
        artifact.capabilities.clear();
        artifact.drift_reason = "protocol drift".into();
        let drift = foks_compat_artifact::SignedCanaryArtifact::sign(artifact, &seed).unwrap();
        let (_, status) = registry
            .hosted_lease_renewal("saved")
            .unwrap()
            .unwrap()
            .apply(&drift, 110)
            .unwrap();
        assert!(matches!(
            status,
            foks_client_app::CompatibilityStatus::Incompatible { .. }
        ));
    }
}
