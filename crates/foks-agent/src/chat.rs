//! Foreground chat dispatch. All operations run inside one checked profile session.
use foks_agent_proto::{chat::*, SecretString, TeamStoreRef};
use foks_client_app::{AccountVault, CheckedProfileSession, ClientCredentials};
use foks_proto::{RealtimeWire, RtChannelId, RtChannelTier, RtSendResult};
use std::path::Path;
use zeroize::Zeroizing;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn id(value: &str) -> Result<[u8; 16]> {
    if !valid_chat_id(value) {
        return Err(Box::new(super::AgentRequestError("invalid chat identity")));
    }
    let mut id = [0; 16];
    for (i, byte) in id.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16)?;
    }
    Ok(id)
}
fn role_label(role: foks_proto::Role) -> String {
    match role {
        foks_proto::Role::OWNER => "Owner".into(),
        foks_proto::Role::ADMIN => "Admin".into(),
        _ => format!("Member ({})", role.visibility().unwrap_or(0)),
    }
}
/// Bounds a channel's name or description to the characters the chat limits
/// admit. This device checks its own before sealing them, but another client
/// can seal a longer one, and nothing on the server can see the plaintext to
/// refuse it. The desktop holds every reply to the same limits and treats one
/// outside them as it treats any reply outside its contract, parking the
/// whole account's chat, so a peer's over-long name is presented to what
/// fits, with the cut marked.
fn bounded_chars(text: &str, maximum: usize) -> String {
    if text.chars().count() <= maximum {
        return text.to_owned();
    }
    let kept: String = text.chars().take(maximum.saturating_sub(1)).collect();
    format!("{kept}…")
}
fn channel(value: foks_client::ChatChannel) -> ChatChannel {
    ChatChannel {
        id: hex(&value.metadata.id.0),
        name: SecretString::new(&bounded_chars(value.name.0.as_str(), CHAT_NAME_MAX_CHARS)),
        description: value.description.map(|description| {
            SecretString::new(&bounded_chars(
                description.0.as_str(),
                CHAT_DESCRIPTION_MAX_CHARS,
            ))
        }),
        admin: value.metadata.tier == RtChannelTier::Admin,
        readable: !value.metadata.unreadable,
        writable: value.writable,
        read_role: role_label(value.metadata.roles.read),
        write_role: role_label(value.metadata.roles.write),
    }
}
fn preview_text(text: &str) -> String {
    let text = text.split(['\r', '\n']).next().unwrap_or_default();
    if text.len() <= CHAT_SNIPPET_BYTES {
        return text.to_owned();
    }
    let mut end = CHAT_SNIPPET_BYTES.saturating_sub('…'.len_utf8());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}
fn preview(value: foks_client::ChatPreview) -> ChatPreview {
    ChatPreview {
        sender: value.sender.map(|sender| hex(sender.entity().as_bytes())),
        send_time: value.send_time.to_string(),
        insert_time: value.insert_time.to_string(),
        content: match value.content {
            foks_client::ChatPreviewContent::Text(text) => ChatContent::Text {
                text: SecretString::new(preview_text(&text)),
            },
            foks_client::ChatPreviewContent::Unsupported(_) => ChatContent::Unsupported,
        },
    }
}
fn inbox(
    value: foks_client::ChatInbox,
    expected: &foks_client_db::ChatScope,
) -> Result<ChatResult> {
    if value.host.entity().as_bytes() != expected.host
        || value.actor.entity().as_bytes() != expected.uid
        || value.team.entity().as_bytes() != expected.team
        || value.conversations.len() > CHAT_INBOX_ROWS
    {
        return Err(foks_client::Error::ChatIntegrity("inbox response scope changed").into());
    }
    Ok(ChatResult::Inbox {
        channels: value.channels.into_iter().map(channel).collect(),
        read_retry_pending: value.read_retry_pending,
        previews_incomplete: value.previews_incomplete,
        blocked_channels: value.blocked_channels.iter().map(|id| hex(&id.0)).collect(),
        cursor: value.cursor.to_string(),
        head: value.head.to_string(),
        degraded: value.degraded,
        conversations: value
            .conversations
            .into_iter()
            .map(|conversation| ChatConversation {
                channel: channel(conversation.channel),
                inbox_version: conversation.inbox_version.to_string(),
                read_through: conversation.read_through.to_string(),
                pending_read: conversation.pending_read.map(|value| value.to_string()),
                unread: conversation.unread.to_string(),
                hidden: conversation.hidden,
                muted: conversation.muted,
                preview: conversation.preview.map(preview),
            })
            .collect(),
    })
}
pub(super) struct PollContext {
    scope: ChatScope,
    connection: foks_client::RealtimeConnection,
}

pub(super) fn resolve_scope(
    session: &CheckedProfileSession<'_>,
    vault: &mut AccountVault<'_>,
    store: &TeamStoreRef,
) -> Result<(foks_client_db::ChatScope, ChatScope)> {
    let resolved = session.chat_scope(
        &store.account_alias,
        &store.team_alias,
        &store.team_id,
        vault,
    )?;
    let scope = ChatScope {
        store: store.clone(),
        host: hex(&resolved.host),
        actor: hex(&resolved.uid),
    };
    Ok((resolved, scope))
}

pub(super) fn prepare_poll(
    session: &CheckedProfileSession<'_>,
    vault: &mut AccountVault<'_>,
    store: &TeamStoreRef,
) -> Result<PollContext> {
    let (_, scope) = resolve_scope(session, vault, store)?;
    Ok(PollContext {
        scope,
        connection: session.chat_poll_connection(&store.team_alias, vault)?,
    })
}

pub(super) fn poll(
    mut context: PollContext,
    since: u64,
    timeout_milliseconds: u64,
) -> Result<ChatReply> {
    let response = context
        .connection
        .call(&foks_rpc::RealtimeRequest::PollInbox(
            foks_proto::RtPollInboxArgument {
                poll: foks_proto::RtPollInbox {
                    app: foks_proto::RtAppId::Chat,
                    since,
                    timeout_milliseconds,
                },
            },
        ))?;
    let foks_rpc::RealtimeResponse::PollResult(result) = response else {
        return Err(foks_client::Error::ChatIntegrity("unexpected inbox poll response").into());
    };
    let result = foks_client::ChatPollResult::checked(result.bumped, result.inbox_version, since)?;
    Ok(ChatReply {
        scope: context.scope,
        result: ChatResult::Poll {
            bumped: result.bumped,
            inbox_version: result.inbox_version.to_string(),
        },
    })
}

fn operation(op: foks_client_db::ChatOperation) -> Result<ChatOperation> {
    use foks_client_db::{ChatOperationKind as K, ChatOperationState as S};
    let sequence = if op.kind == K::Send {
        op.receipt
            .as_deref()
            .map(RtSendResult::decode)
            .transpose()?
            .map(|r| r.sequence.to_string())
    } else {
        None
    };
    Ok(ChatOperation {
        id: hex(&op.id),
        channel: hex(&op.scope.channel),
        kind: match op.kind {
            K::Create => ChatOperationKind::CreateChannel,
            K::Send => ChatOperationKind::SendMessage,
        },
        state: match op.state {
            S::Prepared => ChatState::Prepared,
            S::Uncertain => ChatState::Uncertain,
            S::Confirmed => ChatState::Confirmed,
            S::Rejected => ChatState::Rejected,
            S::Cancelled => ChatState::Cancelled,
        },
        receipt: if op.state == S::Confirmed {
            Some(match op.kind {
                K::Create => ChatReceipt::ChannelCreated,
                K::Send => ChatReceipt::MessageSent {
                    sequence: sequence.ok_or(foks_client::Error::ChatIntegrity(
                        "confirmed send has no receipt",
                    ))?,
                },
            })
        } else {
            None
        },
        rejection_code: op.rejection_code,
    })
}
fn submission_input(action: &ChatAction) -> Result<Zeroizing<Vec<u8>>> {
    let canonical;
    let action = match action {
        ChatAction::SubmitMessage {
            submission,
            channel,
            text,
        } => {
            canonical = ChatAction::PrepareMessage {
                submission: submission.clone(),
                channel: channel.clone(),
                text: text.clone(),
            };
            &canonical
        }
        action => action,
    };
    Ok(Zeroizing::new(serde_json::to_vec(action)?))
}

/// How many decrypted inbox previews the agent keeps. Each entry holds the
/// rendered snippet rather than the whole message body, so the cache is
/// bounded by entries times [`CHAT_SNIPPET_BYTES`] rather than by whatever
/// the longest cached message happens to be. The key pins the reader, the
/// message and the key generation, so a stale entry is never served: it is
/// only re-fetched.
const PREVIEW_CACHE_ENTRIES: usize = 4096;

/// Decrypted inbox snippets held for the life of the agent process, so an
/// idle team's repeated sync fetches no preview it already has. The agent
/// already holds decrypted catalog listings and loaded bot material in
/// process memory; concealing plaintext while the desktop is locked is not
/// something this process does today. Nothing here is ever persisted, and
/// the whole cache is bounded and dropped with the process.
#[derive(Default)]
struct PreviewCache {
    entries: std::collections::VecDeque<(foks_client::ChatPreviewKey, foks_client::ChatPreview)>,
}

/// The preview as the reply would render it. Truncating on the way in keeps
/// a long message body out of the cache; [`preview_text`] is idempotent, so
/// rendering a cached entry produces exactly what rendering the fetched one
/// would have.
fn bounded_preview(preview: foks_client::ChatPreview) -> foks_client::ChatPreview {
    match preview.content {
        foks_client::ChatPreviewContent::Text(text) => foks_client::ChatPreview {
            content: foks_client::ChatPreviewContent::Text(Zeroizing::new(preview_text(&text))),
            ..preview
        },
        foks_client::ChatPreviewContent::Unsupported(_) => preview,
    }
}

impl PreviewCache {
    fn get(&mut self, key: &foks_client::ChatPreviewKey) -> Option<foks_client::ChatPreview> {
        self.entries
            .iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, preview)| preview.clone())
    }

    fn put(&mut self, key: foks_client::ChatPreviewKey, preview: foks_client::ChatPreview) {
        let preview = bounded_preview(preview);
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|(candidate, _)| *candidate == key)
        {
            entry.1 = preview;
            return;
        }
        // Oldest first: a key names one message under one key generation, so
        // an evicted entry is simply fetched again.
        while self.entries.len() >= PREVIEW_CACHE_ENTRIES {
            self.entries.pop_front();
        }
        self.entries.push_back((key, preview));
    }
}

fn preview_cache() -> &'static std::sync::Mutex<PreviewCache> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<PreviewCache>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(PreviewCache::default()))
}

/// The process cache as one sync sees it. The lock is taken for each lookup
/// and each insertion, never across the sync's network work, so two
/// profiles syncing at once do not wait on each other. A poisoned lock is
/// not a reason to refuse a sync: it caches nothing.
struct SharedPreviewCache;

impl foks_client::ChatPreviewCache for SharedPreviewCache {
    fn get(&mut self, key: &foks_client::ChatPreviewKey) -> Option<foks_client::ChatPreview> {
        preview_cache().lock().ok()?.get(key)
    }

    fn put(&mut self, key: foks_client::ChatPreviewKey, preview: foks_client::ChatPreview) {
        if let Ok(mut cache) = preview_cache().lock() {
            cache.put(key, preview);
        }
    }
}

fn synced_inbox(
    session: &CheckedProfileSession<'_>,
    team: &str,
    vault: &mut AccountVault<'_>,
    blocked: &[RtChannelId],
) -> Result<foks_client::ChatInbox> {
    Ok(session
        .sync_chat_inbox_with_preview_cache(team, vault, blocked, &mut SharedPreviewCache)?
        .inbox)
}

pub(super) fn dispatch(
    state_dir: &Path,
    session: &CheckedProfileSession<'_>,
    vault: &mut AccountVault<'_>,
    master: &[u8; 32],
    store: TeamStoreRef,
    action: ChatAction,
) -> Result<serde_json::Value> {
    if !action.validate() {
        return Err(Box::new(super::AgentRequestError("invalid chat request")));
    }
    let (resolved, scope) = resolve_scope(session, vault, &store)?;
    if action.is_intent() {
        let (host, actor, channel) = match &action {
            ChatAction::LoadIntent {
                host,
                actor,
                channel,
            }
            | ChatAction::SaveIntent {
                host,
                actor,
                channel,
                ..
            }
            | ChatAction::ClearIntent {
                host,
                actor,
                channel,
                ..
            }
            | ChatAction::ImportIntent {
                host,
                actor,
                channel,
                ..
            } => (host, actor, channel),
            _ => unreachable!(),
        };
        // Validate before opening/writing recovery state, not merely on reply.
        if host != &scope.host || actor != &scope.actor {
            return Err(foks_client::Error::ChatIntegrity("saved message scope changed").into());
        }
        let binding = foks_client_app::PendingChatBinding {
            host: host.clone(),
            actor: actor.clone(),
            team: store.team_id.clone(),
            channel: channel.clone(),
        };
        let mut pending = foks_client_app::PendingChatStore::open(state_dir, master)?;
        let intent = match action {
            ChatAction::LoadIntent { .. } => pending.load(&binding)?,
            ChatAction::SaveIntent {
                submission, text, ..
            } => {
                let intent = foks_client_app::LocalChatIntent {
                    submission,
                    text: text.expose().to_owned(),
                };
                pending.save(&store.profile, &binding, &intent)?;
                Some(intent)
            }
            ChatAction::ClearIntent { submission, .. } => {
                pending.clear(&binding, &submission)?;
                None
            }
            ChatAction::ImportIntent {
                submission,
                text,
                source,
                ..
            } => {
                pending.import(
                    &store.profile,
                    &binding,
                    &foks_client_app::LocalChatIntent {
                        submission,
                        text: text.expose().to_owned(),
                    },
                    &source,
                )?;
                None
            }
            _ => unreachable!(),
        };
        return Ok(serde_json::to_value(ChatReply {
            scope,
            result: ChatResult::Intent {
                channel: binding.channel,
                intent: intent.map(|intent| SavedChatIntent {
                    submission: intent.submission.clone(),
                    text: SecretString::new(&intent.text),
                }),
            },
        })?);
    }
    let submission = match &action {
        ChatAction::PrepareChannel { submission, .. }
        | ChatAction::PrepareMessage { submission, .. }
        | ChatAction::SubmitMessage { submission, .. } => {
            let input = submission_input(&action)?;
            Some(foks_client_app::chat_submission(
                master,
                id(submission)?,
                &input,
            ))
        }
        _ => None,
    };
    let team = &store.team_alias;
    let result = match action {
        ChatAction::LoadIntent { .. }
        | ChatAction::SaveIntent { .. }
        | ChatAction::ClearIntent { .. }
        | ChatAction::ImportIntent { .. } => unreachable!("local intents dispatched above"),
        ChatAction::OperationBody { operation, channel } => {
            let text = session.recover_chat_operation_text(
                team,
                &id(&operation)?,
                RtChannelId(id(&channel)?),
                vault,
                master,
            )?;
            if text
                .as_ref()
                .is_some_and(|text| text.len() > CHAT_TEXT_BYTES)
            {
                return Err(foks_client::Error::ChatLimit(
                    "retained message exceeds desktop text limit",
                )
                .into());
            }
            ChatResult::OperationBody {
                operation,
                channel,
                text: text.map(|text| SecretString::new(text.as_str())),
            }
        }
        ChatAction::Channels => {
            let channels = session.list_chat_channels(team, vault)?;
            if channels.host.entity().as_bytes() != resolved.host
                || channels.actor.entity().as_bytes() != resolved.uid
                || channels.team.entity().as_bytes() != resolved.team
            {
                return Err(
                    foks_client::Error::ChatIntegrity("channel response scope changed").into(),
                );
            }
            ChatResult::Channels {
                version: channels.version.to_string(),
                channels: channels.channels.into_iter().map(channel).collect(),
            }
        }
        ChatAction::Inbox => inbox(synced_inbox(session, team, vault, &[])?, &resolved)?,
        ChatAction::SyncInbox { blocked_channels } => {
            let blocked = blocked_channels
                .iter()
                .map(|channel| id(channel).map(RtChannelId))
                .collect::<Result<Vec<_>>>()?;
            inbox(synced_inbox(session, team, vault, &blocked)?, &resolved)?
        }
        ChatAction::MarkRead { channel, sequence } => {
            let sequence = chat_sequence(&sequence)
                .ok_or(super::AgentRequestError("invalid chat sequence"))?;
            session.mark_chat_read(team, RtChannelId(id(&channel)?), sequence, vault)?;
            ChatResult::Read {
                channel,
                sequence: sequence.to_string(),
            }
        }
        ChatAction::PollInbox { .. } => {
            return Err(Box::new(super::AgentRequestError(
                "chat polling requires dedicated dispatch",
            )))
        }
        action @ (ChatAction::History { .. } | ChatAction::NotificationHistory { .. }) => {
            let notification = matches!(action, ChatAction::NotificationHistory { .. });
            let (ChatAction::History { channel, before }
            | ChatAction::NotificationHistory { channel, before }) = action
            else {
                unreachable!()
            };
            let channel_id = RtChannelId(id(&channel)?);
            let end = before.as_deref().and_then(chat_sequence).map(|n| n - 1);
            let mut width = CHAT_PAGE_ROWS as u64;
            loop {
                let history = if notification {
                    session.read_notification_chat(team, channel_id, end, width, vault)
                } else if let Some(end) = end {
                    session.read_chat_thread(
                        team,
                        channel_id,
                        end.saturating_sub(width - 1).max(1),
                        end,
                        vault,
                    )
                } else {
                    session.read_recent_chat(team, channel_id, width, vault)
                };
                let history = match history {
                    Err(foks_client_app::Error::Client(foks_client::Error::ChatLimit(_)))
                        if width > 1 =>
                    {
                        width = (width / 2).max(1);
                        continue;
                    }
                    Err(foks_client_app::Error::Client(foks_client::Error::Rpc(
                        foks_rpc::Error::RemoteStatus { code: 12001, .. },
                    ))) if width > 1 => {
                        width = (width / 2).max(1);
                        continue;
                    }
                    other => other?,
                };
                if history.scope.host != resolved.host
                    || history.scope.uid != resolved.uid
                    || history.scope.team != resolved.team
                    || history.scope.channel != channel_id.0
                {
                    return Err(foks_client::Error::ChatIntegrity(
                        "history response scope changed",
                    )
                    .into());
                }
                let before = history
                    .messages
                    .iter()
                    .map(|m| m.message.sequence)
                    .min()
                    .filter(|n| *n > 1)
                    .map(|n| n.to_string());
                let result = ChatResult::History {
                    channel: channel.clone(),
                    before,
                    missing_predecessors: history
                        .missing_predecessors
                        .iter()
                        .map(u64::to_string)
                        .collect(),
                    messages: history
                        .messages
                        .into_iter()
                        .map(|row| ChatMessage {
                            id: hex(&row.message.metadata.id.0),
                            sequence: row.message.sequence.to_string(),
                            sender: row.message.sender.map(|s| hex(s.entity().as_bytes())),
                            send_time: row.message.metadata.send_time.to_string(),
                            insert_time: row.message.insert_time.to_string(),
                            content: match row.content {
                                foks_client::ChatContent::Text(text)
                                    if text.len() <= CHAT_TEXT_BYTES =>
                                {
                                    ChatContent::Text {
                                        text: if notification {
                                            SecretString::new(
                                                text.chars().take(256).collect::<String>(),
                                            )
                                        } else {
                                            SecretString::new(text.as_str())
                                        },
                                    }
                                }
                                foks_client::ChatContent::Text(_) => ChatContent::Oversized,
                                foks_client::ChatContent::Unsupported(_) => {
                                    ChatContent::Unsupported
                                }
                            },
                        })
                        .collect(),
                };
                // Reserve framing headroom and account for JSON escaping of plaintext.
                let bytes = Zeroizing::new(serde_json::to_vec(&result)?);
                if bytes.len() > foks_agent_proto::MAXIMUM_MESSAGE_BYTES / 2 && width > 1 {
                    width = (width / 2).max(1);
                    continue;
                }
                break result;
            }
        }
        ChatAction::PrepareChannel {
            name,
            description,
            admin,
            ..
        } => ChatResult::Operation {
            operation: operation(session.prepare_chat_channel_submission(
                team,
                foks_client_app::ChatChannelInput {
                    name: name.expose(),
                    description: description.expose(),
                    tier: if admin {
                        RtChannelTier::Admin
                    } else {
                        RtChannelTier::Bottom
                    },
                },
                vault,
                master,
                submission.as_ref(),
            )?)?,
        },
        ChatAction::PrepareMessage { channel, text, .. } => ChatResult::Operation {
            operation: operation(session.prepare_chat_send_submission(
                team,
                RtChannelId(id(&channel)?),
                text.expose(),
                vault,
                master,
                submission.as_ref(),
            )?)?,
        },
        ChatAction::SubmitMessage { channel, text, .. } => ChatResult::Operation {
            operation: operation(
                session.submit_chat_message(
                    &ClientCredentials::open(state_dir)?,
                    team,
                    foks_client_app::ChatMessageInput {
                        channel: RtChannelId(id(&channel)?),
                        text: text.expose(),
                        submission: submission
                            .as_ref()
                            .ok_or(super::AgentRequestError("missing chat submission"))?,
                    },
                    vault,
                    master,
                )?,
            )?,
        },
        ChatAction::Status { operation: op } => ChatResult::Operation {
            operation: operation(session.chat_operation_status(team, &id(&op)?, vault)?)?,
        },
        ChatAction::Attempt { operation: op } => ChatResult::Operation {
            operation: operation(session.attempt_chat_operation(
                &ClientCredentials::open(state_dir)?,
                team,
                &id(&op)?,
                vault,
                master,
            )?)?,
        },
        ChatAction::Reconcile { operation: op } => ChatResult::Operation {
            operation: operation(session.reconcile_chat_operation(
                team,
                &id(&op)?,
                vault,
                master,
            )?)?,
        },
        ChatAction::CleanupPending => ChatResult::CleanupPending {
            operations: session
                .list_cleanup_pending_chat(team, vault)?
                .into_iter()
                .map(operation)
                .collect::<Result<_>>()?,
        },
        ChatAction::Cancel { operation: op } => ChatResult::Operation {
            operation: operation(session.cancel_prepared_chat_operation(
                team,
                &id(&op)?,
                vault,
                master,
            )?)?,
        },
        ChatAction::Finalize { operation: op } => ChatResult::Operation {
            operation: operation(session.finalize_chat_operation(
                team,
                &id(&op)?,
                vault,
                master,
            )?)?,
        },
        ChatAction::Pending => ChatResult::Pending {
            operations: session
                .list_pending_chat(team, vault)?
                .into_iter()
                .map(operation)
                .collect::<Result<_>>()?,
        },
    };
    Ok(serde_json::to_value(ChatReply { scope, result })?)
}

/// Preserve chat-specific policy outcomes without changing generic RPC mapping.
pub(super) fn contextual_error(error: Box<dyn std::error::Error>) -> Box<dyn std::error::Error> {
    let mut source = Some(error.as_ref());
    while let Some(cause) = source {
        if matches!(
            cause.downcast_ref::<foks_rpc::Error>(),
            Some(foks_rpc::Error::RemoteStatus { code: 1013, .. })
        ) {
            return Box::new(foks_client::Error::ChatAccessDenied(
                "server denied this chat action",
            ));
        }
        if matches!(
            cause.downcast_ref::<foks_client::Error>(),
            Some(foks_client::Error::Crypto(_))
        ) {
            return Box::new(foks_client::Error::ChatIntegrity(
                "chat cryptographic verification failed",
            ));
        }
        source = cause.source();
    }
    error
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submit_fingerprint_uses_legacy_prepare_message_bytes() {
        let submission = "ab".repeat(16);
        let channel = "cd".repeat(16);
        let text = SecretString::new("private \"message\"\n日本語");
        let prepare = ChatAction::PrepareMessage {
            submission: submission.clone(),
            channel: channel.clone(),
            text: text.clone(),
        };
        let submit = ChatAction::SubmitMessage {
            submission: submission.clone(),
            channel: channel.clone(),
            text: text.clone(),
        };
        let legacy = Zeroizing::new(serde_json::to_vec(&prepare).unwrap());
        assert_eq!(submission_input(&prepare).unwrap(), legacy);
        assert_eq!(submission_input(&submit).unwrap(), legacy);
        let fingerprint = |action: &ChatAction| {
            foks_client_app::chat_submission(
                &[7; 32],
                id(&submission).unwrap(),
                &submission_input(action).unwrap(),
            )
            .input_mac
        };
        assert_eq!(fingerprint(&prepare), fingerprint(&submit));
        for altered in [
            ChatAction::SubmitMessage {
                submission: submission.clone(),
                channel: "ef".repeat(16),
                text,
            },
            ChatAction::SubmitMessage {
                submission: submission.clone(),
                channel,
                text: SecretString::new("changed message"),
            },
        ] {
            assert_ne!(fingerprint(&submit), fingerprint(&altered));
        }
    }

    #[test]
    fn content_failure_retains_channel_classification_at_dispatch() {
        let error = contextual_error(Box::new(foks_client::Error::ChatChannelIntegrity(
            "bad message",
        )));
        let response = crate::dispatch_error_response(1, error.as_ref());
        assert!(matches!(
            response.result,
            foks_agent_proto::ResponseResult::Error {
                code: foks_agent_proto::ErrorCode::ChatChannelIntegrity,
                ..
            }
        ));
    }

    /// The desktop checks a channel name and description before sending them,
    /// out of the same `chat-limits.json` this crate compiles its constants
    /// from. `ChatLimits` is what the client actually admits them on, so the two
    /// have to carry the same numbers or the sheet would accept a name the
    /// agent refuses.
    #[test]
    fn character_bounds_match_the_client_admission_policy() {
        use foks_client_db::ChatLimits;
        assert_eq!(CHAT_NAME_MIN_CHARS, ChatLimits::NAME_MIN_CHARS);
        assert_eq!(CHAT_NAME_MAX_CHARS, ChatLimits::NAME_MAX_CHARS);
        assert_eq!(
            CHAT_DESCRIPTION_MIN_CHARS,
            ChatLimits::DESCRIPTION_MIN_CHARS
        );
        assert_eq!(
            CHAT_DESCRIPTION_MAX_CHARS,
            ChatLimits::DESCRIPTION_MAX_CHARS
        );
    }

    /// A channel another client named past the limit reaches the desktop
    /// within the bytes its contract admits, rather than as a reply it
    /// refuses for the whole account.
    #[test]
    fn peer_channel_text_is_bounded_to_the_limits_the_desktop_holds() {
        let name = bounded_chars(&"x".repeat(CHAT_NAME_MAX_CHARS + 1), CHAT_NAME_MAX_CHARS);
        assert_eq!(name.chars().count(), CHAT_NAME_MAX_CHARS);
        assert!(name.ends_with('…'));
        let wide = bounded_chars(&"💬".repeat(200), CHAT_NAME_MAX_CHARS);
        assert_eq!(wide.chars().count(), CHAT_NAME_MAX_CHARS);
        assert!(wide.len() <= CHAT_NAME_BYTES);
        let description = bounded_chars(
            &"y".repeat(CHAT_DESCRIPTION_MAX_CHARS * 2),
            CHAT_DESCRIPTION_MAX_CHARS,
        );
        assert_eq!(description.chars().count(), CHAT_DESCRIPTION_MAX_CHARS);
        assert!(description.len() <= CHAT_DESCRIPTION_BYTES);
        let exact = "z".repeat(CHAT_NAME_MAX_CHARS);
        assert_eq!(bounded_chars(&exact, CHAT_NAME_MAX_CHARS), exact);
    }

    #[test]
    fn preview_text_is_single_line_and_utf8_bounded() {
        assert_eq!(preview_text("first\nsecond"), "first");
        let preview = preview_text(&"💬".repeat(200));
        assert!(preview.len() <= CHAT_SNIPPET_BYTES);
        assert!(preview.ends_with('…'));
    }

    fn preview_key(
        actor: u8,
        channel: u8,
        sequence: u64,
        generation: u64,
    ) -> foks_client::ChatPreviewKey {
        foks_client::ChatPreviewKey {
            host: vec![1; 33],
            actor: vec![actor; 33],
            team: vec![3; 33],
            channel: [channel; 16],
            sequence,
            role: foks_proto::Role::ADMIN,
            generation,
        }
    }

    fn preview(text: &str) -> foks_client::ChatPreview {
        foks_client::ChatPreview {
            sender: None,
            send_time: 1,
            insert_time: 1,
            content: foks_client::ChatPreviewContent::Text(Zeroizing::new(text.to_owned())),
        }
    }

    #[test]
    fn cached_previews_are_bound_to_reader_message_and_key_generation() {
        let mut cache = PreviewCache::default();
        let key = preview_key(2, 9, 5, 1);
        cache.put(key.clone(), preview("hello"));
        assert_eq!(cache.get(&key), Some(preview("hello")));
        // Another reader, another channel, the next message and the next key
        // generation are all different entries, never this one.
        for other in [
            preview_key(0x22, 9, 5, 1),
            preview_key(2, 0x99, 5, 1),
            preview_key(2, 9, 6, 1),
            preview_key(2, 9, 5, 2),
        ] {
            assert_eq!(cache.get(&other), None);
        }
        // The same key re-read replaces rather than duplicates.
        cache.put(key.clone(), preview("edited"));
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.get(&key), Some(preview("edited")));
    }

    #[test]
    fn the_preview_cache_is_bounded_and_evicts_its_oldest_entry() {
        let mut cache = PreviewCache::default();
        for sequence in 0..(PREVIEW_CACHE_ENTRIES as u64 + 2) {
            cache.put(preview_key(2, 9, sequence, 1), preview("bounded"));
        }
        assert_eq!(cache.entries.len(), PREVIEW_CACHE_ENTRIES);
        // Every entry holds a rendered snippet, so the whole cache is bounded
        // by its entry count times the snippet size, whatever was sent.
        cache.put(preview_key(2, 9, 1_000_000, 1), preview(&"💬".repeat(4000)));
        let stored = cache.get(&preview_key(2, 9, 1_000_000, 1)).unwrap();
        let foks_client::ChatPreviewContent::Text(text) = stored.content else {
            panic!("expected text")
        };
        assert!(text.len() <= CHAT_SNIPPET_BYTES);
        assert!(text.ends_with('…'));
        // Rendering a cached entry gives what rendering the fetched one gave.
        assert_eq!(preview_text(&text), *text);
        assert_eq!(cache.get(&preview_key(2, 9, 0, 1)), None);
        assert_eq!(cache.get(&preview_key(2, 9, 1, 1)), None);
        assert!(cache
            .get(&preview_key(2, 9, PREVIEW_CACHE_ENTRIES as u64 + 1, 1))
            .is_some());
    }
}
