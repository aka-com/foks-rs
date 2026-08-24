//! FOKS v0.1.9 backup-key phrases and deterministic key material.

use bip39::Language;
use foks_proto::{
    DhPublicKey, EntityId, Hepk, RegistrationChallenge, SecretSeed, Signature, BACKUP_SEED_TYPE_ID,
    ENTITY_BACKUP_KEY, REG_CHALLENGE_PAYLOAD_TYPE_ID,
};
use thiserror::Error;
use zeroize::Zeroizing;

use crate::{
    derive_public_material, open_puk_parcel_with_for_role, prefixed_hash, sign_seed_typed,
    software_dh_shared, software_mlkem_decapsulate, DevicePublicMaterial, HybridSecretDecapsulator,
    Result, SharedKeySeed,
};

pub const BACKUP_SEED_BYTES: usize = 26;
pub const BACKUP_PHRASE_TOKENS: usize = 17;
const BACKUP_WORDS: usize = 9;
const BACKUP_NUMBERS: usize = 8;
const WORD_BITS: usize = 11;
const NUMBER_BITS: usize = 13;
const NUMBER_LIMIT: u16 = 1 << NUMBER_BITS;
const BACKUP_TOP_MASK: u8 = 0x07;

#[derive(Debug, Error)]
pub enum BackupPhraseError {
    #[error("backup phrase has {found} tokens, expected {BACKUP_PHRASE_TOKENS}")]
    TokenCount { found: usize },
    #[error("backup phrase word {index} is not in the BIP-39 English dictionary")]
    Word { index: usize },
    #[error("backup phrase number {index} is not an integer")]
    Number { index: usize },
    #[error("backup phrase number {index} is outside 0..{NUMBER_LIMIT}")]
    NumberRange { index: usize },
    #[error("backup seed has nonzero unused high bits")]
    HighBits,
    #[error("backup phrase arithmetic overflowed its fixed-width seed")]
    Overflow,
    #[error("operating-system randomness is unavailable")]
    Entropy,
    #[error("backup-key derivation failed: {0}")]
    Crypto(#[from] crate::Error),
}

pub type BackupResult<T, E = BackupPhraseError> = std::result::Result<T, E>;

/// The exact 203-bit FOKS backup seed, held in zeroizing storage.
pub struct BackupKey(Zeroizing<[u8; BACKUP_SEED_BYTES]>);

impl std::fmt::Debug for BackupKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BackupKey([REDACTED])")
    }
}

/// A sensitive 17-token HESP representation. Debug output is always redacted.
pub struct BackupPhrase(Zeroizing<Vec<String>>);

impl std::fmt::Debug for BackupPhrase {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BackupPhrase([REDACTED])")
    }
}

impl BackupPhrase {
    /// Exposes the secret phrase tokens to the caller.
    pub fn expose_tokens(&self) -> &[String] {
        self.0.as_slice()
    }

    pub fn expose_joined(&self) -> Zeroizing<String> {
        Zeroizing::new(self.0.join(" "))
    }
}

impl BackupKey {
    /// Generates a uniformly random FOKS v0.1.9 backup key.
    pub fn generate() -> BackupResult<Self> {
        let mut seed = Zeroizing::new([0_u8; BACKUP_SEED_BYTES]);
        getrandom::fill(&mut *seed).map_err(|_| BackupPhraseError::Entropy)?;
        seed[0] &= BACKUP_TOP_MASK;
        Self::from_zeroizing_seed(seed)
    }

    /// Imports exact seed bytes. Use [`Self::generate`] for new backup keys.
    pub fn from_seed(seed: [u8; BACKUP_SEED_BYTES]) -> BackupResult<Self> {
        Self::from_zeroizing_seed(Zeroizing::new(seed))
    }

    fn from_zeroizing_seed(seed: Zeroizing<[u8; BACKUP_SEED_BYTES]>) -> BackupResult<Self> {
        if seed[0] & !BACKUP_TOP_MASK != 0 {
            return Err(BackupPhraseError::HighBits);
        }
        Ok(Self(seed))
    }

    /// Parses a HESP phrase.
    ///
    /// This method cannot clear the caller-owned input. Callers should keep
    /// mutable phrase input in zeroizing storage and clear it after parsing.
    pub fn from_phrase(phrase: &str) -> BackupResult<Self> {
        let tokens = phrase.split_whitespace().collect::<Vec<_>>();
        if tokens.len() != BACKUP_PHRASE_TOKENS {
            return Err(BackupPhraseError::TokenCount {
                found: tokens.len(),
            });
        }
        let dictionary = Language::English.word_list();
        let mut words = Zeroizing::new([0_u16; BACKUP_WORDS]);
        let mut numbers = Zeroizing::new([0_u16; BACKUP_NUMBERS]);
        for (index, token) in tokens.iter().step_by(2).enumerate() {
            words[index] = dictionary
                .binary_search(token)
                .map_err(|_| BackupPhraseError::Word { index })?
                .try_into()
                .expect("the BIP-39 dictionary has 2048 entries");
        }
        for (index, token) in tokens.iter().skip(1).step_by(2).enumerate() {
            let value = token
                .parse::<i64>()
                .map_err(|_| BackupPhraseError::Number { index })?;
            if !(0..i64::from(NUMBER_LIMIT)).contains(&value) {
                return Err(BackupPhraseError::NumberRange { index });
            }
            numbers[index] = value as u16;
        }

        let mut seed = Zeroizing::new([0_u8; BACKUP_SEED_BYTES]);
        push_bits(&mut seed, words[BACKUP_WORDS - 1], WORD_BITS)?;
        for index in (0..BACKUP_NUMBERS).rev() {
            push_bits(&mut seed, numbers[index], NUMBER_BITS)?;
            push_bits(&mut seed, words[index], WORD_BITS)?;
        }
        Self::from_zeroizing_seed(seed)
    }

    /// Parses and clears a caller-transferred phrase buffer before returning.
    pub fn from_owned_phrase(phrase: String) -> BackupResult<Self> {
        let phrase = Zeroizing::new(phrase);
        Self::from_phrase(&phrase)
    }

    pub fn phrase(&self) -> BackupPhrase {
        let dictionary = Language::English.word_list();
        let mut remaining = Zeroizing::new(*self.0);
        let mut tokens = Zeroizing::new(Vec::with_capacity(BACKUP_PHRASE_TOKENS));
        for index in 0..BACKUP_WORDS {
            let word = take_low_bits(&mut remaining, WORD_BITS);
            tokens.push(dictionary[usize::from(word)].to_owned());
            if index < BACKUP_NUMBERS {
                let number = take_low_bits(&mut remaining, NUMBER_BITS);
                tokens.push(number.to_string());
            }
        }
        debug_assert!(remaining.iter().all(|byte| *byte == 0));
        BackupPhrase(tokens)
    }

    /// The first word/number pair is the public device name used by FOKS.
    pub fn device_name(&self) -> String {
        let phrase = self.phrase();
        format!(
            "{} {}",
            phrase.expose_tokens()[0],
            phrase.expose_tokens()[1]
        )
    }

    pub(crate) fn derived_seed(&self) -> SecretSeed {
        // Canonical Snowpack encodes a 26-byte blob as bin8(26). Building it
        // directly avoids placing the backup seed in a standard `Vec` owned
        // by the generic value tree.
        let mut encoded = Zeroizing::new([0_u8; BACKUP_SEED_BYTES + 2]);
        encoded[0] = 0xc4;
        encoded[1] = BACKUP_SEED_BYTES as u8;
        encoded[2..].copy_from_slice(self.0.as_slice());
        SecretSeed::new(prefixed_hash(BACKUP_SEED_TYPE_ID, encoded.as_slice()))
    }

    pub fn public_material(&self) -> BackupResult<DevicePublicMaterial> {
        Ok(derive_public_material(
            &self.derived_seed(),
            ENTITY_BACKUP_KEY,
        )?)
    }

    #[cfg(test)]
    fn seed_bytes(&self) -> &[u8; BACKUP_SEED_BYTES] {
        &self.0
    }

    pub fn sign_registration_challenge(
        &self,
        challenge: &RegistrationChallenge,
    ) -> BackupResult<Signature> {
        Ok(sign_seed_typed(
            &self.derived_seed(),
            REG_CHALLENGE_PAYLOAD_TYPE_ID,
            &challenge.payload.encoded().map_err(crate::Error::from)?,
        )?)
    }

    pub(crate) fn key_material(&self) -> BackupResult<BackupKeyMaterial> {
        let seed = self.derived_seed();
        let public = derive_public_material(&seed, ENTITY_BACKUP_KEY)?;
        Ok(BackupKeyMaterial { seed, public })
    }

    pub fn into_key_material(self) -> BackupResult<BackupKeyMaterial> {
        let seed = self.derived_seed();
        let public = derive_public_material(&seed, ENTITY_BACKUP_KEY)?;
        Ok(BackupKeyMaterial { seed, public })
    }
}

/// Ephemeral derived suite used to sign, authenticate, and unbox during recovery.
pub struct BackupKeyMaterial {
    pub(crate) seed: SecretSeed,
    public: DevicePublicMaterial,
}

impl BackupKeyMaterial {
    /// Exposes a temporary PKCS#8 private key for the mTLS library boundary.
    pub fn expose_signing_key_pkcs8(&self) -> Result<Zeroizing<Vec<u8>>> {
        crate::device_signing_key_pkcs8(&self.seed)
    }

    pub fn sign_registration_challenge(
        &self,
        challenge: &RegistrationChallenge,
    ) -> Result<Signature> {
        sign_seed_typed(
            &self.seed,
            REG_CHALLENGE_PAYLOAD_TYPE_ID,
            &challenge.payload.encoded()?,
        )
    }
}

impl HybridSecretDecapsulator for BackupKeyMaterial {
    fn entity_id(&self) -> &EntityId {
        &self.public.id
    }

    fn hepk(&self) -> &Hepk {
        &self.public.hepk
    }

    fn derive_dh_shared(&self, peer: &DhPublicKey) -> Result<Zeroizing<[u8; 32]>> {
        software_dh_shared(&self.seed, peer)
    }

    fn decapsulate_mlkem768(&self, ciphertext: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
        software_mlkem_decapsulate(&self.seed, ciphertext)
    }
}

pub fn open_backup_puk_parcel_for_role(
    backup: &BackupKey,
    parcel: &foks_proto::PukParcel,
    sender_hepk: &Hepk,
    expected_puk_verify_key: &EntityId,
    expected_host: &EntityId,
    expected_role: foks_proto::Role,
) -> BackupResult<SharedKeySeed> {
    let receiver = backup.key_material()?;
    Ok(open_puk_parcel_with_for_role(
        parcel,
        &receiver,
        sender_hepk,
        expected_puk_verify_key,
        expected_host,
        expected_role,
    )?)
}

fn push_bits(bytes: &mut [u8; BACKUP_SEED_BYTES], value: u16, count: usize) -> BackupResult<()> {
    for _ in 0..count {
        let mut carry = 0_u8;
        for byte in bytes.iter_mut().rev() {
            let next = *byte >> 7;
            *byte = (*byte << 1) | carry;
            carry = next;
        }
        if carry != 0 {
            return Err(BackupPhraseError::Overflow);
        }
    }
    let tail = bytes.len() - 2;
    let merged = u16::from_be_bytes([bytes[tail], bytes[tail + 1]]) | value;
    [bytes[tail], bytes[tail + 1]] = merged.to_be_bytes();
    Ok(())
}

fn take_low_bits(bytes: &mut [u8; BACKUP_SEED_BYTES], count: usize) -> u16 {
    let tail = bytes.len() - 2;
    let value = u16::from_be_bytes([bytes[tail], bytes[tail + 1]]) & ((1 << count) - 1);
    for _ in 0..count {
        let mut carry = 0_u8;
        for byte in bytes.iter_mut() {
            let next = *byte & 1;
            *byte = (*byte >> 1) | (carry << 7);
            carry = next;
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use foks_proto::{EntityId, Hepk};
    use foks_snowpack::{decode, Value};
    use quickcheck::quickcheck;

    const FIXTURE_DIR: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9/user-mutations";

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{FIXTURE_DIR}/{name}")).unwrap()
    }

    #[test]
    fn phrase_round_trips_boundary_seed() {
        let mut maximum = [u8::MAX; BACKUP_SEED_BYTES];
        maximum[0] = BACKUP_TOP_MASK;
        for seed in [[0; BACKUP_SEED_BYTES], maximum] {
            let key = BackupKey::from_seed(seed).unwrap();
            let phrase = key.phrase().expose_joined();
            let decoded = BackupKey::from_phrase(&phrase).unwrap();
            assert_eq!(decoded.seed_bytes(), key.seed_bytes());
        }
    }

    #[test]
    fn generated_keys_always_have_a_valid_round_trip() {
        for _ in 0..16 {
            let key = BackupKey::generate().unwrap();
            let phrase = key.phrase().expose_joined();
            assert!(BackupKey::from_phrase(&phrase).is_ok());
        }
    }

    #[test]
    fn validates_phrase_shape_and_accepts_go_integer_syntax() {
        let key = BackupKey::from_seed([0; BACKUP_SEED_BYTES]).unwrap();
        let phrase = key.phrase().expose_joined();
        let too_short = phrase
            .split_whitespace()
            .take(16)
            .collect::<Vec<_>>()
            .join(" ");
        assert!(matches!(
            BackupKey::from_phrase(&too_short),
            Err(BackupPhraseError::TokenCount { found: 16 })
        ));
        assert_eq!(
            BackupKey::from_phrase(&phrase.replacen(" 0 ", " 00 ", 1))
                .unwrap()
                .seed_bytes(),
            key.seed_bytes()
        );
        assert!(matches!(
            BackupKey::from_seed([0xff; BACKUP_SEED_BYTES]),
            Err(BackupPhraseError::HighBits)
        ));

        let tokens = phrase.split_whitespace().collect::<Vec<_>>();
        let mut invalid_word = tokens.clone();
        invalid_word[0] = "not-a-bip39-word";
        assert!(matches!(
            BackupKey::from_phrase(&invalid_word.join(" ")),
            Err(BackupPhraseError::Word { index: 0 })
        ));

        let mut invalid_number = tokens.clone();
        invalid_number[1] = "NaN";
        assert!(matches!(
            BackupKey::from_phrase(&invalid_number.join(" ")),
            Err(BackupPhraseError::Number { index: 0 })
        ));

        let mut out_of_range = tokens;
        out_of_range[1] = "8192";
        assert!(matches!(
            BackupKey::from_phrase(&out_of_range.join(" ")),
            Err(BackupPhraseError::NumberRange { index: 0 })
        ));
    }

    #[test]
    fn debug_is_redacted() {
        let key = BackupKey::from_seed([0; BACKUP_SEED_BYTES]).unwrap();
        assert_eq!(format!("{key:?}"), "BackupKey([REDACTED])");
        assert_eq!(format!("{:?}", key.phrase()), "BackupPhrase([REDACTED])");
    }

    #[test]
    fn matches_official_go_backup_fixture() {
        let seed: [u8; BACKUP_SEED_BYTES] = fixture("backup-seed.bin").try_into().unwrap();
        let expected_phrase: Vec<String> =
            serde_json::from_slice(&fixture("backup-phrase.json")).unwrap();
        let expected_derived: [u8; 32] = fixture("backup-derived-seed.bin").try_into().unwrap();
        let key = BackupKey::from_seed(seed).unwrap();
        assert_eq!(key.phrase().expose_tokens(), expected_phrase);
        assert_eq!(key.derived_seed().as_bytes(), &expected_derived);

        let expected_id = match decode(&fixture("backup-entity-id.snowp")).unwrap() {
            Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
            _ => panic!("official backup EntityID is not a blob"),
        };
        let expected_hepk = Hepk::decode(&fixture("backup-hepk.snowp")).unwrap();
        let public = key.public_material().unwrap();
        assert_eq!(public.id, expected_id);
        assert_eq!(public.hepk, expected_hepk);
    }

    quickcheck! {
        fn arbitrary_valid_seeds_round_trip(left: u128, right: u128) -> bool {
            let mut source = [0_u8; 32];
            source[..16].copy_from_slice(&left.to_be_bytes());
            source[16..].copy_from_slice(&right.to_be_bytes());
            let mut seed = [0_u8; BACKUP_SEED_BYTES];
            seed.copy_from_slice(&source[..BACKUP_SEED_BYTES]);
            seed[0] &= BACKUP_TOP_MASK;
            let key = BackupKey::from_seed(seed).unwrap();
            let phrase = key.phrase().expose_joined();
            BackupKey::from_phrase(&phrase)
                .is_ok_and(|decoded| decoded.seed_bytes() == key.seed_bytes())
        }
    }
}
