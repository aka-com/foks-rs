use super::*;
use foks_server_db::{SsoAccess, SsoAccessState};
use zeroize::Zeroize as _;
impl SsoService {
    pub(super) fn seal_access(
        &self,
        row: &SsoAccess,
        payload: &SsoSessionPayload,
    ) -> Result<Vec<u8>> {
        let bytes = Zeroizing::new(
            serde_json::to_vec(payload).map_err(|_| Error::Sso("invalid access encoding"))?,
        );
        envelope::seal_bytes(self.keys.as_ref(), self.entropy.as_ref(), &aad(row), &bytes)
    }
    fn access_payload(&self, row: &SsoAccess) -> Result<SsoSessionPayload> {
        let bytes = envelope::open_bytes(self.keys.as_ref(), &aad(row), &row.ciphertext)?;
        let payload: SsoSessionPayload =
            serde_json::from_slice(&bytes).map_err(|_| Error::Sso("invalid protected access"))?;
        if payload.bound_uid.as_deref() != Some(row.uid.as_slice())
            || payload.issuer != row.issuer
            || payload.subject != row.subject
            || payload.expires_at_ms != row.expires_at_ms
        {
            return Err(Error::Sso("protected access binding mismatch"));
        }
        Ok(payload)
    }
    /// One refresh claim per account. Network I/O never owns a writer transaction.
    pub fn ensure_access(&self, uid: &[u8], credential: &[u8]) -> Result<()> {
        let uid = uid.to_vec();
        let credential = credential.to_vec();
        let (decision, refreshable) = self.writer.call_with_current_time(self.clock.clone(), {
            let uid = uid.clone();
            let credential = credential.clone();
            move |db, now| {
                if db.active_credential_owner(&uid, &credential)?.is_none() {
                    return Err(Error::Sso("credential was revoked"));
                }
                Ok((
                    db.sso_access_decision(&uid, now / 1000)?,
                    db.sso_refresh_eligible(&uid, now / 1000)?,
                ))
            }
        })?;
        if decision.permits_native() {
            return Ok(());
        }
        if !refreshable {
            return Err(Error::Sso("reauthentication required or provider blocked"));
        }
        let (row, sequence) = self.writer.call({
            let uid = uid.clone();
            let credential = credential.clone();
            move |db| {
                if db.active_credential_owner(&uid, &credential)?.is_none() {
                    return Err(Error::Sso("credential was revoked"));
                }
                let row = db
                    .sso_access(&uid)?
                    .ok_or(Error::Sso("reauthentication required"))?;
                let sequence = db
                    .user_authority(&uid)?
                    .ok_or(Error::Sso("account not found"))?
                    .chain_sequence;
                Ok((row, sequence))
            }
        })?;
        if row.interrupted || row.config_hash != self.config_hash || row.host != self.host {
            return Err(Error::Sso(
                "provider configuration changed; reauthentication required",
            ));
        }
        match row.state {
            SsoAccessState::Active => {}
            SsoAccessState::ProviderUnavailable => {
                return Err(Error::Sso(
                    "provider unavailable; explicit reauthentication required",
                ))
            }
            _ => {
                return Err(Error::Sso(
                    "reauthentication required or refresh in progress",
                ))
            }
        }
        if row.expires_at_ms > self.now_ms()? {
            return self
                .writer
                .call_with_current_time(self.clock.clone(), move |db, now| {
                    db.sso_require_access(&uid, now / 1000)?;
                    Ok(())
                });
        }
        let _permit = self
            .completions
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Sso("provider refresh busy"))?;
        let mut payload = self.access_payload(&row)?;
        let Some(refresh) = payload.refresh_token.as_deref() else {
            return Err(Error::Sso("reauthentication required"));
        };
        // A discovery failure has not submitted a rotating credential and is safe to retry.
        let provider = self.provider(true)?;
        let claimed = self.transition_access(
            &row,
            SsoAccessState::Refreshing,
            &payload,
            &credential,
            sequence,
        )?;
        let result = provider.refresh(
            &self.http,
            &self.client_config(),
            refresh,
            &payload.nonce,
            self.now_ms()?,
        );
        let (tokens, identity) = match result {
            Ok(v) => v,
            Err(error) => {
                let state = if matches!(
                    error,
                    foks_oidc::Error::ProviderUnavailable | foks_oidc::Error::DocumentLimit
                ) {
                    SsoAccessState::ProviderUnavailable
                } else {
                    SsoAccessState::ReauthenticationRequired
                };
                self.transition_access(&claimed, state, &payload, &credential, sequence)?;
                return Err(error.into());
            }
        };
        if tokens.access_expires_at_ms <= self.now_ms()?
            || identity
                .as_ref()
                .is_some_and(|i| i.issuer != row.issuer || i.subject != row.subject)
        {
            self.transition_access(
                &claimed,
                SsoAccessState::ReauthenticationRequired,
                &payload,
                &credential,
                sequence,
            )?;
            return Err(Error::Sso(
                "refreshed provider identity mismatch or expired",
            ));
        }
        payload.access_token.zeroize();
        payload.access_token = tokens.access.to_string();
        if !tokens.id.is_empty() {
            payload.id_token.zeroize();
            payload.id_token = tokens.id.to_string();
        }
        if let Some(refresh) = tokens.refresh {
            payload.refresh_token.zeroize();
            payload.refresh_token = Some(refresh.to_string());
        }
        payload.expires_at_ms = tokens.access_expires_at_ms;
        self.transition_access(
            &claimed,
            SsoAccessState::Active,
            &payload,
            &credential,
            sequence,
        )?;
        Ok(())
    }
    fn transition_access(
        &self,
        old: &SsoAccess,
        state: SsoAccessState,
        payload: &SsoSessionPayload,
        credential: &[u8],
        sequence: u64,
    ) -> Result<SsoAccess> {
        let mut next = old.clone();
        next.state = state;
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or(Error::Sso("access revision overflow"))?;
        next.expires_at_ms = payload.expires_at_ms;
        next.ciphertext = self.seal_access(&next, payload)?;
        let old = old.clone();
        let copy = next.clone();
        let credential = credential.to_vec();
        self.writer
            .call_with_current_time(self.clock.clone(), move |db, now| {
                db.sso_transition_access(&old, &copy, &credential, sequence, now / 1000)?;
                Ok(())
            })?;
        Ok(next)
    }
}
fn aad(row: &SsoAccess) -> Vec<u8> {
    let mut out = b"fennec-oidc-access-v1".to_vec();
    out.extend_from_slice(&row.host);
    out.extend_from_slice(&row.uid);
    out.extend_from_slice(&row.config_hash);
    out.extend_from_slice(&row.revision.to_be_bytes());
    out.extend_from_slice(&row.authorization_epoch.to_be_bytes());
    out.extend_from_slice(&row.authorization_generation.to_be_bytes());
    out.push(row.state as u8);
    out.extend_from_slice(&row.expires_at_ms.to_be_bytes());
    out.extend_from_slice(&(row.issuer.len() as u64).to_be_bytes());
    out.extend_from_slice(row.issuer.as_bytes());
    out.extend_from_slice(&(row.subject.len() as u64).to_be_bytes());
    out.extend_from_slice(row.subject.as_bytes());
    out
}
