use super::super::{permission_denied, ServerData};
use crate::metrics::realtime::ReconcileSource;
use crate::{auth::Principal, rpc::RoutedCall, services::realtime::Response};
use foks_proto::{RealtimeWire, RtAppId, RtInboxPollResult};
use foks_rpc::{
    encode_success_response_at, encode_void_success_response_at, RealtimeRequest, RpcStatus,
};
use rustls::pki_types::CertificateDer;
use std::{sync::Arc, time::Duration};
pub(super) fn response(
    data: &ServerData,
    call: RoutedCall,
    principal: Option<&Principal>,
) -> Result<Vec<u8>, RpcStatus> {
    let principal = principal.ok_or_else(permission_denied)?;
    let result = data.realtime.dispatch(
        call.route.position,
        call.call.argument(),
        principal,
        &data.host_id,
        || data.read_database(),
        data.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
        &data.clock,
    )?;
    match result {
        Response::Void => encode_void_success_response_at(call.call.sequence()),
        Response::Data(bytes) => encode_success_response_at(&bytes, call.call.sequence()),
    }
    .map_err(|_| RpcStatus::TransactionRetry)
}

pub(super) async fn poll_response(
    data: &ServerData,
    call: RoutedCall,
    certificate: Vec<u8>,
) -> Result<Vec<u8>, RpcStatus> {
    let mut outcome = PollOutcome::Cancelled;
    struct Guard<'a>(&'a crate::ServerMetrics, &'a mut PollOutcome);
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            self.0.realtime.update(|s| match self.1 {
                PollOutcome::Bumped => s.bumped_polls += 1,
                PollOutcome::TimedOut => s.timed_out_polls += 1,
                PollOutcome::Failed => s.failed_polls += 1,
                PollOutcome::Cancelled => s.cancelled_polls += 1,
                // Counted in foks_admission_rejected_total.
                PollOutcome::Refused => {}
            });
        }
    }
    let guard = Guard(&data.metrics, &mut outcome);
    let result = poll_inner(data, call, certificate, guard.1).await;
    if result.is_err() && !matches!(guard.1, PollOutcome::Refused) {
        *guard.1 = PollOutcome::Failed;
    }
    result
}

enum PollOutcome {
    Bumped,
    TimedOut,
    Failed,
    Cancelled,
    /// The account already held its maximum concurrent polls.
    Refused,
}

async fn poll_inner(
    data: &ServerData,
    call: RoutedCall,
    certificate: Vec<u8>,
    outcome: &mut PollOutcome,
) -> Result<Vec<u8>, RpcStatus> {
    if call.route.id != crate::rpc::RouteId::RealTimeRtPollInbox {
        return Err(RpcStatus::Unsupported);
    }
    let sequence = call.call.sequence();
    let request = RealtimeRequest::decode_argument(call.route.position, call.call.argument())
        .map_err(|_| RpcStatus::BadArguments("invalid realtime request".into()))?;
    let RealtimeRequest::PollInbox(argument) = request else {
        return Err(RpcStatus::Unsupported);
    };
    let app = argument.poll.app;
    if app != RtAppId::Chat {
        return Err(RpcStatus::BadArguments("invalid realtime app".into()));
    }
    let since = argument.poll.since;
    let timeout = match argument.poll.timeout_milliseconds {
        0 => Duration::from_secs(25),
        milliseconds => Duration::from_millis(milliseconds).min(Duration::from_secs(55)),
    };
    let pool = data.read_database.clone().ok_or(RpcStatus::Unsupported)?;
    let clock = Arc::clone(&data.clock);
    let principal = poll_read(&data.realtime_poll_reads, {
        let pool = pool.clone();
        let metrics = Arc::clone(&data.metrics);
        move || {
            let database = pool.lease(&metrics)?;
            let now = clock
                .now_micros()
                .map_err(|error| crate::status::unmapped(error.into(), "realtime_poll"))?;
            Principal::authenticate(&CertificateDer::from(certificate), &database, now)
                .map_err(|_| permission_denied())
        }
    })
    .await?;
    let auth_data = data.clone();
    let auth_principal = principal.clone();
    blocking(move || auth_data.authorize_principal(&auth_principal)).await?;
    let actor = crate::services::realtime::RealtimeService::actor(&principal, &data.host_id)?;
    let Some(_account_poll) = data
        .account_realtime_polls
        .try_acquire(principal.uid().to_vec())
    else {
        data.metrics
            .admission_rejected(crate::AdmissionRejection::AccountRealtimePolls);
        *outcome = PollOutcome::Refused;
        return Err(RpcStatus::RateLimited);
    };
    let listener = data.realtime.listener(&actor, app)?;
    let writer = data
        .writer
        .as_ref()
        .ok_or(RpcStatus::Unsupported)?
        .for_authenticated_request(principal.uid(), principal.device_id(), data.clock.clone());
    let service = data.realtime.clone();
    let deadline = tokio::time::Instant::now() + timeout;
    let mut active_poll = None;
    loop {
        let mut notified = Box::pin(Arc::clone(&listener).notified_owned());
        notified.as_mut().enable();
        let read_state = |pool: crate::read_pool::ReadPool,
                          actor: foks_server_db::RealtimeActor,
                          clock: Arc<dyn foks_server_db::Clock>,
                          service: crate::services::realtime::RealtimeService,
                          metrics: Arc<crate::ServerMetrics>| async move {
            poll_read(&data.realtime_poll_reads, move || {
                let database = pool.lease(&metrics)?;
                let snapshot = database
                    .snapshot()
                    .map_err(|error| crate::status::unmapped(error.into(), "realtime_poll"))?;
                let now = clock
                    .now_micros()
                    .map_err(|error| crate::status::unmapped(error.into(), "realtime_poll"))?;
                service.inbox_state(&actor, app, &snapshot, now, ReconcileSource::Poll)
            })
            .await
        };
        let mut state = read_state(
            pool.clone(),
            actor.clone(),
            Arc::clone(&data.clock),
            service.clone(),
            Arc::clone(&data.metrics),
        )
        .await?;
        if state.version > since {
            *outcome = PollOutcome::Bumped;
            return poll_result(sequence, state.version, true);
        }
        if state.needs_reconciliation() {
            let reconcile_actor = actor.clone();
            let reconcile_writer = writer.clone();
            let reconcile_service = service.clone();
            let reconcile_clock = Arc::clone(&data.clock);
            blocking(move || {
                reconcile_service.reconcile(
                    reconcile_actor,
                    app,
                    &reconcile_writer,
                    &reconcile_clock,
                    ReconcileSource::Poll,
                )
            })
            .await?;
            state = read_state(
                pool.clone(),
                actor.clone(),
                Arc::clone(&data.clock),
                service.clone(),
                Arc::clone(&data.metrics),
            )
            .await?;
        } else {
            service.clean_skip(ReconcileSource::Poll);
        }
        let head = state.version;
        if head > since {
            *outcome = PollOutcome::Bumped;
            return poll_result(sequence, head, true);
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            *outcome = PollOutcome::TimedOut;
            return poll_result(sequence, head, false);
        }
        active_poll.get_or_insert_with(|| {
            crate::ServerMetrics::realtime_poll_guard(Arc::clone(&data.metrics))
        });
        let hint = tokio::time::timeout(remaining.min(Duration::from_secs(1)), notified)
            .await
            .is_ok();
        data.metrics.realtime.update(|s| {
            if hint {
                s.hint_wakes += 1;
            } else {
                s.fallback_wakes += 1;
            }
        });
    }
}

fn poll_result(sequence: u64, inbox_version: u64, bumped: bool) -> Result<Vec<u8>, RpcStatus> {
    encode_success_response_at(
        &RtInboxPollResult {
            bumped,
            inbox_version,
        }
        .encoded()
        .map_err(|_| RpcStatus::TransactionRetry)?,
        sequence,
    )
    .map_err(|_| RpcStatus::TransactionRetry)
}

/// Runs a poll's database read on a blocking worker after waiting for one of
/// the `realtime_poll_reads` permits. Polls woken together then take their
/// turns for a reader instead of each leasing one at once, which would fail
/// polls and ordinary requests with status 1012 once the pool is exhausted.
async fn poll_read<T: Send + 'static>(
    permits: &Arc<tokio::sync::Semaphore>,
    operation: impl FnOnce() -> Result<T, RpcStatus> + Send + 'static,
) -> Result<T, RpcStatus> {
    let permit = Arc::clone(permits)
        .acquire_owned()
        .await
        .map_err(|_| RpcStatus::TransactionRetry)?;
    blocking(move || {
        let _permit = permit;
        operation()
    })
    .await
}

async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, RpcStatus> + Send + 'static,
) -> Result<T, RpcStatus> {
    let work = crate::net::memory::retain_work(operation);
    tokio::task::spawn_blocking(move || work.run())
        .await
        .map_err(|_| RpcStatus::TransactionRetry)?
        .into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::session::realtime_poll_read_permits;

    #[test]
    fn poll_reads_take_half_of_the_reader_pool_and_at_least_one_reader() {
        assert_eq!(realtime_poll_read_permits(32), 16);
        assert_eq!(realtime_poll_read_permits(3), 1);
        assert_eq!(realtime_poll_read_permits(1), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn waiting_poll_reads_leave_readers_for_ordinary_requests() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("poll.sqlite");
        drop(foks_server_db::Database::open(&path, foks_server_db::Config::default()).unwrap());
        let pool = crate::read_pool::ReadPool::new(
            crate::ReadDatabaseConfig {
                path,
                database: foks_server_db::Config::default(),
            },
            2,
        )
        .unwrap();
        let metrics = Arc::new(crate::ServerMetrics::default());
        let permits = Arc::new(tokio::sync::Semaphore::new(realtime_poll_read_permits(2)));
        let (held, holding) = tokio::sync::oneshot::channel();
        let (release, released) = std::sync::mpsc::channel::<()>();
        let first = tokio::spawn({
            let (pool, metrics, permits) = (pool.clone(), Arc::clone(&metrics), permits.clone());
            async move {
                poll_read(&permits, move || {
                    let _database = pool.lease(&metrics)?;
                    let _ = held.send(());
                    let _ = released.recv();
                    Ok(())
                })
                .await
            }
        });
        holding.await.unwrap();
        // A second poll read, as from a poll woken by the same hint, waits for
        // the permit rather than leasing the remaining reader.
        let second = tokio::spawn({
            let (pool, metrics, permits) = (pool.clone(), Arc::clone(&metrics), permits.clone());
            async move { poll_read(&permits, move || pool.lease(&metrics).map(drop)).await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!second.is_finished());
        drop(pool.lease(&metrics).unwrap());
        release.send(()).unwrap();
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
        assert_eq!(metrics.snapshot().reader_pool_exhausted, 0);
    }
}
