//! Durable, host-bound OAuth browser flows. Public progress never includes provider tokens.
use crate::{
    Error, FederationCredential, FoksClient, HardStateStore, PinnedHost, ProtectedMutationStore,
    Result,
};
use foks_client_db::{SsoFlow, SsoFlowState};
use foks_oidc::{Provider, ProviderHttp};
use foks_proto::{
    EntityId, InitOAuth2SessionArgument, OAuth2Binding, OAuth2IdTokenBindingPayload,
    OAuth2PollResult, OAuth2Secret, PollOAuth2SessionArgument, RegSsoArgs, SsoConfig,
    SsoLoginArgument, SsoProtocol, TreeRoot,
};
use foks_snowpack::{decode, encode, Value};
use zeroize::Zeroizing;

const MATERIAL_HASH: u64 = 0xef13_321b_a6e8_27d0;
const CONFIG_HASH: u64 = 0xc872_c073_8ea5_11a4;
#[derive(Clone)]
pub struct SsoProgress {
    pub operation_id: [u8; 16],
    pub state: SsoFlowState,
    pub browser_url: Option<String>,
    pub expires_at_ms: u64,
}
impl std::fmt::Debug for SsoProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SsoProgress")
            .field("operation_id", &self.operation_id)
            .field("state", &self.state)
            .field("expires_at_ms", &self.expires_at_ms)
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Debug)]
pub struct SsoIntent {
    pub uid: EntityId,
    pub device: EntityId,
    pub purpose: foks_proto::SsoPurpose,
}
mod identity;
mod material;
pub(crate) use material::validate_inventory_stage;
use material::*;
mod signup;
pub use signup::{SsoSigningKey, SsoSignupAuthorization};

impl FoksClient {
    /// A signup UID is derived from its already-protected PUK seed; a login UID/device
    /// comes from the checked local credential. Neither is selected by the browser.
    pub fn begin_sso(
        &self,
        host: &PinnedHost,
        intent: SsoIntent,
        http: &ProviderHttp,
        store: &mut impl ProtectedMutationStore,
    ) -> Result<SsoProgress> {
        let SsoIntent {
            uid,
            device,
            purpose,
        } = intent;
        uid.clone().require_type(foks_proto::ENTITY_USER)?;
        if !matches!(
            device.entity_type(),
            foks_proto::ENTITY_DEVICE
                | foks_proto::ENTITY_YUBI
                | foks_proto::ENTITY_BACKUP_KEY
                | foks_proto::ENTITY_BOT_TOKEN_KEY
        ) {
            return Err(Error::Sso("unsupported SSO signer"));
        }
        let config = self
            .registration_server_config(host)?
            .sso
            .ok_or(Error::Sso("host does not require SSO"))?
            .public();
        if config.active != SsoProtocol::Oauth2 {
            return Err(Error::Sso("host SSO protocol is unsupported"));
        }
        let oauth = config
            .oauth2
            .as_ref()
            .ok_or(Error::Sso("missing OAuth configuration"))?;
        http.policy().check_url(&oauth.redirect_uri)?;
        let provider = Provider::discover(http, &oauth.config_uri, &oauth.client_id)?;
        let (_, root) = self.advance_merkle_root(host)?;
        let now = crate::now_milliseconds()?;
        let binding = OAuth2Binding {
            uid: uid.clone(),
            host: host.host_id().clone(),
            root: TreeRoot {
                epoch: root.snapshot().epoch(),
                hash: root.snapshot().root_hash(),
            },
            random: crate::random_bytes()?,
        };
        let init = InitOAuth2SessionArgument {
            id: foks_crypto::oauth2_session_id()?,
            pkce_verifier: foks_crypto::oauth2_pkce_verifier()?,
            nonce: OAuth2Secret::new(foks_crypto::oauth2_binding_nonce(&binding)?),
            uid: purpose.is_existing().then_some(uid.clone()),
        };
        let expires_at_ms = now
            .checked_add(foks_oidc::SESSION_LIFETIME_MS)
            .ok_or(Error::Sso("session expiration overflow"))?;
        let material = Material {
            purpose,
            device: device.clone(),
            expires_at_ms,
            config: config.clone(),
            binding,
            init,
            issuer: provider.metadata.issuer().as_str().to_owned(),
        };
        let encoded = material.encoded()?;
        let id = crate::random_bytes()?;
        let flow = SsoFlow {
            id,
            host: host.host_id().as_bytes().to_vec(),
            uid: uid.as_bytes().to_vec(),
            device: device.as_bytes().to_vec(),
            purpose,
            state: SsoFlowState::Prepared,
            material_hash: foks_crypto::prefixed_hash(MATERIAL_HASH, &encoded),
            config_hash: config_hash(&config)?,
            expires_at_ms,
            final_operation: None,
            commitment: None,
        };
        HardStateStore::open(&host.database_path)?.sso_record_with_material::<Error>(
            &flow,
            now,
            || put(store, &id, 0, &encoded),
        )?;
        // Claim before delivery. If the init reply is lost, resume never sends init again.
        HardStateStore::open(&host.database_path)?.sso_transition(
            &id,
            SsoFlowState::Prepared,
            SsoFlowState::AwaitingBrowser,
            None,
        )?;
        let reply = self.call_after_vhost_selection(
            host,
            &host.registration,
            &foks_rpc::encode_registration_select_vhost_request(host.host_id())?,
            &foks_rpc::encode_init_oauth2_session_request_at(&material.init, 1)?,
        )?;
        let Value::Text(url) = decode(&reply)? else {
            return Err(Error::Sso("invalid browser start result"));
        };
        let url = String::from_utf8(url).map_err(|_| Error::Sso("invalid browser URL"))?;
        let url = foks_oidc::check_browser_start(http.policy(), &url, &oauth.redirect_uri)?;
        put(store, &id, 1, url.as_bytes())?;
        self.sso_progress(host, id, store)
    }
    pub fn sso_progress(
        &self,
        host: &PinnedHost,
        id: [u8; 16],
        store: &mut impl ProtectedMutationStore,
    ) -> Result<SsoProgress> {
        let mut flow = public_flow(host, &id)?;
        if flow.expires_at_ms <= crate::now_milliseconds()?
            && matches!(
                flow.state,
                SsoFlowState::Prepared | SsoFlowState::AwaitingBrowser | SsoFlowState::Ready
            )
        {
            HardStateStore::open(&host.database_path)?.sso_transition(
                &id,
                flow.state,
                SsoFlowState::Expired,
                None,
            )?;
            flow.state = SsoFlowState::Expired;
        }
        if flow.state == SsoFlowState::Binding {
            if let Some(op) = flow.final_operation {
                let mutation = HardStateStore::open(&host.database_path)?
                    .mutation(&op)?
                    .ok_or(Error::Sso("signup journal is missing"))?;
                if matches!(
                    mutation.state,
                    foks_client_db::MutationState::RemoteVerified
                        | foks_client_db::MutationState::Finalized
                ) {
                    HardStateStore::open(&host.database_path)?.sso_transition(
                        &id,
                        SsoFlowState::Binding,
                        SsoFlowState::Complete,
                        None,
                    )?;
                    flow.state = SsoFlowState::Complete;
                }
            }
        }
        if matches!(
            flow.state,
            SsoFlowState::Complete
                | SsoFlowState::Denied
                | SsoFlowState::Cancelled
                | SsoFlowState::Expired
                | SsoFlowState::Rejected
        ) {
            erase_flow_material(store, &id)?;
        }
        // Expired uncertain login attempts retain an honest receipt, not reusable tokens.
        // Signup's final mutation owns its separate recovery material.
        if flow.expires_at_ms <= crate::now_milliseconds()?
            && flow.final_operation.is_none()
            && matches!(flow.state, SsoFlowState::Binding | SsoFlowState::Unknown)
        {
            if flow.state == SsoFlowState::Binding {
                HardStateStore::open(&host.database_path)?.sso_transition(
                    &id,
                    flow.state,
                    SsoFlowState::Unknown,
                    None,
                )?;
                flow.state = SsoFlowState::Unknown;
            }
            erase_flow_material(store, &id)?;
        }
        let browser_url = if flow.state == SsoFlowState::AwaitingBrowser {
            match store.get(&material_key(&id, 1)) {
                Ok(url) => Some(
                    String::from_utf8(url.to_vec())
                        .map_err(|_| Error::Sso("invalid stored browser URL"))?,
                ),
                Err(crate::ProtectedStoreError::Missing) => None,
                Err(_) => return Err(Error::Sso("protected URL unavailable")),
            }
        } else {
            None
        };
        Ok(SsoProgress {
            operation_id: id,
            state: flow.state,
            browser_url,
            expires_at_ms: flow.expires_at_ms,
        })
    }
    pub fn cancel_sso(
        &self,
        host: &PinnedHost,
        id: [u8; 16],
        store: &mut impl ProtectedMutationStore,
    ) -> Result<SsoProgress> {
        let progress = self.sso_progress(host, id, store)?;
        if progress.state == SsoFlowState::Complete {
            return Err(Error::Sso("completed authentication cannot be cancelled"));
        }
        if matches!(
            progress.state,
            SsoFlowState::Denied
                | SsoFlowState::Cancelled
                | SsoFlowState::Expired
                | SsoFlowState::Rejected
        ) {
            return Ok(progress);
        }
        let (flow, _) = load(host, &id, store)?;
        HardStateStore::open(&host.database_path)?.sso_transition(
            &id,
            flow.state,
            SsoFlowState::Cancelled,
            None,
        )?;
        self.sso_progress(host, id, store)
    }
    fn require_sso_current(&self, host: &PinnedHost, flow: &SsoFlow) -> Result<()> {
        if flow.expires_at_ms <= crate::now_milliseconds()? {
            return Err(Error::Sso("session expired"));
        }
        let current = self
            .registration_server_config(host)?
            .sso
            .ok_or(Error::Sso("host SSO policy changed"))?;
        if config_hash(&current)? != flow.config_hash {
            return Err(Error::Sso("host SSO configuration changed"));
        }
        Ok(())
    }
    pub fn poll_sso(
        &self,
        host: &PinnedHost,
        id: [u8; 16],
        wait_ms: u64,
        http: &ProviderHttp,
        store: &mut impl ProtectedMutationStore,
    ) -> Result<SsoProgress> {
        let flow = public_flow(host, &id)?;
        if flow.state != SsoFlowState::AwaitingBrowser {
            return self.sso_progress(host, id, store);
        }
        let (_, m) = load(host, &id, store)?;
        self.require_sso_current(host, &flow)?;
        if wait_ms > foks_oidc::MAX_POLL_MS {
            return Err(Error::Sso("poll wait exceeds limit"));
        }
        let request = PollOAuth2SessionArgument {
            id: m.init.id.clone(),
            wait_duration_ms: wait_ms,
            for_login: flow.purpose.is_existing(),
        };
        let client = self.isolated_with_timeout(std::time::Duration::from_millis(wait_ms + 5000));
        let response = match client.call_after_vhost_selection(
            host,
            &host.registration,
            &foks_rpc::encode_registration_select_vhost_request(host.host_id())?,
            &foks_rpc::encode_poll_oauth2_session_request_at(&request, 1)?,
        ) {
            Ok(bytes) => Zeroizing::new(bytes),
            Err(Error::Rpc(foks_rpc::Error::RemoteStatus { code: 1009, .. })) => {
                return self.sso_progress(host, id, store)
            }
            Err(Error::Rpc(foks_rpc::Error::RemoteStatus {
                code: 1067,
                ref detail,
            })) if detail
                .detail()
                .is_some_and(|d| d == "authorization denied" || d.contains("access_denied")) =>
            {
                HardStateStore::open(&host.database_path)?.sso_transition(
                    &id,
                    SsoFlowState::AwaitingBrowser,
                    SsoFlowState::Denied,
                    None,
                )?;
                return self.sso_progress(host, id, store);
            }
            Err(error) => return Err(error),
        };
        if response.len() > foks_oidc::MAX_PROVIDER_BYTES {
            return Err(Error::Sso("OAuth poll result exceeds limit"));
        }
        let result = OAuth2PollResult::decode(&response)?;
        self.validate_sso_result(host, &flow, &m, &result, http)?;
        put(store, &id, 2, &response)?;
        HardStateStore::open(&host.database_path)?.sso_transition(
            &id,
            SsoFlowState::AwaitingBrowser,
            SsoFlowState::Ready,
            None,
        )?;
        self.sso_progress(host, id, store)
    }
    fn validate_sso_result(
        &self,
        host: &PinnedHost,
        flow: &SsoFlow,
        m: &Material,
        result: &OAuth2PollResult,
        http: &ProviderHttp,
    ) -> Result<foks_oidc::VerifiedIdentity> {
        self.require_sso_current(host, flow)?;
        let cfg = m
            .config
            .oauth2
            .as_ref()
            .ok_or(Error::Sso("missing OAuth configuration"))?;
        let provider = Provider::discover(http, &cfg.config_uri, &cfg.client_id)?;
        if provider.metadata.issuer().as_str() != m.issuer {
            return Err(Error::Sso("provider issuer changed during session"));
        }
        let identity = provider.validator.validate(
            result.tokens.id_token.expose(),
            m.init.nonce.expose(),
            crate::now_milliseconds()?,
        )?;
        if !flow.purpose.is_existing()
            && (identity.username != result.tokens.username
                || result.reservation.sequence == 0
                || result.reservation.expires_at <= crate::now_milliseconds()?)
        {
            return Err(Error::Sso(
                "signup reservation does not match validated claims",
            ));
        }
        Ok(identity)
    }
    /// Existing device signature is mandatory even after successful browser consent.
    pub fn finish_sso_login(
        &self,
        host: &PinnedHost,
        id: [u8; 16],
        credential: FederationCredential<'_, '_>,
        http: &ProviderHttp,
        store: &mut impl ProtectedMutationStore,
    ) -> Result<SsoProgress> {
        let flow = public_flow(host, &id)?;
        if flow.uid != credential.uid().as_bytes()
            || flow.device != credential.device_id()?.as_bytes()
            || !flow.purpose.is_existing()
        {
            return Err(Error::Sso("login signer or purpose mismatch"));
        }
        if matches!(flow.state, SsoFlowState::Binding | SsoFlowState::Unknown) {
            let key = match credential {
                FederationCredential::Software(c) => SsoSigningKey::Software(&c.seed),
                FederationCredential::Yubi(c) => SsoSigningKey::Yubi(c.parent),
            };
            return self.reconcile_sso_binding(host, id, key, store);
        }
        if flow.state != SsoFlowState::Ready {
            return self.sso_progress(host, id, store);
        }
        let key = match credential {
            FederationCredential::Software(c) => SsoSigningKey::Software(&c.seed),
            FederationCredential::Yubi(c) => SsoSigningKey::Yubi(c.parent),
        };
        match self.identity_status(host, credential.uid(), key, None) {
            Ok(status) => {
                let matches = match flow.purpose {
                    foks_proto::SsoPurpose::LinkExisting => matches!(
                        status.account_state,
                        foks_proto::SsoAccountState::MigrationEligible
                            | foks_proto::SsoAccountState::LockedOut
                    ),
                    foks_proto::SsoPurpose::Reauthenticate => {
                        status.account_state == foks_proto::SsoAccountState::Linked
                    }
                    foks_proto::SsoPurpose::Signup => false,
                };
                if !matches || status.provider_fence != 0 {
                    return Err(Error::Sso(
                        "account linkage state differs from requested authentication purpose",
                    ));
                }
            }
            // Existing Go reauthentication keeps its original wire path. First linkage
            // requires the foks-rs extension and cannot fall back to implicit linking.
            Err(Error::Rpc(foks_rpc::Error::RemoteStatus {
                code: 211 | 1020, ..
            })) if flow.purpose == foks_proto::SsoPurpose::Reauthenticate => {}
            Err(error) => return Err(error),
        }
        let (_, m) = load(host, &id, store)?;
        let result = OAuth2PollResult::decode(&get(store, &id, 2)?)?;
        self.validate_sso_result(host, &flow, &m, &result, http)?;
        let payload = OAuth2IdTokenBindingPayload {
            id_token: result.tokens.id_token.clone(),
            binding: m.binding,
        };
        let request = match store.get(&material_key(&id, 3)) {
            Ok(request) => {
                let mut input = std::io::Cursor::new(request.as_slice());
                let call = foks_rpc::read_call(&mut input, foks_rpc::DEFAULT_MAX_FRAME_LENGTH)?;
                if input.position() != request.len() as u64
                    || call.protocol_id() != foks_rpc::REG_PROTOCOL_ID
                    || call.method_position() != foks_rpc::REG_SSO_LOGIN_METHOD_POSITION
                {
                    return Err(Error::Sso("stored login frame differs"));
                }
                let args = SsoLoginArgument::decode(call.argument())?;
                let RegSsoArgs::Oauth2 {
                    id: old_id,
                    binding,
                } = &args.args
                else {
                    return Err(Error::Sso("missing stored login binding"));
                };
                if args.uid != *credential.uid()
                    || old_id != &m.init.id
                    || binding.key.as_bytes() != flow.device
                    || foks_crypto::verify_oauth2_binding(binding)? != payload
                {
                    return Err(Error::Sso("stored login binding differs"));
                }
                request
            }
            Err(crate::ProtectedStoreError::Missing) => {
                let signed = match credential {
                    FederationCredential::Software(c) => {
                        foks_crypto::sign_oauth2_binding(&c.seed, &payload)?
                    }
                    FederationCredential::Yubi(c) => {
                        foks_crypto::sign_yubi_oauth2_binding(c.parent, &payload)?
                    }
                };
                let args = SsoLoginArgument {
                    uid: credential.uid().clone(),
                    args: RegSsoArgs::Oauth2 {
                        id: m.init.id,
                        binding: signed,
                    },
                };
                let request = Zeroizing::new(foks_rpc::encode_sso_login_request_at(&args, 1)?);
                put(store, &id, 3, &request)?;
                request
            }
            Err(_) => return Err(Error::Sso("protected login binding unavailable")),
        };
        let mut input = std::io::Cursor::new(request.as_slice());
        let call = foks_rpc::read_call(&mut input, foks_rpc::DEFAULT_MAX_FRAME_LENGTH)?;
        let args = SsoLoginArgument::decode(call.argument())?;
        let commitment = foks_crypto::prefixed_hash(0x68b3_c398_ea5f_d71e, &args.args.encoded()?);
        HardStateStore::open(&host.database_path)?.sso_set_commitment(&id, &commitment)?;
        HardStateStore::open(&host.database_path)?.sso_transition(
            &id,
            SsoFlowState::Ready,
            SsoFlowState::Binding,
            None,
        )?;
        let outcome = self.call_void_after_vhost_selection(
            host,
            &host.registration,
            &foks_rpc::encode_registration_select_vhost_request(host.host_id())?,
            &request,
        );
        let next = if outcome.is_ok() {
            SsoFlowState::Complete
        } else {
            SsoFlowState::Unknown
        };
        HardStateStore::open(&host.database_path)?.sso_transition(
            &id,
            SsoFlowState::Binding,
            next,
            None,
        )?;
        outcome?;
        self.sso_progress(host, id, store)
    }
}
