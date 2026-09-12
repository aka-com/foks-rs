use super::*;
pub enum SsoSigningKey<'a> {
    Software(&'a foks_proto::SecretSeed),
    Yubi(&'a dyn foks_crypto::YubiDevice),
}
/// Constructed only after provider validation and scoped signature checks.
pub struct SsoSignupAuthorization {
    pub(crate) flow_id: [u8; 16],
    pub(crate) args: RegSsoArgs,
    pub(crate) reservation: foks_proto::UsernameReservation,
    pub(crate) username: String,
    email: String,
    pub(crate) uid: EntityId,
    pub(crate) device: EntityId,
    expires_at_ms: u64,
}
impl SsoSignupAuthorization {
    pub fn username(&self) -> &str {
        &self.username
    }
    pub fn email(&self) -> &str {
        &self.email
    }
    pub(crate) fn validate(&self, client: &FoksClient, host: &PinnedHost) -> Result<()> {
        let flow = HardStateStore::open(&host.database_path)?
            .sso_flow(&self.flow_id)?
            .ok_or(Error::Sso("unknown signup flow"))?;
        if flow.for_login
            || flow.state != SsoFlowState::Ready
            || flow.uid != self.uid.as_bytes()
            || flow.device != self.device.as_bytes()
            || flow.host != host.host_id().as_bytes()
            || self.expires_at_ms <= crate::now_milliseconds()?
        {
            return Err(Error::Sso("signup authorization is no longer ready"));
        }
        client.require_sso_current(host, &flow)
    }
}
impl FoksClient {
    pub fn authorize_sso_signup(
        &self,
        host: &PinnedHost,
        id: [u8; 16],
        key: SsoSigningKey<'_>,
        http: &ProviderHttp,
        store: &mut impl ProtectedMutationStore,
    ) -> Result<SsoSignupAuthorization> {
        let (flow, m) = load(host, &id, store)?;
        if flow.for_login || flow.state != SsoFlowState::Ready {
            return Err(Error::Sso("signup flow is not ready"));
        }
        let device = match key {
            SsoSigningKey::Software(seed) => foks_crypto::derive_device_public(seed)?.id,
            SsoSigningKey::Yubi(parent) => parent.entity_id().clone(),
        };
        if device.as_bytes() != flow.device {
            return Err(Error::Sso("signup device changed"));
        }
        let result = OAuth2PollResult::decode(&get(store, &id, 2)?)?;
        let identity = self.validate_sso_result(host, &flow, &m, &result, http)?;
        let uid = m.binding.uid.clone();
        let payload = OAuth2IdTokenBindingPayload {
            id_token: result.tokens.id_token.clone(),
            binding: m.binding,
        };
        let args = match store.get(&material_key(&id, 3)) {
            Ok(bytes) => {
                let args = RegSsoArgs::decode(&bytes)?;
                let RegSsoArgs::Oauth2 {
                    id: old_id,
                    binding,
                } = &args
                else {
                    return Err(Error::Sso("missing signup binding"));
                };
                if old_id != &m.init.id
                    || binding.key != device
                    || foks_crypto::verify_oauth2_binding(binding)? != payload
                {
                    return Err(Error::Sso("stored signup binding differs"));
                }
                args
            }
            Err(crate::ProtectedStoreError::Missing) => {
                let binding = match key {
                    SsoSigningKey::Software(seed) => {
                        foks_crypto::sign_oauth2_binding(seed, &payload)?
                    }
                    SsoSigningKey::Yubi(parent) => {
                        foks_crypto::sign_yubi_oauth2_binding(parent, &payload)?
                    }
                };
                let args = RegSsoArgs::Oauth2 {
                    id: m.init.id,
                    binding,
                };
                put(store, &id, 3, &Zeroizing::new(args.encoded()?))?;
                args
            }
            Err(_) => return Err(Error::Sso("protected binding unavailable")),
        };
        Ok(SsoSignupAuthorization {
            flow_id: id,
            args,
            reservation: result.reservation,
            username: result.tokens.username,
            email: identity.email.unwrap_or_default(),
            uid,
            device,
            expires_at_ms: flow.expires_at_ms.min(identity.expires_at_ms),
        })
    }
}

impl FoksClient {
    /// Resume the original signup journal; never create a replacement signup for a lost reply.
    pub fn resume_sso_software_account(
        &self,
        host: &PinnedHost,
        id: [u8; 16],
        soft: &std::path::Path,
        store: &mut impl ProtectedMutationStore,
    ) -> Result<crate::CreatedSoftwareAccount> {
        let flow = public_flow(host, &id)?;
        if flow.for_login || !matches!(flow.state, SsoFlowState::Binding | SsoFlowState::Complete) {
            return Err(Error::Sso("flow has no resumable signup"));
        }
        let operation = flow
            .final_operation
            .ok_or(Error::Sso("flow is missing its signup journal"))?;
        let created = self.resume_software_account(host, operation, soft, store)?;
        self.sso_progress(host, id, store)?;
        Ok(created)
    }
    pub fn resume_sso_yubi_account<'a>(
        &self,
        host: &PinnedHost,
        id: [u8; 16],
        parent: &'a dyn foks_crypto::YubiDevice,
        soft: &std::path::Path,
        store: &mut impl ProtectedMutationStore,
    ) -> Result<crate::CreatedYubiAccount<'a>> {
        let flow = public_flow(host, &id)?;
        if flow.for_login
            || flow.device != parent.entity_id().as_bytes()
            || !matches!(flow.state, SsoFlowState::Binding | SsoFlowState::Complete)
        {
            return Err(Error::Sso("flow has no matching resumable hardware signup"));
        }
        let operation = flow
            .final_operation
            .ok_or(Error::Sso("flow is missing its signup journal"))?;
        let created = self.resume_yubi_account(host, parent, operation, soft, store)?;
        self.sso_progress(host, id, store)?;
        Ok(created)
    }
}
