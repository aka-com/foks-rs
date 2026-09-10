use super::history::anchor;
use super::policy::{ChatLimits, PROTECTED_MATERIAL_DOMAIN, REQUEST_HASH_DOMAIN};
use super::session::{floor, random_id, ChatSession, ChatTransport};
use crate::{Error, ProtectedMutationStore, Result};
use foks_client_db::{
    ChatOperation, ChatOperationKind as Kind, ChatOperationState as State, ChatSubmission,
};
use foks_proto::{
    RealtimeWire, Role, RtAppId, RtBox, RtChannelId, RtChannelMetadata, RtChannelTier,
    RtCreateChannelArgument, RtKeyType, RtMessage, RtMessageBox, RtMessageId, RtMessageMetadata,
    RtMessageType, RtMessageWrapper, RtPartyId, RtRecentsArgument, RtRolePair, RtSend,
    RtSendArgument, RtSendResult, RtTeamId, RtText, RT_MAX_BODY_BYTES,
};
use foks_rpc::{RealtimeRequest, RealtimeResponse};
use zeroize::Zeroizing;
mod names {
    include!("name_ranges.rs");
}

/// Go uses simple Unicode lowercase mapping (not contextual/full case folding).
pub fn normalize_chat_name(name: &str) -> Result<String> {
    let name: String = name
        .trim()
        .chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect();
    if name.is_empty() {
        return Ok(name);
    }
    if !(ChatLimits::NAME_MIN_CHARS..=ChatLimits::NAME_MAX_CHARS).contains(&name.chars().count())
        || name == "general"
        || name.contains("--")
        || name.chars().any(|c| {
            !names::NAME_RANGES
                .iter()
                .any(|&(lo, hi)| (lo..=hi).contains(&(c as u32)))
        })
    {
        return Err(Error::ChatInvalidInput(
            "invalid channel name; use empty name for general",
        ));
    }
    Ok(name)
}
impl ChatSession<'_> {
    /// Resolve a durable submission without connecting or loading protected material.
    pub fn submitted_operation(
        &self,
        submission: Option<&ChatSubmission>,
    ) -> Result<Option<ChatOperation>> {
        match submission {
            Some(submission) => Ok(self
                .hard()?
                .chat_submission(&self.scope(RtChannelId([0; 16])), submission)?),
            None => Ok(None),
        }
    }

    /// Persist first; submit in a later checked application operation.
    pub fn prepare_channel(
        &mut self,
        rpc: &mut impl ChatTransport,
        store: &mut impl ProtectedMutationStore,
        name: &str,
        description: &str,
        tier: RtChannelTier,
    ) -> Result<ChatOperation> {
        self.prepare_channel_submission(rpc, store, name, description, tier, None)
    }
    pub fn prepare_channel_submission(
        &mut self,
        rpc: &mut impl ChatTransport,
        store: &mut impl ProtectedMutationStore,
        name: &str,
        description: &str,
        tier: RtChannelTier,
        submission: Option<&ChatSubmission>,
    ) -> Result<ChatOperation> {
        if let Some(operation) = self.submitted_operation(submission)? {
            return Ok(operation);
        }

        self.refresh()?;
        let name = RtText(normalize_chat_name(name)?);
        let desc = RtText(
            description
                .chars()
                .map(|c| c.to_lowercase().next().unwrap_or(c))
                .collect(),
        );
        if !desc.0.is_empty()
            && !(ChatLimits::DESCRIPTION_MIN_CHARS..=ChatLimits::DESCRIPTION_MAX_CHARS)
                .contains(&desc.0.chars().count())
        {
            return Err(Error::ChatInvalidInput(
                "description must contain 3 to 512 characters",
            ));
        }
        let set = self.list_current_channels(rpc)?;
        if set
            .channels
            .iter()
            .any(|c| c.metadata.tier == tier && c.name.0 == name.0)
        {
            return Err(Error::ChatNameConflict(
                "channel name already exists in tier",
            ));
        }
        let read = if tier == RtChannelTier::Admin {
            Role::ADMIN
        } else {
            Role::member(0)
        };
        if self.role < read {
            return Err(Error::ChatAccessDenied(
                "member role required to create channel",
            ));
        }
        let name_key = self.current(floor(tier)?)?;
        let desc_key = self.current(read)?;
        let id = random_id()?;
        let version = set
            .version
            .checked_add(1)
            .filter(|v| *v <= i64::MAX as u64)
            .ok_or(Error::ChatLimit("channel version overflow"))?;
        let metadata = RtChannelMetadata {
            id: RtChannelId(id),
            team: RtTeamId::new(self.team_id.clone())?,
            app: RtAppId::Chat,
            sequence: 1,
            name: RtBox {
                key: name_key,
                boxed: self.keys(name_key)?.seal_text(
                    RtKeyType::ChannelName,
                    &name,
                    random_id()?,
                )?,
            },
            description: Some(RtBox {
                key: desc_key,
                boxed: self.keys(desc_key)?.seal_text(
                    RtKeyType::ChannelDescription,
                    &desc,
                    random_id()?,
                )?,
            }),
            roles: RtRolePair { read, write: read },
            last_message: None,
            ctime: 0,
            mtime: 0,
            updated_at: version,
            tier,
            unreadable: false,
        };
        self.prepare(
            store,
            id,
            submission,
            RtChannelId(id),
            0,
            RealtimeRequest::CreateChannel(RtCreateChannelArgument {
                metadata,
                set_version: version,
            }),
        )
    }
    pub fn prepare_send(
        &mut self,
        rpc: &mut impl ChatTransport,
        store: &mut impl ProtectedMutationStore,
        channel: RtChannelId,
        text: &str,
    ) -> Result<ChatOperation> {
        self.prepare_send_submission(rpc, store, channel, text, None)
    }
    pub fn prepare_send_submission(
        &mut self,
        rpc: &mut impl ChatTransport,
        store: &mut impl ProtectedMutationStore,
        channel: RtChannelId,
        text: &str,
        submission: Option<&ChatSubmission>,
    ) -> Result<ChatOperation> {
        if let Some(operation) = self.submitted_operation(submission)? {
            return Ok(operation);
        }

        if text.is_empty() || text.len() > RT_MAX_BODY_BYTES {
            return Err(Error::ChatInvalidInput("invalid message text length"));
        }
        self.refresh()?;
        let md = self.channel(rpc, channel, true)?;
        let RealtimeResponse::Messages(recent) =
            rpc.request(&RealtimeRequest::Recents(RtRecentsArgument {
                channel,
                limit: 1,
                stop_at: 0,
            }))?
        else {
            return Err(Error::ChatIntegrity("unexpected recent response"));
        };
        if recent.messages.len() > 1 {
            return Err(Error::ChatIntegrity("recent response exceeds request"));
        }
        let history = self.verify_page(rpc, &md, recent.messages)?;
        let previous = history.messages.first().map(|m| &m.message);
        if history
            .messages
            .iter()
            .any(|m| matches!(m.content, super::history::ChatContent::Unsupported(_)))
        {
            return Err(Error::ChatUnsupported(
                "latest message has unsupported content",
            ));
        }
        let lower = previous.map_or(0, |m| m.sequence);
        let id = random_id()?;
        let metadata = RtMessageMetadata {
            id: RtMessageId(id),
            previous_id: previous.map_or(RtMessageId([0; 16]), |m| m.metadata.id),
            previous_sequence: lower,
            send_time: crate::now_microseconds()? / 1000,
            kind: RtMessageType::Basic,
            further_user_attribution: None,
        };
        let key = self.current(md.roles.read)?;
        let ciphertext = self.keys(key)?.seal_basic_message(
            &self.noncer(channel, metadata.clone(), self.credential.uid.clone()),
            text.as_bytes(),
        )?;
        self.prepare(
            store,
            id,
            submission,
            channel,
            lower,
            RealtimeRequest::Send(RtSendArgument {
                send: RtSend {
                    metadata,
                    channel: channel.short(),
                    wrapper: RtMessageWrapper::Encrypted(RtMessageBox { key, ciphertext }),
                    expected_previous_sequence: 0,
                },
            }),
        )
    }
    fn prepare(
        &self,
        store: &mut impl ProtectedMutationStore,
        id: [u8; 16],
        submission: Option<&ChatSubmission>,
        channel: RtChannelId,
        lower: u64,
        request: RealtimeRequest,
    ) -> Result<ChatOperation> {
        let kind = match &request {
            RealtimeRequest::CreateChannel(_) => Kind::Create,
            RealtimeRequest::Send(_) => Kind::Send,
            _ => return Err(Error::ChatInvalidInput("invalid chat preparation")),
        };
        let bytes = Zeroizing::new(request.argument()?);
        let op = ChatOperation {
            id,
            scope: self.scope(channel),
            kind,
            state: State::Prepared,
            request_hash: foks_crypto::prefixed_hash(REQUEST_HASH_DOMAIN, &bytes),
            scan_cursor: lower,
            receipt: None,
            rejection_code: None,
        };
        store
            .put_if_absent(&material_key(&op), &bytes)
            .map_err(|e| Error::ProtectedMaterial(e.to_string()))?;
        self.hard()?.chat_record_submission(&op, submission)?;
        Ok(op)
    }
    fn operation(&self, id: &[u8; 16]) -> Result<ChatOperation> {
        let op = self
            .hard()?
            .chat_operation(id)?
            .ok_or(Error::ChatNotFound("pending operation missing"))?;
        if op.scope != self.scope(RtChannelId(op.scope.channel)) {
            return Err(Error::ChatIntegrity(
                "pending operation belongs to another account/team/host",
            ));
        }
        Ok(op)
    }
    fn material(
        &self,
        store: &mut impl ProtectedMutationStore,
        op: &ChatOperation,
    ) -> Result<RealtimeRequest> {
        let bytes = store
            .get(&material_key(op))
            .map_err(|e| Error::ProtectedMaterial(e.to_string()))?;
        if foks_crypto::prefixed_hash(REQUEST_HASH_DOMAIN, &bytes) != op.request_hash {
            return Err(Error::ChatIntegrity("pending request changed"));
        }
        let pos = match op.kind {
            Kind::Create => foks_rpc::RT_NEW_CHANNEL_METHOD_POSITION,
            Kind::Send => foks_rpc::RT_SEND_METHOD_POSITION,
        };
        Ok(RealtimeRequest::decode_argument(pos, &bytes)?)
    }
    pub fn list_pending(&self) -> Result<Vec<ChatOperation>> {
        Ok(self.hard()?.chat_pending(
            self.host.host_id().as_bytes(),
            self.credential.uid.as_bytes(),
            self.team_id.as_bytes(),
        )?)
    }
    /// Never automatically resubmits an uncertain operation, including against Go.
    pub fn attempt_operation(
        &mut self,
        rpc: &mut impl ChatTransport,
        store: &mut impl ProtectedMutationStore,
        id: &[u8; 16],
    ) -> Result<ChatOperation> {
        let op = self.operation(id)?;
        if op.state.is_terminal() {
            self.cleanup_terminal(store, &op)?;
            return Ok(op);
        }
        if op.state == State::Uncertain {
            return self.reconcile_operation(rpc, store, id);
        }
        self.refresh()?;
        let request = self.material(store, &op)?;
        match &request {
            RealtimeRequest::Send(arg) => {
                let md = self.channel(rpc, RtChannelId(op.scope.channel), true)?;
                let RtMessageWrapper::Encrypted(b) = &arg.send.wrapper else {
                    return Err(Error::ChatIntegrity("pending message is not encrypted"));
                };
                if self.current(md.roles.read)? != b.key {
                    return Err(Error::ChatReprepareRequired(
                        "pending message key is stale; preserve operation for review",
                    ));
                }
            }
            RealtimeRequest::CreateChannel(arg) => {
                self.validate_channel(&arg.metadata)?;
                if self.current(arg.metadata.name.key.role)? != arg.metadata.name.key
                    || arg
                        .metadata
                        .description
                        .as_ref()
                        .map(|d| self.current(d.key.role).map(|key| key != d.key))
                        .transpose()?
                        .unwrap_or(false)
                {
                    return Err(Error::ChatReprepareRequired(
                        "pending channel keys are stale",
                    ));
                }
                let set = self.list_current_channels(rpc)?;
                if set.version.checked_add(1) != Some(arg.set_version) {
                    return Err(Error::ChatReprepareRequired(
                        "channel set changed; prepare a new create after reviewing names",
                    ));
                }
                if self.role < arg.metadata.roles.write {
                    return Err(Error::ChatAccessDenied("channel create access denied"));
                }
            }
            _ => return Err(Error::ChatIntegrity("invalid pending operation")),
        }
        self.hard()?.chat_begin(id)?;
        let response = match rpc.request(&request) {
            Ok(response) => response,
            Err(error) => {
                if let Some(code) = definite_rejection(&error) {
                    self.hard()?.chat_reject(id, code)?;
                    self.finalize_operation(store, id)?;
                }
                return Err(error);
            }
        };
        match (&request, response) {
            (RealtimeRequest::Send(arg), RealtimeResponse::Sent(receipt)) => {
                self.confirm_send(&op, &arg.send, receipt)?
            }
            (RealtimeRequest::CreateChannel(_), RealtimeResponse::Void) => {
                self.hard()?.chat_confirm(id, &op.scope.channel)?
            }
            _ => return Err(Error::ChatIntegrity("unexpected mutation response")),
        }
        self.finalize_operation(store, id)
    }
    fn confirm_send(&self, op: &ChatOperation, send: &RtSend, receipt: RtSendResult) -> Result<()> {
        if receipt.sequence <= send.metadata.previous_sequence || receipt.sequence > i64::MAX as u64
        {
            return Err(Error::ChatIntegrity("invalid message receipt"));
        }
        let m = RtMessage {
            metadata: send.metadata.clone(),
            wrapper: send.wrapper.clone(),
            sequence: receipt.sequence,
            insert_time: receipt.insert_time,
            sender: Some(RtPartyId::new(self.credential.uid.clone())?),
        };
        self.hard()?.chat_accept(&op.scope, &[anchor(&m)?])?;
        self.hard()?.chat_confirm(&op.id, &receipt.encoded()?)?;
        Ok(())
    }
    /// Advances one forward 100-sequence window, retaining uncertainty on absence.
    /// A caller can repeat with the same operation ID after restart.
    pub fn reconcile_operation(
        &mut self,
        rpc: &mut impl ChatTransport,
        store: &mut impl ProtectedMutationStore,
        id: &[u8; 16],
    ) -> Result<ChatOperation> {
        let op = self.operation(id)?;
        if op.state.is_terminal() {
            return self.finalize_operation(store, id);
        }
        if op.state != State::Uncertain {
            return Ok(op);
        }
        self.refresh()?;
        match self.material(store, &op)? {
            RealtimeRequest::CreateChannel(arg) => {
                if let Some(found) = self
                    .list_current_channels(rpc)?
                    .channels
                    .into_iter()
                    .find(|c| c.metadata.id == arg.metadata.id)
                {
                    let mut actual = found.metadata;
                    let mut expected = arg.metadata;
                    actual.ctime = 0;
                    actual.mtime = 0;
                    actual.last_message = None;
                    expected.ctime = 0;
                    expected.mtime = 0;
                    if actual != expected {
                        return Err(Error::ChatIntegrity(
                            "created channel identity conflicts with intended metadata",
                        ));
                    }
                    self.hard()?.chat_confirm(id, &op.scope.channel)?;
                }
            }
            RealtimeRequest::Send(arg) => {
                let channel = RtChannelId(op.scope.channel);
                let md = self.channel(rpc, channel, false)?;
                let start = op
                    .scan_cursor
                    .checked_add(1)
                    .ok_or(Error::ChatLimit("scan cursor overflow"))?;
                let history = recovery_window(|width| {
                    let end = start.saturating_add(width - 1).min(i64::MAX as u64);
                    let rows = self.range(rpc, channel, start, end)?;
                    self.verify_page(rpc, &md, rows)
                })?;
                let mut cursor = op.scan_cursor;
                for row in history.messages {
                    let m = row.message;
                    cursor = cursor.max(m.sequence);
                    if m.metadata.id == arg.send.metadata.id {
                        if m.metadata != arg.send.metadata
                            || m.wrapper != arg.send.wrapper
                            || m.sender.as_ref().map(|s| s.entity()) != Some(&self.credential.uid)
                        {
                            return Err(Error::ChatIntegrity(
                                "recovered message conflicts with original envelope",
                            ));
                        }
                        self.confirm_send(
                            &op,
                            &arg.send,
                            RtSendResult {
                                sequence: m.sequence,
                                insert_time: m.insert_time,
                            },
                        )?;
                        return self.finalize_operation(store, id);
                    }
                }
                // Don't advance beyond the observed head on an empty/short page.
                self.hard()?.chat_progress(id, cursor)?;
            }
            _ => return Err(Error::ChatIntegrity("invalid pending operation")),
        }
        let op = self.operation(id)?;
        if op.state.is_terminal() {
            self.cleanup_terminal(store, &op)?;
        }
        Ok(op)
    }
    /// Cancel only work that has never crossed the delivery boundary.
    pub fn cancel_prepared_operation(
        &self,
        store: &mut impl ProtectedMutationStore,
        id: &[u8; 16],
    ) -> Result<ChatOperation> {
        self.operation(id)?;
        self.hard()?.chat_cancel(id)?;
        self.finalize_operation(store, id)
    }
    /// Retry terminal material cleanup, without a network connection or PTKs.
    pub fn finalize_operation(
        &self,
        store: &mut impl ProtectedMutationStore,
        id: &[u8; 16],
    ) -> Result<ChatOperation> {
        let op = self.operation(id)?;
        if !op.state.is_terminal() {
            return Err(Error::ChatOperationState("operation is not terminal"));
        }
        self.cleanup_terminal(store, &op)?;
        Ok(op)
    }
    fn cleanup_terminal(
        &self,
        store: &mut impl ProtectedMutationStore,
        op: &ChatOperation,
    ) -> Result<()> {
        match store.remove(&material_key(op)) {
            Ok(()) | Err(crate::ProtectedStoreError::Missing) => Ok(()),
            Err(error) => Err(Error::ProtectedMaterial(error.to_string())),
        }
    }
}
// A completed remote error is not always a pre-commit rejection. In particular,
// transaction-retry and generic realtime errors retain uncertainty.
fn definite_rejection(error: &Error) -> Option<i64> {
    match error {
        Error::Rpc(foks_rpc::Error::RemoteStatus { code, .. })
            if matches!(*code, 1013 | 1030 | 12002 | 12003 | 12005 | 12006) =>
        {
            Some(*code as i64)
        }
        _ => None,
    }
}
// The protocol's generic RT limit status does not distinguish row/byte caps.
// Shrink only read windows; never retry a mutation on this status.
fn recovery_window<T>(mut read: impl FnMut(u64) -> Result<T>) -> Result<T> {
    let mut width = ChatLimits::RECOVERY_WINDOW;
    loop {
        match read(width) {
            Err(
                Error::Rpc(foks_rpc::Error::RemoteStatus { code: 12001, .. }) | Error::ChatLimit(_),
            ) if width > 1 => {
                width = (width / 2).max(1);
            }
            result => return result,
        }
    }
}

fn material_key(op: &ChatOperation) -> Vec<u8> {
    let mut key = PROTECTED_MATERIAL_DOMAIN.to_vec();
    key.extend_from_slice(&op.scope.host);
    key.extend_from_slice(&op.scope.uid);
    key.extend_from_slice(&op.scope.team);
    key.extend_from_slice(&op.scope.channel);
    key.extend_from_slice(&op.id);
    key
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_adapts_to_local_limits_but_does_not_retry_integrity_errors() {
        let mut widths = Vec::new();
        let result = recovery_window(|width| {
            widths.push(width);
            if width > 1 {
                Err(Error::ChatLimit("page bytes"))
            } else {
                Ok(42)
            }
        })
        .unwrap();
        assert_eq!(result, 42);
        assert_eq!(widths, vec![100, 50, 25, 12, 6, 3, 1]);
        let mut attempts = 0;
        assert!(matches!(
            recovery_window::<()>(|_| {
                attempts += 1;
                Err(Error::ChatIntegrity("retained anchor contradiction"))
            }),
            Err(Error::ChatIntegrity(_))
        ));
        assert_eq!(attempts, 1);
    }
    #[test]
    fn go_channel_normalization_rules() {
        for (input, expected) in [
            ("", ""),
            ("  HeLLo  ", "hello"),
            ("日本語", "日本語"),
            ("-abc-", "-abc-"),
            ("İST", "ist"),
            ("💬💬💬", "💬💬💬"),
        ] {
            assert_eq!(normalize_chat_name(input).unwrap(), expected);
        }
        for input in [
            "general",
            "ab",
            "a--b",
            "a b",
            "a_b",
            "abc!",
            "abc\u{200b}",
            "abc\u{7f}",
        ] {
            assert!(
                matches!(normalize_chat_name(input), Err(Error::ChatInvalidInput(_))),
                "{input}"
            );
        }
    }
}
