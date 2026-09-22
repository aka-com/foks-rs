//! Scope-bound protected request and durable operation lifetimes.
use super::*;

impl ChatSession<'_> {
    /// Recover only retained send request, after fresh channel read authorization.
    /// This never attempts delivery, changes a receipt or retains terminal request.
    pub fn recover_operation_text(
        &mut self,
        rpc: &mut impl ChatTransport,
        store: &mut impl ProtectedMutationStore,
        id: &[u8; 16],
        channel: RtChannelId,
    ) -> Result<Option<Zeroizing<String>>> {
        let op = self.operation(id)?;
        if op.kind != Kind::Send || op.scope.channel != channel.0 {
            return Err(Error::ChatInvalidInput(
                "operation is not a send in this channel",
            ));
        }
        self.refresh()?;
        let md = self.channel(rpc, channel, false)?;
        let bytes = match store.get(&protected_request_key(&op)) {
            Ok(bytes) => bytes,
            Err(crate::ProtectedStoreError::Missing) => return Ok(None),
            Err(error) => return Err(Error::ProtectedStore(error.to_string())),
        };
        let RealtimeRequest::Send(request) = decode_protected_request(&op, &bytes)? else {
            return Err(Error::ChatIntegrity("retained operation is not a send"));
        };
        if request.send.metadata.id.0 != op.id || request.send.channel != channel.short() {
            return Err(Error::ChatIntegrity("retained send identity changed"));
        }
        let RtMessageWrapper::Encrypted(boxed) = &request.send.wrapper else {
            return Err(Error::ChatChannelIntegrity(
                "retained send is not encrypted",
            ));
        };
        if boxed.key.role != md.roles.read {
            return Err(Error::ChatChannelIntegrity(
                "retained send read role changed",
            ));
        }
        let plaintext = self
            .keys(boxed.key)?
            .open_basic_message(
                &self.noncer(channel, request.send.metadata, self.credential.uid.clone()),
                &boxed.ciphertext,
            )
            .map_err(|_| Error::ChatChannelIntegrity("retained send authentication failed"))?;
        let text = std::str::from_utf8(&plaintext)
            .map_err(|_| Error::ChatChannelIntegrity("retained send text is not UTF-8"))?;
        Ok(Some(Zeroizing::new(text.to_owned())))
    }
    /// Resolve a durable submission without connecting or loading protected request.
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

    pub(super) fn prepare(
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
        self.hard()?
            .chat_record_submission_with_protected_request(&op, submission, || {
                store
                    .put_if_absent(&protected_request_key(&op), &bytes)
                    .map_err(|e| Error::ProtectedStore(e.to_string()))
            })?;
        Ok(op)
    }

    pub(super) fn operation(&self, id: &[u8; 16]) -> Result<ChatOperation> {
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

    pub(super) fn protected_request(
        &self,
        store: &mut impl ProtectedMutationStore,
        op: &ChatOperation,
    ) -> Result<RealtimeRequest> {
        let bytes = store
            .get(&protected_request_key(op))
            .map_err(|e| Error::ProtectedStore(e.to_string()))?;
        decode_protected_request(op, &bytes)
    }

    pub fn list_cleanup_pending(&self) -> Result<Vec<ChatOperation>> {
        Ok(self.hard()?.chat_cleanup_pending(
            self.host.host_id().as_bytes(),
            self.credential.uid.as_bytes(),
            self.team_id.as_bytes(),
        )?)
    }

    pub fn list_pending(&self) -> Result<Vec<ChatOperation>> {
        Ok(self.hard()?.chat_pending(
            self.host.host_id().as_bytes(),
            self.credential.uid.as_bytes(),
            self.team_id.as_bytes(),
        )?)
    }

    /// Cancel only work that has never crossed the delivery boundary.
    pub fn cancel_prepared_operation(
        &self,
        store: &mut impl ProtectedMutationStore,
        id: &[u8; 16],
    ) -> Result<ChatOperation> {
        self.operation(id)?;
        self.hard()?.chat_cancel(id)?;
        self.terminal_outcome(store, id)
    }

    /// Retry terminal request cleanup, without a network connection or PTKs.
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

    pub(super) fn terminal_outcome(
        &self,
        store: &mut impl ProtectedMutationStore,
        id: &[u8; 16],
    ) -> Result<ChatOperation> {
        let op = self.operation(id)?;
        if !op.state.is_terminal() {
            return Err(Error::ChatOperationState("operation is not terminal"));
        }
        let _ = self.cleanup_terminal(store, &op);
        Ok(op)
    }

    pub(super) fn cleanup_terminal(
        &self,
        store: &mut impl ProtectedMutationStore,
        op: &ChatOperation,
    ) -> Result<()> {
        remove_terminal_request(store, op)?;
        self.hard()?.chat_complete_cleanup(&op.id)?;
        Ok(())
    }
}

fn remove_terminal_request(
    store: &mut impl ProtectedMutationStore,
    op: &ChatOperation,
) -> Result<()> {
    if !op.state.is_terminal() {
        return Err(Error::ChatOperationState("operation is not terminal"));
    }
    match store.remove(&protected_request_key(op)) {
        Ok(()) | Err(crate::ProtectedStoreError::Missing) => Ok(()),
        Err(error) => Err(Error::ProtectedStore(error.to_string())),
    }
}

pub(crate) fn decode_protected_request(
    op: &ChatOperation,
    bytes: &[u8],
) -> Result<RealtimeRequest> {
    if foks_crypto::prefixed_hash(REQUEST_HASH_DOMAIN, bytes) != op.request_hash {
        return Err(Error::ChatIntegrity("pending request changed"));
    }
    let pos = match op.kind {
        Kind::Create => foks_rpc::RT_NEW_CHANNEL_METHOD_POSITION,
        Kind::Send => foks_rpc::RT_SEND_METHOD_POSITION,
    };
    Ok(RealtimeRequest::decode_argument(pos, bytes)?)
}

fn protected_request_key(op: &ChatOperation) -> Vec<u8> {
    crate::ProtectedRecordKey::Chat(op).encoded()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EncryptedFileMutationStore, ProtectedStoreError};
    use foks_client_db::{ChatScope, HardStateStore};

    fn operation() -> ChatOperation {
        ChatOperation {
            id: [7; 16],
            scope: ChatScope {
                host: vec![1; 33],
                uid: vec![2; 33],
                team: vec![3; 33],
                channel: [4; 16],
            },
            kind: Kind::Send,
            state: State::Prepared,
            request_hash: [8; 32],
            scan_cursor: 0,
            receipt: None,
            rejection_code: None,
        }
    }
    fn store(path: &std::path::Path) -> EncryptedFileMutationStore {
        EncryptedFileMutationStore::open(path, Zeroizing::new([17; 32])).unwrap()
    }

    #[test]
    fn interrupted_preparation_leaves_only_protected_request_then_exact_retry_commits() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("hard");
        let protected_request_path = dir.path().join("protected-request");
        let op = operation();
        let submission = ChatSubmission {
            id: [9; 16],
            input_mac: [10; 32],
        };
        let key = protected_request_key(&op);
        let mut db = HardStateStore::open(&db_path).unwrap();
        let mut protected = store(&protected_request_path);
        // Unwind after the durable file write, before SQLite commit. Dropping the
        // transaction simulates the rollback observed when reopening after a crash.
        let interruption = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = db.chat_record_submission_with_protected_request::<Error>(
                &op,
                Some(&submission),
                || {
                    protected
                        .put_if_absent(&key, b"retained encrypted request")
                        .unwrap();
                    panic!("interrupted before ledger commit");
                },
            );
        }));
        assert!(interruption.is_err());
        drop(db);
        drop(protected);
        let mut db = HardStateStore::open(&db_path).unwrap();
        let mut protected = store(&protected_request_path);
        assert!(db.chat_operation(&op.id).unwrap().is_none());
        assert!(db
            .chat_submission(&op.scope, &submission)
            .unwrap()
            .is_none());
        assert_eq!(
            protected.get(&key).unwrap().as_slice(),
            b"retained encrypted request"
        );
        db.chat_record_submission_with_protected_request(&op, Some(&submission), || {
            protected
                .put_if_absent(&key, b"retained encrypted request")
                .map_err(|e| Error::ProtectedStore(e.to_string()))
        })
        .unwrap();
        drop(db);
        assert_eq!(
            HardStateStore::open(&db_path)
                .unwrap()
                .chat_submission(&op.scope, &submission)
                .unwrap(),
            Some(op)
        );
    }

    struct FailingRemoval(EncryptedFileMutationStore);
    impl ProtectedMutationStore for FailingRemoval {
        fn put_if_absent(
            &mut self,
            key: &[u8],
            value: &[u8],
        ) -> std::result::Result<(), ProtectedStoreError> {
            self.0.put_if_absent(key, value)
        }
        fn get(
            &mut self,
            key: &[u8],
        ) -> std::result::Result<Zeroizing<Vec<u8>>, ProtectedStoreError> {
            self.0.get(key)
        }
        fn remove(&mut self, _: &[u8]) -> std::result::Result<(), ProtectedStoreError> {
            Err(ProtectedStoreError::Backend(
                "injected removal failure".into(),
            ))
        }
    }

    #[test]
    fn failed_terminal_cleanup_preserves_identity_and_retries_after_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("hard");
        let protected_request_path = dir.path().join("protected-request");
        let mut db = HardStateStore::open(&db_path).unwrap();
        let op = operation();
        let mut protected = FailingRemoval(store(&protected_request_path));
        protected
            .put_if_absent(&protected_request_key(&op), b"request")
            .unwrap();
        db.chat_record(&op).unwrap();
        assert!(matches!(
            remove_terminal_request(&mut protected, &op),
            Err(Error::ChatOperationState(_))
        ));
        db.chat_cancel(&op.id).unwrap();
        let cancelled = db.chat_operation(&op.id).unwrap().unwrap();
        assert!(matches!(
            remove_terminal_request(&mut protected, &cancelled),
            Err(Error::ProtectedStore(_))
        ));
        drop(db);
        drop(protected);
        let db = HardStateStore::open(&db_path).unwrap();
        let cancelled = db.chat_operation(&op.id).unwrap().unwrap();
        assert_eq!(
            db.chat_cleanup_pending(&op.scope.host, &op.scope.uid, &op.scope.team)
                .unwrap(),
            vec![cancelled.clone()]
        );
        assert_eq!(cancelled.state, State::Cancelled);
        let mut protected = store(&protected_request_path);
        assert!(protected.get(&protected_request_key(&op)).is_ok());
        remove_terminal_request(&mut protected, &cancelled).unwrap();
        drop(db);
        drop(protected);
        let mut db = HardStateStore::open(&db_path).unwrap();
        let mut protected = store(&protected_request_path);
        assert_eq!(
            db.chat_cleanup_pending(&op.scope.host, &op.scope.uid, &op.scope.team)
                .unwrap(),
            vec![cancelled.clone()]
        );
        remove_terminal_request(&mut protected, &cancelled).unwrap();
        db.chat_complete_cleanup(&op.id).unwrap();
        assert!(db
            .chat_cleanup_pending(&op.scope.host, &op.scope.uid, &op.scope.team)
            .unwrap()
            .is_empty());
        assert!(matches!(
            protected.get(&protected_request_key(&op)),
            Err(ProtectedStoreError::Missing)
        ));
        assert_eq!(db.chat_operation(&op.id).unwrap(), Some(cancelled));
    }

    #[test]
    fn retained_request_keys_bind_every_scope_component_and_operation() {
        let dir = tempfile::tempdir().unwrap();
        let mut protected = store(dir.path());
        let op = operation();
        protected
            .put_if_absent(&protected_request_key(&op), b"request")
            .unwrap();
        for component in 0..5 {
            let mut changed = op.clone();
            match component {
                0 => changed.scope.host[0] ^= 1,
                1 => changed.scope.uid[0] ^= 1,
                2 => changed.scope.team[0] ^= 1,
                3 => changed.scope.channel[0] ^= 1,
                _ => changed.id[0] ^= 1,
            }
            assert!(matches!(
                protected.get(&protected_request_key(&changed)),
                Err(ProtectedStoreError::Missing)
            ));
        }
        assert_eq!(
            protected
                .get(&protected_request_key(&op))
                .unwrap()
                .as_slice(),
            b"request"
        );
    }
}
