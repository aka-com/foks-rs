//! Attempt prepared work once and verify exact server confirmations.
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
            (RealtimeRequest::Send(arg), RealtimeResponse::Sent(send_receipt)) => {
                self.confirm_send(op, &arg.send, send_receipt)?
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
        send_receipt: ChatSendReceipt,
    ) -> Result<()> {
        let retained = self.hard()?.chat_anchor(&op.scope, send_receipt.sequence)?;
        check_send_receipt(
            &send_receipt,
            send,
            retained.as_ref().map(|anchor| &anchor.id),
        )?;
        let m = RtMessage {
            metadata: send.metadata.clone(),
            wrapper: send.wrapper.clone(),
            sequence: send_receipt.sequence,
            insert_time: send_receipt.insert_time,
            sender: Some(RtPartyId::new(self.credential.uid.clone())?),
        };
        self.hard()?.chat_accept(&op.scope, &[anchor(&m)?])?;
        self.hard()?
            .chat_confirm(&op.id, &send_receipt.encoded()?)?;
        Ok(())
    }
}

/// Check a receipt's sequence against the message it confirms and the anchor
/// already retained at that sequence, if any.
///
/// A sequence anchored for another message is server misbehaviour. A fresh
/// delivery lands after the predecessor recorded in the message; at or below
/// that predecessor, accept only a replay that retained evidence proves (the
/// rule of upstream's `finalizeAck`). The receipt's `was_replay` flag alone is
/// an unauthenticated server claim and cannot make that ordering consistent.
fn check_send_receipt(
    receipt: &ChatSendReceipt,
    send: &RtSend,
    retained: Option<&[u8; 16]>,
) -> Result<()> {
    if receipt.sequence == 0 || receipt.sequence > i64::MAX as u64 {
        return Err(Error::ChatIntegrity("invalid chat send receipt"));
    }
    let anchored_here = match retained {
        Some(id) if *id != send.metadata.id.0 => {
            return Err(Error::ChatIntegrity(
                "send receipt names a sequence held by another message",
            ));
        }
        Some(_) => true,
        None => false,
    };
    if receipt.sequence <= send.metadata.previous_sequence && !anchored_here {
        return Err(Error::ChatIntegrity("invalid chat send receipt"));
    }
    Ok(())
}

/// `RT_MSG_REPLAY_MISMATCH` (go-foks b41b354): the message ID is already held
/// by a different message, so this send was not and cannot be stored.
const RT_MSG_REPLAY_MISMATCH: u64 = foks_rpc::STATUS_RT_MSG_REPLAY_MISMATCH;

// A completed remote error is not always a pre-commit rejection. In particular,
// transaction-retry and generic realtime errors retain uncertainty.
fn definite_rejection(error: &Error) -> Option<i64> {
    match error {
        Error::Rpc(foks_rpc::Error::RemoteStatus { code, .. })
            if matches!(
                *code,
                1013 | 1030 | 12002 | 12003 | 12005 | 12006 | RT_MSG_REPLAY_MISMATCH
            ) =>
        {
            Some(*code as i64)
        }
        _ => None,
    }
}
// The protocol's generic RT limit status does not distinguish row/byte caps.
// Shrink only read windows; never retry a mutation on this status.

#[cfg(test)]
mod tests {
    use super::*;
    use foks_proto::{RoleAndGeneration, RtChannelIdShort, RtCiphertext, TrailingFields};

    fn send(previous_sequence: u64) -> RtSend {
        RtSend {
            metadata: RtMessageMetadata {
                id: RtMessageId([7; 16]),
                previous_id: RtMessageId(if previous_sequence == 0 {
                    [0; 16]
                } else {
                    [6; 16]
                }),
                previous_sequence,
                send_time: 1,
                kind: RtMessageType::Basic,
                further_user_attribution: None,
            },
            channel: RtChannelIdShort::new(1).unwrap(),
            wrapper: RtMessageWrapper::Encrypted(RtMessageBox {
                ciphertext: RtCiphertext(vec![0; 16]),
                key: RoleAndGeneration {
                    role: Role::member(0),
                    generation: 1,
                },
            }),
            expected_previous_sequence: 0,
        }
    }
    fn receipt(sequence: u64, was_replay: bool) -> ChatSendReceipt {
        ChatSendReceipt {
            sequence,
            insert_time: 1,
            was_replay,
            trailing: TrailingFields::default(),
        }
    }

    #[test]
    fn fresh_and_replayed_receipts_after_the_predecessor_are_accepted() {
        let send = send(4);
        for was_replay in [false, true] {
            assert!(check_send_receipt(&receipt(5, was_replay), &send, None).is_ok());
            assert!(check_send_receipt(&receipt(9, was_replay), &send, None).is_ok());
            // Already anchored for this message, e.g. seen in history first.
            assert!(check_send_receipt(&receipt(5, was_replay), &send, Some(&[7; 16])).is_ok());
        }
    }

    #[test]
    fn receipts_at_or_below_the_predecessor_need_retained_evidence() {
        let send = send(4);
        for sequence in [1, 4] {
            for was_replay in [false, true] {
                assert!(matches!(
                    check_send_receipt(&receipt(sequence, was_replay), &send, None),
                    Err(Error::ChatIntegrity(_))
                ));
                // The cached message at that sequence carries this message ID.
                assert!(
                    check_send_receipt(&receipt(sequence, was_replay), &send, Some(&[7; 16]))
                        .is_ok()
                );
            }
        }
    }

    #[test]
    fn receipts_naming_another_message_or_invalid_sequences_are_rejected() {
        let send = send(4);
        for (sequence, was_replay) in [(4, true), (5, false), (5, true)] {
            assert!(matches!(
                check_send_receipt(&receipt(sequence, was_replay), &send, Some(&[6; 16])),
                Err(Error::ChatIntegrity(_))
            ));
        }
        for sequence in [0, i64::MAX as u64 + 1] {
            assert!(check_send_receipt(&receipt(sequence, true), &send, None).is_err());
        }
        assert!(
            definite_rejection(&Error::Rpc(foks_rpc::Error::RemoteStatus {
                code: 12007,
                detail: Default::default(),
            }))
            .is_some()
        );
    }
}
