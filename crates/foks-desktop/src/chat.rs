use crate::{AgentError, AgentTransport};
use foks_agent_proto::{chat::*, Operation, TeamStoreRef};
use std::collections::HashSet;

fn invalid() -> AgentError {
    AgentError::Protocol {
        code: foks_agent_proto::ErrorCode::ChatIntegrity,
        message: "Chat response does not match the request.".into(),
        fields: Default::default(),
    }
}
fn entity(value: &str) -> bool {
    value.len() == 66
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn valid_channel(channel: &ChatChannel) -> bool {
    (!channel.writable || channel.readable)
        && valid_chat_id(&channel.id)
        && channel.name.expose().len() <= CHAT_NAME_BYTES
        && channel
            .description
            .as_ref()
            .is_none_or(|description| description.expose().len() <= CHAT_DESCRIPTION_BYTES)
        && channel.read_role.len() <= CHAT_LABEL_BYTES
        && channel.write_role.len() <= CHAT_LABEL_BYTES
}
fn valid_content(content: &ChatContent, maximum: usize) -> bool {
    match content {
        ChatContent::Text { text } => text.expose().len() <= maximum,
        ChatContent::Unsupported | ChatContent::Oversized => true,
    }
}
fn valid_preview(preview: &ChatPreview) -> bool {
    preview
        .sender
        .as_ref()
        .is_none_or(|sender| entity(sender) && sender.starts_with("01"))
        && chat_sequence(&preview.send_time).is_some()
        && chat_sequence(&preview.insert_time).is_some()
        && valid_content(&preview.content, CHAT_SNIPPET_BYTES)
}
fn valid_operation(op: &ChatOperation) -> bool {
    valid_chat_id(&op.id)
        && valid_chat_id(&op.channel)
        && match (&op.kind, &op.confirmation) {
            (_, None) => op.state != ChatState::Confirmed,
            (ChatOperationKind::CreateChannel, Some(ChatOperationConfirmation::ChannelCreated)) => {
                op.state == ChatState::Confirmed
            }
            (
                ChatOperationKind::SendMessage,
                Some(ChatOperationConfirmation::MessageSent { sequence }),
            ) => op.state == ChatState::Confirmed && chat_sequence(sequence).is_some_and(|n| n > 0),
            _ => false,
        }
        && (op.state == ChatState::Rejected) == op.rejection_code.is_some()
}
pub fn validate_chat_reply(
    store: &TeamStoreRef,
    action: &ChatAction,
    reply: &ChatReply,
) -> Result<(), AgentError> {
    if store.profile.len() > CHAT_LABEL_BYTES
        || store.account_alias.len() > CHAT_LABEL_BYTES
        || store.team_alias.len() > CHAT_LABEL_BYTES
        || &reply.scope.store != store
        || !entity(&reply.scope.host)
        || !entity(&reply.scope.actor)
        || !reply.scope.host.starts_with("02")
        || !reply.scope.actor.starts_with("01")
        || !store.team_id.starts_with("03")
        || !entity(&store.team_id)
    {
        return Err(invalid());
    }
    let valid = match (action, &reply.result) {
        (
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
            },
            ChatResult::Intent {
                channel: actual,
                intent,
            },
        ) => {
            host == &reply.scope.host
                && actor == &reply.scope.actor
                && channel == actual
                && intent.as_ref().is_none_or(|i| {
                    valid_chat_id(&i.submission)
                        && !i.text.expose().is_empty()
                        && i.text.expose().len() <= CHAT_TEXT_BYTES
                })
                && match action {
                    ChatAction::LoadIntent { .. } => true,
                    ChatAction::SaveIntent {
                        submission, text, ..
                    } => intent
                        .as_ref()
                        .is_some_and(|i| &i.submission == submission && &i.text == text),
                    _ => intent.is_none(),
                }
        }
        (
            ChatAction::OperationBody { operation, channel },
            ChatResult::OperationBody {
                operation: actual,
                channel: actual_channel,
                text,
            },
        ) => {
            operation == actual
                && channel == actual_channel
                && valid_chat_id(actual)
                && valid_chat_id(actual_channel)
                && text
                    .as_ref()
                    .is_none_or(|text| text.expose().len() <= CHAT_TEXT_BYTES)
        }
        (ChatAction::Channels, ChatResult::Channels { channels, version }) => {
            let mut ids = HashSet::new();
            chat_sequence(version).is_some()
                && channels.len() <= CHAT_CHANNEL_ROWS
                && channels
                    .iter()
                    .all(|channel| valid_channel(channel) && ids.insert(&channel.id))
        }
        (
            ChatAction::History {
                channel: expected,
                before: bound,
                ..
            }
            | ChatAction::NotificationHistory {
                channel: expected,
                before: bound,
            },
            ChatResult::History {
                channel,
                messages,
                before,
                missing_predecessors,
                gap,
            },
        ) => {
            let mut ids = HashSet::new();
            let mut sequences = HashSet::new();
            channel == expected
                && match action {
                    ChatAction::History {
                        after: Some(after), ..
                    } => {
                        gap.is_some()
                            && messages.iter().all(|m| {
                                chat_sequence(&m.sequence)
                                    .zip(chat_sequence(after))
                                    .is_some_and(|(n, after)| n > after)
                            })
                    }
                    _ => gap.is_none(),
                }
                && messages.len() <= CHAT_PAGE_ROWS
                && missing_predecessors.len() <= CHAT_MISSING_PREDECESSORS
                && missing_predecessors
                    .iter()
                    .all(|n| chat_sequence(n).is_some_and(|n| n > 0))
                && messages.iter().all(|m| {
                    valid_chat_id(&m.id)
                        && ids.insert(&m.id)
                        && sequences.insert(&m.sequence)
                        && chat_sequence(&m.sequence).is_some_and(|n| {
                            n > 0
                                && bound
                                    .as_ref()
                                    .is_none_or(|b| chat_sequence(b).is_some_and(|b| n < b))
                        })
                        && m.sender
                            .as_ref()
                            .is_none_or(|s| entity(s) && s.starts_with("01"))
                        && chat_sequence(&m.send_time).is_some()
                        && chat_sequence(&m.insert_time).is_some()
                        && valid_content(&m.content, CHAT_TEXT_BYTES)
                        && (!matches!(action, ChatAction::NotificationHistory { .. })
                            || match &m.content {
                                ChatContent::Text { text } => text.expose().chars().count() <= 256,
                                _ => true,
                            })
                })
                && *before
                    == messages
                        .iter()
                        .filter_map(|m| chat_sequence(&m.sequence))
                        .min()
                        .filter(|n| *n > 1)
                        .map(|n| n.to_string())
        }
        (
            ChatAction::Inbox | ChatAction::SyncInbox { .. },
            ChatResult::Inbox {
                channels,
                cursor,
                head,
                degraded,
                conversations,
                blocked_channels,
                ..
            },
        ) => {
            let Some(cursor) = chat_sequence(cursor) else {
                return Err(invalid());
            };
            let Some(head) = chat_sequence(head) else {
                return Err(invalid());
            };
            let mut ids = HashSet::new();
            let mut versions = HashSet::new();
            let mut channel_ids = HashSet::new();
            channels.len() <= CHAT_CHANNEL_ROWS
                && channels
                    .iter()
                    .all(|c| valid_channel(c) && channel_ids.insert(&c.id))
                && blocked_channels.len() <= CHAT_CHANNEL_ROWS
                && blocked_channels.iter().collect::<HashSet<_>>().len() == blocked_channels.len()
                && blocked_channels.iter().all(|id| channel_ids.contains(id))
                && conversations
                    .iter()
                    .all(|c| !blocked_channels.contains(&c.channel.id) || c.preview.is_none())
                && cursor <= head
                && *degraded == (cursor < head)
                && conversations.len() <= CHAT_INBOX_ROWS
                && conversations.iter().all(|conversation| {
                    valid_channel(&conversation.channel)
                        && conversation.channel.readable
                        && ids.insert(&conversation.channel.id)
                        && chat_sequence(&conversation.inbox_version)
                            .is_some_and(|version| version > 0 && version <= head)
                        && versions.insert(&conversation.inbox_version)
                        && chat_sequence(&conversation.read_through).is_some()
                        && conversation
                            .pending_read
                            .as_ref()
                            .is_none_or(|value| chat_sequence(value).is_some_and(|value| value > 0))
                        && chat_sequence(&conversation.unread).is_some()
                        && conversation.preview.as_ref().is_none_or(valid_preview)
                })
        }
        (
            ChatAction::MarkRead {
                channel: expected_channel,
                sequence: expected_sequence,
            },
            ChatResult::Read { channel, sequence },
        ) => channel == expected_channel && sequence == expected_sequence,
        (
            ChatAction::PollInbox { since, .. },
            ChatResult::Poll {
                bumped,
                inbox_version,
            },
        ) => chat_sequence(since).is_some_and(|since| {
            chat_sequence(inbox_version)
                .is_some_and(|head| (*bumped && head > since) || (!*bumped && head <= since))
        }),
        (
            ChatAction::PrepareMessage { channel, .. } | ChatAction::SubmitMessage { channel, .. },
            ChatResult::Operation { operation },
        ) => {
            valid_operation(operation)
                && operation.kind == ChatOperationKind::SendMessage
                && &operation.channel == channel
        }
        (ChatAction::PrepareChannel { .. }, ChatResult::Operation { operation }) => {
            valid_operation(operation) && operation.kind == ChatOperationKind::CreateChannel
        }
        (
            ChatAction::Status {
                operation: expected,
            }
            | ChatAction::Attempt {
                operation: expected,
            }
            | ChatAction::Reconcile {
                operation: expected,
            }
            | ChatAction::Cancel {
                operation: expected,
            }
            | ChatAction::Finalize {
                operation: expected,
            },
            ChatResult::Operation { operation },
        ) => valid_operation(operation) && &operation.id == expected,
        (ChatAction::CleanupPending, ChatResult::CleanupPending { operations }) => {
            let mut ids = HashSet::new();
            operations.len() <= CHAT_PENDING_ROWS
                && operations.iter().all(|op| {
                    valid_operation(op)
                        && ids.insert(&op.id)
                        && matches!(
                            op.state,
                            ChatState::Confirmed | ChatState::Rejected | ChatState::Cancelled
                        )
                })
        }
        (ChatAction::Pending, ChatResult::Pending { operations }) => {
            let mut ids = HashSet::new();
            operations.len() <= CHAT_PENDING_ROWS
                && operations.iter().all(|op| {
                    valid_operation(op)
                        && ids.insert(&op.id)
                        && matches!(op.state, ChatState::Prepared | ChatState::Uncertain)
                })
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(invalid())
    }
}
pub fn chat_request(
    transport: &dyn AgentTransport,
    store: TeamStoreRef,
    action: ChatAction,
) -> Result<ChatReply, AgentError> {
    chat_request_cancellable(transport, store, action, &|| false)
}
pub fn chat_request_cancellable(
    transport: &dyn AgentTransport,
    store: TeamStoreRef,
    action: ChatAction,
    cancelled: &dyn Fn() -> bool,
) -> Result<ChatReply, AgentError> {
    if !action.validate() {
        return Err(AgentError::Protocol {
            code: foks_agent_proto::ErrorCode::InvalidRequest,
            message: "Invalid chat request.".into(),
            fields: Default::default(),
        });
    }
    let value = transport.call_cancellable(
        Operation::Chat {
            store: store.clone(),
            action: action.clone(),
        },
        cancelled,
    )?;
    // Avoid exposing serde parse diagnostics, which can contain plaintext values.
    let reply: ChatReply = serde_json::from_value(value).map_err(|_| invalid())?;
    validate_chat_reply(&store, &action, &reply)?;
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_reconcile_identity_and_bounded_terminal_cleanup() {
        let store = TeamStoreRef {
            profile: "local".into(),
            account_alias: "me".into(),
            team_alias: "team".into(),
            team_id: "03".to_owned() + &"ab".repeat(32),
        };
        let op = ChatOperation {
            id: "ab".repeat(16),
            channel: "cd".repeat(16),
            kind: ChatOperationKind::SendMessage,
            state: ChatState::Cancelled,
            confirmation: None,
            rejection_code: None,
        };
        let mut reply = ChatReply {
            scope: ChatScope {
                store: store.clone(),
                host: "02".to_owned() + &"ab".repeat(32),
                actor: "01".to_owned() + &"ab".repeat(32),
            },
            result: ChatResult::Operation {
                operation: op.clone(),
            },
        };
        assert!(validate_chat_reply(
            &store,
            &ChatAction::Reconcile {
                operation: op.id.clone()
            },
            &reply
        )
        .is_ok());
        assert!(validate_chat_reply(
            &store,
            &ChatAction::Reconcile {
                operation: "ef".repeat(16)
            },
            &reply
        )
        .is_err());
        reply.result = ChatResult::CleanupPending {
            operations: vec![op.clone()],
        };
        assert!(validate_chat_reply(&store, &ChatAction::CleanupPending, &reply).is_ok());
        reply.result = ChatResult::CleanupPending {
            operations: vec![op.clone(), op.clone()],
        };
        assert!(validate_chat_reply(&store, &ChatAction::CleanupPending, &reply).is_err());
        reply.result = ChatResult::CleanupPending {
            operations: vec![op.clone(); CHAT_PENDING_ROWS + 1],
        };
        assert!(validate_chat_reply(&store, &ChatAction::CleanupPending, &reply).is_err());
        for state in [
            ChatState::Prepared,
            ChatState::Uncertain,
            ChatState::Confirmed,
        ] {
            reply.result = ChatResult::CleanupPending {
                operations: vec![ChatOperation {
                    state,
                    ..op.clone()
                }],
            };
            assert!(validate_chat_reply(&store, &ChatAction::CleanupPending, &reply).is_err());
        }
        reply.result = ChatResult::Pending { operations: vec![] };
        assert!(validate_chat_reply(&store, &ChatAction::CleanupPending, &reply).is_err());
    }

    #[test]
    fn submit_reply_requires_matching_channel_send_kind_and_valid_state() {
        let store = TeamStoreRef {
            profile: "local".into(),
            account_alias: "me".into(),
            team_alias: "team".into(),
            team_id: format!("03{}", "ab".repeat(32)),
        };
        let action = ChatAction::SubmitMessage {
            submission: "ef".repeat(16),
            channel: "ab".repeat(16),
            text: foks_agent_proto::SecretString::new("private message"),
        };
        let op = ChatOperation {
            id: "cd".repeat(16),
            channel: "ab".repeat(16),
            kind: ChatOperationKind::SendMessage,
            state: ChatState::Prepared,
            confirmation: None,
            rejection_code: None,
        };
        let mut reply = ChatReply {
            scope: ChatScope {
                store: store.clone(),
                host: format!("02{}", "ab".repeat(32)),
                actor: format!("01{}", "ab".repeat(32)),
            },
            result: ChatResult::Operation {
                operation: op.clone(),
            },
        };
        for state in [
            ChatState::Prepared,
            ChatState::Uncertain,
            ChatState::Cancelled,
        ] {
            reply.result = ChatResult::Operation {
                operation: ChatOperation {
                    state,
                    ..op.clone()
                },
            };
            assert!(validate_chat_reply(&store, &action, &reply).is_ok());
        }
        for operation in [
            ChatOperation {
                channel: "ef".repeat(16),
                ..op.clone()
            },
            ChatOperation {
                kind: ChatOperationKind::CreateChannel,
                ..op.clone()
            },
            ChatOperation {
                state: ChatState::Confirmed,
                ..op.clone()
            },
            ChatOperation {
                id: "0".repeat(32),
                ..op.clone()
            },
        ] {
            reply.result = ChatResult::Operation { operation };
            assert!(validate_chat_reply(&store, &action, &reply).is_err());
        }
        reply.result = ChatResult::Operation {
            operation: ChatOperation {
                state: ChatState::Confirmed,
                confirmation: Some(ChatOperationConfirmation::MessageSent {
                    sequence: "1".into(),
                }),
                ..op
            },
        };
        assert!(validate_chat_reply(&store, &action, &reply).is_ok());
        reply.result = ChatResult::Pending { operations: vec![] };
        assert!(validate_chat_reply(&store, &action, &reply).is_err());
    }

    #[test]
    fn rejects_wrong_scope_duplicate_messages_and_cursor_lies() {
        let store = TeamStoreRef {
            profile: "local".into(),
            account_alias: "me".into(),
            team_alias: "team".into(),
            team_id: format!("03{}", "ab".repeat(32)),
        };
        let action = ChatAction::History {
            after: None,
            channel: "ab".repeat(16),
            before: None,
        };
        let message = ChatMessage {
            id: "cd".repeat(16),
            sequence: "9007199254740993".into(),
            sender: None,
            send_time: "1700000000000".into(),
            insert_time: "1700000000001".into(),
            content: ChatContent::Unsupported,
        };
        let mut reply = ChatReply {
            scope: ChatScope {
                store: store.clone(),
                host: format!("02{}", "ab".repeat(32)),
                actor: format!("01{}", "ab".repeat(32)),
            },
            result: ChatResult::History {
                gap: None,
                channel: "ab".repeat(16),
                messages: vec![message.clone()],
                before: Some(message.sequence.clone()),
                missing_predecessors: vec![],
            },
        };
        validate_chat_reply(&store, &action, &reply).unwrap();
        let incremental = ChatAction::History {
            channel: "ab".repeat(16),
            before: None,
            after: Some("9007199254740992".into()),
        };
        assert!(validate_chat_reply(&store, &incremental, &reply).is_err());
        if let ChatResult::History { gap, .. } = &mut reply.result {
            *gap = Some(false);
        }
        validate_chat_reply(&store, &incremental, &reply).unwrap();
        let at_row = ChatAction::History {
            channel: "ab".repeat(16),
            before: None,
            after: Some(message.sequence.clone()),
        };
        assert!(validate_chat_reply(&store, &at_row, &reply).is_err());
        assert!(validate_chat_reply(&store, &action, &reply).is_err());
        if let ChatResult::History { gap, .. } = &mut reply.result {
            *gap = None;
        }
        reply.scope.store.account_alias = "other".into();
        assert!(validate_chat_reply(&store, &action, &reply).is_err());
        reply.scope.store = store.clone();
        if let ChatResult::History { messages, .. } = &mut reply.result {
            messages.push(message);
        }
        assert!(validate_chat_reply(&store, &action, &reply).is_err());
        if let ChatResult::History {
            messages, before, ..
        } = &mut reply.result
        {
            messages.pop();
            *before = Some("2".into());
        }
        assert!(validate_chat_reply(&store, &action, &reply).is_err());
    }

    #[test]
    fn inbox_and_poll_results_are_bounded_and_cursor_consistent() {
        let store = TeamStoreRef {
            profile: "local".into(),
            account_alias: "me".into(),
            team_alias: "team".into(),
            team_id: format!("03{}", "ab".repeat(32)),
        };
        let scope = ChatScope {
            store: store.clone(),
            host: format!("02{}", "ab".repeat(32)),
            actor: format!("01{}", "ab".repeat(32)),
        };
        let conversation = ChatConversation {
            channel: ChatChannel {
                id: "ab".repeat(16),
                name: foks_agent_proto::SecretString::new("general"),
                description: Some(foks_agent_proto::SecretString::new("Team updates")),
                admin: false,
                readable: true,
                writable: true,
                read_role: "Member (0)".into(),
                write_role: "Member (0)".into(),
            },
            inbox_version: "2".into(),
            read_through: "1".into(),
            pending_read: None,
            unread: "1".into(),
            hidden: false,
            muted: false,
            preview: Some(ChatPreview {
                sender: Some(format!("01{}", "cd".repeat(32))),
                send_time: "1700000000000".into(),
                insert_time: "1700000000001".into(),
                content: ChatContent::Text {
                    text: foks_agent_proto::SecretString::new("Latest update"),
                },
            }),
        };
        let mut reply = ChatReply {
            scope: scope.clone(),
            result: ChatResult::Inbox {
                channels: vec![conversation.channel.clone()],
                read_retry_pending: false,
                previews_incomplete: false,
                blocked_channels: Vec::new(),
                cursor: "2".into(),
                head: "2".into(),
                degraded: false,
                conversations: vec![conversation],
            },
        };
        validate_chat_reply(
            &store,
            &ChatAction::SyncInbox {
                blocked_channels: Vec::new(),
            },
            &reply,
        )
        .unwrap();
        let ChatResult::Inbox { head, .. } = &mut reply.result else {
            panic!()
        };
        *head = "1".into();
        assert!(validate_chat_reply(
            &store,
            &ChatAction::SyncInbox {
                blocked_channels: Vec::new()
            },
            &reply
        )
        .is_err());
        let poll = ChatReply {
            scope,
            result: ChatResult::Poll {
                bumped: true,
                inbox_version: "3".into(),
            },
        };
        validate_chat_reply(
            &store,
            &ChatAction::PollInbox {
                since: "2".into(),
                timeout_milliseconds: 1,
            },
            &poll,
        )
        .unwrap();
    }
}

#[cfg(test)]
mod shared_contract_tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Fixtures {
        store: TeamStoreRef,
        cases: Vec<Case>,
    }
    #[derive(Deserialize)]
    struct Case {
        name: String,
        valid: bool,
        action: ChatAction,
        reply: serde_json::Value,
    }

    #[test]
    fn shared_typescript_and_rust_reply_fixtures_agree() {
        let fixtures: Fixtures = serde_json::from_str(include_str!(
            "../../foks-agent-proto/tests/fixtures/chat-replies.json"
        ))
        .unwrap();
        for case in fixtures.cases {
            assert!(
                case.action.validate(),
                "invalid fixture action: {}",
                case.name
            );
            let accepted = serde_json::from_value::<ChatReply>(case.reply).is_ok_and(|reply| {
                validate_chat_reply(&fixtures.store, &case.action, &reply).is_ok()
            });
            assert_eq!(accepted, case.valid, "{}", case.name);
        }
    }
}
