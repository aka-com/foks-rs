//! One host's OIDC coordinator. HTTP and RPC adapters share durable session ownership.
mod access;
mod binding;
mod config;
mod envelope;
mod http;
pub mod operator;
mod payload;
use crate::{keys::HostKeyProvider, Entropy, Error, Result, WriterHandle};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
pub(crate) use binding::status;
pub use config::{OidcOperatorConfig, OidcRolloutMode};
use foks_oidc::{NetworkPolicy, Provider, ProviderClient, ProviderHttp};
use foks_proto::{InitOAuth2SessionArgument, OAuth2SessionId};
use foks_server_db::{SsoSession, SsoSessionState};
pub use http::OidcHttpServer;
use payload::SsoSessionPayload;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

pub struct SsoService {
    pub(crate) config: OidcOperatorConfig,
    config_hash: [u8; 32],
    host: [u8; 33],
    secret: Zeroizing<String>,
    writer: WriterHandle,
    keys: Arc<dyn HostKeyProvider>,
    clock: Arc<dyn foks_server_db::Clock>,
    entropy: Arc<dyn Entropy>,
    http: ProviderHttp,
    provider: Mutex<Option<(Instant, Arc<Provider>)>>,
    // Different pools prevent new sessions/start requests from starving callback completion.
    starts: Arc<tokio::sync::Semaphore>,
    completions: Arc<tokio::sync::Semaphore>,
    http_starts: Arc<tokio::sync::Semaphore>,
    http_completions: Arc<tokio::sync::Semaphore>,
    polling: Arc<tokio::sync::Semaphore>,
    changed: Arc<tokio::sync::Notify>,
}
impl SsoService {
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        config: OidcOperatorConfig,
        policy: NetworkPolicy,
        host: [u8; 33],
        writer: WriterHandle,
        keys: Arc<dyn HostKeyProvider>,
        clock: Arc<dyn foks_server_db::Clock>,
        entropy: Arc<dyn Entropy>,
    ) -> Result<Arc<Self>> {
        config.validate(policy)?;
        let secret = config.load_secret()?;
        let config_hash = config.fingerprint(&secret)?;
        let http = ProviderHttp::new(policy)?;
        let existing = writer.call(move |db| Ok(db.sso_policy(&host)?))?;
        if let Some(existing) = &existing {
            if existing.config_hash != config_hash
                || existing.issuer != config.issuer
                || existing.rollout_id != config.rollout_id
                || existing.mode != config.rollout_mode.into()
            {
                writer.call(move |db| {
                    db.sso_block_policy(
                        &host,
                        foks_server_db::SsoProviderBlockReason::ConfigurationMismatch,
                    )?;
                    Ok(())
                })?;
                return Err(Error::Sso(
                    "configuration disagrees with durable rollout policy",
                ));
            }
        }
        let key_result = keys.load_existing(crate::keys::KeyPurpose::Recovery);
        if key_result.is_err() && existing.is_some() {
            writer.call(move |db| {
                db.sso_block_policy(
                    &host,
                    foks_server_db::SsoProviderBlockReason::KeyUnavailable,
                )?;
                Ok(())
            })?;
        } else {
            key_result?;
        }
        let discovered = Provider::discover(&http, &config.discovery_uri, &config.client_id);
        let provider = match discovered {
            Ok(provider) if provider.metadata.issuer().as_str() == config.issuer => {
                Some((Instant::now(), Arc::new(provider)))
            }
            Ok(_) => {
                if existing.is_some() {
                    writer.call(move |db| {
                        db.sso_block_policy(
                            &host,
                            foks_server_db::SsoProviderBlockReason::ConfigurationMismatch,
                        )?;
                        Ok(())
                    })?;
                }
                return Err(Error::Config("OIDC discovery issuer mismatch"));
            }
            Err(_) if existing.is_some() => None, // Expiring linked access can still be checked locally.
            Err(error) => return Err(error.into()),
        };
        let rollout_id = config.rollout_id;
        let rollout_mode = config.rollout_mode.into();
        let issuer = config.issuer.clone();
        writer.call(move |db| {
            if existing.is_none() {
                db.sso_activate(&host, &rollout_id, &config_hash, &issuer, rollout_mode)?;
            }
            db.sso_abandon_exchanges(&host)?;
            db.sso_abandon_refreshes()?;
            Ok(())
        })?;
        Ok(Arc::new(Self {
            config,
            config_hash,
            host,
            secret,
            writer,
            keys,
            clock,
            entropy,
            http,
            provider: Mutex::new(provider),
            polling: Arc::new(tokio::sync::Semaphore::new(32)),
            changed: Arc::new(tokio::sync::Notify::new()),
            http_starts: Arc::new(tokio::sync::Semaphore::new(8)),
            http_completions: Arc::new(tokio::sync::Semaphore::new(8)),
            starts: Arc::new(tokio::sync::Semaphore::new(8)),
            completions: Arc::new(tokio::sync::Semaphore::new(8)),
        }))
    }
    pub(crate) fn configured_host(&self) -> &[u8; 33] {
        &self.host
    }
    pub fn public_config(&self) -> foks_proto::SsoConfig {
        self.config.public()
    }
    fn now_ms(&self) -> Result<u64> {
        Ok(self.clock.now_micros()? / 1000)
    }
    fn client_config(&self) -> ProviderClient<'_> {
        ProviderClient {
            client_id: &self.config.client_id,
            secret: &self.secret,
            redirect_uri: &self.config.redirect_uri,
            secret_in_body: self.config.client_secret_post,
        }
    }
    fn provider(&self, fresh: bool) -> Result<Arc<Provider>> {
        let cache = self
            .provider
            .lock()
            .map_err(|_| Error::Config("OIDC provider cache poisoned"))?;
        if !fresh {
            if let Some((at, provider)) = &*cache {
                if at.elapsed() < Duration::from_secs(300) {
                    return Ok(provider.clone());
                }
            }
        }
        drop(cache);
        let provider = Arc::new(Provider::discover(
            &self.http,
            &self.config.discovery_uri,
            &self.config.client_id,
        )?);
        if provider.metadata.issuer().as_str() != self.config.issuer {
            return Err(Error::Config("OIDC discovery issuer changed"));
        }
        *self
            .provider
            .lock()
            .map_err(|_| Error::Config("OIDC provider cache poisoned"))? =
            Some((Instant::now(), provider.clone()));
        Ok(provider)
    }
    /// Per-source limiting is derived from the transport peer, never the unauthenticated UID field.
    pub fn init(&self, arg: &InitOAuth2SessionArgument, source_identity: &[u8]) -> Result<String> {
        let _permit = self
            .starts
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Sso("session admission busy"))?;
        let verifier = arg.pkce_verifier.expose();
        // Pinned Go emits 27 characters. Preserve that wire input; the IdP decides whether to
        // reject it. Our own client emits the RFC-required 43, and production IdPs stay strict.
        if arg.id.0[0] != 51
            || arg.id.0[1..].iter().all(|b| *b == 0)
            || !(verifier.len() == 27 || (43..=128).contains(&verifier.len()))
            || !verifier
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
            || arg.nonce.expose().is_empty()
            || arg.nonce.expose().len() > 256
            || source_identity.is_empty()
            || source_identity.len() > 256
        {
            return Err(Error::Sso("invalid session request"));
        }
        let now = self.now_ms()?;
        let uid = arg
            .uid
            .as_ref()
            .map(|u| {
                u.as_bytes()
                    .try_into()
                    .map_err(|_| Error::Sso("invalid UID"))
            })
            .transpose()?;
        let host = self.host;
        let epoch = self.writer.call(move |db| {
            Ok(db
                .sso_policy(&host)?
                .ok_or(Error::Sso("OIDC policy missing"))?
                .authorization_epoch)
        })?;
        let mut row = SsoSession {
            host: self.host,
            authorization_epoch: epoch,
            interrupted: false,
            session_hash: session_hash(&arg.id),
            config_hash: self.config_hash,
            source_hash: foks_crypto::prefixed_hash(0x13f6_7bc0_b832_a770, source_identity),
            uid,
            state: SsoSessionState::Waiting,
            revision: 1,
            expires_at_ms: now
                .checked_add(foks_oidc::SESSION_LIFETIME_MS)
                .ok_or(Error::Sso("clock overflow"))?,
            ciphertext: Vec::new(),
        };
        let mut payload = SsoSessionPayload::default();
        payload.id = arg.id.0;
        payload.verifier = verifier.into();
        payload.nonce = arg.nonce.expose().into();
        row.ciphertext = self.seal(&row, &payload)?;
        let clock = self.clock.clone();
        self.writer.call_with_current_time(clock, move |db, now| {
            db.sso_insert_session(&row, now / 1000)?;
            Ok(())
        })?;
        let mut url = self.http.policy().check_url(&self.config.redirect_uri)?;
        url.set_path("/oauth2/start");
        url.set_query(None);
        url.query_pairs_mut()
            .append_pair("state", &URL_SAFE_NO_PAD.encode(arg.id.0));
        Ok(url.into())
    }
    fn row(&self, id: &OAuth2SessionId) -> Result<SsoSession> {
        let host = self.host;
        let hash = session_hash(id);
        let config_hash = self.config_hash;
        let row = self
            .writer
            .call(move |db| {
                db.sso_require_policy(&host, &config_hash)?;
                let row = db.sso_session(&host, &hash)?;
                if let Some(row) = &row {
                    db.sso_require_policy_epoch(&host, &config_hash, row.authorization_epoch)?;
                }
                Ok(row)
            })?
            .ok_or(Error::Sso("unknown session"))?;
        if row.interrupted
            || row.config_hash != self.config_hash
            || row.expires_at_ms <= self.now_ms()?
        {
            return Err(Error::Sso("expired or replaced session"));
        }
        Ok(row)
    }
    fn session_payload(&self, row: &SsoSession) -> Result<SsoSessionPayload> {
        let bytes = envelope::open(self.keys.as_ref(), row)?;
        let value: SsoSessionPayload =
            serde_json::from_slice(&bytes).map_err(|_| Error::Sso("invalid protected session"))?;
        if session_hash(&OAuth2SessionId(value.id)) != row.session_hash {
            return Err(Error::Sso("protected session mismatch"));
        }
        Ok(value)
    }
    fn seal(&self, row: &SsoSession, payload: &SsoSessionPayload) -> Result<Vec<u8>> {
        let bytes = Zeroizing::new(
            serde_json::to_vec(payload).map_err(|_| Error::Sso("invalid session encoding"))?,
        );
        envelope::seal(self.keys.as_ref(), self.entropy.as_ref(), row, &bytes)
    }
    fn transition(
        &self,
        old: &SsoSession,
        state: SsoSessionState,
        payload: &SsoSessionPayload,
    ) -> Result<SsoSession> {
        let mut next = old.clone();
        next.state = state;
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or(Error::Sso("session revision overflow"))?;
        next.ciphertext = self.seal(&next, payload)?;
        let old = old.clone();
        let bytes = next.ciphertext.clone();
        self.writer
            .call_with_current_time(self.clock.clone(), move |db, now| {
                db.sso_transition(&old, state, &bytes, now / 1000)?;
                Ok(())
            })?;
        self.changed.notify_waiters();
        Ok(next)
    }
    pub fn browser_start(&self, state: &str) -> Result<String> {
        let _permit = self
            .starts
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Sso("authorization start busy"))?;
        let row = self.row(&parse_state(state)?)?;
        if row.state != SsoSessionState::Waiting {
            return Err(Error::Sso("session is no longer waiting"));
        }
        let payload = self.session_payload(&row)?;
        self.provider(false)?
            .authorization_url(
                &self.client_config(),
                state,
                &payload.nonce,
                &payload.verifier,
            )
            .map_err(Into::into)
    }
    pub fn callback(&self, state: &str, code: Option<&str>, denied: bool) -> Result<()> {
        let _permit = self
            .completions
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Sso("callback completion busy"))?;
        let row = self.row(&parse_state(state)?)?;
        if row.state != SsoSessionState::Waiting {
            return Err(Error::Sso("callback was already claimed"));
        }
        let mut payload = self.session_payload(&row)?;
        if denied {
            self.transition(&row, SsoSessionState::Denied, &payload)?;
            return Ok(());
        }
        let code = code
            .filter(|c| !c.is_empty() && c.len() <= 4096)
            .ok_or(Error::Sso("missing authorization code"))?;
        // Discovery failure is safely retryable before the one-use code is claimed.
        let provider = self.provider(true)?;
        let claimed = self.transition(&row, SsoSessionState::Exchanging, &payload)?;
        let outcome = provider.exchange(
            &self.http,
            &self.client_config(),
            code,
            &payload.verifier,
            &payload.nonce,
            self.now_ms()?,
        );
        let (tokens, identity) = match outcome {
            Ok(v) => v,
            Err(error) => {
                let state = if matches!(
                    error,
                    foks_oidc::Error::GrantRejected | foks_oidc::Error::Token
                ) {
                    SsoSessionState::Rejected
                } else {
                    SsoSessionState::ExchangeUnknown
                };
                self.transition(&claimed, state, &payload)?;
                return Err(error.into());
            }
        };
        if tokens.access_expires_at_ms <= self.now_ms()? {
            self.transition(&claimed, SsoSessionState::Rejected, &payload)?;
            return Err(Error::Sso("provider token expired during exchange"));
        }
        payload.access_token = tokens.access.to_string();
        payload.id_token = tokens.id.to_string();
        payload.refresh_token = tokens.refresh.as_ref().map(|v| v.to_string());
        payload.email = identity.email;
        payload.issuer = identity.issuer;
        payload.subject = identity.subject;
        payload.username = identity.username;
        payload.expires_at_ms = tokens.access_expires_at_ms;
        self.transition(&claimed, SsoSessionState::Ready, &payload)?;
        Ok(())
    }
    pub fn session_state(&self, id: &OAuth2SessionId) -> Result<SsoSessionState> {
        Ok(self.row(id)?.state)
    }
}
fn session_hash(id: &OAuth2SessionId) -> [u8; 32] {
    foks_crypto::prefixed_hash(0x2ab7_ce4f_1a07_aaf3, &id.0)
}
fn parse_state(state: &str) -> Result<OAuth2SessionId> {
    if state.len() != 23 {
        return Err(Error::Sso("invalid browser state"));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(state)
        .map_err(|_| Error::Sso("invalid browser state"))?;
    let id = OAuth2SessionId(
        bytes
            .try_into()
            .map_err(|_| Error::Sso("invalid browser state"))?,
    );
    if id.0[0] != 51 {
        return Err(Error::Sso("invalid browser state type"));
    }
    Ok(id)
}

#[cfg(test)]
mod tests;
