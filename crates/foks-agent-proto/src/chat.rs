//! Bounded, scope-bound desktop chat contract. Plaintext has redacted Debug output.
use crate::{SecretString, TeamStoreRef};
use serde::{Deserialize, Serialize};

include!(concat!(env!("OUT_DIR"), "/chat_limits.rs"));

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ChatAction {
    Channels,
    History {
        channel: String,
        before: Option<String>,
    },
    Inbox,
    SyncInbox,
    MarkRead {
        channel: String,
        sequence: String,
    },
    PollInbox {
        since: String,
        timeout_milliseconds: u64,
    },
    PrepareChannel {
        submission: String,
        name: SecretString,
        admin: bool,
    },
    PrepareMessage {
        submission: String,
        channel: String,
        text: SecretString,
    },
    Status {
        operation: String,
    },
    Attempt {
        operation: String,
    },
    Cancel {
        operation: String,
    },
    Finalize {
        operation: String,
    },
    Pending,
}
impl ChatAction {
    pub fn is_mutation(&self) -> bool {
        !matches!(
            self,
            Self::Channels
                | Self::History { .. }
                | Self::Inbox
                | Self::SyncInbox
                | Self::PollInbox { .. }
                | Self::Pending
                | Self::Status { .. }
        )
    }
    pub fn validate(&self) -> bool {
        match self {
            Self::History { channel, before } => {
                valid_chat_id(channel)
                    && before
                        .as_ref()
                        .is_none_or(|n| chat_sequence(n).is_some_and(|n| n > 1))
            }
            Self::MarkRead { channel, sequence } => {
                valid_chat_id(channel) && chat_sequence(sequence).is_some_and(|n| n > 0)
            }
            Self::PollInbox {
                since,
                timeout_milliseconds,
            } => {
                chat_sequence(since).is_some()
                    && *timeout_milliseconds <= CHAT_POLL_MILLISECONDS as u64
            }
            Self::PrepareChannel {
                submission, name, ..
            } => valid_chat_id(submission) && name.expose().len() <= CHAT_NAME_BYTES,
            Self::PrepareMessage {
                submission,
                channel,
                text,
            } => {
                valid_chat_id(submission)
                    && valid_chat_id(channel)
                    && !text.expose().is_empty()
                    && text.expose().len() <= CHAT_TEXT_BYTES
            }
            Self::Status { operation }
            | Self::Attempt { operation }
            | Self::Cancel { operation }
            | Self::Finalize { operation } => valid_chat_id(operation),
            Self::Channels | Self::Inbox | Self::SyncInbox | Self::Pending => true,
        }
    }
}
pub fn valid_chat_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && id.bytes().any(|b| b != b'0')
}
pub fn chat_sequence(value: &str) -> Option<u64> {
    let n = value.parse::<u64>().ok()?;
    (n <= i64::MAX as u64 && n.to_string() == value).then_some(n)
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChatScope {
    pub store: TeamStoreRef,
    pub host: String,
    pub actor: String,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChatChannel {
    pub id: String,
    pub name: SecretString,
    pub admin: bool,
    pub readable: bool,
    pub read_role: String,
    pub write_role: String,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ChatContent {
    Text { text: SecretString },
    Unsupported,
    Oversized,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChatMessage {
    pub id: String,
    pub sequence: String,
    pub sender: Option<String>,
    pub content: ChatContent,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChatState {
    Prepared,
    Uncertain,
    Confirmed,
    Rejected,
    Cancelled,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChatConversation {
    pub channel: ChatChannel,
    pub inbox_version: String,
    pub read_through: String,
    pub pending_read: Option<String>,
    pub unread: String,
    pub hidden: bool,
    pub muted: bool,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChatOperation {
    pub id: String,
    pub channel: String,
    pub create: bool,
    pub state: ChatState,
    pub sequence: Option<String>,
    pub rejection_code: Option<i64>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ChatResult {
    Channels {
        channels: Vec<ChatChannel>,
        version: String,
    },
    History {
        channel: String,
        messages: Vec<ChatMessage>,
        before: Option<String>,
        missing_predecessors: Vec<String>,
    },
    Inbox {
        cursor: String,
        head: String,
        degraded: bool,
        conversations: Vec<ChatConversation>,
    },
    Read {
        channel: String,
        sequence: String,
    },
    Poll {
        bumped: bool,
        inbox_version: String,
    },
    Operation {
        operation: ChatOperation,
    },
    Pending {
        operations: Vec<ChatOperation>,
    },
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChatReply {
    pub scope: ChatScope,
    pub result: ChatResult,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn submissions_are_bounded_redacted_and_classified() {
        let action = ChatAction::PrepareMessage {
            submission: "ab".repeat(16),
            channel: "cd".repeat(16),
            text: SecretString::new("private hello"),
        };
        assert!(action.validate());
        assert!(action.is_mutation());
        assert!(!format!("{action:?}").contains("private hello"));
        let wire = serde_json::to_value(&action).unwrap();
        assert_eq!(serde_json::from_value::<ChatAction>(wire).unwrap(), action);
        assert!(!ChatAction::History {
            channel: "cd".repeat(16),
            before: Some("01".into())
        }
        .validate());
        assert_eq!(chat_sequence("9007199254740993"), Some(9007199254740993));
        assert!(!ChatAction::PrepareMessage {
            submission: "ab".repeat(16),
            channel: "cd".repeat(16),
            text: SecretString::new("x".repeat(CHAT_TEXT_BYTES + 1))
        }
        .validate());
        let poll = ChatAction::PollInbox {
            since: "9007199254740993".into(),
            timeout_milliseconds: CHAT_POLL_MILLISECONDS as u64,
        };
        assert!(poll.validate());
        assert!(!poll.is_mutation());
        assert!(ChatAction::MarkRead {
            channel: "cd".repeat(16),
            sequence: "1".into(),
        }
        .is_mutation());
        assert!(!ChatAction::SyncInbox.is_mutation());
    }
}
