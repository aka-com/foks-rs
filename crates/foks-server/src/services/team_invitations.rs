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
