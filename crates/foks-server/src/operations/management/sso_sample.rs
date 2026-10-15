//! OIDC rollout gauges for `/metrics`, sampled off the management runtime.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use foks_server_db::{ReadDatabase, SsoRolloutStatus};

use super::InFlight;
use crate::{Error, ReadDatabaseConfig, Result, ServerMetrics};

/// How long one completed sample answers later scrapes.
const CACHE_LIFETIME: Duration = Duration::from_secs(30);
/// How long a scrape waits for a sample before it omits the gauges.
const SAMPLE_TIMEOUT: Duration = Duration::from_secs(1);

/// Samples the SSO rollout status on the blocking pool so that a slow cohort
/// count cannot delay `/healthz` or `/readyz` on the management thread's
/// single runtime thread. One read connection is kept between samples, at
/// most one sample runs at a time, and a completed sample answers scrapes for
/// `CACHE_LIFETIME`. A scrape that gets no sample, because the sample failed,
/// exceeded `SAMPLE_TIMEOUT` or was still running from an earlier scrape,
/// omits the gauges and increments `foks_oidc_sample_failures_total`.
pub(super) struct SsoSampler {
    /// `None` when OIDC is not configured; the database is then never read.
    database: Option<ReadDatabaseConfig>,
    timeout: Duration,
    reader: Arc<Mutex<Option<ReadDatabase>>>,
    cached: Arc<Mutex<Option<Cached>>>,
    in_flight: Arc<AtomicBool>,
}

struct Cached {
    sampled_at: Instant,
    status: Option<SsoRolloutStatus>,
}

impl SsoSampler {
    pub(super) fn new(database: Option<ReadDatabaseConfig>) -> Self {
        Self {
            database,
            timeout: SAMPLE_TIMEOUT,
            reader: Arc::default(),
            cached: Arc::default(),
            in_flight: Arc::default(),
        }
    }

    /// Returns the rollout status to export, or `None` when OIDC is not
    /// configured, the host has no policy, or no sample is available.
    pub(super) async fn sample(
        &self,
        metrics: &ServerMetrics,
        now: Instant,
    ) -> Option<SsoRolloutStatus> {
        let database = self.database.clone()?;
        if let Some(status) = self.fresh(now) {
            return status;
        }
        if self
            .in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            metrics.oidc_sample_failed();
            return None;
        }
        let guard = InFlight(Arc::clone(&self.in_flight));
        let reader = Arc::clone(&self.reader);
        let cached = Arc::clone(&self.cached);
        let task = tokio::task::spawn_blocking(move || {
            let _guard = guard;
            let status = read(&reader, &database)?;
            // A sample that completes after its scrape timed out still
            // answers the scrapes that follow.
            if let Ok(mut cached) = cached.lock() {
                *cached = Some(Cached {
                    sampled_at: now,
                    status: status.clone(),
                });
            }
            Ok::<_, Error>(status)
        });
        match tokio::time::timeout(self.timeout, task).await {
            Ok(Ok(Ok(status))) => status,
            _ => {
                metrics.oidc_sample_failed();
                None
            }
        }
    }

    fn fresh(&self, now: Instant) -> Option<Option<SsoRolloutStatus>> {
        let cached = self.cached.lock().ok()?;
        let cached = cached.as_ref()?;
        (now.saturating_duration_since(cached.sampled_at) < CACHE_LIFETIME)
            .then(|| cached.status.clone())
    }
}

/// Reads one sample on the kept connection, opening it first if needed. A
/// failed read drops the connection so that the next sample reopens it.
fn read(
    reader: &Mutex<Option<ReadDatabase>>,
    database: &ReadDatabaseConfig,
) -> Result<Option<SsoRolloutStatus>> {
    let mut reader = reader.lock().map_err(|_| Error::Thread)?;
    if reader.is_none() {
        *reader = Some(ReadDatabase::open_pooled(
            &database.path,
            database.database.clone(),
        )?);
    }
    let result = reader.as_ref().map_or(Ok(None), rollout_status);
    if result.is_err() {
        *reader = None;
    }
    result
}

// A read-only sample; metrics never enqueue a policy mutation.
fn rollout_status(database: &ReadDatabase) -> Result<Option<SsoRolloutStatus>> {
    let Some(host) = database.host_bootstrap()? else {
        return Ok(None);
    };
    let host = host
        .host_id
        .as_slice()
        .try_into()
        .map_err(|_| Error::Config("invalid host ID"))?;
    let snapshot = database.snapshot()?;
    Ok(snapshot.sso_rollout_status(host)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn database() -> (tempfile::TempDir, ReadDatabaseConfig) {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("sample.sqlite");
        drop(foks_server_db::Database::open(&path, foks_server_db::Config::default()).unwrap());
        (
            temporary,
            ReadDatabaseConfig {
                path,
                database: foks_server_db::Config::default(),
            },
        )
    }

    #[test]
    fn unconfigured_oidc_never_reads_the_database() {
        let metrics = ServerMetrics::default();
        let sampler = SsoSampler::new(None);
        assert_eq!(
            runtime().block_on(sampler.sample(&metrics, Instant::now())),
            None
        );
        assert!(sampler.reader.lock().unwrap().is_none());
        assert!(sampler.cached.lock().unwrap().is_none());
        assert_eq!(metrics.snapshot().oidc_sample_failures, 0);
    }

    #[test]
    fn samples_are_cached_and_failures_are_counted() {
        let (temporary, config) = database();
        let path = config.path.clone();
        let metrics = ServerMetrics::default();
        let sampler = SsoSampler::new(Some(config));
        let runtime = runtime();
        let start = Instant::now();

        assert_eq!(runtime.block_on(sampler.sample(&metrics, start)), None);
        assert!(sampler.reader.lock().unwrap().is_some());
        assert!(sampler.cached.lock().unwrap().is_some());

        // Within the cache lifetime a scrape neither reopens nor rereads.
        *sampler.reader.lock().unwrap() = None;
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            runtime.block_on(sampler.sample(&metrics, start + Duration::from_secs(29))),
            None
        );
        assert!(sampler.reader.lock().unwrap().is_none());
        assert_eq!(metrics.snapshot().oidc_sample_failures, 0);

        // After it, the reopen fails and every failed scrape is counted.
        let later = start + CACHE_LIFETIME;
        assert_eq!(runtime.block_on(sampler.sample(&metrics, later)), None);
        assert_eq!(runtime.block_on(sampler.sample(&metrics, later)), None);
        assert_eq!(metrics.snapshot().oidc_sample_failures, 2);
        drop(temporary);
    }

    #[test]
    fn a_slow_sample_times_out_and_is_not_repeated_while_running() {
        let (_temporary, config) = database();
        let metrics = ServerMetrics::default();
        let mut sampler = SsoSampler::new(Some(config));
        sampler.timeout = Duration::from_millis(50);
        let runtime = runtime();
        let now = Instant::now();

        // Holding the reader stands in for a slow cohort count.
        let held = sampler.reader.lock().unwrap();
        assert_eq!(runtime.block_on(sampler.sample(&metrics, now)), None);
        assert_eq!(metrics.snapshot().oidc_sample_failures, 1);
        assert_eq!(runtime.block_on(sampler.sample(&metrics, now)), None);
        assert_eq!(metrics.snapshot().oidc_sample_failures, 2);
        drop(held);

        let deadline = Instant::now() + Duration::from_secs(10);
        while sampler.in_flight.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "sample did not finish");
            std::thread::yield_now();
        }
        // The late sample filled the cache, so this scrape uses it.
        assert!(sampler.cached.lock().unwrap().is_some());
        assert_eq!(runtime.block_on(sampler.sample(&metrics, now)), None);
        assert_eq!(metrics.snapshot().oidc_sample_failures, 2);
        assert!(sampler.reader.lock().unwrap().is_some());
    }
}
