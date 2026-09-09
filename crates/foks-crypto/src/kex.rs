//! FOKS v0.1.9 HESP channels, encrypted KEX packets, and split provisioning.

use bip39::Language;
use crypto_secretbox::{aead::Aead, KeyInit, XSalsa20Poly1305};
use foks_proto::{
    ChangeMetadata, KexCleartext, KexWrapperMessage, SecretBox, SecretSeed, UnsignedUserLink,
    UserGroupChange, UserLink, UserMemberChange, UserMemberKeys, DEVICE_LABEL_TYPE_ID,
    ENTITY_DEVICE, KEX_CLEARTEXT_TYPE_ID, KEX_KEY_DERIVATION_TYPE_ID, KEX_WRAPPER_MSG_TYPE_ID,
    LINK_OUTER_V1_TYPE_ID,
};
use foks_snowpack::{encode, Value};
use hmac::{Hmac, Mac};
use sha2::Sha512_256;
use zeroize::Zeroizing;

use crate::{
    commitment, derive_device_public, hepk_fingerprint, sign_seed_typed, tree_location_commitment,
    verify_typed, DevicePublicMaterial, Error, Result, SoftwareProvisionInput,
    SoftwareProvisionMaterial,
};

const KEX_WORDS: usize = 7;
const KEX_NUMBERS: usize = 6;
const KEX_PHRASE_TOKENS: usize = KEX_WORDS + KEX_NUMBERS;
const WORD_BITS: usize = 11;
const NUMBER_BITS: usize = 8;
const KEX_TOP_MASK: u8 = 0x1f;

#[derive(Debug, thiserror::Error)]
pub enum KexPhraseError {
    #[error("KEX phrase has {found} tokens, expected {KEX_PHRASE_TOKENS}")]
    TokenCount { found: usize },
    #[error("KEX phrase word {index} is not in the BIP-39 English dictionary")]
    Word { index: usize },
    #[error("KEX phrase number {index} is not an integer")]
    Number { index: usize },
    #[error("KEX phrase number {index} is outside 0..256")]
    NumberRange { index: usize },
    #[error("KEX secret has nonzero unused high bits")]
    HighBits,
    #[error("KEX phrase arithmetic overflowed its fixed-width secret")]
    Overflow,
    #[error("operating-system randomness is unavailable")]
    Entropy,
}

pub struct KexSecret(Zeroizing<[u8; foks_proto::KEX_SECRET_BYTES]>);

impl std::fmt::Debug for KexSecret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("KexSecret([REDACTED])")
    }
}

pub struct KexPhrase(Zeroizing<Vec<String>>);

impl std::fmt::Debug for KexPhrase {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("KexPhrase([REDACTED])")
    }
}

impl KexPhrase {
    pub fn expose_tokens(&self) -> &[String] {
        &self.0
    }

    pub fn expose_joined(&self) -> Zeroizing<String> {
        Zeroizing::new(self.0.join(" "))
    }
}

impl KexSecret {
    pub fn generate() -> std::result::Result<Self, KexPhraseError> {
        let mut bytes = Zeroizing::new([0u8; foks_proto::KEX_SECRET_BYTES]);
        getrandom::fill(&mut *bytes).map_err(|_| KexPhraseError::Entropy)?;
        bytes[0] &= KEX_TOP_MASK;
        Ok(Self(bytes))
    }

    pub fn from_bytes(
        bytes: [u8; foks_proto::KEX_SECRET_BYTES],
    ) -> std::result::Result<Self, KexPhraseError> {
        if bytes[0] & !KEX_TOP_MASK != 0 {
            return Err(KexPhraseError::HighBits);
        }
        Ok(Self(Zeroizing::new(bytes)))
    }

    pub fn from_phrase(phrase: &str) -> std::result::Result<Self, KexPhraseError> {
        let tokens = phrase.split_whitespace().collect::<Vec<_>>();
        if tokens.len() != KEX_PHRASE_TOKENS {
            return Err(KexPhraseError::TokenCount {
                found: tokens.len(),
            });
        }
        let dictionary = Language::English.word_list();
        let mut words = Zeroizing::new([0u16; KEX_WORDS]);
        let mut numbers = Zeroizing::new([0u16; KEX_NUMBERS]);
        for (index, token) in tokens.iter().step_by(2).enumerate() {
            words[index] = dictionary
                .binary_search(&token.to_ascii_lowercase().as_str())
                .map_err(|_| KexPhraseError::Word { index })?
                .try_into()
                .expect("the BIP-39 dictionary has 2048 entries");
        }
        for (index, token) in tokens.iter().skip(1).step_by(2).enumerate() {
            let number = token
                .parse::<i64>()
                .map_err(|_| KexPhraseError::Number { index })?;
            if !(0..256).contains(&number) {
                return Err(KexPhraseError::NumberRange { index });
            }
            numbers[index] = number as u16;
        }
        let mut bytes = Zeroizing::new([0u8; foks_proto::KEX_SECRET_BYTES]);
        push_bits(&mut bytes[..], words[KEX_WORDS - 1], WORD_BITS)?;
        for index in (0..KEX_NUMBERS).rev() {
            push_bits(&mut bytes[..], numbers[index], NUMBER_BITS)?;
            push_bits(&mut bytes[..], words[index], WORD_BITS)?;
        }
        Self::from_bytes(*bytes)
    }

    pub fn phrase(&self) -> KexPhrase {
        let dictionary = Language::English.word_list();
        let mut remaining = Zeroizing::new(*self.0);
        let mut tokens = Zeroizing::new(Vec::with_capacity(KEX_PHRASE_TOKENS));
        for index in 0..KEX_WORDS {
            let word = take_low_bits(&mut remaining[..], WORD_BITS);
            tokens.push(dictionary[usize::from(word)].to_owned());
            if index < KEX_NUMBERS {
                tokens.push(take_low_bits(&mut remaining[..], NUMBER_BITS).to_string());
            }
        }
        debug_assert!(remaining.iter().all(|byte| *byte == 0));
        KexPhrase(tokens)
    }

    pub fn secret_bytes(&self) -> Zeroizing<[u8; foks_proto::KEX_SECRET_BYTES]> {
        Zeroizing::new(*self.0)
    }

    pub fn keys(&self) -> Result<KexKeys> {
        let derive = |kind| {
            let object = encode(&Value::Array(vec![
                Value::Unsigned(kind),
                Value::Variant(None),
            ]))
            .expect("fixed KEX derivation is canonical");
            let mut mac = <Hmac<Sha512_256> as Mac>::new_from_slice(self.0.as_slice())
                .expect("HMAC accepts every key length");
            mac.update(&KEX_KEY_DERIVATION_TYPE_ID.to_be_bytes());
            mac.update(&object);
            <[u8; 32]>::from(mac.finalize().into_bytes())
        };
        Ok(KexKeys {
            session_id: derive(0),
            secretbox_key: Zeroizing::new(derive(1)),
        })
    }
}

pub struct KexKeys {
    pub session_id: [u8; 32],
    secretbox_key: Zeroizing<[u8; 32]>,
}

impl KexKeys {
    pub fn seal(&self, cleartext: &KexCleartext, nonce: [u8; 16]) -> Result<SecretBox> {
        if cleartext.session_id != self.session_id {
            return Err(Error::Protocol(foks_proto::Error::IntegerRange(
                "KEX session binding",
            )));
        }
        let mut full_nonce = [0u8; 24];
        full_nonce[..8].copy_from_slice(&KEX_CLEARTEXT_TYPE_ID.to_be_bytes());
        full_nonce[8..].copy_from_slice(&nonce);
        let ciphertext = XSalsa20Poly1305::new((&*self.secretbox_key).into())
            .encrypt((&full_nonce).into(), cleartext.encoded()?.as_slice())
            .map_err(|_| Error::KvEncryption)?;
        Ok(SecretBox { nonce, ciphertext })
    }

    pub fn open(&self, wrapper: &KexWrapperMessage) -> Result<KexCleartext> {
        if wrapper.session_id != self.session_id {
            return Err(Error::Decryption);
        }
        let mut full_nonce = [0u8; 24];
        full_nonce[..8].copy_from_slice(&KEX_CLEARTEXT_TYPE_ID.to_be_bytes());
        full_nonce[8..].copy_from_slice(&wrapper.payload.nonce);
        let plaintext = XSalsa20Poly1305::new((&*self.secretbox_key).into())
            .decrypt((&full_nonce).into(), wrapper.payload.ciphertext.as_slice())
            .map(Zeroizing::new)
            .map_err(|_| Error::Decryption)?;
        let cleartext = KexCleartext::decode(&plaintext)?;
        if cleartext.session_id != wrapper.session_id
            || cleartext.sender != wrapper.sender
            || cleartext.sequence != wrapper.sequence
        {
            return Err(Error::Verification);
        }
        Ok(cleartext)
    }
}

pub fn sign_kex_wrapper(
    seed: &SecretSeed,
    message: &KexWrapperMessage,
) -> Result<foks_proto::Signature> {
    sign_seed_typed(seed, KEX_WRAPPER_MSG_TYPE_ID, &message.signing_bytes()?)
}

pub fn verify_kex_wrapper(
    message: &KexWrapperMessage,
    signature: &foks_proto::Signature,
) -> Result<()> {
    verify_typed(
        &message.sender,
        signature,
        KEX_WRAPPER_MSG_TYPE_ID,
        &message.signing_bytes()?,
    )
}

pub fn make_software_kex_provision_link(
    input: &SoftwareProvisionInput<'_>,
    existing_device_seed: &SecretSeed,
    new_device: &DevicePublicMaterial,
) -> Result<SoftwareProvisionMaterial> {
    new_device.id.clone().require_type(ENTITY_DEVICE)?;
    let existing = derive_device_public(existing_device_seed)?;
    let label = input.device_label;
    let label_bytes = encode(&Value::Array(vec![
        Value::Unsigned(label.device_type.protocol_value()),
        Value::Text(label.normalized_name.clone()),
        Value::Unsigned(label.serial),
    ]))?;
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
            entity: new_device.id.clone(),
            scoped_host: None,
            source_role: foks_proto::Role::NONE,
            keys: UserMemberKeys::User {
                hepk_fingerprint: hepk_fingerprint(&new_device.hepk)?,
                subkey: None,
            },
        }],
        shared_keys: Vec::new(),
        metadata: vec![ChangeMetadata::DeviceName(commitment(
            DEVICE_LABEL_TYPE_ID,
            &label_bytes,
            &input.device_name_commitment_key,
        ))],
    };
    Ok(SoftwareProvisionMaterial {
        link: UnsignedUserLink::user_group_change(&change)?.finish_for_kex()?,
        device: new_device.clone(),
        introduced_puk: None,
        device_name_commitment_key: input.device_name_commitment_key,
        next_tree_location: input.base.next_tree_location,
    })
}

pub fn countersign_software_kex_provision_link(
    link: &UserLink,
    new_device_seed: &SecretSeed,
) -> Result<UserLink> {
    if !link.signatures().is_empty() {
        return Err(Error::Verification);
    }
    let device = derive_device_public(new_device_seed)?;
    require_kex_device_binding(link, &device)?;
    let signature = sign_seed_typed(
        new_device_seed,
        LINK_OUTER_V1_TYPE_ID,
        &link.signing_bytes(0)?,
    )?;
    Ok(link.with_appended_signature(signature)?)
}

pub fn finish_software_kex_provision_link(
    link: &UserLink,
    existing_device_seed: &SecretSeed,
    new_device: &DevicePublicMaterial,
) -> Result<UserLink> {
    if link.signatures().len() != 1 {
        return Err(Error::Verification);
    }
    require_kex_device_binding(link, new_device)?;
    let change = link.decode_group_change()?;
    let existing = derive_device_public(existing_device_seed)?;
    if change.signer != existing.id {
        return Err(Error::Verification);
    }
    verify_typed(
        &new_device.id,
        &link.signatures()[0],
        LINK_OUTER_V1_TYPE_ID,
        &link.signing_bytes(0)?,
    )?;
    let signature = sign_seed_typed(
        existing_device_seed,
        LINK_OUTER_V1_TYPE_ID,
        &link.signing_bytes(1)?,
    )?;
    Ok(link.with_appended_signature(signature)?)
}

fn require_kex_device_binding(link: &UserLink, device: &DevicePublicMaterial) -> Result<()> {
    let change = link.decode_group_change()?;
    // When enrolling a new role without an existing key, the server validates
    // the role key transition; the provisionee only verifies its own device change.
    if change.changes.len() != 1 {
        return Err(Error::Verification);
    }
    let member = &change.changes[0];
    let UserMemberKeys::User {
        hepk_fingerprint: expected,
        subkey: None,
    } = &member.keys
    else {
        return Err(Error::Verification);
    };
    if member.entity != device.id
        || *expected != hepk_fingerprint(&device.hepk)?
        || member.role == foks_proto::Role::NONE
        || member.source_role != foks_proto::Role::NONE
        || member.scoped_host.is_some()
    {
        return Err(Error::Verification);
    }
    Ok(())
}

fn push_bits(
    bytes: &mut [u8],
    value: u16,
    count: usize,
) -> std::result::Result<(), KexPhraseError> {
    for _ in 0..count {
        let mut carry = 0u8;
        for byte in bytes.iter_mut().rev() {
            let next = *byte >> 7;
            *byte = (*byte << 1) | carry;
            carry = next;
        }
        if carry != 0 {
            return Err(KexPhraseError::Overflow);
        }
    }
    let tail = bytes.len() - 2;
    let merged = u16::from_be_bytes([bytes[tail], bytes[tail + 1]]) | value;
    [bytes[tail], bytes[tail + 1]] = merged.to_be_bytes();
    Ok(())
}

fn take_low_bits(bytes: &mut [u8], count: usize) -> u16 {
    let tail = bytes.len() - 2;
    let value = u16::from_be_bytes([bytes[tail], bytes[tail + 1]]) & ((1 << count) - 1);
    for _ in 0..count {
        let mut carry = 0u8;
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

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn kex_phrase_round_trips_boundaries() {
        let mut maximum = [u8::MAX; 16];
        maximum[0] = KEX_TOP_MASK;
        for bytes in [[0u8; 16], maximum] {
            let secret = KexSecret::from_bytes(bytes).unwrap();
            let phrase = secret.phrase().expose_joined();
            let restored = KexSecret::from_phrase(&phrase).unwrap();
            assert_eq!(
                restored.keys().unwrap().session_id,
                secret.keys().unwrap().session_id
            );
        }
    }

    #[test]
    fn kex_derivation_matches_go_v0_1_9() {
        let secret = KexSecret::from_bytes([1; 16]).unwrap();
        assert_eq!(
            secret.phrase().expose_joined().as_str(),
            "cage 32 advice 4 letter 128 avoid 16 acoustic 2 doctor 64 amount"
        );
        let keys = secret.keys().unwrap();
        assert_eq!(
            hex(&keys.session_id),
            "7efbeaaecd9291ea952864fe363ee175de54b420212ceacc444421cd19dd777d"
        );
        assert_eq!(
            hex(keys.secretbox_key.as_slice()),
            "e9c6819da5d9c7333a282f64c5c8da3ca6351259dcf9c8d81f90c87fc258fc4f"
        );
        let sender = foks_proto::EntityId::from_bytes(
            [vec![foks_proto::ENTITY_DEVICE], vec![2; 32]].concat(),
        )
        .unwrap();
        let cleartext = KexCleartext {
            session_id: keys.session_id,
            sender: sender.clone(),
            sequence: 0,
            message: foks_proto::KexMessage::Start,
        };
        assert_eq!(
            hex(&cleartext.encoded().unwrap()),
            "94c4207efbeaaecd9291ea952864fe363ee175de54b420212ceacc444421cd19dd777dc42104020202020202020202020202020202020202020202020202020202020202020200920180"
        );
        let payload = keys.seal(&cleartext, [3; 16]).unwrap();
        assert_eq!(
            hex(&payload.ciphertext),
            "268c7c445e35566466e0d88d08b32926a7abb4d4566803cb676529d819abc21d9bbb9e346b75d253aed9eaee7c9120b0714c2d4812ae9b67a161f936fe03a7fa31775f4d17da90a3fbf92e5c9b808f27384de7558b6046156ac1"
        );
        assert_eq!(
            keys.open(&KexWrapperMessage {
                session_id: keys.session_id,
                sender,
                sequence: 0,
                payload,
            })
            .unwrap(),
            cleartext
        );
    }

    #[test]
    fn kex_secretbox_rejects_wrapper_rebinding() {
        let secret = KexSecret::from_bytes([1; 16]).unwrap();
        let keys = secret.keys().unwrap();
        let sender = derive_device_public(&SecretSeed::new([2; 32])).unwrap().id;
        let cleartext = KexCleartext {
            session_id: keys.session_id,
            sender: sender.clone(),
            sequence: 0,
            message: foks_proto::KexMessage::Start,
        };
        let payload = keys.seal(&cleartext, [3; 16]).unwrap();
        let mut wrapper = KexWrapperMessage {
            session_id: keys.session_id,
            sender,
            sequence: 0,
            payload,
        };
        assert_eq!(keys.open(&wrapper).unwrap(), cleartext);
        wrapper.sequence = 1;
        assert!(keys.open(&wrapper).is_err());
    }
}
