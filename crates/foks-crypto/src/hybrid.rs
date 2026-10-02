//! Device/shared key derivation and authenticated hybrid key distribution.

use crate::backup::{BackupKey, BackupKeyMaterial};
use crate::primitives::{
    open_typed_secretbox, prefixed_hash_signable, require_zero_padding, seal_typed_secretbox,
};
use crate::signatures::{sign_seed_typed, sign_yubi_typed, verify_typed};
use crate::{Error, Result};
use crypto_secretbox::{aead::Aead, KeyInit, XSalsa20Poly1305};
use ed25519_dalek::SigningKey;
use foks_proto::{
    DhPublicKey, EntityId, Hepk, HybridBox, PukParcel, Role, SecretBox, SecretSeed, SharedKeyBox,
    SharedKeyBoxSet, SharedKeyBoxTarget, SharedKeySeed, Signature, SubkeySeed, HEPK_TYPE_ID,
    HYBRID_SECRET_KEY_SHA3_PAYLOAD_TYPE_ID, SHARED_KEY_SEED_TYPE_ID, SUBKEY_SEED_TYPE_ID,
    TEMP_DH_KEY_SIG_TEMPLATE_TYPE_ID,
};
use foks_snowpack::{decode, decode_prefix, encode, encode_ref, Value, ValueRef};
use hmac::{Hmac, Mac};
use ml_kem::{ml_kem_768, Decapsulate as _, KeyExport as _, TryKeyInit as _};
use p256::ecdh::diffie_hellman as p256_diffie_hellman;
use p256::ecdsa::VerifyingKey as P256VerifyingKey;
use p256::elliptic_curve::sec1::ToEncodedPoint as _;
use p256::{PublicKey as P256PublicKey, SecretKey as P256SecretKey};
use salsa20::cipher::consts::U10;
use salsa20::hsalsa;
use sha2::Sha512_256;
use sha3::{Digest as Sha3Digest, Sha3_256};
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};
use zeroize::Zeroizing;

#[cfg(test)]
type HybridDerivation = (Zeroizing<[u8; 32]>, Zeroizing<Vec<u8>>);

/// Public signing and hybrid-encryption material for a PUK/PTK seed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SharedPublicMaterial {
    pub verify_key: EntityId,
    pub hepk: Hepk,
}

pub(super) fn hybrid_key_derivation_payload(
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

pub(super) fn shared_key_seed_plaintext(
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

pub(super) fn seal_backup_puk_boxes_from_seed(
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

pub(super) fn seal_shared_key_boxes_with_sender(
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

pub(super) fn seal_hybrid_payload(
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

/// Derives the exact Ed25519 identity and hybrid encryption public key used by
/// a v0.1.9 device. Secret intermediates are zeroized on drop.
pub fn derive_device_public(seed: &SecretSeed) -> Result<DevicePublicMaterial> {
    derive_public_material(seed, foks_proto::ENTITY_DEVICE)
}

pub(super) fn derive_public_material(
    seed: &SecretSeed,
    entity_type: u8,
) -> Result<DevicePublicMaterial> {
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

pub(super) fn authenticated_sender_dh(
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

pub(super) fn open_hybrid_box(
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
    pub(super) fn new(seed: &'a SecretSeed) -> Result<Self> {
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

pub(super) fn software_dh_shared(
    seed: &SecretSeed,
    peer: &DhPublicKey,
) -> Result<Zeroizing<[u8; 32]>> {
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

pub(super) fn software_mlkem_decapsulate(
    seed: &SecretSeed,
    ciphertext: &[u8],
) -> Result<Zeroizing<[u8; 32]>> {
    let mlkem_seed = Zeroizing::new(derive_mlkem_seed(seed)?);
    let kem_seed = ml_kem::Seed::try_from(mlkem_seed.as_slice()).map_err(|_| Error::DeviceKey)?;
    let decapsulation = ml_kem_768::DecapsulationKey::from_seed(kem_seed);
    let ciphertext = ml_kem_768::Ciphertext::try_from(ciphertext).map_err(|_| Error::MlKem)?;
    Ok(Zeroizing::new(
        decapsulation.decapsulate(&ciphertext).into(),
    ))
}

pub(super) fn dh_public_value(key: &DhPublicKey) -> Value {
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
pub(super) fn derive_hybrid_key(
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

pub(super) fn derive_mlkem_seed(seed: &SecretSeed) -> Result<Zeroizing<[u8; 64]>> {
    let first = derive_key(seed, 4, Some(0))?;
    let second = derive_key(seed, 4, Some(1))?;
    let mut output = Zeroizing::new([0u8; 64]);
    output[..32].copy_from_slice(first.as_slice());
    output[32..].copy_from_slice(second.as_slice());
    Ok(output)
}

pub(super) fn derive_key(
    seed: &SecretSeed,
    derivation_type: u64,
    index: Option<u64>,
) -> Result<SecretSeed> {
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

pub(super) fn encode_hepk(classical: &[u8; 32], mlkem768: &[u8]) -> Result<Vec<u8>> {
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
