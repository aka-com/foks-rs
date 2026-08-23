//! Cryptographic operations required by the FOKS v0.1.9 public-host path.

#![forbid(unsafe_code)]

use crypto_secretbox::{aead::Aead, KeyInit, XSalsa20Poly1305};
use ed25519_dalek::{Signature as DalekSignature, SigningKey, VerifyingKey};
use foks_proto::{
    DhPublicKey, EntityId, Hepk, HybridBox, PukParcel, Role, SecretSeed, SharedKeySeed, Signature,
    SubkeySeed, HYBRID_SECRET_KEY_SHA3_PAYLOAD_TYPE_ID, SHARED_KEY_SEED_TYPE_ID,
    SUBKEY_SEED_TYPE_ID, TEMP_DH_KEY_SIG_TEMPLATE_TYPE_ID,
};
use foks_snowpack::{encode, Value};
use hmac::{Hmac, Mac};
use ml_kem::{ml_kem_768, Decapsulate as _, KeyExport as _};
use p256::ecdsa::{
    signature::hazmat::PrehashVerifier as _, Signature as P256Signature,
    VerifyingKey as P256VerifyingKey,
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
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
#[cfg(test)]
type HybridDerivation = (Zeroizing<[u8; 32]>, Zeroizing<Vec<u8>>);

/// SHA-512/256 over the 8-byte big-endian type ID and canonical object bytes.
pub fn prefixed_hash(type_id: u64, canonical_object: &[u8]) -> [u8; 32] {
    let mut hash = Sha512_256::new();
    hash.update(type_id.to_be_bytes());
    hash.update(canonical_object);
    hash.finalize().into()
}

/// HMAC-SHA-512/256 commitment used by FOKS for disclosed chain metadata.
pub fn commitment(type_id: u64, canonical_object: &[u8], key: &[u8]) -> [u8; 32] {
    let mut mac =
        <Hmac<Sha512_256> as Mac>::new_from_slice(key).expect("HMAC accepts every key length");
    mac.update(&type_id.to_be_bytes());
    mac.update(canonical_object);
    mac.finalize().into_bytes().into()
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
/// Yubi implementations retain the P-256 and ML-KEM private keys. Only the
/// resulting 32-byte shared secrets cross this interface.
pub trait HybridSecretDecapsulator {
    fn entity_id(&self) -> &EntityId;
    fn hepk(&self) -> &Hepk;
    fn derive_dh_shared(&self, peer: &DhPublicKey) -> Result<Zeroizing<[u8; 32]>>;
    fn decapsulate_mlkem768(&self, ciphertext: &[u8]) -> Result<Zeroizing<[u8; 32]>>;
}

/// Derives the exact Ed25519 identity and hybrid encryption public key used by
/// a v0.1.9 device. Secret intermediates are zeroized on drop.
pub fn derive_device_public(seed: &SecretSeed) -> Result<DevicePublicMaterial> {
    let signing_seed = derive_key(seed, 0, None)?;
    let signing = SigningKey::from_bytes(signing_seed.as_bytes());
    let mut id = Vec::with_capacity(33);
    id.push(foks_proto::ENTITY_DEVICE);
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
    expected_host: &EntityId,
) -> Result<SharedKeySeed> {
    let receiver = SoftwareDecapsulator::new(device_seed)?;
    open_puk_parcel_with(
        parcel,
        &receiver,
        sender_hepk,
        expected_puk_verify_key,
        expected_host,
    )
}

/// Opens a PUK parcel with a software or hardware-backed receiver.
pub fn open_puk_parcel_with(
    parcel: &PukParcel,
    receiver: &dyn HybridSecretDecapsulator,
    sender_hepk: &Hepk,
    expected_puk_verify_key: &EntityId,
    expected_host: &EntityId,
) -> Result<SharedKeySeed> {
    if parcel.role != Role::OWNER || parcel.hybrid.sender_dh.is_some() {
        return Err(Error::HybridBox);
    }
    if &parcel.target != receiver.entity_id() {
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
        || &shared.host != expected_host
        || shared.generation != parcel.generation
        || shared.role != parcel.role
    {
        return Err(Error::PukBinding);
    }
    let derived_puk = derive_shared_verify_key(&shared.seed, foks_proto::ENTITY_PUK_VERIFY)?;
    if &derived_puk != expected_puk_verify_key {
        return Err(Error::PukBinding);
    }
    Ok(shared)
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
        if parcel.temp_dh_key.is_some() {
            return Err(Error::HybridBox);
        }
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
    let payload = Zeroizing::new(encode(&Value::Array(vec![
        Value::Unsigned(1),
        Value::Binary(kem_shared.as_slice().to_vec()),
        Value::Binary(dh_shared.as_slice().to_vec()),
        foks_snowpack::decode(&receiver.hepk().encoded()?)?,
        dh_public_value(sender_dh),
    ]))?);
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

struct SoftwareDecapsulator<'a> {
    seed: &'a SecretSeed,
    public: DevicePublicMaterial,
}

impl<'a> SoftwareDecapsulator<'a> {
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
        let DhPublicKey::Curve25519(peer) = peer else {
            return Err(Error::HybridBox);
        };
        let dh_seed = derive_key(self.seed, 1, None)?;
        let secret = StaticSecret::from(*dh_seed.as_bytes());
        let raw = secret.diffie_hellman(&X25519PublicKey::from(*peer));
        let zero = [0u8; 16];
        let shared = hsalsa::<U10>(raw.as_bytes().into(), (&zero).into());
        Ok(Zeroizing::new(shared.into()))
    }

    fn decapsulate_mlkem768(&self, ciphertext: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
        let mlkem_seed = Zeroizing::new(derive_mlkem_seed(self.seed)?);
        let kem_seed =
            ml_kem::Seed::try_from(mlkem_seed.as_slice()).map_err(|_| Error::DeviceKey)?;
        let decapsulation = ml_kem_768::DecapsulationKey::from_seed(kem_seed);
        let ciphertext = ml_kem_768::Ciphertext::try_from(ciphertext).map_err(|_| Error::MlKem)?;
        Ok(Zeroizing::new(
            decapsulation.decapsulate(&ciphertext).into(),
        ))
    }
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

    let receiver_hepk = foks_snowpack::decode(&receiver_hepk.encoded()?)?;
    let sender_dh = Value::Array(vec![
        Value::Unsigned(1),
        Value::Variant(Some((
            b"0".to_vec(),
            Box::new(Value::Binary(sender_key.to_vec())),
        ))),
    ]);
    let payload = Zeroizing::new(encode(&Value::Array(vec![
        Value::Unsigned(1),
        Value::Binary(kem_shared.as_slice().to_vec()),
        Value::Binary(dh_shared.as_slice().to_vec()),
        receiver_hepk,
        sender_dh,
    ]))?);
    let mut hash = <Sha3_256 as Sha3Digest>::new();
    hash.update(HYBRID_SECRET_KEY_SHA3_PAYLOAD_TYPE_ID.to_be_bytes());
    hash.update(&payload);
    Ok((Zeroizing::new(hash.finalize().into()), payload))
}

fn derive_shared_verify_key(seed: &SecretSeed, entity_type: u8) -> Result<EntityId> {
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

/// Verifies a FOKS Ed25519 signature over an already encoded typed object.
pub fn verify_typed(
    signer: &EntityId,
    signature: &Signature,
    type_id: u64,
    canonical_object: &[u8],
) -> Result<()> {
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
    let encoded_blob = encode(&Value::Binary(inner.to_vec()))?;
    verify_typed(signer, signature, blob_type_id, &encoded_blob)
}

#[cfg(test)]
mod tests {
    use super::*;
    use foks_proto::{ProbeResponse, PukParcel, UserChain, UserLink, PUBLIC_ZONE_BLOB_TYPE_ID};

    const PROBE: &[u8] = include_bytes!(
        "../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
    );
    const USER_DIR: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9/user";

    fn user_fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{USER_DIR}/{name}")).unwrap()
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
        let rotated = chain.links[2].decode_group_change().unwrap().shared_keys[0]
            .verify_key
            .clone();
        let parcel = PukParcel::decode(&user_fixture("puk-parcel.snowp")).unwrap();
        let (hybrid_key, payload) =
            derive_hybrid_key(&parcel, &seed, derived.hepk.classical(), &derived.hepk).unwrap();
        assert_eq!(payload.as_slice(), user_fixture("hybrid-payload.snowp"));
        assert_eq!(
            hybrid_key.as_slice(),
            user_fixture("hybrid-secretbox-key.bin")
        );
        let clear = open_puk_parcel(&parcel, &seed, &derived.hepk, &rotated, &eldest.host).unwrap();
        assert_eq!(clear.seed.as_slice(), user_fixture("puk-seed.bin"));
    }

    #[test]
    fn hybrid_puk_tampering_is_rejected() {
        let seed = SecretSeed::new(user_fixture("device-seed.bin").try_into().unwrap());
        let derived = derive_device_public(&seed).unwrap();
        let chain = UserChain::decode(&user_fixture("user-chain.snowp")).unwrap();
        let eldest = chain.links[0].decode_eldest().unwrap();
        let rotated = chain.links[2].decode_group_change().unwrap().shared_keys[0]
            .verify_key
            .clone();
        let mut parcel = PukParcel::decode(&user_fixture("puk-parcel.snowp")).unwrap();
        parcel.hybrid.ciphertext[0] ^= 1;
        assert!(matches!(
            open_puk_parcel(&parcel, &seed, &derived.hepk, &rotated, &eldest.host,),
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
        let clear = open_puk_parcel(
            &parcel,
            &seed,
            &sender_hepk,
            &eldest.puk_verify_key,
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
            &eldest.host,
        )
        .is_err());
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
        let rotated = chain.links[2].decode_group_change().unwrap().shared_keys[0]
            .verify_key
            .clone();
        let parcel = PukParcel::decode(&user_fixture("puk-parcel.snowp")).unwrap();
        let clear = open_puk_parcel_with(
            &parcel,
            &hardware,
            &hardware.public.hepk,
            &rotated,
            &eldest.host,
        )
        .unwrap();
        assert_eq!(clear.seed.as_slice(), user_fixture("puk-seed.bin"));
    }
}
