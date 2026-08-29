//! Authenticated team-chain and team-view response schemas.

use super::{
    array_any, hepk, name_commitment_and_key, role, user_link, user_merkle_paths, Hepk,
    NameCommitmentAndKey, UserLink, UserMerklePaths,
};
use crate::{
    array, decode, encode, entity, fixed_blob, list, option, puk_parcel, text, type_error,
    unsigned, variant, EntityId, Error, PukParcel, Result, Role, TeamRemoteMemberViewTokenInner,
    TeamRemovalKeyBox, Value, ENTITY_HOST,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamChain {
    pub links: Vec<UserLink>,
    pub locations: Vec<[u8; 32]>,
    pub team_names: Vec<NameCommitmentAndKey>,
    pub merkle: UserMerklePaths,
    pub team_name_utf8: Vec<u8>,
    pub num_team_name_links: u64,
    pub boxes: Vec<PukParcel>,
    pub removal_key: Option<TeamRemovalKeyBox>,
    pub remote_view_tokens: Vec<TeamRemoteMemberViewTokenInner>,
    pub hepks: Vec<Hepk>,
    pub exact_bytes: Vec<u8>,
}

impl TeamChain {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let wire = decode(bytes)?;
        let fields = array(&wire, 10)?;
        let links = list(&fields[0], user_link)?;
        let locations = list(&fields[1], |value| fixed_blob(value, "tree location"))?;
        let team_names = list(&fields[2], name_commitment_and_key)?;
        let merkle = user_merkle_paths(&fields[3])?;
        let team_name_utf8 = text(&fields[4])?.into_bytes();
        let num_team_name_links = unsigned(&fields[5])?;
        let boxes = list(&fields[6], puk_parcel)?;
        let removal_key = option(&fields[7], |value| {
            TeamRemovalKeyBox::decode(&encode(value)?)
        })?;
        let remote_view_tokens = list(&fields[8], |value| {
            TeamRemoteMemberViewTokenInner::decode(&encode(value)?)
        })?;
        let hepk_set = array(&fields[9], 1)?;
        let hepks = array_any(&hepk_set[0])?
            .iter()
            .map(hepk)
            .collect::<Result<Vec<_>>>()?;
        let name_path_count =
            usize::try_from(num_team_name_links).map_err(|_| Error::FieldCount {
                expected: usize::MAX,
                found: merkle.paths().len(),
            })?;
        let expected_paths = links
            .len()
            .checked_add(name_path_count)
            .and_then(|count| count.checked_add(1))
            .ok_or(Error::FieldCount {
                expected: usize::MAX,
                found: merkle.paths().len(),
            })?;
        let valid_location_count = locations.len() == links.len()
            || links
                .len()
                .checked_add(1)
                .is_some_and(|count| locations.len() == count);
        if !valid_location_count || merkle.paths().len() != expected_paths {
            return Err(Error::FieldCount {
                expected: expected_paths,
                found: merkle.paths().len(),
            });
        }
        Ok(Self {
            links,
            locations,
            team_names,
            merkle,
            team_name_utf8,
            num_team_name_links,
            boxes,
            removal_key,
            remote_view_tokens,
            hepks,
            exact_bytes: bytes.to_vec(),
        })
    }
}

pub struct TeamChainResponse<'a> {
    pub exact_links: &'a [Vec<u8>],
    pub locations: &'a [[u8; 32]],
    pub team_names: &'a [NameCommitmentAndKey],
    pub exact_root: &'a [u8],
    pub paths: &'a [super::MerklePathCompressed],
    pub team_name_utf8: &'a [u8],
    pub num_team_name_links: u64,
    pub exact_parcels: &'a [Vec<u8>],
    pub exact_removal_key: Option<&'a [u8]>,
    pub remote_view_tokens: &'a [TeamRemoteMemberViewTokenInner],
    pub exact_hepks: &'a [Vec<u8>],
}

impl TeamChainResponse<'_> {
    pub fn encoded(&self) -> Result<Vec<u8>> {
        let list_or_null = |values: Vec<Value>| {
            if values.is_empty() {
                Value::Null
            } else {
                Value::Array(values)
            }
        };
        let encoded = encode(&Value::Array(vec![
            list_or_null(
                self.exact_links
                    .iter()
                    .map(|exact| decode(exact).map_err(Error::from))
                    .collect::<Result<Vec<_>>>()?,
            ),
            list_or_null(
                self.locations
                    .iter()
                    .map(|location| Value::Binary(location.to_vec()))
                    .collect(),
            ),
            list_or_null(
                self.team_names
                    .iter()
                    .map(NameCommitmentAndKey::to_value)
                    .collect(),
            ),
            Value::Array(vec![
                decode(self.exact_root)?,
                Value::Array(self.paths.iter().map(|path| path.to_value()).collect()),
            ]),
            Value::Text(self.team_name_utf8.to_vec()),
            Value::Unsigned(self.num_team_name_links),
            list_or_null(
                self.exact_parcels
                    .iter()
                    .map(|exact| decode(exact).map_err(Error::from))
                    .collect::<Result<Vec<_>>>()?,
            ),
            self.exact_removal_key
                .map(decode)
                .transpose()?
                .unwrap_or(Value::Null),
            list_or_null(
                self.remote_view_tokens
                    .iter()
                    .map(TeamRemoteMemberViewTokenInner::to_value)
                    .collect(),
            ),
            Value::Array(vec![list_or_null(
                self.exact_hepks
                    .iter()
                    .map(|exact| decode(exact).map_err(Error::from))
                    .collect::<Result<Vec<_>>>()?,
            )]),
        ]))?;
        TeamChain::decode(&encoded)?;
        Ok(encoded)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamViewRequest {
    pub team: EntityId,
    pub host: EntityId,
    pub member: EntityId,
    pub member_host: EntityId,
    pub source_role: Role,
    pub generation: u64,
}

impl TeamViewRequest {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        team_view_request(&decode(bytes)?)
    }

    pub fn to_value(&self) -> Value {
        Value::Array(vec![
            Value::Array(vec![
                Value::Binary(self.host.as_bytes().to_vec()),
                Value::Array(vec![
                    Value::Bool(true),
                    Value::Variant(Some((
                        b"1".to_vec(),
                        Box::new(Value::Binary(self.team.as_bytes().to_vec())),
                    ))),
                ]),
            ]),
            Value::Array(vec![
                Value::Binary(self.member.as_bytes().to_vec()),
                Value::Binary(self.member_host.as_bytes().to_vec()),
            ]),
            self.source_role.to_value(),
            Value::Unsigned(self.generation),
        ])
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&self.to_value())?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamViewChallenge {
    pub request: TeamViewRequest,
    pub time: u64,
    pub token: [u8; 16],
    pub key_id: [u8; 16],
    pub mac: [u8; 32],
}

impl TeamViewChallenge {
    pub fn payload_encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![
            self.request.to_value(),
            Value::Unsigned(self.time),
            Value::Binary(self.token.to_vec()),
            Value::Binary(self.key_id.to_vec()),
        ]))?)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let wire = decode(bytes)?;
        let fields = array(&wire, 2)?;
        let payload = array(&fields[0], 4)?;
        Ok(Self {
            request: team_view_request(&payload[0])?,
            time: unsigned(&payload[1])?,
            token: fixed_blob(&payload[2], "team view token")?,
            key_id: fixed_blob(&payload[3], "team view HMAC key ID")?,
            mac: fixed_blob(&fields[1], "team view challenge MAC")?,
        })
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![
            decode(&self.payload_encoded()?)?,
            Value::Binary(self.mac.to_vec()),
        ]))?)
    }
}

fn team_view_request(value: &Value) -> Result<TeamViewRequest> {
    let fields = array(value, 4)?;
    let team = array(&fields[0], 2)?;
    let id_or_name = array(&team[1], 2)?;
    if id_or_name[0] != Value::Bool(true) {
        return Err(type_error("team ID selector", &id_or_name[0]));
    }
    let member = array(&fields[1], 2)?;
    Ok(TeamViewRequest {
        team: entity(variant(&id_or_name[1], "1")?)?,
        host: entity(&team[0])?.require_type(ENTITY_HOST)?,
        member: entity(&member[0])?,
        member_host: entity(&member[1])?.require_type(ENTITY_HOST)?,
        source_role: role(&fields[2])?,
        generation: unsigned(&fields[3])?,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivatedTeamView {
    pub token: [u8; 16],
    pub team: EntityId,
}

impl ActivatedTeamView {
    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![
            Value::Binary(self.token.to_vec()),
            Value::Binary(self.team.as_bytes().to_vec()),
        ]))?)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let wire = decode(bytes)?;
        let fields = array(&wire, 2)?;
        Ok(Self {
            token: fixed_blob(&fields[0], "activated team view token")?,
            team: entity(&fields[1])?,
        })
    }
}
