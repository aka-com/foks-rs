use super::policy::ChatLimits;
use super::session::{ChatChannel, ChatSession, ChatTransport};
use crate::{Error, Result};
use foks_client_db::{ChatInboxScope, ChatInboxState, SoftStateStore};
use foks_proto::{
    RtAppId, RtChangedThreads, RtChannelId, RtGetChangedThreadsArgument, RtGetInboxVersionArgument,
    RtHostId, RtInboxKey, RtPollInbox, RtPollInboxArgument, RtReadThrough, RtReadThroughArgument,
    RtTeamId, RtUserId,
};
use foks_rpc::{RealtimeRequest, RealtimeResponse};

const DEFAULT_PAGE: u64 = 100;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatConversation {
    pub channel: ChatChannel,
    pub inbox_version: u64,
    pub read_through: u64,
    pub pending_read: Option<u64>,
    pub unread: u64,
    pub hidden: bool,
    pub muted: bool,
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
            return Err(Error::ChatLimit("inbox version overflow"));
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
        Ok(ChatSyncResult {
            inbox: self.inbox_from_store(store)?,
            applied,
        })
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
            });
        }
        Ok(ChatInbox {
            host: RtHostId::new(self.host.host_id().clone())?,
            actor: RtUserId::new(self.credential.uid.clone())?,
            team: RtTeamId::new(self.team_id.clone())?,
            cursor: state.cursor,
            head: state.head,
            degraded: state.degraded,
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
        if result.inbox_version > i64::MAX as u64
            || (result.bumped && result.inbox_version <= since)
            || (!result.bumped && result.inbox_version > since)
        {
            return Err(Error::ChatIntegrity("invalid inbox poll result"));
        }
        Ok(ChatPollResult {
            bumped: result.bumped,
            inbox_version: result.inbox_version,
        })
    }
}
