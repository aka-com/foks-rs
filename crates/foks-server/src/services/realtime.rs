use crate::{auth::Principal, metrics::realtime::ReconcileSource, WriterHandle};
use foks_proto::RealtimeWire;
use foks_rpc::{RealtimeRequest, RpcStatus};
use foks_server_db::{Error as DbError, RealtimeActor, RealtimeCommit, RealtimeWakeTarget};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, Weak},
};

/// Best-effort, nonblocking wake hints after durable commit. Implementations
/// must not panic or perform blocking delivery. Inbox versions remain the source
/// of truth: missed/coalesced hints must never affect receipts or recovery.
pub(crate) trait RealtimeNotifier: Send + Sync {
    fn notify(&self, targets: &[RealtimeWakeTarget]);
}
#[derive(Clone, Eq, Hash, PartialEq)]
struct WakeKey {
    host: Vec<u8>,
    uid: Vec<u8>,
    app: u64,
}
impl From<&RealtimeWakeTarget> for WakeKey {
    fn from(target: &RealtimeWakeTarget) -> Self {
        Self {
            host: target.host.clone(),
            uid: target.uid.clone(),
            app: target.app as u64,
        }
    }
}
#[derive(Default)]
struct InboxHub {
    waiters: Mutex<HashMap<WakeKey, Weak<tokio::sync::Notify>>>,
}
impl InboxHub {
    fn listener(&self, target: &RealtimeWakeTarget) -> Result<Arc<tokio::sync::Notify>, RpcStatus> {
        let mut waiters = self
            .waiters
            .lock()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        let key = WakeKey::from(target);
        if let Some(listener) = waiters.get(&key).and_then(Weak::upgrade) {
            return Ok(listener);
        }
        let listener = Arc::new(tokio::sync::Notify::new());
        waiters.insert(key, Arc::downgrade(&listener));
        Ok(listener)
    }

    fn notify_all(&self) {
        let Ok(mut waiters) = self.waiters.lock() else {
            return;
        };
        waiters.retain(|_, listener| {
            if let Some(listener) = listener.upgrade() {
                listener.notify_waiters();
                true
            } else {
                false
            }
        });
    }
}
impl RealtimeNotifier for InboxHub {
    fn notify(&self, targets: &[RealtimeWakeTarget]) {
        let Ok(mut waiters) = self.waiters.lock() else {
            return;
        };
        let targets = targets.iter().map(WakeKey::from).collect::<HashSet<_>>();
        waiters.retain(|key, listener| {
            if let Some(listener) = listener.upgrade() {
                if targets.contains(key) {
                    listener.notify_waiters();
                }
                true
            } else {
                false
            }
        });
    }
}
#[derive(Clone)]
pub(crate) struct RealtimeService {
    notifier: Arc<dyn RealtimeNotifier>,
    inbox_hub: Arc<InboxHub>,
    metrics: Arc<crate::ServerMetrics>,
}
impl Default for RealtimeService {
    fn default() -> Self {
        Self::new(Arc::new(crate::ServerMetrics::default()))
    }
}
impl RealtimeService {
    pub(crate) fn new(metrics: Arc<crate::ServerMetrics>) -> Self {
        let inbox_hub = Arc::new(InboxHub::default());
        Self {
            notifier: inbox_hub.clone(),
            inbox_hub,
            metrics,
        }
    }
}
fn after_commit<T>(
    result: crate::Result<RealtimeCommit<T>>,
    notifier: &dyn RealtimeNotifier,
) -> Result<T, RpcStatus> {
    let committed = result.map_err(write_error)?;
    if !committed.wake.is_empty() {
        notifier.notify(&committed.wake);
    }
    Ok(committed.value)
}

pub(crate) enum Response {
    Void,
    Data(Vec<u8>),
}
impl RealtimeService {
    pub(crate) fn actor(principal: &Principal, host: &[u8]) -> Result<RealtimeActor, RpcStatus> {
        principal.require_interactive_device()?;
        Ok(RealtimeActor {
            host: host.to_vec(),
            uid: principal.uid().to_vec(),
            credential: principal.device_id().to_vec(),
            certificate_expires_at: principal.certificate_expires_at(),
        })
    }

    pub(crate) fn membership_changed(&self) {
        self.inbox_hub.notify_all();
    }

    pub(crate) fn listener(
        &self,
        actor: &RealtimeActor,
        app: foks_proto::RtAppId,
    ) -> Result<Arc<tokio::sync::Notify>, RpcStatus> {
        self.inbox_hub.listener(&RealtimeWakeTarget {
            host: actor.host.clone(),
            uid: actor.uid.clone(),
            app,
        })
    }

    pub(crate) fn reconcile(
        &self,
        actor: RealtimeActor,
        app: foks_proto::RtAppId,
        writer: &WriterHandle,
        clock: &Arc<dyn foks_server_db::Clock>,
        source: ReconcileSource,
    ) -> Result<(), RpcStatus> {
        self.metrics.realtime.source(source, |s| s.submissions += 1);
        let clock = Arc::clone(clock);
        let result = writer.call_observed(Some((Arc::clone(&self.metrics), source)), move |db| {
            Ok(db.rt_reconcile_inbox(&actor, app, clock.now_micros()?)?)
        });
        if matches!(&result, Err(crate::Error::WriterQueue)) {
            self.metrics
                .realtime
                .source(source, |s| s.submission_failures += 1);
        }
        let report = after_commit(result, self.notifier.as_ref())?;
        self.metrics.realtime.report(source, report);
        Ok(())
    }

    pub(crate) fn inbox_state(
        &self,
        actor: &RealtimeActor,
        app: foks_proto::RtAppId,
        snapshot: &foks_server_db::ReadSnapshot<'_>,
        now: u64,
        source: ReconcileSource,
    ) -> Result<foks_server_db::RealtimeInboxState, RpcStatus> {
        let state = snapshot.rt_inbox_state(actor, app, now).map_err(db_error)?;
        self.metrics.realtime.source(source, |s| s.state_reads += 1);
        Ok(state)
    }

    pub(crate) fn clean_skip(&self, source: ReconcileSource) {
        self.metrics.realtime.source(source, |s| s.clean_skips += 1);
    }

    #[allow(clippy::too_many_arguments)] // Authenticated request and host execution context.
    pub(crate) fn dispatch(
        &self,
        position: u64,
        argument: &[u8],
        principal: &Principal,
        host: &[u8],
        checkout: impl Fn() -> Result<crate::read_pool::ReadLease, RpcStatus>,
        writer: &WriterHandle,
        clock: &Arc<dyn foks_server_db::Clock>,
    ) -> Result<Response, RpcStatus> {
        let request = RealtimeRequest::decode_argument(position, argument)
            .map_err(|_| RpcStatus::BadArguments("invalid realtime request".into()))?;
        let actor = Self::actor(principal, host)?;
        match request {
            RealtimeRequest::CreateChannel(arg) => {
                after_commit(
                    writer.call_with_current_time(Arc::clone(clock), move |db, now| {
                        Ok(db.rt_create_channel(&actor, &arg, now)?)
                    }),
                    self.notifier.as_ref(),
                )?;
                Ok(Response::Void)
            }
            RealtimeRequest::Send(arg) => {
                let receipt = after_commit(
                    writer.call_with_current_time(Arc::clone(clock), move |db, now| {
                        Ok(db.rt_send(&actor, &arg, now)?)
                    }),
                    self.notifier.as_ref(),
                )?;
                Ok(Response::Data(
                    receipt.encoded().map_err(|_| RpcStatus::TransactionRetry)?,
                ))
            }
            RealtimeRequest::ReadThrough(arg) => {
                after_commit(
                    writer.call_with_current_time(Arc::clone(clock), move |db, now| {
                        Ok(db.rt_read_through(&actor, &arg, now)?)
                    }),
                    self.notifier.as_ref(),
                )?;
                Ok(Response::Void)
            }
            RealtimeRequest::GetChangedThreads(arg) => {
                {
                    let reader = checkout()?;
                    let snapshot = reader.snapshot().map_err(db_error)?;
                    let now = clock.now_micros().map_err(db_error)?;
                    let state = self.inbox_state(
                        &actor,
                        arg.query.app,
                        &snapshot,
                        now,
                        ReconcileSource::Delta,
                    )?;
                    if !state.needs_reconciliation() {
                        self.clean_skip(ReconcileSource::Delta);
                        return Ok(Response::Data(
                            snapshot
                                .rt_changed_threads(&actor, &arg, now)
                                .map_err(db_error)?
                                .encoded()
                                .map_err(|_| RpcStatus::TransactionRetry)?,
                        ));
                    }
                } // Return the entire lease before waiting for the writer.
                self.reconcile(
                    actor.clone(),
                    arg.query.app,
                    writer,
                    clock,
                    ReconcileSource::Delta,
                )?;
                let reader = checkout()?;
                let snapshot = reader.snapshot().map_err(db_error)?;
                let now = clock.now_micros().map_err(db_error)?;
                Ok(Response::Data(
                    snapshot
                        .rt_changed_threads(&actor, &arg, now)
                        .map_err(db_error)?
                        .encoded()
                        .map_err(|_| RpcStatus::TransactionRetry)?,
                ))
            }
            RealtimeRequest::PollInbox(_) => Err(RpcStatus::Unsupported),
            request => {
                let reader = checkout()?;
                let snapshot = reader.snapshot().map_err(db_error)?;
                let now = clock.now_micros().map_err(db_error)?;
                let data = match request {
                    RealtimeRequest::ChatCapabilities(arg) => {
                        if arg.host.entity().as_bytes() != host {
                            return Err(RpcStatus::PermissionDenied("wrong realtime host".into()));
                        }
                        snapshot.rt_check_actor(&actor, now).map_err(db_error)?;
                        // Discovery is implemented. Extended channel behavior is
                        // enabled only as its end-to-end stages are completed.
                        foks_proto::RtChatCapabilities::basic_only(arg.host).encoded()
                    }
                    RealtimeRequest::SelectVhost(arg) => {
                        if arg.host.entity().as_bytes() != host {
                            return Err(RpcStatus::PermissionDenied("wrong realtime host".into()));
                        }
                        snapshot.rt_check_actor(&actor, now).map_err(db_error)?;
                        return Ok(Response::Void);
                    }
                    RealtimeRequest::ListChannels(arg) => snapshot
                        .rt_list_channels(&actor, &arg, now)
                        .map_err(db_error)?
                        .encoded(),
                    RealtimeRequest::GetInboxVersion(arg) => snapshot
                        .rt_inbox_version(&actor, arg.key.app, now)
                        .map_err(db_error)?
                        .encoded(),
                    RealtimeRequest::Recents(arg) => snapshot
                        .rt_recents(&actor, &arg, now)
                        .map_err(db_error)?
                        .encoded(),
                    RealtimeRequest::GetThread(arg) => snapshot
                        .rt_thread(&actor, &arg, now)
                        .map_err(db_error)?
                        .encoded(),
                    _ => unreachable!(),
                }
                .map_err(|_| RpcStatus::TransactionRetry)?;
                Ok(Response::Data(data))
            }
        }
    }
}
fn write_error(error: crate::Error) -> RpcStatus {
    match error {
        crate::Error::Database(e) => db_error(e),
        _ => RpcStatus::TransactionRetry,
    }
}
fn db_error(error: DbError) -> RpcStatus {
    match error {
        DbError::AuthorizationChanged => {
            RpcStatus::PermissionDenied("realtime access denied".into())
        }
        DbError::RtRace => RpcStatus::RtRace("realtime state changed; refresh before retry".into()),
        DbError::RtUnsupportedFormat => RpcStatus::Unsupported,
        DbError::RtMessageOrder => {
            RpcStatus::RtMessageOrder("realtime predecessor mismatch".into())
        }
        DbError::Duplicate("realtime channel") => RpcStatus::RtChannelExists,
        DbError::NotFound(_) => RpcStatus::RtNotFound("realtime object not found".into()),
        DbError::ReceiptConflict => {
            RpcStatus::RtRace("message ID conflicts with an existing message".into())
        }
        DbError::Invalid(_) | DbError::IntegerRange => {
            RpcStatus::BadArguments("invalid realtime value".into())
        }
        DbError::Capacity(_) => RpcStatus::Realtime("realtime service limit exceeded".into()),
        _ => RpcStatus::TransactionRetry,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    #[derive(Default)]
    struct RecordingNotifier(Mutex<Vec<RealtimeWakeTarget>>);
    impl RealtimeNotifier for RecordingNotifier {
        fn notify(&self, targets: &[RealtimeWakeTarget]) {
            self.0.lock().unwrap().extend_from_slice(targets);
        }
    }
    #[test]
    fn only_successful_new_commits_deliver_wake_hints() {
        let notifier = RecordingNotifier::default();
        let target = RealtimeWakeTarget {
            host: vec![2],
            uid: vec![1],
            app: foks_proto::RtAppId::Chat,
        };
        let value = after_commit(
            Ok(RealtimeCommit {
                value: 42,
                wake: vec![target.clone()],
            }),
            &notifier,
        )
        .unwrap();
        assert_eq!(value, 42);
        assert_eq!(*notifier.0.lock().unwrap(), vec![target.clone()]);
        // Exact retries have no new inbox version and no wake targets.
        assert_eq!(
            after_commit(
                Ok(RealtimeCommit {
                    value: 42,
                    wake: vec![]
                }),
                &notifier
            )
            .unwrap(),
            42
        );
        assert!(after_commit::<()>(Err(DbError::AuthorizationChanged.into()), &notifier).is_err());
        assert_eq!(*notifier.0.lock().unwrap(), vec![target]);
    }

    #[tokio::test]
    async fn listener_registered_before_commit_observes_the_wake() {
        let service = RealtimeService::default();
        let target = RealtimeWakeTarget {
            host: vec![2],
            uid: vec![1],
            app: foks_proto::RtAppId::Chat,
        };
        let listener = service.inbox_hub.listener(&target).unwrap();
        let mut notified = Box::pin(listener.notified());
        notified.as_mut().enable();
        service.notifier.notify(std::slice::from_ref(&target));
        tokio::time::timeout(std::time::Duration::from_secs(1), notified)
            .await
            .unwrap();
    }
}
