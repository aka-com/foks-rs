use super::policy::{ChatLimits, MESSAGE_ANCHOR_HASH_DOMAIN};
use super::session::{ChatSession, ChatTransport};
use crate::{Error, Result};
use foks_client_db::ChatAnchor;
use foks_proto::{
    RealtimeWire, RtChannelId, RtChannelMetadata, RtGetThreadArgument, RtMessage, RtMessageType,
    RtMessageWrapper, RtRecentsArgument, RtThreadQuery, RtThreadRange, ENTITY_USER,
};
use foks_rpc::{RealtimeRequest, RealtimeResponse};
use std::collections::{BTreeMap, BTreeSet};
use zeroize::Zeroizing;

pub enum ChatContent {
    Text(Zeroizing<String>),
    Unsupported(RtMessageType),
}
pub struct ChatMessage {
    pub message: RtMessage,
    pub content: ChatContent,
}
pub struct ChatHistory {
    pub scope: foks_client_db::ChatScope,
    pub messages: Vec<ChatMessage>,
    /// Missing predecessors are incomplete evidence, never a successful check.
    pub missing_predecessors: Vec<u64>,
}
pub(super) fn anchor(m: &RtMessage) -> Result<ChatAnchor> {
    Ok(ChatAnchor {
        sequence: m.sequence,
        id: m.metadata.id.0,
        digest: foks_crypto::prefixed_hash(MESSAGE_ANCHOR_HASH_DOMAIN, &m.encoded()?),
    })
}
impl ChatSession<'_> {
    /// Newest messages first, with the same verification as explicit windows.
    pub fn read_recent(
        &mut self,
        rpc: &mut impl ChatTransport,
        channel: RtChannelId,
        limit: u64,
    ) -> Result<ChatHistory> {
        if !(1..=ChatLimits::HISTORY_ROWS as u64).contains(&limit) {
            return Err(Error::ChatInvalidInput(
                "recent page limit must be 1 to 1000",
            ));
        }
        self.refresh()?;
        let md = self.channel(rpc, channel, false)?;
        let RealtimeResponse::Messages(page) =
            rpc.request(&RealtimeRequest::Recents(RtRecentsArgument {
                channel,
                limit,
                stop_at: 0,
            }))?
        else {
            return Err(Error::ChatIntegrity("unexpected recent response"));
        };
        if page.messages.len() > limit as usize
            || page
                .messages
                .windows(2)
                .any(|pair| pair[0].sequence <= pair[1].sequence)
        {
            return Err(Error::ChatChannelIntegrity(
                "recent ordering or limit mismatch",
            ));
        }
        self.verify_page(rpc, &md, page.messages)
    }
    /// One bounded history window. Callers serialize reads per checked profile.
    pub fn read_thread(
        &mut self,
        rpc: &mut impl ChatTransport,
        channel: RtChannelId,
        start: u64,
        end: u64,
    ) -> Result<ChatHistory> {
        self.refresh()?;
        let md = self.channel(rpc, channel, false)?;
        let rows = self.range(rpc, channel, start, end)?;
        self.verify_page(rpc, &md, rows)
    }
    pub(super) fn range(
        &self,
        rpc: &mut impl ChatTransport,
        channel: RtChannelId,
        start: u64,
        end: u64,
    ) -> Result<Vec<RtMessage>> {
        if start == 0
            || start.abs_diff(end) >= ChatLimits::HISTORY_ROWS as u64
            || end == 0
            || start > i64::MAX as u64
            || end > i64::MAX as u64
        {
            return Err(Error::ChatInvalidInput(
                "history window must contain 1 to 1000 sequences",
            ));
        }
        let RealtimeResponse::Thread(mut page) =
            rpc.request(&RealtimeRequest::GetThread(RtGetThreadArgument {
                query: RtThreadQuery {
                    channel,
                    ranges: vec![RtThreadRange { start, end }],
                    sequences: vec![],
                },
            }))?
        else {
            return Err(Error::ChatIntegrity("unexpected history response"));
        };
        if page.ranges.len() != 1 || !page.sequences.is_empty() {
            return Err(Error::ChatChannelIntegrity("history grouping mismatch"));
        }
        let rows = page.ranges.remove(0).messages;
        let mut last = None;
        for m in &rows {
            if m.sequence < start.min(end)
                || m.sequence > start.max(end)
                || last.is_some_and(|prev| {
                    if start <= end {
                        m.sequence <= prev
                    } else {
                        m.sequence >= prev
                    }
                })
            {
                return Err(Error::ChatChannelIntegrity("history range/order mismatch"));
            }
            last = Some(m.sequence);
        }
        Ok(rows)
    }
    pub(super) fn verify_page(
        &self,
        rpc: &mut impl ChatTransport,
        md: &RtChannelMetadata,
        rows: Vec<RtMessage>,
    ) -> Result<ChatHistory> {
        if rows.len() > ChatLimits::HISTORY_ROWS
            || rows.iter().try_fold(0usize, |sum, m| {
                Ok::<_, Error>(sum.saturating_add(m.encoded()?.len()))
            })? > ChatLimits::HISTORY_BYTES
        {
            return Err(Error::ChatLimit("history page limit"));
        }
        let scope = self.scope(md.id);
        let mut hard = self.hard()?;
        let mut known = BTreeMap::new();
        let mut ids = BTreeSet::new();
        let mut seqs = BTreeSet::new();
        let mut anchors = Vec::new();
        let mut observed = Vec::new();
        let mut messages = Vec::new();
        for m in rows {
            if !ids.insert(m.metadata.id.0) || !seqs.insert(m.sequence) {
                return Err(Error::ChatChannelIntegrity("duplicate message mapping"));
            }
            observed.push(anchor(&m)?);
            let content = self.open_message(md, &m)?;
            if matches!(content, ChatContent::Text(_)) {
                known.insert(m.sequence, m.metadata.id.0);
                anchors.push(anchor(&m)?);
            }
            messages.push(ChatMessage {
                message: m,
                content,
            });
        }
        let mut need = BTreeSet::new();
        for row in &messages {
            let m = &row.message;
            if !matches!(row.content, ChatContent::Text(_)) || m.metadata.previous_sequence == 0 {
                continue;
            }
            let prev = m.metadata.previous_sequence;
            let id = known
                .get(&prev)
                .copied()
                .or(hard.chat_anchor(&scope, prev)?.map(|a| a.id));
            if let Some(id) = id {
                if id != m.metadata.previous_id.0 {
                    return Err(Error::ChatChannelIntegrity("predecessor ID mismatch"));
                }
            } else {
                need.insert(prev);
            }
        }
        let requested: Vec<_> = need
            .iter()
            .copied()
            .take(ChatLimits::PREDECESSORS)
            .collect();
        if !requested.is_empty() {
            let RealtimeResponse::Thread(page) =
                rpc.request(&RealtimeRequest::GetThread(RtGetThreadArgument {
                    query: RtThreadQuery {
                        channel: md.id,
                        ranges: vec![],
                        sequences: requested.clone(),
                    },
                }))?
            else {
                return Err(Error::ChatIntegrity("unexpected predecessor response"));
            };
            if !page.ranges.is_empty() || page.sequences.len() > requested.len() {
                return Err(Error::ChatChannelIntegrity("predecessor response mismatch"));
            }
            let mut seen = BTreeSet::new();
            for m in page.sequences {
                if !requested.contains(&m.sequence) || !seen.insert(m.sequence) {
                    return Err(Error::ChatChannelIntegrity("unrequested predecessor"));
                }
                observed.push(anchor(&m)?);
                if matches!(self.open_message(md, &m)?, ChatContent::Text(_)) {
                    for row in &messages {
                        if matches!(row.content, ChatContent::Text(_))
                            && row.message.metadata.previous_sequence == m.sequence
                            && row.message.metadata.previous_id != m.metadata.id
                        {
                            return Err(Error::ChatChannelIntegrity("predecessor contradiction"));
                        }
                    }
                    need.remove(&m.sequence);
                    anchors.push(anchor(&m)?);
                }
            }
        }
        hard.chat_accept_page(&scope, &observed, &anchors)
            .map_err(|error| match error {
                foks_client_db::Error::ChatConflict(_) => {
                    Error::ChatChannelIntegrity("message anchor contradiction")
                }
                other => Error::from(other),
            })?;
        Ok(ChatHistory {
            scope,
            messages,
            missing_predecessors: need.into_iter().collect(),
        })
    }
    fn open_message(&self, md: &RtChannelMetadata, m: &RtMessage) -> Result<ChatContent> {
        if m.sequence == 0
            || m.sequence > i64::MAX as u64
            || m.metadata.id.0 == [0; 16]
            || m.metadata.previous_sequence >= m.sequence
            || (m.metadata.previous_sequence == 0) != (m.metadata.previous_id.0 == [0; 16])
        {
            return Err(Error::ChatChannelIntegrity(
                "invalid message sequence metadata",
            ));
        }
        let sender = m
            .sender
            .as_ref()
            .ok_or(Error::ChatChannelIntegrity("message sender missing"))?;
        if sender.entity().entity_type() != ENTITY_USER {
            return Err(Error::ChatUnsupported("unsupported message sender"));
        }
        if m.metadata.kind != RtMessageType::Basic {
            return Ok(ChatContent::Unsupported(m.metadata.kind));
        }
        if m.metadata.further_user_attribution.is_some() {
            return Err(Error::ChatUnsupported("unsupported message attribution"));
        }
        let RtMessageWrapper::Encrypted(b) = &m.wrapper else {
            return Err(Error::ChatChannelIntegrity("unencrypted chat message"));
        };
        if b.key.role != md.roles.read {
            return Err(Error::ChatChannelIntegrity("message read role mismatch"));
        }
        let bytes = self
            .keys(b.key)?
            .open_basic_message(
                &self.noncer(md.id, m.metadata.clone(), sender.entity().clone()),
                &b.ciphertext,
            )
            .map_err(|_| Error::ChatChannelIntegrity("message authentication failed"))?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| Error::ChatChannelIntegrity("Basic text is not UTF-8"))?;
        Ok(ChatContent::Text(Zeroizing::new(text.to_owned())))
    }
}
