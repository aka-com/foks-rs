//! Registration, device mutation, host-policy, and ad-hoc team arguments.

use super::{DeviceLabelNameAndCommitmentKey, Hepk, UserLink};
use crate::{
    array, boolean, decode, encode, fixed_blob, list_or_null, unsigned, Error, Result,
    SeedChainBox, SharedKeyBoxSet, Value,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UsernameReservation {
    pub token: [u8; 17],
    pub sequence: u64,
    pub expires_at: u64,
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
                        Value::Unsigned(label.device_type),
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
                        Value::Unsigned(label.device_type),
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
