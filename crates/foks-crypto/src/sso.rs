//! Go-compatible OAuth nonce, PKCE and device binding. IdP claims are a separate authority.
use crate::{Result, YubiDevice};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use foks_proto::{
    OAuth2Binding, OAuth2IdTokenBinding, OAuth2IdTokenBindingPayload, OAuth2Secret,
    OAuth2SessionId, SecretSeed, OAUTH2_BINDING_TYPE_ID, OAUTH2_ID_TOKEN_BINDING_BLOB_TYPE_ID,
};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub fn oauth2_binding_nonce(binding: &OAuth2Binding) -> Result<String> {
    Ok(URL_SAFE_NO_PAD.encode(crate::prefixed_hash_signable(
        OAUTH2_BINDING_TYPE_ID,
        &binding.encoded()?,
    )?))
}
pub fn oauth2_pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
/// 32 random bytes produce the RFC 7636 minimum of 43 characters.
pub fn oauth2_pkce_verifier() -> Result<OAuth2Secret> {
    let mut bytes = Zeroizing::new([0u8; 32]);
    getrandom::fill(bytes.as_mut()).map_err(|_| crate::Error::Entropy)?;
    Ok(OAuth2Secret::new(URL_SAFE_NO_PAD.encode(bytes.as_ref())))
}
pub fn oauth2_session_id() -> Result<OAuth2SessionId> {
    let mut bytes = [0u8; 17];
    bytes[0] = 51;
    getrandom::fill(&mut bytes[1..]).map_err(|_| crate::Error::Entropy)?;
    Ok(OAuth2SessionId(bytes))
}
pub fn sign_oauth2_binding(
    seed: &SecretSeed,
    payload: &OAuth2IdTokenBindingPayload,
) -> Result<OAuth2IdTokenBinding> {
    let inner = Zeroizing::new(payload.encoded()?);
    let signing_seed = crate::derive_key(seed, 0, None)?;
    let signature = crate::sign_ed25519_blob(
        signing_seed.as_bytes(),
        OAUTH2_ID_TOKEN_BINDING_BLOB_TYPE_ID,
        &inner,
    )?;
    Ok(OAuth2IdTokenBinding {
        inner,
        signature,
        key: crate::derive_device_public(seed)?.id,
    })
}
pub fn sign_yubi_oauth2_binding(
    signer: &dyn YubiDevice,
    payload: &OAuth2IdTokenBindingPayload,
) -> Result<OAuth2IdTokenBinding> {
    let inner = Zeroizing::new(payload.encoded()?);
    foks_snowpack::validate_signable(&inner)?;
    let blob = Zeroizing::new(foks_snowpack::encode(&foks_snowpack::Value::Binary(
        inner.to_vec(),
    ))?);
    let signature = crate::sign_yubi_typed(signer, OAUTH2_ID_TOKEN_BINDING_BLOB_TYPE_ID, &blob)?;
    Ok(OAuth2IdTokenBinding {
        inner,
        signature,
        key: signer.entity_id().clone(),
    })
}
pub fn verify_oauth2_binding(
    binding: &OAuth2IdTokenBinding,
) -> Result<OAuth2IdTokenBindingPayload> {
    crate::verify_blob(
        &binding.key,
        &binding.signature,
        OAUTH2_ID_TOKEN_BINDING_BLOB_TYPE_ID,
        &binding.inner,
    )?;
    Ok(OAuth2IdTokenBindingPayload::decode(&binding.inner)?)
}

pub fn sign_identity_proof(
    seed: &SecretSeed,
    challenge: foks_proto::IdentityChallenge,
) -> Result<foks_proto::IdentityProof> {
    if challenge.claim.signer != crate::derive_device_public(seed)?.id {
        return Err(crate::Error::Verification);
    }
    let signing_seed = crate::derive_key(seed, 0, None)?;
    let signature = crate::sign_ed25519_blob(
        signing_seed.as_bytes(),
        foks_proto::IDENTITY_PROOF_TYPE_ID,
        &challenge.encoded()?,
    )?;
    Ok(foks_proto::IdentityProof {
        challenge,
        signature,
    })
}
pub fn sign_yubi_identity_proof(
    signer: &dyn YubiDevice,
    challenge: foks_proto::IdentityChallenge,
) -> Result<foks_proto::IdentityProof> {
    if &challenge.claim.signer != signer.entity_id() {
        return Err(crate::Error::Verification);
    }
    let inner = challenge.encoded()?;
    foks_snowpack::validate_signable(&inner)?;
    let blob = foks_snowpack::encode(&foks_snowpack::Value::Binary(inner))?;
    let signature = crate::sign_yubi_typed(signer, foks_proto::IDENTITY_PROOF_TYPE_ID, &blob)?;
    Ok(foks_proto::IdentityProof {
        challenge,
        signature,
    })
}
pub fn verify_identity_proof(proof: &foks_proto::IdentityProof) -> Result<()> {
    crate::verify_blob(
        &proof.challenge.claim.signer,
        &proof.signature,
        foks_proto::IDENTITY_PROOF_TYPE_ID,
        &proof.challenge.encoded()?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!(
            "../foks-snowpack/tests/fixtures/foks-v0.1.9/sso/{name}"
        ))
        .unwrap()
    }
    #[test]
    fn nonce_pkce_and_signed_binding_match_pinned_go() {
        let binding = OAuth2Binding::decode(&fixture("binding.snowp")).unwrap();
        assert_eq!(
            oauth2_binding_nonce(&binding).unwrap().as_bytes(),
            fixture("nonce.txt")
        );
        let verifier = String::from_utf8(fixture("verifier.txt")).unwrap();
        assert_eq!(verifier.len(), 43);
        assert_eq!(
            oauth2_pkce_challenge(&verifier).as_bytes(),
            fixture("challenge.txt")
        );
        let payload =
            OAuth2IdTokenBindingPayload::decode(&fixture("binding-payload.snowp")).unwrap();
        let seed = SecretSeed::new(fixture("seed.bin").try_into().unwrap());
        let signed = sign_oauth2_binding(&seed, &payload).unwrap();
        assert_eq!(signed.encoded().unwrap(), fixture("signed-binding.snowp"));
        assert_eq!(verify_oauth2_binding(&signed).unwrap(), payload);
        let mut tampered = signed.clone();
        *tampered.inner.last_mut().unwrap() ^= 1;
        assert!(verify_oauth2_binding(&tampered).is_err());
        assert_eq!(oauth2_pkce_verifier().unwrap().expose().len(), 43);
        assert_eq!(oauth2_session_id().unwrap().0[0], 51);
    }
}
