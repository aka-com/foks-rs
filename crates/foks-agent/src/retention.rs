//! Single-flight local maintenance, independent of data/network worker capacity.
use foks_client_app::{
    AdapterMaintenanceCursor, ClientCredentials, ProfileRegistry, ProfileSession,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex, OnceLock,
    },
};

static RETENTION_FULL: AtomicU64 = AtomicU64::new(0);
static ACTIVE_FULL: AtomicU64 = AtomicU64::new(0);
static ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static SUCCESSES: AtomicU64 = AtomicU64::new(0);
static FAILURES: AtomicU64 = AtomicU64::new(0);
static SKIPPED: AtomicU64 = AtomicU64::new(0);
static COMPACTED: AtomicU64 = AtomicU64::new(0);
static PRUNED: AtomicU64 = AtomicU64::new(0);
static TEMPORARY: AtomicU64 = AtomicU64::new(0);
static FINAL: AtomicU64 = AtomicU64::new(0);
static INVENTORY_SATURATED: AtomicU64 = AtomicU64::new(0);
static DEFERRED: AtomicU64 = AtomicU64::new(0);
static CLOCK_UNTRUSTED: AtomicU64 = AtomicU64::new(0);
static FAILING: Mutex<BTreeMap<String, Failure>> = Mutex::new(BTreeMap::new());

/// The budget of one retention pass: profile admission, the registry,
/// credential and profile-session opens and the checked-session entry. The
/// maintenance slice is taken only after all of them, so a slow keystore or
/// a short foreground hold does not consume the slice or fail the pass.
pub(super) const PASS_BUDGET: std::time::Duration = std::time::Duration::from_secs(1);
/// The work one pass does once the checked session has been entered.
const SLICE: std::time::Duration = std::time::Duration::from_millis(50);
/// A profile whose passes keep failing with the same error is logged at
/// most this often.
const ERROR_LOG_INTERVAL_MS: u64 = 10 * 60 * 1000;

/// The latest failed pass of a profile, kept until a pass of that profile
/// succeeds. Only `kind` and the age leave the agent through
/// RetentionStatus; the profile and message appear in the agent's log.
struct Failure {
    kind: &'static str,
    message: String,
    at_ms: u64,
    logged_at_ms: u64,
}
/// Failures are tracked for at most this many profiles.
const FAILING_PROFILES: usize = 64;

#[derive(Default)]
struct Cursors {
    after_profile: Option<String>,
    profiles: BTreeMap<String, AdapterMaintenanceCursor>,
    schedule: Schedule,
}

#[derive(Default)]
struct Schedule {
    due: BTreeMap<String, (u64, u8)>,
}
impl Schedule {
    fn choose(&mut self, names: &[String], after: Option<&str>, now: u64) -> Option<String> {
        self.due.retain(|name, _| names.contains(name));
        for name in names {
            self.due.entry(name.clone()).or_insert((0, 2));
        }
        names
            .iter()
            .filter(|name| after.is_none_or(|after| name.as_str() > after))
            .chain(
                names
                    .iter()
                    .filter(|name| after.is_some_and(|after| name.as_str() <= after)),
            )
            .find(|name| self.due[*name].0 <= now)
            .cloned()
    }
    fn finish(
        &mut self,
        name: &str,
        now: u64,
        report: Option<&foks_client_app::AdapterMaintenanceReport>,
    ) {
        let entry = self.due.get_mut(name).expect("selected profile");
        entry.1 = entry.1.saturating_sub(1);
        if report.is_some_and(|report| report.more_work || report.made_progress)
            || (report.is_some() && entry.1 > 0)
        {
            entry.0 = now.saturating_add(250);
        } else {
            *entry = (now.saturating_add(60_000), 2);
        }
    }
    fn wake(&mut self, name: String) {
        self.due.insert(name, (0, 2));
    }
    fn next(&self, now: u64) -> u64 {
        self.due
            .values()
            .map(|v| v.0)
            .min()
            .unwrap_or(now.saturating_add(60_000))
    }
}
static WAKE: AtomicBool = AtomicBool::new(true);
static NEXT: AtomicU64 = AtomicU64::new(0);
static HINTS: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
fn now_ms() -> u64 {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
pub(super) fn due() -> bool {
    WAKE.load(Ordering::Relaxed) || now_ms() >= NEXT.load(Ordering::Relaxed)
}
pub(super) fn wake_profile(name: &str) {
    if let Ok(mut hints) = HINTS.get_or_init(Default::default).lock() {
        if hints.len() < 64 {
            hints.insert(name.to_owned());
        }
    }
    WAKE.store(true, Ordering::Relaxed);
}

pub(super) fn snapshot() -> serde_json::Value {
    // Counters and a redacted failure kind only: no profile names or messages.
    let (failing_profiles, latest_failure) = FAILING.lock().map_or((0, None), |failing| {
        let latest = failing
            .values()
            .max_by_key(|failure| failure.at_ms)
            .map(|failure| {
                serde_json::json!({
                    "kind": failure.kind,
                    "age_ms": now_ms().saturating_sub(failure.at_ms),
                })
            });
        (failing.len(), latest)
    });
    serde_json::json!({
        "retention_full":RETENTION_FULL.load(Ordering::Relaxed),
        "active_full":ACTIVE_FULL.load(Ordering::Relaxed),
        "attempts":ATTEMPTS.load(Ordering::Relaxed), "successes":SUCCESSES.load(Ordering::Relaxed),
        "failures":FAILURES.load(Ordering::Relaxed), "skipped":SKIPPED.load(Ordering::Relaxed),
        "compacted":COMPACTED.load(Ordering::Relaxed), "pruned":PRUNED.load(Ordering::Relaxed),
        "temporary_removed":TEMPORARY.load(Ordering::Relaxed), "cleanup_deferred":DEFERRED.load(Ordering::Relaxed),
        "unowned_final_removed":FINAL.load(Ordering::Relaxed),
        "inventory_saturated":INVENTORY_SATURATED.load(Ordering::Relaxed),
        "clock_untrusted":CLOCK_UNTRUSTED.load(Ordering::Relaxed),
        "failing_profiles": failing_profiles,
        "latest_failure": latest_failure,
    })
}

/// A redacted classification of a failed pass, for the status report.
fn failure_kind(error: &(dyn std::error::Error + 'static)) -> &'static str {
    let mut next = Some(error);
    while let Some(error) = next {
        if error.is::<foks_local_deadline::Error>()
            || matches!(
                error.downcast_ref::<foks_client::Error>(),
                Some(foks_client::Error::DeadlineExceeded)
            )
        {
            return "deadline";
        }
        if error.is::<foks_keystore::Error>() {
            return "keystore";
        }
        if error.is::<foks_client_db::Error>() {
            return "database";
        }
        if error.is::<std::io::Error>() {
            return "io";
        }
        next = error.source();
    }
    "other"
}

/// Records a failed pass of `profile`, and logs it unless that profile
/// failed the same way within [`ERROR_LOG_INTERVAL_MS`].
fn record_failure(profile: &str, error: &(dyn std::error::Error + 'static)) {
    let message = error.to_string();
    let now = now_ms();
    let Ok(mut failing) = FAILING.lock() else {
        return;
    };
    let repeated = failing.get(profile).filter(|failure| {
        failure.message == message
            && now.saturating_sub(failure.logged_at_ms) < ERROR_LOG_INTERVAL_MS
    });
    let logged_at_ms = match repeated {
        Some(failure) => failure.logged_at_ms,
        None => {
            eprintln!("retention maintenance of profile {profile} failed: {message}");
            now
        }
    };
    if !failing.contains_key(profile) && failing.len() >= FAILING_PROFILES {
        // Evict the failure recorded longest ago.
        if let Some(oldest) = failing
            .iter()
            .min_by_key(|(_, failure)| failure.at_ms)
            .map(|(name, _)| name.clone())
        {
            failing.remove(&oldest);
        }
    }
    failing.insert(
        profile.to_owned(),
        Failure {
            kind: failure_kind(error),
            message,
            at_ms: now,
            logged_at_ms,
        },
    );
}

/// A profile removed from the registry no longer has a failing pass.
fn forget_removed_profiles(names: &[String]) {
    if let Ok(mut failing) = FAILING.lock() {
        failing.retain(|name, _| names.contains(name));
    }
}

/// A successful pass clears the profile's recorded failure.
fn record_success(profile: &str) {
    if let Ok(mut failing) = FAILING.lock() {
        failing.remove(profile);
    }
}

pub(super) fn rejected(code: foks_agent_proto::ErrorCode) {
    use foks_agent_proto::ErrorCode;
    match code {
        ErrorCode::RetentionFull | ErrorCode::UploadRetentionFull => {
            RETENTION_FULL.fetch_add(1, Ordering::Relaxed);
        }
        ErrorCode::SubmissionActiveFull | ErrorCode::UploadPendingFull => {
            ACTIVE_FULL.fetch_add(1, Ordering::Relaxed);
        }
        ErrorCode::ClockUntrusted | ErrorCode::UploadClockUntrusted => {
            CLOCK_UNTRUSTED.fetch_add(1, Ordering::Relaxed);
        }
        _ => (),
    }
}

pub(super) fn run_one_profile(state: &Path) {
    ATTEMPTS.fetch_add(1, Ordering::Relaxed);
    match run(state) {
        Ok(Some(report)) => {
            SUCCESSES.fetch_add(1, Ordering::Relaxed);
            COMPACTED.fetch_add(report.compacted, Ordering::Relaxed);
            PRUNED.fetch_add(report.pruned, Ordering::Relaxed);
            TEMPORARY.fetch_add(report.temporary_removed, Ordering::Relaxed);
            FINAL.fetch_add(report.final_removed, Ordering::Relaxed);
            INVENTORY_SATURATED.fetch_add(u64::from(report.inventory_saturated), Ordering::Relaxed);
            DEFERRED.fetch_add(u64::from(report.cleanup_deferred), Ordering::Relaxed);
            CLOCK_UNTRUSTED.fetch_add(u64::from(report.clock_untrusted), Ordering::Relaxed);
        }
        Ok(None) => {
            SKIPPED.fetch_add(1, Ordering::Relaxed);
        }
        Err(_) => {
            FAILURES.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn run(
    state: &Path,
) -> Result<Option<foks_client_app::AdapterMaintenanceReport>, Box<dyn std::error::Error>> {
    static CURSORS: OnceLock<Mutex<Cursors>> = OnceLock::new();
    let mut cursors = CURSORS
        .get_or_init(|| Mutex::new(Cursors::default()))
        .lock()
        .map_err(|_| "retention cursor unavailable")?;
    let registry = match ProfileRegistry::open(state) {
        Ok(registry) => registry,
        Err(error) => {
            WAKE.store(false, Ordering::Relaxed);
            NEXT.store(now_ms().saturating_add(60_000), Ordering::Relaxed);
            return Err(error.into());
        }
    };
    let names: Vec<String> = registry.profiles().map(|p| p.name.clone()).collect();
    forget_removed_profiles(&names);
    WAKE.store(false, Ordering::Relaxed);
    if let Ok(mut hints) = HINTS.get_or_init(Default::default).lock() {
        for name in std::mem::take(&mut *hints) {
            cursors.schedule.wake(name);
        }
    }
    let after = cursors.after_profile.clone();
    let now = now_ms();
    let Some(profile) = cursors.schedule.choose(&names, after.as_deref(), now) else {
        NEXT.store(cursors.schedule.next(now), Ordering::Relaxed);
        return Ok(None);
    };
    cursors.after_profile = Some(profile.clone());
    let result = run_profile(state, &profile, &mut cursors.profiles);
    match &result {
        Ok(Some(_)) => record_success(&profile),
        Ok(None) => (),
        Err(error) => record_failure(&profile, error.as_ref()),
    }
    cursors.schedule.finish(
        &profile,
        now_ms(),
        result.as_ref().ok().and_then(|r| r.as_ref()),
    );
    NEXT.store(cursors.schedule.next(now_ms()), Ordering::Relaxed);
    result
}

fn run_profile(
    state: &Path,
    profile: &str,
    profiles: &mut BTreeMap<String, AdapterMaintenanceCursor>,
) -> Result<Option<foks_client_app::AdapterMaintenanceReport>, Box<dyn std::error::Error>> {
    let Some(_admission) = admit(state, profile) else {
        return Ok(None);
    };
    // A root-scoped registry edit may have completed while admission waited.
    let registry = ProfileRegistry::open(state)?;
    let paths = registry.paths(profile)?;
    // Registry-only profiles must not be initialized by maintenance.
    if !paths.hard_database.try_exists()? {
        return Ok(None);
    }
    let credentials = ClientCredentials::open(state)?;
    let session = ProfileSession::open(&registry, profile)?;
    if !profiles.contains_key(profile) && profiles.len() >= 64 {
        profiles.pop_first();
    }
    let cursor = profiles.entry(profile.to_owned()).or_default();
    credentials.try_with_checked_session(&session, |checked| {
        let master = credentials.master_key()?;
        let deadline = std::time::Instant::now() + SLICE;
        checked
            .maintain_adapter_state_until(cursor, &master, deadline)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
    })
}

/// Local maintenance participates in the same FIFO as foreground requests.
/// It waits at most the pass budget and never holds a partial profile set.
fn admit(state: &Path, profile: &str) -> Option<super::profile_work::Permit> {
    super::profile_work::coordinator()
        .acquire_blocking(
            state,
            super::profile_work::Scope::profile(profile),
            PASS_BUDGET,
            &foks_client_app::CancellationToken::new(),
        )
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The failure registry is process-wide; its tests run one at a time.
    static FAILING_TESTS: Mutex<()> = Mutex::new(());

    /// A foreground request that holds the profile for longer than the
    /// maintenance slice delays the pass instead of failing its admission.
    #[test]
    fn admission_waits_past_the_slice_within_the_pass_budget() {
        let state = tempfile::tempdir().unwrap();
        let held = super::super::profile_work::coordinator()
            .try_acquire(
                state.path(),
                super::super::profile_work::Scope::profile("p"),
            )
            .unwrap()
            .unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(SLICE * 4);
            drop(held);
        });
        let control = super::super::request_control::RequestControl::background(
            PASS_BUDGET,
            foks_client_app::CancellationToken::new(),
        )
        .unwrap();
        let _scope = control.enter().unwrap();
        let started = std::time::Instant::now();
        assert!(admit(state.path(), "p").is_some());
        assert!(started.elapsed() >= SLICE);
        release.join().unwrap();
    }

    /// Failures are logged at most once per interval for each profile, even
    /// when profiles alternate; the status carries a kind and an age, never
    /// the profile or message; a later success clears the profile.
    #[test]
    fn failures_are_redacted_rate_limited_per_profile_and_cleared() {
        let _serial = FAILING_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let logged_at = |profile: &str| FAILING.lock().unwrap()[profile].logged_at_ms;
        let error = std::io::Error::other("checked session refused for private-name");
        record_failure("retention-test-a", &error);
        let first_a = logged_at("retention-test-a");
        record_failure("retention-test-b", &error);
        let first_b = logged_at("retention-test-b");
        record_failure("retention-test-a", &error);
        record_failure("retention-test-b", &error);
        assert_eq!(logged_at("retention-test-a"), first_a);
        assert_eq!(logged_at("retention-test-b"), first_b);
        let status = snapshot();
        assert!(status["failing_profiles"].as_u64().unwrap() >= 2);
        assert_eq!(status["latest_failure"]["kind"], "io");
        let text = status.to_string();
        assert!(!text.contains("retention-test") && !text.contains("private-name"));
        record_success("retention-test-a");
        assert!(!FAILING.lock().unwrap().contains_key("retention-test-a"));
        // A profile no longer in the registry is forgotten.
        let others: Vec<String> = FAILING
            .lock()
            .unwrap()
            .keys()
            .filter(|name| *name != "retention-test-b")
            .cloned()
            .collect();
        forget_removed_profiles(&others);
        assert!(!FAILING.lock().unwrap().contains_key("retention-test-b"));
    }

    /// At the cap the failure recorded longest ago is evicted, whatever its
    /// profile's name.
    #[test]
    fn the_oldest_failure_is_evicted_at_the_cap() {
        let _serial = FAILING_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        FAILING.lock().unwrap().clear();
        let error = std::io::Error::other("refused");
        for index in 0..FAILING_PROFILES - 1 {
            record_failure(&format!("retention-cap-{index:02}"), &error);
        }
        record_failure("zz-retention-cap-oldest", &error);
        for (name, failure) in FAILING.lock().unwrap().iter_mut() {
            failure.at_ms = if name == "zz-retention-cap-oldest" {
                5
            } else {
                10
            };
        }
        record_failure("retention-cap-new", &error);
        let failing = FAILING.lock().unwrap();
        assert_eq!(failing.len(), FAILING_PROFILES);
        assert!(!failing.contains_key("zz-retention-cap-oldest"));
        assert!(failing.contains_key("retention-cap-00"));
        assert!(failing.contains_key("retention-cap-new"));
    }

    #[test]
    fn startup_discovers_last_profile_and_backoff_is_profile_local() {
        let names = vec!["a".into(), "b".into(), "c".into()];
        let mut schedule = Schedule::default();
        let idle = foks_client_app::AdapterMaintenanceReport::default();
        let active = foks_client_app::AdapterMaintenanceReport {
            more_work: true,
            made_progress: true,
            ..Default::default()
        };
        let mut after = None;
        for (now, expected) in [
            (0, "a"),
            (250, "b"),
            (500, "c"),
            (750, "a"),
            (1000, "b"),
            (1250, "c"),
        ] {
            let chosen = schedule.choose(&names, after.as_deref(), now).unwrap();
            assert_eq!(chosen, expected);
            schedule.finish(
                &chosen,
                now,
                Some(if chosen == "c" { &active } else { &idle }),
            );
            after = Some(chosen);
        }
        assert_eq!(
            schedule.choose(&names, after.as_deref(), 1500).as_deref(),
            Some("c")
        );
        schedule.finish("c", 1500, None);
        let c_due = schedule.due["c"];
        schedule.wake("b".into());
        assert_eq!(
            schedule.choose(&names, Some("c"), 1750).as_deref(),
            Some("b")
        );
        assert_eq!(schedule.due["c"], c_due);
    }
}
