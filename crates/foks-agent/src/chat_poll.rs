//! Dedicated, cancellable account poll admission and supervision.
use super::{
    chat, dispatch_error_response, profile_work, with_vault, write_response, CANCELLATION_GRACE,
    CHAT_POLL_TIMEOUT,
};
use foks_agent_proto::{ErrorCode, Operation, Request, Response, ResponseTiming, TeamStoreRef};
use foks_client_app::{CancellationToken, ProfileRegistry, ProfileSession};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Semaphore;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct ChatPollKey {
    profile: String,
    account: String,
}
impl From<&TeamStoreRef> for ChatPollKey {
    fn from(store: &TeamStoreRef) -> Self {
        Self {
            profile: store.profile.clone(),
            account: store.account_alias.clone(),
        }
    }
}
pub(super) struct ActiveChatPollGuard {
    pub(super) polls: Arc<Mutex<std::collections::HashSet<ChatPollKey>>>,
    pub(super) key: ChatPollKey,
}
impl Drop for ActiveChatPollGuard {
    fn drop(&mut self) {
        if let Ok(mut polls) = self.polls.lock() {
            polls.remove(&self.key);
        }
    }
}

#[cfg(unix)]
pub(super) async fn handle_chat_poll(
    stream: &mut tokio::net::UnixStream,
    state_dir: PathBuf,
    polling: Arc<Semaphore>,
    active: Arc<Mutex<std::collections::HashSet<ChatPollKey>>>,
    timeout: Duration,
    request: Request,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let Operation::Chat { store, action } = &request.operation else {
        return Err("chat poll dispatch received another operation".into());
    };
    if !action.validate() {
        write_response(
            stream,
            &Response::error(
                request.id,
                ErrorCode::InvalidRequest,
                "invalid chat poll request",
            ),
            timeout,
        )
        .await?;
        return Ok(false);
    }
    let foks_agent_proto::chat::ChatAction::PollInbox {
        since,
        timeout_milliseconds,
    } = action
    else {
        return Err("chat poll dispatch received another action".into());
    };
    let since =
        foks_agent_proto::chat::chat_sequence(since).ok_or("chat poll cursor is invalid")?;
    let poll_timeout = *timeout_milliseconds;
    let key = ChatPollKey::from(store);
    let inserted = active
        .lock()
        .map_err(|_| "chat poll registry is unavailable")?
        .insert(key.clone());
    if !inserted {
        write_response(
            stream,
            &Response::error(
                request.id,
                ErrorCode::Busy,
                "chat synchronization is already active for this account",
            ),
            timeout,
        )
        .await?;
        return Ok(false);
    }
    let guard = ActiveChatPollGuard { polls: active, key };
    let admission = tokio::time::timeout(timeout, polling.acquire_owned());
    tokio::pin!(admission);
    let admitted = loop {
        tokio::select! {
            result = &mut admission => break result,
            readable = stream.readable() => {
                readable?;
                let mut byte = [0];
                match stream.try_read(&mut byte) {
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                    _ => return Ok(true),
                }
            }
        }
    };
    let permit = match admitted {
        Ok(Ok(permit)) => permit,
        Ok(Err(_)) => return Err("agent chat poll pool closed".into()),
        Err(_) => {
            write_response(
                stream,
                &Response::error(
                    request.id,
                    ErrorCode::Busy,
                    "agent chat poll pool is saturated",
                ),
                timeout,
            )
            .await?;
            return Ok(false);
        }
    };
    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.clone();
    let store = store.clone();
    let id = request.id;
    let mut task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let _guard = guard;
        let mut timing = ResponseTiming::default();
        match foks_keystore::without_user_interaction(|| {
            run_chat_poll(
                &state_dir,
                store,
                since,
                poll_timeout,
                timeout,
                worker_cancellation,
                &mut timing,
            )
        }) {
            Ok(value) => Response::success(id, value),
            Err(error) => dispatch_error_response(id, error.as_ref()),
        }
        .with_timing(timing)
    });
    let deadline = tokio::time::Instant::now() + CHAT_POLL_TIMEOUT;
    let (response, close) = loop {
        tokio::select! {
            result = &mut task => {
                let response = result.unwrap_or_else(|error| Response::error(
                    id,
                    ErrorCode::OperationFailed,
                    format!("agent chat poll worker failed: {error}"),
                ));
                break (response, false);
            }
            _ = tokio::time::sleep_until(deadline) => {
                cancellation.cancel();
                let _ = tokio::time::timeout(CANCELLATION_GRACE, &mut task).await;
                break (Response::error(
                    id,
                    ErrorCode::DeadlineExceeded,
                    "chat poll operation deadline exceeded",
                ), true);
            }
            result = stream.readable() => {
                result?;
                let mut byte = [0; 1];
                match stream.try_read(&mut byte) {
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => continue,
                    Ok(_) | Err(_) => {
                        cancellation.cancel();
                        let _ = tokio::time::timeout(CANCELLATION_GRACE, &mut task).await;
                        return Ok(true);
                    }
                }
            }
        }
    };
    write_response(stream, &response, timeout).await?;
    Ok(close)
}

/// Admission, session opens and work are timed into `timing` separately as
/// each ends, before propagating errors, so a failed poll reports both the
/// failing phase and the earlier ones. A poll that waited behind other work
/// for the profile does not report that wait as its own cost. The spans are disjoint and
/// together account for the worker's time: admission into `queue_ms`,
/// opening the profile's registry and session into `session_ms`, then
/// `prepare_ms`, the network `wait_ms` that holds nothing, and `rescope_ms`.
fn run_chat_poll(
    state_dir: &Path,
    store: TeamStoreRef,
    since: u64,
    timeout_milliseconds: u64,
    timeout: Duration,
    cancellation: CancellationToken,
    timing: &mut ResponseTiming,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let mut admission = Duration::ZERO;
    let mut sessions = Duration::ZERO;
    let context = {
        let phase = std::time::Instant::now();
        let permit = profile_work::coordinator().acquire_blocking(
            state_dir,
            profile_work::Scope::profile(&store.profile),
            timeout,
            &cancellation,
        );
        admission += phase.elapsed();
        timing.queue_ms = ResponseTiming::millis(admission);
        let permit = permit?;
        if timing.waited_behind.is_none() {
            timing.waited_behind = permit.waited_behind.map(str::to_owned);
        }
        let phase = std::time::Instant::now();
        let session = (|| -> Result<_, Box<dyn std::error::Error>> {
            let registry = ProfileRegistry::open(state_dir)?;
            let session = ProfileSession::open_with_control(
                &registry,
                &store.profile,
                timeout,
                cancellation.clone(),
            )?;
            Ok((registry, session))
        })();
        sessions += phase.elapsed();
        timing.session_ms = ResponseTiming::millis(sessions);
        let (_registry, session) = session?;
        let phase = std::time::Instant::now();
        let context = profile_work::with_control(timeout, cancellation.clone(), || {
            with_vault(state_dir, &session, |session, vault| {
                chat::prepare_poll(session, vault, &store)
            })
        });
        timing.prepare_ms = ResponseTiming::millis(phase.elapsed());
        context?
    };
    let phase = std::time::Instant::now();
    // A long network poll owns no profile admission or checked-session lock.
    let reply = chat::poll(context, since, timeout_milliseconds);
    timing.wait_ms = ResponseTiming::millis(phase.elapsed());
    let reply = reply?;
    let (_, current_scope) = {
        let phase = std::time::Instant::now();
        let permit = profile_work::coordinator().acquire_blocking(
            state_dir,
            profile_work::Scope::profile(&store.profile),
            timeout,
            &cancellation,
        );
        admission += phase.elapsed();
        timing.queue_ms = ResponseTiming::millis(admission);
        let permit = permit?;
        if timing.waited_behind.is_none() {
            timing.waited_behind = permit.waited_behind.map(str::to_owned);
        }
        let phase = std::time::Instant::now();
        let session = (|| -> Result<_, Box<dyn std::error::Error>> {
            let registry = ProfileRegistry::open(state_dir)?;
            let session = ProfileSession::open_with_control(
                &registry,
                &store.profile,
                timeout,
                cancellation.clone(),
            )?;
            Ok((registry, session))
        })();
        sessions += phase.elapsed();
        timing.session_ms = ResponseTiming::millis(sessions);
        let (_registry, session) = session?;
        let phase = std::time::Instant::now();
        let scope = profile_work::with_control(timeout, cancellation, || {
            with_vault(state_dir, &session, |session, vault| {
                chat::resolve_scope(session, vault, &store)
            })
        });
        timing.rescope_ms = ResponseTiming::millis(phase.elapsed());
        scope?
    };
    if current_scope != reply.scope {
        return Err(foks_client::Error::ChatIntegrity("chat poll scope changed").into());
    }
    Ok(serde_json::to_value(reply)?)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use foks_agent_proto::ResponseResult;

    #[tokio::test]
    async fn cancellation_during_admission_releases_account() {
        let (mut server, client) = tokio::net::UnixStream::pair().unwrap();
        let polls = Arc::new(Mutex::new(std::collections::HashSet::new()));
        let capacity = Arc::new(Semaphore::new(0));
        let request = Request::new(
            1,
            Operation::Chat {
                store: TeamStoreRef {
                    profile: "test".into(),
                    account_alias: "me".into(),
                    team_alias: "team".into(),
                    team_id: format!("03{}", "ab".repeat(32)),
                },
                action: foks_agent_proto::chat::ChatAction::PollInbox {
                    since: "0".into(),
                    timeout_milliseconds: 25_000,
                },
            },
        );
        drop(client);
        let closed = tokio::time::timeout(
            Duration::from_millis(100),
            handle_chat_poll(
                &mut server,
                PathBuf::new(),
                capacity,
                polls.clone(),
                Duration::from_secs(15),
                request,
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(closed);
        assert!(polls.lock().unwrap().is_empty());
    }

    #[test]
    fn malformed_inbox_is_fatal_at_agent_boundary() {
        let response = dispatch_error_response(
            1,
            &foks_client_db::Error::InvalidChatInbox("duplicate version"),
        );
        assert!(matches!(
            response.result,
            ResponseResult::Error {
                code: ErrorCode::ChatIntegrity,
                ..
            }
        ));
    }

    /// A poll that waits for the profile must not report that wait as the
    /// cost of preparing itself.
    #[test]
    fn a_poll_reports_its_admission_wait_apart_from_its_own_work() {
        let root = tempfile::tempdir().unwrap();
        let held = profile_work::coordinator()
            .acquire_blocking(
                root.path(),
                profile_work::Scope::profile("test"),
                Duration::from_secs(10),
                &CancellationToken::new(),
            )
            .unwrap();
        let state_dir = root.path().to_owned();
        let worker = std::thread::spawn(move || {
            let mut timing = ResponseTiming::default();
            let result = run_chat_poll(
                &state_dir,
                TeamStoreRef {
                    profile: "test".into(),
                    account_alias: "me".into(),
                    team_alias: "team".into(),
                    team_id: format!("03{}", "ab".repeat(32)),
                },
                0,
                1,
                Duration::from_secs(10),
                CancellationToken::new(),
                &mut timing,
            );
            (result.is_err(), timing)
        });
        std::thread::sleep(Duration::from_millis(80));
        drop(held);
        let (failed, timing) = worker.join().unwrap();
        // There is no profile to open under this root, so the poll fails as
        // soon as it is admitted and never enters a phase of its own.
        assert!(failed);
        assert!(timing.queue_ms >= 50, "admission {}ms", timing.queue_ms);
        assert_eq!(timing.waited_behind.as_deref(), Some("profile"));
        assert_eq!(timing.prepare_ms, 0);
        assert_eq!(timing.session_ms, 0);
        assert_eq!(timing.wait_ms, 0);
        assert_eq!(timing.rescope_ms, 0);
    }
}
