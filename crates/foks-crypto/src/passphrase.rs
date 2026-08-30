//! v0.1.9 passphrase stretching and passphrase-encryption (PPE) ceremonies.

use argon2::{Algorithm, Argon2, Params, Version};
use foks_proto::{
    EntityId, Hepk, PassphraseLoginResult, PassphraseUpdateArgument, PpeParcel, PpePassphraseBox,
    PpePassphraseBoxPayload, PpePukBox, PpePukBoxPayload, RegistrationChallenge, Role, SecretBox,
    SecretSeed, SkmwkList, StretchVersion, FIRST_PASSPHRASE_GENERATION,
    PPE_PASSPHRASE_BOX_PAYLOAD_TYPE_ID, PPE_PUK_BOX_PAYLOAD_TYPE_ID, SKMWK_LIST_TYPE_ID,
};
use zeroize::{Zeroize, Zeroizing};

use super::{
    derive_key, derive_public_material, open_hybrid_box, open_typed_secretbox, seal_hybrid_payload,
    seal_typed_secretbox, sign_seed_typed, Error, PukBoxRandomness, Result, SoftwareDecapsulator,
};

pub const MAX_PASSPHRASE_BYTES: usize = 1024;

/// Raw user input retained only in zeroizing memory and always redacted from
/// diagnostics. Whitespace is significant at this layer; UIs decide whether
/// to trim before construction.
pub struct Passphrase(Zeroizing<Vec<u8>>);

impl Passphrase {
    pub fn new(input: impl AsRef<[u8]>) -> Result<Self> {
        let input = input.as_ref();
        if input.is_empty() || input.len() > MAX_PASSPHRASE_BYTES {
            return Err(Error::Passphrase);
        }
        Ok(Self(Zeroizing::new(input.to_vec())))
    }

    pub fn expose(&self) -> &[u8] {
        self.0.as_slice()
    }
}

impl std::fmt::Debug for Passphrase {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Passphrase([REDACTED])")
    }
}

pub struct PassphraseKeyring {
    generation: u64,
    keys: Vec<[u8; 32]>,
}

impl PassphraseKeyring {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn key(&self, generation: u64) -> Option<&[u8; 32]> {
        generation
            .checked_sub(FIRST_PASSPHRASE_GENERATION)
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| self.keys.get(index))
    }

    fn from_list(list: SkmwkList, expected_generation: u64) -> Result<Self> {
        if expected_generation < FIRST_PASSPHRASE_GENERATION
            || list.keys.len()
                != usize::try_from(expected_generation).map_err(|_| Error::Passphrase)?
        {
            return Err(Error::Passphrase);
        }
        let mut list = list;
        Ok(Self {
            generation: expected_generation,
            keys: std::mem::take(&mut list.keys),
        })
    }
}

impl Drop for PassphraseKeyring {
    fn drop(&mut self) {
        self.keys.zeroize();
    }
}

impl std::fmt::Debug for PassphraseKeyring {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PassphraseKeyring")
            .field("generation", &self.generation)
            .field("keys", &"[REDACTED]")
            .finish()
    }
}

pub struct PassphraseUpdate {
    pub verify_key: EntityId,
    pub salt: [u8; 16],
    pub generation: u64,
    pub stretch_version: StretchVersion,
    pub skmwk_box: SecretBox,
    pub passphrase_box: PpePassphraseBox,
    pub puk_box: PpePukBox,
    pub keyring: PassphraseKeyring,
}

impl std::fmt::Debug for PassphraseUpdate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PassphraseUpdate")
            .field("verify_key", &self.verify_key)
            .field("generation", &self.generation)
            .field("stretch_version", &self.stretch_version)
            .field("encrypted", &"PPE boxes")
            .finish()
    }
}

impl PassphraseUpdate {
    pub fn argument(&self) -> PassphraseUpdateArgument {
        PassphraseUpdateArgument {
            verify_key: self.verify_key.clone(),
            salt: self.salt,
            generation: self.generation,
            skmwk_box: self.skmwk_box.clone(),
            passphrase_box: self.passphrase_box.clone(),
            puk_box: Some(self.puk_box.clone()),
            stretch_version: self.stretch_version,
            user_settings_link: None,
        }
    }
}

struct StretchedPassphrase {
    seed: SecretSeed,
    _local_box_key: Zeroizing<[u8; 32]>,
    public: super::DevicePublicMaterial,
}

/// One stretched login credential reused for both challenge signing and PPE
/// decryption. Keeping it across the RPC avoids running production Argon2id
/// twice for one verification ceremony.
pub struct PassphraseLoginCredential(StretchedPassphrase);

impl std::fmt::Debug for PassphraseLoginCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PassphraseLoginCredential([REDACTED])")
    }
}

pub fn prepare_passphrase_login(
    passphrase: &Passphrase,
    salt: &[u8; 16],
    stretch_version: StretchVersion,
) -> Result<PassphraseLoginCredential> {
    Ok(PassphraseLoginCredential(stretch(
        passphrase,
        salt,
        stretch_version,
        false,
    )?))
}

/// Applies the exact v0.1.9 Argon2id parameter set. `Test` exists only for
/// deterministic unit tests and must be opted into explicitly.
fn stretch(
    passphrase: &Passphrase,
    salt: &[u8; 16],
    version: StretchVersion,
    allow_test: bool,
) -> Result<StretchedPassphrase> {
    let (memory_kib, iterations, lanes) = match version {
        StretchVersion::V1 => (64 * 1024, 3, 2),
        StretchVersion::Test if allow_test => (512, 1, 1),
        StretchVersion::Test => return Err(Error::PassphraseStretch),
    };
    let params = Params::new(memory_kib, iterations, lanes, Some(64))
        .map_err(|_| Error::PassphraseStretch)?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut stream = Zeroizing::new([0u8; 64]);
    argon
        .hash_password_into(passphrase.expose(), salt, stream.as_mut())
        .map_err(|_| Error::PassphraseStretch)?;
    let seed = SecretSeed::from_slice(&stream[..32])?;
    let mut local_box_key = Zeroizing::new([0u8; 32]);
    local_box_key.copy_from_slice(&stream[32..]);
    let public = derive_public_material(&seed, foks_proto::ENTITY_PASSPHRASE_KEY)?;
    Ok(StretchedPassphrase {
        seed,
        _local_box_key: local_box_key,
        public,
    })
}

pub fn create_passphrase_enrollment(
    passphrase: &Passphrase,
    uid: &EntityId,
    host: &EntityId,
    owner_puk: &SecretSeed,
    owner_puk_generation: u64,
    stretch_version: StretchVersion,
) -> Result<PassphraseUpdate> {
    let salt = random_array()?;
    create_passphrase_enrollment_with_options(
        passphrase,
        uid,
        host,
        owner_puk,
        owner_puk_generation,
        stretch_version,
        false,
        salt,
    )
}

#[allow(clippy::too_many_arguments)]
fn create_passphrase_enrollment_with_options(
    passphrase: &Passphrase,
    uid: &EntityId,
    host: &EntityId,
    owner_puk: &SecretSeed,
    owner_puk_generation: u64,
    stretch_version: StretchVersion,
    allow_test: bool,
    salt: [u8; 16],
) -> Result<PassphraseUpdate> {
    if salt == [0; 16] || owner_puk_generation == 0 {
        return Err(Error::Passphrase);
    }
    let stretched = stretch(passphrase, &salt, stretch_version, allow_test)?;
    build_update(
        uid,
        host,
        salt,
        stretch_version,
        stretched.public.id.clone(),
        &stretched.public.hepk,
        Vec::new(),
        owner_puk,
        owner_puk_generation,
    )
}

pub fn change_passphrase_with_puk(
    passphrase: &Passphrase,
    uid: &EntityId,
    host: &EntityId,
    parcel: &PpeParcel,
    current_owner_puk: &SecretSeed,
    new_owner_puk: &SecretSeed,
    new_owner_puk_generation: u64,
) -> Result<PassphraseUpdate> {
    let keyring = unlock_with_puk(uid, host, parcel, current_owner_puk)?;
    let stretched = stretch(passphrase, &parcel.salt, parcel.stretch_version, false)?;
    build_update(
        uid,
        host,
        parcel.salt,
        parcel.stretch_version,
        stretched.public.id.clone(),
        &stretched.public.hepk,
        keyring.keys.clone(),
        new_owner_puk,
        new_owner_puk_generation,
    )
}

/// Reboxes PPE during an owner-PUK rotation without asking for the raw
/// passphrase. This is the v0.1.9 revoke/rotation annex operation.
pub fn rotate_passphrase_for_puk(
    uid: &EntityId,
    host: &EntityId,
    parcel: &PpeParcel,
    current_owner_puk: &SecretSeed,
    new_owner_puk: &SecretSeed,
    new_owner_puk_generation: u64,
) -> Result<PassphraseUpdate> {
    let (keyring, public) = unlock_with_puk_and_public(uid, host, parcel, current_owner_puk)?;
    build_update(
        uid,
        host,
        parcel.salt,
        parcel.stretch_version,
        parcel.verify_key.clone(),
        &public,
        keyring.keys.clone(),
        new_owner_puk,
        new_owner_puk_generation,
    )
}

/// Proves that the stored PPE recovery parcel is authenticated to the given
/// owner PUK and that its encrypted SKMWK history is internally consistent.
pub fn verify_passphrase_puk_recovery(
    uid: &EntityId,
    host: &EntityId,
    parcel: &PpeParcel,
    owner_puk: &SecretSeed,
) -> Result<()> {
    drop(unlock_with_puk(uid, host, parcel, owner_puk)?);
    Ok(())
}

/// Opens the current PUK recovery parcel into the compact package carried by
/// interactive KEX. The returned SKMWK is secret and zeroizes on drop.
pub fn export_kex_ppe(
    uid: &EntityId,
    host: &EntityId,
    parcel: &PpeParcel,
    owner_puk: &SecretSeed,
) -> Result<foks_proto::KexPpe> {
    let keyring = unlock_with_puk(uid, host, parcel, owner_puk)?;
    let skmwk = *keyring.key(parcel.generation).ok_or(Error::Passphrase)?;
    Ok(foks_proto::KexPpe {
        skmwk,
        passphrase_generation: parcel.generation,
        salt: parcel.salt,
        stretch_version: parcel.stretch_version,
    })
}

#[allow(clippy::too_many_arguments)]
fn build_update(
    uid: &EntityId,
    host: &EntityId,
    salt: [u8; 16],
    stretch_version: StretchVersion,
    verify_key: EntityId,
    passphrase_public: &Hepk,
    mut keys: Vec<[u8; 32]>,
    owner_puk: &SecretSeed,
    owner_puk_generation: u64,
) -> Result<PassphraseUpdate> {
    if owner_puk_generation == 0 || salt == [0; 16] {
        return Err(Error::Passphrase);
    }
    let generation = u64::try_from(keys.len())
        .ok()
        .and_then(|value| value.checked_add(FIRST_PASSPHRASE_GENERATION))
        .ok_or(Error::Passphrase)?;
    keys.push(random_array()?);
    let list = SkmwkList {
        uid: uid.clone().require_type(foks_proto::ENTITY_USER)?,
        host: host.clone().require_type(foks_proto::ENTITY_HOST)?,
        keys: keys.clone(),
    };
    let session_key = Zeroizing::new(random_array()?);
    let list_nonce = random_array()?;
    let exact_list = Zeroizing::new(list.encoded()?);
    let skmwk_box = SecretBox {
        nonce: list_nonce,
        ciphertext: seal_typed_secretbox(
            &session_key,
            SKMWK_LIST_TYPE_ID,
            &list_nonce,
            &exact_list,
            false,
        )?,
    };

    let payload = PpePassphraseBoxPayload {
        generation,
        session_key: *session_key,
    };
    let ephemeral_seed = SecretSeed::new(random_array()?);
    let ephemeral_public = derive_public_material(&ephemeral_seed, foks_proto::ENTITY_DEVICE)?;
    let hybrid_randomness = PukBoxRandomness {
        kem_message: random_array()?,
        nonce: random_array()?,
    };
    let exact_payload = Zeroizing::new(payload.encoded()?);
    let passphrase_box = PpePassphraseBox {
        hybrid: seal_hybrid_payload(
            &ephemeral_seed,
            &ephemeral_public.hepk,
            passphrase_public,
            PPE_PASSPHRASE_BOX_PAYLOAD_TYPE_ID,
            &exact_payload,
            &hybrid_randomness,
            true,
        )?,
    };

    let puk_payload = PpePukBoxPayload {
        generation,
        session_key: *session_key,
        passphrase_public_key: passphrase_public.clone(),
    };
    let puk_key = derive_key(owner_puk, 2, None)?;
    let puk_nonce = random_array()?;
    let exact_puk_payload = Zeroizing::new(puk_payload.encoded()?);
    let puk_box = PpePukBox {
        secret_box: SecretBox {
            nonce: puk_nonce,
            ciphertext: seal_typed_secretbox(
                puk_key.as_bytes(),
                PPE_PUK_BOX_PAYLOAD_TYPE_ID,
                &puk_nonce,
                &exact_puk_payload,
                false,
            )?,
        },
        puk_generation: owner_puk_generation,
        puk_role: Role::OWNER,
    };
    Ok(PassphraseUpdate {
        verify_key,
        salt,
        generation,
        stretch_version,
        skmwk_box,
        passphrase_box,
        puk_box,
        keyring: PassphraseKeyring { generation, keys },
    })
}

impl PassphraseLoginCredential {
    pub fn sign_challenge(
        &self,
        challenge: &RegistrationChallenge,
    ) -> Result<foks_proto::Signature> {
        sign_seed_typed(
            &self.0.seed,
            foks_proto::REG_CHALLENGE_PAYLOAD_TYPE_ID,
            &challenge.payload.encoded()?,
        )
    }

    pub fn unlock(
        &self,
        uid: &EntityId,
        host: &EntityId,
        result: &PassphraseLoginResult,
    ) -> Result<PassphraseKeyring> {
        let sender = result
            .passphrase_box
            .hybrid
            .sender_dh
            .as_ref()
            .ok_or(Error::HybridBox)?;
        let receiver = SoftwareDecapsulator::new(&self.0.seed)?;
        let cleartext = open_hybrid_box(
            &result.passphrase_box.hybrid,
            &receiver,
            sender,
            PPE_PASSPHRASE_BOX_PAYLOAD_TYPE_ID,
        )?;
        let payload = PpePassphraseBoxPayload::decode(&cleartext)?;
        if payload.generation != result.generation {
            return Err(Error::Passphrase);
        }
        open_keyring(
            uid,
            host,
            result.generation,
            &payload.session_key,
            &result.skmwk_box,
        )
    }
}

fn unlock_with_puk(
    uid: &EntityId,
    host: &EntityId,
    parcel: &PpeParcel,
    owner_puk: &SecretSeed,
) -> Result<PassphraseKeyring> {
    unlock_with_puk_and_public(uid, host, parcel, owner_puk).map(|(keyring, _)| keyring)
}

fn unlock_with_puk_and_public(
    uid: &EntityId,
    host: &EntityId,
    parcel: &PpeParcel,
    owner_puk: &SecretSeed,
) -> Result<(PassphraseKeyring, Hepk)> {
    let puk_box = parcel.puk_box.as_ref().ok_or(Error::Passphrase)?;
    if puk_box.puk_role != Role::OWNER {
        return Err(Error::Passphrase);
    }
    let key = derive_key(owner_puk, 2, None)?;
    let cleartext = open_typed_secretbox(
        key.as_bytes(),
        PPE_PUK_BOX_PAYLOAD_TYPE_ID,
        &puk_box.secret_box.nonce,
        &puk_box.secret_box.ciphertext,
    )?;
    let payload = PpePukBoxPayload::decode(&cleartext)?;
    if payload.generation != parcel.generation {
        return Err(Error::Passphrase);
    }
    let keyring = open_keyring(
        uid,
        host,
        parcel.generation,
        &payload.session_key,
        &parcel.skmwk_box,
    )?;
    Ok((keyring, payload.passphrase_public_key.clone()))
}

fn open_keyring(
    uid: &EntityId,
    host: &EntityId,
    generation: u64,
    session_key: &[u8; 32],
    boxed: &SecretBox,
) -> Result<PassphraseKeyring> {
    let cleartext = open_typed_secretbox(
        session_key,
        SKMWK_LIST_TYPE_ID,
        &boxed.nonce,
        &boxed.ciphertext,
    )?;
    let list = SkmwkList::decode(&cleartext)?;
    if list.uid != *uid || list.host != *host {
        return Err(Error::Passphrase);
    }
    PassphraseKeyring::from_list(list, generation)
}

fn random_array<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| Error::Passphrase)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(kind: u8, fill: u8) -> EntityId {
        EntityId::from_bytes([vec![kind], vec![fill; 32]].concat()).unwrap()
    }

    #[test]
    fn test_only_stretch_matches_the_upstream_v019_parameters() {
        let passphrase = Passphrase::new("correct horse battery staple").unwrap();
        let salt = *b"0123456789abcdef";
        let stretched = stretch(&passphrase, &salt, StretchVersion::Test, true).unwrap();
        assert_eq!(
            stretched.public.id.as_bytes(),
            &[
                17, 74, 226, 63, 162, 8, 43, 26, 47, 245, 173, 75, 181, 197, 21, 222, 201, 8, 74,
                21, 31, 164, 74, 27, 101, 117, 251, 203, 252, 154, 64, 114, 81,
            ]
        );
        assert_eq!(
            stretched._local_box_key.as_slice(),
            &[
                180, 195, 116, 183, 228, 32, 222, 53, 24, 208, 215, 141, 173, 231, 11, 252, 54, 71,
                33, 169, 88, 195, 252, 149, 84, 104, 7, 43, 29, 86, 154, 147,
            ]
        );
    }

    #[test]
    fn production_stretch_matches_the_go_v019_argon2id_stream() {
        // Reference: go-foks v0.1.9 client/libclient/passphrase.go's
        // argon2.IDKey(raw, salt, 3, 64*1024, 2, 64).
        let passphrase = Passphrase::new("correct horse battery staple").unwrap();
        let salt = *b"0123456789abcdef";
        let stretched = stretch(&passphrase, &salt, StretchVersion::V1, false).unwrap();
        assert_eq!(
            stretched.seed.as_slice(),
            &[
                0x9e, 0xe7, 0xc3, 0xa4, 0x2b, 0x41, 0x51, 0xaa, 0x50, 0xab, 0xaa, 0xd3, 0xaa, 0xf8,
                0x63, 0x01, 0xf4, 0xef, 0x30, 0x5e, 0x65, 0xae, 0x89, 0xb0, 0x0d, 0x1e, 0x7a, 0x25,
                0xf6, 0x18, 0x0b, 0xb5,
            ]
        );
        assert_eq!(
            stretched._local_box_key.as_slice(),
            &[
                0x34, 0xb2, 0x57, 0x4e, 0xc4, 0xcd, 0xf8, 0xc9, 0x61, 0x07, 0x12, 0x12, 0x63, 0x76,
                0xfc, 0x62, 0x56, 0xe1, 0xbb, 0x7f, 0x69, 0x6d, 0x62, 0x78, 0xb4, 0x3b, 0xe5, 0x62,
                0x83, 0xb0, 0xf8, 0xaa,
            ]
        );
    }

    #[test]
    fn enrollment_login_change_and_puk_rotation_preserve_the_key_history() {
        let uid = entity(foks_proto::ENTITY_USER, 3);
        let host = entity(foks_proto::ENTITY_HOST, 4);
        let puk1 = SecretSeed::new([5; 32]);
        let puk2 = SecretSeed::new([6; 32]);
        let passphrase = Passphrase::new("first passphrase").unwrap();
        let update = create_passphrase_enrollment_with_options(
            &passphrase,
            &uid,
            &host,
            &puk1,
            1,
            StretchVersion::Test,
            true,
            [7; 16],
        )
        .unwrap();
        assert_eq!(update.generation, 1);
        assert!(update.keyring.key(1).is_some());

        let parcel = PpeParcel {
            skmwk_box: update.skmwk_box.clone(),
            generation: update.generation,
            passphrase_box: update.passphrase_box.clone(),
            puk_box: Some(update.puk_box.clone()),
            salt: update.salt,
            stretch_version: update.stretch_version,
            verify_key: update.verify_key.clone(),
        };
        verify_passphrase_puk_recovery(&uid, &host, &parcel, &puk1).unwrap();
        let mut forged_generation = parcel.clone();
        forged_generation.puk_box.as_mut().unwrap().puk_generation = 2;
        assert!(verify_passphrase_puk_recovery(&uid, &host, &forged_generation, &puk2).is_err());
        let mut forged_role = parcel.clone();
        forged_role.puk_box.as_mut().unwrap().puk_role = Role::ADMIN;
        assert!(verify_passphrase_puk_recovery(&uid, &host, &forged_role, &puk1).is_err());
        let rotated = rotate_passphrase_for_puk(&uid, &host, &parcel, &puk1, &puk2, 2).unwrap();
        assert_eq!(rotated.generation, 2);
        assert_eq!(rotated.verify_key, parcel.verify_key);
        assert_eq!(rotated.keyring.key(1), update.keyring.key(1));
        assert!(rotated.keyring.key(2).is_some());
    }
}
