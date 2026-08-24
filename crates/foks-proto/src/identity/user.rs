//! Authenticated user-chain response schema.

use super::{
    array_any, device_label_name_and_commitment_key, hepk, name_commitment_and_key, user_link,
    user_merkle_paths, DeviceLabelNameAndCommitmentKey, Hepk, NameCommitmentAndKey, UserLink,
    UserMerklePaths,
};
use crate::{array, decode, fixed_blob, list, text, unsigned, Error, Result};

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
