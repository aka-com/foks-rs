//! Authenticated team-chain and team-view response schemas.

use super::{
    array_any, hepk, list_values, name_commitment_and_key, role, user_link, user_merkle_paths,
    Hepk, NameCommitmentAndKey, UserLink, UserMerklePaths,
};
use crate::{
    array, decode, encode, entity, fixed_blob, list, option, puk_parcel, text, type_error,
    unsigned, variant, EntityId, Error, PukParcel, Result, Role, Value, ENTITY_HOST,
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
        // Removal keys and remote-view tokens are intentionally retained in
        // the exact transcript but are not part of the read-only PTK slice.
        option(&fields[7], |_| Ok(()))?;
        list_values(&fields[8])?;
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
            hepks,
            exact_bytes: bytes.to_vec(),
        })
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
            Value::Array(vec![
                self.request.to_value(),
                Value::Unsigned(self.time),
                Value::Binary(self.token.to_vec()),
                Value::Binary(self.key_id.to_vec()),
            ]),
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
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let wire = decode(bytes)?;
        let fields = array(&wire, 2)?;
        Ok(Self {
            token: fixed_blob(&fields[0], "activated team view token")?,
            team: entity(&fields[1])?,
        })
    }
}
