//! Generic user subchains and server-trusted local team listings.

use super::{role, user_link, user_merkle_paths, UserLink, UserMerklePaths};
use crate::{
    array, decode, encode, fixed_blob, list, option, EntityId, Error, MerklePathCompressed, Result,
    Role, Value,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GenericChain {
    pub links: Vec<UserLink>,
    pub locations: Vec<[u8; 32]>,
    pub merkle: UserMerklePaths,
    pub location_seed: Option<[u8; 32]>,
}

impl GenericChain {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let wire = decode(bytes)?;
        let fields = array(&wire, 4)?;
        let links = list(&fields[0], user_link)?;
        let locations = list(&fields[1], |value| {
            fixed_blob(value, "generic tree location")
        })?;
        let merkle = user_merkle_paths(&fields[2])?;
        let location_seed = option(&fields[3], |value| {
            fixed_blob(value, "subchain tree-location seed")
        })?;
        let expected_paths = links.len().checked_add(1).ok_or(Error::FieldCount {
            expected: usize::MAX,
            found: merkle.paths().len(),
        })?;
        let valid_location_count = match links.first() {
            Some(link) if link.decode_generic()?.sequence == 1 => locations.len() == links.len(),
            Some(_) => locations.len() == expected_paths,
            // An empty suffix has no returned link from which to recover its
            // starting seqno. Accept the only two Go shapes: empty eldest or
            // the single prior location for an up-to-date incremental load.
            None => locations.len() <= 1,
        };
        if !valid_location_count || merkle.paths().len() != expected_paths {
            return Err(Error::FieldCount {
                expected: expected_paths,
                found: merkle.paths().len(),
            });
        }
        Ok(Self {
            links,
            locations,
            merkle,
            location_seed,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GenericChainResponse<'a> {
    pub exact_links: &'a [Vec<u8>],
    pub locations: &'a [[u8; 32]],
    pub exact_root: &'a [u8],
    pub paths: &'a [MerklePathCompressed],
    pub location_seed: Option<[u8; 32]>,
}

impl GenericChainResponse<'_> {
    pub fn encoded(&self) -> Result<Vec<u8>> {
        let links = list_value(
            self.exact_links
                .iter()
                .map(|link| decode(link).map_err(Error::from))
                .collect::<Result<Vec<_>>>()?,
        );
        let locations = list_value(
            self.locations
                .iter()
                .map(|location| Value::Binary(location.to_vec()))
                .collect(),
        );
        let paths = list_value(
            self.paths
                .iter()
                .map(MerklePathCompressed::to_value)
                .collect(),
        );
        encode(&Value::Array(vec![
            links,
            locations,
            Value::Array(vec![decode(self.exact_root)?, paths]),
            self.location_seed
                .map(|seed| Value::Binary(seed.to_vec()))
                .unwrap_or(Value::Null),
        ]))
        .map_err(Into::into)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalTeamListEntry {
    pub team: EntityId,
    pub source_role: Role,
    pub destination_role: Role,
    pub team_sequence: u64,
    pub key_generation: u64,
}

pub fn encode_local_team_list(entries: &[LocalTeamListEntry]) -> Result<Vec<u8>> {
    let values = entries
        .iter()
        .map(|entry| {
            Value::Array(vec![
                Value::Binary(entry.team.as_bytes().to_vec()),
                entry.source_role.to_value(),
                entry.destination_role.to_value(),
                Value::Unsigned(entry.team_sequence),
                Value::Unsigned(entry.key_generation),
            ])
        })
        .collect();
    Ok(encode(&list_value(values))?)
}

pub fn decode_local_team_list(bytes: &[u8]) -> Result<Vec<LocalTeamListEntry>> {
    list(&decode(bytes)?, |value| {
        let fields = array(value, 5)?;
        Ok(LocalTeamListEntry {
            team: crate::entity(&fields[0])?,
            source_role: role(&fields[1])?,
            destination_role: role(&fields[2])?,
            team_sequence: crate::unsigned(&fields[3])?,
            key_generation: crate::unsigned(&fields[4])?,
        })
    })
}

fn list_value(values: Vec<Value>) -> Value {
    if values.is_empty() {
        Value::Null
    } else {
        Value::Array(values)
    }
}
