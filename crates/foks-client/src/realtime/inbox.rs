use super::policy::ChatLimits;
use super::session::{ChatChannel, ChatSession, ChatTransport};
use crate::{Error, Result};
use foks_client_db::{ChatInboxScope, ChatInboxState, SoftStateStore};
use foks_proto::{
    RtAppId, RtChangedThreads, RtChannelId, RtGetChangedThreadsArgument, RtGetInboxVersionArgument,
    RtHostId, RtInboxKey, RtMessageType, RtPartyId, RtPollInbox, RtPollInboxArgument,
    RtReadThrough, RtReadThroughArgument, RtTeamId, RtUserId,
};
use foks_rpc::{RealtimeRequest, RealtimeResponse};
use zeroize::Zeroizing;

const DEFAULT_PAGE: u64 = 100;

#[derive(Clone, Eq, PartialEq)]
pub enum ChatPreviewContent {
    Text(Zeroizing<String>),
    Unsupported(RtMessageType),
}
impl std::fmt::Debug for ChatPreviewContent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text(_) => formatter.write_str("Text([REDACTED])"),
            Self::Unsupported(kind) => formatter.debug_tuple("Unsupported").field(kind).finish(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatPreview {
    pub sender: Option<RtPartyId>,
    pub send_time: u64,
    pub insert_time: u64,
    pub content: ChatPreviewContent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatConversation {
    pub channel: ChatChannel,
    pub inbox_version: u64,
    pub read_through: u64,
    pub pending_read: Option<u64>,
    pub unread: u64,
    pub hidden: bool,
    pub muted: bool,
    pub preview: Option<ChatPreview>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatInbox {
    pub host: RtHostId,
    pub actor: RtUserId,
    pub team: RtTeamId,
    pub cursor: u64,
    pub head: u64,
    pub degraded: bool,
    pub conversations: Vec<ChatConversation>,
    pub channels: Vec<ChatChannel>,
    pub read_retry_pending: bool,
    pub previews_incomplete: bool,
    pub blocked_channels: Vec<RtChannelId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatSyncResult {
    pub inbox: ChatInbox,
    pub applied: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChatPollResult {
    pub bumped: bool,
    pub inbox_version: u64,
}

impl ChatPollResult {
    pub fn checked(bumped: bool, inbox_version: u64, since: u64) -> Result<Self> {
        if inbox_version > i64::MAX as u64 || bumped != (inbox_version > since) {
            return Err(Error::ChatIntegrity("invalid inbox poll result"));
        }
        Ok(Self {
            bumped,
            inbox_version,
        })
    }
}

impl ChatSession<'_> {
    fn inbox_scope(&self) -> ChatInboxScope {
        ChatInboxScope {
            host: self.host.host_id().as_bytes().to_vec(),
            uid: self.credential.uid.as_bytes().to_vec(),
            app: RtAppId::Chat,
        }
    }

    pub fn sync_inbox(
        &mut self,
        rpc: &mut impl ChatTransport,
        store: &mut SoftStateStore,
    ) -> Result<ChatSyncResult> {
        self.sync_inbox_excluding_previews(rpc, store, &[])
    }

    /// Exclusions belong to the caller's trusted lifetime, never to durable state.
    /// Discovery and account delta verification still run for every sync.
    pub fn sync_inbox_excluding_previews(
        &mut self,
        rpc: &mut impl ChatTransport,
        store: &mut SoftStateStore,
        blocked: &[RtChannelId],
    ) -> Result<ChatSyncResult> {
        if blocked.len() > ChatLimits::CHANNELS {
            return Err(Error::ChatInvalidInput("too many blocked channels"));
        }
        self.refresh()?;
        let available = self.list_current_channels(rpc)?;
        let scope = self.inbox_scope();
        let RealtimeResponse::InboxVersion(remote_head) = rpc.request(
            &RealtimeRequest::GetInboxVersion(RtGetInboxVersionArgument {
                key: RtInboxKey { app: RtAppId::Chat },
            }),
        )?
        else {
            return Err(Error::ChatIntegrity("unexpected inbox version response"));
        };
        if remote_head > i64::MAX as u64 {
            return Err(Error::ChatIntegrity("inbox version overflow"));
        }
        let mut state = store.chat_inbox_state(&scope)?;
        if remote_head < state.head || remote_head < state.cursor {
            store.reset_chat_inbox(&scope)?;
            state = ChatInboxState::default();
        }
        let mut applied = 0u64;
        let mut maximum = DEFAULT_PAGE;
        let pages = if state.degraded && remote_head == state.head {
            0
        } else {
            ChatLimits::INBOX_PAGES
        };
        for _ in 0..pages {
            let RealtimeResponse::InboxDelta(delta) = rpc.request(
                &RealtimeRequest::GetChangedThreads(RtGetChangedThreadsArgument {
                    query: RtChangedThreads {
                        app: RtAppId::Chat,
                        since: state.cursor,
                        maximum,
                    },
                }),
            )?
            else {
                return Err(Error::ChatIntegrity("unexpected inbox delta response"));
            };
            if delta.app != RtAppId::Chat
                || delta.inbox_version > i64::MAX as u64
                || delta.inbox_version < state.cursor
                || delta.channels.len() > ChatLimits::INBOX_ROWS
                || delta.channels.len() > maximum as usize
            {
                return Err(Error::ChatIntegrity(
                    "invalid inbox delta identity or bounds",
                ));
            }
            if delta.channels.is_empty() {
                if delta.inbox_version > state.cursor && maximum < ChatLimits::INBOX_ROWS as u64 {
                    maximum = ChatLimits::INBOX_ROWS as u64;
                    continue;
                }
                state = store.observe_chat_inbox_head(
                    &scope,
                    delta.inbox_version,
                    delta.inbox_version > state.cursor,
                )?;
                break;
            }
            state = store.apply_chat_inbox_page(&scope, delta.inbox_version, &delta.channels)?;
            applied = applied
                .checked_add(delta.channels.len() as u64)
                .ok_or(Error::ChatLimit("inbox apply count overflow"))?;
            maximum = DEFAULT_PAGE;
            if state.cursor >= state.head {
                break;
            }
        }
        if state.cursor < state.head && !state.degraded {
            return Err(Error::ChatLimit("inbox pagination limit"));
        }
        let retained = available
            .channels
            .iter()
            .filter(|channel| {
                !channel.metadata.unreadable && self.role >= channel.metadata.roles.read
            })
            .map(|channel| channel.metadata.id)
            .collect::<Vec<_>>();
        store.retain_chat_inbox_team(&scope, self.team_id.as_bytes(), &retained)?;
        let read_retry_pending = match self.retry_pending_reads_current(rpc, store) {
            Ok(_) => false,
            Err(error) if optional_network_failure(&error) => true,
            Err(error) => return Err(error),
        };
        let mut inbox = self.inbox_from_store(store)?;
        inbox.channels = available.channels;
        inbox.read_retry_pending = read_retry_pending;
        self.hydrate_previews(rpc, &mut inbox, blocked)?;
        Ok(ChatSyncResult { inbox, applied })
    }

    fn hydrate_previews(
        &self,
        rpc: &mut impl ChatTransport,
        inbox: &mut ChatInbox,
        blocked: &[RtChannelId],
    ) -> Result<()> {
        inbox.blocked_channels = inbox
            .channels
            .iter()
            .filter(|c| blocked.contains(&c.metadata.id))
            .map(|c| c.metadata.id)
            .collect();
        for conversation in inbox
            .conversations
            .iter_mut()
            .take(ChatLimits::INBOX_PREVIEWS)
        {
            if inbox
                .blocked_channels
                .contains(&conversation.channel.metadata.id)
            {
                continue;
            }
            match self.preview(rpc, &conversation.channel) {
                Ok(preview) => conversation.preview = preview,
                Err(Error::ChatChannelIntegrity(_)) => {
                    // Keep independently verified channel/inbox facts, never publish bad content.
                    inbox
                        .blocked_channels
                        .push(conversation.channel.metadata.id);
                }
                Err(error) if optional_network_failure(&error) => {
                    inbox.previews_incomplete = true;
                    break;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn preview(
        &self,
        rpc: &mut impl ChatTransport,
        channel: &ChatChannel,
    ) -> Result<Option<ChatPreview>> {
        let Some(last) = channel.metadata.last_message.as_ref() else {
            return Ok(None);
        };
        let rows = self.range(rpc, channel.metadata.id, last.sequence, last.sequence)?;
        let mut history = self.verify_page(rpc, &channel.metadata, rows)?;
        if history.messages.len() != 1 {
            return Err(Error::ChatChannelIntegrity(
                "inbox preview message is missing",
            ));
        }
        let message = history.messages.pop().expect("checked preview length");
        Ok(Some(ChatPreview {
            sender: message.message.sender,
            send_time: message.message.metadata.send_time,
            insert_time: message.message.insert_time,
            content: match message.content {
                super::history::ChatContent::Text(text) => ChatPreviewContent::Text(text),
                super::history::ChatContent::Unsupported(kind) => {
                    ChatPreviewContent::Unsupported(kind)
                }
            },
        }))
    }

    pub fn inbox_from_store(&self, store: &SoftStateStore) -> Result<ChatInbox> {
        self.team()?;
        let scope = self.inbox_scope();
        let state = store.chat_inbox_state(&scope)?;
        let mut conversations = Vec::new();
        for entry in store.chat_inbox_entries(&scope, self.team_id.as_bytes())? {
            let channel = self.open_channel(entry.metadata)?;
            if channel.metadata.unreadable || self.role < channel.metadata.roles.read {
                continue;
            }
            let effective_read = entry.pending_read.unwrap_or(0).max(entry.read_through);
            let unread = channel
                .metadata
                .last_message
                .as_ref()
                .map_or(0, |last| last.sequence.saturating_sub(effective_read));
            conversations.push(ChatConversation {
                channel,
                inbox_version: entry.inbox_version,
                read_through: entry.read_through,
                pending_read: entry.pending_read,
                unread,
                hidden: entry.hidden,
                muted: entry.muted,
                preview: None,
            });
        }
        Ok(ChatInbox {
            host: RtHostId::new(self.host.host_id().clone())?,
            actor: RtUserId::new(self.credential.uid.clone())?,
            team: RtTeamId::new(self.team_id.clone())?,
            cursor: state.cursor,
            head: state.head,
            degraded: state.degraded,
            channels: conversations.iter().map(|c| c.channel.clone()).collect(),
            read_retry_pending: conversations.iter().any(|c| c.pending_read.is_some()),
            previews_incomplete: false,
            blocked_channels: Vec::new(),
            conversations,
        })
    }

    pub fn mark_read(
        &mut self,
        rpc: &mut impl ChatTransport,
        store: &mut SoftStateStore,
        channel: RtChannelId,
        sequence: u64,
    ) -> Result<()> {
        self.refresh()?;
        let metadata = self.channel(rpc, channel, false)?;
        if sequence == 0
            || metadata
                .last_message
                .as_ref()
                .is_none_or(|last| sequence > last.sequence)
        {
            return Err(Error::ChatInvalidInput(
                "read pointer exceeds displayed messages",
            ));
        }
        let scope = self.inbox_scope();
        store.stage_chat_read(&scope, channel, sequence)?;
        let response = rpc.request(&RealtimeRequest::ReadThrough(RtReadThroughArgument {
            read: RtReadThrough { channel, sequence },
        }));
        match response {
            Ok(RealtimeResponse::Void) => {
                store.confirm_chat_read(&scope, channel, sequence)?;
                Ok(())
            }
            Ok(_) => Err(Error::ChatIntegrity("unexpected read-through response")),
            Err(error) => Err(error),
        }
    }

    pub fn retry_pending_reads(
        &mut self,
        rpc: &mut impl ChatTransport,
        store: &mut SoftStateStore,
    ) -> Result<usize> {
        self.refresh()?;
        self.retry_pending_reads_current(rpc, store)
    }

    fn retry_pending_reads_current(
        &self,
        rpc: &mut impl ChatTransport,
        store: &mut SoftStateStore,
    ) -> Result<usize> {
        let scope = self.inbox_scope();
        let entries = store.chat_inbox_entries(&scope, self.team_id.as_bytes())?;
        let mut completed = 0;
        for entry in entries {
            let Some(sequence) = entry.pending_read else {
                continue;
            };
            let response = rpc.request(&RealtimeRequest::ReadThrough(RtReadThroughArgument {
                read: RtReadThrough {
                    channel: entry.metadata.id,
                    sequence,
                },
            }))?;
            if response != RealtimeResponse::Void {
                return Err(Error::ChatIntegrity("unexpected read-through response"));
            }
            store.confirm_chat_read(&scope, entry.metadata.id, sequence)?;
            completed += 1;
        }
        Ok(completed)
    }

    pub fn poll_inbox(
        &self,
        rpc: &mut impl ChatTransport,
        since: u64,
        timeout_milliseconds: u64,
    ) -> Result<ChatPollResult> {
        if since > i64::MAX as u64 || timeout_milliseconds > 55_000 {
            return Err(Error::ChatInvalidInput("invalid inbox poll bounds"));
        }
        let RealtimeResponse::PollResult(result) =
            rpc.request(&RealtimeRequest::PollInbox(RtPollInboxArgument {
                poll: RtPollInbox {
                    app: RtAppId::Chat,
                    since,
                    timeout_milliseconds,
                },
            }))?
        else {
            return Err(Error::ChatIntegrity("unexpected inbox poll response"));
        };
        ChatPollResult::checked(result.bumped, result.inbox_version, since)
    }
}

#[cfg(test)]
mod poll_validation_tests {
    use super::*;
    #[test]
    fn poll_validation_accepts_rollback_but_rejects_contradictions() {
        assert!(ChatPollResult::checked(false, 2, 5).is_ok());
        assert!(ChatPollResult::checked(true, 6, 5).is_ok());
        assert!(ChatPollResult::checked(true, 5, 5).is_err());
        assert!(ChatPollResult::checked(false, 6, 5).is_err());
        assert!(ChatPollResult::checked(true, u64::MAX, 5).is_err());
    }
}

// Only transport loss is optional. Shared integrity and permission failures
// propagate; channel content failures are explicitly quarantined by hydration.
fn optional_network_failure(error: &Error) -> bool {
    matches!(
        error,
        Error::Connect(_) | Error::DeadlineExceeded | Error::Rpc(foks_rpc::Error::Io(_))
    )
}
