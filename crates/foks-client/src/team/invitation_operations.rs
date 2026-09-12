//! Invitation sends cross the same no-replay boundary as identity mutations.
use crate::*;
use foks_proto::{LocalInviteAcceptance, TeamCertificate, TeamInvite, TeamRsvp};
use zeroize::Zeroizing;

pub enum InvitationIntent {
    RemoteAcceptance(Box<super::PreparedRemoteInvitation>),
    Certificate {
        team: EntityId,
        prepared: super::PreparedTeamInvitation,
    },
    LocalAcceptance(LocalInviteAcceptance),
    Rejection {
        team: EntityId,
        receipt: TeamRsvp,
    },
}
pub struct InvitationProgress {
    pub operation: MutationOperation,
    /// Delivery acknowledgement only; never proof of membership.
    pub receipt: Option<TeamRsvp>,
    pub invite: Option<String>,
}
impl InvitationIntent {
    pub(super) fn encode(&self) -> Result<Vec<u8>> {
        use foks_snowpack::Value;
        let (kind, fields) = match self {
            Self::RemoteAcceptance(p) => (
                3,
                vec![
                    Value::Binary(p.team.as_bytes().to_vec()),
                    Value::Binary(p.invite.encoded()?),
                    Value::Binary(p.request.encoded()?),
                    Value::Binary(p.home_link.encoded()?),
                ],
            ),
            Self::Certificate { team, prepared } => (
                0,
                vec![
                    Value::Binary(team.as_bytes().to_vec()),
                    Value::Binary(prepared.invite.encoded()?),
                    Value::Binary(prepared.certificate.encoded()?),
                ],
            ),
            Self::LocalAcceptance(a) => (1, vec![Value::Binary(a.encoded()?)]),
            Self::Rejection { team, receipt } => (
                2,
                vec![
                    Value::Binary(team.as_bytes().to_vec()),
                    Value::Binary(receipt.encoded()?),
                ],
            ),
        };
        Ok(foks_snowpack::encode(&Zeroizing::new(Value::Array(vec![
            Value::Unsigned(kind),
            Value::Array(fields),
        ])))?)
    }
    pub(super) fn decode(b: &[u8]) -> Result<Self> {
        use foks_snowpack::Value;
        let sensitive = foks_snowpack::decode_sensitive(b)?;
        let Value::Array(v) = &*sensitive else {
            return Err(Error::OperationBinding("invitation material"));
        };
        let [Value::Unsigned(kind), Value::Array(f)] = v.as_slice() else {
            return Err(Error::OperationBinding("invitation variant"));
        };
        let bin = |i: usize| -> Result<&[u8]> {
            match f.get(i) {
                Some(Value::Binary(b)) => Ok(b),
                _ => Err(Error::OperationBinding("invitation field")),
            }
        };
        match (*kind, f.len()) {
            (3, 4) => Ok(Self::RemoteAcceptance(Box::new(
                super::PreparedRemoteInvitation {
                    team: EntityId::from_bytes(bin(0)?.to_vec())?,
                    invite: TeamInvite::decode(bin(1)?)?,
                    request: foks_proto::RemoteJoinRequest::decode(bin(2)?)?,
                    home_link: foks_proto::PostGenericLinkArgument::decode(bin(3)?)?,
                },
            ))),
            (0, 3) => Ok(Self::Certificate {
                team: EntityId::from_bytes(bin(0)?.to_vec())?,
                prepared: super::PreparedTeamInvitation {
                    invite: TeamInvite::decode(bin(1)?)?,
                    certificate: TeamCertificate::decode(bin(2)?)?,
                },
            }),
            (1, 1) => Ok(Self::LocalAcceptance(LocalInviteAcceptance::decode(bin(
                0,
            )?)?)),
            (2, 2) => Ok(Self::Rejection {
                team: EntityId::from_bytes(bin(0)?.to_vec())?,
                receipt: TeamRsvp::decode(bin(1)?)?,
            }),
            _ => Err(Error::OperationBinding("invitation fields")),
        }
    }
}
impl FoksClient {
    pub fn prepare_invitation_operation(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        intent: InvitationIntent,
        protected: &mut impl ProtectedMutationStore,
    ) -> Result<InvitationProgress> {
        self.authenticate_credential_and_pin(host, credential)?;
        let subject = match &intent {
            InvitationIntent::RemoteAcceptance(p) => {
                let decoded = p.home_link.link.decode_generic()?;
                if &p.invite.host == host.host_id()
                    || &decoded.entity != credential.uid()
                    || &decoded.host != host.host_id()
                {
                    return Err(Error::OperationBinding("remote invitation source"));
                }
                p.team.clone()
            }
            InvitationIntent::Certificate { team, prepared } => {
                let p =
                    foks_crypto::verify_team_certificate(&prepared.certificate, &prepared.invite)?;
                if &p.team.team != team || &p.team.host != host.host_id() {
                    return Err(Error::OperationBinding("certificate destination"));
                }
                team.clone()
            }
            InvitationIntent::LocalAcceptance(a) => {
                if &a.invite.host != host.host_id() {
                    return Err(Error::OperationBinding("acceptance host"));
                }
                self.preview_team_invitation(host, &a.invite)?
                    .advertised
                    .team
                    .team
            }
            InvitationIntent::Rejection { team, .. } => team.clone(),
        };
        let material = Zeroizing::new(intent.encode()?);
        let operation = MutationCoordinator::new(&host.database_path, protected).prepare(
            MutationDraft {
                operation_id: random_bytes()?,
                kind: MutationKind::Invitation,
                host_id: host.host_id().as_bytes().to_vec(),
                scope_id: credential.uid().as_bytes().to_vec(),
                subject_id: subject.as_bytes().to_vec(),
                expected_version: None,
                request_hash: foks_crypto::prefixed_hash(0x6d8c_97c9_315a_1084, &material),
            },
            material,
        )?;
        Ok(InvitationProgress {
            operation,
            receipt: None,
            invite: None,
        })
    }
    pub fn invitation_operation(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        id: [u8; 16],
    ) -> Result<MutationOperation> {
        let op = HardStateStore::open(&host.database_path)?
            .mutation(&id)?
            .ok_or(Error::OperationBinding("invitation operation missing"))?;
        if op.kind != MutationKind::Invitation
            || op.host_id != host.host_id().as_bytes()
            || op.scope_id != uid.as_bytes()
        {
            return Err(Error::OperationBinding("invitation operation scope"));
        }
        Ok(op)
    }
    pub fn invitation_progress(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        id: [u8; 16],
        attempt: bool,
        protected: &mut impl ProtectedMutationStore,
    ) -> Result<InvitationProgress> {
        let mut op = self.invitation_operation(host, credential.uid(), id)?;
        if op.state.is_terminal() {
            crate::mutation::remove_terminal_material(protected, &op.material_ref)?;
            return Ok(InvitationProgress {
                operation: op,
                receipt: None,
                invite: None,
            });
        }
        let intent = InvitationIntent::decode(
            &MutationCoordinator::new(&host.database_path, protected).load_bound_material(&op)?,
        )?;
        if matches!(&intent, InvitationIntent::RemoteAcceptance(_)) {
            return Err(Error::OperationBinding(
                "remote invitation requires its destination profile",
            ));
        }
        // A durable acknowledgement survives a crash before RemoteVerified.
        let receipt_key = [id.as_slice(), b"/invitation-ack"].concat();
        let ack = match protected.get(&receipt_key) {
            Ok(bytes) => Some(bytes),
            Err(ProtectedStoreError::Missing) => None,
            Err(e) => return Err(Error::ProtectedMaterial(e.to_string())),
        };
        let invite = match &intent {
            InvitationIntent::Certificate { prepared, .. } => Some(prepared.invite.export()?),
            _ => None,
        };
        let mut receipt = None;
        if let Some(ack) = ack {
            if !ack.is_empty() {
                receipt = Some(TeamRsvp::decode(&ack)?);
            }
            MutationCoordinator::new(&host.database_path, protected).remote_verified(&id)?;
        } else if op.state == MutationState::Prepared && attempt {
            // Authenticate before crossing the no-replay boundary. Failure afterwards
            // remains unknown, including SSO challenges; reauthentication never resends.
            self.authenticate_credential_and_pin(host, credential)?;
            MutationCoordinator::new(&host.database_path, protected).begin_submission(&id)?;
            let sent = match &intent {
                InvitationIntent::RemoteAcceptance(_) => {
                    return Err(Error::OperationBinding(
                        "remote invitation requires its destination profile",
                    ))
                }
                InvitationIntent::Certificate { prepared, .. } => self
                    .upload_team_invitation(host, credential, prepared)
                    .map(|_| None),
                InvitationIntent::LocalAcceptance(a) => self
                    .submit_local_invitation_acceptance(host, credential, a)
                    .map(Some),
                InvitationIntent::Rejection { team, receipt } => self
                    .reject_team_invitation(host, credential, team, receipt)
                    .map(|_| None),
            };
            match sent {
                Ok(r) => {
                    let ack = r
                        .as_ref()
                        .map(TeamRsvp::encoded)
                        .transpose()?
                        .unwrap_or_default();
                    protected
                        .put_if_absent(&receipt_key, &ack)
                        .map_err(|e| Error::ProtectedMaterial(e.to_string()))?;
                    MutationCoordinator::new(&host.database_path, protected)
                        .remote_verified(&id)?;
                    receipt = r;
                }
                Err(e) => {
                    MutationCoordinator::new(&host.database_path, protected)
                        .submission_unknown(&id)?;
                    return Err(e);
                }
            }
        } else if matches!(
            op.state,
            MutationState::Submitting | MutationState::SubmissionUnknown
        ) {
            if let InvitationIntent::Certificate { prepared, .. } = &intent {
                if self
                    .preview_team_invitation(host, &prepared.invite)
                    .is_ok_and(|p| p.certificate == prepared.certificate)
                {
                    MutationCoordinator::new(&host.database_path, protected)
                        .remote_verified(&id)?;
                }
            }
            // Neither a Requested link nor absence from a pending-only inbox proves
            // delivery/rejection. Preserve uncertainty without a second submission.
        }
        op = self.invitation_operation(host, credential.uid(), id)?;
        Ok(InvitationProgress {
            operation: op,
            receipt,
            invite,
        })
    }
}
