//! Registration, device mutation, host-policy, and ad-hoc team arguments.

use super::{
    device_label_name_and_commitment_key, hepk, DeviceLabelNameAndCommitmentKey, Hepk, UserLink,
};
use crate::{
    array, boolean, decode, encode, entity, expect_unsigned, fixed_blob, list, list_or_null, role,
    text, unsigned, EntityId, Error, HybridBox, Result, Role, SecretSeed, SeedChainBox,
    SharedKeyBoxSet, TeamRemoteMemberViewToken, TeamRemovalKeyBox, Value, ENTITY_AD_HOC_TEAM,
    ENTITY_HOST, ENTITY_NAMED_TEAM, ENTITY_USER,
};
use zeroize::Zeroizing;

pub type TeamBearerToken = [u8; 16];

/// Exact inner object signed to activate a TeamAdmin bearer token.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamBearerTokenChallenge {
    pub user: EntityId,
    pub user_host: EntityId,
    pub team: EntityId,
    pub role: Role,
    pub generation: u64,
    pub token: TeamBearerToken,
    pub time: u64,
}

impl TeamBearerTokenChallenge {
    pub fn decode_payload(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 6)?;
        let user = array(&fields[0], 2)?;
        let result = Self {
            user: entity(&user[0])?,
            user_host: entity(&user[1])?,
            team: entity(&fields[1])?,
            role: role(&fields[2])?,
            generation: unsigned(&fields[3])?,
            token: fixed_blob(&fields[4], "team bearer token")?,
            time: unsigned(&fields[5])?,
        };
        result.validate()?;
        Ok(result)
    }

    pub fn encoded_payload(&self) -> Result<Vec<u8>> {
        self.validate()?;
        Ok(encode(&Value::Array(vec![
            Value::Array(vec![
                Value::Binary(self.user.as_bytes().to_vec()),
                Value::Binary(self.user_host.as_bytes().to_vec()),
            ]),
            Value::Binary(self.team.as_bytes().to_vec()),
            self.role.to_value(),
            Value::Unsigned(self.generation),
            Value::Binary(self.token.to_vec()),
            Value::Unsigned(self.time),
        ]))?)
    }

    /// Canonical `Future(T)` wrapper covered by the PTK signature.
    pub fn encoded_blob(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Binary(self.encoded_payload()?))?)
    }

    fn validate(&self) -> Result<()> {
        self.user.clone().require_type(ENTITY_USER)?;
        self.user_host.clone().require_type(ENTITY_HOST)?;
        self.team.clone().require_type(ENTITY_NAMED_TEAM)?;
        if self.role == Role::NONE || self.generation == 0 || self.time == 0 {
            return Err(Error::IntegerRange("team bearer-token challenge"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UsernameReservation {
    pub token: [u8; 17],
    pub sequence: u64,
    pub expires_at: u64,
}

/// Team and user name reservations have the same exact v0.1.9 wire schema.
pub type TeamNameReservation = UsernameReservation;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamRemovalKeyMetadata {
    pub team: EntityId,
    pub host: EntityId,
    pub member: EntityId,
    pub member_host: EntityId,
    pub source_role: Role,
    pub destination_role: Role,
    pub team_sequence: u64,
}

impl TeamRemovalKeyMetadata {
    fn from_value(value: &Value) -> Result<Self> {
        let fields = array(value, 4)?;
        let team = array(&fields[0], 2)?;
        let member = array(&fields[1], 2)?;
        let destination = array(&fields[3], 2)?;
        let result = Self {
            team: entity(&team[0])?,
            host: entity(&team[1])?,
            member: entity(&member[0])?,
            member_host: entity(&member[1])?,
            source_role: role(&fields[2])?,
            destination_role: role(&destination[0])?,
            team_sequence: unsigned(&destination[1])?,
        };
        result.validate()?;
        Ok(result)
    }

    pub(crate) fn to_value(&self) -> Value {
        Value::Array(vec![
            Value::Array(vec![
                Value::Binary(self.team.as_bytes().to_vec()),
                Value::Binary(self.host.as_bytes().to_vec()),
            ]),
            Value::Array(vec![
                Value::Binary(self.member.as_bytes().to_vec()),
                Value::Binary(self.member_host.as_bytes().to_vec()),
            ]),
            self.source_role.to_value(),
            Value::Array(vec![
                self.destination_role.to_value(),
                Value::Unsigned(self.team_sequence),
            ]),
        ])
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        self.validate()?;
        Ok(encode(&self.to_value())?)
    }

    fn validate(&self) -> Result<()> {
        self.team.clone().require_type(ENTITY_NAMED_TEAM)?;
        self.host.clone().require_type(ENTITY_HOST)?;
        self.member_host.clone().require_type(ENTITY_HOST)?;
        if !matches!(
            self.member.entity_type(),
            ENTITY_USER | ENTITY_NAMED_TEAM | ENTITY_AD_HOC_TEAM
        ) {
            return Err(Error::WrongEntityType {
                expected: ENTITY_USER,
                found: self.member.entity_type(),
            });
        }
        if self.source_role == Role::NONE
            || self.destination_role == Role::NONE
            || self.team_sequence == 0
        {
            return Err(Error::IntegerRange("team removal-key metadata"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamRemovalBoxData {
    pub commitment: [u8; 32],
    pub team_box: TeamRemovalKeyBox,
    pub member_box: TeamRemovalKeyBox,
    pub metadata: TeamRemovalKeyMetadata,
}

pub struct TeamRemovalKeyPayload {
    key: SecretSeed,
    pub metadata: TeamRemovalKeyMetadata,
}

impl TeamRemovalKeyPayload {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 35 || bytes[..3] != [0x92, 0xc4, 32] {
            return Err(Error::Length {
                kind: "team removal-key payload",
                expected: 35,
                found: bytes.len(),
            });
        }
        let mut redacted = Zeroizing::new(bytes.to_vec());
        let key = SecretSeed::from_slice(&redacted[3..35])?;
        redacted[3..35].fill(0);
        let value = decode(&redacted)?;
        let fields = array(&value, 2)?;
        if fixed_blob::<32>(&fields[0], "redacted team removal key")? != [0; 32] {
            return Err(Error::Type {
                expected: "redacted team removal key",
                found: "nonzero binary",
            });
        }
        Ok(Self {
            key,
            metadata: TeamRemovalKeyMetadata::from_value(&fields[1])?,
        })
    }

    pub fn into_key(self) -> SecretSeed {
        self.key
    }
}

impl TeamRemovalBoxData {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 4)?;
        Ok(Self {
            commitment: fixed_blob(&fields[0], "team removal-key commitment")?,
            team_box: TeamRemovalKeyBox::from_value(&fields[1])?,
            member_box: TeamRemovalKeyBox::from_value(&fields[2])?,
            metadata: TeamRemovalKeyMetadata::from_value(&fields[3])?,
        })
    }

    fn to_value(&self) -> Value {
        Value::Array(vec![
            Value::Binary(self.commitment.to_vec()),
            self.team_box.to_value(),
            self.member_box.to_value(),
            self.metadata.to_value(),
        ])
    }

    fn validate(&self) -> Result<()> {
        self.team_box.validate()?;
        self.member_box.validate()?;
        self.metadata.validate()
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        self.validate()?;
        Ok(encode(&self.to_value())?)
    }
}

impl UsernameReservation {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 3)?;
        Ok(Self {
            token: fixed_blob(&fields[0], "username reservation token")?,
            sequence: unsigned(&fields[1])?,
            expires_at: unsigned(&fields[2])?,
        })
    }

    pub fn to_value(&self) -> Value {
        Value::Array(vec![
            Value::Binary(self.token.to_vec()),
            Value::Unsigned(self.sequence),
            Value::Unsigned(self.expires_at),
        ])
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&self.to_value())?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InviteCode {
    None,
    Standard(Vec<u8>),
    MultiUse(Vec<u8>),
    Sso,
    Empty,
}

impl InviteCode {
    pub fn to_value(&self) -> Value {
        match self {
            Self::None => Value::Array(vec![Value::Unsigned(0), Value::Variant(None)]),
            Self::Standard(code) => Value::Array(vec![
                Value::Unsigned(1),
                Value::Variant(Some((b"1".to_vec(), Box::new(Value::Binary(code.clone()))))),
            ]),
            Self::MultiUse(code) => Value::Array(vec![
                Value::Unsigned(2),
                Value::Variant(Some((b"2".to_vec(), Box::new(Value::Text(code.clone()))))),
            ]),
            Self::Sso => Value::Array(vec![Value::Unsigned(3), Value::Variant(None)]),
            Self::Empty => Value::Array(vec![Value::Unsigned(4), Value::Variant(None)]),
        }
    }

    pub fn from_value(value: &Value) -> Result<Self> {
        let fields = array(value, 2)?;
        let discriminant = unsigned(&fields[0])?;
        match (discriminant, &fields[1]) {
            (0, Value::Variant(None)) => Ok(Self::None),
            (1, Value::Variant(Some((tag, value)))) if tag == b"1" => {
                let Value::Binary(code) = value.as_ref() else {
                    return Err(Error::Type {
                        expected: "standard invite code",
                        found: "another value",
                    });
                };
                Ok(Self::Standard(code.clone()))
            }
            (2, Value::Variant(Some((tag, value)))) if tag == b"2" => {
                let Value::Text(code) = value.as_ref() else {
                    return Err(Error::Type {
                        expected: "multi-use invite code",
                        found: "another value",
                    });
                };
                Ok(Self::MultiUse(code.clone()))
            }
            (3, Value::Variant(None)) => Ok(Self::Sso),
            (4, Value::Variant(None)) => Ok(Self::Empty),
            _ => Err(Error::UnknownEnum {
                kind: "invite code",
                value: discriminant,
            }),
        }
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        self.validate()?;
        Ok(encode(&self.to_value())?)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let code = Self::from_value(&decode(bytes)?)?;
        code.validate()?;
        Ok(code)
    }

    /// Parses the user-facing v0.1.9 representation. Standard one-use codes
    /// use the `s.` prefix and Keybase's base-62 copy/paste rules; other input
    /// is a case-insensitive multi-use code.
    pub fn from_user_input(input: &str, empty_allowed: bool) -> Result<Self> {
        if input.is_empty() {
            return if empty_allowed {
                Ok(Self::Empty)
            } else {
                Err(Error::Type {
                    expected: "invite code",
                    found: "empty text",
                })
            };
        }
        let code = if let Some(encoded) = input.strip_prefix("s.") {
            Self::Standard(decode_base62_invite(encoded)?)
        } else {
            Self::MultiUse(input.to_ascii_lowercase().into_bytes())
        };
        code.validate()?;
        Ok(code)
    }

    pub fn to_user_string(&self) -> Result<String> {
        self.validate()?;
        match self {
            Self::Standard(code) => Ok(format!("s.{}", encode_base62_strict(code))),
            Self::MultiUse(code) => String::from_utf8(code.clone()).map_err(|_| Error::Utf8),
            Self::Empty => Ok(String::new()),
            _ => Err(Error::Type {
                expected: "displayable invite code",
                found: "unsupported invite variant",
            }),
        }
    }

    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Standard(code) if code.len() >= 10 => Ok(()),
            Self::MultiUse(code)
                if code.len() >= 5
                    && code.iter().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'-')
                    }) =>
            {
                Ok(())
            }
            Self::Empty => Ok(()),
            Self::Standard(_) | Self::MultiUse(_) | Self::None | Self::Sso => Err(Error::Type {
                expected: "supported invite code",
                found: "invalid invite code",
            }),
        }
    }

    pub fn kind(&self) -> Option<u8> {
        match self {
            Self::Standard(_) => Some(1),
            Self::MultiUse(_) => Some(2),
            _ => None,
        }
    }
}

const BASE62_ALPHABET: &[u8; 62] =
    b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

fn encode_base62_strict(input: &[u8]) -> String {
    let mut encoded = Vec::with_capacity(base62_encoded_len(input.len()));
    for block in input.chunks(32) {
        let output_len = base62_encoded_len(block.len());
        let mut number = block.to_vec();
        let mut output = vec![b'0'; output_len];
        for position in (0..output_len).rev() {
            let mut remainder = 0u16;
            for byte in &mut number {
                let value = (remainder << 8) | u16::from(*byte);
                *byte = u8::try_from(value / 62).expect("base-62 quotient fits a byte");
                remainder = value % 62;
            }
            output[position] = BASE62_ALPHABET[usize::from(remainder)];
        }
        encoded.extend_from_slice(&output);
    }
    String::from_utf8(encoded).expect("base-62 alphabet is ASCII")
}

fn decode_base62_invite(input: &str) -> Result<Vec<u8>> {
    let mut cleaned = Vec::with_capacity(input.len());
    for byte in input.bytes() {
        if base62_digit(byte).is_some() {
            cleaned.push(byte);
        } else if !matches!(byte, b'\t' | b'\n' | b'\r' | b' ' | b'>') {
            return Err(Error::Type {
                expected: "base-62 invite code",
                found: "non-base-62 text",
            });
        }
    }
    if cleaned.is_empty() {
        return Err(Error::Length {
            kind: "base-62 invite code",
            expected: 14,
            found: cleaned.len(),
        });
    }
    let mut decoded = Vec::with_capacity(base62_decoded_len(cleaned.len()));
    for block in cleaned.chunks(43) {
        decoded.extend_from_slice(&decode_base62_block(block)?);
    }
    Ok(decoded)
}

fn decode_base62_block(bytes: &[u8]) -> Result<Vec<u8>> {
    let decoded_len = base62_decoded_len(bytes.len());
    let prior_len = base62_decoded_len(bytes.len().saturating_sub(1));
    if decoded_len == prior_len || decoded_len == 0 {
        return Err(Error::Length {
            kind: "base-62 invite code",
            expected: 14,
            found: bytes.len(),
        });
    }
    let mut number = vec![0u8];
    for byte in bytes {
        let digit = base62_digit(*byte).ok_or(Error::Type {
            expected: "strict base-62 invite code",
            found: "non-base-62 text",
        })? as u16;
        let mut carry = digit;
        for limb in number.iter_mut().rev() {
            let value = u16::from(*limb) * 62 + carry;
            *limb = value as u8;
            carry = value >> 8;
        }
        while carry != 0 {
            number.insert(0, carry as u8);
            carry >>= 8;
        }
    }
    if number.len() > decoded_len {
        return Err(Error::Type {
            expected: "canonical base-62 invite code",
            found: "overflowing base-62 text",
        });
    }
    let mut output = vec![0; decoded_len - number.len()];
    output.extend_from_slice(&number);
    Ok(output)
}

fn base62_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'A'..=b'Z' => Some(byte - b'A' + 10),
        b'a'..=b'z' => Some(byte - b'a' + 36),
        _ => None,
    }
}

fn base62_encoded_len(bytes: usize) -> usize {
    let blocks = bytes / 32;
    let remainder = bytes % 32;
    blocks * 43
        + if remainder == 0 {
            0
        } else {
            ((remainder as f64 * 8.0) / 62_f64.log2()).ceil() as usize
        }
}

fn base62_decoded_len(characters: usize) -> usize {
    let blocks = characters / 43;
    let remainder = characters % 43;
    blocks * 32 + ((remainder as f64 * 62_f64.log2()) / 8.0).floor() as usize
}

/// Owned, strictly decoded v0.1.9 software-signup request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedSignupArgument {
    pub username_utf8: Vec<u8>,
    pub reservation: UsernameReservation,
    pub link: UserLink,
    pub puk_box: SharedKeyBoxSet,
    pub username_commitment_key: [u8; 16],
    pub device_name: DeviceLabelNameAndCommitmentKey,
    pub next_tree_location: [u8; 32],
    pub invite_code: InviteCode,
    pub email: Vec<u8>,
    pub subchain_tree_location: [u8; 32],
    pub self_token: [u8; 17],
    pub puk_hepk: Hepk,
    pub device_hepk: Hepk,
    pub passphrase: Option<crate::PassphraseUpdateArgument>,
    pub subkey_box: Option<HybridBox>,
    pub yubi_pq_hint: Option<crate::YubiSlotAndPqKeyId>,
}

impl DecodedSignupArgument {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 16)?;
        let host_policy = array(&fields[15], 2)?;
        expect_unsigned(&host_policy[0], "signup host policy", 0)?;
        if !matches!(host_policy[1], Value::Variant(None)) {
            return Err(Error::Type {
                expected: "empty signup host policy",
                found: "another value",
            });
        }
        let hepk_outer = array(&fields[13], 1)?;
        let hepks = array(&hepk_outer[0], 2)?;
        let invite_code = InviteCode::from_value(&fields[7])?;
        invite_code.validate()?;
        Ok(Self {
            username_utf8: text(&fields[0])?.into_bytes(),
            reservation: UsernameReservation::decode(&encode(&fields[1])?)?,
            link: UserLink::decode(&encode(&fields[2])?)?,
            puk_box: SharedKeyBoxSet::decode(&encode(&fields[3])?)?,
            username_commitment_key: fixed_blob(&fields[4], "username commitment key")?,
            device_name: device_label_name_and_commitment_key(&fields[5])?,
            next_tree_location: fixed_blob(&fields[6], "next tree location")?,
            invite_code,
            email: text(&fields[8])?.into_bytes(),
            subchain_tree_location: fixed_blob(&fields[11], "subchain tree location")?,
            self_token: fixed_blob(&fields[12], "signup self token")?,
            puk_hepk: hepk(&hepks[0])?,
            device_hepk: hepk(&hepks[1])?,
            passphrase: match &fields[10] {
                Value::Null => None,
                value => Some(crate::PassphraseUpdateArgument::from_set_value(value)?),
            },
            subkey_box: match &fields[9] {
                Value::Null => None,
                value => Some(HybridBox::decode(&encode(value)?)?),
            },
            yubi_pq_hint: match &fields[14] {
                Value::Null => None,
                value => Some(crate::YubiSlotAndPqKeyId::from_value(value)?),
            },
        })
    }
}

fn require_null(value: &Value, kind: &'static str) -> Result<()> {
    if matches!(value, Value::Null) {
        Ok(())
    } else {
        Err(Error::Type {
            expected: kind,
            found: "another value",
        })
    }
}

/// Narrow v0.1.9 signup argument for a software eldest credential. Optional
/// Yubi and subkey fields are fixed to absent. The
/// optional passphrase field carries an exact v0.1.9 set-passphrase request.
pub struct SoftwareSignupArgument<'a> {
    pub username_utf8: &'a [u8],
    pub reservation: &'a UsernameReservation,
    pub link: &'a UserLink,
    pub puk_box: &'a SharedKeyBoxSet,
    pub username_commitment_key: [u8; 16],
    pub device_name: &'a DeviceLabelNameAndCommitmentKey,
    pub next_tree_location: [u8; 32],
    pub invite_code: &'a InviteCode,
    pub email: &'a [u8],
    pub subchain_tree_location: [u8; 32],
    pub self_token: [u8; 17],
    pub puk_hepk: &'a Hepk,
    pub device_hepk: &'a Hepk,
    pub passphrase: Option<&'a crate::PassphraseUpdateArgument>,
}

/// Exact v0.1.9 signup argument for a Yubi parent with an encrypted delegated
/// subkey and its second-slot PQ derivation hint.
pub struct YubiSignupArgument<'a> {
    pub username_utf8: &'a [u8],
    pub reservation: &'a UsernameReservation,
    pub link: &'a UserLink,
    pub puk_box: &'a SharedKeyBoxSet,
    pub username_commitment_key: [u8; 16],
    pub device_name: &'a DeviceLabelNameAndCommitmentKey,
    pub next_tree_location: [u8; 32],
    pub invite_code: &'a InviteCode,
    pub email: &'a [u8],
    pub subkey_box: &'a HybridBox,
    pub passphrase: Option<&'a crate::PassphraseUpdateArgument>,
    pub subchain_tree_location: [u8; 32],
    pub self_token: [u8; 17],
    pub puk_hepk: &'a Hepk,
    pub device_hepk: &'a Hepk,
    pub yubi_pq_hint: &'a crate::YubiSlotAndPqKeyId,
}

impl YubiSignupArgument<'_> {
    pub fn encoded(&self) -> Result<Vec<u8>> {
        signup_value(
            self.username_utf8,
            self.reservation,
            self.link,
            self.puk_box,
            self.username_commitment_key,
            self.device_name,
            self.next_tree_location,
            self.invite_code,
            self.email,
            Some(self.subkey_box),
            self.passphrase,
            self.subchain_tree_location,
            self.self_token,
            self.puk_hepk,
            self.device_hepk,
            Some(self.yubi_pq_hint),
        )
    }
}

impl SoftwareSignupArgument<'_> {
    pub fn encoded(&self) -> Result<Vec<u8>> {
        signup_value(
            self.username_utf8,
            self.reservation,
            self.link,
            self.puk_box,
            self.username_commitment_key,
            self.device_name,
            self.next_tree_location,
            self.invite_code,
            self.email,
            None,
            self.passphrase,
            self.subchain_tree_location,
            self.self_token,
            self.puk_hepk,
            self.device_hepk,
            None,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn signup_value(
    username_utf8: &[u8],
    reservation: &UsernameReservation,
    link: &UserLink,
    puk_box: &SharedKeyBoxSet,
    username_commitment_key: [u8; 16],
    device_name: &DeviceLabelNameAndCommitmentKey,
    next_tree_location: [u8; 32],
    invite_code: &InviteCode,
    email: &[u8],
    subkey_box: Option<&HybridBox>,
    passphrase: Option<&crate::PassphraseUpdateArgument>,
    subchain_tree_location: [u8; 32],
    self_token: [u8; 17],
    puk_hepk: &Hepk,
    device_hepk: &Hepk,
    yubi_pq_hint: Option<&crate::YubiSlotAndPqKeyId>,
) -> Result<Vec<u8>> {
    let link = decode(&link.encoded()?)?;
    let puk_box = decode(&puk_box.encoded())?;
    let puk_hepk = decode(&puk_hepk.encoded()?)?;
    let device_hepk = decode(&device_hepk.encoded()?)?;
    let label = &device_name.label;
    let fields = vec![
        Value::Text(username_utf8.to_vec()),
        reservation.to_value(),
        link,
        puk_box,
        Value::Binary(username_commitment_key.to_vec()),
        Value::Array(vec![
            Value::Array(vec![
                Value::Array(vec![
                    Value::Unsigned(label.device_type.protocol_value()),
                    Value::Text(label.normalized_name.clone()),
                    Value::Unsigned(label.serial),
                ]),
                Value::Unsigned(device_name.normalization_version),
                Value::Text(device_name.display_name.clone()),
            ]),
            Value::Binary(device_name.commitment_key.to_vec()),
        ]),
        Value::Binary(next_tree_location.to_vec()),
        invite_code.to_value(),
        Value::Text(email.to_vec()),
        subkey_box.map_or(Value::Null, HybridBox::to_value),
        passphrase.map_or(Value::Null, crate::PassphraseUpdateArgument::to_set_value),
        Value::Binary(subchain_tree_location.to_vec()),
        Value::Binary(self_token.to_vec()),
        Value::Array(vec![Value::Array(vec![puk_hepk, device_hepk])]),
        yubi_pq_hint.map_or(Value::Null, crate::YubiSlotAndPqKeyId::to_value),
        Value::Array(vec![Value::Unsigned(0), Value::Variant(None)]),
    ];
    Ok(encode(&Value::Array(fields))?)
}

pub struct ProvisionDeviceArgument<'a> {
    pub link: &'a UserLink,
    pub puk_boxes: &'a SharedKeyBoxSet,
    pub device_name: &'a DeviceLabelNameAndCommitmentKey,
    pub next_tree_location: [u8; 32],
    pub self_token: [u8; 17],
    pub hepks: &'a [Hepk],
    pub subkey_box: Option<&'a HybridBox>,
    pub yubi_pq_hint: Option<&'a crate::YubiSlotAndPqKeyId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedProvisionDeviceArgument {
    pub link: UserLink,
    pub puk_boxes: SharedKeyBoxSet,
    pub device_name: DeviceLabelNameAndCommitmentKey,
    pub next_tree_location: [u8; 32],
    pub self_token: [u8; 17],
    pub hepks: Vec<Hepk>,
    pub subkey_box: Option<HybridBox>,
    pub yubi_pq_hint: Option<crate::YubiSlotAndPqKeyId>,
}

impl DecodedProvisionDeviceArgument {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 15)?;
        for field in &fields[7..14] {
            require_null(field, "unsupported provisioning field")?;
        }
        Ok(Self {
            link: UserLink::decode(&encode(&fields[0])?)?,
            puk_boxes: SharedKeyBoxSet::decode(&encode(&fields[1])?)?,
            device_name: DeviceLabelNameAndCommitmentKey::decode(&encode(&fields[2])?)?,
            next_tree_location: fixed_blob(&fields[3], "next tree location")?,
            self_token: fixed_blob(&fields[5], "provisioning self token")?,
            hepks: decode_hepk_set(&fields[6])?,
            subkey_box: match &fields[4] {
                Value::Null => None,
                value => Some(HybridBox::decode(&encode(value)?)?),
            },
            yubi_pq_hint: match &fields[14] {
                Value::Null => None,
                value => Some(crate::YubiSlotAndPqKeyId::from_value(value)?),
            },
        })
    }
}

pub struct AdHocTeamCreateArgument<'a> {
    pub link: &'a UserLink,
    pub next_tree_location: [u8; 32],
    pub ptk_boxes: &'a SharedKeyBoxSet,
    pub hepks: &'a [Hepk],
    pub subchain_tree_location: [u8; 32],
    pub membership_link: &'a UserLink,
    pub membership_next_tree_location: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedAdHocTeamCreateArgument {
    pub link: UserLink,
    pub next_tree_location: [u8; 32],
    pub ptk_boxes: SharedKeyBoxSet,
    pub hepks: Vec<Hepk>,
    pub subchain_tree_location: [u8; 32],
    pub membership_link: UserLink,
    pub membership_next_tree_location: [u8; 32],
}

impl DecodedAdHocTeamCreateArgument {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let outer = array(&value, 1)?;
        let common = array(&outer[0], 3)?;
        let edit = decode_team_edit_common(&common[1])?;
        if !edit.seed_chain.is_empty()
            || !edit.removal_keys.is_empty()
            || !edit.removals.is_empty()
            || !edit.remote_member_view_tokens.is_empty()
            || !edit.local_permissions_for.is_empty()
        {
            return Err(Error::IntegerRange("ad-hoc team founding edit"));
        }
        let membership = array(&common[2], 2)?;
        Ok(Self {
            link: edit.link,
            next_tree_location: edit.next_tree_location,
            ptk_boxes: edit.ptk_boxes,
            hepks: edit.hepks,
            subchain_tree_location: fixed_blob(&common[0], "team subchain tree location")?,
            membership_link: UserLink::decode(&encode(&membership[0])?)?,
            membership_next_tree_location: fixed_blob(
                &membership[1],
                "membership next tree location",
            )?,
        })
    }
}

pub struct NamedTeamCreateArgument<'a> {
    pub name_utf8: &'a [u8],
    pub team_name_commitment_key: [u8; 16],
    pub subchain_tree_location: [u8; 32],
    pub reservation: &'a TeamNameReservation,
    pub link: &'a UserLink,
    pub next_tree_location: [u8; 32],
    pub ptk_boxes: &'a SharedKeyBoxSet,
    pub removal_keys: &'a [TeamRemovalBoxData],
    pub hepks: &'a [Hepk],
    pub membership_link: &'a UserLink,
    pub membership_next_tree_location: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedNamedTeamCreateArgument {
    pub name_utf8: Vec<u8>,
    pub team_name_commitment_key: [u8; 16],
    pub subchain_tree_location: [u8; 32],
    pub reservation: TeamNameReservation,
    pub edit: DecodedTeamEditArgument,
    pub membership_link: UserLink,
    pub membership_next_tree_location: [u8; 32],
}

impl DecodedNamedTeamCreateArgument {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 6)?;
        let membership = array(&fields[5], 2)?;
        let edit = decode_team_edit_common(&fields[4])?;
        if !edit.seed_chain.is_empty()
            || !edit.removals.is_empty()
            || !edit.remote_member_view_tokens.is_empty()
            || !edit.local_permissions_for.is_empty()
        {
            return Err(Error::IntegerRange("named-team founding edit"));
        }
        Ok(Self {
            name_utf8: text(&fields[0])?.into_bytes(),
            team_name_commitment_key: fixed_blob(&fields[1], "team name commitment key")?,
            subchain_tree_location: fixed_blob(&fields[2], "team subchain tree location")?,
            reservation: TeamNameReservation::decode(&encode(&fields[3])?)?,
            edit,
            membership_link: UserLink::decode(&encode(&membership[0])?)?,
            membership_next_tree_location: fixed_blob(
                &membership[1],
                "membership next tree location",
            )?,
        })
    }
}

pub struct AddTeamMemberArgument<'a> {
    pub link: &'a UserLink,
    pub next_tree_location: [u8; 32],
    pub ptk_boxes: &'a SharedKeyBoxSet,
    pub removal_keys: &'a [TeamRemovalBoxData],
    pub hepks: &'a [Hepk],
    pub remote_member_view_tokens: &'a [TeamRemoteMemberViewToken],
    pub local_permissions_for: &'a [EntityId],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedTeamEditArgument {
    pub link: UserLink,
    pub next_tree_location: [u8; 32],
    pub ptk_boxes: SharedKeyBoxSet,
    pub seed_chain: Vec<SeedChainBox>,
    pub removal_keys: Vec<TeamRemovalBoxData>,
    pub removals: Vec<TeamRemovalAndCommitment>,
    pub hepks: Vec<Hepk>,
    pub remote_member_view_tokens: Vec<TeamRemoteMemberViewToken>,
    pub local_permissions_for: Vec<EntityId>,
}

impl DecodedTeamEditArgument {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        decode_team_edit_common(&decode(bytes)?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamRemovalMacPayload {
    pub team: EntityId,
    pub host: EntityId,
    pub member: EntityId,
    pub member_host: EntityId,
    pub source_role: Role,
    pub admin: EntityId,
    pub admin_host: EntityId,
    pub root: crate::TreeRoot,
    pub time: u64,
}

impl TeamRemovalMacPayload {
    fn from_value(value: &Value) -> Result<Self> {
        let fields = array(value, 6)?;
        let team = array(&fields[0], 2)?;
        let member = array(&fields[1], 2)?;
        let admin = array(&fields[3], 2)?;
        let root = array(&fields[4], 2)?;
        let payload = Self {
            team: entity(&team[0])?,
            host: entity(&team[1])?,
            member: entity(&member[0])?,
            member_host: entity(&member[1])?,
            source_role: role(&fields[2])?,
            admin: entity(&admin[0])?,
            admin_host: entity(&admin[1])?,
            root: crate::TreeRoot {
                epoch: unsigned(&root[0])?,
                hash: fixed_blob(&root[1], "team removal Merkle root")?,
            },
            time: unsigned(&fields[5])?,
        };
        payload.validate()?;
        Ok(payload)
    }

    fn validate(&self) -> Result<()> {
        self.team.clone().require_type(ENTITY_NAMED_TEAM)?;
        self.host.clone().require_type(ENTITY_HOST)?;
        self.member_host.clone().require_type(ENTITY_HOST)?;
        self.admin.clone().require_type(ENTITY_USER)?;
        self.admin_host.clone().require_type(ENTITY_HOST)?;
        if !matches!(
            self.member.entity_type(),
            ENTITY_USER | ENTITY_NAMED_TEAM | ENTITY_AD_HOC_TEAM
        ) || self.source_role == Role::NONE
            || self.root.epoch == 0
            || self.time == 0
        {
            return Err(Error::IntegerRange("team removal MAC payload"));
        }
        Ok(())
    }

    pub(crate) fn to_value(&self) -> Value {
        Value::Array(vec![
            Value::Array(vec![
                Value::Binary(self.team.as_bytes().to_vec()),
                Value::Binary(self.host.as_bytes().to_vec()),
            ]),
            Value::Array(vec![
                Value::Binary(self.member.as_bytes().to_vec()),
                Value::Binary(self.member_host.as_bytes().to_vec()),
            ]),
            self.source_role.to_value(),
            Value::Array(vec![
                Value::Binary(self.admin.as_bytes().to_vec()),
                Value::Binary(self.admin_host.as_bytes().to_vec()),
            ]),
            Value::Array(vec![
                Value::Unsigned(self.root.epoch),
                Value::Binary(self.root.hash.to_vec()),
            ]),
            Value::Unsigned(self.time),
        ])
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        self.validate()?;
        Ok(encode(&self.to_value())?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamRemovalProof {
    pub mac: [u8; 32],
    pub payload: TeamRemovalMacPayload,
}

impl TeamRemovalProof {
    fn to_value(&self) -> Value {
        Value::Array(vec![
            Value::Binary(self.mac.to_vec()),
            self.payload.to_value(),
        ])
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamRemovalAndCommitment {
    pub removal: TeamRemovalProof,
    pub commitment: [u8; 32],
}

impl TeamRemovalAndCommitment {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 2)?;
        let removal = array(&fields[0], 2)?;
        Ok(Self {
            removal: TeamRemovalProof {
                mac: fixed_blob(&removal[0], "team removal MAC")?,
                payload: TeamRemovalMacPayload::from_value(&removal[1])?,
            },
            commitment: fixed_blob(&fields[1], "team removal-key commitment")?,
        })
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        self.removal.payload.validate()?;
        Ok(encode(&self.to_value())?)
    }

    fn to_value(&self) -> Value {
        Value::Array(vec![
            self.removal.to_value(),
            Value::Binary(self.commitment.to_vec()),
        ])
    }
}

/// Exact v0.1.9 TeamAdmin edit argument for a removal with PTK rotation.
pub struct RemoveTeamMemberArgument<'a> {
    pub link: &'a UserLink,
    pub next_tree_location: [u8; 32],
    pub ptk_boxes: &'a SharedKeyBoxSet,
    pub seed_chain: &'a [SeedChainBox],
    pub removals: &'a [TeamRemovalAndCommitment],
    pub hepks: &'a [Hepk],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamEditResult {
    pub local_invitees: Vec<(EntityId, Role)>,
}

impl TeamEditResult {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 1)?;
        let invitees = crate::list(&fields[0], |value| {
            let fields = array(value, 2)?;
            Ok((entity(&fields[0])?, role(&fields[1])?))
        })?;
        Ok(Self {
            local_invitees: invitees,
        })
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![list_or_null(
            self.local_invitees.iter().map(|(entity, role)| {
                Value::Array(vec![
                    Value::Binary(entity.as_bytes().to_vec()),
                    role.to_value(),
                ])
            }),
        )]))?)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ViewershipMode {
    Closed = 0,
    OpenToAdmin = 1,
    Open = 2,
}

impl ViewershipMode {
    fn decode(value: &Value) -> Result<Self> {
        match unsigned(value)? {
            0 => Ok(Self::Closed),
            1 => Ok(Self::OpenToAdmin),
            2 => Ok(Self::Open),
            _ => Err(Error::IntegerRange("viewership mode")),
        }
    }

    pub fn to_value(self) -> Value {
        Value::Unsigned(self as u64)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostConfig {
    pub meter_users: bool,
    pub meter_vhosts: bool,
    pub meter_per_vhost_disk: bool,
    pub user_viewership: ViewershipMode,
    pub team_viewership: ViewershipMode,
    pub host_type: u64,
    pub invite_code_regime: u64,
}

impl HostConfig {
    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![
            Value::Array(vec![
                Value::Bool(self.meter_users),
                Value::Bool(self.meter_vhosts),
                Value::Bool(self.meter_per_vhost_disk),
            ]),
            Value::Array(vec![
                self.user_viewership.to_value(),
                self.team_viewership.to_value(),
            ]),
            Value::Unsigned(self.host_type),
            Value::Unsigned(self.invite_code_regime),
        ]))?)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let wire = decode(bytes)?;
        let fields = array(&wire, 4)?;
        let metering = array(&fields[0], 3)?;
        let viewership = array(&fields[1], 2)?;
        let host_type = unsigned(&fields[2])?;
        let invite_code_regime = unsigned(&fields[3])?;
        if host_type > 4 || invite_code_regime > 3 {
            return Err(Error::IntegerRange("host configuration enum"));
        }
        Ok(Self {
            meter_users: boolean(&metering[0])?,
            meter_vhosts: boolean(&metering[1])?,
            meter_per_vhost_disk: boolean(&metering[2])?,
            user_viewership: ViewershipMode::decode(&viewership[0])?,
            team_viewership: ViewershipMode::decode(&viewership[1])?,
            host_type,
            invite_code_regime,
        })
    }
}

impl AdHocTeamCreateArgument<'_> {
    pub fn encoded(&self) -> Result<Vec<u8>> {
        let hepks = self
            .hepks
            .iter()
            .map(|hepk| Ok(decode(&hepk.encoded()?)?))
            .collect::<Result<Vec<_>>>()?;
        let offchain = Value::Array(vec![
            decode(&self.ptk_boxes.encoded())?,
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Array(vec![Value::Array(hepks)]),
            Value::Null,
        ]);
        let edit = Value::Array(vec![
            decode(&self.link.encoded()?)?,
            Value::Binary(self.next_tree_location.to_vec()),
            offchain,
            Value::Null,
            Value::Null,
        ]);
        let membership = Value::Array(vec![
            decode(&self.membership_link.encoded()?)?,
            Value::Binary(self.membership_next_tree_location.to_vec()),
        ]);
        let common = Value::Array(vec![
            Value::Binary(self.subchain_tree_location.to_vec()),
            edit,
            membership,
        ]);
        Ok(encode(&Value::Array(vec![common]))?)
    }
}

impl NamedTeamCreateArgument<'_> {
    pub fn encoded(&self) -> Result<Vec<u8>> {
        if self.name_utf8.is_empty()
            || self.reservation.sequence == 0
            || self.removal_keys.is_empty()
        {
            return Err(Error::FieldCount {
                expected: 1,
                found: 0,
            });
        }
        for removal_key in self.removal_keys {
            removal_key.validate()?;
        }
        let hepks = self
            .hepks
            .iter()
            .map(|hepk| Ok(decode(&hepk.encoded()?)?))
            .collect::<Result<Vec<_>>>()?;
        let offchain = Value::Array(vec![
            decode(&self.ptk_boxes.encoded())?,
            Value::Null,
            Value::Null,
            list_or_null(self.removal_keys.iter().map(TeamRemovalBoxData::to_value)),
            Value::Null,
            Value::Array(vec![Value::Array(hepks)]),
            Value::Null,
        ]);
        let edit = Value::Array(vec![
            decode(&self.link.encoded()?)?,
            Value::Binary(self.next_tree_location.to_vec()),
            offchain,
            Value::Null,
            Value::Null,
        ]);
        let membership = Value::Array(vec![
            decode(&self.membership_link.encoded()?)?,
            Value::Binary(self.membership_next_tree_location.to_vec()),
        ]);
        Ok(encode(&Value::Array(vec![
            Value::Text(self.name_utf8.to_vec()),
            Value::Binary(self.team_name_commitment_key.to_vec()),
            Value::Binary(self.subchain_tree_location.to_vec()),
            self.reservation.to_value(),
            edit,
            membership,
        ]))?)
    }
}

impl AddTeamMemberArgument<'_> {
    pub fn encoded(&self) -> Result<Vec<u8>> {
        for found in [self.removal_keys.len(), self.hepks.len()] {
            if found != 1 {
                return Err(Error::FieldCount { expected: 1, found });
            }
        }
        if self.remote_member_view_tokens.len() + self.local_permissions_for.len() != 1 {
            return Err(Error::FieldCount {
                expected: 1,
                found: self.remote_member_view_tokens.len() + self.local_permissions_for.len(),
            });
        }
        for removal_key in self.removal_keys {
            removal_key.validate()?;
        }
        let hepks = self
            .hepks
            .iter()
            .map(|hepk| Ok(decode(&hepk.encoded()?)?))
            .collect::<Result<Vec<_>>>()?;
        let offchain = Value::Array(vec![
            decode(&self.ptk_boxes.encoded())?,
            Value::Null,
            list_or_null(
                self.remote_member_view_tokens
                    .iter()
                    .map(TeamRemoteMemberViewToken::to_value),
            ),
            list_or_null(self.removal_keys.iter().map(TeamRemovalBoxData::to_value)),
            Value::Null,
            Value::Array(vec![Value::Array(hepks)]),
            Value::Null,
        ]);
        Ok(encode(&Value::Array(vec![
            decode(&self.link.encoded()?)?,
            Value::Binary(self.next_tree_location.to_vec()),
            offchain,
            Value::Null,
            list_or_null(
                self.local_permissions_for
                    .iter()
                    .map(|party| Value::Binary(party.as_bytes().to_vec())),
            ),
        ]))?)
    }
}

impl RemoveTeamMemberArgument<'_> {
    pub fn encoded(&self) -> Result<Vec<u8>> {
        if self.seed_chain.is_empty() || self.removals.len() != 1 || self.hepks.is_empty() {
            return Err(Error::IntegerRange("team removal edit contents"));
        }
        for removal in self.removals {
            removal.removal.payload.validate()?;
        }
        let hepks = self
            .hepks
            .iter()
            .map(|hepk| Ok(decode(&hepk.encoded()?)?))
            .collect::<Result<Vec<_>>>()?;
        let offchain = Value::Array(vec![
            decode(&self.ptk_boxes.encoded())?,
            list_or_null(self.seed_chain.iter().map(|boxed| {
                Value::Array(vec![
                    Value::Unsigned(boxed.generation),
                    boxed.role.to_value(),
                    boxed.secret_box.to_value(),
                ])
            })),
            Value::Null,
            Value::Null,
            list_or_null(self.removals.iter().map(TeamRemovalAndCommitment::to_value)),
            Value::Array(vec![Value::Array(hepks)]),
            Value::Null,
        ]);
        Ok(encode(&Value::Array(vec![
            decode(&self.link.encoded()?)?,
            Value::Binary(self.next_tree_location.to_vec()),
            offchain,
            Value::Null,
            Value::Null,
        ]))?)
    }
}

impl ProvisionDeviceArgument<'_> {
    pub fn encoded(&self) -> Result<Vec<u8>> {
        let label = &self.device_name.label;
        let hepks = self
            .hepks
            .iter()
            .map(|hepk| Ok(decode(&hepk.encoded()?)?))
            .collect::<Result<Vec<_>>>()?;
        Ok(encode(&Value::Array(vec![
            decode(&self.link.encoded()?)?,
            decode(&self.puk_boxes.encoded())?,
            Value::Array(vec![
                Value::Array(vec![
                    Value::Array(vec![
                        Value::Unsigned(label.device_type.protocol_value()),
                        Value::Text(label.normalized_name.clone()),
                        Value::Unsigned(label.serial),
                    ]),
                    Value::Unsigned(self.device_name.normalization_version),
                    Value::Text(self.device_name.display_name.clone()),
                ]),
                Value::Binary(self.device_name.commitment_key.to_vec()),
            ]),
            Value::Binary(self.next_tree_location.to_vec()),
            self.subkey_box.map_or(Value::Null, HybridBox::to_value),
            Value::Binary(self.self_token.to_vec()),
            Value::Array(vec![Value::Array(hepks)]),
            // Positions 7 through 13 are retained by the v0.1.9 wire
            // format even though their fields have been deprecated.
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            // Position 14 is the optional YubiKey PQ hint.
            self.yubi_pq_hint
                .map_or(Value::Null, crate::YubiSlotAndPqKeyId::to_value),
        ]))?)
    }
}

pub struct RevokeDeviceArgument<'a> {
    pub link: &'a UserLink,
    pub puk_boxes: &'a SharedKeyBoxSet,
    pub seed_chain: &'a [SeedChainBox],
    pub next_tree_location: [u8; 32],
    pub hepks: &'a [Hepk],
    pub passphrase: Option<&'a crate::PassphraseUpdateArgument>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedRevokeDeviceArgument {
    pub link: UserLink,
    pub puk_boxes: SharedKeyBoxSet,
    pub seed_chain: Vec<SeedChainBox>,
    pub next_tree_location: [u8; 32],
    pub hepks: Vec<Hepk>,
    pub passphrase: Option<crate::PassphraseUpdateArgument>,
}

impl DecodedRevokeDeviceArgument {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 6)?;
        let passphrase = match &fields[4] {
            Value::Null => None,
            value => {
                let annex = array(value, 2)?;
                // Go clients repeat their generic user-settings link here.
                // This protocol slice accepts that canonical value but does
                // not project the unsupported chain into local state.
                Some(crate::PassphraseUpdateArgument::from_unbound_change_value(
                    &annex[0],
                )?)
            }
        };
        Ok(Self {
            link: UserLink::decode(&encode(&fields[0])?)?,
            puk_boxes: SharedKeyBoxSet::decode(&encode(&fields[1])?)?,
            seed_chain: list(&fields[2], |value| SeedChainBox::decode(&encode(value)?))?,
            next_tree_location: fixed_blob(&fields[3], "next tree location")?,
            hepks: decode_hepk_set(&fields[5])?,
            passphrase,
        })
    }
}

impl RevokeDeviceArgument<'_> {
    pub fn encoded(&self) -> Result<Vec<u8>> {
        let hepks = self
            .hepks
            .iter()
            .map(|hepk| Ok(decode(&hepk.encoded()?)?))
            .collect::<Result<Vec<_>>>()?;
        Ok(encode(&Value::Array(vec![
            decode(&self.link.encoded()?)?,
            decode(&self.puk_boxes.encoded())?,
            list_or_null(self.seed_chain.iter().map(|boxed| {
                Value::Array(vec![
                    Value::Unsigned(boxed.generation),
                    boxed.role.to_value(),
                    boxed.secret_box.to_value(),
                ])
            })),
            Value::Binary(self.next_tree_location.to_vec()),
            self.passphrase.map_or(Value::Null, |passphrase| {
                Value::Array(vec![passphrase.to_change_value(), Value::Null])
            }),
            Value::Array(vec![Value::Array(hepks)]),
        ]))?)
    }
}

fn decode_hepk_set(value: &Value) -> Result<Vec<Hepk>> {
    let outer = array(value, 1)?;
    let values = match &outer[0] {
        Value::Null => &[][..],
        Value::Array(values) => values.as_slice(),
        _ => {
            return Err(Error::Type {
                expected: "HEPK list",
                found: "another value",
            });
        }
    };
    values
        .iter()
        .map(|value| Hepk::decode(&encode(value)?))
        .collect()
}

fn decode_team_edit_common(value: &Value) -> Result<DecodedTeamEditArgument> {
    let fields = array(value, 5)?;
    require_null(&fields[3], "team edit invite links")?;
    let offchain = array(&fields[2], 7)?;
    require_null(&offchain[6], "team edit remote join requests")?;
    Ok(DecodedTeamEditArgument {
        link: UserLink::decode(&encode(&fields[0])?)?,
        next_tree_location: fixed_blob(&fields[1], "team next tree location")?,
        ptk_boxes: SharedKeyBoxSet::decode(&encode(&offchain[0])?)?,
        seed_chain: list(&offchain[1], |value| SeedChainBox::decode(&encode(value)?))?,
        removal_keys: list(&offchain[3], |value| {
            TeamRemovalBoxData::decode(&encode(value)?)
        })?,
        removals: list(&offchain[4], |value| {
            TeamRemovalAndCommitment::decode(&encode(value)?)
        })?,
        hepks: decode_hepk_set(&offchain[5])?,
        remote_member_view_tokens: list(&offchain[2], |value| {
            TeamRemoteMemberViewToken::decode(&encode(value)?)
        })?,
        local_permissions_for: list(&fields[4], entity)?,
    })
}

#[cfg(test)]
mod invite_tests {
    use super::*;

    #[test]
    fn standard_invite_text_matches_the_v019_base62_encoding() {
        let first = InviteCode::Standard((0u8..10).collect());
        assert_eq!(first.to_user_string().unwrap(), "s.000M9ODf6X0ft3");
        assert_eq!(
            InviteCode::from_user_input("s.000M9ODf6X0ft3", false).unwrap(),
            first
        );

        let second = InviteCode::Standard(vec![
            0xff, 0xee, 0xdd, 0xcc, 0xbb, 0xaa, 0x99, 0x88, 0x77, 0x66,
        ]);
        assert_eq!(second.to_user_string().unwrap(), "s.62cAFRrdMSO7mg");
    }

    #[test]
    fn standard_invite_text_uses_v019_blocks_and_copy_whitespace_rules() {
        let code = InviteCode::Standard(vec![0; 33]);
        let exported = code.to_user_string().unwrap();
        assert_eq!(exported, format!("s.{}", "0".repeat(45)));
        assert_eq!(InviteCode::from_user_input(&exported, false).unwrap(), code);
        assert_eq!(
            InviteCode::from_user_input("s.000M9O >Df6X0ft3\n", false).unwrap(),
            InviteCode::Standard((0u8..10).collect())
        );
        assert!(InviteCode::from_user_input("s.000M9O/Df6X0ft3", false).is_err());
    }

    #[test]
    fn multiuse_invites_are_normalized_and_validated_like_v019() {
        assert_eq!(
            InviteCode::from_user_input("Team+A", false).unwrap(),
            InviteCode::MultiUse(b"team+a".to_vec())
        );
        assert!(InviteCode::from_user_input("abcd", false).is_err());
        assert!(InviteCode::from_user_input("team code", false).is_err());
        assert_eq!(
            InviteCode::from_user_input("", true).unwrap(),
            InviteCode::Empty
        );
        assert!(InviteCode::from_user_input("", false).is_err());
    }
}
