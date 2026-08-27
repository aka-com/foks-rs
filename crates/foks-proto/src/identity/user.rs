//! Authenticated user-chain response schema.

use super::{
    array_any, device_label_name_and_commitment_key, hepk, name_commitment_and_key, user_link,
    user_merkle_paths, DeviceLabelNameAndCommitmentKey, Hepk, NameCommitmentAndKey, UserLink,
    UserMerklePaths,
};
use crate::{array, decode, encode, fixed_blob, list, text, unsigned, Error, Result, Value};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserChain {
    pub links: Vec<UserLink>,
    pub locations: Vec<[u8; 32]>,
    pub usernames: Vec<NameCommitmentAndKey>,
    pub merkle: UserMerklePaths,
    pub device_names: Vec<DeviceLabelNameAndCommitmentKey>,
    pub username_utf8: Vec<u8>,
    pub num_username_links: u64,
    pub hepks: Vec<Hepk>,
    pub exact_bytes: Vec<u8>,
}

/// Canonical server-side construction of the v0.1.9 `UserChain` result.
pub struct UserChainResponse<'a> {
    pub exact_links: &'a [Vec<u8>],
    pub locations: &'a [[u8; 32]],
    pub usernames: &'a [NameCommitmentAndKey],
    pub exact_root: &'a [u8],
    pub paths: &'a [super::MerklePathCompressed],
    pub device_names: &'a [DeviceLabelNameAndCommitmentKey],
    pub username_utf8: &'a [u8],
    pub num_username_links: u64,
    pub exact_hepks: &'a [Vec<u8>],
}

impl UserChainResponse<'_> {
    pub fn encoded(&self) -> Result<Vec<u8>> {
        let links = self
            .exact_links
            .iter()
            .map(|link| decode(link).map_err(Error::from))
            .collect::<Result<Vec<_>>>()?;
        let root = decode(self.exact_root)?;
        let hepks = self
            .exact_hepks
            .iter()
            .map(|hepk| decode(hepk).map_err(Error::from))
            .collect::<Result<Vec<_>>>()?;
        let wire = Value::Array(vec![
            list_value(links),
            list_value(
                self.locations
                    .iter()
                    .map(|location| Value::Binary(location.to_vec()))
                    .collect(),
            ),
            list_value(
                self.usernames
                    .iter()
                    .map(NameCommitmentAndKey::to_value)
                    .collect(),
            ),
            Value::Array(vec![
                root,
                Value::Array(self.paths.iter().map(|path| path.to_value()).collect()),
            ]),
            list_value(
                self.device_names
                    .iter()
                    .map(DeviceLabelNameAndCommitmentKey::to_value)
                    .collect(),
            ),
            Value::Text(self.username_utf8.to_vec()),
            Value::Unsigned(self.num_username_links),
            Value::Array(vec![list_value(hepks)]),
        ]);
        let encoded = encode(&wire)?;
        UserChain::decode(&encoded)?;
        Ok(encoded)
    }
}

fn list_value(values: Vec<Value>) -> Value {
    if values.is_empty() {
        Value::Null
    } else {
        Value::Array(values)
    }
}

impl UserChain {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let wire = decode(bytes)?;
        let fields = array(&wire, 8)?;
        let links = list(&fields[0], user_link)?;
        let locations = list(&fields[1], |value| fixed_blob(value, "tree location"))?;
        let usernames = list(&fields[2], name_commitment_and_key)?;
        let merkle = user_merkle_paths(&fields[3])?;
        let device_names = list(&fields[4], device_label_name_and_commitment_key)?;
        let username_utf8 = text(&fields[5])?.into_bytes();
        let num_username_links = unsigned(&fields[6])?;
        let hepk_set = array(&fields[7], 1)?;
        let hepks = array_any(&hepk_set[0])?
            .iter()
            .map(hepk)
            .collect::<Result<Vec<_>>>()?;
        let username_path_count =
            usize::try_from(num_username_links).map_err(|_| Error::FieldCount {
                expected: usize::MAX,
                found: merkle.paths().len(),
            })?;
        let expected_paths = links
            .len()
            .checked_add(username_path_count)
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
            usernames,
            merkle,
            device_names,
            username_utf8,
            num_username_links,
            hepks,
            exact_bytes: bytes.to_vec(),
        })
    }
}
