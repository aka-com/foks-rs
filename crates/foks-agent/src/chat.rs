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
fn channel(value: foks_client::ChatChannel) -> ChatChannel {
    ChatChannel {
        id: hex(&value.metadata.id.0),
        name: SecretString::new(value.name.0.as_str()),
        admin: value.metadata.tier == RtChannelTier::Admin,
        readable: !value.metadata.unreadable,
        read_role: role_label(value.metadata.roles.read),
        write_role: role_label(value.metadata.roles.write),
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
    if result.inbox_version > i64::MAX as u64
        || (result.bumped && result.inbox_version <= since)
        || (!result.bumped && result.inbox_version > since)
    {
        return Err(foks_client::Error::ChatIntegrity("invalid inbox poll result").into());
    }
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
        create: op.kind == K::Create,
        state: match op.state {
            S::Prepared => ChatState::Prepared,
            S::Uncertain => ChatState::Uncertain,
            S::Confirmed => ChatState::Confirmed,
            S::Rejected => ChatState::Rejected,
            S::Cancelled => ChatState::Cancelled,
        },
        sequence,
        rejection_code: op.rejection_code,
    })
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
    let submission = match &action {
        ChatAction::PrepareChannel { submission, .. }
        | ChatAction::PrepareMessage { submission, .. } => {
            let input = Zeroizing::new(serde_json::to_vec(&action)?);
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
        ChatAction::Inbox | ChatAction::SyncInbox => {
            inbox(session.sync_chat_inbox(team, vault)?.inbox, &resolved)?
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
        ChatAction::History { channel, before } => {
            let channel_id = RtChannelId(id(&channel)?);
            let end = before.as_deref().and_then(chat_sequence).map(|n| n - 1);
            let mut width = CHAT_PAGE_ROWS as u64;
            loop {
                let history = if let Some(end) = end {
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
                            content: match row.content {
                                foks_client::ChatContent::Text(text)
                                    if text.len() <= CHAT_TEXT_BYTES =>
                                {
                                    ChatContent::Text {
                                        text: SecretString::new(text.as_str()),
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
        ChatAction::PrepareChannel { name, admin, .. } => ChatResult::Operation {
            operation: operation(session.prepare_chat_channel_submission(
                team,
                foks_client_app::ChatChannelInput {
                    name: name.expose(),
                    description: "",
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
