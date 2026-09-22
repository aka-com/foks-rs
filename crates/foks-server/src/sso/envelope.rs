use crate::{
    keys::{HostKeyProvider, KeyGenerationId, KeyPurpose},
    Entropy, Error, Result,
};
use chacha20poly1305::{
    aead::{Aead as _, KeyInit as _, Payload},
    XChaCha20Poly1305, XNonce,
};
use foks_server_db::SsoSession;
use zeroize::Zeroizing;

fn aad(row: &SsoSession) -> Vec<u8> {
    let mut out = b"fennec-oidc-session-v1".to_vec();
    out.extend_from_slice(&row.host);
    out.extend_from_slice(&row.session_hash);
    out.extend_from_slice(&row.config_hash);
    out.extend_from_slice(&row.source_hash);
    out.push(u8::from(row.uid.is_some()));
    if let Some(uid) = row.uid {
        out.extend_from_slice(&uid);
    }
    out.push(row.state as u8);
    out.extend_from_slice(&row.revision.to_be_bytes());
    out.extend_from_slice(&row.authorization_epoch.to_be_bytes());
    out.extend_from_slice(&row.expires_at_ms.to_be_bytes());
    out
}
fn key(secret: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    // Independent domain from recovery tokens; underlying key remains in the server key store.
    Zeroizing::new(foks_crypto::prefixed_hash(0x7a90_3c95_34cf_dbf1, secret))
}
pub(super) fn seal(
    keys: &dyn HostKeyProvider,
    entropy: &dyn Entropy,
    row: &SsoSession,
    bytes: &[u8],
) -> Result<Vec<u8>> {
    seal_bytes(keys, entropy, &aad(row), bytes)
}
pub(super) fn seal_bytes(
    keys: &dyn HostKeyProvider,
    entropy: &dyn Entropy,
    aad: &[u8],
    bytes: &[u8],
) -> Result<Vec<u8>> {
    let secret = keys.load_existing(KeyPurpose::Recovery)?;
    let derived = key(secret.expose());
    let mut nonce = [0; 24];
    entropy.fill(&mut nonce)?;
    let cipher = XChaCha20Poly1305::new((&*derived).into());
    let encrypted = cipher
        .encrypt(&XNonce::from(nonce), Payload { msg: bytes, aad })
        .map_err(|_| Error::KeyCrypto)?;
    let mut out = vec![1];
    out.extend_from_slice(&secret.generation().as_bytes());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&encrypted);
    Ok(out)
}
pub(super) fn open(keys: &dyn HostKeyProvider, row: &SsoSession) -> Result<Zeroizing<Vec<u8>>> {
    open_bytes(keys, &aad(row), &row.ciphertext)
}
pub(super) fn open_bytes(
    keys: &dyn HostKeyProvider,
    aad: &[u8],
    bytes: &[u8],
) -> Result<Zeroizing<Vec<u8>>> {
    if bytes.len() < 57 || bytes.len() > 2 * 1024 * 1024 || bytes[0] != 1 {
        return Err(Error::KeyCrypto);
    }
    let generation =
        KeyGenerationId::from_bytes(bytes[1..17].try_into().map_err(|_| Error::KeyCrypto)?);
    let secret = keys.load_generation(KeyPurpose::Recovery, generation)?;
    let derived = key(secret.expose());
    let cipher = XChaCha20Poly1305::new((&*derived).into());
    Ok(Zeroizing::new(
        cipher
            .decrypt(
                &XNonce::try_from(&bytes[17..41]).map_err(|_| Error::KeyCrypto)?,
                Payload {
                    msg: &bytes[41..],
                    aad,
                },
            )
            .map_err(|_| Error::KeyCrypto)?,
    ))
}
