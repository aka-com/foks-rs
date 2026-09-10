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
fn valid_operation(op: &ChatOperation) -> bool {
    valid_chat_id(&op.id)
        && valid_chat_id(&op.channel)
        && op
            .sequence
            .as_ref()
            .is_none_or(|n| chat_sequence(n).is_some_and(|n| n > 0))
        && (op.state == ChatState::Rejected) == op.rejection_code.is_some()
        && (op.state == ChatState::Confirmed && !op.create) == op.sequence.is_some()
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
        (ChatAction::Channels, ChatResult::Channels { channels, version }) => {
            let mut ids = HashSet::new();
            chat_sequence(version).is_some()
                && channels.len() <= CHAT_CHANNEL_ROWS
                && channels.iter().all(|c| {
                    valid_chat_id(&c.id)
                        && ids.insert(&c.id)
                        && c.name.expose().len() <= CHAT_NAME_BYTES
                        && c.read_role.len() <= CHAT_LABEL_BYTES
                        && c.write_role.len() <= CHAT_LABEL_BYTES
                })
        }
        (
            ChatAction::History {
                channel: expected,
                before: bound,
            },
            ChatResult::History {
                channel,
                messages,
                before,
                missing_predecessors,
            },
        ) => {
            let mut ids = HashSet::new();
            let mut sequences = HashSet::new();
            channel == expected
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
                        && match &m.content {
                            ChatContent::Text { text } => text.expose().len() <= CHAT_TEXT_BYTES,
                            _ => true,
                        }
                })
                && *before
                    == messages
                        .iter()
                        .filter_map(|m| chat_sequence(&m.sequence))
                        .min()
                        .filter(|n| *n > 1)
                        .map(|n| n.to_string())
        }
        (ChatAction::PrepareMessage { channel, .. }, ChatResult::Operation { operation }) => {
            valid_operation(operation) && !operation.create && &operation.channel == channel
        }
        (ChatAction::PrepareChannel { .. }, ChatResult::Operation { operation }) => {
            valid_operation(operation) && operation.create
        }
        (
            ChatAction::Status {
                operation: expected,
            }
            | ChatAction::Attempt {
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
    fn rejects_wrong_scope_duplicate_messages_and_cursor_lies() {
        let store = TeamStoreRef {
            profile: "local".into(),
            account_alias: "me".into(),
            team_alias: "team".into(),
            team_id: format!("03{}", "ab".repeat(32)),
        };
        let action = ChatAction::History {
            channel: "ab".repeat(16),
            before: None,
        };
        let message = ChatMessage {
            id: "cd".repeat(16),
            sequence: "9007199254740993".into(),
            sender: None,
            content: ChatContent::Unsupported,
        };
        let mut reply = ChatReply {
            scope: ChatScope {
                store: store.clone(),
                host: format!("02{}", "ab".repeat(32)),
                actor: format!("01{}", "ab".repeat(32)),
            },
            result: ChatResult::History {
                channel: "ab".repeat(16),
                messages: vec![message.clone()],
                before: Some(message.sequence.clone()),
                missing_predecessors: vec![],
            },
        };
        validate_chat_reply(&store, &action, &reply).unwrap();
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
