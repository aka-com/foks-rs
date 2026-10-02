//! Shared typed hashes, commitments, MACs, and secretbox primitives.

use crate::{Error, Result};
use crypto_secretbox::{aead::Aead, KeyInit, XSalsa20Poly1305};
use foks_proto::TREE_LOCATION_TYPE_ID;
use foks_snowpack::{encode, Value};
use hmac::{Hmac, Mac};
use sha2::{Digest as _, Sha512_256};
use zeroize::Zeroizing;

// Rust-local storage/journal domain; never encoded on the v0.1.9 wire.
const FEDERATION_PERMISSION_TOKEN_HASH_TYPE_ID: u64 = 0x45cf_32f3_7d38_a811;

pub(super) fn typed_hmac(key: &[u8], type_id: u64, object: &[u8]) -> [u8; 32] {
    let mut mac =
        <Hmac<Sha512_256> as Mac>::new_from_slice(key).expect("HMAC accepts every key length");
    mac.update(&type_id.to_be_bytes());
    mac.update(object);
    mac.finalize().into_bytes().into()
}

pub(super) fn verify_mac(
    key: &[u8],
    type_id: u64,
    object: &[u8],
    expected: &[u8; 32],
) -> Result<()> {
    let mut mac =
        <Hmac<Sha512_256> as Mac>::new_from_slice(key).expect("HMAC accepts every key length");
    mac.update(&type_id.to_be_bytes());
    mac.update(object);
    mac.verify_slice(expected).map_err(|_| Error::KvBinding)
}

/// Computes a domain-separated HMAC-SHA-512/256 for opaque server
/// capabilities. Protocol-specific code should assign a stable, unique
/// `type_id` and MAC the exact canonical payload bytes.
pub fn capability_mac(key: &[u8], type_id: u64, object: &[u8]) -> [u8; 32] {
    typed_hmac(key, type_id, object)
}

/// Derives the hidden first Merkle location for a user or team subchain.
pub fn subchain_tree_location(seed: &[u8; 32], chain_type: u64) -> Result<[u8; 32]> {
    if !matches!(
        chain_type,
        foks_proto::CHAIN_TYPE_USER_SETTINGS | foks_proto::CHAIN_TYPE_TEAM_MEMBERSHIP
    ) {
        return Err(foks_proto::Error::UnknownEnum {
            kind: "subchain type",
            value: chain_type,
        }
        .into());
    }
    let object = encode(&Value::Array(vec![
        Value::Unsigned(chain_type),
        Value::Variant(None),
    ]))?;
    Ok(typed_hmac(
        seed,
        foks_proto::CHAIN_LOCATION_DERIVATION_TYPE_ID,
        &object,
    ))
}

/// Verifies a capability MAC in constant time.
pub fn verify_capability_mac(
    key: &[u8],
    type_id: u64,
    object: &[u8],
    expected: &[u8; 32],
) -> Result<()> {
    verify_mac(key, type_id, object, expected)
}

pub(super) fn open_typed_secretbox(
    key: &[u8; 32],
    type_id: u64,
    partial_nonce: &[u8; 16],
    ciphertext: &[u8],
) -> Result<Zeroizing<Vec<u8>>> {
    let mut nonce = [0u8; 24];
    nonce[..8].copy_from_slice(&type_id.to_be_bytes());
    nonce[8..].copy_from_slice(partial_nonce);
    let cipher = XSalsa20Poly1305::new(key.into());
    cipher
        .decrypt((&nonce).into(), ciphertext)
        .map(Zeroizing::new)
        .map_err(|_| Error::Decryption)
}

pub(super) fn seal_typed_secretbox(
    key: &[u8; 32],
    type_id: u64,
    partial_nonce: &[u8; 16],
    plaintext: &[u8],
    padded: bool,
) -> Result<Vec<u8>> {
    let mut nonce = [0u8; 24];
    nonce[..8].copy_from_slice(&type_id.to_be_bytes());
    nonce[8..].copy_from_slice(partial_nonce);
    let mut plaintext = Zeroizing::new(plaintext.to_vec());
    if padded {
        let target = plaintext.len().max(32).next_power_of_two();
        plaintext.resize(target, 0);
    }
    XSalsa20Poly1305::new(key.into())
        .encrypt((&nonce).into(), plaintext.as_ref())
        .map_err(|_| Error::KvEncryption)
}

pub(super) fn require_zero_padding(plaintext: &[u8], consumed: usize) -> Result<()> {
    if plaintext[consumed..].iter().any(|byte| *byte != 0) {
        return Err(Error::KvPadding);
    }
    Ok(())
}

/// SHA-512/256 over the 8-byte big-endian type ID and arbitrary bytes.
///
/// This primitive does not validate that `object` is a signable Snowpack
/// encoding. Protocol objects must use [`prefixed_hash_signable`] to ensure
/// canonical wire representation before hashing.
pub fn prefixed_hash(type_id: u64, object: &[u8]) -> [u8; 32] {
    let mut hash = Sha512_256::new();
    hash.update(type_id.to_be_bytes());
    hash.update(object);
    hash.finalize().into()
}

/// SHA-512/256 over a typed, signable Snowpack object.
///
/// Enforces canonical signable Snowpack rules before hashing to ensure
/// cross-implementation digest consistency.
pub fn prefixed_hash_signable(type_id: u64, canonical_object: &[u8]) -> Result<[u8; 32]> {
    foks_snowpack::validate_signable(canonical_object)?;
    Ok(prefixed_hash(type_id, canonical_object))
}

pub(super) fn tree_location_commitment(location: &[u8; 32]) -> Result<[u8; 32]> {
    prefixed_hash_signable(
        TREE_LOCATION_TYPE_ID,
        &encode(&Value::Binary(location.to_vec()))?,
    )
}

/// One-way storage and journal identity for a remote-view bearer token.
pub fn federation_permission_token_hash(token: &foks_proto::PermissionToken) -> [u8; 32] {
    prefixed_hash(FEDERATION_PERMISSION_TOKEN_HASH_TYPE_ID, token.expose())
}

/// HMAC-SHA-512/256 commitment used by FOKS for disclosed chain metadata.
pub fn commitment(type_id: u64, canonical_object: &[u8], key: &[u8]) -> [u8; 32] {
    let mut mac =
        <Hmac<Sha512_256> as Mac>::new_from_slice(key).expect("HMAC accepts every key length");
    mac.update(&type_id.to_be_bytes());
    mac.update(canonical_object);
    mac.finalize().into_bytes().into()
}
