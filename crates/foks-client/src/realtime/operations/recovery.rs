//! Bounded reconciliation against the Basic Go protocol.
use super::*;

impl ChatSession<'_> {
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
}

pub(super) fn recovery_window<T>(mut read: impl FnMut(u64) -> Result<T>) -> Result<T> {
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
