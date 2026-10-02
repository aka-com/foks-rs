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

// Keep the public API at the crate root; implementation domains stay private.
mod kv;
pub use kv::*;
mod primitives;
pub use primitives::*;
mod signatures;
pub use signatures::*;
mod team;
pub use team::*;
mod user;
pub use user::*;
mod hybrid;
pub use hybrid::*;

#[cfg(test)]
mod tests;
