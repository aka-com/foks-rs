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
    })
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
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(50);
    // Local maintenance participates in the same FIFO as foreground requests.
    // It has a short admission budget and never holds a partial profile set.
    let _admission = match super::profile_work::coordinator().acquire_blocking(
        state,
        super::profile_work::Scope::profile(profile),
        deadline.saturating_duration_since(std::time::Instant::now()),
        &foks_client_app::CancellationToken::new(),
    ) {
        Ok(permit) => permit,
        Err(_) => return Ok(None),
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
        checked
            .maintain_adapter_state_until(cursor, &master, deadline)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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
