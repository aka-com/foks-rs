//! Certificate and scoped permission service. The writer rechecks current authority.
use crate::{auth::Principal, Entropy, WriterHandle};
use foks_proto::{
    EntityId, LocalViewPermissionPayload, Role, Signature, TeamCertificate, TeamCertificatePayload,
    TeamInvite,
};
use foks_rpc::RpcStatus;
use foks_server_db::{CertificateUpload, InvitationActor, ReadDatabase, TeamGrantAuthority};
use foks_snowpack::{decode, encode, Value};
use std::sync::Arc;
pub(crate) struct InvitationService<'a> {
    pub host: &'a EntityId,
    pub reader: &'a ReadDatabase,
    pub writer: &'a WriterHandle,
    pub clock: &'a Arc<dyn foks_server_db::Clock>,
    pub entropy: &'a dyn Entropy,
}
fn bad(_: impl std::fmt::Display) -> RpcStatus {
    RpcStatus::BadArguments("invalid invitation request".into())
}
fn internal(_: impl std::fmt::Display) -> RpcStatus {
    RpcStatus::TransactionRetry
}
fn denied() -> RpcStatus {
    RpcStatus::PermissionDenied("invitation authority".into())
}
fn write_error(e: crate::Error) -> RpcStatus {
    crate::error::mutation_failure_status(&e).unwrap_or(match e {
        crate::Error::WriterQueue => RpcStatus::RateLimited,
        crate::Error::Database(foks_server_db::Error::InvitationAlreadyPending) => {
            RpcStatus::TeamInviteAlreadyAccepted
        }
        crate::Error::Database(foks_server_db::Error::InvitationDecisionConflict) => {
            RpcStatus::TeamRace("invitation decision conflict".into())
        }
        crate::Error::Database(foks_server_db::Error::QuotaExceeded) => RpcStatus::QuotaExceeded,
        _ => RpcStatus::TransactionRetry,
    })
}
fn fields(bytes: &[u8], n: usize) -> Result<Vec<Value>, RpcStatus> {
    if bytes.len() > 32768 {
        return Err(bad("limit"));
    }
    match decode(bytes).map_err(bad)? {
        Value::Array(f) if f.len() == n => Ok(f),
        _ => Err(bad("fields")),
    }
}
fn token(v: &Value) -> Result<[u8; 16], RpcStatus> {
    if let Value::Binary(b) = v {
        b.as_slice().try_into().map_err(bad)
    } else {
        Err(bad("token"))
    }
}
impl InvitationService<'_> {
    pub fn certificate_lookup(&self, bytes: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        let f = fields(bytes, 1)?;
        let i = TeamInvite::decode(&encode(&f[0]).map_err(bad)?).map_err(bad)?;
        if i.host != *self.host {
            return Err(denied());
        }
        let snapshot = self.reader.snapshot().map_err(internal)?;
        let exact = snapshot
            .invitation_certificate(&i.hash)
            .map_err(internal)?
            .ok_or_else(|| RpcStatus::NotFound("team certificate".into()))?;
        let certificate = TeamCertificate::decode(&exact).map_err(internal)?;
        let p = TeamCertificatePayload::decode(&certificate.payload).map_err(internal)?;
        let t = snapshot
            .team(p.team.team.as_bytes())
            .map_err(internal)?
            .ok_or_else(denied)?;
        let links = t
            .links
            .iter()
            .map(|l| foks_proto::UserLink::decode(&l.exact_link))
            .collect::<Result<Vec<_>, _>>()
            .map_err(internal)?;
        let index_range = foks_verify::persisted_team_index_range(&links).map_err(internal)?;
        foks_proto::TeamCertificateAndMetadata {
            certificate,
            index_range,
        }
        .encoded()
        .map_err(internal)
    }
    pub fn certificate_put(&self, bytes: &[u8], principal: &Principal) -> Result<(), RpcStatus> {
        principal.require_ordinary_device()?;
        let f = fields(bytes, 2)?;
        let tok = token(&f[0])?;
        let exact = encode(&f[1]).map_err(bad)?;
        let cert = TeamCertificate::decode(&exact).map_err(bad)?;
        let invite = foks_crypto::team_certificate_invite(&cert).map_err(bad)?;
        let p = foks_crypto::verify_team_certificate(&cert, &invite)
            .map_err(|_| RpcStatus::TeamCertificate("invalid certificate signature".into()))?;
        if p.team.host != *self.host {
            return Err(denied());
        }
        let uid = principal.uid().to_vec();
        let credential = principal.device_id().to_vec();
        let hepk = p.hepk.encoded().map_err(bad)?;
        let hash = crate::auth::team::admin_token_hash(&tok);
        self.writer
            .call_with_current_time(Arc::clone(self.clock), move |db, now| {
                db.put_invitation_certificate(
                    InvitationActor {
                        uid: &uid,
                        credential: &credential,
                    },
                    &hash,
                    CertificateUpload {
                        hash: &invite.hash,
                        team: p.team.team.as_bytes(),
                        generation: p.key.generation,
                        verify_key: p.key.verify_key.as_bytes(),
                        exact_hepk: &hepk,
                        exact: &exact,
                    },
                    now,
                )?;
                Ok(())
            })
            .map_err(write_error)
    }
    pub fn certificate_list(
        &self,
        bytes: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus> {
        principal.require_ordinary_device()?;
        let f = fields(bytes, 1)?;
        let tok = token(&f[0])?;
        let now = self.clock.now_micros().map_err(internal)?;
        let snapshot = self.reader.snapshot().map_err(internal)?;
        if snapshot
            .active_credential_owner(principal.uid(), principal.device_id())
            .map_err(internal)?
            .is_none()
        {
            return Err(denied());
        }
        snapshot
            .sso_require_access(principal.uid(), now / 1000)
            .map_err(|e| write_error(e.into()))?;
        let authority = snapshot
            .resolve_team_admin_token(&crate::auth::team::admin_token_hash(&tok), now)
            .map_err(internal)?
            .ok_or_else(denied)?;
        if authority.holder_id != principal.uid() {
            return Err(denied());
        }
        let rows = snapshot
            .current_invitation_certificates(&authority.team_id)
            .map_err(internal)?;
        encode(&if rows.is_empty() {
            Value::Null
        } else {
            Value::Array(
                rows.iter()
                    .map(|b| decode(b))
                    .collect::<Result<_, _>>()
                    .map_err(internal)?,
            )
        })
        .map_err(internal)
    }
    pub fn grant_local_view(
        &self,
        bytes: &[u8],
        principal: &Principal,
        team: bool,
    ) -> Result<Vec<u8>, RpcStatus> {
        principal.require_ordinary_device()?;
        let f = fields(bytes, if team { 2 } else { 1 })?;
        let exact = encode(&f[0]).map_err(bad)?;
        let p = LocalViewPermissionPayload::decode(&exact).map_err(bad)?;
        let now = self.clock.now_micros().map_err(internal)?;
        let signing = if team {
            if !matches!(p.viewee.entity_type(), 3 | 20)
                || !super::protocol_time_is_nowish(p.time, now)
            {
                return Err(denied());
            }
            let Value::Array(sig) = &f[1] else {
                return Err(bad("signature"));
            };
            let [signature, Value::Unsigned(generation), role] = sig.as_slice() else {
                return Err(bad("signature"));
            };
            let role = Role::decode(&encode(role).map_err(bad)?).map_err(bad)?;
            let signature = Signature::decode(&encode(signature).map_err(bad)?).map_err(bad)?;
            let snapshot = self
                .reader
                .team(p.viewee.as_bytes())
                .map_err(internal)?
                .ok_or_else(denied)?;
            let (kind, visibility) = crate::auth::team::role_parts(role);
            let key = snapshot
                .shared_keys
                .iter()
                .filter(|k| k.role_type == kind && k.visibility == visibility)
                .max_by_key(|k| k.generation)
                .filter(|k| k.generation == *generation)
                .ok_or_else(denied)?;
            let id = EntityId::from_bytes(key.verify_key.clone()).map_err(internal)?;
            foks_crypto::verify_typed(
                &id,
                &signature,
                foks_proto::LOCAL_VIEW_PERMISSION_TYPE_ID,
                &exact,
            )
            .map_err(|_| denied())?;
            Some((kind, visibility, *generation, key.verify_key.clone()))
        } else {
            if p.viewee.as_bytes() != principal.uid() {
                return Err(denied());
            }
            None
        };
        let uid = principal.uid().to_vec();
        let credential = principal.device_id().to_vec();
        self.writer
            .call_with_current_time(Arc::clone(self.clock), move |db, time| {
                let authority = signing
                    .as_ref()
                    .map(
                        |(role_type, visibility, generation, key)| TeamGrantAuthority {
                            role_type: *role_type,
                            visibility: *visibility,
                            generation: *generation,
                            verify_key: key,
                        },
                    );
                db.grant_local_view(
                    InvitationActor {
                        uid: &uid,
                        credential: &credential,
                    },
                    &p,
                    authority.as_ref(),
                    time,
                )?;
                Ok(())
            })
            .map_err(write_error)?;
        // Go local grants return a freshly allocated token even when updating a scope.
        // Local loading checks the authenticated viewer and stored scope, not this token.
        let mut tok = [0; 17];
        self.entropy.fill(&mut tok).map_err(internal)?;
        tok[0] = 54;
        foks_proto::PermissionToken::new(tok)
            .encoded()
            .map_err(internal)
    }
}

impl InvitationService<'_> {
    #[allow(clippy::too_many_arguments)]
    pub fn accept_local(
        &self,
        bytes: &[u8],
        principal: &Principal,
        keys: &Arc<dyn crate::keys::HostKeyProvider>,
        tail: &foks_proto::HostchainTail,
    ) -> Result<Vec<u8>, RpcStatus> {
        principal.require_ordinary_device()?;
        let f = fields(bytes, 4)?;
        let invite = TeamInvite::decode(&encode(&f[0]).map_err(bad)?).map_err(bad)?;
        if invite.host != *self.host {
            return Err(denied());
        }
        let role = Role::decode(&encode(&f[1]).map_err(bad)?).map_err(bad)?;
        let token = if f[2] == Value::Null {
            None
        } else {
            Some(token(&f[2])?)
        };
        let link = if f[3] == Value::Null {
            None
        } else {
            Some(
                foks_proto::PostGenericLinkArgument::decode(&encode(&f[3]).map_err(bad)?)
                    .map_err(bad)?,
            )
        };
        let snapshot = self.reader.snapshot().map_err(internal)?;
        let exact = snapshot
            .invitation_certificate(&invite.hash)
            .map_err(internal)?
            .ok_or_else(|| RpcStatus::NotFound("team certificate".into()))?;
        let cert = TeamCertificate::decode(&exact).map_err(internal)?;
        let advertised = TeamCertificatePayload::decode(&cert.payload).map_err(internal)?;
        let now = self.clock.now_micros().map_err(internal)?;
        let source_admin = token.as_ref().map(crate::auth::team::admin_token_hash);
        let (joiner, source_head, destination_head, authorized_signer) = if let Some(hash) =
            source_admin
        {
            let authority = snapshot
                .resolve_team_admin_token(&hash, now)
                .map_err(internal)?
                .ok_or_else(denied)?;
            if authority.holder_id != principal.uid() {
                return Err(denied());
            }
            let source = snapshot
                .team(&authority.team_id)
                .map_err(internal)?
                .ok_or_else(denied)?;
            let destination = snapshot
                .team(advertised.team.team.as_bytes())
                .map_err(internal)?
                .ok_or_else(denied)?;
            let range =
                |t: &foks_server_db::TeamSnapshot| -> Result<foks_proto::RationalRange, RpcStatus> {
                    let links = t
                        .links
                        .iter()
                        .map(|l| foks_proto::UserLink::decode(&l.exact_link))
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(internal)?;
                    foks_verify::persisted_team_index_range(&links).map_err(internal)
                };
            if !foks_verify::rational_range_strictly_before(&range(&source)?, &range(&destination)?)
                .map_err(internal)?
            {
                return Err(RpcStatus::TeamRoster(
                    "joining team index ranges overlap".into(),
                ));
            }
            let head = |t: &foks_server_db::TeamSnapshot| -> Result<[u8; 32], RpcStatus> {
                let last = t.links.last().ok_or_else(denied)?;
                Ok(foks_crypto::prefixed_hash(
                    foks_proto::LINK_OUTER_TYPE_ID,
                    &last.exact_link,
                ))
            };
            (
                EntityId::from_bytes(authority.team_id).map_err(internal)?,
                Some(head(&source)?),
                Some(head(&destination)?),
                None::<Vec<u8>>,
            )
        } else {
            if role != Role::OWNER {
                return Err(denied());
            }
            (
                EntityId::from_bytes(principal.uid().to_vec()).map_err(internal)?,
                None,
                None,
                None,
            )
        };
        let mut rsvp = [0; 17];
        self.entropy.fill(&mut rsvp).map_err(internal)?;
        rsvp[0] = 57;
        let mut permission = [0; 17];
        self.entropy.fill(&mut permission).map_err(internal)?;
        permission[0] = 54;
        let acceptance = foks_server_db::LocalInvitationAcceptance {
            uid: principal.uid().to_vec(),
            credential: principal.device_id().to_vec(),
            certificate_hash: invite.hash,
            destination: advertised.team.team.as_bytes().to_vec(),
            joiner: joiner.clone(),
            source_role: role,
            source_admin,
            source_head,
            destination_head,
            rsvp,
            permission,
        };
        drop(snapshot);
        if let Some(link) = link {
            let decoded = link.link.decode_generic().map_err(bad)?;
            let foks_proto::GenericLinkPayload::TeamMembership(m) = decoded.payload else {
                return Err(bad("requested membership"));
            };
            if m.team != advertised.team.team
                || m.team_host != *self.host
                || m.source_role != role
                || m.state != foks_proto::TeamMembershipState::Requested
            {
                return Err(bad("requested membership binding"));
            }
            super::generic::commit_for_entity_with_invitation(
                link,
                principal,
                &joiner,
                authorized_signer.as_deref(),
                self.host,
                self.writer,
                keys,
                self.clock,
                tail,
                None,
                Some(acceptance),
            )?;
        } else {
            self.writer
                .call_with_current_time(Arc::clone(self.clock), move |db, time| {
                    db.accept_local_invitation(&acceptance, time)?;
                    Ok(())
                })
                .map_err(write_error)?;
        }
        foks_proto::TeamRsvp::new(rsvp)
            .map_err(internal)?
            .encoded()
            .map_err(internal)
    }
    pub fn accept_remote(&self, bytes: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        let f = fields(bytes, 2)?;
        let invite = TeamInvite::decode(&encode(&f[0]).map_err(bad)?).map_err(bad)?;
        if invite.host != *self.host {
            return Err(denied());
        }
        let request =
            foks_proto::RemoteJoinRequest::decode(&encode(&f[1]).map_err(bad)?).map_err(bad)?;
        let preview =
            foks_proto::TeamCertificateAndMetadata::decode(&self.certificate_lookup(
                &encode(&Value::Array(vec![invite.to_value()])).map_err(bad)?,
            )?)
            .map_err(internal)?;
        let p = foks_crypto::verify_team_certificate(&preview.certificate, &invite).map_err(bad)?;
        if foks_crypto::hepk_fingerprint(&p.hepk).map_err(bad)? != request.hepk_fingerprint {
            return Err(bad("encryption recipient"));
        }
        let snapshot = self.reader.snapshot().map_err(internal)?;
        let team = snapshot
            .team(p.team.team.as_bytes())
            .map_err(internal)?
            .ok_or_else(denied)?;
        let links = team
            .links
            .iter()
            .map(|l| foks_proto::UserLink::decode(&l.exact_link))
            .collect::<Result<Vec<_>, _>>()
            .map_err(internal)?;
        let range = foks_verify::persisted_team_index_range(&links).map_err(internal)?;
        if let Some(source) = &request.visible.index_range {
            if !foks_verify::rational_range_strictly_before(source, &range).map_err(bad)? {
                return Err(RpcStatus::TeamRoster("joining team index range".into()));
            }
        }
        let destination_head = foks_crypto::prefixed_hash(
            foks_proto::LINK_OUTER_TYPE_ID,
            &team.links.last().ok_or_else(denied)?.exact_link,
        );
        let mut rsvp = [0; 17];
        self.entropy.fill(&mut rsvp).map_err(internal)?;
        rsvp[0] = 56;
        let acceptance = foks_server_db::RemoteInvitationAcceptance {
            certificate_hash: invite.hash,
            team: p.team.team.as_bytes().to_vec(),
            generation: p.key.generation,
            exact_hepk: p.hepk.encoded().map_err(bad)?,
            exact_request: request.encoded().map_err(bad)?,
            rsvp,
            destination_head,
        };
        drop(snapshot);
        self.writer
            .call_with_current_time(Arc::clone(self.clock), move |db, now| {
                db.accept_remote_invitation(&acceptance, now)?;
                Ok(())
            })
            .map_err(write_error)?;
        foks_proto::TeamRsvp::new(rsvp)
            .map_err(internal)?
            .encoded()
            .map_err(internal)
    }
    pub fn load_remote(&self, bytes: &[u8], principal: &Principal) -> Result<Vec<u8>, RpcStatus> {
        principal.require_ordinary_device()?;
        let f = fields(bytes, 2)?;
        let token = crate::auth::team::admin_token_hash(&token(&f[0])?);
        let rsvp = foks_proto::TeamRsvp::decode(&encode(&f[1]).map_err(bad)?).map_err(bad)?;
        if !rsvp.is_remote() {
            return Err(bad("remote rsvp"));
        }
        let now = self.clock.now_micros().map_err(internal)?;
        let snapshot = self.reader.snapshot().map_err(internal)?;
        let authority = snapshot
            .resolve_team_admin_token(&token, now)
            .map_err(internal)?
            .ok_or_else(denied)?;
        if authority.holder_id != principal.uid()
            || snapshot
                .active_credential_owner(principal.uid(), principal.device_id())
                .map_err(internal)?
                .is_none()
        {
            return Err(denied());
        }
        snapshot
            .sso_require_access(principal.uid(), now / 1000)
            .map_err(|e| write_error(e.into()))?;
        snapshot
            .remote_invitation_request(&authority.team_id, rsvp.expose())
            .map_err(internal)?
            .ok_or_else(|| RpcStatus::NotFound("remote join request".into()))
    }
    pub fn inbox(&self, bytes: &[u8], principal: &Principal) -> Result<Vec<u8>, RpcStatus> {
        principal.require_ordinary_device()?;
        let f = fields(bytes, 2)?;
        let tok = token(&f[0])?;
        let pagination = if f[1] == Value::Null {
            foks_proto::InboxPagination {
                start: 0,
                end: 0,
                limit: 100,
            }
        } else {
            foks_proto::InboxPagination::decode(&encode(&f[1]).map_err(bad)?).map_err(bad)?
        };
        let now = self.clock.now_micros().map_err(internal)?;
        let snapshot = self.reader.snapshot().map_err(internal)?;
        let authority = snapshot
            .resolve_team_admin_token(&crate::auth::team::admin_token_hash(&tok), now)
            .map_err(internal)?
            .ok_or_else(denied)?;
        if authority.holder_id != principal.uid()
            || snapshot
                .active_credential_owner(principal.uid(), principal.device_id())
                .map_err(internal)?
                .is_none()
        {
            return Err(denied());
        }
        snapshot
            .sso_require_access(principal.uid(), now / 1000)
            .map_err(|e| write_error(e.into()))?;
        let mut rows = snapshot
            .local_invitation_inbox(&authority.team_id, pagination)
            .map_err(internal)?;
        rows.extend(
            snapshot
                .remote_invitation_inbox(&authority.team_id, pagination)
                .map_err(internal)?,
        );
        rows.sort_by(|a, b| {
            b.time
                .cmp(&a.time)
                .then_with(|| a.rsvp.expose().cmp(b.rsvp.expose()))
        });
        foks_proto::encode_team_inbox(&rows).map_err(internal)
    }
    pub fn post_removal(&self, bytes: &[u8], principal: &Principal) -> Result<(), RpcStatus> {
        principal.require_ordinary_device()?;
        let f = fields(bytes, 2)?;
        let hash = crate::auth::team::admin_token_hash(&token(&f[0])?);
        let removal = foks_proto::TeamRemovalAndCommitment::decode(&encode(&f[1]).map_err(bad)?)
            .map_err(bad)?;
        let uid = principal.uid().to_vec();
        let credential = principal.device_id().to_vec();
        self.writer
            .call_with_current_time(Arc::clone(self.clock), move |db, now| {
                db.post_team_removal(
                    InvitationActor {
                        uid: &uid,
                        credential: &credential,
                    },
                    &hash,
                    &removal,
                    now,
                )?;
                Ok(())
            })
            .map_err(write_error)
    }
    pub fn reject(&self, bytes: &[u8], principal: &Principal) -> Result<(), RpcStatus> {
        principal.require_ordinary_device()?;
        let f = fields(bytes, 2)?;
        let token = crate::auth::team::admin_token_hash(&token(&f[0])?);
        let rsvp = foks_proto::TeamRsvp::decode(&encode(&f[1]).map_err(bad)?).map_err(bad)?;
        let uid = principal.uid().to_vec();
        let credential = principal.device_id().to_vec();
        self.writer
            .call_with_current_time(Arc::clone(self.clock), move |db, time| {
                if rsvp.is_remote() {
                    db.reject_remote_invitation(
                        InvitationActor {
                            uid: &uid,
                            credential: &credential,
                        },
                        &token,
                        rsvp.expose(),
                        time,
                    )?;
                    return Ok(());
                }
                db.reject_local_invitation(
                    InvitationActor {
                        uid: &uid,
                        credential: &credential,
                    },
                    &token,
                    rsvp.expose(),
                    time,
                )?;
                Ok(())
            })
            .map_err(write_error)
    }
}
