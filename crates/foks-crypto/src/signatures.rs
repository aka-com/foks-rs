//! Canonical typed signing and verification for software and hardware keys.

use super::*;

/// Returns the Ed25519 public key for a raw 32-byte server signing seed.
pub fn ed25519_public_key(seed: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(seed).verifying_key().to_bytes()
}

/// Signs a canonical object using the FOKS typed Ed25519 message format.
///
/// This low-level primitive is intended for host keys whose persistence and
/// lifetime are managed outside the client-oriented [`SecretSeed`] type. Use
/// [`sign_ed25519_blob`] for `Future(T)` so the inner object is also checked.
pub fn sign_ed25519_typed(
    seed: &[u8; 32],
    type_id: u64,
    canonical_object: &[u8],
) -> Result<Signature> {
    foks_snowpack::validate_signable(canonical_object)?;
    let signing = SigningKey::from_bytes(seed);
    let mut message = Vec::with_capacity(8 + canonical_object.len());
    message.extend_from_slice(&type_id.to_be_bytes());
    message.extend_from_slice(canonical_object);
    Ok(Signature::Ed25519(signing.sign(&message).to_bytes()))
}

/// Signs a Snowpack `Future(T)` blob after validating both its inner object
/// and the binary wrapper covered by the signature.
pub fn sign_ed25519_blob(seed: &[u8; 32], blob_type_id: u64, inner: &[u8]) -> Result<Signature> {
    foks_snowpack::validate_signable(inner)?;
    let encoded_blob = encode(&Value::Binary(inner.to_vec()))?;
    sign_ed25519_typed(seed, blob_type_id, &encoded_blob)
}

/// Signs an exact canonical object with a PUK/PTK seed using FOKS's typed
/// Ed25519 message format. Use [`sign_shared_key_blob`] for `Future(T)` so the
/// inner object is also checked.
pub fn sign_shared_key_typed(
    seed: &SecretSeed,
    type_id: u64,
    canonical_object: &[u8],
) -> Result<Signature> {
    foks_snowpack::validate_signable(canonical_object)?;
    let signing_seed = derive_key(seed, 0, None)?;
    let signing = SigningKey::from_bytes(signing_seed.as_bytes());
    let mut message = Vec::with_capacity(8 + canonical_object.len());
    message.extend_from_slice(&type_id.to_be_bytes());
    message.extend_from_slice(canonical_object);
    Ok(Signature::Ed25519(signing.sign(&message).to_bytes()))
}

/// Signs a Snowpack `Future(T)` blob with a PUK/PTK seed after validating both
/// its inner object and the binary wrapper covered by the signature.
pub fn sign_shared_key_blob(
    seed: &SecretSeed,
    blob_type_id: u64,
    inner: &[u8],
) -> Result<Signature> {
    foks_snowpack::validate_signable(inner)?;
    let encoded_blob = encode(&Value::Binary(inner.to_vec()))?;
    sign_shared_key_typed(seed, blob_type_id, &encoded_blob)
}

/// Signs the exact `Future(TeamBearerTokenChallengePayload)` blob expected by
/// TeamAdmin.activateTeamBearerToken.
pub fn sign_team_bearer_token_challenge(
    seed: &SecretSeed,
    challenge: &foks_proto::TeamBearerTokenChallenge,
) -> Result<Signature> {
    sign_shared_key_blob(
        seed,
        foks_proto::TEAM_BEARER_TOKEN_CHALLENGE_BLOB_TYPE_ID,
        &challenge.encoded_payload()?,
    )
}

pub(super) fn sign_seed_typed(
    seed: &SecretSeed,
    type_id: u64,
    canonical_object: &[u8],
) -> Result<Signature> {
    sign_shared_key_typed(seed, type_id, canonical_object)
}

pub fn sign_yubi_typed(
    signer: &dyn YubiDevice,
    type_id: u64,
    canonical_object: &[u8],
) -> Result<Signature> {
    let mut message = Vec::with_capacity(8 + canonical_object.len());
    message.extend(type_id.to_be_bytes());
    message.extend(canonical_object);
    let digest = prefixed_hash_without_type(&message);
    let signature = Signature::Ecdsa(signer.sign_sha512_256(&digest)?);
    verify_typed(signer.entity_id(), &signature, type_id, canonical_object)?;
    Ok(signature)
}

/// Verifies a FOKS signature over an already encoded typed object.
///
/// Use [`verify_blob`] for `Future(T)` so the inner object, rather than only
/// its binary wrapper, is checked for Go-compatible canonicality.
pub fn verify_typed(
    signer: &EntityId,
    signature: &Signature,
    type_id: u64,
    canonical_object: &[u8],
) -> Result<()> {
    // Verify that the signed object conforms to canonical signable encoding
    // before evaluating the signature.
    foks_snowpack::validate_signable(canonical_object)?;
    let mut message = Vec::with_capacity(8 + canonical_object.len());
    message.extend(type_id.to_be_bytes());
    message.extend(canonical_object);
    match signature {
        Signature::Ed25519(signature) if signer.entity_type() != foks_proto::ENTITY_YUBI => {
            let key =
                VerifyingKey::from_bytes(&signer.ed25519_key()?).map_err(|_| Error::PublicKey)?;
            key.verify_strict(&message, &DalekSignature::from_bytes(signature))
                .map_err(|_| Error::Verification)
        }
        Signature::Ecdsa(signature) if signer.entity_type() == foks_proto::ENTITY_YUBI => {
            let key = P256VerifyingKey::from_sec1_bytes(&signer.p256_key()?)
                .map_err(|_| Error::PublicKey)?;
            // Accepts both low-S and high-S ECDSA signatures to accommodate
            // YubiKey PIV hardware, which does not normalize S values. Mutation
            // replay protection relies on chain sequence, previous hash, and
            // Merkle root bindings rather than signature malleability resistance.
            let signature = P256Signature::from_der(signature).map_err(|_| Error::Verification)?;
            let digest = prefixed_hash_without_type(&message);
            key.verify_prehash(&digest, &signature)
                .map_err(|_| Error::Verification)
        }
        _ => Err(Error::SignatureType),
    }
}

pub(super) fn prefixed_hash_without_type(message: &[u8]) -> [u8; 32] {
    Sha512_256::digest(message).into()
}

/// Verifies a Snowpack `Future(T)` blob. Its signature covers the typed blob,
/// which means the inner canonical bytes are themselves encoded as a binary
/// Snowpack value after the type prefix.
pub fn verify_blob(
    signer: &EntityId,
    signature: &Signature,
    blob_type_id: u64,
    inner: &[u8],
) -> Result<()> {
    // Verify2 checks the Future's decoded inner bytes for canonicality before
    // it checks the signature. Checking only the bin wrapper would allow every
    // byte string, including Rust-only array16(16..=31) encodings.
    foks_snowpack::validate_signable(inner)?;
    let encoded_blob = encode(&Value::Binary(inner.to_vec()))?;
    verify_typed(signer, signature, blob_type_id, &encoded_blob)
}
