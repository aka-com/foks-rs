use super::super::{permission_denied, ServerData};
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
    let database = data.read_database()?;
    let result = data.realtime.dispatch(
        call.route.position,
        call.call.argument(),
        principal,
        &data.host_id,
        &database,
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
    let pool = data
        .read_database
        .clone()
        .ok_or(RpcStatus::TransactionRetry)?;
    let clock = Arc::clone(&data.clock);
    let principal = blocking({
        let pool = pool.clone();
        move || {
            let database = pool.checkout().map_err(|_| RpcStatus::TransactionRetry)?;
            let now = clock
                .now_micros()
                .map_err(|_| RpcStatus::TransactionRetry)?;
            Principal::authenticate(&CertificateDer::from(certificate), &database, now)
                .map_err(|_| permission_denied())
        }
    })
    .await?;
    let actor = crate::services::realtime::RealtimeService::actor(&principal, &data.host_id)?;
    let listener = data.realtime.listener(&actor, app)?;
    let writer = data.writer.clone().ok_or(RpcStatus::Unsupported)?;
    let service = data.realtime.clone();
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let mut notified = Box::pin(Arc::clone(&listener).notified_owned());
        notified.as_mut().enable();
        let read_head = |pool: crate::read_pool::ReadPool,
                         actor: foks_server_db::RealtimeActor,
                         clock: Arc<dyn foks_server_db::Clock>| async move {
            blocking(move || {
                let database = pool.checkout().map_err(|_| RpcStatus::TransactionRetry)?;
                crate::services::realtime::RealtimeService::inbox_head(
                    &actor, app, &database, &clock,
                )
            })
            .await
        };
        let head = read_head(pool.clone(), actor.clone(), Arc::clone(&data.clock)).await?;
        if head > since {
            return poll_result(sequence, head, true);
        }
        let reconcile_actor = actor.clone();
        let reconcile_writer = writer.clone();
        let reconcile_service = service.clone();
        let reconcile_clock = Arc::clone(&data.clock);
        blocking(move || {
            reconcile_service.reconcile(reconcile_actor, app, &reconcile_writer, &reconcile_clock)
        })
        .await?;
        let head = read_head(pool.clone(), actor.clone(), Arc::clone(&data.clock)).await?;
        if head > since {
            return poll_result(sequence, head, true);
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return poll_result(sequence, head, false);
        }
        let _active_poll = crate::ServerMetrics::realtime_poll_guard(Arc::clone(&data.metrics));
        if tokio::time::timeout(remaining, notified).await.is_err() {
            return poll_result(sequence, head, false);
        }
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

async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, RpcStatus> + Send + 'static,
) -> Result<T, RpcStatus> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|_| RpcStatus::TransactionRetry)?
}
