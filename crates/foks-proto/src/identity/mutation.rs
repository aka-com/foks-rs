//! Registration, device mutation, host-policy, and ad-hoc team arguments.

use super::{
    device_label_name_and_commitment_key, hepk, DeviceLabelNameAndCommitmentKey, Hepk, UserLink,
};
use crate::{
    array, boolean, decode, encode, entity, expect_unsigned, fixed_blob, list_or_null, role, text,
    unsigned, EntityId, Error, Result, Role, SecretSeed, SeedChainBox, SharedKeyBoxSet,
    TeamRemovalKeyBox, Value, ENTITY_AD_HOC_TEAM, ENTITY_HOST, ENTITY_NAMED_TEAM, ENTITY_USER,
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
    fn to_value(&self) -> Value {
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

    fn from_value(value: &Value) -> Result<Self> {
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
}

/// Owned, strictly decoded v0.1.9 software-signup request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedSoftwareSignupArgument {
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
}

impl DecodedSoftwareSignupArgument {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 16)?;
        require_null(&fields[9], "signup Yubi registration")?;
        require_null(&fields[10], "signup passphrase stretch")?;
        require_null(&fields[14], "signup subkey")?;
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
        Ok(Self {
            username_utf8: text(&fields[0])?.into_bytes(),
            reservation: UsernameReservation::decode(&encode(&fields[1])?)?,
            link: UserLink::decode(&encode(&fields[2])?)?,
            puk_box: SharedKeyBoxSet::decode(&encode(&fields[3])?)?,
            username_commitment_key: fixed_blob(&fields[4], "username commitment key")?,
            device_name: device_label_name_and_commitment_key(&fields[5])?,
            next_tree_location: fixed_blob(&fields[6], "next tree location")?,
            invite_code: InviteCode::from_value(&fields[7])?,
            email: text(&fields[8])?.into_bytes(),
            subchain_tree_location: fixed_blob(&fields[11], "subchain tree location")?,
            self_token: fixed_blob(&fields[12], "signup self token")?,
            puk_hepk: hepk(&hepks[0])?,
            device_hepk: hepk(&hepks[1])?,
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
/// Yubi, passphrase, subkey, and SSO fields are intentionally fixed to absent.
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
}

impl SoftwareSignupArgument<'_> {
    pub fn encoded(&self) -> Result<Vec<u8>> {
        let link = decode(&self.link.encoded()?)?;
        let puk_box = decode(&self.puk_box.encoded())?;
        let puk_hepk = decode(&self.puk_hepk.encoded()?)?;
        let device_hepk = decode(&self.device_hepk.encoded()?)?;
        let label = &self.device_name.label;
        let fields = vec![
            Value::Text(self.username_utf8.to_vec()),
            self.reservation.to_value(),
            link,
            puk_box,
            Value::Binary(self.username_commitment_key.to_vec()),
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
            self.invite_code.to_value(),
            Value::Text(self.email.to_vec()),
            Value::Null,
            Value::Null,
            Value::Binary(self.subchain_tree_location.to_vec()),
            Value::Binary(self.self_token.to_vec()),
            Value::Array(vec![Value::Array(vec![puk_hepk, device_hepk])]),
            Value::Null,
            Value::Array(vec![Value::Unsigned(0), Value::Variant(None)]),
        ];
        Ok(encode(&Value::Array(fields))?)
    }
}

pub struct ProvisionDeviceArgument<'a> {
    pub link: &'a UserLink,
    pub puk_boxes: &'a SharedKeyBoxSet,
    pub device_name: &'a DeviceLabelNameAndCommitmentKey,
    pub next_tree_location: [u8; 32],
    pub self_token: [u8; 17],
    pub hepks: &'a [Hepk],
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

pub struct AddTeamMemberArgument<'a> {
    pub link: &'a UserLink,
    pub next_tree_location: [u8; 32],
    pub ptk_boxes: &'a SharedKeyBoxSet,
    pub removal_keys: &'a [TeamRemovalBoxData],
    pub hepks: &'a [Hepk],
    pub local_permissions_for: &'a [EntityId],
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
        for found in [
            self.removal_keys.len(),
            self.hepks.len(),
            self.local_permissions_for.len(),
        ] {
            if found != 1 {
                return Err(Error::FieldCount { expected: 1, found });
            }
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
            Value::Null,
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
            Value::Null,
        ]))?)
    }
}

pub struct RevokeDeviceArgument<'a> {
    pub link: &'a UserLink,
    pub puk_boxes: &'a SharedKeyBoxSet,
    pub seed_chain: &'a [SeedChainBox],
    pub next_tree_location: [u8; 32],
    pub hepks: &'a [Hepk],
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
            Value::Null,
            Value::Array(vec![Value::Array(hepks)]),
        ]))?)
    }
}
