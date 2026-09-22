//! Attempt prepared work once and verify exact receipts.
use super::*;

impl ChatSession<'_> {
    /// Never automatically resubmits an uncertain operation, including against Go.
    pub fn attempt_operation(
        &mut self,
        rpc: &mut impl ChatTransport,
        store: &mut impl ProtectedMutationStore,
        id: &[u8; 16],
    ) -> Result<ChatOperation> {
        let op = self.operation(id)?;
        if op.state.is_terminal() {
            return self.terminal_outcome(store, id);
        }
        if op.state == State::Uncertain {
            return self.reconcile_operation(rpc, store, id);
        }
        self.refresh()?;
        let request = self.protected_request(store, &op)?;
        match &request {
            RealtimeRequest::Send(arg) => {
                let md = self.channel(rpc, RtChannelId(op.scope.channel), true)?;
                self.validate_prepared_send(&md, &arg.send)?;
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
        self.deliver_prepared(rpc, store, &op, request)
    }

    pub(super) fn validate_prepared_send(
        &self,
        md: &RtChannelMetadata,
        send: &RtSend,
    ) -> Result<()> {
        let RtMessageWrapper::Encrypted(b) = &send.wrapper else {
            return Err(Error::ChatIntegrity("pending message is not encrypted"));
        };
        if self.current(md.roles.read)? != b.key {
            return Err(Error::ChatReprepareRequired(
                "pending message key is stale; preserve operation for review",
            ));
        }
        Ok(())
    }

    pub(super) fn deliver_prepared(
        &self,
        rpc: &mut impl ChatTransport,
        store: &mut impl ProtectedMutationStore,
        op: &ChatOperation,
        request: RealtimeRequest,
    ) -> Result<ChatOperation> {
        let id = &op.id;
        self.hard()?.chat_begin(id)?;
        let response = match rpc.request(&request) {
            Ok(response) => response,
            Err(error) => {
                if let Some(code) = definite_rejection(&error) {
                    self.hard()?.chat_reject(id, code)?;
                    self.terminal_outcome(store, id)?;
                }
                return Err(error);
            }
        };
        match (&request, response) {
            (RealtimeRequest::Send(arg), RealtimeResponse::Sent(receipt)) => {
                self.confirm_send(op, &arg.send, receipt)?
            }
            (RealtimeRequest::CreateChannel(_), RealtimeResponse::Void) => {
                self.hard()?.chat_confirm(id, &op.scope.channel)?
            }
            _ => return Err(Error::ChatIntegrity("unexpected mutation response")),
        }
        self.terminal_outcome(store, id)
    }

    pub(super) fn confirm_send(
        &self,
        op: &ChatOperation,
        send: &RtSend,
        receipt: RtSendResult,
    ) -> Result<()> {
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
