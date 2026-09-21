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

/// Identifies one cached inbox preview. The key names the reader, the
/// channel and the exact message the preview renders, together with the role
/// and key generation the reader currently holds for that channel, so a
/// preview is never served to another reader, for another message, or across
/// a key rotation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatPreviewKey {
    pub host: Vec<u8>,
    pub actor: Vec<u8>,
    pub team: Vec<u8>,
    pub channel: [u8; 16],
    pub sequence: u64,
    pub role: foks_proto::Role,
    pub generation: u64,
}

/// A preview store the caller owns. `foks-client` keeps no process-global
/// state, so a process that wants previews to survive one sync supplies this.
pub trait ChatPreviewCache {
    fn get(&mut self, key: &ChatPreviewKey) -> Option<ChatPreview>;
    fn put(&mut self, key: ChatPreviewKey, preview: ChatPreview);
}

/// The cache that keeps nothing, for a caller that wants today's behavior.
pub struct NoChatPreviewCache;

impl ChatPreviewCache for NoChatPreviewCache {
    fn get(&mut self, _key: &ChatPreviewKey) -> Option<ChatPreview> {
        None
    }
    fn put(&mut self, _key: ChatPreviewKey, _preview: ChatPreview) {}
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
    /// Whether this sync took the account scope to the head it was told,
    /// applying every changed thread the server had, with no degraded
    /// observation anywhere in the loop. The inbox scope is host, user and
    /// application with no team in it, so a caller syncing several teams of
    /// one account may use this to skip the version and delta phases for the
    /// rest of a cycle: every row the drain applied is in the store,
    /// whichever team it belongs to. It is false on a partial page, on the
    /// degraded path and on a sync that skipped the drain — a drain that
    /// did not finish is never evidence that the account is current.
    pub drained: bool,
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
        self.sync_inbox_with_preview_cache(rpc, store, blocked, &mut NoChatPreviewCache)
    }

    /// [`Self::sync_inbox_excluding_previews`] served by a caller-owned
    /// preview cache. A conversation whose last message is already cached
    /// under the reader's current key costs no preview round trip; every
    /// other check of the sync is unchanged, and access is re-derived from
    /// the freshly authenticated team on every call.
    pub fn sync_inbox_with_preview_cache(
        &mut self,
        rpc: &mut impl ChatTransport,
        store: &mut SoftStateStore,
        blocked: &[RtChannelId],
        previews: &mut dyn ChatPreviewCache,
    ) -> Result<ChatSyncResult> {
        self.sync_inbox_gated(rpc, store, blocked, previews, true)
    }

    /// Synchronizes a team inbox with caller-controlled account-level draining.
    /// The version and delta requests use a cursor shared by host, user, and
    /// application; team filtering occurs after changed rows are stored. When
    /// `drain` is false, the method reads stored rows without marking the account
    /// current, because no inbox head was verified by this call.
    pub fn sync_inbox_gated(
        &mut self,
        rpc: &mut impl ChatTransport,
        store: &mut SoftStateStore,
        blocked: &[RtChannelId],
        previews: &mut dyn ChatPreviewCache,
        drain: bool,
    ) -> Result<ChatSyncResult> {
        if blocked.len() > ChatLimits::CHANNELS {
            return Err(Error::ChatInvalidInput("too many blocked channels"));
        }
        self.refresh()?;
        let available = self.list_current_channels(rpc)?;
        let scope = self.inbox_scope();
        let (applied, drained) = if drain {
            let (state, applied) = drain_inbox_scope(rpc, store, &scope)?;
            (applied, scope_drained(&state))
        } else {
            (0, false)
        };
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
        let mut inbox = self.inbox_from_store_with_channels(store, &available.channels)?;
        inbox.channels = available.channels;
        inbox.read_retry_pending = read_retry_pending;
        self.hydrate_previews(rpc, &mut inbox, blocked, previews)?;
        Ok(ChatSyncResult {
            inbox,
            applied,
            drained,
        })
    }

    /// The key a preview of `channel`'s last message is cached under, or
    /// `None` when the channel has no last message or the reader holds no
    /// current key for its read role. A channel with no current key is never
    /// cached, so a reader that has lost the role cannot be served one.
    fn preview_key(&self, channel: &ChatChannel) -> Option<ChatPreviewKey> {
        let last = channel.metadata.last_message.as_ref()?;
        let role = channel.metadata.roles.read;
        let generation = self.team().ok()?.verified.shared_key(role)?.generation;
        Some(ChatPreviewKey {
            host: self.host.host_id().as_bytes().to_vec(),
            actor: self.credential.uid.as_bytes().to_vec(),
            team: self.team_id.as_bytes().to_vec(),
            channel: channel.metadata.id.0,
            sequence: last.sequence,
            role,
            generation,
        })
    }

    fn hydrate_previews(
        &self,
        rpc: &mut impl ChatTransport,
        inbox: &mut ChatInbox,
        blocked: &[RtChannelId],
        previews: &mut dyn ChatPreviewCache,
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
            let key = self.preview_key(&conversation.channel);
            if let Some(cached) = key.as_ref().and_then(|key| previews.get(key)) {
                conversation.preview = Some(cached);
                continue;
            }
            match self.preview(rpc, &conversation.channel) {
                Ok(preview) => {
                    if let (Some(key), Some(preview)) = (key, preview.as_ref()) {
                        previews.put(key, preview.clone());
                    }
                    conversation.preview = preview;
                }
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
        self.inbox_from_store_with_channels(store, &[])
    }

    /// [`Self::inbox_from_store`] given channels this sync already opened.
    /// A stored entry whose metadata is exactly what the listing opened
    /// reuses that channel instead of decrypting its name and description a
    /// second time; any other entry is opened as before.
    pub fn inbox_from_store_with_channels(
        &self,
        store: &SoftStateStore,
        opened: &[ChatChannel],
    ) -> Result<ChatInbox> {
        self.team()?;
        let scope = self.inbox_scope();
        let state = store.chat_inbox_state(&scope)?;
        let mut conversations = Vec::new();
        for entry in store.chat_inbox_entries(&scope, self.team_id.as_bytes())? {
            let reusable = opened
                .iter()
                .find(|channel| channel.metadata == entry.metadata);
            let channel = match reusable {
                // The identity and policy checks still run against this
                // session's own role; only the name and description are
                // taken as already decrypted.
                Some(channel) => {
                    self.validate_channel(&channel.metadata)?;
                    channel.clone()
                }
                None => self.open_channel(entry.metadata)?,
            };
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

/// Returns whether synchronization reached the reported inbox head without
/// encountering an incomplete server response.
fn scope_drained(state: &ChatInboxState) -> bool {
    !state.degraded && state.cursor >= state.head
}

/// Reads the account inbox version and applies changed-thread pages until the
/// cursor reaches the reported head. Returns the final state and applied-row
/// count. Stored rows retain their team identifiers for subsequent filtering.
fn drain_inbox_scope(
    rpc: &mut impl ChatTransport,
    store: &mut SoftStateStore,
    scope: &ChatInboxScope,
) -> Result<(ChatInboxState, u64)> {
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
    let mut state = store.chat_inbox_state(scope)?;
    if remote_head < state.head || remote_head < state.cursor {
        store.reset_chat_inbox(scope)?;
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
                scope,
                delta.inbox_version,
                delta.inbox_version > state.cursor,
            )?;
            break;
        }
        state = store.apply_chat_inbox_page(scope, delta.inbox_version, &delta.channels)?;
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
    Ok((state, applied))
}

// Only transport loss is optional. Shared integrity and permission failures
// propagate; channel content failures are explicitly quarantined by hydration.
fn optional_network_failure(error: &Error) -> bool {
    matches!(
        error,
        Error::Connect(_) | Error::DeadlineExceeded | Error::Rpc(foks_rpc::Error::Io(_))
    )
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

#[cfg(test)]
mod drain_tests {
    use super::*;
    use foks_proto::{
        EntityId, RoleAndGeneration, RtBox, RtChannelMetadata, RtChannelTier, RtInboxChannel,
        RtInboxDelta, RtRolePair, SecretBox, ENTITY_HOST, ENTITY_NAMED_TEAM, ENTITY_USER,
    };

    /// One reply per request, in order, so a test states exactly what the
    /// server answered and the drain's round trips are counted, not inferred.
    struct Replies {
        requests: Vec<RealtimeRequest>,
        replies: std::collections::VecDeque<RealtimeResponse>,
    }
    impl Replies {
        fn new(replies: Vec<RealtimeResponse>) -> Self {
            Self {
                requests: Vec::new(),
                replies: replies.into(),
            }
        }
        fn counts(&self) -> (usize, usize) {
            (
                self.requests
                    .iter()
                    .filter(|r| matches!(r, RealtimeRequest::GetInboxVersion(_)))
                    .count(),
                self.requests
                    .iter()
                    .filter(|r| matches!(r, RealtimeRequest::GetChangedThreads(_)))
                    .count(),
            )
        }
    }
    impl ChatTransport for Replies {
        fn request(&mut self, request: &RealtimeRequest) -> Result<RealtimeResponse> {
            self.requests.push(request.clone());
            self.replies
                .pop_front()
                .ok_or(Error::ChatIntegrity("unexpected extra request"))
        }
    }

    fn entity(kind: u8, tag: u8) -> EntityId {
        let mut bytes = vec![kind];
        bytes.extend_from_slice(&[tag; 32]);
        EntityId::from_bytes(bytes).expect("entity")
    }
    fn scope() -> ChatInboxScope {
        ChatInboxScope {
            host: entity(ENTITY_HOST, 1).as_bytes().to_vec(),
            uid: entity(ENTITY_USER, 2).as_bytes().to_vec(),
            app: RtAppId::Chat,
        }
    }
    fn store(directory: &tempfile::TempDir, name: &str) -> SoftStateStore {
        SoftStateStore::open(&directory.path().join(name)).expect("soft store")
    }
    /// A changed thread in the account's delta. `team` names which team owns
    /// it: the scope has no team in it, so one drain applies every team's.
    fn row(version: u64, channel: u8, team: u8) -> RtInboxChannel {
        let boxed = RtBox {
            key: RoleAndGeneration {
                role: foks_proto::Role::member(0),
                generation: 1,
            },
            boxed: SecretBox {
                nonce: [0; 16],
                ciphertext: vec![7; 32],
            },
        };
        RtInboxChannel {
            metadata: RtChannelMetadata {
                id: RtChannelId([channel; 16]),
                team: foks_proto::RtTeamId::new(entity(ENTITY_NAMED_TEAM, team)).expect("team"),
                app: RtAppId::Chat,
                sequence: 1,
                name: boxed.clone(),
                description: None,
                roles: RtRolePair {
                    read: foks_proto::Role::member(0),
                    write: foks_proto::Role::member(0),
                },
                last_message: None,
                ctime: 1,
                mtime: 1,
                updated_at: 1,
                tier: RtChannelTier::Bottom,
                unreadable: false,
            },
            inbox_version: version,
            read_through: 0,
            hidden: false,
            muted: false,
        }
    }
    fn delta(version: u64, channels: Vec<RtInboxChannel>) -> RealtimeResponse {
        RealtimeResponse::InboxDelta(RtInboxDelta {
            inbox_version: version,
            app: RtAppId::Chat,
            channels,
        })
    }

    /// The account scope is drained once per cycle however many teams sync:
    /// the first pays the version query and the delta page, and the rest are
    /// gated off by the caller and pay neither.
    #[test]
    fn teams_of_one_account_drain_the_scope_once_and_share_every_row() {
        let directory = tempfile::tempdir().expect("temp dir");
        let mut soft = store(&directory, "inbox.sqlite3");
        let scope = scope();
        let mut rpc = Replies::new(vec![
            RealtimeResponse::InboxVersion(2),
            delta(2, vec![row(1, 0xa1, 0xb1), row(2, 0xa2, 0xb2)]),
        ]);
        let (state, applied) = drain_inbox_scope(&mut rpc, &mut soft, &scope).expect("drain");
        assert_eq!(applied, 2);
        assert!(scope_drained(&state));
        assert_eq!(rpc.counts(), (1, 1));
        // Both teams' rows landed under the one account scope, so a team that
        // skips the drain still reads its own changed thread.
        for team in [0xb1, 0xb2] {
            assert_eq!(
                soft.chat_inbox_entries(&scope, entity(ENTITY_NAMED_TEAM, team).as_bytes())
                    .expect("entries")
                    .len(),
                1
            );
        }
        // The four other teams of this account sync behind the gate in the
        // same cycle. They run no drain, so the cycle's cost stays at the one
        // version query and the one delta page above; a transport holding no
        // replies proves nothing was asked of the server.
        let mut gated = Replies::new(vec![]);
        let mut drained = scope_drained(&state);
        for _ in 0..4 {
            if !drained {
                let (left, _) =
                    drain_inbox_scope(&mut gated, &mut soft, &scope).expect("gated drain");
                drained = scope_drained(&left);
            }
        }
        assert_eq!(gated.counts(), (0, 0));
    }

    /// A server that reports a newer version but enumerates no change leaves
    /// the scope degraded. That is not a drain, and the gate must not be set
    /// from it: the rows behind that version have never been read.
    #[test]
    fn a_degraded_delta_never_reports_the_scope_drained() {
        let directory = tempfile::tempdir().expect("temp dir");
        let mut soft = store(&directory, "degraded.sqlite3");
        let scope = scope();
        let mut rpc = Replies::new(vec![
            RealtimeResponse::InboxVersion(9),
            delta(9, vec![]),
            delta(9, vec![]),
        ]);
        let (state, applied) = drain_inbox_scope(&mut rpc, &mut soft, &scope).expect("drain");
        assert_eq!(applied, 0);
        assert!(state.degraded);
        assert!(state.cursor < state.head);
        assert!(!scope_drained(&state));
    }

    /// Pages that make progress without reaching the head exhaust the page
    /// budget and fail. The caller never receives a result, so it never marks
    /// the scope drained on a partial delta.
    #[test]
    fn a_partial_page_never_reports_the_scope_drained() {
        let directory = tempfile::tempdir().expect("temp dir");
        let mut soft = store(&directory, "partial.sqlite3");
        let scope = scope();
        let head = ChatLimits::INBOX_PAGES as u64 + 10;
        let mut replies = vec![RealtimeResponse::InboxVersion(head)];
        for version in 1..=ChatLimits::INBOX_PAGES as u64 {
            replies.push(delta(head, vec![row(version, version as u8, 0xb1)]));
        }
        let mut rpc = Replies::new(replies);
        let error = drain_inbox_scope(&mut rpc, &mut soft, &scope).expect_err("pagination limit");
        assert!(matches!(error, Error::ChatLimit(_)));
        // The state the partial pages left behind is not a drained scope.
        let state = soft.chat_inbox_state(&scope).expect("state");
        assert!(state.cursor < state.head);
        assert!(!scope_drained(&state));
    }

    /// An empty delta at the cursor is a complete drain: the account holds
    /// every changed thread, and there is nothing degraded about it.
    #[test]
    fn an_empty_delta_at_the_head_is_a_drain() {
        let directory = tempfile::tempdir().expect("temp dir");
        let mut soft = store(&directory, "head.sqlite3");
        let scope = scope();
        let mut rpc = Replies::new(vec![RealtimeResponse::InboxVersion(0), delta(0, vec![])]);
        let (state, applied) = drain_inbox_scope(&mut rpc, &mut soft, &scope).expect("drain");
        assert_eq!(applied, 0);
        assert!(scope_drained(&state));
        assert_eq!(rpc.counts(), (1, 1));
    }
}
