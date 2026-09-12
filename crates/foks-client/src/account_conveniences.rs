//! Durable account changes. An uncertain request is read back, never reposted.
use crate::*;
use foks_proto::{ChangeUsernameArgument, ChangedUsernameFullUpdate};

pub struct UsernameChangeProgress {
    pub operation: MutationOperation,
    pub target: Option<String>,
    pub current: Option<foks_verify::VerifiedUserState>,
}

impl FoksClient {
    pub fn prepare_username_change(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        username: &str,
        protected: &mut impl ProtectedMutationStore,
    ) -> Result<UsernameChangeProgress> {
        if matches!(credential, FederationCredential::Software(c) if c.key_kind == SoftwareKeyKind::BotToken)
        {
            return Err(Error::AccountRequest(
                "username changes require a permanent account credential",
            ));
        }
        if username.len() > 256 {
            return Err(Error::AccountRequest("username exceeds 256 bytes"));
        }
        let normalized = foks_verify::normalize_username(username.as_bytes())
            .ok_or(Error::AccountRequest("invalid username"))?;
        let user = self
            .authenticate_credential_and_pin(host, credential)?
            .verified;
        let mut store = HardStateStore::open(&host.database_path)?;
        for receipt in store.expired_username_change_receipts(
            host.host_id().as_bytes(),
            credential.uid().as_bytes(),
            now_microseconds()?.saturating_sub(30 * 24 * 60 * 60 * 1_000_000),
        )? {
            crate::mutation::remove_terminal_material(protected, &receipt.material_ref)?;
            store.delete_username_change_receipt(&receipt.operation_id)?;
        }
        if user.username_utf8() == username.as_bytes() {
            return Err(Error::AccountRequest("username is unchanged"));
        }
        let (seed, certs) = credential.transport();
        let full = if normalized != user.username() {
            let bytes = self.call_with_material(
                host,
                &host.user,
                &foks_rpc::encode_reserve_username_for_change_request_at(&normalized, 0)?,
                seed,
                certs,
            )?;
            let reservation = foks_proto::UsernameReservation::decode(&bytes)?;
            let next_tree_location = random_bytes()?;
            let commitment_key = random_bytes()?;
            let root = user.tree_root();
            let input = foks_crypto::UsernameChangeInput {
                base: UserMutationBase {
                    uid: user.uid(),
                    host: host.host_id(),
                    seqno: user.chain_seqno() + 1,
                    previous: user.chain_tail_hash(),
                    root: &root,
                    time: now_microseconds()? / 1000,
                    next_tree_location,
                },
                normalized_name: &normalized,
                name_sequence: reservation.sequence,
                commitment_key,
            };
            let link = match credential {
                FederationCredential::Software(c) => {
                    foks_crypto::make_software_username_change(&input, &c.seed)?
                }
                FederationCredential::Yubi(c) => {
                    foks_crypto::make_yubi_username_change(&input, c.parent)?
                }
            };
            Some(ChangedUsernameFullUpdate {
                link,
                commitment_key,
                reservation,
                next_tree_location,
            })
        } else {
            None
        };
        let expected = user.chain_seqno() + u64::from(full.is_some());
        let arg = ChangeUsernameArgument {
            username: username.into(),
            full,
        };
        let frame = foks_rpc::encode_change_username_request_at(&arg, 0)?;
        let id = self.prepare_user_mutation(
            host,
            MutationKind::UsernameChange,
            credential.uid(),
            &credential.device_id()?,
            expected,
            &frame,
            protected,
        )?;
        let operation = HardStateStore::open(&host.database_path)?
            .mutation(&id)?
            .ok_or(Error::OperationBinding("rename was not recorded"))?;
        Ok(UsernameChangeProgress {
            operation,
            target: Some(username.into()),
            current: Some(user),
        })
    }

    /// Local status is available without unlocking hardware or submitting a request.
    pub fn username_change_operation(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        device: &EntityId,
        id: [u8; 16],
    ) -> Result<MutationOperation> {
        let op = HardStateStore::open(&host.database_path)?
            .mutation(&id)?
            .ok_or(Error::OperationBinding("rename operation is missing"))?;
        if op.kind != MutationKind::UsernameChange
            || op.host_id != host.host_id().as_bytes()
            || op.scope_id != uid.as_bytes()
            || op.subject_id != device.as_bytes()
        {
            return Err(Error::OperationBinding(
                "rename belongs to another host, account or credential",
            ));
        }
        Ok(op)
    }

    /// `attempt` permits only Prepared -> Submitting. All later calls reconcile.
    pub fn username_change_progress(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        id: [u8; 16],
        attempt: bool,
        protected: &mut impl ProtectedMutationStore,
    ) -> Result<UsernameChangeProgress> {
        let mut op =
            self.username_change_operation(host, credential.uid(), &credential.device_id()?, id)?;
        if op.state.is_terminal() {
            crate::mutation::remove_terminal_material(protected, &op.material_ref)?;
            return Ok(UsernameChangeProgress {
                operation: op,
                target: None,
                current: None,
            });
        }
        self.bound_user_mutation(
            host,
            id,
            MutationKind::UsernameChange,
            credential.uid(),
            protected,
        )?;
        let frame =
            MutationCoordinator::new(&host.database_path, protected).load_bound_material(&op)?;
        let call = foks_rpc::read_call(
            &mut std::io::Cursor::new(&*frame),
            foks_rpc::DEFAULT_MAX_FRAME_LENGTH,
        )?;
        if call.protocol_id() != foks_rpc::USER_PROTOCOL_ID || call.method_position() != 11 {
            return Err(Error::OperationBinding(
                "rename material contains another RPC",
            ));
        }
        let arg = ChangeUsernameArgument::decode(call.argument())?;
        if attempt && op.state == MutationState::Prepared {
            // Authenticate before crossing the submission boundary; refresh failures preserve Prepared.
            self.authenticate_credential_and_pin(host, credential)?;
            MutationCoordinator::new(&host.database_path, protected).begin_submission(&id)?;
            let (seed, certs) = credential.transport();
            match self.call_void_with_material(host, &host.user, &frame, seed, certs) {
                Ok(()) if arg.full.is_none() => {
                    MutationCoordinator::new(&host.database_path, protected).remote_verified(&id)?
                }
                Ok(()) => {}
                Err(e) => {
                    MutationCoordinator::new(&host.database_path, protected)
                        .submission_unknown(&id)?;
                    // A server rejection means this one attempt definitely
                    // failed. A transport error does not say whether the
                    // rename succeeded.
                    if matches!(
                        &e,
                        Error::Rpc(foks_rpc::Error::RemoteStatus {
                            code: 1023 | 1029 | 1030 | 1013 | 1069,
                            ..
                        })
                    ) {
                        MutationCoordinator::new(&host.database_path, protected).rejected(&id)?;
                    }
                    return Err(e);
                }
            }
            op = self.username_change_operation(
                host,
                credential.uid(),
                &credential.device_id()?,
                id,
            )?;
        }
        let current = match self.authenticate_credential_and_pin(host, credential) {
            Ok(outcome) => outcome.verified,
            Err(Error::Verify(foks_verify::Error::UserMerkleProof))
                if matches!(
                    op.state,
                    MutationState::Submitting | MutationState::SubmissionUnknown
                ) =>
            {
                // Go publishes chain and name leaves asynchronously. A missing or
                // invalid proof proves no outcome; retain the original request.
                if op.state == MutationState::Submitting {
                    MutationCoordinator::new(&host.database_path, protected)
                        .submission_unknown(&id)?;
                    op = self.username_change_operation(
                        host,
                        credential.uid(),
                        &credential.device_id()?,
                        id,
                    )?;
                }
                return Ok(UsernameChangeProgress {
                    operation: op,
                    target: Some(arg.username),
                    current: None,
                });
            }
            Err(error) => return Err(error),
        };
        if matches!(
            op.state,
            MutationState::Submitting | MutationState::SubmissionUnknown
        ) {
            let (accepted, rejected) = if let Some(full) = &arg.full {
                let seq = full.link.decode_group_change()?.seqno;
                let link = current.authenticated_link(seq)?;
                let accepted = link.as_ref().map(|l| l.encoded()).transpose()?.as_deref()
                    == Some(full.link.encoded()?.as_slice());
                (accepted, link.is_some() && !accepted)
            } else {
                (current.username_utf8() == arg.username.as_bytes(), false)
            };
            let mut coordinator = MutationCoordinator::new(&host.database_path, protected);
            if accepted {
                coordinator.remote_verified(&id)?;
            } else if rejected {
                coordinator.rejected(&id)?;
            } else if op.state == MutationState::Submitting {
                coordinator.submission_unknown(&id)?;
            }
            op = self.username_change_operation(
                host,
                credential.uid(),
                &credential.device_id()?,
                id,
            )?;
        }
        Ok(UsernameChangeProgress {
            operation: op,
            target: Some(arg.username),
            current: Some(current),
        })
    }

    pub fn cancel_username_change(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        device: &EntityId,
        id: [u8; 16],
        protected: &mut impl ProtectedMutationStore,
    ) -> Result<()> {
        let op = self.username_change_operation(host, uid, device, id)?;
        if op.state == MutationState::Rejected {
            return crate::mutation::remove_terminal_material(protected, &op.material_ref);
        }
        if op.state != MutationState::Prepared {
            return Err(Error::OperationBinding(
                "submitted rename must be reconciled",
            ));
        }
        MutationCoordinator::new(&host.database_path, protected).rejected(&id)
    }
}
