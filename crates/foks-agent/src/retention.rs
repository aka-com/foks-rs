//! Single-flight local maintenance, independent of data/network worker capacity.
use foks_client_app::{
    AdapterMaintenanceCursor, ClientCredentials, ProfileRegistry, ProfileSession,
};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
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
        ErrorCode::RetentionFull => {
            RETENTION_FULL.fetch_add(1, Ordering::Relaxed);
        }
        ErrorCode::SubmissionActiveFull => {
            ACTIVE_FULL.fetch_add(1, Ordering::Relaxed);
        }
        ErrorCode::ClockUntrusted => {
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
    let registry = ProfileRegistry::open(state)?;
    let profile = registry
        .profiles()
        .find(|p| {
            cursors
                .after_profile
                .as_ref()
                .is_none_or(|last| p.name > *last)
        })
        .or_else(|| registry.profiles().next())
        .map(|p| p.name.clone());
    let Some(profile) = profile else {
        return Ok(None);
    };
    cursors.after_profile = Some(profile.clone());
    // Local maintenance participates in the same FIFO as foreground requests.
    // It has a short admission budget and never holds a partial profile set.
    let _admission = match super::profile_work::coordinator().acquire_blocking(
        state,
        super::profile_work::Scope::profile(&profile),
        std::time::Duration::from_secs(1),
        &foks_client_app::CancellationToken::new(),
    ) {
        Ok(permit) => permit,
        Err(_) => return Ok(None),
    };
    // A root-scoped registry edit may have completed while admission waited.
    let registry = ProfileRegistry::open(state)?;
    let paths = registry.paths(&profile)?;
    // Registry-only profiles must not be initialized by maintenance.
    if !paths.hard_database.try_exists()? {
        return Ok(None);
    }
    let credentials = ClientCredentials::open(state)?;
    let session = ProfileSession::open(&registry, &profile)?;
    if !cursors.profiles.contains_key(&profile) && cursors.profiles.len() >= 64 {
        cursors.profiles.pop_first();
    }
    let cursor = cursors.profiles.entry(profile).or_default();
    credentials.try_with_checked_session(&session, |checked| {
        let master = credentials.master_key()?;
        checked
            .maintain_adapter_state(cursor, &master)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
    })
}
