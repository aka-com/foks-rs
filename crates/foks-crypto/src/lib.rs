//! Cryptographic operations for the implemented FOKS v0.1.9 client protocols.
//!
//! This crate owns typed hashes and signatures, device and shared-key
//! derivation, hybrid key distribution, mutation construction, and KV
//! authenticated encryption. Protocol policy remains in `foks-verify`.

#![forbid(unsafe_code)]

mod invitations;
pub use invitations::*;
mod account;
mod backup;
mod bot_token;
pub use account::*;
pub use bot_token::*;
mod chat_v2;
mod kex;
mod passphrase;
mod realtime;
mod sso;
pub use sso::*;

pub use backup::*;
pub use chat_v2::*;
pub use kex::*;
pub use passphrase::*;
pub use realtime::*;

use crypto_secretbox::{aead::Aead, KeyInit, XSalsa20Poly1305};
use ed25519_dalek::{Signature as DalekSignature, Signer as _, SigningKey, VerifyingKey};
use foks_proto::{
    AdHocMembershipLinkPublic, ApprovedMembershipLinkPublic, ChangeMetadata,
    DeviceLabelNameAndCommitmentKey, DhPublicKey, EntityId, Hepk, HybridBox, KvDirectory, KvDirent,
    KvDirentName, KvEncryptedChunk, KvLargeFileMetadata, KvNodeId, KvParty, KvRoot, KvSmallFileBox,
    KvSmallFilePlaintext, KvUploadChunk, KvUploadFinal, PukParcel, Role, RoleAndGeneration,
    SecretBox, SecretSeed, SharedKeyBox, SharedKeyBoxSet, SharedKeyBoxTarget, SharedKeySeed,
    Signature, SoftwareEldestPublic, SubkeySeed, TeamGroupChange, TeamKeyOwner, TeamMemberChange,
    TeamMemberKeys, TeamRemovalAndCommitment, TeamRemovalBoxData, TeamRemovalKeyBox,
    TeamRemovalKeyMetadata, TeamRemovalKeyPayload, TeamRemovalMacPayload, TeamRemovalProof,
    TreeRoot, UnsignedUserLink, UserGroupChange, UserLink, UserMemberChange, UserMemberKeys,
    UserSharedKey, YubiEldestPublic, APP_KEY_DERIVATION_TYPE_ID, DEVICE_LABEL_TYPE_ID,
    HEPK_TYPE_ID, HYBRID_SECRET_KEY_SHA3_PAYLOAD_TYPE_ID, KV_CHUNK_NONCE_PAYLOAD_TYPE_ID,
    KV_DIRENT_BINDING_PAYLOAD_TYPE_ID, KV_DIRENT_NAME_PAYLOAD_TYPE_ID, KV_FILE_KEY_PAYLOAD_TYPE_ID,
    KV_KEY_DERIVATION_TYPE_ID, KV_ROOT_BINDING_PAYLOAD_TYPE_ID, LINK_OUTER_V1_TYPE_ID,
    NAME_COMMITMENT_TYPE_ID, SHARED_KEY_SEED_TYPE_ID, SUBKEY_SEED_TYPE_ID,
    TEAM_REMOVAL_KEY_BOX_PAYLOAD_TYPE_ID, TEAM_REMOVAL_KEY_TYPE_ID,
    TEAM_REMOVAL_MAC_PAYLOAD_TYPE_ID, TEMP_DH_KEY_SIG_TEMPLATE_TYPE_ID, TREE_LOCATION_TYPE_ID,
};
use foks_snowpack::{decode, decode_prefix, encode, encode_ref, Value, ValueRef};
use hmac::{Hmac, Mac};
use ml_kem::{ml_kem_768, Decapsulate as _, KeyExport as _, TryKeyInit as _};
use p256::ecdsa::{
    signature::hazmat::PrehashVerifier as _, Signature as P256Signature,
    VerifyingKey as P256VerifyingKey,
};
use p256::{
    ecdh::diffie_hellman as p256_diffie_hellman, elliptic_curve::sec1::ToEncodedPoint as _,
    PublicKey as P256PublicKey, SecretKey as P256SecretKey,
};
use salsa20::{cipher::consts::U10, hsalsa};
use sha2::{Digest as _, Sha512_256};
use sha3::{Digest as Sha3Digest, Sha3_256};
use thiserror::Error;
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};
use zeroize::Zeroizing;

// Rust-standalone local storage/journal domain separator. This value is never
// encoded on the FOKS v0.1.9 wire and is not an upstream Snowpack type ID.
const FEDERATION_PERMISSION_TOKEN_HASH_TYPE_ID: u64 = 0x45cf_32f3_7d38_a811;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid FOKS protocol value: {0}")]
    Protocol(#[from] foks_proto::Error),
    #[error("invalid canonical Snowpack: {0}")]
    Snowpack(#[from] foks_snowpack::Error),
    #[error("signature type does not match its entity key")]
    SignatureType,
    #[error("invalid Ed25519 public key")]
    PublicKey,
    #[error("Ed25519 signature verification failed")]
    Verification,
    #[error("invalid device seed or derived key material")]
    DeviceKey,
    #[error("unsupported FOKS hybrid box")]
    HybridBox,
    #[error("ML-KEM-768 decapsulation input is invalid")]
    MlKem,
    #[error("PUK authenticated decryption failed")]
    Decryption,
    #[error("PUK parcel does not target this device")]
    WrongReceiver,
    #[error("PUK cleartext does not match its parcel or verified user chain")]
    PukBinding,
    #[error("invalid ad-hoc team key or hidden-location material")]
    AdHocTeamMaterial,
    #[error("invalid named-team key, name, removal-key, or hidden-location material")]
    NamedTeamMaterial,
    #[error("Yubi signing operation failed")]
    YubiSigning,
    #[error("KV authenticated binding failed")]
    KvBinding,
    #[error("KV ciphertext role or generation does not match the selected shared key")]
    KvKeyMismatch,
    #[error("invalid KV plaintext padding")]
    KvPadding,
    #[error("KV authenticated encryption failed")]
    KvEncryption,
    #[error("invalid FOKS passphrase input or state")]
    Passphrase,
    #[error("FOKS passphrase stretching failed")]
    PassphraseStretch,
    #[error("invalid realtime crypto context or encryption input")]
    Realtime,
    #[error("OS randomness is unavailable")]
    Entropy,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
#[cfg(test)]
type HybridDerivation = (Zeroizing<[u8; 32]>, Zeroizing<Vec<u8>>);

const SMALL_FILE_PAYLOAD_TYPE_ID: u64 = 0xaeec_688f_3145_fddf;
const DIR_KEY_SEED_TYPE_ID: u64 = 0x8aec_e656_6b24_4356;

/// Application-specific MAC and box keys derived from one exact PUK/PTK seed.
pub struct KvKeySet {
    mac: Zeroizing<[u8; 32]>,
    box_key: Zeroizing<[u8; 32]>,
}

/// Derives the v0.1.9 KV application key and its MAC/secretbox subkeys.
pub fn derive_kv_keys(shared_key_seed: &SecretSeed) -> Result<KvKeySet> {
    let app_key = derive_key(shared_key_seed, 5, None)?;
    let kv_app = typed_hmac(
        app_key.as_slice(),
        APP_KEY_DERIVATION_TYPE_ID,
        &encode(&Value::Array(vec![
            Value::Unsigned(0),
            Value::Variant(Some((b"0".to_vec(), Box::new(Value::Unsigned(0))))),
        ]))?,
    );
    let derivation = |kind| {
        typed_hmac(
            &kv_app,
            KV_KEY_DERIVATION_TYPE_ID,
            &encode(&Value::Array(vec![
                Value::Unsigned(kind),
                Value::Variant(None),
            ]))
            .expect("fixed KV derivation is canonical"),
        )
    };
    Ok(KvKeySet {
        mac: Zeroizing::new(derivation(1)),
        box_key: Zeroizing::new(derivation(2)),
    })
}

impl KvKeySet {
    pub fn bind_root(
        &self,
        party: &KvParty,
        root: [u8; 16],
        version: u64,
        key: RoleAndGeneration,
    ) -> Result<[u8; 32]> {
        let payload = encode(&Value::Array(vec![
            party.to_value(),
            key.to_value(),
            Value::Binary(root.to_vec()),
            Value::Unsigned(version),
        ]))?;
        Ok(typed_hmac(
            self.mac.as_slice(),
            KV_ROOT_BINDING_PAYLOAD_TYPE_ID,
            &payload,
        ))
    }

    pub fn verify_root(&self, root: &KvRoot, party: &KvParty) -> Result<()> {
        let payload = root.binding_payload(party)?;
        verify_mac(
            self.mac.as_slice(),
            KV_ROOT_BINDING_PAYLOAD_TYPE_ID,
            &payload,
            &root.binding_mac,
        )
    }

    pub fn open_directory_seed(&self, directory: &KvDirectory) -> Result<SecretSeed> {
        let plaintext = open_typed_secretbox(
            &self.box_key,
            DIR_KEY_SEED_TYPE_ID,
            &directory.id,
            &directory.seed_ciphertext,
        )?;
        if plaintext.len() < 34 || plaintext[..2] != [0xc4, 32] {
            return Err(Error::KvBinding);
        }
        require_zero_padding(&plaintext, 34)?;
        Ok(SecretSeed::from_slice(&plaintext[2..34])?)
    }

    pub fn open_small_file(
        &self,
        id: KvNodeId,
        boxed: &KvSmallFileBox,
    ) -> Result<KvSmallFilePlaintext> {
        if boxed.key.generation == 0 {
            return Err(Error::KvKeyMismatch);
        }
        let object_id = id.object_id();
        let plaintext = open_typed_secretbox(
            &self.box_key,
            SMALL_FILE_PAYLOAD_TYPE_ID,
            &object_id,
            &boxed.ciphertext,
        )?;
        let (value, consumed) = decode_prefix(&plaintext)?;
        require_zero_padding(&plaintext, consumed)?;
        KvSmallFilePlaintext::decode_value(value).map_err(Into::into)
    }

    pub fn open_file_seed(
        &self,
        id: KvNodeId,
        metadata: &KvLargeFileMetadata,
    ) -> Result<SecretSeed> {
        let plaintext = open_typed_secretbox(
            &self.box_key,
            KV_FILE_KEY_PAYLOAD_TYPE_ID,
            &metadata.key_seed.nonce,
            &metadata.key_seed.ciphertext,
        )?;
        let (value, seed) = decode_with_redacted_trailing_seed(&plaintext)?;
        let fields = match value {
            Value::Array(fields) if fields.len() == 3 => fields,
            _ => return Err(Error::KvBinding),
        };
        if fields[0] != Value::Binary(id.object_id().to_vec())
            || fields[1] != Value::Unsigned(metadata.version)
        {
            return Err(Error::KvBinding);
        }
        let Value::Binary(redacted) = &fields[2] else {
            return Err(Error::KvBinding);
        };
        if redacted.as_slice() != [0; 32] {
            return Err(Error::KvBinding);
        }
        Ok(seed)
    }

    pub fn seal_directory_seed(
        &self,
        directory_id: [u8; 16],
        seed: &SecretSeed,
    ) -> Result<Vec<u8>> {
        let plaintext = Zeroizing::new(encode_ref(&ValueRef::Binary(seed.as_slice()))?);
        seal_typed_secretbox(
            &self.box_key,
            DIR_KEY_SEED_TYPE_ID,
            &directory_id,
            &plaintext,
            false,
        )
    }

    /// Seals a v0.1.9 small-file or symlink payload.
    ///
    /// `id.object_id()` supplies the deterministic Secretbox nonce suffix and
    /// must be freshly generated for every distinct plaintext under this key.
    /// Callers constructing IDs directly must ensure object ID uniqueness
    /// across file and symlink variants.
    pub fn seal_small_file(
        &self,
        id: KvNodeId,
        key: RoleAndGeneration,
        plaintext: KvSmallFilePlaintext,
    ) -> Result<KvSmallFileBox> {
        let plaintext = match &plaintext {
            KvSmallFilePlaintext::File(bytes) => ValueRef::Array(vec![
                ValueRef::Unsigned(3),
                ValueRef::Variant(Some((b"0", Box::new(ValueRef::Binary(bytes))))),
            ]),
            KvSmallFilePlaintext::Symlink(path) => ValueRef::Array(vec![
                ValueRef::Unsigned(4),
                ValueRef::Variant(Some((b"1", Box::new(ValueRef::Text(path))))),
            ]),
        };
        let plaintext = Zeroizing::new(encode_ref(&plaintext)?);
        Ok(KvSmallFileBox {
            key,
            ciphertext: seal_typed_secretbox(
                &self.box_key,
                SMALL_FILE_PAYLOAD_TYPE_ID,
                &id.object_id(),
                &plaintext,
                true,
            )?,
        })
    }

    pub fn seal_file_seed(
        &self,
        id: KvNodeId,
        key: RoleAndGeneration,
        version: u64,
        file_seed: &SecretSeed,
        nonce: [u8; 16],
    ) -> Result<KvLargeFileMetadata> {
        let object_id = id.object_id();
        let plaintext = Zeroizing::new(encode_ref(&ValueRef::Array(vec![
            ValueRef::Binary(&object_id),
            ValueRef::Unsigned(version),
            ValueRef::Binary(file_seed.as_slice()),
        ]))?);
        Ok(KvLargeFileMetadata {
            key,
            key_seed: SecretBox {
                nonce,
                ciphertext: seal_typed_secretbox(
                    &self.box_key,
                    KV_FILE_KEY_PAYLOAD_TYPE_ID,
                    &nonce,
                    &plaintext,
                    false,
                )?,
            },
            version,
            custom_metadata: None,
        })
    }
}

pub fn seal_kv_dirent_name(
    directory_seed: &SecretSeed,
    parent: [u8; 16],
    directory_version: u64,
    name: Vec<u8>,
    nonce: [u8; 16],
) -> Result<([u8; 32], SecretBox)> {
    let keys = derive_seed_kv_keys(directory_seed)?;
    let payload = Zeroizing::new(encode_ref(&ValueRef::Array(vec![
        ValueRef::Binary(&parent),
        ValueRef::Unsigned(directory_version),
        ValueRef::Text(&name),
    ]))?);
    let mac = typed_hmac(
        keys.mac.as_slice(),
        KV_DIRENT_NAME_PAYLOAD_TYPE_ID,
        &payload,
    );
    let ciphertext = seal_typed_secretbox(
        &keys.box_key,
        KV_DIRENT_NAME_PAYLOAD_TYPE_ID,
        &nonce,
        &payload,
        true,
    )?;
    Ok((mac, SecretBox { nonce, ciphertext }))
}

pub fn bind_kv_dirent(directory_seed: &SecretSeed, dirent: &KvDirent) -> Result<[u8; 32]> {
    let keys = derive_seed_kv_keys(directory_seed)?;
    Ok(typed_hmac(
        keys.mac.as_slice(),
        KV_DIRENT_BINDING_PAYLOAD_TYPE_ID,
        &dirent.binding_payload()?,
    ))
}

pub fn seal_kv_chunk(
    file_seed: &SecretSeed,
    file_id: KvNodeId,
    offset: u64,
    final_chunk: bool,
    cleartext: &[u8],
    encrypted_size_before: u64,
) -> Result<KvUploadChunk> {
    let encoded = Zeroizing::new(encode_ref(&ValueRef::Binary(cleartext))?);
    let padded_length = kv_chunk_padded_length(encoded.len())?;
    let mut padded = Zeroizing::new(vec![0; padded_length]);
    padded[..encoded.len()].copy_from_slice(&encoded);
    let nonce_value = encode(&Value::Array(vec![
        Value::Binary(file_id.object_id().to_vec()),
        Value::Unsigned(offset),
        Value::Bool(final_chunk),
    ]))?;
    let hash = prefixed_hash_signable(KV_CHUNK_NONCE_PAYLOAD_TYPE_ID, &nonce_value)?;
    let nonce: [u8; 24] = hash[..24].try_into().expect("slice length is fixed");
    let cipher = XSalsa20Poly1305::new(file_seed.as_bytes().into());
    let ciphertext = cipher
        .encrypt((&nonce).into(), padded.as_ref())
        .map_err(|_| Error::KvEncryption)?;
    let encrypted_size = encrypted_size_before
        .checked_add(u64::try_from(ciphertext.len()).map_err(|_| Error::KvPadding)?)
        .ok_or(Error::KvPadding)?;
    Ok(KvUploadChunk {
        ciphertext,
        offset,
        final_upload: final_chunk.then_some(KvUploadFinal {
            size: encrypted_size,
            chunk_sum: [0; 32],
        }),
    })
}

/// Verifies and decrypts one directory entry name under its directory seed.
pub fn open_kv_dirent_name(directory_seed: &SecretSeed, dirent: &KvDirent) -> Result<KvDirentName> {
    let keys = derive_seed_kv_keys(directory_seed)?;
    let plaintext = open_typed_secretbox(
        &keys.box_key,
        KV_DIRENT_NAME_PAYLOAD_TYPE_ID,
        &dirent.name_box.nonce,
        &dirent.name_box.ciphertext,
    )?;
    let (value, consumed) = decode_prefix(&plaintext)?;
    require_zero_padding(&plaintext, consumed)?;
    let canonical = encode(&value)?;
    verify_mac(
        keys.mac.as_slice(),
        KV_DIRENT_NAME_PAYLOAD_TYPE_ID,
        &canonical,
        &dirent.name_mac,
    )?;
    verify_mac(
        keys.mac.as_slice(),
        KV_DIRENT_BINDING_PAYLOAD_TYPE_ID,
        &dirent.binding_payload()?,
        &dirent.binding_mac,
    )?;
    let name = KvDirentName::decode_value(value)?;
    if name.parent != dirent.parent || name.directory_version != dirent.directory_version {
        return Err(Error::KvBinding);
    }
    Ok(name)
}

/// Opens the large-file chunk at `requested_offset`.
///
/// Verifies the decrypted chunk offset against `requested_offset` before
/// returning plaintext to prevent chunk substitution by a malicious peer.
pub fn open_kv_chunk(
    file_seed: &SecretSeed,
    file_id: KvNodeId,
    requested_offset: u64,
    chunk: &KvEncryptedChunk,
) -> Result<Vec<u8>> {
    if chunk.offset != requested_offset {
        return Err(Error::KvBinding);
    }
    let nonce_value = encode(&Value::Array(vec![
        Value::Binary(file_id.object_id().to_vec()),
        Value::Unsigned(requested_offset),
        Value::Bool(chunk.final_chunk),
    ]))?;
    let hash = prefixed_hash_signable(KV_CHUNK_NONCE_PAYLOAD_TYPE_ID, &nonce_value)?;
    let nonce: [u8; 24] = hash[..24].try_into().expect("slice length is fixed");
    let cipher = XSalsa20Poly1305::new(file_seed.as_bytes().into());
    let plaintext = Zeroizing::new(
        cipher
            .decrypt((&nonce).into(), chunk.ciphertext.as_ref())
            .map_err(|_| Error::Decryption)?,
    );
    let (value, consumed) = decode_prefix(&plaintext)?;
    require_zero_padding(&plaintext, consumed)?;
    let Value::Binary(bytes) = value else {
        return Err(Error::KvBinding);
    };
    Ok(bytes)
}

fn derive_seed_kv_keys(seed: &SecretSeed) -> Result<KvKeySet> {
    let derivation = |kind| {
        typed_hmac(
            seed.as_slice(),
            KV_KEY_DERIVATION_TYPE_ID,
            &encode(&Value::Array(vec![
                Value::Unsigned(kind),
                Value::Variant(None),
            ]))
            .expect("fixed KV derivation is canonical"),
        )
    };
    Ok(KvKeySet {
        mac: Zeroizing::new(derivation(1)),
        box_key: Zeroizing::new(derivation(2)),
    })
}

fn typed_hmac(key: &[u8], type_id: u64, object: &[u8]) -> [u8; 32] {
    let mut mac =
        <Hmac<Sha512_256> as Mac>::new_from_slice(key).expect("HMAC accepts every key length");
    mac.update(&type_id.to_be_bytes());
    mac.update(object);
    mac.finalize().into_bytes().into()
}

fn verify_mac(key: &[u8], type_id: u64, object: &[u8], expected: &[u8; 32]) -> Result<()> {
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

fn open_typed_secretbox(
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

fn seal_typed_secretbox(
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

fn kv_chunk_padded_length(length: usize) -> Result<usize> {
    if length > u32::MAX as usize {
        return Err(Error::KvPadding);
    }
    let mut base = 32usize;
    let mut overhead = 2usize;
    loop {
        if base >= 0x1_0000 {
            overhead = 5;
        } else if base >= 0x100 {
            overhead = 3;
        }
        let total = base.checked_add(overhead).ok_or(Error::KvPadding)?;
        if length <= total {
            return Ok(total);
        }
        base = base.checked_mul(2).ok_or(Error::KvPadding)?;
    }
}

fn require_zero_padding(plaintext: &[u8], consumed: usize) -> Result<()> {
    if plaintext[consumed..].iter().any(|byte| *byte != 0) {
        return Err(Error::KvPadding);
    }
    Ok(())
}

fn decode_with_redacted_trailing_seed(plaintext: &[u8]) -> Result<(Value, SecretSeed)> {
    const ENCODED_SEED_LENGTH: usize = 34;
    if plaintext.len() < ENCODED_SEED_LENGTH
        || plaintext[plaintext.len() - ENCODED_SEED_LENGTH..plaintext.len() - 32] != [0xc4, 32]
    {
        return Err(Error::KvBinding);
    }
    let seed_offset = plaintext.len() - 32;
    let mut redacted = Zeroizing::new(plaintext.to_vec());
    let seed = SecretSeed::from_slice(&redacted[seed_offset..])?;
    redacted[seed_offset..].fill(0);
    Ok((decode(&redacted)?, seed))
}

/// Fingerprint of bounded plaintext for a local adapter's durable upload intent.
/// The domain separator is local and never emitted on the FOKS wire.
pub fn kv_adapter_body_hash(body: &[u8]) -> [u8; 32] {
    prefixed_hash(0x5de4_c49e_cabc_9643, body)
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

fn tree_location_commitment(location: &[u8; 32]) -> Result<[u8; 32]> {
    prefixed_hash_signable(
        TREE_LOCATION_TYPE_ID,
        &encode(&Value::Binary(location.to_vec()))?,
    )
}

/// One-way storage and journal identity for a remote-view bearer token.
pub fn federation_permission_token_hash(token: &foks_proto::PermissionToken) -> [u8; 32] {
    prefixed_hash(FEDERATION_PERMISSION_TOKEN_HASH_TYPE_ID, token.expose())
}

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

/// Computes the exact v0.1.9 commitment authenticated by named-team member
/// links and removal-key boxes.
pub fn team_removal_key_commitment(removal_key: &SecretSeed) -> Result<[u8; 32]> {
    // Canonical Snowpack encodes a 32-byte blob as bin8(32). Construct it in
    // zeroizing fixed storage so commitment calculation makes no ordinary
    // heap copy of the removal key.
    let mut encoded = Zeroizing::new([0_u8; 34]);
    encoded[0] = 0xc4;
    encoded[1] = 32;
    encoded[2..].copy_from_slice(removal_key.as_bytes());
    prefixed_hash_signable(TEAM_REMOVAL_KEY_TYPE_ID, encoded.as_slice())
}

/// HMAC-SHA-512/256 commitment used by FOKS for disclosed chain metadata.
pub fn commitment(type_id: u64, canonical_object: &[u8], key: &[u8]) -> [u8; 32] {
    let mut mac =
        <Hmac<Sha512_256> as Mac>::new_from_slice(key).expect("HMAC accepts every key length");
    mac.update(&type_id.to_be_bytes());
    mac.update(canonical_object);
    mac.finalize().into_bytes().into()
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

fn sign_seed_typed(seed: &SecretSeed, type_id: u64, canonical_object: &[u8]) -> Result<Signature> {
    sign_shared_key_typed(seed, type_id, canonical_object)
}

/// Public signing and hybrid-encryption material for a PUK/PTK seed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SharedPublicMaterial {
    pub verify_key: EntityId,
    pub hepk: Hepk,
}

/// Complete public result of constructing a software-device eldest link.
pub struct SoftwareEldestMaterial {
    pub uid: EntityId,
    pub device: DevicePublicMaterial,
    pub puk: SharedPublicMaterial,
    pub link: UserLink,
}

/// Complete public result of constructing a Yubi-parent eldest link.
pub struct YubiEldestMaterial {
    pub uid: EntityId,
    pub device: DevicePublicMaterial,
    pub subkey: DevicePublicMaterial,
    pub puk: SharedPublicMaterial,
    pub link: UserLink,
}

pub struct AdHocTeamInput<'a> {
    pub user: &'a EntityId,
    pub host: &'a EntityId,
    pub root: &'a TreeRoot,
    pub time: u64,
    pub owner_puk_generation: u64,
    pub membership_sequence: u64,
    pub membership_previous: Option<[u8; 32]>,
    pub next_tree_location: [u8; 32],
    pub subchain_tree_location: [u8; 32],
    pub membership_next_tree_location: [u8; 32],
}

pub struct AdHocTeamMaterial {
    pub team: EntityId,
    pub link: UserLink,
    pub membership_link: UserLink,
    pub ptks: Vec<SharedPublicMaterial>,
    pub next_tree_location: [u8; 32],
    pub subchain_tree_location: [u8; 32],
    pub membership_next_tree_location: [u8; 32],
}

pub struct NamedTeamInput<'a> {
    pub user: &'a EntityId,
    pub host: &'a EntityId,
    pub root: &'a TreeRoot,
    pub time: u64,
    pub owner_puk_generation: u64,
    pub membership_sequence: u64,
    pub membership_previous: Option<[u8; 32]>,
    pub normalized_name: &'a [u8],
    pub name_sequence: u64,
    pub team_name_commitment_key: [u8; 16],
    pub next_tree_location: [u8; 32],
    pub subchain_tree_location: [u8; 32],
    pub membership_next_tree_location: [u8; 32],
}

pub struct NamedTeamMaterial {
    pub team: EntityId,
    pub link: UserLink,
    pub membership_link: UserLink,
    pub ptks: Vec<SharedPublicMaterial>,
    pub removal_key_commitment: [u8; 32],
    pub next_tree_location: [u8; 32],
    pub subchain_tree_location: [u8; 32],
    pub membership_next_tree_location: [u8; 32],
}

pub struct AddLocalTeamMemberInput<'a> {
    pub actor: &'a EntityId,
    pub actor_source_role: Role,
    pub team: &'a EntityId,
    pub host: &'a EntityId,
    pub sequence: u64,
    pub previous: [u8; 32],
    pub root: &'a TreeRoot,
    pub time: u64,
    pub next_tree_location: [u8; 32],
    pub member: &'a EntityId,
    pub member_source_role: Role,
    pub member_destination_role: Role,
    pub member_generation: u64,
    pub member_public: &'a SharedPublicMaterial,
}

pub struct AddRemoteTeamMemberInput<'a> {
    pub actor: &'a EntityId,
    pub actor_source_role: Role,
    pub team: &'a EntityId,
    pub host: &'a EntityId,
    pub sequence: u64,
    pub previous: [u8; 32],
    pub root: &'a TreeRoot,
    pub time: u64,
    pub next_tree_location: [u8; 32],
    pub member: &'a EntityId,
    pub member_host: &'a EntityId,
    pub member_source_role: Role,
    pub member_destination_role: Role,
    pub member_generation: u64,
    pub member_public: &'a SharedPublicMaterial,
    pub member_index_range: Option<&'a foks_proto::RationalRange>,
}

pub struct AddLocalTeamMemberMaterial {
    pub link: UserLink,
    pub removal_key_commitment: [u8; 32],
    pub next_tree_location: [u8; 32],
}

pub struct TeamMetadataInput<'a> {
    pub actor: &'a EntityId,
    pub actor_source_role: Role,
    pub team: &'a EntityId,
    pub host: &'a EntityId,
    pub sequence: u64,
    pub previous: [u8; 32],
    pub root: &'a TreeRoot,
    pub time: u64,
    pub next_tree_location: [u8; 32],
    pub index_range: &'a foks_proto::RationalRange,
}

pub struct TeamMetadataMaterial {
    pub link: UserLink,
    pub next_tree_location: [u8; 32],
}

pub struct TeamPtkRotation<'a> {
    pub role: Role,
    pub generation: u64,
    pub seed: &'a SecretSeed,
}

pub struct RemoveLocalTeamMemberInput<'a> {
    pub actor: &'a EntityId,
    pub actor_source_role: Role,
    pub team: &'a EntityId,
    pub host: &'a EntityId,
    pub sequence: u64,
    pub previous: [u8; 32],
    pub root: &'a TreeRoot,
    pub time: u64,
    pub next_tree_location: [u8; 32],
    pub member: &'a EntityId,
    pub member_source_role: Role,
}

pub struct ChangeTeamMemberInput<'a> {
    pub actor: &'a EntityId,
    pub actor_source_role: Role,
    pub team: &'a EntityId,
    pub host: &'a EntityId,
    pub sequence: u64,
    pub previous: [u8; 32],
    pub root: &'a TreeRoot,
    pub time: u64,
    pub next_tree_location: [u8; 32],
    pub member: &'a EntityId,
    pub member_host: Option<&'a EntityId>,
    pub member_source_role: Role,
    pub destination_role: Role,
    pub member_generation: Option<u64>,
    pub member_public: Option<&'a SharedPublicMaterial>,
    pub member_index_range: Option<&'a foks_proto::RationalRange>,
}

pub struct ChangeTeamMemberEntryInput<'a> {
    pub member: &'a EntityId,
    pub member_host: Option<&'a EntityId>,
    pub member_source_role: Role,
    pub destination_role: Role,
    pub member_generation: Option<u64>,
    pub member_public: Option<&'a SharedPublicMaterial>,
    pub member_index_range: Option<&'a foks_proto::RationalRange>,
}

pub struct ChangeTeamMembersInput<'a> {
    pub actor: &'a EntityId,
    pub actor_source_role: Role,
    pub team: &'a EntityId,
    pub host: &'a EntityId,
    pub sequence: u64,
    pub previous: [u8; 32],
    pub root: &'a TreeRoot,
    pub time: u64,
    pub next_tree_location: [u8; 32],
    pub members: &'a [ChangeTeamMemberEntryInput<'a>],
}

pub struct RemoveLocalTeamMemberMaterial {
    pub link: UserLink,
    pub ptks: Vec<SharedPublicMaterial>,
    pub next_tree_location: [u8; 32],
}

/// Derives the permanent ad-hoc TeamID selected by FOKS from its admin PTK.
pub fn adhoc_team_id_from_admin_seed(seed: &SecretSeed) -> Result<EntityId> {
    let admin = derive_shared_public(seed, foks_proto::ENTITY_PTK_VERIFY)?;
    let mut team_bytes = admin.verify_key.as_bytes().to_vec();
    team_bytes[0] = foks_proto::ENTITY_AD_HOC_TEAM;
    Ok(EntityId::from_bytes(team_bytes)?)
}

/// Derives the permanent named TeamID selected by FOKS from its admin PTK.
pub fn named_team_id_from_admin_seed(seed: &SecretSeed) -> Result<EntityId> {
    let admin = derive_shared_public(seed, foks_proto::ENTITY_PTK_VERIFY)?;
    let mut team_bytes = admin.verify_key.as_bytes().to_vec();
    team_bytes[0] = foks_proto::ENTITY_NAMED_TEAM;
    Ok(EntityId::from_bytes(team_bytes)?)
}

/// Constructs the exact single-owner ad-hoc team and owner membership links.
/// PTK seeds are ordered member-min, member, admin, owner.
pub fn make_single_owner_adhoc_team(
    input: &AdHocTeamInput<'_>,
    device_seed: &SecretSeed,
    owner_puk_seed: &SecretSeed,
    ptk_seeds: [&SecretSeed; 4],
) -> Result<AdHocTeamMaterial> {
    let device = derive_device_public(device_seed)?;
    make_single_owner_adhoc_team_with_signer(
        input,
        &device.id,
        owner_puk_seed,
        ptk_seeds,
        |canonical_object| sign_seed_typed(device_seed, LINK_OUTER_V1_TYPE_ID, canonical_object),
    )
}

/// Hardware-backed variant of [`make_single_owner_adhoc_team`]. The Yubi
/// parent signs the creator's membership link; its delegated Ed25519 subkey is
/// intentionally not involved in chain signing.
pub fn make_single_owner_adhoc_team_yubi(
    input: &AdHocTeamInput<'_>,
    device: &dyn YubiDevice,
    owner_puk_seed: &SecretSeed,
    ptk_seeds: [&SecretSeed; 4],
) -> Result<AdHocTeamMaterial> {
    if device.entity_id().entity_type() != foks_proto::ENTITY_YUBI {
        return Err(Error::SignatureType);
    }
    make_single_owner_adhoc_team_with_signer(
        input,
        device.entity_id(),
        owner_puk_seed,
        ptk_seeds,
        |canonical_object| sign_yubi_typed(device, LINK_OUTER_V1_TYPE_ID, canonical_object),
    )
}

fn make_single_owner_adhoc_team_with_signer(
    input: &AdHocTeamInput<'_>,
    device_id: &EntityId,
    owner_puk_seed: &SecretSeed,
    ptk_seeds: [&SecretSeed; 4],
    sign_membership: impl FnOnce(&[u8]) -> Result<Signature>,
) -> Result<AdHocTeamMaterial> {
    if input.owner_puk_generation == 0
        || input.membership_sequence == 0
        || (input.membership_sequence == 1) != input.membership_previous.is_none()
        || input.next_tree_location == input.subchain_tree_location
        || input.next_tree_location == input.membership_next_tree_location
        || input.subchain_tree_location == input.membership_next_tree_location
    {
        return Err(Error::AdHocTeamMaterial);
    }
    let owner_puk = derive_shared_public(owner_puk_seed, foks_proto::ENTITY_PUK_VERIFY)?;
    let mut signer_bytes = owner_puk.verify_key.as_bytes().to_vec();
    signer_bytes[0] = foks_proto::ENTITY_USER;
    let signer = EntityId::from_bytes(signer_bytes)?;
    let roles = [
        Role::member(-0x4000),
        Role::member(0),
        Role::ADMIN,
        Role::OWNER,
    ];
    let ptks = ptk_seeds
        .iter()
        .map(|seed| derive_shared_public(seed, foks_proto::ENTITY_PTK_VERIFY))
        .collect::<Result<Vec<_>>>()?;
    if ptks.iter().enumerate().any(|(index, ptk)| {
        ptks[index + 1..]
            .iter()
            .any(|other| other.verify_key == ptk.verify_key)
    }) {
        return Err(Error::AdHocTeamMaterial);
    }
    let team = adhoc_team_id_from_admin_seed(ptk_seeds[2])?;
    let shared_keys = roles
        .iter()
        .zip(&ptks)
        .map(|(role, ptk)| {
            Ok(UserSharedKey {
                generation: 1,
                role: *role,
                verify_key: ptk.verify_key.clone(),
                hepk_fingerprint: hepk_fingerprint(&ptk.hepk)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let change = TeamGroupChange {
        seqno: 1,
        previous: None,
        root: input.root.clone(),
        time: input.time,
        next_location_commitment: tree_location_commitment(&input.next_tree_location)?,
        team: team.clone(),
        host: input.host.clone(),
        signer,
        signer_owner: TeamKeyOwner {
            party: input.user.clone(),
            source_role: Role::OWNER,
        },
        changes: vec![TeamMemberChange {
            role: Role::OWNER,
            party: input.user.clone(),
            scoped_host: None,
            source_role: Role::OWNER,
            keys: Some(TeamMemberKeys {
                verify_key: owner_puk.verify_key.clone(),
                hepk_fingerprint: hepk_fingerprint(&owner_puk.hepk)?,
                generation: input.owner_puk_generation,
                removal_key_commitment: None,
                index_range: None,
            }),
        }],
        shared_keys,
        metadata: vec![
            ChangeMetadata::Eldest {
                subchain_location_commitment: tree_location_commitment(
                    &input.subchain_tree_location,
                )?,
            },
            ChangeMetadata::TeamIndexRange(foks_proto::RationalRange {
                low: foks_proto::Rational {
                    infinity: false,
                    base: vec![1],
                    exponent: 0,
                },
                high: foks_proto::Rational {
                    infinity: true,
                    base: Vec::new(),
                    exponent: 0,
                },
            }),
            ChangeMetadata::MemberLoadFloor(Role::member(0)),
        ],
    };
    let unsigned = UnsignedUserLink::team_group_change(&change)?;
    let mut signatures = Vec::with_capacity(5);
    for seed in ptk_seeds {
        signatures.push(sign_seed_typed(
            seed,
            LINK_OUTER_V1_TYPE_ID,
            &unsigned.signing_bytes(&signatures)?,
        )?);
    }
    signatures.push(sign_seed_typed(
        owner_puk_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&signatures)?,
    )?);
    let link = unsigned.finish(signatures)?;

    let membership_unsigned =
        UnsignedUserLink::approved_adhoc_membership(&AdHocMembershipLinkPublic {
            user: input.user,
            host: input.host,
            signer: device_id,
            sequence: input.membership_sequence,
            previous: input.membership_previous,
            root: input.root,
            time: 0,
            next_location_commitment: tree_location_commitment(
                &input.membership_next_tree_location,
            )?,
            team: &team,
            source_role: Role::OWNER,
            destination_role: Role::OWNER,
            team_sequence: 1,
        })?;
    let membership_signature = sign_membership(&membership_unsigned.signing_bytes(&[])?)?;
    let membership_link = membership_unsigned.finish(vec![membership_signature])?;
    Ok(AdHocTeamMaterial {
        team,
        link,
        membership_link,
        ptks,
        next_tree_location: input.next_tree_location,
        subchain_tree_location: input.subchain_tree_location,
        membership_next_tree_location: input.membership_next_tree_location,
    })
}

/// Constructs a named team's eldest link and its creator's approved
/// membership link. PTK seeds are ordered member-min, member, admin, owner.
pub fn make_single_owner_named_team(
    input: &NamedTeamInput<'_>,
    device_seed: &SecretSeed,
    owner_puk_seed: &SecretSeed,
    ptk_seeds: [&SecretSeed; 4],
    removal_key: &SecretSeed,
) -> Result<NamedTeamMaterial> {
    let device = derive_device_public(device_seed)?;
    make_single_owner_named_team_with_signer(
        input,
        &device.id,
        owner_puk_seed,
        ptk_seeds,
        removal_key,
        |canonical_object| sign_seed_typed(device_seed, LINK_OUTER_V1_TYPE_ID, canonical_object),
    )
}

/// Hardware-backed variant of [`make_single_owner_named_team`].
pub fn make_single_owner_named_team_yubi(
    input: &NamedTeamInput<'_>,
    device: &dyn YubiDevice,
    owner_puk_seed: &SecretSeed,
    ptk_seeds: [&SecretSeed; 4],
    removal_key: &SecretSeed,
) -> Result<NamedTeamMaterial> {
    if device.entity_id().entity_type() != foks_proto::ENTITY_YUBI {
        return Err(Error::SignatureType);
    }
    make_single_owner_named_team_with_signer(
        input,
        device.entity_id(),
        owner_puk_seed,
        ptk_seeds,
        removal_key,
        |canonical_object| sign_yubi_typed(device, LINK_OUTER_V1_TYPE_ID, canonical_object),
    )
}

fn make_single_owner_named_team_with_signer(
    input: &NamedTeamInput<'_>,
    device_id: &EntityId,
    owner_puk_seed: &SecretSeed,
    ptk_seeds: [&SecretSeed; 4],
    removal_key: &SecretSeed,
    sign_membership: impl FnOnce(&[u8]) -> Result<Signature>,
) -> Result<NamedTeamMaterial> {
    if input.owner_puk_generation == 0
        || input.membership_sequence == 0
        || (input.membership_sequence == 1) != input.membership_previous.is_none()
        || input.normalized_name.is_empty()
        || input.name_sequence == 0
        || input.next_tree_location == input.subchain_tree_location
        || input.next_tree_location == input.membership_next_tree_location
        || input.subchain_tree_location == input.membership_next_tree_location
    {
        return Err(Error::NamedTeamMaterial);
    }
    let owner_puk = derive_shared_public(owner_puk_seed, foks_proto::ENTITY_PUK_VERIFY)?;
    let mut signer_bytes = owner_puk.verify_key.as_bytes().to_vec();
    signer_bytes[0] = foks_proto::ENTITY_USER;
    let signer = EntityId::from_bytes(signer_bytes)?;
    let roles = [
        Role::member(-0x4000),
        Role::member(0),
        Role::ADMIN,
        Role::OWNER,
    ];
    let ptks = ptk_seeds
        .iter()
        .map(|seed| derive_shared_public(seed, foks_proto::ENTITY_PTK_VERIFY))
        .collect::<Result<Vec<_>>>()?;
    if ptks.iter().enumerate().any(|(index, ptk)| {
        ptks[index + 1..]
            .iter()
            .any(|other| other.verify_key == ptk.verify_key)
    }) {
        return Err(Error::NamedTeamMaterial);
    }
    let team = named_team_id_from_admin_seed(ptk_seeds[2])?;
    let removal_key_commitment = team_removal_key_commitment(removal_key)?;
    let name_commitment = commitment(
        NAME_COMMITMENT_TYPE_ID,
        &encode(&Value::Array(vec![
            Value::Text(input.normalized_name.to_vec()),
            Value::Unsigned(input.name_sequence),
        ]))?,
        &input.team_name_commitment_key,
    );
    let shared_keys = roles
        .iter()
        .zip(&ptks)
        .map(|(role, ptk)| {
            Ok(UserSharedKey {
                generation: 1,
                role: *role,
                verify_key: ptk.verify_key.clone(),
                hepk_fingerprint: hepk_fingerprint(&ptk.hepk)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let change = TeamGroupChange {
        seqno: 1,
        previous: None,
        root: input.root.clone(),
        time: input.time,
        next_location_commitment: tree_location_commitment(&input.next_tree_location)?,
        team: team.clone(),
        host: input.host.clone(),
        signer,
        signer_owner: TeamKeyOwner {
            party: input.user.clone(),
            source_role: Role::OWNER,
        },
        changes: vec![TeamMemberChange {
            role: Role::OWNER,
            party: input.user.clone(),
            scoped_host: None,
            source_role: Role::OWNER,
            keys: Some(TeamMemberKeys {
                verify_key: owner_puk.verify_key.clone(),
                hepk_fingerprint: hepk_fingerprint(&owner_puk.hepk)?,
                generation: input.owner_puk_generation,
                removal_key_commitment: Some(removal_key_commitment),
                index_range: None,
            }),
        }],
        shared_keys,
        metadata: vec![
            ChangeMetadata::TeamName(name_commitment),
            ChangeMetadata::Eldest {
                subchain_location_commitment: tree_location_commitment(
                    &input.subchain_tree_location,
                )?,
            },
            ChangeMetadata::TeamIndexRange(foks_proto::RationalRange {
                low: foks_proto::Rational {
                    infinity: false,
                    base: vec![1],
                    exponent: 0,
                },
                high: foks_proto::Rational {
                    infinity: true,
                    base: Vec::new(),
                    exponent: 0,
                },
            }),
            ChangeMetadata::MemberLoadFloor(Role::member(0)),
        ],
    };
    let unsigned = UnsignedUserLink::team_group_change(&change)?;
    let mut signatures = Vec::with_capacity(5);
    for seed in ptk_seeds {
        signatures.push(sign_seed_typed(
            seed,
            LINK_OUTER_V1_TYPE_ID,
            &unsigned.signing_bytes(&signatures)?,
        )?);
    }
    signatures.push(sign_seed_typed(
        owner_puk_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&signatures)?,
    )?);
    let link = unsigned.finish(signatures)?;
    let membership_unsigned =
        UnsignedUserLink::approved_membership(&ApprovedMembershipLinkPublic {
            user: input.user,
            host: input.host,
            signer: device_id,
            sequence: input.membership_sequence,
            previous: input.membership_previous,
            root: input.root,
            time: 0,
            next_location_commitment: tree_location_commitment(
                &input.membership_next_tree_location,
            )?,
            team: &team,
            source_role: Role::OWNER,
            destination_role: Role::OWNER,
            team_sequence: 1,
            removal_key_commitment,
        })?;
    let membership_signature = sign_membership(&membership_unsigned.signing_bytes(&[])?)?;
    let membership_link = membership_unsigned.finish(vec![membership_signature])?;
    Ok(NamedTeamMaterial {
        team,
        link,
        membership_link,
        ptks,
        removal_key_commitment,
        next_tree_location: input.next_tree_location,
        subchain_tree_location: input.subchain_tree_location,
        membership_next_tree_location: input.membership_next_tree_location,
    })
}

/// Constructs one additive local-user named-team transition. Pure additions
/// reuse existing PTK generations; the caller separately boxes those keys and
/// the new member's removal key.
pub fn make_add_local_team_member_link(
    input: &AddLocalTeamMemberInput<'_>,
    actor_puk_seed: &SecretSeed,
    removal_key: &SecretSeed,
) -> Result<AddLocalTeamMemberMaterial> {
    input.actor.clone().require_type(foks_proto::ENTITY_USER)?;
    input.member.clone().require_type(foks_proto::ENTITY_USER)?;
    input
        .team
        .clone()
        .require_type(foks_proto::ENTITY_NAMED_TEAM)?;
    input.host.clone().require_type(foks_proto::ENTITY_HOST)?;
    if input.sequence < 2
        || input.actor_source_role == Role::NONE
        || input.member_source_role == Role::NONE
        || input.member_destination_role == Role::NONE
        || input.member_generation == 0
        || input.actor == input.member
    {
        return Err(Error::NamedTeamMaterial);
    }
    let actor_public = derive_shared_public(actor_puk_seed, foks_proto::ENTITY_PUK_VERIFY)?;
    let signer = actor_public.verify_key;
    let removal_key_commitment = team_removal_key_commitment(removal_key)?;
    let change = TeamGroupChange {
        seqno: input.sequence,
        previous: Some(input.previous),
        root: input.root.clone(),
        time: input.time,
        next_location_commitment: tree_location_commitment(&input.next_tree_location)?,
        team: input.team.clone(),
        host: input.host.clone(),
        signer,
        signer_owner: TeamKeyOwner {
            party: input.actor.clone(),
            source_role: input.actor_source_role,
        },
        changes: vec![TeamMemberChange {
            role: input.member_destination_role,
            party: input.member.clone(),
            scoped_host: None,
            source_role: input.member_source_role,
            keys: Some(TeamMemberKeys {
                verify_key: input.member_public.verify_key.clone(),
                hepk_fingerprint: hepk_fingerprint(&input.member_public.hepk)?,
                generation: input.member_generation,
                removal_key_commitment: Some(removal_key_commitment),
                index_range: None,
            }),
        }],
        shared_keys: Vec::new(),
        metadata: Vec::new(),
    };
    let unsigned = UnsignedUserLink::team_group_change(&change)?;
    let signature = sign_seed_typed(
        actor_puk_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&[])?,
    )?;
    Ok(AddLocalTeamMemberMaterial {
        link: unsigned.finish(vec![signature])?,
        removal_key_commitment,
        next_tree_location: input.next_tree_location,
    })
}

/// Constructs the signed roster transition for a remote user or team. The
/// remote host scope is part of the signed link and therefore cannot be
/// rewritten by the local server.
pub fn make_add_remote_team_member_link(
    input: &AddRemoteTeamMemberInput<'_>,
    actor_puk_seed: &SecretSeed,
    removal_key: &SecretSeed,
) -> Result<AddLocalTeamMemberMaterial> {
    input.actor.clone().require_type(foks_proto::ENTITY_USER)?;
    if !matches!(
        input.member.entity_type(),
        foks_proto::ENTITY_USER | foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM
    ) {
        return Err(Error::NamedTeamMaterial);
    }
    input
        .team
        .clone()
        .require_type(foks_proto::ENTITY_NAMED_TEAM)?;
    input.host.clone().require_type(foks_proto::ENTITY_HOST)?;
    input
        .member_host
        .clone()
        .require_type(foks_proto::ENTITY_HOST)?;
    if input.sequence < 2
        || input.actor_source_role == Role::NONE
        || input.member_source_role == Role::NONE
        || input.member_destination_role == Role::NONE
        || input.member_generation == 0
        || input.member_host == input.host
        || input.actor == input.member
        || matches!(
            input.member.entity_type(),
            foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM
        ) != input.member_index_range.is_some()
    {
        return Err(Error::NamedTeamMaterial);
    }
    let actor_public = derive_shared_public(actor_puk_seed, foks_proto::ENTITY_PUK_VERIFY)?;
    let removal_key_commitment = team_removal_key_commitment(removal_key)?;
    let change = TeamGroupChange {
        seqno: input.sequence,
        previous: Some(input.previous),
        root: input.root.clone(),
        time: input.time,
        next_location_commitment: tree_location_commitment(&input.next_tree_location)?,
        team: input.team.clone(),
        host: input.host.clone(),
        signer: actor_public.verify_key,
        signer_owner: TeamKeyOwner {
            party: input.actor.clone(),
            source_role: input.actor_source_role,
        },
        changes: vec![TeamMemberChange {
            role: input.member_destination_role,
            party: input.member.clone(),
            scoped_host: Some(input.member_host.clone()),
            source_role: input.member_source_role,
            keys: Some(TeamMemberKeys {
                verify_key: input.member_public.verify_key.clone(),
                hepk_fingerprint: hepk_fingerprint(&input.member_public.hepk)?,
                generation: input.member_generation,
                removal_key_commitment: Some(removal_key_commitment),
                index_range: input.member_index_range.cloned(),
            }),
        }],
        shared_keys: Vec::new(),
        metadata: Vec::new(),
    };
    let unsigned = UnsignedUserLink::team_group_change(&change)?;
    let signature = sign_seed_typed(
        actor_puk_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&[])?,
    )?;
    Ok(AddLocalTeamMemberMaterial {
        link: unsigned.finish(vec![signature])?,
        removal_key_commitment,
        next_tree_location: input.next_tree_location,
    })
}

/// Constructs one signed, metadata-only named-team transition. The caller must
/// derive `index_range` from the authenticated current team state; chain replay
/// enforces that the new range is a strict narrowing.
pub fn make_team_index_range_link(
    input: &TeamMetadataInput<'_>,
    actor_puk_seed: &SecretSeed,
) -> Result<TeamMetadataMaterial> {
    input.actor.clone().require_type(foks_proto::ENTITY_USER)?;
    input
        .team
        .clone()
        .require_type(foks_proto::ENTITY_NAMED_TEAM)?;
    input.host.clone().require_type(foks_proto::ENTITY_HOST)?;
    if input.sequence < 2 || input.actor_source_role == Role::NONE {
        return Err(Error::NamedTeamMaterial);
    }
    let actor_public = derive_shared_public(actor_puk_seed, foks_proto::ENTITY_PUK_VERIFY)?;
    let change = TeamGroupChange {
        seqno: input.sequence,
        previous: Some(input.previous),
        root: input.root.clone(),
        time: input.time,
        next_location_commitment: tree_location_commitment(&input.next_tree_location)?,
        team: input.team.clone(),
        host: input.host.clone(),
        signer: actor_public.verify_key,
        signer_owner: TeamKeyOwner {
            party: input.actor.clone(),
            source_role: input.actor_source_role,
        },
        changes: Vec::new(),
        shared_keys: Vec::new(),
        metadata: vec![ChangeMetadata::TeamIndexRange(input.index_range.clone())],
    };
    let unsigned = UnsignedUserLink::team_group_change(&change)?;
    let signature = sign_seed_typed(
        actor_puk_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&[])?,
    )?;
    Ok(TeamMetadataMaterial {
        link: unsigned.finish(vec![signature])?,
        next_tree_location: input.next_tree_location,
    })
}

/// Constructs and stacked-signs one local-user removal and its exact PTK
/// generation advances. The caller derives the required role set from the
/// authenticated pre-transition roster and key schedule.
pub fn make_remove_local_team_member_link(
    input: &RemoveLocalTeamMemberInput<'_>,
    actor_puk_seed: &SecretSeed,
    rotations: &[TeamPtkRotation<'_>],
) -> Result<RemoveLocalTeamMemberMaterial> {
    make_change_team_member_link(
        &ChangeTeamMemberInput {
            actor: input.actor,
            actor_source_role: input.actor_source_role,
            team: input.team,
            host: input.host,
            sequence: input.sequence,
            previous: input.previous,
            root: input.root,
            time: input.time,
            next_tree_location: input.next_tree_location,
            member: input.member,
            member_host: None,
            member_source_role: input.member_source_role,
            destination_role: Role::NONE,
            member_generation: None,
            member_public: None,
            member_index_range: None,
        },
        actor_puk_seed,
        rotations,
    )
}

/// Constructs and stacked-signs a removal, demotion, or member credential
/// generation advance together with its exact PTK rotations.
pub fn make_change_team_member_link(
    input: &ChangeTeamMemberInput<'_>,
    actor_puk_seed: &SecretSeed,
    rotations: &[TeamPtkRotation<'_>],
) -> Result<RemoveLocalTeamMemberMaterial> {
    make_change_team_members_link(
        &ChangeTeamMembersInput {
            actor: input.actor,
            actor_source_role: input.actor_source_role,
            team: input.team,
            host: input.host,
            sequence: input.sequence,
            previous: input.previous,
            root: input.root,
            time: input.time,
            next_tree_location: input.next_tree_location,
            members: &[ChangeTeamMemberEntryInput {
                member: input.member,
                member_host: input.member_host,
                member_source_role: input.member_source_role,
                destination_role: input.destination_role,
                member_generation: input.member_generation,
                member_public: input.member_public,
                member_index_range: input.member_index_range,
            }],
        },
        actor_puk_seed,
        rotations,
    )
}

/// Constructs and stacked-signs one atomic group change covering multiple
/// roster transitions and their union PTK rotation schedule.
pub fn make_change_team_members_link(
    input: &ChangeTeamMembersInput<'_>,
    actor_shared_key_seed: &SecretSeed,
    rotations: &[TeamPtkRotation<'_>],
) -> Result<RemoveLocalTeamMemberMaterial> {
    let actor_key_type = match input.actor.entity_type() {
        foks_proto::ENTITY_USER => foks_proto::ENTITY_PUK_VERIFY,
        foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM => {
            foks_proto::ENTITY_PTK_VERIFY
        }
        _ => return Err(Error::NamedTeamMaterial),
    };
    input
        .team
        .clone()
        .require_type(foks_proto::ENTITY_NAMED_TEAM)?;
    input.host.clone().require_type(foks_proto::ENTITY_HOST)?;
    if input.sequence < 2 || input.actor_source_role == Role::NONE || input.members.is_empty() {
        return Err(Error::NamedTeamMaterial);
    }
    let mut member_bindings = std::collections::BTreeSet::new();
    for member in input.members {
        if !matches!(
            member.member.entity_type(),
            foks_proto::ENTITY_USER
                | foks_proto::ENTITY_NAMED_TEAM
                | foks_proto::ENTITY_AD_HOC_TEAM
        ) || member.member_source_role == Role::NONE
            || (member.destination_role == Role::NONE)
                != (member.member_generation.is_none()
                    && member.member_public.is_none()
                    && member.member_index_range.is_none())
            || (member.destination_role != Role::NONE
                && matches!(
                    member.member.entity_type(),
                    foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM
                ) != member.member_index_range.is_some())
            || member
                .member_host
                .is_some_and(|host| host.entity_type() != foks_proto::ENTITY_HOST)
            || !member_bindings.insert((
                member.member.as_bytes().to_vec(),
                member.member_host.map(|host| host.as_bytes().to_vec()),
                member.member_source_role,
            ))
        {
            return Err(Error::NamedTeamMaterial);
        }
    }
    let actor = derive_shared_public(actor_shared_key_seed, actor_key_type)?;
    let mut prior_role = None;
    let mut verify_keys = std::collections::BTreeSet::new();
    let mut ptks = Vec::with_capacity(rotations.len());
    let mut shared_keys = Vec::with_capacity(rotations.len());
    for rotation in rotations {
        if rotation.role == Role::NONE
            || rotation.generation < 2
            || prior_role.is_some_and(|role| role >= rotation.role)
        {
            return Err(Error::NamedTeamMaterial);
        }
        let public = derive_shared_public(rotation.seed, foks_proto::ENTITY_PTK_VERIFY)?;
        if !verify_keys.insert(public.verify_key.as_bytes().to_vec()) {
            return Err(Error::NamedTeamMaterial);
        }
        shared_keys.push(UserSharedKey {
            generation: rotation.generation,
            role: rotation.role,
            verify_key: public.verify_key.clone(),
            hepk_fingerprint: hepk_fingerprint(&public.hepk)?,
        });
        ptks.push(public);
        prior_role = Some(rotation.role);
    }
    let change = TeamGroupChange {
        seqno: input.sequence,
        previous: Some(input.previous),
        root: input.root.clone(),
        time: input.time,
        next_location_commitment: tree_location_commitment(&input.next_tree_location)?,
        team: input.team.clone(),
        host: input.host.clone(),
        signer: actor.verify_key,
        signer_owner: TeamKeyOwner {
            party: input.actor.clone(),
            source_role: input.actor_source_role,
        },
        changes: input
            .members
            .iter()
            .map(|member| {
                Ok(TeamMemberChange {
                    role: member.destination_role,
                    party: member.member.clone(),
                    scoped_host: member.member_host.cloned(),
                    source_role: member.member_source_role,
                    keys: match (member.member_generation, member.member_public) {
                        (Some(generation), Some(public)) if generation > 0 => {
                            Some(TeamMemberKeys {
                                verify_key: public.verify_key.clone(),
                                hepk_fingerprint: hepk_fingerprint(&public.hepk)?,
                                generation,
                                removal_key_commitment: None,
                                index_range: member.member_index_range.cloned(),
                            })
                        }
                        (None, None) => None,
                        _ => return Err(Error::NamedTeamMaterial),
                    },
                })
            })
            .collect::<Result<Vec<_>>>()?,
        shared_keys,
        metadata: Vec::new(),
    };
    let unsigned = UnsignedUserLink::team_group_change(&change)?;
    let mut signatures = Vec::with_capacity(rotations.len() + 1);
    for rotation in rotations {
        signatures.push(sign_seed_typed(
            rotation.seed,
            LINK_OUTER_V1_TYPE_ID,
            &unsigned.signing_bytes(&signatures)?,
        )?);
    }
    signatures.push(sign_seed_typed(
        actor_shared_key_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&signatures)?,
    )?);
    Ok(RemoveLocalTeamMemberMaterial {
        link: unsigned.finish(signatures)?,
        ptks,
        next_tree_location: input.next_tree_location,
    })
}

/// Authenticates a removal against the member's committed removal key and
/// the exact Merkle root used by the edit.
pub fn make_team_removal_proof(
    removal_key: &SecretSeed,
    payload: TeamRemovalMacPayload,
) -> Result<TeamRemovalAndCommitment> {
    let encoded = Zeroizing::new(payload.encoded()?);
    let mut mac = <Hmac<Sha512_256> as Mac>::new_from_slice(removal_key.as_slice())
        .map_err(|_| Error::NamedTeamMaterial)?;
    mac.update(&TEAM_REMOVAL_MAC_PAYLOAD_TYPE_ID.to_be_bytes());
    mac.update(encoded.as_slice());
    Ok(TeamRemovalAndCommitment {
        removal: TeamRemovalProof {
            mac: mac.finalize().into_bytes().into(),
            payload,
        },
        commitment: team_removal_key_commitment(removal_key)?,
    })
}

pub struct UserMutationBase<'a> {
    pub uid: &'a EntityId,
    pub host: &'a EntityId,
    pub seqno: u64,
    pub previous: [u8; 32],
    pub root: &'a TreeRoot,
    pub time: u64,
    pub next_tree_location: [u8; 32],
}

pub struct SoftwareProvisionInput<'a> {
    pub base: UserMutationBase<'a>,
    pub role: Role,
    pub device_label: &'a foks_proto::DeviceLabel,
    pub device_name_commitment_key: [u8; 16],
}

pub struct SoftwareProvisionMaterial {
    pub link: UserLink,
    pub device: DevicePublicMaterial,
    pub introduced_puk: Option<SharedPublicMaterial>,
    pub device_name_commitment_key: [u8; 16],
    pub next_tree_location: [u8; 32],
}

pub struct YubiProvisionMaterial {
    pub link: UserLink,
    pub device: DevicePublicMaterial,
    pub subkey: DevicePublicMaterial,
    pub introduced_puk: Option<SharedPublicMaterial>,
    pub device_name_commitment_key: [u8; 16],
    pub next_tree_location: [u8; 32],
}

/// Constructs a complete software-device provision link. If the requested
/// role has no PUK yet, `introduced_puk` supplies generation 1 and signs first.
/// The new device countersigns before the existing owner device.
pub fn make_software_provision_link(
    input: &SoftwareProvisionInput<'_>,
    existing_device_seed: &SecretSeed,
    new_device_seed: &SecretSeed,
    introduced_puk: Option<&SecretSeed>,
) -> Result<SoftwareProvisionMaterial> {
    make_provision_link(
        input,
        existing_device_seed,
        foks_proto::ENTITY_DEVICE,
        foks_proto::ENTITY_DEVICE,
        new_device_seed,
        introduced_puk,
    )
}

/// Constructs a provisioning link for a hardware Yubi parent and its
/// delegated Ed25519 mTLS subkey. The new PUK (when present), subkey, parent,
/// and existing software owner sign the stack in that order.
pub fn make_yubi_provision_link(
    input: &SoftwareProvisionInput<'_>,
    existing_device_seed: &SecretSeed,
    parent: &dyn YubiDevice,
    subkey_seed: &SecretSeed,
    introduced_puk: Option<&SecretSeed>,
) -> Result<YubiProvisionMaterial> {
    if input.device_label.device_type != foks_proto::DeviceType::YubiKey
        || input.role == Role::NONE
        || parent.entity_id().entity_type() != foks_proto::ENTITY_YUBI
        || parent.hepk().p256().is_none()
    {
        return Err(Error::DeviceKey);
    }
    let existing = derive_device_public(existing_device_seed)?;
    let device = DevicePublicMaterial {
        id: parent.entity_id().clone(),
        hepk: parent.hepk().clone(),
    };
    let subkey = derive_public_material(subkey_seed, foks_proto::ENTITY_SUBKEY)?;
    let introduced = introduced_puk
        .map(|seed| derive_shared_public(seed, foks_proto::ENTITY_PUK_VERIFY))
        .transpose()?;
    let label = input.device_label;
    let label_bytes = encode(&Value::Array(vec![
        Value::Unsigned(label.device_type.protocol_value()),
        Value::Text(label.normalized_name.clone()),
        Value::Unsigned(label.serial),
    ]))?;
    let shared_keys = match introduced.as_ref() {
        Some(key) => vec![UserSharedKey {
            generation: 1,
            role: input.role,
            verify_key: key.verify_key.clone(),
            hepk_fingerprint: hepk_fingerprint(&key.hepk)?,
        }],
        None => Vec::new(),
    };
    let change = UserGroupChange {
        seqno: input.base.seqno,
        previous: Some(input.base.previous),
        root: input.base.root.clone(),
        time: input.base.time,
        next_location_commitment: tree_location_commitment(&input.base.next_tree_location)?,
        uid: input.base.uid.clone(),
        host: input.base.host.clone(),
        signer: existing.id,
        changes: vec![UserMemberChange {
            role: input.role,
            entity: device.id.clone(),
            scoped_host: None,
            source_role: Role::NONE,
            keys: UserMemberKeys::User {
                hepk_fingerprint: hepk_fingerprint(&device.hepk)?,
                subkey: Some(subkey.id.clone()),
            },
        }],
        shared_keys,
        metadata: vec![ChangeMetadata::DeviceName(commitment(
            DEVICE_LABEL_TYPE_ID,
            &label_bytes,
            &input.device_name_commitment_key,
        ))],
    };
    let unsigned = UnsignedUserLink::user_group_change(&change)?;
    let mut signatures = Vec::with_capacity(if introduced_puk.is_some() { 4 } else { 3 });
    if let Some(seed) = introduced_puk {
        signatures.push(sign_seed_typed(
            seed,
            LINK_OUTER_V1_TYPE_ID,
            &unsigned.signing_bytes(&signatures)?,
        )?);
    }
    signatures.push(sign_seed_typed(
        subkey_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&signatures)?,
    )?);
    signatures.push(sign_yubi_typed(
        parent,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&signatures)?,
    )?);
    signatures.push(sign_seed_typed(
        existing_device_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&signatures)?,
    )?);
    Ok(YubiProvisionMaterial {
        link: unsigned.finish(signatures)?,
        device,
        subkey,
        introduced_puk: introduced,
        device_name_commitment_key: input.device_name_commitment_key,
        next_tree_location: input.base.next_tree_location,
    })
}

/// Constructs the same provision link with a FOKS backup key as the new
/// member. Backup keys have a distinct EntityID type but otherwise use the
/// exact Curve25519/ML-KEM software suite.
pub fn make_backup_provision_link(
    input: &SoftwareProvisionInput<'_>,
    existing_device_seed: &SecretSeed,
    backup: &BackupKey,
    introduced_puk: Option<&SecretSeed>,
) -> Result<SoftwareProvisionMaterial> {
    let backup_seed = backup.derived_seed();
    make_provision_link(
        input,
        existing_device_seed,
        foks_proto::ENTITY_DEVICE,
        foks_proto::ENTITY_BACKUP_KEY,
        &backup_seed,
        introduced_puk,
    )
}

/// Provisions a permanent software device while an ephemeral backup key is
/// the authenticated existing member and final countersigner.
pub fn make_software_provision_link_from_backup(
    input: &SoftwareProvisionInput<'_>,
    backup: &BackupKey,
    new_device_seed: &SecretSeed,
    introduced_puk: Option<&SecretSeed>,
) -> Result<SoftwareProvisionMaterial> {
    let backup_seed = backup.derived_seed();
    make_software_provision_link_from_backup_seed(
        input,
        &backup_seed,
        new_device_seed,
        introduced_puk,
    )
}

fn make_software_provision_link_from_backup_seed(
    input: &SoftwareProvisionInput<'_>,
    backup_seed: &SecretSeed,
    new_device_seed: &SecretSeed,
    introduced_puk: Option<&SecretSeed>,
) -> Result<SoftwareProvisionMaterial> {
    make_provision_link(
        input,
        backup_seed,
        foks_proto::ENTITY_BACKUP_KEY,
        foks_proto::ENTITY_DEVICE,
        new_device_seed,
        introduced_puk,
    )
}

pub fn make_software_provision_link_from_backup_credential(
    input: &SoftwareProvisionInput<'_>,
    backup: &BackupKeyMaterial,
    new_device_seed: &SecretSeed,
    introduced_puk: Option<&SecretSeed>,
) -> Result<SoftwareProvisionMaterial> {
    make_software_provision_link_from_backup_seed(
        input,
        &backup.seed,
        new_device_seed,
        introduced_puk,
    )
}

fn key_provision_unsigned(
    input: &SoftwareProvisionInput<'_>,
    existing: &EntityId,
    device: &DevicePublicMaterial,
    introduced: Option<&SharedPublicMaterial>,
) -> Result<UnsignedUserLink> {
    let label = input.device_label;
    let label_bytes = encode(&Value::Array(vec![
        Value::Unsigned(label.device_type.protocol_value()),
        Value::Text(label.normalized_name.clone()),
        Value::Unsigned(label.serial),
    ]))?;
    let shared_keys = match introduced {
        Some(key) => {
            vec![UserSharedKey {
                generation: 1,
                role: input.role,
                verify_key: key.verify_key.clone(),
                hepk_fingerprint: hepk_fingerprint(&key.hepk)?,
            }]
        }
        None => Vec::new(),
    };
    let change = UserGroupChange {
        seqno: input.base.seqno,
        previous: Some(input.base.previous),
        root: input.base.root.clone(),
        time: input.base.time,
        next_location_commitment: tree_location_commitment(&input.base.next_tree_location)?,
        uid: input.base.uid.clone(),
        host: input.base.host.clone(),
        signer: existing.clone(),
        changes: vec![UserMemberChange {
            role: input.role,
            entity: device.id.clone(),
            scoped_host: None,
            source_role: Role::NONE,
            keys: UserMemberKeys::User {
                hepk_fingerprint: hepk_fingerprint(&device.hepk)?,
                subkey: None,
            },
        }],
        shared_keys,
        metadata: vec![ChangeMetadata::DeviceName(commitment(
            DEVICE_LABEL_TYPE_ID,
            &label_bytes,
            &input.device_name_commitment_key,
        ))],
    };
    UnsignedUserLink::user_group_change(&change).map_err(Into::into)
}

/// Software token/device provisioning shares the exact group-change construction.
pub fn make_software_key_provision_link(
    input: &SoftwareProvisionInput<'_>,
    existing_seed: &SecretSeed,
    existing_kind: SoftwareKeyKind,
    new_seed: &SecretSeed,
    new_kind: SoftwareKeyKind,
    introduced_puk: Option<&SecretSeed>,
) -> Result<SoftwareProvisionMaterial> {
    make_provision_link(
        input,
        existing_seed,
        existing_kind.entity_type(),
        new_kind.entity_type(),
        new_seed,
        introduced_puk,
    )
}

/// A hardware owner countersigns enrollment of a software bot key.
pub fn make_bot_provision_link_from_yubi(
    input: &SoftwareProvisionInput<'_>,
    parent: &dyn YubiDevice,
    bot: &BotToken,
    introduced_puk: Option<&SecretSeed>,
) -> Result<SoftwareProvisionMaterial> {
    let device = bot.public_material()?;
    let introduced = introduced_puk
        .map(|s| derive_shared_public(s, foks_proto::ENTITY_PUK_VERIFY))
        .transpose()?;
    let unsigned = key_provision_unsigned(input, parent.entity_id(), &device, introduced.as_ref())?;
    let mut signatures = Vec::new();
    if let Some(seed) = introduced_puk {
        signatures.push(sign_seed_typed(
            seed,
            LINK_OUTER_V1_TYPE_ID,
            &unsigned.signing_bytes(&signatures)?,
        )?);
    }
    signatures.push(sign_seed_typed(
        &bot.derived_seed(),
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&signatures)?,
    )?);
    signatures.push(sign_yubi_typed(
        parent,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&signatures)?,
    )?);
    Ok(SoftwareProvisionMaterial {
        link: unsigned.finish(signatures)?,
        device,
        introduced_puk: introduced,
        device_name_commitment_key: input.device_name_commitment_key,
        next_tree_location: input.base.next_tree_location,
    })
}

fn make_provision_link(
    input: &SoftwareProvisionInput<'_>,
    existing_device_seed: &SecretSeed,
    existing_entity_type: u8,
    new_entity_type: u8,
    new_device_seed: &SecretSeed,
    introduced_puk: Option<&SecretSeed>,
) -> Result<SoftwareProvisionMaterial> {
    let existing = derive_public_material(existing_device_seed, existing_entity_type)?;
    let device = derive_public_material(new_device_seed, new_entity_type)?;
    let introduced = introduced_puk
        .map(|seed| derive_shared_public(seed, foks_proto::ENTITY_PUK_VERIFY))
        .transpose()?;
    let unsigned = key_provision_unsigned(input, &existing.id, &device, introduced.as_ref())?;
    let mut signatures = Vec::new();
    if let Some(seed) = introduced_puk {
        signatures.push(sign_seed_typed(
            seed,
            LINK_OUTER_V1_TYPE_ID,
            &unsigned.signing_bytes(&signatures)?,
        )?);
    }
    signatures.push(sign_seed_typed(
        new_device_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&signatures)?,
    )?);
    signatures.push(sign_seed_typed(
        existing_device_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&signatures)?,
    )?);
    Ok(SoftwareProvisionMaterial {
        link: unsigned.finish(signatures)?,
        device,
        introduced_puk: introduced,
        device_name_commitment_key: input.device_name_commitment_key,
        next_tree_location: input.base.next_tree_location,
    })
}

pub struct PukRotation<'a> {
    pub role: Role,
    pub generation: u64,
    pub seed: &'a SecretSeed,
}

fn hybrid_key_derivation_payload(
    kem_shared: &[u8],
    dh_shared: &[u8],
    receiver_hepk: &Hepk,
    sender_dh: &DhPublicKey,
) -> Result<Zeroizing<Vec<u8>>> {
    let receiver = decode(&receiver_hepk.encoded()?)?;
    let sender = dh_public_value(sender_dh);
    Ok(Zeroizing::new(encode_ref(&ValueRef::Array(vec![
        ValueRef::Unsigned(1),
        ValueRef::Binary(kem_shared),
        ValueRef::Binary(dh_shared),
        ValueRef::from(&receiver),
        ValueRef::from(&sender),
    ]))?))
}

fn shared_key_seed_plaintext(
    receiver: &EntityId,
    host: &EntityId,
    generation: u64,
    role: Role,
    seed: &SecretSeed,
) -> Result<Zeroizing<Vec<u8>>> {
    let role = role.to_value();
    Ok(Zeroizing::new(encode_ref(&ValueRef::Array(vec![
        ValueRef::Array(vec![
            ValueRef::Binary(receiver.as_bytes()),
            ValueRef::Binary(host.as_bytes()),
        ]),
        ValueRef::Unsigned(generation),
        ValueRef::from(&role),
        ValueRef::Binary(seed.as_slice()),
    ]))?))
}

pub fn seal_puk_seed_chain_box(
    new_seed: &SecretSeed,
    previous_seed: &SecretSeed,
    party: &EntityId,
    host: &EntityId,
    generation: u64,
    role: Role,
    nonce: [u8; 16],
) -> Result<foks_proto::SeedChainBox> {
    let cleartext = shared_key_seed_plaintext(party, host, generation, role, previous_seed)?;
    let key = derive_key(new_seed, 2, None)?;
    Ok(foks_proto::SeedChainBox {
        generation,
        role,
        secret_box: SecretBox {
            nonce,
            ciphertext: seal_typed_secretbox(
                key.as_bytes(),
                SHARED_KEY_SEED_TYPE_ID,
                &nonce,
                cleartext.as_slice(),
                false,
            )?,
        },
    })
}

pub fn make_software_revoke_link(
    base: &UserMutationBase<'_>,
    signing_device_seed: &SecretSeed,
    target: &EntityId,
    rotations: &[PukRotation<'_>],
) -> Result<UserLink> {
    make_software_puk_change_link(base, signing_device_seed, Some(target), rotations)
}

/// Constructs a standalone PUK-rotation link. FOKS uses the revoke RPC for
/// this operation but authenticates an empty member-change list.
pub fn make_software_puk_rotation_link(
    base: &UserMutationBase<'_>,
    signing_device_seed: &SecretSeed,
    rotations: &[PukRotation<'_>],
) -> Result<UserLink> {
    if rotations.is_empty() {
        return Err(Error::PukBinding);
    }
    make_software_puk_change_link(base, signing_device_seed, None, rotations)
}

/// Constructs a standalone PUK-rotation link signed by an enrolled Yubi
/// parent. Each rotated PUK signs the accumulated stack first, in shared-key
/// order, and the P-256 parent signs last as required by user-chain replay.
pub fn make_yubi_puk_rotation_link(
    base: &UserMutationBase<'_>,
    parent: &dyn YubiDevice,
    rotations: &[PukRotation<'_>],
) -> Result<UserLink> {
    make_yubi_puk_change_link(base, parent, None, rotations)
}
pub fn make_yubi_revoke_link(
    base: &UserMutationBase<'_>,
    parent: &dyn YubiDevice,
    target: &EntityId,
    rotations: &[PukRotation<'_>],
) -> Result<UserLink> {
    make_yubi_puk_change_link(base, parent, Some(target), rotations)
}
fn make_yubi_puk_change_link(
    base: &UserMutationBase<'_>,
    parent: &dyn YubiDevice,
    target: Option<&EntityId>,
    rotations: &[PukRotation<'_>],
) -> Result<UserLink> {
    if rotations.is_empty() {
        return Err(Error::PukBinding);
    }
    if parent.entity_id().entity_type() != foks_proto::ENTITY_YUBI
        || parent.entity_id().p256_key().ok().as_ref() != parent.hepk().p256()
    {
        return Err(Error::DeviceKey);
    }
    let mut shared_keys = Vec::with_capacity(rotations.len());
    for rotation in rotations {
        let public = derive_shared_public(rotation.seed, foks_proto::ENTITY_PUK_VERIFY)?;
        shared_keys.push(UserSharedKey {
            generation: rotation.generation,
            role: rotation.role,
            verify_key: public.verify_key,
            hepk_fingerprint: hepk_fingerprint(&public.hepk)?,
        });
    }
    let change = UserGroupChange {
        seqno: base.seqno,
        previous: Some(base.previous),
        root: base.root.clone(),
        time: base.time,
        next_location_commitment: tree_location_commitment(&base.next_tree_location)?,
        uid: base.uid.clone(),
        host: base.host.clone(),
        signer: parent.entity_id().clone(),
        changes: target
            .map(|target| {
                vec![UserMemberChange {
                    role: Role::NONE,
                    entity: target.clone(),
                    scoped_host: None,
                    source_role: Role::NONE,
                    keys: UserMemberKeys::None,
                }]
            })
            .unwrap_or_default(),
        shared_keys,
        metadata: Vec::new(),
    };
    let unsigned = UnsignedUserLink::user_group_change(&change)?;
    let mut signatures = Vec::with_capacity(rotations.len() + 1);
    for rotation in rotations {
        signatures.push(sign_seed_typed(
            rotation.seed,
            LINK_OUTER_V1_TYPE_ID,
            &unsigned.signing_bytes(&signatures)?,
        )?);
    }
    signatures.push(sign_yubi_typed(
        parent,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&signatures)?,
    )?);
    unsigned.finish(signatures).map_err(Into::into)
}

fn make_software_puk_change_link(
    base: &UserMutationBase<'_>,
    signing_device_seed: &SecretSeed,
    target: Option<&EntityId>,
    rotations: &[PukRotation<'_>],
) -> Result<UserLink> {
    let signer = derive_device_public(signing_device_seed)?;
    let mut shared_keys = Vec::with_capacity(rotations.len());
    for rotation in rotations {
        let public = derive_shared_public(rotation.seed, foks_proto::ENTITY_PUK_VERIFY)?;
        shared_keys.push(UserSharedKey {
            generation: rotation.generation,
            role: rotation.role,
            verify_key: public.verify_key,
            hepk_fingerprint: hepk_fingerprint(&public.hepk)?,
        });
    }
    let change = UserGroupChange {
        seqno: base.seqno,
        previous: Some(base.previous),
        root: base.root.clone(),
        time: base.time,
        next_location_commitment: tree_location_commitment(&base.next_tree_location)?,
        uid: base.uid.clone(),
        host: base.host.clone(),
        signer: signer.id,
        changes: target
            .map(|target| {
                vec![UserMemberChange {
                    role: Role::NONE,
                    entity: target.clone(),
                    scoped_host: None,
                    source_role: Role::NONE,
                    keys: UserMemberKeys::None,
                }]
            })
            .unwrap_or_default(),
        shared_keys,
        metadata: Vec::new(),
    };
    let unsigned = UnsignedUserLink::user_group_change(&change)?;
    let mut signatures = Vec::with_capacity(rotations.len() + 1);
    for rotation in rotations {
        signatures.push(sign_seed_typed(
            rotation.seed,
            LINK_OUTER_V1_TYPE_ID,
            &unsigned.signing_bytes(&signatures)?,
        )?);
    }
    signatures.push(sign_seed_typed(
        signing_device_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&signatures)?,
    )?);
    unsigned.finish(signatures).map_err(Into::into)
}

/// Random values consumed once by the initial PUK hybrid box. Exposing them
/// as a value enables deterministic Go/Rust fixtures without weakening the
/// production path, which fills every field from the OS CSPRNG.
pub struct InitialPukBoxRandomness {
    pub box_id: [u8; 16],
    pub kem_message: [u8; 32],
    pub nonce: [u8; 16],
}

pub struct PukBoxRandomness {
    pub kem_message: [u8; 32],
    pub nonce: [u8; 16],
}

pub struct YubiPukBoxRandomness {
    pub ephemeral_secret: [u8; 32],
    pub kem_message: [u8; 32],
    pub nonce: [u8; 16],
    pub time: u64,
}

/// Set-level randomness for the authenticated temporary X25519 sender used
/// when a Yubi parent boxes to one or more software devices.
pub struct YubiPukBoxSetRandomness {
    pub ephemeral_secret: [u8; 32],
    pub time: u64,
}

/// Set-level P-256 sender randomness for software parents boxing to Yubi
/// recipients in an otherwise mixed X25519/P-256 device roster.
pub struct SoftwarePukBoxSetRandomness {
    pub ephemeral_secret: [u8; 32],
    pub time: u64,
}

pub struct YubiPukBoxInput<'a> {
    pub seed: &'a SecretSeed,
    pub generation: u64,
    pub role: Role,
    pub receiver: &'a DevicePublicMaterial,
}

pub struct SoftwarePukBoxInput<'a> {
    pub seed: &'a SecretSeed,
    pub generation: u64,
    pub role: Role,
    pub receiver: &'a DevicePublicMaterial,
}

pub struct SharedKeyBoxInput<'a> {
    pub seed: &'a SecretSeed,
    pub generation: u64,
    pub role: Role,
    pub receiver_id: &'a EntityId,
    /// Remote host scope for a federated recipient; local recipients are
    /// represented by `None` on the wire.
    pub receiver_host: Option<&'a EntityId>,
    pub receiver_hepk: &'a Hepk,
    pub receiver_role: Role,
    pub receiver_generation: u64,
}

/// Boxes one or more PUK generations from a software sender to software
/// devices. A single authenticated box-set ID binds the complete recipient
/// manifest; callers must supply one independent randomness pair per box.
pub fn seal_software_puk_boxes(
    host: &EntityId,
    sender_seed: &SecretSeed,
    box_id: [u8; 16],
    inputs: &[SoftwarePukBoxInput<'_>],
    randomness: &[PukBoxRandomness],
) -> Result<SharedKeyBoxSet> {
    let sender = derive_device_public(sender_seed)?;
    let generic = inputs
        .iter()
        .map(|input| SharedKeyBoxInput {
            seed: input.seed,
            generation: input.generation,
            role: input.role,
            receiver_id: &input.receiver.id,
            receiver_host: None,
            receiver_hepk: &input.receiver.hepk,
            receiver_role: Role::NONE,
            receiver_generation: 0,
        })
        .collect::<Vec<_>>();
    seal_shared_key_boxes_with_sender(
        host,
        sender_seed,
        &sender.hepk,
        box_id,
        &generic,
        randomness,
    )
}

/// Boxes PUK generations from a software sender to a mixed software/Yubi
/// roster. X25519 recipients use the enrolled sender directly; P-256
/// recipients share one signed temporary key, matching Go's box-set rules.
pub fn seal_software_puk_boxes_mixed(
    host: &EntityId,
    sender_seed: &SecretSeed,
    box_id: [u8; 16],
    inputs: &[SoftwarePukBoxInput<'_>],
    randomness: &[PukBoxRandomness],
    set_randomness: SoftwarePukBoxSetRandomness,
) -> Result<SharedKeyBoxSet> {
    let host = host.clone().require_type(foks_proto::ENTITY_HOST)?;
    if inputs.is_empty() || inputs.len() != randomness.len() {
        return Err(Error::HybridBox);
    }
    let sender = derive_device_public(sender_seed)?;
    let sender_curve = sender.hepk.curve25519().copied().ok_or(Error::HybridBox)?;
    let ephemeral = P256SecretKey::from_slice(&set_randomness.ephemeral_secret)
        .map_err(|_| Error::HybridBox)?;
    let ephemeral_public = ephemeral.public_key().to_encoded_point(true);
    let ephemeral_public: [u8; 33] = ephemeral_public
        .as_bytes()
        .try_into()
        .map_err(|_| Error::HybridBox)?;
    let temporary_dh = DhPublicKey::P256(ephemeral_public);
    let mut used_temporary = false;
    let mut boxes = Vec::with_capacity(inputs.len());
    for (input, random) in inputs.iter().zip(randomness) {
        if input.generation == 0 || input.role == Role::NONE {
            return Err(Error::WrongReceiver);
        }
        let (sender_dh, dh_shared, dh_type) = match input.receiver.hepk.classical() {
            DhPublicKey::Curve25519(receiver_dh)
                if matches!(
                    input.receiver.id.entity_type(),
                    foks_proto::ENTITY_DEVICE
                        | foks_proto::ENTITY_BACKUP_KEY
                        | foks_proto::ENTITY_BOT_TOKEN_KEY
                ) =>
            {
                (
                    DhPublicKey::Curve25519(sender_curve),
                    software_dh_shared(sender_seed, &DhPublicKey::Curve25519(*receiver_dh))?,
                    1,
                )
            }
            DhPublicKey::P256(receiver_dh)
                if input.receiver.id.entity_type() == foks_proto::ENTITY_YUBI
                    && input.receiver.id.p256_key().ok().as_ref() == input.receiver.hepk.p256() =>
            {
                used_temporary = true;
                let receiver_public =
                    P256PublicKey::from_sec1_bytes(receiver_dh).map_err(|_| Error::HybridBox)?;
                let shared =
                    p256_diffie_hellman(ephemeral.to_nonzero_scalar(), receiver_public.as_affine());
                (
                    temporary_dh.clone(),
                    Zeroizing::new(<[u8; 32]>::from(*shared.raw_secret_bytes())),
                    2,
                )
            }
            _ => return Err(Error::WrongReceiver),
        };
        let receiver_mlkem =
            ml_kem_768::EncapsulationKey::new_from_slice(input.receiver.hepk.mlkem768())
                .map_err(|_| Error::MlKem)?;
        let (kem_ciphertext, kem_shared) =
            receiver_mlkem.encapsulate_deterministic(&ml_kem::B32::from(random.kem_message));
        let derivation = hybrid_key_derivation_payload(
            kem_shared.as_slice(),
            dh_shared.as_slice(),
            &input.receiver.hepk,
            &sender_dh,
        )?;
        let mut hash = <Sha3_256 as Sha3Digest>::new();
        hash.update(HYBRID_SECRET_KEY_SHA3_PAYLOAD_TYPE_ID.to_be_bytes());
        hash.update(derivation.as_slice());
        let key = Zeroizing::new(<[u8; 32]>::from(hash.finalize()));
        let cleartext = shared_key_seed_plaintext(
            &input.receiver.id,
            &host,
            input.generation,
            input.role,
            input.seed,
        )?;
        let mut nonce = [0u8; 24];
        nonce[..8].copy_from_slice(&SHARED_KEY_SEED_TYPE_ID.to_be_bytes());
        nonce[8..].copy_from_slice(&random.nonce);
        let ciphertext = XSalsa20Poly1305::new(key.as_slice().into())
            .encrypt((&nonce).into(), cleartext.as_slice())
            .map_err(|_| Error::Decryption)?;
        boxes.push(SharedKeyBox {
            generation: input.generation,
            role: input.role,
            hybrid: HybridBox {
                kem_ciphertext: kem_ciphertext.as_slice().to_vec(),
                dh_type,
                sender_dh: None,
                nonce: random.nonce,
                ciphertext,
            },
            target: SharedKeyBoxTarget {
                entity: input.receiver.id.clone(),
                host: None,
                role: Role::NONE,
                generation: 0,
            },
        });
    }
    let temporary = if used_temporary {
        let mut temporary = foks_proto::TempDhKeySigned {
            key: temporary_dh,
            time: set_randomness.time,
            signature: Signature::Ed25519([0; 64]),
        };
        temporary.signature = sign_seed_typed(
            sender_seed,
            TEMP_DH_KEY_SIG_TEMPLATE_TYPE_ID,
            &temporary.signing_bytes(&box_id, &sender.id, &host)?,
        )?;
        Some(temporary)
    } else {
        None
    };
    SharedKeyBoxSet::new(box_id, boxes, temporary).map_err(Into::into)
}

/// Seals one PUK from an existing software device to a P-256 Yubi recipient.
/// The authenticated temporary P-256 key is required by v0.1.9 whenever the
/// sender and receiver use different classical curves.
pub fn seal_software_puk_box_to_yubi(
    host: &EntityId,
    sender_seed: &SecretSeed,
    box_id: [u8; 16],
    input: &YubiPukBoxInput<'_>,
    randomness: YubiPukBoxRandomness,
) -> Result<SharedKeyBoxSet> {
    if input.generation == 0
        || input.role == Role::NONE
        || input.receiver.id.entity_type() != foks_proto::ENTITY_YUBI
    {
        return Err(Error::WrongReceiver);
    }
    let sender = derive_device_public(sender_seed)?;
    let receiver_dh = input.receiver.hepk.p256().ok_or(Error::HybridBox)?;
    let ephemeral =
        P256SecretKey::from_slice(&randomness.ephemeral_secret).map_err(|_| Error::HybridBox)?;
    let ephemeral_public = ephemeral.public_key().to_encoded_point(true);
    let ephemeral_public: [u8; 33] = ephemeral_public
        .as_bytes()
        .try_into()
        .map_err(|_| Error::HybridBox)?;
    let receiver_public =
        P256PublicKey::from_sec1_bytes(receiver_dh).map_err(|_| Error::HybridBox)?;
    let dh_shared = p256_diffie_hellman(ephemeral.to_nonzero_scalar(), receiver_public.as_affine());
    let receiver_mlkem =
        ml_kem_768::EncapsulationKey::new_from_slice(input.receiver.hepk.mlkem768())
            .map_err(|_| Error::MlKem)?;
    let (kem_ciphertext, kem_shared) =
        receiver_mlkem.encapsulate_deterministic(&ml_kem::B32::from(randomness.kem_message));
    let sender_dh = DhPublicKey::P256(ephemeral_public);
    let derivation = hybrid_key_derivation_payload(
        kem_shared.as_slice(),
        dh_shared.raw_secret_bytes().as_slice(),
        &input.receiver.hepk,
        &sender_dh,
    )?;
    let mut hash = <Sha3_256 as Sha3Digest>::new();
    hash.update(HYBRID_SECRET_KEY_SHA3_PAYLOAD_TYPE_ID.to_be_bytes());
    hash.update(derivation.as_slice());
    let key = Zeroizing::new(<[u8; 32]>::from(hash.finalize()));
    let cleartext = shared_key_seed_plaintext(
        &input.receiver.id,
        host,
        input.generation,
        input.role,
        input.seed,
    )?;
    let mut nonce = [0u8; 24];
    nonce[..8].copy_from_slice(&SHARED_KEY_SEED_TYPE_ID.to_be_bytes());
    nonce[8..].copy_from_slice(&randomness.nonce);
    let ciphertext = XSalsa20Poly1305::new(key.as_slice().into())
        .encrypt((&nonce).into(), cleartext.as_slice())
        .map_err(|_| Error::Decryption)?;
    let mut temporary = foks_proto::TempDhKeySigned {
        key: sender_dh,
        time: randomness.time,
        signature: Signature::Ed25519([0; 64]),
    };
    temporary.signature = sign_seed_typed(
        sender_seed,
        TEMP_DH_KEY_SIG_TEMPLATE_TYPE_ID,
        &temporary.signing_bytes(&box_id, &sender.id, host)?,
    )?;
    SharedKeyBoxSet::new(
        box_id,
        vec![SharedKeyBox {
            generation: input.generation,
            role: input.role,
            hybrid: HybridBox {
                kem_ciphertext: kem_ciphertext.as_slice().to_vec(),
                dh_type: 2,
                sender_dh: None,
                nonce: randomness.nonce,
                ciphertext,
            },
            target: SharedKeyBoxTarget {
                entity: input.receiver.id.clone(),
                host: None,
                role: Role::NONE,
                generation: 0,
            },
        }],
        Some(temporary),
    )
    .map_err(Into::into)
}

/// Boxes one or more PUK generations from a Yubi parent to a mixed software
/// and Yubi recipient set. P-256 recipients use the enrolled parent directly;
/// X25519 recipients share one temporary key authenticated by that parent,
/// matching the v0.1.9 `SharedKeyBoxer` construction.
pub fn seal_yubi_puk_boxes(
    host: &EntityId,
    parent: &dyn YubiDevice,
    box_id: [u8; 16],
    inputs: &[YubiPukBoxInput<'_>],
    randomness: &[PukBoxRandomness],
    set_randomness: YubiPukBoxSetRandomness,
) -> Result<SharedKeyBoxSet> {
    let host = host.clone().require_type(foks_proto::ENTITY_HOST)?;
    if inputs.is_empty()
        || inputs.len() != randomness.len()
        || parent.entity_id().entity_type() != foks_proto::ENTITY_YUBI
        || parent.entity_id().p256_key().ok().as_ref() != parent.hepk().p256()
    {
        return Err(Error::HybridBox);
    }
    let parent_dh = parent.hepk().p256().copied().ok_or(Error::HybridBox)?;
    let ephemeral_secret = StaticSecret::from(set_randomness.ephemeral_secret);
    let ephemeral_public = X25519PublicKey::from(&ephemeral_secret);
    let temporary_dh = DhPublicKey::Curve25519(*ephemeral_public.as_bytes());
    let mut used_temporary = false;
    let mut boxes = Vec::with_capacity(inputs.len());
    for (input, random) in inputs.iter().zip(randomness) {
        if input.generation == 0
            || input.role == Role::NONE
            || !matches!(
                input.receiver.id.entity_type(),
                foks_proto::ENTITY_DEVICE
                    | foks_proto::ENTITY_YUBI
                    | foks_proto::ENTITY_BACKUP_KEY
                    | foks_proto::ENTITY_BOT_TOKEN_KEY
            )
        {
            return Err(Error::WrongReceiver);
        }
        let (sender_dh, dh_shared, dh_type) = match input.receiver.hepk.classical() {
            DhPublicKey::P256(receiver_dh)
                if input.receiver.id.entity_type() == foks_proto::ENTITY_YUBI
                    && input.receiver.id.p256_key().ok().as_ref() == input.receiver.hepk.p256() =>
            {
                (
                    DhPublicKey::P256(parent_dh),
                    parent.derive_dh_shared(&DhPublicKey::P256(*receiver_dh))?,
                    2,
                )
            }
            DhPublicKey::Curve25519(receiver_dh)
                if matches!(
                    input.receiver.id.entity_type(),
                    foks_proto::ENTITY_DEVICE
                        | foks_proto::ENTITY_BACKUP_KEY
                        | foks_proto::ENTITY_BOT_TOKEN_KEY
                ) =>
            {
                used_temporary = true;
                let raw = ephemeral_secret.diffie_hellman(&X25519PublicKey::from(*receiver_dh));
                let zero = [0u8; 16];
                let shared = hsalsa::<U10>(raw.as_bytes().into(), (&zero).into());
                (temporary_dh.clone(), Zeroizing::new(shared.into()), 1)
            }
            _ => return Err(Error::WrongReceiver),
        };
        let receiver_mlkem =
            ml_kem_768::EncapsulationKey::new_from_slice(input.receiver.hepk.mlkem768())
                .map_err(|_| Error::MlKem)?;
        let (kem_ciphertext, kem_shared) =
            receiver_mlkem.encapsulate_deterministic(&ml_kem::B32::from(random.kem_message));
        let derivation = hybrid_key_derivation_payload(
            kem_shared.as_slice(),
            dh_shared.as_slice(),
            &input.receiver.hepk,
            &sender_dh,
        )?;
        let mut hash = <Sha3_256 as Sha3Digest>::new();
        hash.update(HYBRID_SECRET_KEY_SHA3_PAYLOAD_TYPE_ID.to_be_bytes());
        hash.update(derivation.as_slice());
        let key = Zeroizing::new(<[u8; 32]>::from(hash.finalize()));
        let cleartext = shared_key_seed_plaintext(
            &input.receiver.id,
            &host,
            input.generation,
            input.role,
            input.seed,
        )?;
        let mut nonce = [0u8; 24];
        nonce[..8].copy_from_slice(&SHARED_KEY_SEED_TYPE_ID.to_be_bytes());
        nonce[8..].copy_from_slice(&random.nonce);
        let ciphertext = XSalsa20Poly1305::new(key.as_slice().into())
            .encrypt((&nonce).into(), cleartext.as_slice())
            .map_err(|_| Error::Decryption)?;
        boxes.push(SharedKeyBox {
            generation: input.generation,
            role: input.role,
            hybrid: HybridBox {
                kem_ciphertext: kem_ciphertext.as_slice().to_vec(),
                dh_type,
                sender_dh: None,
                nonce: random.nonce,
                ciphertext,
            },
            target: SharedKeyBoxTarget {
                entity: input.receiver.id.clone(),
                host: None,
                role: Role::NONE,
                generation: 0,
            },
        });
    }
    let temporary = if used_temporary {
        let mut temporary = foks_proto::TempDhKeySigned {
            key: temporary_dh,
            time: set_randomness.time,
            signature: Signature::Ecdsa(Vec::new()),
        };
        temporary.signature = sign_yubi_typed(
            parent,
            TEMP_DH_KEY_SIG_TEMPLATE_TYPE_ID,
            &temporary.signing_bytes(&box_id, parent.entity_id(), &host)?,
        )?;
        Some(temporary)
    } else {
        None
    };
    SharedKeyBoxSet::new(box_id, boxes, temporary).map_err(Into::into)
}

/// Boxes PUKs from an ephemeral backup key to newly provisioned software
/// devices during recovery.
pub fn seal_backup_puk_boxes(
    host: &EntityId,
    backup: &BackupKey,
    box_id: [u8; 16],
    inputs: &[SoftwarePukBoxInput<'_>],
    randomness: &[PukBoxRandomness],
) -> Result<SharedKeyBoxSet> {
    let sender_seed = backup.derived_seed();
    seal_backup_puk_boxes_from_seed(host, &sender_seed, box_id, inputs, randomness)
}

fn seal_backup_puk_boxes_from_seed(
    host: &EntityId,
    sender_seed: &SecretSeed,
    box_id: [u8; 16],
    inputs: &[SoftwarePukBoxInput<'_>],
    randomness: &[PukBoxRandomness],
) -> Result<SharedKeyBoxSet> {
    let sender = derive_public_material(sender_seed, foks_proto::ENTITY_BACKUP_KEY)?;
    let generic = inputs
        .iter()
        .map(|input| SharedKeyBoxInput {
            seed: input.seed,
            generation: input.generation,
            role: input.role,
            receiver_id: &input.receiver.id,
            receiver_host: None,
            receiver_hepk: &input.receiver.hepk,
            receiver_role: Role::NONE,
            receiver_generation: 0,
        })
        .collect::<Vec<_>>();
    seal_shared_key_boxes_with_sender(
        host,
        sender_seed,
        &sender.hepk,
        box_id,
        &generic,
        randomness,
    )
}

pub fn seal_backup_puk_boxes_from_credential(
    host: &EntityId,
    backup: &BackupKeyMaterial,
    box_id: [u8; 16],
    inputs: &[SoftwarePukBoxInput<'_>],
    randomness: &[PukBoxRandomness],
) -> Result<SharedKeyBoxSet> {
    seal_backup_puk_boxes_from_seed(host, &backup.seed, box_id, inputs, randomness)
}

/// Boxes shared keys from a software PUK/PTK sender to Curve25519 PUK/PTK
/// recipients. The receiver entity is the owning user or team, not its verify
/// key, matching FOKS's shared-key parcel targets.
pub fn seal_shared_key_boxes(
    host: &EntityId,
    sender_seed: &SecretSeed,
    sender_hepk: &Hepk,
    box_id: [u8; 16],
    inputs: &[SharedKeyBoxInput<'_>],
    randomness: &[PukBoxRandomness],
) -> Result<SharedKeyBoxSet> {
    if derive_device_public(sender_seed)?.hepk != *sender_hepk {
        return Err(Error::HybridBox);
    }
    seal_shared_key_boxes_with_sender(host, sender_seed, sender_hepk, box_id, inputs, randomness)
}

fn seal_shared_key_boxes_with_sender(
    host: &EntityId,
    sender_seed: &SecretSeed,
    sender_hepk: &Hepk,
    box_id: [u8; 16],
    inputs: &[SharedKeyBoxInput<'_>],
    randomness: &[PukBoxRandomness],
) -> Result<SharedKeyBoxSet> {
    if inputs.is_empty() || inputs.len() != randomness.len() {
        return Err(Error::HybridBox);
    }
    if inputs.iter().any(|input| {
        input
            .receiver_host
            .is_some_and(|host| host.entity_type() != foks_proto::ENTITY_HOST)
            || !match input.receiver_id.entity_type() {
                foks_proto::ENTITY_DEVICE
                | foks_proto::ENTITY_YUBI
                | foks_proto::ENTITY_BACKUP_KEY
                | foks_proto::ENTITY_BOT_TOKEN_KEY => {
                    input.receiver_role == Role::NONE && input.receiver_generation == 0
                }
                foks_proto::ENTITY_USER
                | foks_proto::ENTITY_NAMED_TEAM
                | foks_proto::ENTITY_AD_HOC_TEAM => {
                    input.receiver_role != Role::NONE && input.receiver_generation > 0
                }
                _ => false,
            }
    }) {
        return Err(Error::WrongReceiver);
    }
    let sender_dh = sender_hepk.curve25519().copied().ok_or(Error::HybridBox)?;
    let mut boxes = Vec::with_capacity(inputs.len());
    for (input, random) in inputs.iter().zip(randomness) {
        let receiver_mlkem =
            ml_kem_768::EncapsulationKey::new_from_slice(input.receiver_hepk.mlkem768())
                .map_err(|_| Error::MlKem)?;
        let (kem_ciphertext, kem_shared) =
            receiver_mlkem.encapsulate_deterministic(&ml_kem::B32::from(random.kem_message));
        let receiver_dh = input
            .receiver_hepk
            .curve25519()
            .copied()
            .ok_or(Error::HybridBox)?;
        let dh_shared = software_dh_shared(sender_seed, &DhPublicKey::Curve25519(receiver_dh))?;
        let payload = hybrid_key_derivation_payload(
            kem_shared.as_slice(),
            dh_shared.as_slice(),
            input.receiver_hepk,
            &DhPublicKey::Curve25519(sender_dh),
        )?;
        let mut hash = <Sha3_256 as Sha3Digest>::new();
        hash.update(HYBRID_SECRET_KEY_SHA3_PAYLOAD_TYPE_ID.to_be_bytes());
        hash.update(payload.as_slice());
        let key = Zeroizing::new(<[u8; 32]>::from(hash.finalize()));
        let receiver_host = input.receiver_host.unwrap_or(host);
        let cleartext = shared_key_seed_plaintext(
            input.receiver_id,
            receiver_host,
            input.generation,
            input.role,
            input.seed,
        )?;
        let mut nonce = [0u8; 24];
        nonce[..8].copy_from_slice(&SHARED_KEY_SEED_TYPE_ID.to_be_bytes());
        nonce[8..].copy_from_slice(&random.nonce);
        let ciphertext = XSalsa20Poly1305::new(key.as_slice().into())
            .encrypt((&nonce).into(), cleartext.as_slice())
            .map_err(|_| Error::Decryption)?;
        boxes.push(SharedKeyBox {
            generation: input.generation,
            role: input.role,
            hybrid: HybridBox {
                kem_ciphertext: kem_ciphertext.as_slice().to_vec(),
                dh_type: 1,
                sender_dh: None,
                nonce: random.nonce,
                ciphertext,
            },
            target: SharedKeyBoxTarget {
                entity: input.receiver_id.clone(),
                host: input.receiver_host.cloned(),
                role: input.receiver_role,
                generation: input.receiver_generation,
            },
        });
    }
    SharedKeyBoxSet::new(box_id, boxes, None).map_err(Into::into)
}

/// Boxes one team-removal key both to the team's admin PTK and to the member's
/// PUK/PTK. The same authenticated metadata is included in each independent
/// hybrid box.
#[allow(clippy::too_many_arguments)]
pub fn seal_team_removal_key(
    sender_seed: &SecretSeed,
    sender_hepk: &Hepk,
    team_receiver_hepk: &Hepk,
    team_receiver_role: Role,
    team_receiver_generation: u64,
    member_receiver_hepk: &Hepk,
    member_receiver_role: Role,
    member_receiver_generation: u64,
    removal_key: &SecretSeed,
    metadata: TeamRemovalKeyMetadata,
    randomness: [PukBoxRandomness; 2],
) -> Result<TeamRemovalBoxData> {
    if team_receiver_generation == 0 || member_receiver_generation == 0 {
        return Err(Error::NamedTeamMaterial);
    }
    let metadata_bytes = metadata.encoded()?;
    let mut payload = Zeroizing::new(Vec::with_capacity(35 + metadata_bytes.len()));
    payload.extend_from_slice(&[0x92, 0xc4, 32]);
    payload.extend_from_slice(removal_key.as_bytes());
    payload.extend_from_slice(&metadata_bytes);
    let team_box = TeamRemovalKeyBox {
        hybrid: seal_hybrid_payload(
            sender_seed,
            sender_hepk,
            team_receiver_hepk,
            TEAM_REMOVAL_KEY_BOX_PAYLOAD_TYPE_ID,
            payload.as_slice(),
            &randomness[0],
            true,
        )?,
        role: team_receiver_role,
        generation: team_receiver_generation,
    };
    let member_box = TeamRemovalKeyBox {
        hybrid: seal_hybrid_payload(
            sender_seed,
            sender_hepk,
            member_receiver_hepk,
            TEAM_REMOVAL_KEY_BOX_PAYLOAD_TYPE_ID,
            payload.as_slice(),
            &randomness[1],
            true,
        )?,
        role: member_receiver_role,
        generation: member_receiver_generation,
    };
    let commitment = team_removal_key_commitment(removal_key)?;
    Ok(TeamRemovalBoxData {
        commitment,
        team_box,
        member_box,
        metadata,
    })
}

/// Opens either side of a team-removal key box and binds the plaintext to its
/// authenticated chain commitment and exact metadata.
pub fn open_team_removal_key(
    boxed: &TeamRemovalKeyBox,
    receiver: &dyn HybridSecretDecapsulator,
    expected_role: Role,
    expected_generation: u64,
    expected_commitment: &[u8; 32],
    expected_metadata: &TeamRemovalKeyMetadata,
) -> Result<SecretSeed> {
    if boxed.role != expected_role || boxed.generation != expected_generation {
        return Err(Error::PukBinding);
    }
    let sender_dh = boxed.hybrid.sender_dh.as_ref().ok_or(Error::HybridBox)?;
    let cleartext = open_hybrid_box(
        &boxed.hybrid,
        receiver,
        sender_dh,
        TEAM_REMOVAL_KEY_BOX_PAYLOAD_TYPE_ID,
    )?;
    let payload = TeamRemovalKeyPayload::decode(&cleartext)?;
    if payload.metadata != *expected_metadata {
        return Err(Error::PukBinding);
    }
    let key = payload.into_key();
    let commitment = team_removal_key_commitment(&key)?;
    if &commitment != expected_commitment {
        return Err(Error::PukBinding);
    }
    Ok(key)
}

/// Opens an administrator copy of a member removal key when the original
/// destination role and addition sequence are known only inside the box.
/// The stable member identity fields and the chain commitment remain exact,
/// while authenticated destination metadata is returned to the caller.
pub struct TeamRemovalKeyExpectation<'a> {
    pub commitment: &'a [u8; 32],
    pub team: &'a EntityId,
    pub host: &'a EntityId,
    pub member: &'a EntityId,
    pub member_host: &'a EntityId,
    pub source_role: Role,
}

pub fn open_team_removal_key_for_member(
    boxed: &TeamRemovalKeyBox,
    receiver: &dyn HybridSecretDecapsulator,
    expected: &TeamRemovalKeyExpectation<'_>,
) -> Result<(SecretSeed, TeamRemovalKeyMetadata)> {
    let sender_dh = boxed.hybrid.sender_dh.as_ref().ok_or(Error::HybridBox)?;
    let cleartext = open_hybrid_box(
        &boxed.hybrid,
        receiver,
        sender_dh,
        TEAM_REMOVAL_KEY_BOX_PAYLOAD_TYPE_ID,
    )?;
    let payload = TeamRemovalKeyPayload::decode(&cleartext)?;
    if payload.metadata.team != *expected.team
        || payload.metadata.host != *expected.host
        || payload.metadata.member != *expected.member
        || payload.metadata.member_host != *expected.member_host
        || payload.metadata.source_role != expected.source_role
    {
        return Err(Error::PukBinding);
    }
    let metadata = payload.metadata.clone();
    let key = payload.into_key();
    if &team_removal_key_commitment(&key)? != expected.commitment {
        return Err(Error::PukBinding);
    }
    Ok((key, metadata))
}

fn seal_hybrid_payload(
    sender_seed: &SecretSeed,
    sender_hepk: &Hepk,
    receiver_hepk: &Hepk,
    payload_type_id: u64,
    cleartext: &[u8],
    randomness: &PukBoxRandomness,
    include_sender: bool,
) -> Result<HybridBox> {
    if derive_device_public(sender_seed)?.hepk != *sender_hepk {
        return Err(Error::HybridBox);
    }
    let sender_dh = sender_hepk.curve25519().copied().ok_or(Error::HybridBox)?;
    let receiver_mlkem = ml_kem_768::EncapsulationKey::new_from_slice(receiver_hepk.mlkem768())
        .map_err(|_| Error::MlKem)?;
    let (kem_ciphertext, kem_shared) =
        receiver_mlkem.encapsulate_deterministic(&ml_kem::B32::from(randomness.kem_message));
    let receiver_dh = receiver_hepk
        .curve25519()
        .copied()
        .ok_or(Error::HybridBox)?;
    let dh_shared = software_dh_shared(sender_seed, &DhPublicKey::Curve25519(receiver_dh))?;
    let derivation = hybrid_key_derivation_payload(
        kem_shared.as_slice(),
        dh_shared.as_slice(),
        receiver_hepk,
        &DhPublicKey::Curve25519(sender_dh),
    )?;
    let mut hash = <Sha3_256 as Sha3Digest>::new();
    hash.update(HYBRID_SECRET_KEY_SHA3_PAYLOAD_TYPE_ID.to_be_bytes());
    hash.update(derivation.as_slice());
    let key = Zeroizing::new(<[u8; 32]>::from(hash.finalize()));
    let mut nonce = [0u8; 24];
    nonce[..8].copy_from_slice(&payload_type_id.to_be_bytes());
    nonce[8..].copy_from_slice(&randomness.nonce);
    let ciphertext = XSalsa20Poly1305::new(key.as_slice().into())
        .encrypt((&nonce).into(), cleartext)
        .map_err(|_| Error::Decryption)?;
    Ok(HybridBox {
        kem_ciphertext: kem_ciphertext.as_slice().to_vec(),
        dh_type: 1,
        sender_dh: include_sender.then_some(DhPublicKey::Curve25519(sender_dh)),
        nonce: randomness.nonce,
        ciphertext,
    })
}

/// Caller-controlled inputs that are intentionally retained after signup.
/// The two seeds and permission token are not part of this structure and must
/// already be durable in an encrypted credential store before submission.
pub struct SoftwareEldestInput<'a> {
    pub host: &'a EntityId,
    pub root: &'a TreeRoot,
    pub time: u64,
    pub next_tree_location: [u8; 32],
    pub subchain_tree_location: [u8; 32],
    pub normalized_username: &'a [u8],
    pub username_sequence: u64,
    pub username_commitment_key: [u8; 16],
    pub device_name: &'a DeviceLabelNameAndCommitmentKey,
}

/// Constructs and stacked-signs the exact v0.1.9 eldest link for one software
/// owner device and its first owner PUK.
pub fn make_software_eldest_link(
    input: &SoftwareEldestInput<'_>,
    device_seed: &SecretSeed,
    puk_seed: &SecretSeed,
) -> Result<SoftwareEldestMaterial> {
    if input.normalized_username.is_empty()
        || !input.normalized_username.is_ascii()
        || input.username_sequence == 0
    {
        return Err(Error::DeviceKey);
    }
    let host = input.host.clone().require_type(foks_proto::ENTITY_HOST)?;
    let device = derive_device_public(device_seed)?;
    let puk = derive_shared_public(puk_seed, foks_proto::ENTITY_PUK_VERIFY)?;
    let mut uid_bytes = puk.verify_key.as_bytes().to_vec();
    uid_bytes[0] = foks_proto::ENTITY_USER;
    let uid = EntityId::from_bytes(uid_bytes)?;

    let username_object = encode(&Value::Array(vec![
        Value::Text(input.normalized_username.to_vec()),
        Value::Unsigned(input.username_sequence),
    ]))?;
    let label = &input.device_name.label;
    let device_label_object = encode(&Value::Array(vec![
        Value::Unsigned(label.device_type.protocol_value()),
        Value::Text(label.normalized_name.clone()),
        Value::Unsigned(label.serial),
    ]))?;
    let device_hepk_fingerprint = hepk_fingerprint(&device.hepk)?;
    let puk_hepk_fingerprint = hepk_fingerprint(&puk.hepk)?;
    let public = SoftwareEldestPublic {
        host: &host,
        uid: &uid,
        device: &device.id,
        device_hepk_fingerprint,
        puk_verify_key: &puk.verify_key,
        puk_hepk_fingerprint,
        root: input.root,
        time: input.time,
        next_location_commitment: tree_location_commitment(&input.next_tree_location)?,
        username_commitment: commitment(
            NAME_COMMITMENT_TYPE_ID,
            &username_object,
            &input.username_commitment_key,
        ),
        device_name_commitment: commitment(
            DEVICE_LABEL_TYPE_ID,
            &device_label_object,
            &input.device_name.commitment_key,
        ),
        subchain_location_commitment: tree_location_commitment(&input.subchain_tree_location)?,
    };
    let unsigned = UnsignedUserLink::software_eldest(&public)?;
    let puk_signature = sign_seed_typed(
        puk_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&[])?,
    )?;
    let device_signature = sign_seed_typed(
        device_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(std::slice::from_ref(&puk_signature))?,
    )?;
    let link = unsigned.finish(vec![puk_signature, device_signature])?;
    Ok(SoftwareEldestMaterial {
        uid,
        device,
        puk,
        link,
    })
}

/// Constructs the exact v0.1.9 eldest stack for a Yubi parent: owner PUK,
/// delegated Ed25519 subkey, then the P-256 parent signature.
pub fn make_yubi_eldest_link(
    input: &SoftwareEldestInput<'_>,
    device: &dyn YubiDevice,
    subkey_seed: &SecretSeed,
    puk_seed: &SecretSeed,
) -> Result<YubiEldestMaterial> {
    if input.normalized_username.is_empty()
        || !input.normalized_username.is_ascii()
        || input.username_sequence == 0
        || device.entity_id().entity_type() != foks_proto::ENTITY_YUBI
        || device.hepk().p256().is_none()
    {
        return Err(Error::DeviceKey);
    }
    let host = input.host.clone().require_type(foks_proto::ENTITY_HOST)?;
    let parent = DevicePublicMaterial {
        id: device.entity_id().clone(),
        hepk: device.hepk().clone(),
    };
    let subkey = derive_public_material(subkey_seed, foks_proto::ENTITY_SUBKEY)?;
    let puk = derive_shared_public(puk_seed, foks_proto::ENTITY_PUK_VERIFY)?;
    let mut uid_bytes = puk.verify_key.as_bytes().to_vec();
    uid_bytes[0] = foks_proto::ENTITY_USER;
    let uid = EntityId::from_bytes(uid_bytes)?;
    let username_object = encode(&Value::Array(vec![
        Value::Text(input.normalized_username.to_vec()),
        Value::Unsigned(input.username_sequence),
    ]))?;
    let label = &input.device_name.label;
    let device_label_object = encode(&Value::Array(vec![
        Value::Unsigned(label.device_type.protocol_value()),
        Value::Text(label.normalized_name.clone()),
        Value::Unsigned(label.serial),
    ]))?;
    let unsigned = UnsignedUserLink::yubi_eldest(&YubiEldestPublic {
        host: &host,
        uid: &uid,
        device: &parent.id,
        subkey: &subkey.id,
        device_hepk_fingerprint: hepk_fingerprint(&parent.hepk)?,
        puk_verify_key: &puk.verify_key,
        puk_hepk_fingerprint: hepk_fingerprint(&puk.hepk)?,
        root: input.root,
        time: input.time,
        next_location_commitment: tree_location_commitment(&input.next_tree_location)?,
        username_commitment: commitment(
            NAME_COMMITMENT_TYPE_ID,
            &username_object,
            &input.username_commitment_key,
        ),
        device_name_commitment: commitment(
            DEVICE_LABEL_TYPE_ID,
            &device_label_object,
            &input.device_name.commitment_key,
        ),
        subchain_location_commitment: tree_location_commitment(&input.subchain_tree_location)?,
    })?;
    let puk_signature = sign_seed_typed(
        puk_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&[])?,
    )?;
    let subkey_signature = sign_seed_typed(
        subkey_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(std::slice::from_ref(&puk_signature))?,
    )?;
    let prefix = [puk_signature, subkey_signature];
    let parent_signature = sign_yubi_typed(
        device,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&prefix)?,
    )?;
    Ok(YubiEldestMaterial {
        uid,
        device: parent,
        subkey,
        puk,
        link: unsigned.finish(vec![prefix[0].clone(), prefix[1].clone(), parent_signature])?,
    })
}

pub fn derive_shared_public(seed: &SecretSeed, entity_type: u8) -> Result<SharedPublicMaterial> {
    let device = derive_public_material(seed, entity_type)?;
    Ok(SharedPublicMaterial {
        verify_key: device.id,
        hepk: device.hepk,
    })
}

pub fn hepk_fingerprint(hepk: &Hepk) -> Result<[u8; 32]> {
    prefixed_hash_signable(HEPK_TYPE_ID, &hepk.encoded()?)
}

/// Seals the first owner PUK to its software eldest device using the exact
/// X25519 + ML-KEM-768 + XSalsa20-Poly1305 v0.1.9 construction.
pub fn seal_initial_puk_box(
    host: &EntityId,
    device_seed: &SecretSeed,
    puk_seed: &SecretSeed,
    randomness: InitialPukBoxRandomness,
) -> Result<SharedKeyBoxSet> {
    let host = host.clone().require_type(foks_proto::ENTITY_HOST)?;
    let device = derive_device_public(device_seed)?;
    let receiver_mlkem = ml_kem_768::EncapsulationKey::new_from_slice(device.hepk.mlkem768())
        .map_err(|_| Error::MlKem)?;
    let kem_message = ml_kem::B32::from(randomness.kem_message);
    let (kem_ciphertext, kem_shared) = receiver_mlkem.encapsulate_deterministic(&kem_message);
    let sender_dh = device.hepk.curve25519().copied().ok_or(Error::HybridBox)?;
    let dh_shared = software_dh_shared(device_seed, &DhPublicKey::Curve25519(sender_dh))?;
    let payload = hybrid_key_derivation_payload(
        kem_shared.as_slice(),
        dh_shared.as_slice(),
        &device.hepk,
        &DhPublicKey::Curve25519(sender_dh),
    )?;
    let mut hash = <Sha3_256 as Sha3Digest>::new();
    hash.update(HYBRID_SECRET_KEY_SHA3_PAYLOAD_TYPE_ID.to_be_bytes());
    hash.update(payload.as_slice());
    let key = Zeroizing::new(<[u8; 32]>::from(hash.finalize()));
    let cleartext = shared_key_seed_plaintext(&device.id, &host, 1, Role::OWNER, puk_seed)?;
    let mut nonce = [0u8; 24];
    nonce[..8].copy_from_slice(&SHARED_KEY_SEED_TYPE_ID.to_be_bytes());
    nonce[8..].copy_from_slice(&randomness.nonce);
    let cipher = XSalsa20Poly1305::new(key.as_slice().into());
    let ciphertext = cipher
        .encrypt((&nonce).into(), cleartext.as_slice())
        .map_err(|_| Error::Decryption)?;
    SharedKeyBoxSet::software_initial(
        randomness.box_id,
        device.id,
        HybridBox {
            kem_ciphertext: kem_ciphertext.as_slice().to_vec(),
            dh_type: 1,
            sender_dh: None,
            nonce: randomness.nonce,
            ciphertext,
        },
    )
    .map_err(Into::into)
}

/// Seals the initial owner PUK to a Yubi parent using its P-256 self-DH and
/// host-derived ML-KEM key. No parent private key leaves the provider.
pub fn seal_initial_yubi_puk_box(
    host: &EntityId,
    parent: &dyn YubiDevice,
    puk_seed: &SecretSeed,
    randomness: InitialPukBoxRandomness,
) -> Result<SharedKeyBoxSet> {
    let host = host.clone().require_type(foks_proto::ENTITY_HOST)?;
    if parent.entity_id().entity_type() != foks_proto::ENTITY_YUBI {
        return Err(Error::WrongReceiver);
    }
    let receiver_mlkem = ml_kem_768::EncapsulationKey::new_from_slice(parent.hepk().mlkem768())
        .map_err(|_| Error::MlKem)?;
    let (kem_ciphertext, kem_shared) =
        receiver_mlkem.encapsulate_deterministic(&ml_kem::B32::from(randomness.kem_message));
    let sender_dh = parent.hepk().p256().copied().ok_or(Error::HybridBox)?;
    let dh_shared = parent.derive_dh_shared(&DhPublicKey::P256(sender_dh))?;
    let payload = hybrid_key_derivation_payload(
        kem_shared.as_slice(),
        dh_shared.as_slice(),
        parent.hepk(),
        &DhPublicKey::P256(sender_dh),
    )?;
    let mut hash = <Sha3_256 as Sha3Digest>::new();
    hash.update(HYBRID_SECRET_KEY_SHA3_PAYLOAD_TYPE_ID.to_be_bytes());
    hash.update(payload.as_slice());
    let key = Zeroizing::new(<[u8; 32]>::from(hash.finalize()));
    let cleartext = shared_key_seed_plaintext(parent.entity_id(), &host, 1, Role::OWNER, puk_seed)?;
    let mut nonce = [0u8; 24];
    nonce[..8].copy_from_slice(&SHARED_KEY_SEED_TYPE_ID.to_be_bytes());
    nonce[8..].copy_from_slice(&randomness.nonce);
    let ciphertext = XSalsa20Poly1305::new(key.as_slice().into())
        .encrypt((&nonce).into(), cleartext.as_slice())
        .map_err(|_| Error::Decryption)?;
    SharedKeyBoxSet::new(
        randomness.box_id,
        vec![SharedKeyBox {
            generation: 1,
            role: Role::OWNER,
            hybrid: HybridBox {
                kem_ciphertext: kem_ciphertext.as_slice().to_vec(),
                dh_type: 2,
                sender_dh: None,
                nonce: randomness.nonce,
                ciphertext,
            },
            target: SharedKeyBoxTarget {
                entity: parent.entity_id().clone(),
                host: None,
                role: Role::NONE,
                generation: 0,
            },
        }],
        None,
    )
    .map_err(Into::into)
}

/// Public material deterministically derived by FOKS v0.1.9 from a device's
/// 32-byte master seed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DevicePublicMaterial {
    pub id: EntityId,
    pub hepk: Hepk,
}

/// Hardware boundary required to decrypt FOKS hybrid boxes.
///
/// A v0.1.9 Yubi implementation retains its P-256 private keys on-card. Its
/// ML-KEM key is deterministically derived in host memory from a P-256
/// self-DH secret; only the derived shared secrets cross this interface.
pub trait HybridSecretDecapsulator {
    fn entity_id(&self) -> &EntityId;
    fn hepk(&self) -> &Hepk;
    fn derive_dh_shared(&self, peer: &DhPublicKey) -> Result<Zeroizing<[u8; 32]>>;
    fn decapsulate_mlkem768(&self, ciphertext: &[u8]) -> Result<Zeroizing<[u8; 32]>>;
}

/// Complete hardware boundary for a FOKS v0.1.9 Yubi credential.
///
/// `sign_sha512_256` receives the already hashed 32-byte FOKS signature
/// payload and returns an ASN.1 DER P-256 ECDSA signature. Implementations
/// must keep the P-256 private key inside the hardware provider. The v0.1.9
/// compatibility construction necessarily materializes derived ML-KEM state
/// in the process and must not be described as hardware-native PQ security.
pub trait YubiDevice: HybridSecretDecapsulator {
    fn pq_key_id(&self) -> [u8; 32];
    fn sign_sha512_256(&self, digest: &[u8; 32]) -> Result<Vec<u8>>;
}

/// Public v0.1.9 material for one two-slot Yubi credential.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct YubiPublicMaterial {
    pub device: DevicePublicMaterial,
    pub pq_key_id: [u8; 32],
}

/// Derives the v0.1.9 `YubiPQKeyID`, a SHA-512/256 typed hash of the
/// compressed public key in the second PIV slot.
pub fn yubi_pq_key_id(compressed_public_key: &[u8; 33]) -> Result<[u8; 32]> {
    let encoded = encode(&Value::Binary(compressed_public_key.to_vec()))?;
    prefixed_hash_signable(foks_proto::ECDSA_COMPRESSED_PUBLIC_KEY_TYPE_ID, &encoded)
}

/// Builds the exact P-256 + ML-KEM HEPK used by v0.1.9. `pq_self_secret` is
/// the raw 32-byte x-coordinate produced by ECDH of the PQ slot with itself.
pub fn derive_yubi_public_material(
    signing_public_key: [u8; 33],
    pq_public_key: [u8; 33],
    pq_self_secret: [u8; 32],
) -> Result<YubiPublicMaterial> {
    P256VerifyingKey::from_sec1_bytes(&signing_public_key).map_err(|_| Error::PublicKey)?;
    P256VerifyingKey::from_sec1_bytes(&pq_public_key).map_err(|_| Error::PublicKey)?;
    let mut id = Vec::with_capacity(34);
    id.push(foks_proto::ENTITY_YUBI);
    id.extend_from_slice(&signing_public_key);
    let id = EntityId::from_bytes(id)?;
    let seed = SecretSeed::new(pq_self_secret);
    let mlkem_seed = Zeroizing::new(derive_mlkem_seed(&seed)?);
    let kem_seed = ml_kem::Seed::try_from(mlkem_seed.as_slice()).map_err(|_| Error::DeviceKey)?;
    let decapsulation = ml_kem_768::DecapsulationKey::from_seed(kem_seed);
    let hepk = Hepk::yubi(
        signing_public_key,
        decapsulation.encapsulation_key().to_bytes().to_vec(),
    )?;
    Ok(YubiPublicMaterial {
        device: DevicePublicMaterial { id, hepk },
        pq_key_id: yubi_pq_key_id(&pq_public_key)?,
    })
}

/// Decapsulates the host-derived ML-KEM key for a v0.1.9 Yubi credential.
pub fn yubi_mlkem_decapsulate(
    pq_self_secret: [u8; 32],
    ciphertext: &[u8],
) -> Result<Zeroizing<[u8; 32]>> {
    software_mlkem_decapsulate(&SecretSeed::new(pq_self_secret), ciphertext)
}

/// Caller-controlled randomness for a Yubi subkey recovery box.
pub struct YubiSubkeyBoxRandomness {
    pub kem_message: [u8; 32],
    pub nonce: [u8; 16],
}

/// Seals the delegated Ed25519 subkey to the Yubi parent. The parent performs
/// P-256 self-DH while ML-KEM encapsulation uses only public material.
pub fn seal_yubi_subkey_box(
    parent: &dyn YubiDevice,
    subkey_seed: &SecretSeed,
    randomness: YubiSubkeyBoxRandomness,
) -> Result<HybridBox> {
    let subkey = derive_subkey_id(subkey_seed)?;
    let receiver_mlkem = ml_kem_768::EncapsulationKey::new_from_slice(parent.hepk().mlkem768())
        .map_err(|_| Error::MlKem)?;
    let (kem_ciphertext, kem_shared) =
        receiver_mlkem.encapsulate_deterministic(&ml_kem::B32::from(randomness.kem_message));
    let sender = parent.hepk().p256().copied().ok_or(Error::HybridBox)?;
    let dh_shared = parent.derive_dh_shared(&DhPublicKey::P256(sender))?;
    let payload = hybrid_key_derivation_payload(
        kem_shared.as_slice(),
        dh_shared.as_slice(),
        parent.hepk(),
        &DhPublicKey::P256(sender),
    )?;
    let mut hash = <Sha3_256 as Sha3Digest>::new();
    hash.update(HYBRID_SECRET_KEY_SHA3_PAYLOAD_TYPE_ID.to_be_bytes());
    hash.update(payload.as_slice());
    let key = Zeroizing::new(<[u8; 32]>::from(hash.finalize()));
    let cleartext = Zeroizing::new(encode(&Value::Array(vec![
        Value::Binary(parent.entity_id().as_bytes().to_vec()),
        Value::Binary(subkey.as_bytes().to_vec()),
        Value::Binary(subkey_seed.as_slice().to_vec()),
    ]))?);
    let mut nonce = [0u8; 24];
    nonce[..8].copy_from_slice(&SUBKEY_SEED_TYPE_ID.to_be_bytes());
    nonce[8..].copy_from_slice(&randomness.nonce);
    let ciphertext = XSalsa20Poly1305::new(key.as_slice().into())
        .encrypt((&nonce).into(), cleartext.as_slice())
        .map_err(|_| Error::Decryption)?;
    Ok(HybridBox {
        kem_ciphertext: kem_ciphertext.as_slice().to_vec(),
        dh_type: 2,
        sender_dh: None,
        nonce: randomness.nonce,
        ciphertext,
    })
}

/// Encrypts a Yubi PIV management key under the current FOKS PUK. The
/// generation and role travel beside the box and must be checked by callers
/// against the PUK used here.
pub fn seal_yubi_management_key(
    puk_seed: &SecretSeed,
    payload: &foks_proto::YubiManagementKeyBoxPayload,
    nonce: [u8; 16],
) -> Result<SecretBox> {
    let key = derive_key(puk_seed, 2, None)?;
    let cleartext = Zeroizing::new(payload.encoded()?);
    Ok(SecretBox {
        nonce,
        ciphertext: seal_typed_secretbox(
            key.as_bytes(),
            foks_proto::YUBI_MANAGEMENT_KEY_BOX_PAYLOAD_TYPE_ID,
            &nonce,
            cleartext.as_slice(),
            false,
        )?,
    })
}

pub fn open_yubi_management_key(
    puk_seed: &SecretSeed,
    boxed: &SecretBox,
) -> Result<foks_proto::YubiManagementKeyBoxPayload> {
    let key = derive_key(puk_seed, 2, None)?;
    let cleartext = open_typed_secretbox(
        key.as_bytes(),
        foks_proto::YUBI_MANAGEMENT_KEY_BOX_PAYLOAD_TYPE_ID,
        &boxed.nonce,
        &boxed.ciphertext,
    )?;
    foks_proto::YubiManagementKeyBoxPayload::decode(&cleartext).map_err(Into::into)
}

/// Seals a federation bearer token to the target team's member-load-floor
/// PTK using the exact v0.1.9 typed SecretBox construction.
pub fn seal_team_remote_member_view_token(
    ptk_seed: &SecretSeed,
    payload: &foks_proto::TeamRemoteMemberViewTokenBoxPayload,
    nonce: [u8; 16],
) -> Result<SecretBox> {
    let key = derive_key(ptk_seed, 2, None)?;
    let cleartext = Zeroizing::new(payload.encoded()?);
    Ok(SecretBox {
        nonce,
        ciphertext: seal_typed_secretbox(
            key.as_bytes(),
            foks_proto::TEAM_REMOTE_MEMBER_VIEW_TOKEN_BOX_PAYLOAD_TYPE_ID,
            &nonce,
            cleartext.as_slice(),
            false,
        )?,
    })
}

/// Opens and authenticates a federation token box. Callers must additionally
/// bind the cleartext party and the outer PTK metadata to verified team state.
pub fn open_team_remote_member_view_token(
    ptk_seed: &SecretSeed,
    boxed: &SecretBox,
) -> Result<foks_proto::TeamRemoteMemberViewTokenBoxPayload> {
    let key = derive_key(ptk_seed, 2, None)?;
    let cleartext = open_typed_secretbox(
        key.as_bytes(),
        foks_proto::TEAM_REMOTE_MEMBER_VIEW_TOKEN_BOX_PAYLOAD_TYPE_ID,
        &boxed.nonce,
        &boxed.ciphertext,
    )?;
    foks_proto::TeamRemoteMemberViewTokenBoxPayload::decode(&cleartext).map_err(Into::into)
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

/// Derives the exact Ed25519 identity and hybrid encryption public key used by
/// a v0.1.9 device. Secret intermediates are zeroized on drop.
pub fn derive_device_public(seed: &SecretSeed) -> Result<DevicePublicMaterial> {
    derive_public_material(seed, foks_proto::ENTITY_DEVICE)
}

fn derive_public_material(seed: &SecretSeed, entity_type: u8) -> Result<DevicePublicMaterial> {
    let signing_seed = derive_key(seed, 0, None)?;
    let signing = SigningKey::from_bytes(signing_seed.as_bytes());
    let mut id = Vec::with_capacity(33);
    id.push(entity_type);
    id.extend_from_slice(signing.verifying_key().as_bytes());
    let id = EntityId::from_bytes(id)?;

    let dh_seed = derive_key(seed, 1, None)?;
    let dh_secret = StaticSecret::from(*dh_seed.as_bytes());
    let dh_public = X25519PublicKey::from(&dh_secret);
    let mlkem_seed = Zeroizing::new(derive_mlkem_seed(seed)?);
    let kem_seed = ml_kem::Seed::try_from(mlkem_seed.as_slice()).map_err(|_| Error::DeviceKey)?;
    let decapsulation = ml_kem_768::DecapsulationKey::from_seed(kem_seed);
    let kem_public = decapsulation.encapsulation_key().to_bytes().to_vec();
    let hepk_bytes = encode_hepk(dh_public.as_bytes(), &kem_public)?;
    let hepk = Hepk::decode(&hepk_bytes)?;
    Ok(DevicePublicMaterial { id, hepk })
}

/// Returns the RFC 8410 PKCS#8 DER encoding of the Ed25519 key FOKS derives
/// from a device seed. This is suitable for rustls client authentication.
pub fn device_signing_key_pkcs8(seed: &SecretSeed) -> Result<Zeroizing<Vec<u8>>> {
    let signing_seed = derive_key(seed, 0, None)?;
    let mut der = Zeroizing::new(Vec::with_capacity(48));
    der.extend_from_slice(&[
        0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04,
        0x20,
    ]);
    der.extend_from_slice(signing_seed.as_slice());
    Ok(der)
}

/// Opens and validates an owner PUK parcel using exact FOKS v0.1.9 hybrid
/// X25519 + ML-KEM-768 and XSalsa20-Poly1305 semantics.
pub fn open_puk_parcel(
    parcel: &PukParcel,
    device_seed: &SecretSeed,
    sender_hepk: &Hepk,
    expected_puk_verify_key: &EntityId,
    expected_puk_hepk: &Hepk,
    expected_puk_generation: u64,
    expected_host: &EntityId,
) -> Result<SharedKeySeed> {
    open_puk_parcel_for_role(
        parcel,
        device_seed,
        sender_hepk,
        expected_puk_verify_key,
        expected_puk_hepk,
        expected_puk_generation,
        expected_host,
        Role::OWNER,
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "the authenticated PUK, host, and role bindings must remain explicit"
)]
pub fn open_puk_parcel_for_role(
    parcel: &PukParcel,
    device_seed: &SecretSeed,
    sender_hepk: &Hepk,
    expected_puk_verify_key: &EntityId,
    expected_puk_hepk: &Hepk,
    expected_puk_generation: u64,
    expected_host: &EntityId,
    expected_role: Role,
) -> Result<SharedKeySeed> {
    let receiver = SoftwareDecapsulator::new(device_seed)?;
    open_puk_parcel_with_for_role(
        parcel,
        &receiver,
        sender_hepk,
        expected_puk_verify_key,
        expected_puk_hepk,
        expected_puk_generation,
        expected_host,
        expected_role,
    )
}

/// Opens a PUK parcel with a software or hardware-backed receiver.
pub fn open_puk_parcel_with(
    parcel: &PukParcel,
    receiver: &dyn HybridSecretDecapsulator,
    sender_hepk: &Hepk,
    expected_puk_verify_key: &EntityId,
    expected_puk_hepk: &Hepk,
    expected_puk_generation: u64,
    expected_host: &EntityId,
) -> Result<SharedKeySeed> {
    open_puk_parcel_with_for_role(
        parcel,
        receiver,
        sender_hepk,
        expected_puk_verify_key,
        expected_puk_hepk,
        expected_puk_generation,
        expected_host,
        Role::OWNER,
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "the authenticated PUK, host, and role bindings must remain explicit"
)]
pub fn open_puk_parcel_with_for_role(
    parcel: &PukParcel,
    receiver: &dyn HybridSecretDecapsulator,
    sender_hepk: &Hepk,
    expected_puk_verify_key: &EntityId,
    expected_puk_hepk: &Hepk,
    expected_puk_generation: u64,
    expected_host: &EntityId,
    expected_role: Role,
) -> Result<SharedKeySeed> {
    open_shared_key_parcel_with(
        parcel,
        receiver,
        sender_hepk,
        expected_puk_verify_key,
        expected_puk_hepk,
        expected_puk_generation,
        expected_host,
        Role::NONE,
        0,
        expected_role,
        foks_proto::ENTITY_PUK_VERIFY,
    )
}

/// Opens every prior PUK generation carried by a verified parcel. The
/// returned keys are ordered from oldest to current generation.
pub fn open_puk_seed_chain(
    current: SharedKeySeed,
    parcel: &PukParcel,
    expected_user: &EntityId,
    expected_host: &EntityId,
) -> Result<Vec<SharedKeySeed>> {
    open_shared_key_seed_chain(current, parcel, expected_user, expected_host)
}

/// Opens the prior PUK or PTK generations chained from a current parcel.
pub fn open_shared_key_seed_chain(
    current: SharedKeySeed,
    parcel: &PukParcel,
    expected_party: &EntityId,
    expected_host: &EntityId,
) -> Result<Vec<SharedKeySeed>> {
    if current.generation != parcel.generation || current.role != parcel.role {
        return Err(Error::PukBinding);
    }
    let mut descending = vec![current];
    for boxed in parcel.seed_chain.iter().rev() {
        let newer = descending.last().ok_or(Error::PukBinding)?;
        let expected_generation = newer.generation.checked_sub(1).ok_or(Error::PukBinding)?;
        if boxed.generation != expected_generation || boxed.role != newer.role {
            return Err(Error::PukBinding);
        }
        let secretbox_key = derive_key(&newer.seed, 2, None)?;
        let plaintext = open_typed_secretbox(
            secretbox_key.as_bytes(),
            SHARED_KEY_SEED_TYPE_ID,
            &boxed.secret_box.nonce,
            &boxed.secret_box.ciphertext,
        )?;
        let (_, consumed) = decode_prefix(&plaintext)?;
        require_zero_padding(&plaintext, consumed)?;
        let older = SharedKeySeed::decode(&plaintext[..consumed])?;
        if &older.receiver != expected_party
            || &older.host != expected_host
            || older.generation != boxed.generation
            || older.role != boxed.role
        {
            return Err(Error::PukBinding);
        }
        descending.push(older);
    }
    descending.reverse();
    Ok(descending)
}

/// Opens a team PTK generation chain. FOKS v0.1.9 creates one global chain
/// per rotated PTK and binds its informational receiver field to the editor,
/// then attaches that chain to every recipient parcel. Callers must bind each
/// recovered seed to the authenticated team-chain public-key history.
pub fn open_team_shared_key_seed_chain(
    current: SharedKeySeed,
    parcel: &PukParcel,
    expected_host: &EntityId,
) -> Result<Vec<SharedKeySeed>> {
    if current.generation != parcel.generation || current.role != parcel.role {
        return Err(Error::PukBinding);
    }
    let mut descending = vec![current];
    for boxed in parcel.seed_chain.iter().rev() {
        let newer = descending.last().ok_or(Error::PukBinding)?;
        let expected_generation = newer.generation.checked_sub(1).ok_or(Error::PukBinding)?;
        if boxed.generation != expected_generation || boxed.role != newer.role {
            return Err(Error::PukBinding);
        }
        let secretbox_key = derive_key(&newer.seed, 2, None)?;
        let plaintext = open_typed_secretbox(
            secretbox_key.as_bytes(),
            SHARED_KEY_SEED_TYPE_ID,
            &boxed.secret_box.nonce,
            &boxed.secret_box.ciphertext,
        )?;
        let (_, consumed) = decode_prefix(&plaintext)?;
        require_zero_padding(&plaintext, consumed)?;
        let older = SharedKeySeed::decode(&plaintext[..consumed])?;
        if &older.host != expected_host
            || older.generation != boxed.generation
            || older.role != boxed.role
        {
            return Err(Error::PukBinding);
        }
        descending.push(older);
    }
    descending.reverse();
    Ok(descending)
}

/// Opens a PUK or PTK parcel and binds its cleartext to the authenticated
/// target, role, generation, host, and expected verification key.
#[allow(clippy::too_many_arguments)]
pub fn open_shared_key_parcel_with(
    parcel: &PukParcel,
    receiver: &dyn HybridSecretDecapsulator,
    sender_hepk: &Hepk,
    expected_verify_key: &EntityId,
    expected_shared_hepk: &Hepk,
    expected_shared_generation: u64,
    expected_host: &EntityId,
    expected_receiver_role: Role,
    expected_receiver_generation: u64,
    expected_role: Role,
    expected_verify_key_type: u8,
) -> Result<SharedKeySeed> {
    open_scoped_shared_key_parcel_with(
        parcel,
        receiver,
        sender_hepk,
        expected_verify_key,
        expected_shared_hepk,
        expected_shared_generation,
        expected_host,
        None,
        expected_receiver_role,
        expected_receiver_generation,
        expected_role,
        expected_verify_key_type,
    )
}

/// Opens a parcel for an explicitly scoped foreign recipient. Go binds the
/// current seed payload to the receiver host; the supplied verified PTK and
/// temporary sender signature remain bound to the issuer.
#[allow(clippy::too_many_arguments)]
pub fn open_scoped_shared_key_parcel_with(
    parcel: &PukParcel,
    receiver: &dyn HybridSecretDecapsulator,
    sender_hepk: &Hepk,
    expected_verify_key: &EntityId,
    expected_shared_hepk: &Hepk,
    expected_shared_generation: u64,
    expected_host: &EntityId,
    receiver_host: Option<&EntityId>,
    expected_receiver_role: Role,
    expected_receiver_generation: u64,
    expected_role: Role,
    expected_verify_key_type: u8,
) -> Result<SharedKeySeed> {
    if parcel.role != expected_role
        || parcel.generation != expected_shared_generation
        || parcel.hybrid.sender_dh.is_some()
    {
        return Err(Error::HybridBox);
    }
    if &parcel.target != receiver.entity_id()
        || parcel.target_host.as_ref() != receiver_host
        || parcel.target_role != expected_receiver_role
        || parcel.target_generation != expected_receiver_generation
    {
        return Err(Error::WrongReceiver);
    }
    let sender_dh = authenticated_sender_dh(parcel, receiver.hepk(), sender_hepk, expected_host)?;
    let cleartext = open_hybrid_box(
        &parcel.hybrid,
        receiver,
        &sender_dh,
        SHARED_KEY_SEED_TYPE_ID,
    )?;
    let shared = SharedKeySeed::decode(&cleartext)?;
    if &shared.receiver != receiver.entity_id()
        || &shared.host != receiver_host.unwrap_or(expected_host)
        || shared.generation != parcel.generation
        || shared.role != parcel.role
    {
        return Err(Error::PukBinding);
    }
    let derived = derive_shared_public(&shared.seed, expected_verify_key_type)?;
    if &derived.verify_key != expected_verify_key || &derived.hepk != expected_shared_hepk {
        return Err(Error::PukBinding);
    }
    Ok(shared)
}

/// Software PUK/PTK receiver whose target is the persistent user or team ID.
pub struct SharedKeyDecapsulator<'a> {
    seed: &'a SecretSeed,
    target: EntityId,
    hepk: Hepk,
}

impl<'a> SharedKeyDecapsulator<'a> {
    pub fn new(seed: &'a SecretSeed, target: EntityId) -> Result<Self> {
        if !matches!(
            target.entity_type(),
            foks_proto::ENTITY_USER
                | foks_proto::ENTITY_NAMED_TEAM
                | foks_proto::ENTITY_AD_HOC_TEAM
        ) {
            return Err(Error::WrongReceiver);
        }
        Ok(Self {
            seed,
            target,
            hepk: derive_device_public(seed)?.hepk,
        })
    }
}

impl HybridSecretDecapsulator for SharedKeyDecapsulator<'_> {
    fn entity_id(&self) -> &EntityId {
        &self.target
    }
    fn hepk(&self) -> &Hepk {
        &self.hepk
    }
    fn derive_dh_shared(&self, peer: &DhPublicKey) -> Result<Zeroizing<[u8; 32]>> {
        software_dh_shared(self.seed, peer)
    }
    fn decapsulate_mlkem768(&self, ciphertext: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
        software_mlkem_decapsulate(self.seed, ciphertext)
    }
}

/// Opens and validates the Ed25519 RPC subkey in a Yubi self-box.
pub fn open_subkey_box(
    box_bytes: &[u8],
    parent: &dyn HybridSecretDecapsulator,
    expected_subkey: &EntityId,
) -> Result<SecretSeed> {
    let hybrid = HybridBox::decode(box_bytes)?;
    if hybrid.sender_dh.is_some() {
        return Err(Error::HybridBox);
    }
    let cleartext = open_hybrid_box(
        &hybrid,
        parent,
        parent.hepk().classical(),
        SUBKEY_SEED_TYPE_ID,
    )?;
    let subkey = SubkeySeed::decode(&cleartext)?;
    if &subkey.parent != parent.entity_id()
        || &subkey.subkey != expected_subkey
        || derive_subkey_id(&subkey.seed)? != *expected_subkey
    {
        return Err(Error::PukBinding);
    }
    Ok(subkey.into_seed())
}

fn authenticated_sender_dh(
    parcel: &PukParcel,
    receiver_hepk: &Hepk,
    sender_hepk: &Hepk,
    expected_host: &EntityId,
) -> Result<DhPublicKey> {
    let same_type = std::mem::discriminant(receiver_hepk.classical())
        == std::mem::discriminant(sender_hepk.classical());
    if same_type {
        // A mixed-curve box set carries one temporary key for mismatched
        // recipients. Go ignores it for same-curve boxes in that same set.
        return Ok(sender_hepk.classical().clone());
    }
    let temporary = parcel.temp_dh_key.as_ref().ok_or(Error::HybridBox)?;
    if std::mem::discriminant(receiver_hepk.classical()) != std::mem::discriminant(&temporary.key) {
        return Err(Error::HybridBox);
    }
    let signing_bytes = temporary.signing_bytes(&parcel.box_id, &parcel.sender, expected_host)?;
    verify_typed(
        &parcel.sender,
        &temporary.signature,
        TEMP_DH_KEY_SIG_TEMPLATE_TYPE_ID,
        &signing_bytes,
    )?;
    Ok(temporary.key.clone())
}

fn open_hybrid_box(
    hybrid: &HybridBox,
    receiver: &dyn HybridSecretDecapsulator,
    sender_dh: &DhPublicKey,
    payload_type_id: u64,
) -> Result<Zeroizing<Vec<u8>>> {
    let expected_type = match receiver.hepk().classical() {
        DhPublicKey::Curve25519(_) => 1,
        DhPublicKey::P256(_) => 2,
    };
    if hybrid.dh_type != expected_type {
        return Err(Error::HybridBox);
    }
    let dh_shared = receiver.derive_dh_shared(sender_dh)?;
    let kem_shared = receiver.decapsulate_mlkem768(&hybrid.kem_ciphertext)?;
    let payload = hybrid_key_derivation_payload(
        kem_shared.as_slice(),
        dh_shared.as_slice(),
        receiver.hepk(),
        sender_dh,
    )?;
    let mut hash = <Sha3_256 as Sha3Digest>::new();
    hash.update(HYBRID_SECRET_KEY_SHA3_PAYLOAD_TYPE_ID.to_be_bytes());
    hash.update(&payload);
    let key = Zeroizing::new(<[u8; 32]>::from(hash.finalize()));
    let mut nonce = [0u8; 24];
    nonce[..8].copy_from_slice(&payload_type_id.to_be_bytes());
    nonce[8..].copy_from_slice(&hybrid.nonce);
    let cipher = XSalsa20Poly1305::new(key.as_slice().into());
    Ok(Zeroizing::new(
        cipher
            .decrypt((&nonce).into(), hybrid.ciphertext.as_slice())
            .map_err(|_| Error::Decryption)?,
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SoftwareKeyKind {
    Device,
    BotToken,
}
impl SoftwareKeyKind {
    pub fn entity_type(self) -> u8 {
        match self {
            Self::Device => foks_proto::ENTITY_DEVICE,
            Self::BotToken => foks_proto::ENTITY_BOT_TOKEN_KEY,
        }
    }
}
pub fn derive_software_public(
    seed: &SecretSeed,
    kind: SoftwareKeyKind,
) -> Result<DevicePublicMaterial> {
    derive_public_material(seed, kind.entity_type())
}

pub struct SoftwareDecapsulator<'a> {
    seed: &'a SecretSeed,
    public: DevicePublicMaterial,
}

impl<'a> SoftwareDecapsulator<'a> {
    pub fn for_kind(seed: &'a SecretSeed, kind: SoftwareKeyKind) -> Result<Self> {
        Ok(Self {
            seed,
            public: derive_software_public(seed, kind)?,
        })
    }
    fn new(seed: &'a SecretSeed) -> Result<Self> {
        Ok(Self {
            seed,
            public: derive_device_public(seed)?,
        })
    }
}

impl HybridSecretDecapsulator for SoftwareDecapsulator<'_> {
    fn entity_id(&self) -> &EntityId {
        &self.public.id
    }

    fn hepk(&self) -> &Hepk {
        &self.public.hepk
    }

    fn derive_dh_shared(&self, peer: &DhPublicKey) -> Result<Zeroizing<[u8; 32]>> {
        software_dh_shared(self.seed, peer)
    }

    fn decapsulate_mlkem768(&self, ciphertext: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
        software_mlkem_decapsulate(self.seed, ciphertext)
    }
}

fn software_dh_shared(seed: &SecretSeed, peer: &DhPublicKey) -> Result<Zeroizing<[u8; 32]>> {
    let DhPublicKey::Curve25519(peer) = peer else {
        return Err(Error::HybridBox);
    };
    let dh_seed = derive_key(seed, 1, None)?;
    let secret = StaticSecret::from(*dh_seed.as_bytes());
    let raw = secret.diffie_hellman(&X25519PublicKey::from(*peer));
    let zero = [0u8; 16];
    let shared = hsalsa::<U10>(raw.as_bytes().into(), (&zero).into());
    Ok(Zeroizing::new(shared.into()))
}

fn software_mlkem_decapsulate(seed: &SecretSeed, ciphertext: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
    let mlkem_seed = Zeroizing::new(derive_mlkem_seed(seed)?);
    let kem_seed = ml_kem::Seed::try_from(mlkem_seed.as_slice()).map_err(|_| Error::DeviceKey)?;
    let decapsulation = ml_kem_768::DecapsulationKey::from_seed(kem_seed);
    let ciphertext = ml_kem_768::Ciphertext::try_from(ciphertext).map_err(|_| Error::MlKem)?;
    Ok(Zeroizing::new(
        decapsulation.decapsulate(&ciphertext).into(),
    ))
}

fn dh_public_value(key: &DhPublicKey) -> Value {
    let (kind, tag, bytes): (u64, Vec<u8>, &[u8]) = match key {
        DhPublicKey::Curve25519(bytes) => (1, b"0".to_vec(), bytes),
        DhPublicKey::P256(bytes) => (2, b"1".to_vec(), bytes),
    };
    Value::Array(vec![
        Value::Unsigned(kind),
        Value::Variant(Some((tag, Box::new(Value::Binary(bytes.to_vec()))))),
    ])
}

#[cfg(test)]
fn derive_hybrid_key(
    parcel: &PukParcel,
    device_seed: &SecretSeed,
    sender_dh: &DhPublicKey,
    receiver_hepk: &Hepk,
) -> Result<HybridDerivation> {
    let dh_seed = derive_key(device_seed, 1, None)?;
    let dh_secret = StaticSecret::from(*dh_seed.as_bytes());
    let DhPublicKey::Curve25519(sender_key) = sender_dh else {
        return Err(Error::HybridBox);
    };
    let sender_public = X25519PublicKey::from(*sender_key);
    let raw_dh = dh_secret.diffie_hellman(&sender_public);
    // Go's nacl/box.Precompute applies HSalsa20 to the raw X25519 result;
    // FOKS commits that NaCl precomputed key, not raw X25519, into the hybrid
    // SHA3 payload.
    let zero = [0u8; 16];
    let dh_shared = hsalsa::<U10>(raw_dh.as_bytes().into(), (&zero).into());

    let mlkem_seed = Zeroizing::new(derive_mlkem_seed(device_seed)?);
    let kem_seed = ml_kem::Seed::try_from(mlkem_seed.as_slice()).map_err(|_| Error::DeviceKey)?;
    let decapsulation = ml_kem_768::DecapsulationKey::from_seed(kem_seed);
    let ciphertext = ml_kem_768::Ciphertext::try_from(parcel.hybrid.kem_ciphertext.as_slice())
        .map_err(|_| Error::MlKem)?;
    let kem_shared = decapsulation.decapsulate(&ciphertext);

    let payload = hybrid_key_derivation_payload(
        kem_shared.as_slice(),
        dh_shared.as_slice(),
        receiver_hepk,
        &DhPublicKey::Curve25519(*sender_key),
    )?;
    let mut hash = <Sha3_256 as Sha3Digest>::new();
    hash.update(HYBRID_SECRET_KEY_SHA3_PAYLOAD_TYPE_ID.to_be_bytes());
    hash.update(&payload);
    Ok((Zeroizing::new(hash.finalize().into()), payload))
}

pub fn derive_shared_verify_key(seed: &SecretSeed, entity_type: u8) -> Result<EntityId> {
    let signing_seed = derive_key(seed, 0, None)?;
    let signing = SigningKey::from_bytes(signing_seed.as_bytes());
    let mut id = Vec::with_capacity(33);
    id.push(entity_type);
    id.extend_from_slice(signing.verifying_key().as_bytes());
    EntityId::from_bytes(id).map_err(Into::into)
}

pub fn derive_subkey_id(seed: &SecretSeed) -> Result<EntityId> {
    derive_shared_verify_key(seed, foks_proto::ENTITY_SUBKEY)
}

fn derive_mlkem_seed(seed: &SecretSeed) -> Result<Zeroizing<[u8; 64]>> {
    let first = derive_key(seed, 4, Some(0))?;
    let second = derive_key(seed, 4, Some(1))?;
    let mut output = Zeroizing::new([0u8; 64]);
    output[..32].copy_from_slice(first.as_slice());
    output[32..].copy_from_slice(second.as_slice());
    Ok(output)
}

fn derive_key(seed: &SecretSeed, derivation_type: u64, index: Option<u64>) -> Result<SecretSeed> {
    let payload = index.map_or(Value::Variant(None), |index| {
        Value::Variant(Some((b"4".to_vec(), Box::new(Value::Unsigned(index)))))
    });
    let object = encode(&Value::Array(vec![
        Value::Unsigned(derivation_type),
        payload,
    ]))?;
    let mut mac =
        <Hmac<Sha512_256> as Mac>::new_from_slice(seed.as_slice()).map_err(|_| Error::DeviceKey)?;
    mac.update(&0xd35c_dcc9_5cae_f674_u64.to_be_bytes());
    mac.update(&object);
    Ok(SecretSeed::new(mac.finalize().into_bytes().into()))
}

fn encode_hepk(classical: &[u8; 32], mlkem768: &[u8]) -> Result<Vec<u8>> {
    Ok(encode(&Value::Array(vec![
        Value::Unsigned(1),
        Value::Variant(Some((
            b"1".to_vec(),
            Box::new(Value::Array(vec![
                Value::Array(vec![
                    Value::Unsigned(1),
                    Value::Variant(Some((
                        b"0".to_vec(),
                        Box::new(Value::Binary(classical.to_vec())),
                    ))),
                ]),
                Value::Array(vec![
                    Value::Unsigned(1),
                    Value::Variant(Some((
                        b"1".to_vec(),
                        Box::new(Value::Binary(mlkem768.to_vec())),
                    ))),
                ]),
            ])),
        ))),
    ]))?)
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

fn prefixed_hash_without_type(message: &[u8]) -> [u8; 32] {
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

#[cfg(test)]
mod tests {
    use super::*;
    use foks_proto::{
        ProbeResponse, PukParcel, TeamChain, UserChain, UserLink, ENTITY_PTK_VERIFY,
        ENTITY_PUK_VERIFY, PUBLIC_ZONE_BLOB_TYPE_ID,
    };

    const PROBE: &[u8] = include_bytes!(
        "../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
    );
    const USER_DIR: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9/user";
    const SIGNUP_DIR: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9/signup";
    const MUTATION_DIR: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9/user-mutations";

    fn user_fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{USER_DIR}/{name}")).unwrap()
    }

    fn signup_fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{SIGNUP_DIR}/{name}")).unwrap()
    }

    fn mutation_fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{MUTATION_DIR}/{name}")).unwrap()
    }

    struct MockYubi {
        public: YubiPublicMaterial,
        signing: p256::ecdsa::SigningKey,
        dh: P256SecretKey,
        pq_self_secret: [u8; 32],
    }

    impl MockYubi {
        fn new(signing_scalar: u8, pq_scalar: u8) -> Self {
            let signing_bytes = [signing_scalar; 32];
            let pq_bytes = [pq_scalar; 32];
            let signing = p256::ecdsa::SigningKey::from_bytes((&signing_bytes).into()).unwrap();
            let dh = P256SecretKey::from_slice(&signing_bytes).unwrap();
            let pq = P256SecretKey::from_slice(&pq_bytes).unwrap();
            let signing_public: [u8; 33] = signing
                .verifying_key()
                .to_encoded_point(true)
                .as_bytes()
                .try_into()
                .unwrap();
            let pq_public: [u8; 33] = pq
                .public_key()
                .to_encoded_point(true)
                .as_bytes()
                .try_into()
                .unwrap();
            let pq_self = p256_diffie_hellman(pq.to_nonzero_scalar(), pq.public_key().as_affine());
            let pq_self_secret = pq_self.raw_secret_bytes().as_slice().try_into().unwrap();
            let public =
                derive_yubi_public_material(signing_public, pq_public, pq_self_secret).unwrap();
            Self {
                public,
                signing,
                dh,
                pq_self_secret,
            }
        }
    }

    impl HybridSecretDecapsulator for MockYubi {
        fn entity_id(&self) -> &EntityId {
            &self.public.device.id
        }

        fn hepk(&self) -> &Hepk {
            &self.public.device.hepk
        }

        fn derive_dh_shared(&self, peer: &DhPublicKey) -> Result<Zeroizing<[u8; 32]>> {
            let DhPublicKey::P256(peer) = peer else {
                return Err(Error::HybridBox);
            };
            let peer = P256PublicKey::from_sec1_bytes(peer).map_err(|_| Error::HybridBox)?;
            let shared = p256_diffie_hellman(self.dh.to_nonzero_scalar(), peer.as_affine());
            Ok(Zeroizing::new(
                shared.raw_secret_bytes().as_slice().try_into().unwrap(),
            ))
        }

        fn decapsulate_mlkem768(&self, ciphertext: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
            yubi_mlkem_decapsulate(self.pq_self_secret, ciphertext)
        }
    }

    impl YubiDevice for MockYubi {
        fn pq_key_id(&self) -> [u8; 32] {
            self.public.pq_key_id
        }

        fn sign_sha512_256(&self, digest: &[u8; 32]) -> Result<Vec<u8>> {
            use p256::ecdsa::signature::hazmat::PrehashSigner as _;

            let signature: P256Signature = self
                .signing
                .sign_prehash(digest)
                .map_err(|_| Error::YubiSigning)?;
            Ok(signature.to_der().as_bytes().to_vec())
        }
    }

    #[test]
    fn subchain_location_matches_the_go_reference() {
        assert_eq!(
            subchain_tree_location(&[0x35; 32], foks_proto::CHAIN_TYPE_TEAM_MEMBERSHIP).unwrap(),
            [
                0x90, 0xde, 0x90, 0x44, 0xfc, 0xee, 0x5b, 0x28, 0x84, 0xe0, 0xd0, 0x4f, 0x7c, 0x71,
                0xd6, 0xaf, 0x83, 0xce, 0x04, 0x7b, 0x05, 0x1f, 0x56, 0xc2, 0x07, 0xb5, 0x0b, 0xdb,
                0x9d, 0x7f, 0x40, 0x96,
            ]
        );
    }

    #[test]
    fn signing_and_verification_reject_a_non_canonical_signable_object() {
        // array16 with 16..=31 elements is canonical for RPC arguments (the
        // signup argument is array16(16)) but not for signed, verified, or
        // hashed objects, matching go-foks's AssertCanonicalMsgpack. Both the
        // signer and the verifier reject it before any signature operation.
        let mut non_canonical = vec![0xdc, 0x00, 0x10];
        non_canonical.extend(std::iter::repeat_n(0xc0, 16));
        let signer =
            EntityId::from_bytes([vec![ENTITY_PUK_VERIFY], vec![0x11; 32]].concat()).unwrap();
        assert!(matches!(
            sign_ed25519_typed(&[0u8; 32], 1, &non_canonical).unwrap_err(),
            Error::Snowpack(_)
        ));
        assert!(matches!(
            verify_typed(&signer, &Signature::Ed25519([0; 64]), 1, &non_canonical).unwrap_err(),
            Error::Snowpack(_)
        ));
        assert!(matches!(
            sign_ed25519_blob(&[0u8; 32], 1, &non_canonical).unwrap_err(),
            Error::Snowpack(_)
        ));
        assert!(matches!(
            sign_shared_key_blob(&SecretSeed::new([0; 32]), 1, &non_canonical).unwrap_err(),
            Error::Snowpack(_)
        ));
        assert!(matches!(
            verify_blob(&signer, &Signature::Ed25519([0; 64]), 1, &non_canonical,).unwrap_err(),
            Error::Snowpack(_)
        ));
        assert!(matches!(
            prefixed_hash_signable(1, &non_canonical).unwrap_err(),
            Error::Snowpack(_)
        ));

        // A fixarray-shaped signed object is still accepted by the signer (the
        // error path is specific to the disallowed array16 form).
        let canonical = vec![0x91, 0xc0];
        assert!(sign_ed25519_typed(&[0u8; 32], 1, &canonical).is_ok());
        assert!(sign_ed25519_blob(&[0u8; 32], 1, &canonical).is_ok());
        assert_eq!(
            prefixed_hash_signable(1, &canonical).unwrap(),
            prefixed_hash(1, &canonical)
        );
    }

    #[test]
    fn software_eldest_link_matches_official_signup_fixture() {
        let expected = UserLink::decode(&signup_fixture("eldest-link.snowp")).unwrap();
        let opened = expected.decode_eldest().unwrap();
        let device_seed = SecretSeed::new(signup_fixture("device-seed.bin").try_into().unwrap());
        let puk_seed = SecretSeed::new(signup_fixture("puk-seed.bin").try_into().unwrap());
        let material = make_software_eldest_link(
            &SoftwareEldestInput {
                host: &opened.host,
                root: &opened.root,
                time: opened.time,
                next_tree_location: signup_fixture("next-tree-location.bin").try_into().unwrap(),
                subchain_tree_location: signup_fixture("subchain-tree-location.bin")
                    .try_into()
                    .unwrap(),
                normalized_username: b"signupfixture",
                username_sequence: 1,
                username_commitment_key: signup_fixture("username-commitment-key.bin")
                    .try_into()
                    .unwrap(),
                device_name: &DeviceLabelNameAndCommitmentKey {
                    label: foks_proto::DeviceLabel {
                        device_type: foks_proto::DeviceType::Computer,
                        normalized_name: b"signup device".to_vec(),
                        serial: 1,
                    },
                    normalization_version: 0,
                    display_name: b"signup device".to_vec(),
                    commitment_key: signup_fixture("device-commitment-key.bin")
                        .try_into()
                        .unwrap(),
                },
            },
            &device_seed,
            &puk_seed,
        )
        .unwrap();
        assert_eq!(
            material.link.encoded().unwrap(),
            expected.encoded().unwrap()
        );
        let expected_uid = match decode(&signup_fixture("uid.snowp")).unwrap() {
            Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
            other => panic!("expected UID fixture, got {other:?}"),
        };
        assert_eq!(material.uid, expected_uid);
    }

    #[test]
    fn software_device_mutations_match_official_user_fixtures() {
        let chain = UserChain::decode(&user_fixture("user-chain.snowp")).unwrap();
        let expected_provision =
            UserLink::decode(&user_fixture("user-provision-link.snowp")).unwrap();
        let provision_change = expected_provision.decode_group_change().unwrap();
        let existing_seed = SecretSeed::new(user_fixture("device-seed.bin").try_into().unwrap());
        let new_seed = SecretSeed::new(user_fixture("second-device-seed.bin").try_into().unwrap());
        let disclosed = &chain.device_names[1];
        let provision = make_software_provision_link(
            &SoftwareProvisionInput {
                base: UserMutationBase {
                    uid: &provision_change.uid,
                    host: &provision_change.host,
                    seqno: provision_change.seqno,
                    previous: provision_change.previous.unwrap(),
                    root: &provision_change.root,
                    time: provision_change.time,
                    next_tree_location: chain.locations[1],
                },
                role: Role::OWNER,
                device_label: &disclosed.label,
                device_name_commitment_key: disclosed.commitment_key,
            },
            &existing_seed,
            &new_seed,
            None,
        )
        .unwrap();
        assert_eq!(
            provision.link.encoded().unwrap(),
            expected_provision.encoded().unwrap()
        );

        let expected_revoke = UserLink::decode(&user_fixture("user-revoke-link.snowp")).unwrap();
        let revoke_change = expected_revoke.decode_group_change().unwrap();
        let rotated = SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap());
        let revoke = make_software_revoke_link(
            &UserMutationBase {
                uid: &revoke_change.uid,
                host: &revoke_change.host,
                seqno: revoke_change.seqno,
                previous: revoke_change.previous.unwrap(),
                root: &revoke_change.root,
                time: revoke_change.time,
                next_tree_location: chain.locations[2],
            },
            &existing_seed,
            &provision.device.id,
            &[PukRotation {
                role: Role::OWNER,
                generation: 2,
                seed: &rotated,
            }],
        )
        .unwrap();
        assert_eq!(
            revoke.encoded().unwrap(),
            expected_revoke.encoded().unwrap()
        );
    }

    #[test]
    fn standalone_puk_rotation_matches_official_go_fixture() {
        let expected = UserLink::decode(&mutation_fixture("rotation-link.snowp")).unwrap();
        let change = expected.decode_group_change().unwrap();
        assert!(change.changes.is_empty());
        let signer_seed = SecretSeed::new(user_fixture("device-seed.bin").try_into().unwrap());
        let rotation_seed = SecretSeed::new(
            mutation_fixture("rotation-puk-seed.bin")
                .try_into()
                .unwrap(),
        );
        let actual = make_software_puk_rotation_link(
            &UserMutationBase {
                uid: &change.uid,
                host: &change.host,
                seqno: change.seqno,
                previous: change.previous.unwrap(),
                root: &change.root,
                time: change.time,
                next_tree_location: mutation_fixture("rotation-next-tree-location.bin")
                    .try_into()
                    .unwrap(),
            },
            &signer_seed,
            &[PukRotation {
                role: Role::OWNER,
                generation: 3,
                seed: &rotation_seed,
            }],
        )
        .unwrap();
        assert_eq!(actual.encoded().unwrap(), expected.encoded().unwrap());
        assert!(make_software_puk_rotation_link(
            &UserMutationBase {
                uid: &change.uid,
                host: &change.host,
                seqno: change.seqno,
                previous: change.previous.unwrap(),
                root: &change.root,
                time: change.time,
                next_tree_location: [0; 32],
            },
            &signer_seed,
            &[],
        )
        .is_err());
    }

    #[test]
    fn yubi_puk_rotation_stacks_rotated_keys_before_the_parent() {
        let expected = UserLink::decode(&mutation_fixture("rotation-link.snowp")).unwrap();
        let base_change = expected.decode_group_change().unwrap();
        let parent = MockYubi::new(0x11, 0x12);
        let member_seed = SecretSeed::new([0x21; 32]);
        let owner_seed = SecretSeed::new([0x22; 32]);
        let rotations = [
            PukRotation {
                role: Role::member(0),
                generation: 2,
                seed: &member_seed,
            },
            PukRotation {
                role: Role::OWNER,
                generation: 3,
                seed: &owner_seed,
            },
        ];
        let base = UserMutationBase {
            uid: &base_change.uid,
            host: &base_change.host,
            seqno: base_change.seqno,
            previous: base_change.previous.unwrap(),
            root: &base_change.root,
            time: base_change.time,
            next_tree_location: [0x31; 32],
        };
        let link = make_yubi_puk_rotation_link(&base, &parent, &rotations).unwrap();
        assert_eq!(
            link.encoded().unwrap(),
            make_yubi_puk_rotation_link(&base, &parent, &rotations)
                .unwrap()
                .encoded()
                .unwrap()
        );
        let change = link.decode_group_change().unwrap();
        assert_eq!(change.signer, *parent.entity_id());
        assert!(change.changes.is_empty());
        assert_eq!(
            change
                .shared_keys
                .iter()
                .map(|key| (key.role, key.generation))
                .collect::<Vec<_>>(),
            vec![(Role::member(0), 2), (Role::OWNER, 3)]
        );
        assert_eq!(link.signatures().len(), 3);
        let member = derive_shared_public(&member_seed, ENTITY_PUK_VERIFY).unwrap();
        let owner = derive_shared_public(&owner_seed, ENTITY_PUK_VERIFY).unwrap();
        verify_typed(
            &member.verify_key,
            &link.signatures()[0],
            LINK_OUTER_V1_TYPE_ID,
            &link.signing_bytes(0).unwrap(),
        )
        .unwrap();
        verify_typed(
            &owner.verify_key,
            &link.signatures()[1],
            LINK_OUTER_V1_TYPE_ID,
            &link.signing_bytes(1).unwrap(),
        )
        .unwrap();
        verify_typed(
            parent.entity_id(),
            &link.signatures()[2],
            LINK_OUTER_V1_TYPE_ID,
            &link.signing_bytes(2).unwrap(),
        )
        .unwrap();
        assert!(verify_typed(
            parent.entity_id(),
            &link.signatures()[2],
            LINK_OUTER_V1_TYPE_ID,
            &link.signing_bytes(1).unwrap(),
        )
        .is_err());

        let mut impostor = MockYubi::new(0x13, 0x14);
        impostor.public = parent.public.clone();
        assert!(matches!(
            make_yubi_puk_rotation_link(&base, &impostor, &rotations),
            Err(Error::Verification)
        ));
        assert!(make_yubi_puk_rotation_link(&base, &parent, &[]).is_err());
    }

    #[test]
    fn yubi_puk_boxes_round_trip_without_temp_dh_and_reject_wrong_bindings() {
        let host =
            EntityId::from_bytes([vec![foks_proto::ENTITY_HOST], vec![0x41; 32]].concat()).unwrap();
        let wrong_host =
            EntityId::from_bytes([vec![foks_proto::ENTITY_HOST], vec![0x42; 32]].concat()).unwrap();
        let parent = MockYubi::new(0x31, 0x32);
        let receiver_a = MockYubi::new(0x33, 0x34);
        let receiver_b = MockYubi::new(0x35, 0x36);
        let wrong_parent = MockYubi::new(0x37, 0x38);
        let member_seed = SecretSeed::new([0x51; 32]);
        let owner_seed = SecretSeed::new([0x52; 32]);
        let inputs = [
            YubiPukBoxInput {
                seed: &member_seed,
                generation: 2,
                role: Role::member(0),
                receiver: &receiver_a.public.device,
            },
            YubiPukBoxInput {
                seed: &owner_seed,
                generation: 3,
                role: Role::OWNER,
                receiver: &receiver_b.public.device,
            },
        ];
        let randomness = [
            PukBoxRandomness {
                kem_message: [0x61; 32],
                nonce: [0x62; 16],
            },
            PukBoxRandomness {
                kem_message: [0x63; 32],
                nonce: [0x64; 16],
            },
        ];
        let set_randomness = YubiPukBoxSetRandomness {
            ephemeral_secret: [0x65; 32],
            time: 1_724_000_000_000,
        };
        let boxes = seal_yubi_puk_boxes(
            &host,
            &parent,
            [0x71; 16],
            &inputs,
            &randomness,
            set_randomness,
        )
        .unwrap();
        let repeated = seal_yubi_puk_boxes(
            &host,
            &parent,
            [0x71; 16],
            &inputs,
            &randomness,
            YubiPukBoxSetRandomness {
                ephemeral_secret: [0x65; 32],
                time: 1_724_000_000_000,
            },
        )
        .unwrap();
        assert_eq!(boxes.encoded(), repeated.encoded());
        assert!(boxes.temp_dh_key.is_none());
        assert!(boxes
            .boxes
            .iter()
            .all(|boxed| boxed.hybrid.dh_type == 2 && boxed.hybrid.sender_dh.is_none()));

        let member_public = derive_shared_public(&member_seed, ENTITY_PUK_VERIFY).unwrap();
        let owner_public = derive_shared_public(&owner_seed, ENTITY_PUK_VERIFY).unwrap();
        let member_parcel =
            PukParcel::from_box_set(&boxes, 0, parent.entity_id().clone(), Vec::new()).unwrap();
        let owner_parcel =
            PukParcel::from_box_set(&boxes, 1, parent.entity_id().clone(), Vec::new()).unwrap();
        let opened_member = open_puk_parcel_with_for_role(
            &member_parcel,
            &receiver_a,
            parent.hepk(),
            &member_public.verify_key,
            &member_public.hepk,
            2,
            &host,
            Role::member(0),
        )
        .unwrap();
        let opened_owner = open_puk_parcel_with_for_role(
            &owner_parcel,
            &receiver_b,
            parent.hepk(),
            &owner_public.verify_key,
            &owner_public.hepk,
            3,
            &host,
            Role::OWNER,
        )
        .unwrap();
        assert_eq!(opened_member.seed, member_seed);
        assert_eq!(opened_owner.seed, owner_seed);

        assert!(open_puk_parcel_with_for_role(
            &member_parcel,
            &receiver_b,
            parent.hepk(),
            &member_public.verify_key,
            &member_public.hepk,
            2,
            &host,
            Role::member(0),
        )
        .is_err());
        let mut rebound_receiver = member_parcel.clone();
        rebound_receiver.target = receiver_b.entity_id().clone();
        assert!(open_puk_parcel_with_for_role(
            &rebound_receiver,
            &receiver_b,
            parent.hepk(),
            &member_public.verify_key,
            &member_public.hepk,
            2,
            &host,
            Role::member(0),
        )
        .is_err());
        assert!(open_puk_parcel_with_for_role(
            &member_parcel,
            &receiver_a,
            wrong_parent.hepk(),
            &member_public.verify_key,
            &member_public.hepk,
            2,
            &host,
            Role::member(0),
        )
        .is_err());
        assert!(matches!(
            open_puk_parcel_with_for_role(
                &member_parcel,
                &receiver_a,
                parent.hepk(),
                &member_public.verify_key,
                &member_public.hepk,
                2,
                &wrong_host,
                Role::member(0),
            ),
            Err(Error::PukBinding)
        ));
        let mut rebound_generation = member_parcel.clone();
        rebound_generation.generation = 4;
        assert!(matches!(
            open_puk_parcel_with_for_role(
                &rebound_generation,
                &receiver_a,
                parent.hepk(),
                &member_public.verify_key,
                &member_public.hepk,
                4,
                &host,
                Role::member(0),
            ),
            Err(Error::PukBinding)
        ));
        let mut rebound_role = member_parcel.clone();
        rebound_role.role = Role::ADMIN;
        assert!(matches!(
            open_puk_parcel_with_for_role(
                &rebound_role,
                &receiver_a,
                parent.hepk(),
                &member_public.verify_key,
                &member_public.hepk,
                2,
                &host,
                Role::ADMIN,
            ),
            Err(Error::PukBinding)
        ));
        let wrong_puk =
            derive_shared_public(&SecretSeed::new([0x53; 32]), ENTITY_PUK_VERIFY).unwrap();
        assert!(matches!(
            open_puk_parcel_with_for_role(
                &member_parcel,
                &receiver_a,
                parent.hepk(),
                &wrong_puk.verify_key,
                &wrong_puk.hepk,
                2,
                &host,
                Role::member(0),
            ),
            Err(Error::PukBinding)
        ));
    }

    #[test]
    fn yubi_puk_boxes_round_trip_for_mixed_recipient_curves() {
        let host =
            EntityId::from_bytes([vec![foks_proto::ENTITY_HOST], vec![0x43; 32]].concat()).unwrap();
        let parent = MockYubi::new(0x39, 0x3a);
        let yubi_receiver = MockYubi::new(0x3b, 0x3c);
        let software_seed = SecretSeed::new([0x3d; 32]);
        let software_receiver = derive_device_public(&software_seed).unwrap();
        let member_seed = SecretSeed::new([0x54; 32]);
        let owner_seed = SecretSeed::new([0x55; 32]);
        let inputs = [
            YubiPukBoxInput {
                seed: &member_seed,
                generation: 2,
                role: Role::member(0),
                receiver: &software_receiver,
            },
            YubiPukBoxInput {
                seed: &owner_seed,
                generation: 3,
                role: Role::OWNER,
                receiver: &yubi_receiver.public.device,
            },
        ];
        let boxes = seal_yubi_puk_boxes(
            &host,
            &parent,
            [0x72; 16],
            &inputs,
            &[
                PukBoxRandomness {
                    kem_message: [0x66; 32],
                    nonce: [0x67; 16],
                },
                PukBoxRandomness {
                    kem_message: [0x68; 32],
                    nonce: [0x69; 16],
                },
            ],
            YubiPukBoxSetRandomness {
                ephemeral_secret: [0x6a; 32],
                time: 1_724_000_000_001,
            },
        )
        .unwrap();
        assert!(boxes.temp_dh_key.is_some());
        assert_eq!(boxes.boxes[0].hybrid.dh_type, 1);
        assert_eq!(boxes.boxes[1].hybrid.dh_type, 2);

        let member_public = derive_shared_public(&member_seed, ENTITY_PUK_VERIFY).unwrap();
        let owner_public = derive_shared_public(&owner_seed, ENTITY_PUK_VERIFY).unwrap();
        let software_parcel =
            PukParcel::from_box_set(&boxes, 0, parent.entity_id().clone(), Vec::new()).unwrap();
        let yubi_parcel =
            PukParcel::from_box_set(&boxes, 1, parent.entity_id().clone(), Vec::new()).unwrap();
        assert_eq!(
            open_puk_parcel_for_role(
                &software_parcel,
                &software_seed,
                parent.hepk(),
                &member_public.verify_key,
                &member_public.hepk,
                2,
                &host,
                Role::member(0),
            )
            .unwrap()
            .seed,
            member_seed
        );
        assert_eq!(
            open_puk_parcel_with_for_role(
                &yubi_parcel,
                &yubi_receiver,
                parent.hepk(),
                &owner_public.verify_key,
                &owner_public.hepk,
                3,
                &host,
                Role::OWNER,
            )
            .unwrap()
            .seed,
            owner_seed
        );

        let mut tampered = software_parcel.clone();
        let Signature::Ecdsa(signature) = &mut tampered
            .temp_dh_key
            .as_mut()
            .expect("mixed set has a temporary key")
            .signature
        else {
            panic!("Yubi temporary key has the wrong signature type")
        };
        signature[0] ^= 1;
        assert!(open_puk_parcel_for_role(
            &tampered,
            &software_seed,
            parent.hepk(),
            &member_public.verify_key,
            &member_public.hepk,
            2,
            &host,
            Role::member(0),
        )
        .is_err());
        // Go ignores the set-level temporary key for same-curve boxes.
        let mut same_curve = yubi_parcel;
        same_curve.temp_dh_key = tampered.temp_dh_key;
        assert!(open_puk_parcel_with_for_role(
            &same_curve,
            &yubi_receiver,
            parent.hepk(),
            &owner_public.verify_key,
            &owner_public.hepk,
            3,
            &host,
            Role::OWNER,
        )
        .is_ok());
    }

    #[test]
    fn mixed_puk_boxers_support_backup_and_bot_token_recipients() {
        let host =
            EntityId::from_bytes([vec![foks_proto::ENTITY_HOST], vec![0x4a; 32]].concat()).unwrap();
        let software_seed = SecretSeed::new([0x4b; 32]);
        let backup = BackupKey::from_seed([0x01; BACKUP_SEED_BYTES])
            .unwrap()
            .into_key_material()
            .unwrap();
        let backup_public = DevicePublicMaterial {
            id: backup.entity_id().clone(),
            hepk: backup.hepk().clone(),
        };
        let bot_id =
            EntityId::from_bytes([vec![foks_proto::ENTITY_BOT_TOKEN_KEY], vec![0x4c; 32]].concat())
                .unwrap();
        let bot_public = DevicePublicMaterial {
            id: bot_id,
            hepk: derive_device_public(&SecretSeed::new([0x4d; 32]))
                .unwrap()
                .hepk,
        };
        let puk_seed = SecretSeed::new([0x58; 32]);
        let inputs = [&backup_public, &bot_public].map(|receiver| SoftwarePukBoxInput {
            seed: &puk_seed,
            generation: 2,
            role: Role::OWNER,
            receiver,
        });
        let software_boxes = seal_software_puk_boxes_mixed(
            &host,
            &software_seed,
            [0x74; 16],
            &inputs,
            &[
                PukBoxRandomness {
                    kem_message: [0x70; 32],
                    nonce: [0x71; 16],
                },
                PukBoxRandomness {
                    kem_message: [0x72; 32],
                    nonce: [0x73; 16],
                },
            ],
            SoftwarePukBoxSetRandomness {
                ephemeral_secret: [0x75; 32],
                time: 1_724_000_000_003,
            },
        )
        .unwrap();
        assert!(software_boxes.temp_dh_key.is_none());

        let parent = MockYubi::new(0x4e, 0x4f);
        let yubi_boxes = seal_yubi_puk_boxes(
            &host,
            &parent,
            [0x76; 16],
            &[YubiPukBoxInput {
                seed: &puk_seed,
                generation: 2,
                role: Role::OWNER,
                receiver: &backup_public,
            }],
            &[PukBoxRandomness {
                kem_message: [0x77; 32],
                nonce: [0x78; 16],
            }],
            YubiPukBoxSetRandomness {
                ephemeral_secret: [0x79; 32],
                time: 1_724_000_000_004,
            },
        )
        .unwrap();
        assert!(yubi_boxes.temp_dh_key.is_some());
        let public_puk = derive_shared_public(&puk_seed, ENTITY_PUK_VERIFY).unwrap();
        let parcel =
            PukParcel::from_box_set(&yubi_boxes, 0, parent.entity_id().clone(), Vec::new())
                .unwrap();
        assert_eq!(
            open_puk_parcel_with_for_role(
                &parcel,
                &backup,
                parent.hepk(),
                &public_puk.verify_key,
                &public_puk.hepk,
                2,
                &host,
                Role::OWNER,
            )
            .unwrap()
            .seed,
            puk_seed
        );
    }

    #[test]
    fn software_puk_boxes_round_trip_for_mixed_recipient_curves() {
        let host =
            EntityId::from_bytes([vec![foks_proto::ENTITY_HOST], vec![0x44; 32]].concat()).unwrap();
        let sender_seed = SecretSeed::new([0x45; 32]);
        let sender = derive_device_public(&sender_seed).unwrap();
        let software_seed = SecretSeed::new([0x46; 32]);
        let software_receiver = derive_device_public(&software_seed).unwrap();
        let yubi_receiver = MockYubi::new(0x47, 0x48);
        let member_seed = SecretSeed::new([0x56; 32]);
        let owner_seed = SecretSeed::new([0x57; 32]);
        let inputs = [
            SoftwarePukBoxInput {
                seed: &member_seed,
                generation: 2,
                role: Role::member(0),
                receiver: &software_receiver,
            },
            SoftwarePukBoxInput {
                seed: &owner_seed,
                generation: 3,
                role: Role::OWNER,
                receiver: &yubi_receiver.public.device,
            },
        ];
        let boxes = seal_software_puk_boxes_mixed(
            &host,
            &sender_seed,
            [0x73; 16],
            &inputs,
            &[
                PukBoxRandomness {
                    kem_message: [0x6b; 32],
                    nonce: [0x6c; 16],
                },
                PukBoxRandomness {
                    kem_message: [0x6d; 32],
                    nonce: [0x6e; 16],
                },
            ],
            SoftwarePukBoxSetRandomness {
                ephemeral_secret: [0x6f; 32],
                time: 1_724_000_000_002,
            },
        )
        .unwrap();
        assert!(matches!(
            boxes.temp_dh_key.as_ref().map(|key| &key.signature),
            Some(Signature::Ed25519(_))
        ));

        let member_public = derive_shared_public(&member_seed, ENTITY_PUK_VERIFY).unwrap();
        let owner_public = derive_shared_public(&owner_seed, ENTITY_PUK_VERIFY).unwrap();
        let software_parcel =
            PukParcel::from_box_set(&boxes, 0, sender.id.clone(), Vec::new()).unwrap();
        let yubi_parcel =
            PukParcel::from_box_set(&boxes, 1, sender.id.clone(), Vec::new()).unwrap();
        assert_eq!(
            open_puk_parcel_for_role(
                &software_parcel,
                &software_seed,
                &sender.hepk,
                &member_public.verify_key,
                &member_public.hepk,
                2,
                &host,
                Role::member(0),
            )
            .unwrap()
            .seed,
            member_seed
        );
        assert_eq!(
            open_puk_parcel_with_for_role(
                &yubi_parcel,
                &yubi_receiver,
                &sender.hepk,
                &owner_public.verify_key,
                &owner_public.hepk,
                3,
                &host,
                Role::OWNER,
            )
            .unwrap()
            .seed,
            owner_seed
        );
    }

    #[test]
    fn single_owner_adhoc_team_matches_official_go_fixture() {
        let expected = UserLink::decode(&mutation_fixture("adhoc-team-link.snowp")).unwrap();
        let change = expected.decode_team_group_change().unwrap();
        let membership =
            UserLink::decode(&mutation_fixture("adhoc-membership-link.snowp")).unwrap();
        let device_seed = SecretSeed::new(user_fixture("device-seed.bin").try_into().unwrap());
        let owner_seed = SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap());
        let ptk_seeds = [
            "adhoc-ptk-member-min-seed.bin",
            "adhoc-ptk-member-seed.bin",
            "adhoc-ptk-admin-seed.bin",
            "adhoc-ptk-owner-seed.bin",
        ]
        .map(|name| SecretSeed::new(mutation_fixture(name).try_into().unwrap()));
        let owner = &change.changes[0];
        let material = make_single_owner_adhoc_team(
            &AdHocTeamInput {
                user: &owner.party,
                host: &change.host,
                root: &change.root,
                time: change.time,
                owner_puk_generation: owner.keys.as_ref().unwrap().generation,
                membership_sequence: 1,
                membership_previous: None,
                next_tree_location: mutation_fixture("adhoc-next-tree-location.bin")
                    .try_into()
                    .unwrap(),
                subchain_tree_location: mutation_fixture("adhoc-subchain-tree-location.bin")
                    .try_into()
                    .unwrap(),
                membership_next_tree_location: mutation_fixture(
                    "adhoc-membership-next-tree-location.bin",
                )
                .try_into()
                .unwrap(),
            },
            &device_seed,
            &owner_seed,
            [&ptk_seeds[0], &ptk_seeds[1], &ptk_seeds[2], &ptk_seeds[3]],
        )
        .unwrap();
        assert_eq!(
            material.link.encoded().unwrap(),
            expected.encoded().unwrap()
        );
        assert_eq!(
            material.membership_link.encoded().unwrap(),
            membership.encoded().unwrap()
        );
        let expected_team = match decode(&mutation_fixture("adhoc-team-id.snowp")).unwrap() {
            Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
            other => panic!("expected ad-hoc TeamID fixture, got {other:?}"),
        };
        assert_eq!(material.team, expected_team);

        let owner_public =
            derive_shared_public(&owner_seed, foks_proto::ENTITY_PUK_VERIFY).unwrap();
        let roles = [
            Role::member(-0x4000),
            Role::member(0),
            Role::ADMIN,
            Role::OWNER,
        ];
        let inputs = ptk_seeds
            .iter()
            .zip(roles)
            .map(|(seed, role)| SharedKeyBoxInput {
                seed,
                generation: 1,
                role,
                receiver_id: &owner.party,
                receiver_host: None,
                receiver_hepk: &owner_public.hepk,
                receiver_role: Role::OWNER,
                receiver_generation: owner.keys.as_ref().unwrap().generation,
            })
            .collect::<Vec<_>>();
        let randomness = (0_u8..4)
            .map(|offset| PukBoxRandomness {
                kem_message: [31 + offset; 32],
                nonce: [41 + offset; 16],
            })
            .collect::<Vec<_>>();
        let boxes = seal_shared_key_boxes(
            &change.host,
            &owner_seed,
            &owner_public.hepk,
            [51; 16],
            &inputs,
            &randomness,
        )
        .unwrap();
        let receiver = SharedKeyDecapsulator::new(&owner_seed, owner.party.clone()).unwrap();
        for (index, shared) in boxes.boxes.iter().enumerate() {
            let parcel = PukParcel {
                generation: shared.generation,
                role: shared.role,
                hybrid: shared.hybrid.clone(),
                target: shared.target.entity.clone(),
                target_host: shared.target.host.clone(),
                target_role: shared.target.role,
                target_generation: shared.target.generation,
                sender: owner_public.verify_key.clone(),
                box_id: boxes.box_id,
                temp_dh_key: boxes.temp_dh_key.clone(),
                seed_chain: Vec::new(),
            };
            let clear = open_shared_key_parcel_with(
                &parcel,
                &receiver,
                &owner_public.hepk,
                &material.ptks[index].verify_key,
                &material.ptks[index].hepk,
                shared.generation,
                &change.host,
                Role::OWNER,
                owner.keys.as_ref().unwrap().generation,
                roles[index],
                ENTITY_PTK_VERIFY,
            )
            .unwrap();
            assert_eq!(clear.seed, ptk_seeds[index]);
        }
        let wrong_sender = derive_shared_public(&ptk_seeds[0], ENTITY_PTK_VERIFY).unwrap();
        assert!(seal_shared_key_boxes(
            &change.host,
            &owner_seed,
            &wrong_sender.hepk,
            [51; 16],
            &inputs,
            &randomness,
        )
        .is_err());
        assert!(make_single_owner_adhoc_team(
            &AdHocTeamInput {
                user: &owner.party,
                host: &change.host,
                root: &change.root,
                time: change.time,
                owner_puk_generation: 0,
                membership_sequence: 1,
                membership_previous: None,
                next_tree_location: [1; 32],
                subchain_tree_location: [1; 32],
                membership_next_tree_location: [1; 32],
            },
            &device_seed,
            &owner_seed,
            [&ptk_seeds[0], &ptk_seeds[1], &ptk_seeds[2], &ptk_seeds[3],],
        )
        .is_err());
    }

    #[test]
    fn single_owner_named_team_matches_official_go_fixture() {
        let expected = UserLink::decode(&mutation_fixture("named-team-link.snowp")).unwrap();
        let change = expected.decode_team_group_change().unwrap();
        let expected_membership =
            UserLink::decode(&mutation_fixture("named-membership-link.snowp")).unwrap();
        let device_seed = SecretSeed::new(user_fixture("device-seed.bin").try_into().unwrap());
        let owner_seed = SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap());
        let ptk_seeds = [
            "adhoc-ptk-member-min-seed.bin",
            "adhoc-ptk-member-seed.bin",
            "adhoc-ptk-admin-seed.bin",
            "adhoc-ptk-owner-seed.bin",
        ]
        .map(|name| SecretSeed::new(mutation_fixture(name).try_into().unwrap()));
        let removal_key = SecretSeed::new(
            mutation_fixture("named-removal-key.bin")
                .try_into()
                .unwrap(),
        );
        let owner = &change.changes[0];
        let material = make_single_owner_named_team(
            &NamedTeamInput {
                user: &owner.party,
                host: &change.host,
                root: &change.root,
                time: change.time,
                owner_puk_generation: owner.keys.as_ref().unwrap().generation,
                membership_sequence: 1,
                membership_previous: None,
                normalized_name: b"auditteam",
                name_sequence: 7,
                team_name_commitment_key: mutation_fixture("named-team-name-commitment-key.bin")
                    .try_into()
                    .unwrap(),
                next_tree_location: mutation_fixture("named-next-tree-location.bin")
                    .try_into()
                    .unwrap(),
                subchain_tree_location: mutation_fixture("named-subchain-tree-location.bin")
                    .try_into()
                    .unwrap(),
                membership_next_tree_location: mutation_fixture(
                    "named-membership-next-tree-location.bin",
                )
                .try_into()
                .unwrap(),
            },
            &device_seed,
            &owner_seed,
            [&ptk_seeds[0], &ptk_seeds[1], &ptk_seeds[2], &ptk_seeds[3]],
            &removal_key,
        )
        .unwrap();
        assert_eq!(
            material.link.encoded().unwrap(),
            expected.encoded().unwrap()
        );
        assert_eq!(
            material.membership_link.encoded().unwrap(),
            expected_membership.encoded().unwrap()
        );
        let expected_team = match decode(&mutation_fixture("named-team-id.snowp")).unwrap() {
            Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
            other => panic!("expected named TeamID fixture, got {other:?}"),
        };
        assert_eq!(material.team, expected_team);
        assert_eq!(
            material.removal_key_commitment,
            owner.keys.as_ref().unwrap().removal_key_commitment.unwrap()
        );
        let removal_box =
            TeamRemovalBoxData::decode(&mutation_fixture("named-removal-boxes.snowp")).unwrap();
        let member_receiver = SharedKeyDecapsulator::new(&owner_seed, owner.party.clone()).unwrap();
        let opened_member = open_team_removal_key(
            &removal_box.member_box,
            &member_receiver,
            Role::OWNER,
            owner.keys.as_ref().unwrap().generation,
            &removal_box.commitment,
            &removal_box.metadata,
        )
        .unwrap();
        let team_receiver =
            SharedKeyDecapsulator::new(&ptk_seeds[2], material.team.clone()).unwrap();
        let opened_team = open_team_removal_key(
            &removal_box.team_box,
            &team_receiver,
            Role::ADMIN,
            1,
            &removal_box.commitment,
            &removal_box.metadata,
        )
        .unwrap();
        assert_eq!(opened_member, removal_key);
        assert_eq!(opened_team, removal_key);
    }

    #[test]
    fn team_admin_opens_official_member_removal_key_box() {
        let addition = UserLink::decode(&mutation_fixture("add-member-link.snowp"))
            .unwrap()
            .decode_team_group_change()
            .unwrap();
        let member = &addition.changes[0];
        let commitment = member
            .keys
            .as_ref()
            .unwrap()
            .removal_key_commitment
            .unwrap();
        let admin_seed = SecretSeed::new(
            mutation_fixture("adhoc-ptk-admin-seed.bin")
                .try_into()
                .unwrap(),
        );
        let receiver = SharedKeyDecapsulator::new(&admin_seed, addition.team.clone()).unwrap();
        let boxed =
            TeamRemovalKeyBox::decode(&mutation_fixture("team-removal-admin-box.snowp")).unwrap();
        let (opened, metadata) = open_team_removal_key_for_member(
            &boxed,
            &receiver,
            &TeamRemovalKeyExpectation {
                commitment: &commitment,
                team: &addition.team,
                host: &addition.host,
                member: &member.party,
                member_host: &addition.host,
                source_role: member.source_role,
            },
        )
        .unwrap();
        assert_eq!(
            opened.as_slice(),
            mutation_fixture("add-member-removal-key.bin")
        );
        assert_eq!(metadata.destination_role, member.role);
        assert_eq!(metadata.team_sequence, addition.seqno);
    }

    #[test]
    fn federated_shared_key_box_binds_remote_host_in_target_and_plaintext() {
        let sender_seed = SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap());
        let receiver_seed = SecretSeed::new(
            mutation_fixture("add-member-target-puk-seed.bin")
                .try_into()
                .unwrap(),
        );
        let sender = derive_shared_public(&sender_seed, foks_proto::ENTITY_PUK_VERIFY).unwrap();
        let receiver = derive_shared_public(&receiver_seed, foks_proto::ENTITY_PUK_VERIFY).unwrap();
        let receiver_id = match decode(&mutation_fixture("add-member-target-uid.snowp")).unwrap() {
            Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
            other => panic!("expected target user fixture, got {other:?}"),
        };
        let local_host = UserLink::decode(&mutation_fixture("named-team-link.snowp"))
            .unwrap()
            .decode_team_group_change()
            .unwrap()
            .host;
        let mut remote_bytes = local_host.as_bytes().to_vec();
        remote_bytes[32] ^= 1;
        let remote_host = EntityId::from_bytes(remote_bytes).unwrap();
        let shared_seed = SecretSeed::new([0x55; 32]);
        let boxes = seal_shared_key_boxes(
            &local_host,
            &sender_seed,
            &sender.hepk,
            [0x33; 16],
            &[SharedKeyBoxInput {
                seed: &shared_seed,
                generation: 4,
                role: Role::member(0),
                receiver_id: &receiver_id,
                receiver_host: Some(&remote_host),
                receiver_hepk: &receiver.hepk,
                receiver_role: Role::OWNER,
                receiver_generation: 3,
            }],
            &[PukBoxRandomness {
                kem_message: [0x44; 32],
                nonce: [0x22; 16],
            }],
        )
        .unwrap();
        assert_eq!(boxes.boxes[0].target.host, Some(remote_host.clone()));
        let decapsulator = SharedKeyDecapsulator::new(&receiver_seed, receiver_id).unwrap();
        let sender_dh = sender.hepk.curve25519().copied().unwrap();
        let plaintext = open_hybrid_box(
            &boxes.boxes[0].hybrid,
            &decapsulator,
            &DhPublicKey::Curve25519(sender_dh),
            SHARED_KEY_SEED_TYPE_ID,
        )
        .unwrap();
        let clear = SharedKeySeed::decode(&plaintext).unwrap();
        assert_eq!(clear.host, remote_host);
        assert_eq!(clear.seed, shared_seed);
    }

    #[test]
    fn additive_named_team_link_matches_official_go_fixture() {
        let eldest = UserLink::decode(&mutation_fixture("named-team-link.snowp")).unwrap();
        let expected = UserLink::decode(&mutation_fixture("add-member-link.snowp")).unwrap();
        let change = expected.decode_team_group_change().unwrap();
        let actor_seed = SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap());
        let target_seed = SecretSeed::new(
            mutation_fixture("add-member-target-puk-seed.bin")
                .try_into()
                .unwrap(),
        );
        let target = derive_shared_public(&target_seed, foks_proto::ENTITY_PUK_VERIFY).unwrap();
        let removal_key = SecretSeed::new(
            mutation_fixture("add-member-removal-key.bin")
                .try_into()
                .unwrap(),
        );
        let member = &change.changes[0];
        let material = make_add_local_team_member_link(
            &AddLocalTeamMemberInput {
                actor: &change.signer_owner.party,
                actor_source_role: change.signer_owner.source_role,
                team: &change.team,
                host: &change.host,
                sequence: change.seqno,
                previous: prefixed_hash(foks_proto::LINK_OUTER_TYPE_ID, &eldest.encoded().unwrap()),
                root: &change.root,
                time: change.time,
                next_tree_location: mutation_fixture("add-member-next-tree-location.bin")
                    .try_into()
                    .unwrap(),
                member: &member.party,
                member_source_role: member.source_role,
                member_destination_role: member.role,
                member_generation: member.keys.as_ref().unwrap().generation,
                member_public: &target,
            },
            &actor_seed,
            &removal_key,
        )
        .unwrap();
        assert_eq!(material.link.decode_team_group_change().unwrap(), change);
        assert_eq!(
            material.link.encoded().unwrap(),
            expected.encoded().unwrap()
        );
        assert_eq!(
            material.removal_key_commitment,
            member
                .keys
                .as_ref()
                .unwrap()
                .removal_key_commitment
                .unwrap()
        );
        assert_eq!(
            material.removal_key_commitment,
            team_removal_key_commitment(&removal_key).unwrap()
        );
    }

    #[test]
    fn named_team_demotion_matches_official_go_fixture() {
        let addition = UserLink::decode(&mutation_fixture("add-member-link.snowp")).unwrap();
        let expected = UserLink::decode(&mutation_fixture("demote-member-link.snowp")).unwrap();
        let change = expected.decode_team_group_change().unwrap();
        let member = &change.changes[0];
        let actor_seed = SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap());
        let target_seed = SecretSeed::new(
            mutation_fixture("add-member-target-puk-seed.bin")
                .try_into()
                .unwrap(),
        );
        let target = derive_shared_public(&target_seed, foks_proto::ENTITY_PUK_VERIFY).unwrap();
        let rotation_seed = SecretSeed::new(
            mutation_fixture("demote-member-ptk-seed.bin")
                .try_into()
                .unwrap(),
        );
        let rotations = [TeamPtkRotation {
            role: Role::member(0),
            generation: 2,
            seed: &rotation_seed,
        }];
        let material = make_change_team_member_link(
            &ChangeTeamMemberInput {
                actor: &change.signer_owner.party,
                actor_source_role: change.signer_owner.source_role,
                team: &change.team,
                host: &change.host,
                sequence: change.seqno,
                previous: prefixed_hash(
                    foks_proto::LINK_OUTER_TYPE_ID,
                    &addition.encoded().unwrap(),
                ),
                root: &change.root,
                time: change.time,
                next_tree_location: mutation_fixture("demote-member-next-tree-location.bin")
                    .try_into()
                    .unwrap(),
                member: &member.party,
                member_host: member.scoped_host.as_ref(),
                member_source_role: member.source_role,
                destination_role: member.role,
                member_generation: Some(member.keys.as_ref().unwrap().generation),
                member_public: Some(&target),
                member_index_range: None,
            },
            &actor_seed,
            &rotations,
        )
        .unwrap();
        assert_eq!(
            material.link.encoded().unwrap(),
            expected.encoded().unwrap()
        );
        assert_eq!(material.link.decode_team_group_change().unwrap(), change);
    }

    #[test]
    fn role_only_team_promotion_needs_no_ptk_rotation() {
        let addition = UserLink::decode(&mutation_fixture("add-member-link.snowp")).unwrap();
        let prior = addition.decode_team_group_change().unwrap();
        let member = &prior.changes[0];
        let actor_seed = SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap());
        let target_seed = SecretSeed::new(
            mutation_fixture("add-member-target-puk-seed.bin")
                .try_into()
                .unwrap(),
        );
        let target = derive_shared_public(&target_seed, foks_proto::ENTITY_PUK_VERIFY).unwrap();
        let material = make_change_team_member_link(
            &ChangeTeamMemberInput {
                actor: &prior.signer_owner.party,
                actor_source_role: prior.signer_owner.source_role,
                team: &prior.team,
                host: &prior.host,
                sequence: prior.seqno + 1,
                previous: prefixed_hash(
                    foks_proto::LINK_OUTER_TYPE_ID,
                    &addition.encoded().unwrap(),
                ),
                root: &prior.root,
                time: prior.time + 1,
                next_tree_location: [7; 32],
                member: &member.party,
                member_host: member.scoped_host.as_ref(),
                member_source_role: member.source_role,
                destination_role: Role::OWNER,
                member_generation: Some(member.keys.as_ref().unwrap().generation),
                member_public: Some(&target),
                member_index_range: None,
            },
            &actor_seed,
            &[],
        )
        .unwrap();
        let change = material.link.decode_team_group_change().unwrap();
        assert!(change.shared_keys.is_empty());
        assert_eq!(change.changes[0].role, Role::OWNER);
        assert_eq!(material.link.signatures().len(), 1);
    }

    #[test]
    fn named_team_removal_rotation_matches_official_go_fixture() {
        let addition = UserLink::decode(&mutation_fixture("add-member-link.snowp")).unwrap();
        let expected = UserLink::decode(&mutation_fixture("remove-member-link.snowp")).unwrap();
        let change = expected.decode_team_group_change().unwrap();
        let actor_seed = SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap());
        let old_seeds = [
            SecretSeed::new(
                mutation_fixture("adhoc-ptk-member-min-seed.bin")
                    .try_into()
                    .unwrap(),
            ),
            SecretSeed::new(
                mutation_fixture("adhoc-ptk-member-seed.bin")
                    .try_into()
                    .unwrap(),
            ),
        ];
        let new_seeds = [
            SecretSeed::new(
                mutation_fixture("remove-member-ptk-member-min-seed.bin")
                    .try_into()
                    .unwrap(),
            ),
            SecretSeed::new(
                mutation_fixture("remove-member-ptk-member-seed.bin")
                    .try_into()
                    .unwrap(),
            ),
        ];
        let roles = [Role::member(-0x4000), Role::member(0)];
        let rotations = new_seeds
            .iter()
            .zip(roles)
            .map(|(seed, role)| TeamPtkRotation {
                role,
                generation: 2,
                seed,
            })
            .collect::<Vec<_>>();
        let removed = &change.changes[0];
        let material = make_remove_local_team_member_link(
            &RemoveLocalTeamMemberInput {
                actor: &change.signer_owner.party,
                actor_source_role: change.signer_owner.source_role,
                team: &change.team,
                host: &change.host,
                sequence: change.seqno,
                previous: prefixed_hash(
                    foks_proto::LINK_OUTER_TYPE_ID,
                    &addition.encoded().unwrap(),
                ),
                root: &change.root,
                time: change.time,
                next_tree_location: mutation_fixture("remove-member-next-tree-location.bin")
                    .try_into()
                    .unwrap(),
                member: &removed.party,
                member_source_role: removed.source_role,
            },
            &actor_seed,
            &rotations,
        )
        .unwrap();
        assert_eq!(
            material.link.encoded().unwrap(),
            expected.encoded().unwrap()
        );
        assert_eq!(material.link.decode_team_group_change().unwrap(), change);

        let duplicated = [
            TeamPtkRotation {
                role: roles[0],
                generation: 2,
                seed: &new_seeds[0],
            },
            TeamPtkRotation {
                role: roles[1],
                generation: 2,
                seed: &new_seeds[0],
            },
        ];
        assert!(make_remove_local_team_member_link(
            &RemoveLocalTeamMemberInput {
                actor: &change.signer_owner.party,
                actor_source_role: change.signer_owner.source_role,
                team: &change.team,
                host: &change.host,
                sequence: change.seqno,
                previous: change.previous.unwrap(),
                root: &change.root,
                time: change.time,
                next_tree_location: [1; 32],
                member: &removed.party,
                member_source_role: removed.source_role,
            },
            &actor_seed,
            &duplicated,
        )
        .is_err());

        let rotated_boxes =
            SharedKeyBoxSet::decode(&mutation_fixture("remove-member-ptk-boxes.snowp")).unwrap();
        let actor_public =
            derive_shared_public(&actor_seed, foks_proto::ENTITY_PUK_VERIFY).unwrap();
        let receiver =
            SharedKeyDecapsulator::new(&actor_seed, change.signer_owner.party.clone()).unwrap();
        for (index, role) in roles.into_iter().enumerate() {
            let expected_box = foks_proto::SeedChainBox::decode(&mutation_fixture(
                [
                    "remove-member-seed-chain-member-min.snowp",
                    "remove-member-seed-chain-member.snowp",
                ][index],
            ))
            .unwrap();
            let actual = seal_puk_seed_chain_box(
                &new_seeds[index],
                &old_seeds[index],
                &change.signer_owner.party,
                &change.host,
                1,
                role,
                expected_box.secret_box.nonce,
            )
            .unwrap();
            assert_eq!(actual, expected_box);
            let shared = &rotated_boxes.boxes[index];
            let parcel = PukParcel {
                generation: shared.generation,
                role: shared.role,
                hybrid: shared.hybrid.clone(),
                target: shared.target.entity.clone(),
                target_host: shared.target.host.clone(),
                target_role: shared.target.role,
                target_generation: shared.target.generation,
                sender: actor_public.verify_key.clone(),
                box_id: rotated_boxes.box_id,
                temp_dh_key: rotated_boxes.temp_dh_key.clone(),
                seed_chain: vec![expected_box],
            };
            let clear = open_shared_key_parcel_with(
                &parcel,
                &receiver,
                &actor_public.hepk,
                &material.ptks[index].verify_key,
                &material.ptks[index].hepk,
                shared.generation,
                &change.host,
                Role::OWNER,
                2,
                role,
                ENTITY_PTK_VERIFY,
            )
            .unwrap();
            let history = open_shared_key_seed_chain(
                clear,
                &parcel,
                &change.signer_owner.party,
                &change.host,
            )
            .unwrap();
            assert_eq!(history.len(), 2);
            assert_eq!(history[0].seed, old_seeds[index]);
            assert_eq!(history[1].seed, new_seeds[index]);
        }

        let removal_key = SecretSeed::new(
            mutation_fixture("add-member-removal-key.bin")
                .try_into()
                .unwrap(),
        );
        let expected_proof = foks_proto::TeamRemovalAndCommitment::decode(&mutation_fixture(
            "remove-member-proof.snowp",
        ))
        .unwrap();
        let actual_proof =
            make_team_removal_proof(&removal_key, expected_proof.removal.payload.clone()).unwrap();
        assert_eq!(actual_proof, expected_proof);
        assert_eq!(
            actual_proof.encoded().unwrap(),
            mutation_fixture("remove-member-proof.snowp")
        );
    }

    #[test]
    fn yubi_parent_signs_adhoc_membership_without_exporting_its_key() {
        use p256::ecdsa::{
            signature::hazmat::PrehashSigner as _, Signature as P256Signature,
            SigningKey as P256SigningKey,
        };

        struct FixtureYubi {
            id: EntityId,
            hepk: Hepk,
            signing: P256SigningKey,
        }

        impl HybridSecretDecapsulator for FixtureYubi {
            fn entity_id(&self) -> &EntityId {
                &self.id
            }

            fn hepk(&self) -> &Hepk {
                &self.hepk
            }

            fn derive_dh_shared(&self, _: &DhPublicKey) -> Result<Zeroizing<[u8; 32]>> {
                Err(Error::YubiSigning)
            }

            fn decapsulate_mlkem768(&self, _: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
                Err(Error::YubiSigning)
            }
        }

        impl YubiDevice for FixtureYubi {
            fn pq_key_id(&self) -> [u8; 32] {
                [0; 32]
            }

            fn sign_sha512_256(&self, digest: &[u8; 32]) -> Result<Vec<u8>> {
                let signature: P256Signature = self
                    .signing
                    .sign_prehash(digest)
                    .map_err(|_| Error::YubiSigning)?;
                Ok(signature.to_der().as_bytes().to_vec())
            }
        }

        let signing = P256SigningKey::from_bytes((&[7_u8; 32]).into()).unwrap();
        let mut id = vec![foks_proto::ENTITY_YUBI];
        id.extend_from_slice(signing.verifying_key().to_encoded_point(true).as_bytes());
        let software_seed = SecretSeed::new(user_fixture("device-seed.bin").try_into().unwrap());
        let yubi = FixtureYubi {
            id: EntityId::from_bytes(id).unwrap(),
            hepk: derive_device_public(&software_seed).unwrap().hepk,
            signing,
        };
        let expected = UserLink::decode(&mutation_fixture("adhoc-team-link.snowp")).unwrap();
        let change = expected.decode_team_group_change().unwrap();
        let owner = &change.changes[0];
        let owner_seed = SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap());
        let ptk_seeds = [
            "adhoc-ptk-member-min-seed.bin",
            "adhoc-ptk-member-seed.bin",
            "adhoc-ptk-admin-seed.bin",
            "adhoc-ptk-owner-seed.bin",
        ]
        .map(|name| SecretSeed::new(mutation_fixture(name).try_into().unwrap()));
        let material = make_single_owner_adhoc_team_yubi(
            &AdHocTeamInput {
                user: &owner.party,
                host: &change.host,
                root: &change.root,
                time: change.time,
                owner_puk_generation: owner.keys.as_ref().unwrap().generation,
                membership_sequence: 1,
                membership_previous: None,
                next_tree_location: mutation_fixture("adhoc-next-tree-location.bin")
                    .try_into()
                    .unwrap(),
                subchain_tree_location: mutation_fixture("adhoc-subchain-tree-location.bin")
                    .try_into()
                    .unwrap(),
                membership_next_tree_location: mutation_fixture(
                    "adhoc-membership-next-tree-location.bin",
                )
                .try_into()
                .unwrap(),
            },
            &yubi,
            &owner_seed,
            [&ptk_seeds[0], &ptk_seeds[1], &ptk_seeds[2], &ptk_seeds[3]],
        )
        .unwrap();
        assert_eq!(
            material.link.encoded().unwrap(),
            expected.encoded().unwrap()
        );
        let [Signature::Ecdsa(_)] = material.membership_link.signatures() else {
            panic!("Yubi membership link must carry exactly one ECDSA signature");
        };
        verify_typed(
            &yubi.id,
            &material.membership_link.signatures()[0],
            LINK_OUTER_V1_TYPE_ID,
            &material.membership_link.signing_bytes(0).unwrap(),
        )
        .unwrap();

        let wrong_signer = FixtureYubi {
            id: yubi.id.clone(),
            hepk: yubi.hepk.clone(),
            signing: P256SigningKey::from_bytes((&[8_u8; 32]).into()).unwrap(),
        };
        assert!(matches!(
            make_single_owner_adhoc_team_yubi(
                &AdHocTeamInput {
                    user: &owner.party,
                    host: &change.host,
                    root: &change.root,
                    time: change.time,
                    owner_puk_generation: owner.keys.as_ref().unwrap().generation,
                    membership_sequence: 1,
                    membership_previous: None,
                    next_tree_location: [1; 32],
                    subchain_tree_location: [2; 32],
                    membership_next_tree_location: [3; 32],
                },
                &wrong_signer,
                &owner_seed,
                [&ptk_seeds[0], &ptk_seeds[1], &ptk_seeds[2], &ptk_seeds[3]],
            ),
            Err(Error::Verification)
        ));
    }

    #[test]
    fn software_mutation_boxes_round_trip_and_preserve_history() {
        let eldest = UserLink::decode(&user_fixture("user-eldest-link.snowp"))
            .unwrap()
            .decode_eldest()
            .unwrap();
        let sender_seed = SecretSeed::new(user_fixture("device-seed.bin").try_into().unwrap());
        let receiver_seed =
            SecretSeed::new(user_fixture("second-device-seed.bin").try_into().unwrap());
        let receiver = derive_device_public(&receiver_seed).unwrap();
        let current_seed = SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap());
        let boxes = seal_software_puk_boxes(
            &eldest.host,
            &sender_seed,
            [21; 16],
            &[SoftwarePukBoxInput {
                seed: &current_seed,
                generation: 2,
                role: Role::OWNER,
                receiver: &receiver,
            }],
            &[PukBoxRandomness {
                kem_message: [22; 32],
                nonce: [23; 16],
            }],
        )
        .unwrap();
        assert_eq!(SharedKeyBoxSet::decode(&boxes.encoded()).unwrap(), boxes);
        let shared = boxes.boxes[0].clone();
        let sender = derive_device_public(&sender_seed).unwrap();
        let current_public =
            derive_shared_public(&current_seed, foks_proto::ENTITY_PUK_VERIFY).unwrap();
        let previous_seed =
            SecretSeed::new(user_fixture("initial-puk-seed.bin").try_into().unwrap());
        let historical = seal_puk_seed_chain_box(
            &current_seed,
            &previous_seed,
            &eldest.uid,
            &eldest.host,
            1,
            Role::OWNER,
            [24; 16],
        )
        .unwrap();
        let parcel = PukParcel {
            generation: shared.generation,
            role: shared.role,
            hybrid: shared.hybrid,
            target: shared.target.entity,
            target_host: shared.target.host,
            target_role: shared.target.role,
            target_generation: shared.target.generation,
            sender: sender.id,
            box_id: boxes.box_id,
            temp_dh_key: boxes.temp_dh_key,
            seed_chain: vec![historical],
        };
        let current = open_puk_parcel_for_role(
            &parcel,
            &receiver_seed,
            &sender.hepk,
            &current_public.verify_key,
            &current_public.hepk,
            parcel.generation,
            &eldest.host,
            Role::OWNER,
        )
        .unwrap();
        let history = open_puk_seed_chain(current, &parcel, &eldest.uid, &eldest.host).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].seed, previous_seed);
        assert_eq!(history[1].seed, current_seed);
    }

    #[test]
    fn official_initial_signup_puk_box_opens() {
        let expected = UserLink::decode(&signup_fixture("eldest-link.snowp")).unwrap();
        let opened = expected.decode_eldest().unwrap();
        let device_seed = SecretSeed::new(signup_fixture("device-seed.bin").try_into().unwrap());
        let puk_bytes: [u8; 32] = signup_fixture("puk-seed.bin").try_into().unwrap();
        let puk_seed = SecretSeed::new(puk_bytes);
        let device = derive_device_public(&device_seed).unwrap();
        let puk = derive_shared_public(&puk_seed, foks_proto::ENTITY_PUK_VERIFY).unwrap();
        let boxed = SharedKeyBoxSet::decode(&signup_fixture("puk-box-set.snowp")).unwrap();
        let box_id = boxed.box_id;
        let temporary = boxed.temp_dh_key;
        let shared = boxed.boxes.into_iter().next().unwrap();
        let parcel = PukParcel {
            generation: shared.generation,
            role: shared.role,
            hybrid: shared.hybrid,
            target: shared.target.entity,
            target_host: shared.target.host,
            target_role: shared.target.role,
            target_generation: shared.target.generation,
            sender: device.id,
            box_id,
            temp_dh_key: temporary,
            seed_chain: Vec::new(),
        };
        let clear = open_puk_parcel_for_role(
            &parcel,
            &device_seed,
            &device.hepk,
            &puk.verify_key,
            &puk.hepk,
            parcel.generation,
            &opened.host,
            Role::OWNER,
        )
        .unwrap();
        assert_eq!(clear.seed.as_bytes(), &puk_bytes);
    }

    #[test]
    fn sha512_256_type_prefix_is_big_endian() {
        let object = [0x91, 0xc0];
        let mut direct = Sha512_256::new();
        direct.update(0x0102_0304_0506_0708_u64.to_be_bytes());
        direct.update(object);
        assert_eq!(
            prefixed_hash(0x0102_0304_0506_0708, &object),
            <[u8; 32]>::from(direct.finalize())
        );
    }

    #[test]
    fn initial_software_puk_box_round_trips_and_is_bound() {
        let chain = UserChain::decode(&user_fixture("user-chain.snowp")).unwrap();
        let host = chain.links[0].decode_eldest().unwrap().host;
        let device_seed = SecretSeed::new(user_fixture("device-seed.bin").try_into().unwrap());
        let puk_bytes: [u8; 32] = user_fixture("initial-puk-seed.bin").try_into().unwrap();
        let puk_seed = SecretSeed::new(puk_bytes);
        let boxed = seal_initial_puk_box(
            &host,
            &device_seed,
            &puk_seed,
            InitialPukBoxRandomness {
                box_id: [7; 16],
                kem_message: [8; 32],
                nonce: [9; 16],
            },
        )
        .unwrap();
        assert_eq!(SharedKeyBoxSet::decode(&boxed.encoded()).unwrap(), boxed);
        let device = derive_device_public(&device_seed).unwrap();
        let puk = derive_shared_public(&puk_seed, foks_proto::ENTITY_PUK_VERIFY).unwrap();
        let shared = boxed.boxes.first().unwrap();
        let parcel = PukParcel {
            generation: shared.generation,
            role: shared.role,
            hybrid: shared.hybrid.clone(),
            target: shared.target.entity.clone(),
            target_host: shared.target.host.clone(),
            target_role: shared.target.role,
            target_generation: shared.target.generation,
            sender: device.id.clone(),
            box_id: boxed.box_id,
            temp_dh_key: boxed.temp_dh_key.clone(),
            seed_chain: Vec::new(),
        };
        let opened = open_puk_parcel_for_role(
            &parcel,
            &device_seed,
            &device.hepk,
            &puk.verify_key,
            &puk.hepk,
            parcel.generation,
            &host,
            Role::OWNER,
        )
        .unwrap();
        assert_eq!(opened.seed.as_bytes(), &puk_bytes);

        let wrong_puk =
            derive_shared_public(&SecretSeed::new([0x55; 32]), ENTITY_PUK_VERIFY).unwrap();
        assert!(matches!(
            open_puk_parcel_for_role(
                &parcel,
                &device_seed,
                &device.hepk,
                &puk.verify_key,
                &wrong_puk.hepk,
                parcel.generation,
                &host,
                Role::OWNER,
            ),
            Err(Error::PukBinding)
        ));
        assert!(open_puk_parcel_for_role(
            &parcel,
            &device_seed,
            &device.hepk,
            &puk.verify_key,
            &puk.hepk,
            parcel.generation + 1,
            &host,
            Role::OWNER,
        )
        .is_err());

        let mut tampered = parcel;
        tampered.hybrid.ciphertext[0] ^= 1;
        assert!(open_puk_parcel_for_role(
            &tampered,
            &device_seed,
            &device.hepk,
            &puk.verify_key,
            &puk.hepk,
            tampered.generation,
            &host,
            Role::OWNER,
        )
        .is_err());
    }

    #[test]
    fn official_public_zone_signature_verifies() {
        let probe = ProbeResponse::decode(PROBE).unwrap();
        let change = probe.hostchain[0].decode_change().unwrap();
        let signer = change
            .changes
            .iter()
            .find_map(|change| match change {
                foks_proto::HostchainChangeItem::Key(id)
                    if id.entity_type() == foks_proto::ENTITY_HOST_METADATA_SIGNER =>
                {
                    Some(id)
                }
                _ => None,
            })
            .unwrap();
        verify_blob(
            signer,
            &probe.public_zone.signature,
            PUBLIC_ZONE_BLOB_TYPE_ID,
            &probe.public_zone.inner,
        )
        .unwrap();
    }

    #[test]
    fn signed_blob_tampering_is_rejected() {
        let probe = ProbeResponse::decode(PROBE).unwrap();
        let change = probe.hostchain[0].decode_change().unwrap();
        let signer = change
            .changes
            .iter()
            .find_map(|change| match change {
                foks_proto::HostchainChangeItem::Key(id)
                    if id.entity_type() == foks_proto::ENTITY_HOST_METADATA_SIGNER =>
                {
                    Some(id)
                }
                _ => None,
            })
            .unwrap();
        let mut inner = probe.public_zone.inner.clone();
        inner[5] ^= 1;
        assert!(verify_blob(
            signer,
            &probe.public_zone.signature,
            PUBLIC_ZONE_BLOB_TYPE_ID,
            &inner,
        )
        .is_err());
    }

    #[test]
    fn official_device_derivation_and_puk_unboxing_match() {
        let seed = SecretSeed::new(user_fixture("device-seed.bin").try_into().unwrap());
        let derived = derive_device_public(&seed).unwrap();
        let expected_id = match foks_snowpack::decode(&user_fixture("device-id.snowp")).unwrap() {
            Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
            _ => panic!("device fixture is not binary"),
        };
        assert_eq!(derived.id, expected_id);

        let chain = UserChain::decode(&user_fixture("user-chain.snowp")).unwrap();
        assert!(chain.hepks.iter().any(|hepk| hepk == &derived.hepk));
        let eldest = chain.links[0].decode_eldest().unwrap();
        let rotated = chain.links[2].decode_group_change().unwrap().shared_keys[0].clone();
        let rotated_public = derive_shared_public(
            &SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap()),
            ENTITY_PUK_VERIFY,
        )
        .unwrap();
        assert_eq!(rotated_public.verify_key, rotated.verify_key);
        let mut parcel = PukParcel::decode(&user_fixture("puk-parcel.snowp")).unwrap();
        let (hybrid_key, payload) =
            derive_hybrid_key(&parcel, &seed, derived.hepk.classical(), &derived.hepk).unwrap();
        assert_eq!(payload.as_slice(), user_fixture("hybrid-payload.snowp"));
        assert_eq!(
            hybrid_key.as_slice(),
            user_fixture("hybrid-secretbox-key.bin")
        );
        let clear = open_puk_parcel(
            &parcel,
            &seed,
            &derived.hepk,
            &rotated.verify_key,
            &rotated_public.hepk,
            rotated.generation,
            &eldest.host,
        )
        .unwrap();
        assert_eq!(clear.seed.as_slice(), user_fixture("puk-seed.bin"));
        let puks = open_puk_seed_chain(clear, &parcel, &eldest.uid, &eldest.host).unwrap();
        assert_eq!(puks.len(), 2);
        assert_eq!(puks[0].generation, 1);
        assert_eq!(
            puks[0].seed.as_slice(),
            user_fixture("initial-puk-seed.bin")
        );
        assert_eq!(puks[1].generation, 2);
        assert_eq!(puks[1].seed.as_slice(), user_fixture("puk-seed.bin"));
        let mut wrong_generation = parcel.clone();
        wrong_generation.seed_chain[0].generation = 2;
        let clear = open_puk_parcel(
            &wrong_generation,
            &seed,
            &derived.hepk,
            &rotated.verify_key,
            &rotated_public.hepk,
            rotated.generation,
            &eldest.host,
        )
        .unwrap();
        assert!(open_puk_seed_chain(clear, &wrong_generation, &eldest.uid, &eldest.host).is_err());
        parcel.seed_chain[0].secret_box.ciphertext[0] ^= 1;
        let clear = open_puk_parcel(
            &parcel,
            &seed,
            &derived.hepk,
            &rotated.verify_key,
            &rotated_public.hepk,
            rotated.generation,
            &eldest.host,
        )
        .unwrap();
        assert!(open_puk_seed_chain(clear, &parcel, &eldest.uid, &eldest.host).is_err());
    }

    #[test]
    fn official_team_ptk_parcels_unbox_for_every_role() {
        let puk_seed = SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap());
        let uid = match foks_snowpack::decode(&user_fixture("uid.snowp")).unwrap() {
            Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
            _ => panic!("UID fixture is not binary"),
        };
        let receiver = SharedKeyDecapsulator::new(&puk_seed, uid).unwrap();
        let chain = TeamChain::decode(&user_fixture("team-chain.snowp")).unwrap();
        let change = chain.links[0].decode_team_group_change().unwrap();
        let member = &change.changes[0];
        let member_keys = member.keys.as_ref().unwrap();
        let expected = [
            (Role::member(-0x4000), "team-ptk-member-min-seed.bin"),
            (Role::member(0), "team-ptk-member-seed.bin"),
            (Role::ADMIN, "team-ptk-admin-seed.bin"),
            (Role::OWNER, "team-ptk-owner-seed.bin"),
        ];
        assert_eq!(chain.boxes.len(), expected.len());
        for (role, seed_file) in expected {
            let key = change
                .shared_keys
                .iter()
                .find(|key| key.role == role)
                .expect("fixture has a PTK for every eldest role");
            let parcel = chain
                .boxes
                .iter()
                .find(|parcel| parcel.role == role)
                .expect("fixture has a parcel for every eldest role");
            assert_eq!(parcel.target_role, member.source_role);
            assert_eq!(parcel.target_generation, member_keys.generation);
            assert!(parcel.target_host.is_none());
            let expected_public = derive_shared_public(
                &SecretSeed::new(user_fixture(seed_file).try_into().unwrap()),
                ENTITY_PTK_VERIFY,
            )
            .unwrap();
            assert_eq!(expected_public.verify_key, key.verify_key);
            let clear = open_shared_key_parcel_with(
                parcel,
                &receiver,
                receiver.hepk(),
                &key.verify_key,
                &expected_public.hepk,
                key.generation,
                &change.host,
                member.source_role,
                member_keys.generation,
                role,
                ENTITY_PTK_VERIFY,
            )
            .unwrap();
            assert_eq!(clear.seed.as_slice(), user_fixture(seed_file));
        }
    }

    #[test]
    fn official_team_kv_tree_verifies_and_decrypts_end_to_end() {
        let seed = SecretSeed::new(
            user_fixture("team-ptk-member-min-seed.bin")
                .try_into()
                .unwrap(),
        );
        let team = match foks_snowpack::decode(&user_fixture("team-id.snowp")).unwrap() {
            Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
            _ => panic!("team ID fixture is not binary"),
        };
        let chain = TeamChain::decode(&user_fixture("team-chain.snowp")).unwrap();
        let host = chain.links[0].decode_team_group_change().unwrap().host;
        let party = KvParty { party: team, host };
        let root = KvRoot::decode(&user_fixture("kv-root.snowp")).unwrap();
        let keys = derive_kv_keys(&seed).unwrap();
        keys.verify_root(&root, &party).unwrap();

        let directory =
            foks_proto::KvDirectoryPair::decode(&user_fixture("kv-root-dir.snowp")).unwrap();
        assert_eq!(directory.active.id, root.root);
        let directory_seed = keys.open_directory_seed(&directory.active).unwrap();
        assert_eq!(
            directory_seed.as_slice(),
            user_fixture("kv-root-dir-seed.bin")
        );

        let listing = foks_proto::KvListResponse::decode(&user_fixture("kv-list.snowp")).unwrap();
        assert!(listing.final_page);
        assert_eq!(listing.entries.len(), 3);
        let names = listing
            .entries
            .iter()
            .map(|entry| open_kv_dirent_name(&directory_seed, entry).unwrap().name)
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                b"small.txt".to_vec(),
                b"latest".to_vec(),
                b"large.bin".to_vec()
            ]
        );

        let small = match foks_proto::KvNode::decode(&user_fixture("kv-small-node.snowp")).unwrap()
        {
            foks_proto::KvNode::SmallFile(boxed) => boxed,
            _ => panic!("small-file fixture has wrong node type"),
        };
        let small_id = listing.entries[0].value;
        assert_eq!(
            keys.open_small_file(small_id, &small).unwrap(),
            KvSmallFilePlaintext::File(user_fixture("kv-small-plaintext.bin"))
        );

        let symlink =
            match foks_proto::KvNode::decode(&user_fixture("kv-symlink-node.snowp")).unwrap() {
                foks_proto::KvNode::Symlink(boxed) => boxed,
                _ => panic!("symlink fixture has wrong node type"),
            };
        assert_eq!(
            keys.open_small_file(listing.entries[1].value, &symlink)
                .unwrap(),
            KvSmallFilePlaintext::Symlink(user_fixture("kv-symlink-plaintext.bin"))
        );

        let metadata =
            match foks_proto::KvNode::decode(&user_fixture("kv-large-node.snowp")).unwrap() {
                foks_proto::KvNode::File(metadata) => metadata,
                _ => panic!("large-file fixture has wrong node type"),
            };
        let large_id = listing.entries[2].value;
        let file_seed = keys.open_file_seed(large_id, &metadata).unwrap();
        assert_eq!(file_seed.as_slice(), user_fixture("kv-file-seed.bin"));
        let chunk = KvEncryptedChunk::decode(&user_fixture("kv-large-chunk.snowp")).unwrap();
        assert_eq!(
            open_kv_chunk(&file_seed, large_id, 0, &chunk).unwrap(),
            user_fixture("kv-large-plaintext.bin")
        );
    }

    #[test]
    fn kv_write_sealing_matches_official_v019_objects() {
        let shared = SecretSeed::new(
            user_fixture("team-ptk-member-min-seed.bin")
                .try_into()
                .unwrap(),
        );
        let keys = derive_kv_keys(&shared).unwrap();
        let key = RoleAndGeneration {
            role: Role::member(-16_384),
            generation: 1,
        };
        let listing = foks_proto::KvListResponse::decode(&user_fixture("kv-list.snowp")).unwrap();
        let small_id = listing.entries[0].value;
        let small = keys
            .seal_small_file(
                small_id,
                key,
                KvSmallFilePlaintext::File(user_fixture("kv-small-plaintext.bin")),
            )
            .unwrap();
        assert_eq!(small.encode().unwrap(), user_fixture("kv-small-box.snowp"));

        let file_seed = SecretSeed::new(user_fixture("kv-file-seed.bin").try_into().unwrap());
        let large_id = listing.entries[2].value;
        let expected_metadata =
            KvLargeFileMetadata::decode(&user_fixture("kv-write-large-metadata.snowp")).unwrap();
        let metadata = keys
            .seal_file_seed(
                large_id,
                key,
                1,
                &file_seed,
                expected_metadata.key_seed.nonce,
            )
            .unwrap();
        assert_eq!(metadata, expected_metadata);

        let clear = user_fixture("kv-large-plaintext.bin");
        let chunk = seal_kv_chunk(&file_seed, large_id, 0, true, &clear, 0).unwrap();
        assert_eq!(
            chunk.encode().unwrap(),
            user_fixture("kv-upload-chunk.snowp")
        );

        let directory_seed =
            SecretSeed::new(user_fixture("kv-root-dir-seed.bin").try_into().unwrap());
        let expected = KvDirent::decode(&user_fixture("kv-write-dirent.snowp")).unwrap();
        let (name_mac, name_box) = seal_kv_dirent_name(
            &directory_seed,
            expected.parent,
            expected.directory_version,
            b"write.txt".to_vec(),
            expected.name_box.nonce,
        )
        .unwrap();
        assert_eq!(name_mac, expected.name_mac);
        assert_eq!(name_box, expected.name_box);
        assert_eq!(
            bind_kv_dirent(&directory_seed, &expected).unwrap(),
            expected.binding_mac
        );
    }

    #[test]
    fn kv_bindings_and_ciphertexts_fail_closed() {
        let seed = SecretSeed::new(
            user_fixture("team-ptk-member-min-seed.bin")
                .try_into()
                .unwrap(),
        );
        let keys = derive_kv_keys(&seed).unwrap();
        let mut root = KvRoot::decode(&user_fixture("kv-root.snowp")).unwrap();
        let team = match foks_snowpack::decode(&user_fixture("team-id.snowp")).unwrap() {
            Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
            _ => unreachable!(),
        };
        let chain = TeamChain::decode(&user_fixture("team-chain.snowp")).unwrap();
        let party = KvParty {
            party: team,
            host: chain.links[0].decode_team_group_change().unwrap().host,
        };
        root.binding_mac[0] ^= 1;
        assert!(matches!(
            keys.verify_root(&root, &party),
            Err(Error::KvBinding)
        ));

        let mut small = KvSmallFileBox::decode(&user_fixture("kv-small-box.snowp")).unwrap();
        small.ciphertext[0] ^= 1;
        let listing = foks_proto::KvListResponse::decode(&user_fixture("kv-list.snowp")).unwrap();
        assert!(matches!(
            keys.open_small_file(listing.entries[0].value, &small),
            Err(Error::Decryption)
        ));
    }

    #[test]
    fn large_chunk_requires_the_exact_requested_offset() {
        let file_seed = SecretSeed::new([0x55; 32]);
        let mut id = [0x77; 17];
        id[0] = 2;
        let id = KvNodeId(id);
        let chunk_offset = 0u64;
        let clear = b"abcdef".to_vec();
        let chunk = seal_kv_chunk(&file_seed, id, chunk_offset, true, &clear, 0).unwrap();
        let chunk = KvEncryptedChunk {
            ciphertext: chunk.ciphertext,
            offset: chunk.offset,
            final_chunk: chunk.final_upload.is_some(),
        };
        assert_eq!(
            open_kv_chunk(&file_seed, id, chunk_offset, &chunk).unwrap(),
            b"abcdef"
        );
        let requested = 2u64;
        assert!(matches!(
            open_kv_chunk(&file_seed, id, requested, &chunk),
            Err(Error::KvBinding)
        ));
        let ahead = KvEncryptedChunk {
            offset: requested + 1,
            ..chunk.clone()
        };
        assert!(matches!(
            open_kv_chunk(&file_seed, id, requested, &ahead),
            Err(Error::KvBinding)
        ));
        let lying = KvEncryptedChunk {
            offset: requested,
            ..chunk
        };
        assert!(matches!(
            open_kv_chunk(&file_seed, id, requested, &lying),
            Err(Error::Decryption)
        ));
    }

    #[test]
    fn hybrid_puk_tampering_is_rejected() {
        let seed = SecretSeed::new(user_fixture("device-seed.bin").try_into().unwrap());
        let derived = derive_device_public(&seed).unwrap();
        let chain = UserChain::decode(&user_fixture("user-chain.snowp")).unwrap();
        let eldest = chain.links[0].decode_eldest().unwrap();
        let rotated = chain.links[2].decode_group_change().unwrap().shared_keys[0].clone();
        let rotated_public = derive_shared_public(
            &SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap()),
            ENTITY_PUK_VERIFY,
        )
        .unwrap();
        let mut parcel = PukParcel::decode(&user_fixture("puk-parcel.snowp")).unwrap();
        parcel.hybrid.ciphertext[0] ^= 1;
        assert!(matches!(
            open_puk_parcel(
                &parcel,
                &seed,
                &derived.hepk,
                &rotated.verify_key,
                &rotated_public.hepk,
                rotated.generation,
                &eldest.host,
            ),
            Err(Error::Decryption)
        ));
    }

    #[test]
    fn official_mock_yubi_eldest_signature_stack_verifies() {
        let link = UserLink::decode(&user_fixture("yubi/yubi-eldest-link.snowp")).unwrap();
        let eldest = link.decode_eldest().unwrap();
        let subkey = eldest.member_subkey.as_ref().unwrap();
        assert_eq!(link.signatures().len(), 3);
        verify_typed(
            &eldest.puk_verify_key,
            &link.signatures()[0],
            foks_proto::LINK_OUTER_V1_TYPE_ID,
            &link.signing_bytes(0).unwrap(),
        )
        .unwrap();
        verify_typed(
            subkey,
            &link.signatures()[1],
            foks_proto::LINK_OUTER_V1_TYPE_ID,
            &link.signing_bytes(1).unwrap(),
        )
        .unwrap();
        verify_typed(
            &eldest.member,
            &link.signatures()[2],
            foks_proto::LINK_OUTER_V1_TYPE_ID,
            &link.signing_bytes(2).unwrap(),
        )
        .unwrap();

        let mut tampered = link.signatures()[2].clone();
        let Signature::Ecdsa(bytes) = &mut tampered else {
            panic!("mock Yubi fixture did not use ECDSA");
        };
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        assert!(verify_typed(
            &eldest.member,
            &tampered,
            foks_proto::LINK_OUTER_V1_TYPE_ID,
            &link.signing_bytes(2).unwrap(),
        )
        .is_err());
    }

    #[test]
    fn official_yubi_to_software_cross_curve_parcel_unboxes() {
        let seed = SecretSeed::new(
            user_fixture("yubi/software-device-seed.bin")
                .try_into()
                .unwrap(),
        );
        let sender_hepk = Hepk::decode(&user_fixture("yubi/yubi-hepk.snowp")).unwrap();
        let sender = match foks_snowpack::decode(&user_fixture("yubi/yubi-id.snowp")).unwrap() {
            Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
            _ => panic!("Yubi fixture is not an EntityID"),
        };
        let parcel =
            PukParcel::decode(&user_fixture("yubi/yubi-to-software-puk-parcel.snowp")).unwrap();
        assert_eq!(parcel.sender, sender);
        assert!(parcel.temp_dh_key.is_some());
        let link = UserLink::decode(&user_fixture("yubi/yubi-eldest-link.snowp")).unwrap();
        let eldest = link.decode_eldest().unwrap();
        let expected_puk = derive_shared_public(
            &SecretSeed::new(user_fixture("yubi/puk-seed.bin").try_into().unwrap()),
            ENTITY_PUK_VERIFY,
        )
        .unwrap();
        let clear = open_puk_parcel(
            &parcel,
            &seed,
            &sender_hepk,
            &eldest.puk_verify_key,
            &expected_puk.hepk,
            parcel.generation,
            &eldest.host,
        )
        .unwrap();
        assert_eq!(clear.seed.as_slice(), user_fixture("yubi/puk-seed.bin"));

        let mut tampered = parcel;
        let Signature::Ecdsa(signature) = &mut tampered
            .temp_dh_key
            .as_mut()
            .expect("cross-curve parcel has a temporary key")
            .signature
        else {
            panic!("Yubi fixture has an unexpected signature type");
        };
        signature[0] ^= 1;
        assert!(open_puk_parcel(
            &tampered,
            &seed,
            &sender_hepk,
            &eldest.puk_verify_key,
            &expected_puk.hepk,
            tampered.generation,
            &eldest.host,
        )
        .is_err());
    }

    #[test]
    fn official_go_mixed_curve_box_sets_open_with_the_matching_sender_path() {
        let software_seed = SecretSeed::new(
            user_fixture("yubi/software-device-seed.bin")
                .try_into()
                .unwrap(),
        );
        let software = derive_device_public(&software_seed).unwrap();
        let link = UserLink::decode(&user_fixture("yubi/yubi-eldest-link.snowp")).unwrap();
        let eldest = link.decode_eldest().unwrap();
        let puk_seed = SecretSeed::new(user_fixture("yubi/puk-seed.bin").try_into().unwrap());
        let puk = derive_shared_public(&puk_seed, ENTITY_PUK_VERIFY).unwrap();

        let software_mixed =
            SharedKeyBoxSet::decode(&user_fixture("yubi/software-mixed-puk-box-set.snowp"))
                .unwrap();
        assert_eq!(software_mixed.boxes.len(), 2);
        assert!(software_mixed.temp_dh_key.is_some());
        let mut same_curve =
            PukParcel::from_box_set(&software_mixed, 0, software.id.clone(), Vec::new()).unwrap();
        // Go's OpenBoxInSet deliberately ignores the set-level temporary key
        // when sender and receiver use the same classical curve.
        let Signature::Ed25519(signature) = &mut same_curve
            .temp_dh_key
            .as_mut()
            .expect("mixed set has a temporary key")
            .signature
        else {
            panic!("software sender used the wrong temporary-key signature")
        };
        signature[0] ^= 1;
        assert_eq!(
            open_puk_parcel(
                &same_curve,
                &software_seed,
                &software.hepk,
                &puk.verify_key,
                &puk.hepk,
                1,
                &eldest.host,
            )
            .unwrap()
            .seed,
            puk_seed
        );

        let yubi_mixed =
            SharedKeyBoxSet::decode(&user_fixture("yubi/yubi-mixed-puk-box-set.snowp")).unwrap();
        assert_eq!(yubi_mixed.boxes.len(), 2);
        let yubi_sender_hepk = Hepk::decode(&user_fixture("yubi/yubi-hepk.snowp")).unwrap();
        let yubi_sender = match foks_snowpack::decode(&user_fixture("yubi/yubi-id.snowp")).unwrap()
        {
            Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
            _ => panic!("Yubi fixture is not an EntityID"),
        };
        let cross_curve = PukParcel::from_box_set(&yubi_mixed, 1, yubi_sender, Vec::new()).unwrap();
        assert_eq!(
            open_puk_parcel(
                &cross_curve,
                &software_seed,
                &yubi_sender_hepk,
                &puk.verify_key,
                &puk.hepk,
                1,
                &eldest.host,
            )
            .unwrap()
            .seed,
            puk_seed
        );
    }

    #[test]
    fn hardware_boundary_reproduces_the_official_hybrid_secrets() {
        struct FixtureHardware {
            public: DevicePublicMaterial,
        }

        impl HybridSecretDecapsulator for FixtureHardware {
            fn entity_id(&self) -> &EntityId {
                &self.public.id
            }
            fn hepk(&self) -> &Hepk {
                &self.public.hepk
            }
            fn derive_dh_shared(&self, _: &DhPublicKey) -> Result<Zeroizing<[u8; 32]>> {
                Ok(Zeroizing::new(
                    user_fixture("hybrid-dh-shared.bin").try_into().unwrap(),
                ))
            }
            fn decapsulate_mlkem768(&self, _: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
                Ok(Zeroizing::new(
                    user_fixture("hybrid-kem-shared.bin").try_into().unwrap(),
                ))
            }
        }

        let seed = SecretSeed::new(user_fixture("device-seed.bin").try_into().unwrap());
        let hardware = FixtureHardware {
            public: derive_device_public(&seed).unwrap(),
        };
        let chain = UserChain::decode(&user_fixture("user-chain.snowp")).unwrap();
        let eldest = chain.links[0].decode_eldest().unwrap();
        let rotated = chain.links[2].decode_group_change().unwrap().shared_keys[0].clone();
        let rotated_public = derive_shared_public(
            &SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap()),
            ENTITY_PUK_VERIFY,
        )
        .unwrap();
        let parcel = PukParcel::decode(&user_fixture("puk-parcel.snowp")).unwrap();
        let clear = open_puk_parcel_with(
            &parcel,
            &hardware,
            &hardware.public.hepk,
            &rotated.verify_key,
            &rotated_public.hepk,
            rotated.generation,
            &eldest.host,
        )
        .unwrap();
        assert_eq!(clear.seed.as_slice(), user_fixture("puk-seed.bin"));
    }

    #[test]
    fn backup_enrollment_and_recovery_links_match_go_v019() {
        let backup =
            BackupKey::from_seed(mutation_fixture("backup-seed.bin").try_into().unwrap()).unwrap();
        let existing_seed = SecretSeed::new(user_fixture("device-seed.bin").try_into().unwrap());

        let expected_enroll =
            UserLink::decode(&mutation_fixture("backup-enroll-link.snowp")).unwrap();
        let enroll = expected_enroll.decode_group_change().unwrap();
        let name = backup.device_name();
        let enroll_material = make_backup_provision_link(
            &SoftwareProvisionInput {
                base: UserMutationBase {
                    uid: &enroll.uid,
                    host: &enroll.host,
                    seqno: enroll.seqno,
                    previous: enroll.previous.unwrap(),
                    root: &enroll.root,
                    time: enroll.time,
                    next_tree_location: mutation_fixture("backup-enroll-next-tree-location.bin")
                        .try_into()
                        .unwrap(),
                },
                role: Role::OWNER,
                device_label: &foks_proto::DeviceLabel {
                    device_type: foks_proto::DeviceType::Backup,
                    normalized_name: name.as_bytes().to_vec(),
                    serial: 1,
                },
                device_name_commitment_key: mutation_fixture(
                    "backup-enroll-device-name-commitment-key.bin",
                )
                .try_into()
                .unwrap(),
            },
            &existing_seed,
            &backup,
            None,
        )
        .unwrap();
        assert_eq!(
            enroll_material.link.encoded().unwrap(),
            expected_enroll.encoded().unwrap()
        );

        let expected_recover =
            UserLink::decode(&mutation_fixture("backup-recover-link.snowp")).unwrap();
        let recover = expected_recover.decode_group_change().unwrap();
        let replacement_seed = SecretSeed::new(
            mutation_fixture("backup-recover-device-seed.bin")
                .try_into()
                .unwrap(),
        );
        let recover_material = make_software_provision_link_from_backup(
            &SoftwareProvisionInput {
                base: UserMutationBase {
                    uid: &recover.uid,
                    host: &recover.host,
                    seqno: recover.seqno,
                    previous: recover.previous.unwrap(),
                    root: &recover.root,
                    time: recover.time,
                    next_tree_location: mutation_fixture("backup-recover-next-tree-location.bin")
                        .try_into()
                        .unwrap(),
                },
                role: Role::OWNER,
                device_label: &foks_proto::DeviceLabel {
                    device_type: foks_proto::DeviceType::Computer,
                    normalized_name: b"recovered fixture device".to_vec(),
                    serial: 1,
                },
                device_name_commitment_key: mutation_fixture(
                    "backup-recover-device-name-commitment-key.bin",
                )
                .try_into()
                .unwrap(),
            },
            &backup,
            &replacement_seed,
            None,
        )
        .unwrap();
        assert_eq!(
            recover_material.link.encoded().unwrap(),
            expected_recover.encoded().unwrap()
        );
    }

    #[test]
    fn backup_opens_the_official_enrollment_puk_box() {
        let backup =
            BackupKey::from_seed(mutation_fixture("backup-seed.bin").try_into().unwrap()).unwrap();
        let boxes =
            SharedKeyBoxSet::decode(&mutation_fixture("backup-enroll-boxes.snowp")).unwrap();
        let boxed = boxes.boxes[0].clone();
        let sender_seed = SecretSeed::new(user_fixture("device-seed.bin").try_into().unwrap());
        let sender = derive_device_public(&sender_seed).unwrap();
        let parcel = PukParcel {
            generation: boxed.generation,
            role: boxed.role,
            hybrid: boxed.hybrid,
            target: boxed.target.entity,
            target_host: boxed.target.host,
            target_role: boxed.target.role,
            target_generation: boxed.target.generation,
            sender: sender.id,
            box_id: boxes.box_id,
            temp_dh_key: boxes.temp_dh_key,
            seed_chain: Vec::new(),
        };
        let expected_seed = SecretSeed::new(user_fixture("puk-seed.bin").try_into().unwrap());
        let expected = derive_shared_public(&expected_seed, ENTITY_PUK_VERIFY).unwrap();
        let receiver = backup.key_material().unwrap();
        let opened = open_puk_parcel_with_for_role(
            &parcel,
            &receiver,
            &sender.hepk,
            &expected.verify_key,
            &expected.hepk,
            parcel.generation,
            &expected_enroll_host(),
            Role::OWNER,
        )
        .unwrap();
        assert_eq!(opened.seed, expected_seed);
    }

    #[test]
    fn remote_member_view_token_box_round_trips_and_binds_ciphertext() {
        let seed = SecretSeed::new([0xa1; 32]);
        let party = foks_proto::FqParty::new(
            EntityId::from_bytes([vec![foks_proto::ENTITY_NAMED_TEAM], vec![0xa2; 32]].concat())
                .unwrap(),
            EntityId::from_bytes([vec![foks_proto::ENTITY_HOST], vec![0xa3; 32]].concat()).unwrap(),
        )
        .unwrap();
        let mut token = [0xa4; 17];
        token[0] = 54;
        let payload = foks_proto::TeamRemoteMemberViewTokenBoxPayload {
            token: foks_proto::PermissionToken::new(token),
            party,
            time: 42,
        };
        let mut boxed = seal_team_remote_member_view_token(&seed, &payload, [0xa5; 16]).unwrap();
        assert_eq!(
            open_team_remote_member_view_token(&seed, &boxed).unwrap(),
            payload
        );
        boxed.ciphertext[0] ^= 1;
        assert!(open_team_remote_member_view_token(&seed, &boxed).is_err());
    }

    fn expected_enroll_host() -> EntityId {
        UserLink::decode(&mutation_fixture("backup-enroll-link.snowp"))
            .unwrap()
            .decode_group_change()
            .unwrap()
            .host
    }
}
