use crate::{auth::Principal, WriterHandle};
use foks_proto::RealtimeWire;
use foks_rpc::{RealtimeRequest, RpcStatus};
use foks_server_db::{Error as DbError, RealtimeActor, RealtimeCommit, RealtimeWakeTarget};
use std::sync::Arc;

/// Best-effort, nonblocking wake hints after durable commit. Implementations
/// must not panic or perform blocking delivery. Inbox versions remain the source
/// of truth: missed/coalesced hints must never affect receipts or recovery.
pub(crate) trait RealtimeNotifier: Send + Sync {
    fn notify(&self, targets: &[RealtimeWakeTarget]);
}
pub(crate) struct RealtimeService {
    notifier: Box<dyn RealtimeNotifier>,
}
impl Default for RealtimeService {
    fn default() -> Self {
        Self {
            notifier: Box::new(NoopNotifier),
        }
    }
}
#[derive(Default)]
struct NoopNotifier;
impl RealtimeNotifier for NoopNotifier {
    fn notify(&self, _targets: &[RealtimeWakeTarget]) {}
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
    #[allow(clippy::too_many_arguments)] // Authenticated request and host execution context.
    pub(crate) fn dispatch(
        &self,
        position: u64,
        argument: &[u8],
        principal: &Principal,
        host: &[u8],
        reader: &foks_server_db::ReadDatabase,
        writer: &WriterHandle,
        clock: &Arc<dyn foks_server_db::Clock>,
    ) -> Result<Response, RpcStatus> {
        principal.require_ordinary_device()?;
        let request = RealtimeRequest::decode_argument(position, argument)
            .map_err(|_| RpcStatus::BadArguments("invalid realtime request".into()))?;
        let actor = RealtimeActor {
            host: host.to_vec(),
            uid: principal.uid().to_vec(),
            credential: principal.device_id().to_vec(),
            certificate_expires_at: principal.certificate_expires_at(),
        };
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
            request => {
                let snapshot = reader.snapshot().map_err(db_error)?;
                let now = clock.now_micros().map_err(db_error)?;
                let data = match request {
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
}
