//! Read-only outcome reconciliation over server-authenticated registration TLS.
use super::*;
use foks_proto::{IdentityChallenge, IdentityClaim, IdentityProof, IdentityStatus};
impl FoksClient {
    fn identity_call(&self, host: &PinnedHost, position: u64, argument: &[u8]) -> Result<Vec<u8>> {
        self.call_after_vhost_selection(
            host,
            &host.registration,
            &foks_rpc::encode_registration_select_vhost_request(host.host_id())?,
            &foks_rpc::encode_call(foks_rpc::IDENTITY_PROTOCOL_ID, position, argument, 1)?,
        )
    }
    pub fn identity_status(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        key: SsoSigningKey<'_>,
        authorization_binding_commitment: Option<[u8; 32]>,
    ) -> Result<IdentityStatus> {
        let capability = self.identity_call(
            host,
            foks_rpc::IDENTITY_CAPABILITIES_POSITION,
            &encode(&Value::Null)?,
        )?;
        if decode(&capability)? != Value::Unsigned(foks_proto::IDENTITY_EXTENSION_VERSION) {
            return Err(Error::Sso("host identity extension is unsupported"));
        }
        let signer = match key {
            SsoSigningKey::Software(seed) => foks_crypto::derive_device_public(seed)?.id,
            SsoSigningKey::Yubi(parent) => parent.entity_id().clone(),
        };
        let claim = IdentityClaim {
            host: host.host_id().clone(),
            uid: uid.clone(),
            signer,
            authorization_binding_commitment,
        };
        let bytes = self.identity_call(
            host,
            foks_rpc::IDENTITY_CHALLENGE_POSITION,
            &claim.encoded()?,
        )?;
        let challenge = IdentityChallenge::decode(&bytes)?;
        if challenge.claim != claim
            || challenge.expires_at_ms.checked_sub(challenge.issued_at_ms) != Some(60_000)
        {
            return Err(Error::Sso("identity challenge binding or lifetime differs"));
        }
        let expected = challenge.challenge;
        let proof: IdentityProof = match key {
            SsoSigningKey::Software(seed) => foks_crypto::sign_identity_proof(seed, challenge)?,
            SsoSigningKey::Yubi(parent) => {
                foks_crypto::sign_yubi_identity_proof(parent, challenge)?
            }
        };
        let bytes =
            self.identity_call(host, foks_rpc::IDENTITY_PROVE_POSITION, &proof.encoded()?)?;
        let status = IdentityStatus::decode(&bytes)?;
        if status.challenge != expected
            || status
                .committed_authorization_binding
                .is_some_and(|r| Some(r) != authorization_binding_commitment)
        {
            return Err(Error::Sso("identity response does not match proof"));
        }
        Ok(status)
    }
    pub fn reconcile_sso_binding(
        &self,
        host: &PinnedHost,
        id: [u8; 16],
        key: SsoSigningKey<'_>,
        store: &mut impl ProtectedMutationStore,
    ) -> Result<SsoProgress> {
        let flow = public_flow(host, &id)?;
        if !matches!(flow.state, SsoFlowState::Binding | SsoFlowState::Unknown) {
            return self.sso_progress(host, id, store);
        }
        let commitment = flow
            .commitment
            .ok_or(Error::Sso("binding outcome evidence unavailable"))?;
        let uid = EntityId::from_bytes(flow.uid.clone())?;
        let signer = match key {
            SsoSigningKey::Software(seed) => foks_crypto::derive_device_public(seed)?.id,
            SsoSigningKey::Yubi(parent) => parent.entity_id().clone(),
        };
        if signer.as_bytes() != flow.device {
            return Err(Error::Sso("outcome proof signer differs from flow"));
        }
        let status = self.identity_status(host, &uid, key, Some(commitment))?;
        if status.committed_authorization_binding == Some(commitment) {
            HardStateStore::open(&host.database_path)?.sso_transition(
                &id,
                flow.state,
                SsoFlowState::Complete,
                None,
            )?;
        } else if flow.state == SsoFlowState::Binding && flow.final_operation.is_none() {
            HardStateStore::open(&host.database_path)?.sso_transition(
                &id,
                flow.state,
                SsoFlowState::Unknown,
                None,
            )?;
        }
        self.sso_progress(host, id, store)
    }
}
