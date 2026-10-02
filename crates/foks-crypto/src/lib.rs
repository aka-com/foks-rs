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

use thiserror::Error;

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
