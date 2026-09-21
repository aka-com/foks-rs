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

/// The poll's three phases are timed into `timing` as each ends, so a poll
/// that fails in its last phase still reports the first two.
fn run_chat_poll(
    state_dir: &Path,
    store: TeamStoreRef,
    since: u64,
    timeout_milliseconds: u64,
    timeout: Duration,
    cancellation: CancellationToken,
    timing: &mut ResponseTiming,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let phase = std::time::Instant::now();
    let context = {
        let _admission = profile_work::coordinator().acquire_blocking(
            state_dir,
            profile_work::Scope::profile(&store.profile),
            timeout,
            &cancellation,
        )?;
        let registry = ProfileRegistry::open(state_dir)?;
        let session = ProfileSession::open_with_control(
            &registry,
            &store.profile,
            timeout,
            cancellation.clone(),
        )?;
        profile_work::with_control(timeout, cancellation.clone(), || {
            with_vault(state_dir, &session, |session, vault| {
                chat::prepare_poll(session, vault, &store)
            })
        })?
    };
    timing.prepare_ms = ResponseTiming::millis(phase.elapsed());
    let phase = std::time::Instant::now();
    // A long network poll owns no profile admission or checked-session lock.
    let reply = chat::poll(context, since, timeout_milliseconds)?;
    timing.wait_ms = ResponseTiming::millis(phase.elapsed());
    let phase = std::time::Instant::now();
    let (_, current_scope) = {
        let _admission = profile_work::coordinator().acquire_blocking(
            state_dir,
            profile_work::Scope::profile(&store.profile),
            timeout,
            &cancellation,
        )?;
        let registry = ProfileRegistry::open(state_dir)?;
        let session = ProfileSession::open_with_control(
            &registry,
            &store.profile,
            timeout,
            cancellation.clone(),
        )?;
        profile_work::with_control(timeout, cancellation, || {
            with_vault(state_dir, &session, |session, vault| {
                chat::resolve_scope(session, vault, &store)
            })
        })?
    };
    timing.rescope_ms = ResponseTiming::millis(phase.elapsed());
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
}
