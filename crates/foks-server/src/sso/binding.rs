use super::*;
use foks_proto::{
    EntityId, OAuth2PollResult, OAuth2Secret, OAuth2TokenSet, PollOAuth2SessionArgument,
    RegSsoArgs, UsernameReservation,
};
use foks_server_db::{SsoAccess, SsoAccessState, SsoAccountBinding};

impl SsoService {
    pub async fn poll(
        self: &Arc<Self>,
        arg: PollOAuth2SessionArgument,
    ) -> std::result::Result<Vec<u8>, foks_rpc::RpcStatus> {
        let _permit = self
            .polling
            .clone()
            .try_acquire_owned()
            .map_err(|_| foks_rpc::RpcStatus::RateLimited)?;
        let deadline = tokio::time::Instant::now()
            + Duration::from_millis(arg.wait_duration_ms.min(foks_oidc::MAX_POLL_MS));
        loop {
            let mut changed = Box::pin(self.changed.clone().notified_owned());
            changed.as_mut().enable();
            let service = self.clone();
            let arg = arg.clone();
            let result = tokio::task::spawn_blocking(move || service.poll_once(&arg))
                .await
                .map_err(|_| foks_rpc::RpcStatus::TransactionRetry)?
                .map_err(status)?;
            if let Some(bytes) = result {
                return Ok(bytes);
            }
            if tokio::time::timeout_at(deadline, changed).await.is_err() {
                return Err(foks_rpc::RpcStatus::Timeout);
            }
        }
    }
    fn poll_once(&self, arg: &PollOAuth2SessionArgument) -> Result<Option<Vec<u8>>> {
        // CAS losers reload the already-persisted result/reservation instead of making a second one.
        for _ in 0..4 {
            let row = self.row(&arg.id)?;
            if arg.for_login != row.uid.is_some() {
                return Err(Error::Sso("session purpose mismatch"));
            }
            match row.state {
                SsoSessionState::Waiting | SsoSessionState::Exchanging => return Ok(None),
                SsoSessionState::Ready | SsoSessionState::Completed => {}
                SsoSessionState::Denied => return Err(Error::Sso("authorization denied")),
                SsoSessionState::ExchangeUnknown => {
                    return Err(Error::Sso(
                        "exchange outcome unknown; new authentication required",
                    ))
                }
                _ => return Err(Error::Sso("authentication rejected or expired")),
            }
            let mut payload = self.session_payload(&row)?;
            if payload.expires_at_ms <= self.now_ms()? {
                return Err(Error::Sso("provider token expired"));
            }
            if let Some(reservation) = &payload.poll_reservation {
                return Ok(Some(poll_result(
                    &payload,
                    UsernameReservation::decode(reservation)?,
                )?));
            }
            if row.state != SsoSessionState::Ready {
                return Err(Error::Sso("completed session has no durable poll result"));
            }
            let mut reservation = UsernameReservation {
                token: [0; 17],
                sequence: 0,
                expires_at: 0,
            };
            let normalized = if arg.for_login {
                None
            } else {
                let name = foks_verify::normalize_username(payload.username.as_bytes())
                    .ok_or(Error::Sso("provider username is invalid"))?;
                self.entropy.fill(&mut reservation.token)?;
                reservation.sequence = 1;
                reservation.expires_at = row.expires_at_ms.min(payload.expires_at_ms);
                Some(name)
            };
            let result = poll_result(&payload, reservation.clone())?;
            payload.poll_reservation = Some(reservation.encoded()?);
            let mut next = row.clone();
            next.revision = next
                .revision
                .checked_add(1)
                .ok_or(Error::Sso("session revision overflow"))?;
            next.ciphertext = self.seal(&next, &payload)?;
            let ciphertext = next.ciphertext;
            match self
                .writer
                .call_with_current_time(self.clock.clone(), move |db, now| {
                    db.sso_store_poll(
                        &row,
                        &ciphertext,
                        normalized
                            .as_deref()
                            .map(|name| (name, &reservation.token, reservation.expires_at)),
                        now / 1000,
                    )?;
                    Ok(())
                }) {
                Ok(()) => return Ok(Some(result)),
                Err(Error::Database(foks_server_db::Error::AuthorizationChanged)) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(Error::Sso("session changed during polling"))
    }
    pub(crate) fn prepare_signup(
        &self,
        arg: &RegSsoArgs,
        uid: &EntityId,
        device: &EntityId,
        username: &[u8],
        email: &[u8],
        reservation: &UsernameReservation,
    ) -> Result<SsoAccountBinding> {
        self.prepare_binding(arg, uid, Some((device, username, email, reservation)))?
            .ok_or(Error::Sso("signup was already bound"))
    }
    pub fn login(&self, arg: &foks_proto::SsoLoginArgument) -> Result<()> {
        let Some(binding) = self.prepare_binding(&arg.args, &arg.uid, None)? else {
            return Ok(());
        };
        self.writer
            .call_with_current_time(self.clock.clone(), move |db, now| {
                db.sso_login(&binding, now / 1000)?;
                Ok(())
            })?;
        self.changed.notify_waiters();
        Ok(())
    }
    fn prepare_binding(
        &self,
        arg: &RegSsoArgs,
        uid: &EntityId,
        signup: Option<(&EntityId, &[u8], &[u8], &UsernameReservation)>,
    ) -> Result<Option<SsoAccountBinding>> {
        let RegSsoArgs::Oauth2 { id, binding } = arg else {
            return Err(Error::Sso("OIDC is required by this host"));
        };
        let row = self.row(id)?;
        let mut session_payload = self.session_payload(&row)?;
        let binding_payload = foks_crypto::verify_oauth2_binding(binding)?;
        let uid_bytes: [u8; 33] = uid
            .as_bytes()
            .try_into()
            .map_err(|_| Error::Sso("invalid UID"))?;
        let hash = foks_crypto::prefixed_hash(0x68b3_c398_ea5f_d71e, &arg.encoded()?);
        if binding_payload.binding.uid != *uid
            || binding_payload.binding.host.as_bytes() != self.host
            || foks_crypto::oauth2_binding_nonce(&binding_payload.binding)? != session_payload.nonce
            || binding_payload.id_token.expose() != session_payload.id_token
            || session_payload.expires_at_ms <= self.now_ms()?
        {
            return Err(Error::Sso("signed token binding mismatch"));
        }
        if row.state == SsoSessionState::Completed {
            if signup.is_none()
                && row.uid == Some(uid_bytes)
                && session_payload.bound_uid.as_deref() == Some(uid_bytes.as_slice())
                && session_payload.binding_hash == Some(hash)
            {
                return Ok(None);
            }
            return Err(Error::Sso("session was already bound"));
        }
        if row.state != SsoSessionState::Ready || row.uid != signup.is_none().then_some(uid_bytes) {
            return Err(Error::Sso("binding session purpose or state mismatch"));
        }
        let host = self.host;
        let config_hash = self.config_hash;
        let root = binding_payload.binding.root.clone();
        let device = binding.key.as_bytes().to_vec();
        let (old, sequence) = self.writer.call(move |db| {
            db.sso_require_policy(&host, &config_hash)?;
            if db
                .root_at(root.epoch)?
                .is_none_or(|r| r.root_hash != root.hash)
            {
                return Err(Error::Sso("unknown signed Merkle root"));
            }
            let old = db.sso_access(&uid_bytes)?;
            let sequence = db.user_authority(&uid_bytes)?.map(|s| s.chain_sequence);
            if sequence.is_some() && db.active_credential_owner(&uid_bytes, &device)?.is_none() {
                return Err(Error::Sso("binding signer is no longer active"));
            }
            Ok((old, sequence))
        })?;
        let purpose = if signup.is_some() {
            foks_proto::SsoPurpose::Signup
        } else if old.is_some() {
            foks_proto::SsoPurpose::Reauthenticate
        } else {
            foks_proto::SsoPurpose::LinkExisting
        };
        let authorization_generation = old
            .as_ref()
            .map_or(Some(1), |old| old.authorization_generation.checked_add(1))
            .ok_or(Error::Sso("authorization generation overflow"))?;
        let revision = if let Some((device, username, email, reservation)) = signup {
            if binding.key != *device
                || username != session_payload.username.as_bytes()
                || email != session_payload.email.as_deref().unwrap_or("").as_bytes()
                || old.is_some()
                || sequence.is_some()
            {
                return Err(Error::Sso("signup provider identity or signer mismatch"));
            }
            let stored = UsernameReservation::decode(
                session_payload
                    .poll_reservation
                    .as_deref()
                    .ok_or(Error::Sso("signup must use its reserved poll result"))?,
            )?;
            if stored != *reservation {
                return Err(Error::Sso("signup reservation mismatch"));
            }
            1
        } else if let Some(old) = old {
            if old.issuer != session_payload.issuer || old.subject != session_payload.subject {
                return Err(Error::Sso("provider subject mismatch"));
            }
            old.revision
                .checked_add(1)
                .ok_or(Error::Sso("access revision overflow"))?
        } else {
            if sequence.is_none() {
                return Err(Error::Sso("account has no active owner"));
            }
            1
        };
        session_payload.binding_hash = Some(hash);
        session_payload.bound_uid = Some(uid_bytes.to_vec());
        let mut completed = row.clone();
        completed.state = SsoSessionState::Completed;
        completed.revision = completed
            .revision
            .checked_add(1)
            .ok_or(Error::Sso("session revision overflow"))?;
        let completed_ciphertext = self.seal(&completed, &session_payload)?;
        let mut access = SsoAccess {
            host: self.host,
            uid: uid_bytes,
            issuer: session_payload.issuer.clone(),
            subject: session_payload.subject.clone(),
            config_hash: self.config_hash,
            revision,
            authorization_epoch: row.authorization_epoch,
            authorization_generation,
            interrupted: false,
            state: SsoAccessState::Active,
            expires_at_ms: session_payload.expires_at_ms,
            ciphertext: Vec::new(),
        };
        access.ciphertext = self.seal_access(&access, &session_payload)?;
        Ok(Some(SsoAccountBinding {
            purpose,
            commitment: hash,
            flow: row,
            completed_ciphertext,
            access,
            device: binding.key.as_bytes().to_vec(),
            expected_user_sequence: sequence,
        }))
    }
}
pub(crate) fn status(error: Error) -> foks_rpc::RpcStatus {
    use foks_rpc::RpcStatus;
    match error {
        Error::Database(foks_server_db::Error::NameInUse) => RpcStatus::NameInUse,
        Error::Database(foks_server_db::Error::Capacity(_)) | Error::WriterQueue => {
            RpcStatus::RateLimited
        }
        Error::Sso(message) => RpcStatus::OAuth2(message.into()),
        Error::Oidc(foks_oidc::Error::ProviderUnavailable) => {
            RpcStatus::OAuth2("provider unavailable".into())
        }
        _ => RpcStatus::OAuth2("authentication could not be verified".into()),
    }
}

fn poll_result(payload: &SsoSessionPayload, reservation: UsernameReservation) -> Result<Vec<u8>> {
    Ok(OAuth2PollResult {
        tokens: OAuth2TokenSet {
            access_token: OAuth2Secret::new(payload.access_token.clone()),
            id_token: OAuth2Secret::new(payload.id_token.clone()),
            expires_at: payload.expires_at_ms,
            username: payload.username.clone(),
        },
        reservation,
    }
    .encoded()?)
}
