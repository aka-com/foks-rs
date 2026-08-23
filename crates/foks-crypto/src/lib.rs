//! Cryptographic operations for the implemented FOKS v0.1.9 client protocols.
//!
//! This crate owns typed hashes and signatures, device and shared-key
//! derivation, hybrid key distribution, mutation construction, and KV
//! authenticated encryption. Protocol policy remains in `foks-verify`.

#![forbid(unsafe_code)]

use crypto_secretbox::{aead::Aead, KeyInit, XSalsa20Poly1305};
use ed25519_dalek::{Signature as DalekSignature, Signer as _, SigningKey, VerifyingKey};
use foks_proto::{
    AdHocMembershipLinkPublic, ChangeMetadata, DeviceLabelNameAndCommitmentKey, DhPublicKey,
    EntityId, Hepk, HybridBox, KvDirectory, KvDirent, KvDirentName, KvEncryptedChunk,
    KvLargeFileMetadata, KvNodeId, KvParty, KvRoot, KvSmallFileBox, KvSmallFilePlaintext,
    KvUploadChunk, KvUploadFinal, PukParcel, Role, RoleAndGeneration, SecretBox, SecretSeed,
    SharedKeyBox, SharedKeyBoxSet, SharedKeyBoxTarget, SharedKeySeed, Signature,
    SoftwareEldestPublic, SubkeySeed, TeamGroupChange, TeamKeyOwner, TeamMemberChange,
    TeamMemberKeys, TreeRoot, UnsignedUserLink, UserGroupChange, UserLink, UserMemberChange,
    UserMemberKeys, UserSharedKey, APP_KEY_DERIVATION_TYPE_ID, DEVICE_LABEL_TYPE_ID, HEPK_TYPE_ID,
    HYBRID_SECRET_KEY_SHA3_PAYLOAD_TYPE_ID, KV_CHUNK_NONCE_PAYLOAD_TYPE_ID,
    KV_DIRENT_BINDING_PAYLOAD_TYPE_ID, KV_DIRENT_NAME_PAYLOAD_TYPE_ID, KV_FILE_KEY_PAYLOAD_TYPE_ID,
    KV_KEY_DERIVATION_TYPE_ID, KV_ROOT_BINDING_PAYLOAD_TYPE_ID, LINK_OUTER_V1_TYPE_ID,
    NAME_COMMITMENT_TYPE_ID, SHARED_KEY_SEED_TYPE_ID, SUBKEY_SEED_TYPE_ID,
    TEMP_DH_KEY_SIG_TEMPLATE_TYPE_ID, TREE_LOCATION_TYPE_ID,
};
use foks_snowpack::{decode, decode_prefix, encode, Value};
use hmac::{Hmac, Mac};
use ml_kem::{ml_kem_768, Decapsulate as _, KeyExport as _, TryKeyInit as _};
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
    #[error("invalid ad-hoc team key or hidden-location material")]
    AdHocTeamMaterial,
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
        let (value, consumed) = decode_prefix(&plaintext)?;
        require_zero_padding(&plaintext, consumed)?;
        let Value::Binary(seed) = value else {
            return Err(Error::KvBinding);
        };
        let seed: [u8; 32] = seed.try_into().map_err(|_| Error::KvBinding)?;
        Ok(SecretSeed::new(seed))
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
        KvSmallFilePlaintext::decode_value(&value).map_err(Into::into)
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
        let (value, consumed) = decode_prefix(&plaintext)?;
        require_zero_padding(&plaintext, consumed)?;
        let fields = match value {
            Value::Array(fields) if fields.len() == 3 => fields,
            _ => return Err(Error::KvBinding),
        };
        if fields[0] != Value::Binary(id.object_id().to_vec())
            || fields[1] != Value::Unsigned(metadata.version)
        {
            return Err(Error::KvBinding);
        }
        let Value::Binary(seed) = &fields[2] else {
            return Err(Error::KvBinding);
        };
        let seed: [u8; 32] = seed.as_slice().try_into().map_err(|_| Error::KvBinding)?;
        Ok(SecretSeed::new(seed))
    }

    pub fn seal_directory_seed(
        &self,
        directory_id: [u8; 16],
        seed: &SecretSeed,
    ) -> Result<Vec<u8>> {
        seal_typed_secretbox(
            &self.box_key,
            DIR_KEY_SEED_TYPE_ID,
            &directory_id,
            &encode(&Value::Binary(seed.as_bytes().to_vec()))?,
            false,
        )
    }

    pub fn seal_small_file(
        &self,
        id: KvNodeId,
        key: RoleAndGeneration,
        plaintext: KvSmallFilePlaintext,
    ) -> Result<KvSmallFileBox> {
        Ok(KvSmallFileBox {
            key,
            ciphertext: seal_typed_secretbox(
                &self.box_key,
                SMALL_FILE_PAYLOAD_TYPE_ID,
                &id.object_id(),
                &encode(&plaintext.to_value())?,
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
        let plaintext = encode(&Value::Array(vec![
            Value::Binary(id.object_id().to_vec()),
            Value::Unsigned(version),
            Value::Binary(file_seed.as_bytes().to_vec()),
        ]))?;
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
    let payload = encode(
        &KvDirentName {
            parent,
            directory_version,
            name,
        }
        .to_value(),
    )?;
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
    let encoded = encode(&Value::Binary(cleartext.to_vec()))?;
    let padded_length = kv_chunk_padded_length(encoded.len())?;
    let mut padded = Zeroizing::new(vec![0; padded_length]);
    padded[..encoded.len()].copy_from_slice(&encoded);
    let nonce_value = encode(&Value::Array(vec![
        Value::Binary(file_id.object_id().to_vec()),
        Value::Unsigned(offset),
        Value::Bool(final_chunk),
    ]))?;
    let hash = prefixed_hash(KV_CHUNK_NONCE_PAYLOAD_TYPE_ID, &nonce_value);
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
    let name = KvDirentName::decode_value(&value)?;
    if name.parent != dirent.parent || name.directory_version != dirent.directory_version {
        return Err(Error::KvBinding);
    }
    Ok(name)
}

/// Opens one large-file chunk and rejects a server-adjusted offset that does
/// not contain the requested byte.
pub fn open_kv_chunk(
    file_seed: &SecretSeed,
    file_id: KvNodeId,
    requested_offset: u64,
    chunk: &KvEncryptedChunk,
) -> Result<Vec<u8>> {
    if chunk.offset < requested_offset {
        return Err(Error::KvBinding);
    }
    let nonce_value = encode(&Value::Array(vec![
        Value::Binary(file_id.object_id().to_vec()),
        Value::Unsigned(requested_offset),
        Value::Bool(chunk.final_chunk),
    ]))?;
    let hash = prefixed_hash(KV_CHUNK_NONCE_PAYLOAD_TYPE_ID, &nonce_value);
    let nonce: [u8; 24] = hash[..24].try_into().expect("slice length is fixed");
    let cipher = XSalsa20Poly1305::new(file_seed.as_bytes().into());
    let plaintext = cipher
        .decrypt((&nonce).into(), chunk.ciphertext.as_ref())
        .map_err(|_| Error::Decryption)?;
    let (value, consumed) = decode_prefix(&plaintext)?;
    require_zero_padding(&plaintext, consumed)?;
    let Value::Binary(bytes) = value else {
        return Err(Error::KvBinding);
    };
    let skip = usize::try_from(chunk.offset - requested_offset).map_err(|_| Error::KvBinding)?;
    bytes
        .get(skip..)
        .map(ToOwned::to_owned)
        .ok_or(Error::KvBinding)
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

/// Signs an exact canonical object with a PUK/PTK seed using FOKS's typed
/// Ed25519 message format.
pub fn sign_shared_key_typed(
    seed: &SecretSeed,
    type_id: u64,
    canonical_object: &[u8],
) -> Result<Signature> {
    foks_snowpack::validate(canonical_object)?;
    let signing_seed = derive_key(seed, 0, None)?;
    let signing = SigningKey::from_bytes(signing_seed.as_bytes());
    let mut message = Vec::with_capacity(8 + canonical_object.len());
    message.extend_from_slice(&type_id.to_be_bytes());
    message.extend_from_slice(canonical_object);
    Ok(Signature::Ed25519(signing.sign(&message).to_bytes()))
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

pub struct AdHocTeamInput<'a> {
    pub user: &'a EntityId,
    pub host: &'a EntityId,
    pub root: &'a TreeRoot,
    pub time: u64,
    pub owner_puk_generation: u64,
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

/// Derives the permanent ad-hoc TeamID selected by FOKS from its admin PTK.
pub fn adhoc_team_id_from_admin_seed(seed: &SecretSeed) -> Result<EntityId> {
    let admin = derive_shared_public(seed, foks_proto::ENTITY_PTK_VERIFY)?;
    let mut team_bytes = admin.verify_key.as_bytes().to_vec();
    team_bytes[0] = foks_proto::ENTITY_AD_HOC_TEAM;
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
        next_location_commitment: prefixed_hash(
            TREE_LOCATION_TYPE_ID,
            &encode(&Value::Binary(input.next_tree_location.to_vec()))?,
        ),
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
                subchain_location_commitment: prefixed_hash(
                    TREE_LOCATION_TYPE_ID,
                    &encode(&Value::Binary(input.subchain_tree_location.to_vec()))?,
                ),
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
            sequence: 1,
            previous: None,
            root: input.root,
            time: 0,
            next_location_commitment: prefixed_hash(
                TREE_LOCATION_TYPE_ID,
                &encode(&Value::Binary(input.membership_next_tree_location.to_vec()))?,
            ),
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

/// Constructs a complete software-device provision link. If the requested
/// role has no PUK yet, `introduced_puk` supplies generation 1 and signs first.
/// The new device countersigns before the existing owner device.
pub fn make_software_provision_link(
    input: &SoftwareProvisionInput<'_>,
    existing_device_seed: &SecretSeed,
    new_device_seed: &SecretSeed,
    introduced_puk: Option<&SecretSeed>,
) -> Result<SoftwareProvisionMaterial> {
    let existing = derive_device_public(existing_device_seed)?;
    let device = derive_device_public(new_device_seed)?;
    let introduced = introduced_puk
        .map(|seed| derive_shared_public(seed, foks_proto::ENTITY_PUK_VERIFY))
        .transpose()?;
    let label = input.device_label;
    let label_bytes = encode(&Value::Array(vec![
        Value::Unsigned(label.device_type),
        Value::Text(label.normalized_name.clone()),
        Value::Unsigned(label.serial),
    ]))?;
    let shared_keys = match introduced.as_ref() {
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
        next_location_commitment: prefixed_hash(
            TREE_LOCATION_TYPE_ID,
            &encode(&Value::Binary(input.base.next_tree_location.to_vec()))?,
        ),
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
    let unsigned = UnsignedUserLink::user_group_change(&change)?;
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

pub fn seal_puk_seed_chain_box(
    new_seed: &SecretSeed,
    previous_seed: &SecretSeed,
    party: &EntityId,
    host: &EntityId,
    generation: u64,
    role: Role,
    nonce: [u8; 16],
) -> Result<foks_proto::SeedChainBox> {
    let cleartext = Zeroizing::new(encode(&Value::Array(vec![
        Value::Array(vec![
            Value::Binary(party.as_bytes().to_vec()),
            Value::Binary(host.as_bytes().to_vec()),
        ]),
        Value::Unsigned(generation),
        role.to_value(),
        Value::Binary(previous_seed.as_slice().to_vec()),
    ]))?);
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
        next_location_commitment: prefixed_hash(
            TREE_LOCATION_TYPE_ID,
            &encode(&Value::Binary(base.next_tree_location.to_vec()))?,
        ),
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
    pub receiver_hepk: &'a Hepk,
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
            receiver_hepk: &input.receiver.hepk,
        })
        .collect::<Vec<_>>();
    seal_shared_key_boxes(
        host,
        sender_seed,
        &sender.hepk,
        box_id,
        &generic,
        randomness,
    )
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
    if inputs.is_empty() || inputs.len() != randomness.len() {
        return Err(Error::HybridBox);
    }
    if derive_device_public(sender_seed)?.hepk != *sender_hepk {
        return Err(Error::HybridBox);
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
        let receiver_hepk = decode(&input.receiver_hepk.encoded()?)?;
        let payload = Zeroizing::new(encode(&Value::Array(vec![
            Value::Unsigned(1),
            Value::Binary(kem_shared.as_slice().to_vec()),
            Value::Binary(dh_shared.as_slice().to_vec()),
            receiver_hepk,
            dh_public_value(&DhPublicKey::Curve25519(sender_dh)),
        ]))?);
        let mut hash = <Sha3_256 as Sha3Digest>::new();
        hash.update(HYBRID_SECRET_KEY_SHA3_PAYLOAD_TYPE_ID.to_be_bytes());
        hash.update(payload.as_slice());
        let key = Zeroizing::new(<[u8; 32]>::from(hash.finalize()));
        let cleartext = Zeroizing::new(encode(&Value::Array(vec![
            Value::Array(vec![
                Value::Binary(input.receiver_id.as_bytes().to_vec()),
                Value::Binary(host.as_bytes().to_vec()),
            ]),
            Value::Unsigned(input.generation),
            input.role.to_value(),
            Value::Binary(input.seed.as_slice().to_vec()),
        ]))?);
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
                host: None,
                role: Role::NONE,
                generation: 0,
            },
        });
    }
    SharedKeyBoxSet::new(box_id, boxes, None).map_err(Into::into)
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
        Value::Unsigned(label.device_type),
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
        next_location_commitment: prefixed_hash(
            TREE_LOCATION_TYPE_ID,
            &encode(&Value::Binary(input.next_tree_location.to_vec()))?,
        ),
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
        subchain_location_commitment: prefixed_hash(
            TREE_LOCATION_TYPE_ID,
            &encode(&Value::Binary(input.subchain_tree_location.to_vec()))?,
        ),
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

pub fn derive_shared_public(seed: &SecretSeed, entity_type: u8) -> Result<SharedPublicMaterial> {
    let device = derive_public_material(seed, entity_type)?;
    Ok(SharedPublicMaterial {
        verify_key: device.id,
        hepk: device.hepk,
    })
}

pub fn hepk_fingerprint(hepk: &Hepk) -> Result<[u8; 32]> {
    Ok(prefixed_hash(HEPK_TYPE_ID, &hepk.encoded()?))
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
    let receiver_hepk = decode(&device.hepk.encoded()?)?;
    let sender_dh_value = dh_public_value(&DhPublicKey::Curve25519(sender_dh));
    let payload = Zeroizing::new(encode(&Value::Array(vec![
        Value::Unsigned(1),
        Value::Binary(kem_shared.as_slice().to_vec()),
        Value::Binary(dh_shared.as_slice().to_vec()),
        receiver_hepk,
        sender_dh_value,
    ]))?);
    let mut hash = <Sha3_256 as Sha3Digest>::new();
    hash.update(HYBRID_SECRET_KEY_SHA3_PAYLOAD_TYPE_ID.to_be_bytes());
    hash.update(payload.as_slice());
    let key = Zeroizing::new(<[u8; 32]>::from(hash.finalize()));
    let cleartext = encode(&Value::Array(vec![
        Value::Array(vec![
            Value::Binary(device.id.as_bytes().to_vec()),
            Value::Binary(host.as_bytes().to_vec()),
        ]),
        Value::Unsigned(1),
        Role::OWNER.to_value(),
        Value::Binary(puk_seed.as_slice().to_vec()),
    ]))?;
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

/// Complete hardware boundary for a FOKS v0.1.9 Yubi credential.
///
/// `sign_sha512_256` receives the already hashed 32-byte FOKS signature
/// payload and returns an ASN.1 DER P-256 ECDSA signature. Implementations
/// should keep the P-256 and ML-KEM private keys inside the hardware provider.
pub trait YubiDevice: HybridSecretDecapsulator {
    fn sign_sha512_256(&self, digest: &[u8; 32]) -> Result<Vec<u8>>;
}

fn sign_yubi_typed(
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
    expected_host: &EntityId,
) -> Result<SharedKeySeed> {
    open_puk_parcel_for_role(
        parcel,
        device_seed,
        sender_hepk,
        expected_puk_verify_key,
        expected_host,
        Role::OWNER,
    )
}

pub fn open_puk_parcel_for_role(
    parcel: &PukParcel,
    device_seed: &SecretSeed,
    sender_hepk: &Hepk,
    expected_puk_verify_key: &EntityId,
    expected_host: &EntityId,
    expected_role: Role,
) -> Result<SharedKeySeed> {
    let receiver = SoftwareDecapsulator::new(device_seed)?;
    open_puk_parcel_with_for_role(
        parcel,
        &receiver,
        sender_hepk,
        expected_puk_verify_key,
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
    expected_host: &EntityId,
) -> Result<SharedKeySeed> {
    open_puk_parcel_with_for_role(
        parcel,
        receiver,
        sender_hepk,
        expected_puk_verify_key,
        expected_host,
        Role::OWNER,
    )
}

pub fn open_puk_parcel_with_for_role(
    parcel: &PukParcel,
    receiver: &dyn HybridSecretDecapsulator,
    sender_hepk: &Hepk,
    expected_puk_verify_key: &EntityId,
    expected_host: &EntityId,
    expected_role: Role,
) -> Result<SharedKeySeed> {
    open_shared_key_parcel_with(
        parcel,
        receiver,
        sender_hepk,
        expected_puk_verify_key,
        expected_host,
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

/// Opens a PUK or PTK parcel and binds its cleartext to the authenticated
/// target, role, generation, host, and expected verification key.
#[allow(clippy::too_many_arguments)]
pub fn open_shared_key_parcel_with(
    parcel: &PukParcel,
    receiver: &dyn HybridSecretDecapsulator,
    sender_hepk: &Hepk,
    expected_verify_key: &EntityId,
    expected_host: &EntityId,
    expected_role: Role,
    expected_verify_key_type: u8,
) -> Result<SharedKeySeed> {
    if parcel.role != expected_role || parcel.hybrid.sender_dh.is_some() {
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
    let derived = derive_shared_verify_key(&shared.seed, expected_verify_key_type)?;
    if &derived != expected_verify_key {
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
    use foks_proto::{
        ProbeResponse, PukParcel, TeamChain, UserChain, UserLink, ENTITY_PTK_VERIFY,
        PUBLIC_ZONE_BLOB_TYPE_ID,
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
                        device_type: 0,
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
                receiver_hepk: &owner_public.hepk,
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
                &change.host,
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
            &host,
            Role::OWNER,
        )
        .unwrap();
        assert_eq!(opened.seed.as_bytes(), &puk_bytes);

        let mut tampered = parcel;
        tampered.hybrid.ciphertext[0] ^= 1;
        assert!(open_puk_parcel_for_role(
            &tampered,
            &device_seed,
            &device.hepk,
            &puk.verify_key,
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
        let rotated = chain.links[2].decode_group_change().unwrap().shared_keys[0]
            .verify_key
            .clone();
        let mut parcel = PukParcel::decode(&user_fixture("puk-parcel.snowp")).unwrap();
        let (hybrid_key, payload) =
            derive_hybrid_key(&parcel, &seed, derived.hepk.classical(), &derived.hepk).unwrap();
        assert_eq!(payload.as_slice(), user_fixture("hybrid-payload.snowp"));
        assert_eq!(
            hybrid_key.as_slice(),
            user_fixture("hybrid-secretbox-key.bin")
        );
        let clear = open_puk_parcel(&parcel, &seed, &derived.hepk, &rotated, &eldest.host).unwrap();
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
            &rotated,
            &eldest.host,
        )
        .unwrap();
        assert!(open_puk_seed_chain(clear, &wrong_generation, &eldest.uid, &eldest.host).is_err());
        parcel.seed_chain[0].secret_box.ciphertext[0] ^= 1;
        let clear = open_puk_parcel(&parcel, &seed, &derived.hepk, &rotated, &eldest.host).unwrap();
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
            let clear = open_shared_key_parcel_with(
                parcel,
                &receiver,
                receiver.hepk(),
                &key.verify_key,
                &change.host,
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
    fn large_chunk_matches_official_requested_and_returned_offset_semantics() {
        let file_seed = SecretSeed::new([0x55; 32]);
        let mut id = [0x77; 17];
        id[0] = 2;
        let id = KvNodeId(id);
        let requested = 3;
        let returned = 5;
        let clear = b"abcdef".to_vec();
        let encoded = encode(&Value::Binary(clear)).unwrap();
        let nonce_value = encode(&Value::Array(vec![
            Value::Binary(id.object_id().to_vec()),
            Value::Unsigned(requested),
            Value::Bool(true),
        ]))
        .unwrap();
        let hash = prefixed_hash(KV_CHUNK_NONCE_PAYLOAD_TYPE_ID, &nonce_value);
        let nonce: [u8; 24] = hash[..24].try_into().unwrap();
        let cipher = XSalsa20Poly1305::new(file_seed.as_bytes().into());
        let ciphertext = cipher.encrypt((&nonce).into(), encoded.as_ref()).unwrap();
        let chunk = KvEncryptedChunk {
            ciphertext,
            offset: returned,
            final_chunk: true,
        };
        assert_eq!(
            open_kv_chunk(&file_seed, id, requested, &chunk).unwrap(),
            b"cdef"
        );
        let behind = KvEncryptedChunk { offset: 2, ..chunk };
        assert!(matches!(
            open_kv_chunk(&file_seed, id, requested, &behind),
            Err(Error::KvBinding)
        ));
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
