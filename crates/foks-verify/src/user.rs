//! User-chain replay, disclosure verification, and sealed user state.

use unicode_normalization::{char::is_combining_mark, UnicodeNormalization as _};

use std::collections::BTreeSet;

use crate::{
    commitment, encode, prefixed_hash, user_transition::user_member_hepk_matches,
    verify_hostchain_at_tail, verify_merkle_path, verify_merkle_path_present, verify_typed,
    AuthenticatedMerkleRoots, ChangeMetadata, EntityId, Error, Hepk, HostchainTail, Result, Role,
    TreeRoot, UserChain, UserEldest, UserReplayState, UserTransitionRule, Value,
    DEVICE_LABEL_TYPE_ID, ENTITY_ID_MERKLE_VALUE_TYPE_ID, HEPK_TYPE_ID, LINK_OUTER_TYPE_ID,
    LINK_OUTER_V1_TYPE_ID, MERKLE_ROOT_TYPE_ID, MERKLE_TREE_RF_INPUT_TYPE_ID,
    NAME_COMMITMENT_TYPE_ID, NAME_HASH_PREIMAGE_TYPE_ID, TREE_LOCATION_TYPE_ID,
};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct VerifiedUserDevice {
    pub device_id: Vec<u8>,
    pub role: Role,
    pub hepk_bytes: Vec<u8>,
    pub subkey_id: Option<Vec<u8>>,
}

/// Extracts the bounded Merkle epochs an untrusted user-chain response asks
/// the caller to authenticate. This grants no trust; callers must prove every
/// returned epoch from an already authenticated root before replay.
pub fn user_chain_root_epochs(chain_bytes: &[u8]) -> Result<Vec<u64>> {
    let chain = UserChain::decode(chain_bytes)?;
    if chain.links.len() > 4096 {
        return Err(Error::UserChainContinuity);
    }
    let mut epochs = BTreeSet::from([chain.merkle.root().epoch]);
    for link in &chain.links {
        epochs.insert(link.decode_group_change()?.root.epoch);
    }
    Ok(epochs.into_iter().collect())
}

impl VerifiedUserDevice {
    pub fn device_id(&self) -> &[u8] {
        &self.device_id
    }
    pub fn role(&self) -> Role {
        self.role
    }
    pub fn hepk_bytes(&self) -> &[u8] {
        &self.hepk_bytes
    }
    pub fn subkey_id(&self) -> Option<&[u8]> {
        self.subkey_id.as_deref()
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct VerifiedUserSharedKey {
    pub role: Role,
    pub generation: u64,
    pub verify_key: Vec<u8>,
    pub hepk_bytes: Vec<u8>,
}

impl VerifiedUserSharedKey {
    pub fn role(&self) -> Role {
        self.role
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn verify_key(&self) -> &[u8] {
        &self.verify_key
    }
    pub fn hepk_bytes(&self) -> &[u8] {
        &self.hepk_bytes
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedUserSnapshot {
    pub(crate) host_id: Vec<u8>,
    pub(crate) uid: Vec<u8>,
    pub(crate) chain_seqno: u64,
    pub(crate) chain_tail_hash: [u8; 32],
    pub(crate) chain_bytes: Vec<u8>,
    pub(crate) evidence_bytes: Vec<u8>,
    pub(crate) username: Vec<u8>,
    pub(crate) username_utf8: Vec<u8>,
    pub(crate) username_sequence: u64,
    pub(crate) merkle_epoch: u64,
    pub(crate) merkle_root_hash: [u8; 32],
    pub(crate) merkle_root_bytes: Vec<u8>,
    pub(crate) devices: Vec<VerifiedUserDevice>,
    pub(crate) shared_keys: Vec<VerifiedUserSharedKey>,
}

impl VerifiedUserSnapshot {
    pub fn parts(&self) -> VerifiedUserSnapshotParts<'_> {
        VerifiedUserSnapshotParts {
            host_id: &self.host_id,
            uid: &self.uid,
            chain_seqno: self.chain_seqno,
            chain_tail_hash: self.chain_tail_hash,
            chain_bytes: &self.chain_bytes,
            evidence_bytes: &self.evidence_bytes,
            username: &self.username,
            username_utf8: &self.username_utf8,
            username_sequence: self.username_sequence,
            merkle_epoch: self.merkle_epoch,
            merkle_root_hash: self.merkle_root_hash,
            merkle_root_bytes: &self.merkle_root_bytes,
            devices: &self.devices,
            shared_keys: &self.shared_keys,
        }
    }

    pub fn host_id(&self) -> &[u8] {
        &self.host_id
    }
    pub fn uid(&self) -> &[u8] {
        &self.uid
    }
    pub fn chain_seqno(&self) -> u64 {
        self.chain_seqno
    }
    pub fn chain_tail_hash(&self) -> [u8; 32] {
        self.chain_tail_hash
    }
    pub fn chain_bytes(&self) -> &[u8] {
        &self.chain_bytes
    }
    pub fn evidence_bytes(&self) -> &[u8] {
        &self.evidence_bytes
    }
    pub fn username(&self) -> &[u8] {
        &self.username
    }
    pub fn username_utf8(&self) -> &[u8] {
        &self.username_utf8
    }
    pub fn username_sequence(&self) -> u64 {
        self.username_sequence
    }
    pub fn merkle_epoch(&self) -> u64 {
        self.merkle_epoch
    }
    pub fn merkle_root_hash(&self) -> [u8; 32] {
        self.merkle_root_hash
    }
    pub fn merkle_root_bytes(&self) -> &[u8] {
        &self.merkle_root_bytes
    }
    pub fn devices(&self) -> &[VerifiedUserDevice] {
        &self.devices
    }
    pub fn shared_keys(&self) -> &[VerifiedUserSharedKey] {
        &self.shared_keys
    }
}

#[derive(Clone, Copy)]
pub struct VerifiedUserSnapshotParts<'a> {
    pub host_id: &'a [u8],
    pub uid: &'a [u8],
    pub chain_seqno: u64,
    pub chain_tail_hash: [u8; 32],
    pub chain_bytes: &'a [u8],
    pub evidence_bytes: &'a [u8],
    pub username: &'a [u8],
    pub username_utf8: &'a [u8],
    pub username_sequence: u64,
    pub merkle_epoch: u64,
    pub merkle_root_hash: [u8; 32],
    pub merkle_root_bytes: &'a [u8],
    pub devices: &'a [VerifiedUserDevice],
    pub shared_keys: &'a [VerifiedUserSharedKey],
}

/// Verified public user-chain projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedUserState {
    uid: EntityId,
    host: EntityId,
    chain_tail_hash: [u8; 32],
    merkle_root_hash: [u8; 32],
    merkle_epoch: u64,
    merkle_root_bytes: Vec<u8>,
    authenticated_chain_bytes: Vec<u8>,
    evidence_bytes: Vec<u8>,
    chain_seqno: u64,
    next_tree_location: [u8; 32],
    username: Vec<u8>,
    username_utf8: Vec<u8>,
    username_sequence: u64,
    devices: Vec<VerifiedDevice>,
    shared_keys: Vec<VerifiedSharedKey>,
    shared_key_history: Vec<VerifiedSharedKey>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedDevice {
    pub id: EntityId,
    pub role: Role,
    pub hepk: Hepk,
    pub subkey: Option<EntityId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedSharedKey {
    pub role: Role,
    pub generation: u64,
    pub verify_key: EntityId,
    pub hepk: Hepk,
}

impl VerifiedUserState {
    pub fn uid(&self) -> &EntityId {
        &self.uid
    }

    pub fn host(&self) -> &EntityId {
        &self.host
    }

    pub fn chain_seqno(&self) -> u64 {
        self.chain_seqno
    }

    pub fn chain_tail_hash(&self) -> [u8; 32] {
        self.chain_tail_hash
    }

    /// Hidden location committed by the current tail and used to address the
    /// next user-chain link. It is reproduced from authenticated chain
    /// evidence whenever hard state is reopened.
    pub fn next_tree_location(&self) -> [u8; 32] {
        self.next_tree_location
    }

    pub fn tree_root(&self) -> TreeRoot {
        TreeRoot {
            epoch: self.merkle_epoch,
            hash: self.merkle_root_hash,
        }
    }

    pub fn username(&self) -> &[u8] {
        &self.username
    }

    pub fn username_utf8(&self) -> &[u8] {
        &self.username_utf8
    }

    pub fn username_sequence(&self) -> u64 {
        self.username_sequence
    }

    pub fn devices(&self) -> &[VerifiedDevice] {
        &self.devices
    }

    pub fn shared_keys(&self) -> &[VerifiedSharedKey] {
        &self.shared_keys
    }

    pub fn shared_key(&self, role: Role) -> Option<&VerifiedSharedKey> {
        self.shared_keys.iter().find(|key| key.role == role)
    }

    /// Every authenticated PUK generation observed while replaying the user
    /// chain, including the current keys. Rotation code uses this to prevent
    /// reintroducing previously retired key material.
    pub fn shared_key_history(&self) -> &[VerifiedSharedKey] {
        &self.shared_key_history
    }

    pub fn hard_state_snapshot(&self) -> Result<VerifiedUserSnapshot> {
        Ok(VerifiedUserSnapshot {
            host_id: self.host.as_bytes().to_vec(),
            uid: self.uid.as_bytes().to_vec(),
            chain_seqno: self.chain_seqno,
            chain_tail_hash: self.chain_tail_hash,
            chain_bytes: self.authenticated_chain_bytes.clone(),
            evidence_bytes: self.evidence_bytes.clone(),
            username: self.username.clone(),
            username_utf8: self.username_utf8.clone(),
            username_sequence: self.username_sequence,
            merkle_epoch: self.merkle_epoch,
            merkle_root_hash: self.merkle_root_hash,
            merkle_root_bytes: self.merkle_root_bytes.clone(),
            devices: self
                .devices
                .iter()
                .map(|device| {
                    Ok(VerifiedUserDevice {
                        device_id: device.id.as_bytes().to_vec(),
                        role: device.role,
                        hepk_bytes: device.hepk.encoded()?,
                        subkey_id: device
                            .subkey
                            .as_ref()
                            .map(|subkey| subkey.as_bytes().to_vec()),
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            shared_keys: self
                .shared_keys
                .iter()
                .map(|key| {
                    Ok(VerifiedUserSharedKey {
                        role: key.role,
                        generation: key.generation,
                        verify_key: key.verify_key.as_bytes().to_vec(),
                        hepk_bytes: key.hepk.encoded()?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        })
    }
}

/// Replays an arbitrary v0.1.9 user group-change chain from eldest through
/// device provisioning, revocation, and PUK rotation. Every link must be
/// committed by the final Merkle root and must cite an independently
/// authenticated historical root.
pub fn verify_user_chain(
    chain_bytes: &[u8],
    expected_uid: &EntityId,
    expected_host: &EntityId,
    authenticated_roots: &AuthenticatedMerkleRoots,
    trusted_hostchain: &HostchainTail,
) -> Result<VerifiedUserState> {
    let chain = UserChain::decode(chain_bytes)?;
    if chain.links.is_empty() || chain.locations.len() != chain.links.len() {
        return Err(Error::UnsupportedUserChain);
    }
    let root_bytes = chain.merkle.encoded_root()?;
    let authenticated_chain_bytes = authenticated_user_chain_bytes(&chain.links)?;
    let root_hash = prefixed_hash(MERKLE_ROOT_TYPE_ID, &root_bytes)?;
    if authenticated_roots.get(&chain.merkle.root().epoch) != Some(&root_hash)
        || &chain.merkle.root().hostchain != trusted_hostchain
    {
        return Err(Error::UntrustedUserRoot);
    }

    let (username, username_utf8, username_sequence) =
        verify_user_disclosures(&chain, expected_uid, expected_host)?;
    let path_offset =
        usize::try_from(chain.num_username_links).map_err(|_| Error::UserMerkleProof)?;
    // The username-link offset and link count are attacker-controlled fields of
    // the decoded chain; reject a response whose Merkle path slice would fall
    // outside the supplied paths rather than panicking on an out-of-bounds index.
    let path_end = path_offset
        .checked_add(chain.links.len())
        .filter(|end| *end <= chain.merkle.paths().len())
        .ok_or(Error::UserMerkleProof)?;

    let mut replay_state = None;
    let mut previous_hash = None;

    for (index, ((link, location), path)) in chain
        .links
        .iter()
        .zip(&chain.locations)
        .zip(&chain.merkle.paths()[path_offset..path_end])
        .enumerate()
    {
        let sequence = u64::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .ok_or(Error::UserChainContinuity)?;
        let change = link.decode_group_change()?;
        if change.seqno != sequence
            || change.previous != previous_hash
            || &change.uid != expected_uid
            || &change.host != expected_host
            || authenticated_roots.get(&change.root.epoch) != Some(&change.root.hash)
        {
            return Err(Error::UserChainContinuity);
        }
        let location_wire = encode(&Value::Binary(location.to_vec()))?;
        if prefixed_hash(TREE_LOCATION_TYPE_ID, &location_wire)? != change.next_location_commitment
        {
            return Err(Error::UserChainContinuity);
        }
        let location_for_key = index.checked_sub(1).map(|prior| &chain.locations[prior]);
        let merkle_key = user_merkle_key(expected_uid, sequence, location_for_key)?;
        let link_hash = prefixed_hash(LINK_OUTER_TYPE_ID, &link.encoded()?)?;
        verify_merkle_path(
            path,
            &merkle_key,
            Some(&link_hash),
            &chain.merkle.root().root_node,
        )?;

        if index == 0 {
            let eldest = link.decode_eldest()?;
            verify_eldest_bindings(&eldest, expected_uid, expected_host)?;
            if eldest.seqno != 1
                || eldest.previous.is_some()
                || eldest.puk_generation != 1
                || change.changes.len() != 1
                || change.shared_keys.len() != 1
            {
                return Err(Error::UserBinding);
            }
            let persistent_puk = persistent_user_id(&eldest.puk_verify_key)?;
            let expected_signatures = if eldest.member_subkey.is_some() { 3 } else { 2 };
            if &persistent_puk != expected_uid || link.signatures().len() != expected_signatures {
                return Err(Error::UserBinding);
            }
            verify_typed(
                &eldest.puk_verify_key,
                &link.signatures()[0],
                LINK_OUTER_V1_TYPE_ID,
                &link.signing_bytes(0)?,
            )?;
            if let Some(subkey) = &eldest.member_subkey {
                verify_typed(
                    subkey,
                    &link.signatures()[1],
                    LINK_OUTER_V1_TYPE_ID,
                    &link.signing_bytes(1)?,
                )?;
            }
            let device_index = expected_signatures - 1;
            verify_typed(
                &eldest.signer,
                &link.signatures()[device_index],
                LINK_OUTER_V1_TYPE_ID,
                &link.signing_bytes(device_index)?,
            )?;
            let device_hepk = find_hepk(&chain.hepks, eldest.member_hepk_fingerprint)?;
            let puk_hepk = find_hepk(&chain.hepks, eldest.puk_hepk_fingerprint)?;
            if !user_member_hepk_matches(&eldest.member, &device_hepk)
                || puk_hepk.curve25519().is_none()
            {
                return Err(Error::UserBinding);
            }
            replay_state = Some(UserReplayState::from_eldest(
                VerifiedDevice {
                    id: eldest.member,
                    role: Role::OWNER,
                    hepk: device_hepk,
                    subkey: eldest.member_subkey,
                },
                VerifiedSharedKey {
                    role: Role::OWNER,
                    generation: 1,
                    verify_key: eldest.puk_verify_key,
                    hepk: puk_hepk,
                },
            ));
        } else {
            replay_state.as_mut().ok_or(Error::UserBinding)?.replay(
                link,
                &change,
                &chain.hepks,
                expected_host,
            )?;
        }
        previous_hash = Some(link_hash);
    }

    let chain_seqno = u64::try_from(chain.links.len()).map_err(|_| Error::UserChainContinuity)?;
    let next_sequence = chain_seqno
        .checked_add(1)
        .ok_or(Error::UserChainContinuity)?;
    let next_key = user_merkle_key(expected_uid, next_sequence, chain.locations.last())?;
    verify_merkle_path(
        chain.merkle.paths().last().ok_or(Error::UserMerkleProof)?,
        &next_key,
        None,
        &chain.merkle.root().root_node,
    )?;
    let replay_state = replay_state.ok_or(Error::UserBinding)?;
    if !replay_state.is_complete() {
        return Err(Error::UserTransition {
            seqno: chain_seqno,
            rule: UserTransitionRule::EmptyResult,
        });
    }
    let (devices, shared_keys, shared_key_history) = replay_state.into_parts();
    Ok(VerifiedUserState {
        uid: expected_uid.clone(),
        host: expected_host.clone(),
        chain_tail_hash: previous_hash.ok_or(Error::UserChainContinuity)?,
        merkle_root_hash: root_hash,
        merkle_epoch: chain.merkle.root().epoch,
        merkle_root_bytes: root_bytes,
        authenticated_chain_bytes,
        evidence_bytes: chain_bytes.to_vec(),
        chain_seqno,
        next_tree_location: *chain.locations.last().ok_or(Error::UserChainContinuity)?,
        username,
        username_utf8,
        username_sequence,
        devices,
        shared_keys,
        shared_key_history,
    })
}

/// Verifies a server response beginning immediately after an already verified
/// user-chain tail. Only the returned suffix is replayed, while the resulting
/// evidence retains every independently authenticated response needed to
/// reconstruct the state after restart.
pub fn verify_user_chain_increment(
    chain_bytes: &[u8],
    prior: &VerifiedUserState,
    expected_uid: &EntityId,
    expected_host: &EntityId,
    authenticated_roots: &AuthenticatedMerkleRoots,
    trusted_hostchain: &HostchainTail,
) -> Result<VerifiedUserState> {
    if prior.uid != *expected_uid || prior.host != *expected_host {
        return Err(Error::UserChainContinuity);
    }
    let chain = UserChain::decode(chain_bytes)?;
    if chain.locations.len() != chain.links.len().saturating_add(1)
        || chain.locations.first() != Some(&prior.next_tree_location)
    {
        return Err(Error::UserChainContinuity);
    }
    let root_bytes = chain.merkle.encoded_root()?;
    let root_hash = prefixed_hash(MERKLE_ROOT_TYPE_ID, &root_bytes)?;
    if authenticated_roots.get(&chain.merkle.root().epoch) != Some(&root_hash)
        || &chain.merkle.root().hostchain != trusted_hostchain
    {
        return Err(Error::UntrustedUserRoot);
    }
    let (username, username_utf8, username_sequence) =
        verify_incremental_user_disclosures(&chain, prior, expected_uid, expected_host)?;
    let path_offset =
        usize::try_from(chain.num_username_links).map_err(|_| Error::UserMerkleProof)?;
    let mut replay_state = UserReplayState::from_verified(
        &prior.devices,
        &prior.shared_keys,
        &prior.shared_key_history,
    );
    let mut previous_hash = prior.chain_tail_hash;
    let start_sequence = prior
        .chain_seqno
        .checked_add(1)
        .ok_or(Error::UserChainContinuity)?;

    for (index, ((link, locations), path)) in chain
        .links
        .iter()
        .zip(chain.locations.windows(2))
        .zip(&chain.merkle.paths()[path_offset..path_offset + chain.links.len()])
        .enumerate()
    {
        let sequence = start_sequence
            .checked_add(u64::try_from(index).map_err(|_| Error::UserChainContinuity)?)
            .ok_or(Error::UserChainContinuity)?;
        let change = link.decode_group_change()?;
        if change.seqno != sequence
            || change.previous != Some(previous_hash)
            || &change.uid != expected_uid
            || &change.host != expected_host
            || authenticated_roots.get(&change.root.epoch) != Some(&change.root.hash)
        {
            return Err(Error::UserChainContinuity);
        }
        let location_wire = encode(&Value::Binary(locations[1].to_vec()))?;
        if prefixed_hash(TREE_LOCATION_TYPE_ID, &location_wire)? != change.next_location_commitment
        {
            return Err(Error::UserChainContinuity);
        }
        let merkle_key = user_merkle_key(expected_uid, sequence, Some(&locations[0]))?;
        let link_hash = prefixed_hash(LINK_OUTER_TYPE_ID, &link.encoded()?)?;
        verify_merkle_path(
            path,
            &merkle_key,
            Some(&link_hash),
            &chain.merkle.root().root_node,
        )?;
        replay_state.replay(link, &change, &chain.hepks, expected_host)?;
        previous_hash = link_hash;
    }

    let chain_seqno = prior
        .chain_seqno
        .checked_add(u64::try_from(chain.links.len()).map_err(|_| Error::UserChainContinuity)?)
        .ok_or(Error::UserChainContinuity)?;
    let next_location = *chain.locations.last().ok_or(Error::UserChainContinuity)?;
    let next_key = user_merkle_key(
        expected_uid,
        chain_seqno
            .checked_add(1)
            .ok_or(Error::UserChainContinuity)?,
        Some(&next_location),
    )?;
    verify_merkle_path(
        chain.merkle.paths().last().ok_or(Error::UserMerkleProof)?,
        &next_key,
        None,
        &chain.merkle.root().root_node,
    )?;
    if chain.links.is_empty()
        && root_hash == prior.merkle_root_hash
        && username == prior.username
        && username_sequence == prior.username_sequence
    {
        return Ok(prior.clone());
    }
    let (devices, shared_keys, shared_key_history) = replay_state.into_parts();
    Ok(VerifiedUserState {
        uid: expected_uid.clone(),
        host: expected_host.clone(),
        chain_tail_hash: previous_hash,
        merkle_root_hash: root_hash,
        merkle_epoch: chain.merkle.root().epoch,
        merkle_root_bytes: root_bytes,
        authenticated_chain_bytes: append_authenticated_user_chain_bytes(
            &prior.authenticated_chain_bytes,
            &chain.links,
        )?,
        evidence_bytes: append_user_evidence(&prior.evidence_bytes, chain_bytes)?,
        chain_seqno,
        next_tree_location: next_location,
        username,
        username_utf8,
        username_sequence,
        devices,
        shared_keys,
        shared_key_history,
    })
}

/// Replays persisted user-chain evidence and refuses any projection that is
/// not reproduced byte-for-byte. This is the only supported path from
/// untrusted SQLite rows back to a sealed user state.
pub fn restore_verified_user(
    persisted: VerifiedUserSnapshotParts<'_>,
    authenticated_roots: &AuthenticatedMerkleRoots,
    trusted_hostchain_bytes: &[u8],
) -> Result<VerifiedUserState> {
    let uid = EntityId::from_bytes(persisted.uid.to_vec())?;
    let host = EntityId::from_bytes(persisted.host_id.to_vec())?;
    let hostchain = foks_proto::decode_hostchain(trusted_hostchain_bytes)?;
    let segments = user_evidence_segments(persisted.evidence_bytes)?;
    let mut segments = segments.into_iter();
    let first = segments.next().ok_or(Error::PersistedUserEvidence)?;
    let first_chain = UserChain::decode(&first)?;
    verify_hostchain_at_tail(&hostchain, &first_chain.merkle.root().hostchain)?;
    let mut verified = verify_user_chain(
        &first,
        &uid,
        &host,
        authenticated_roots,
        &first_chain.merkle.root().hostchain,
    )?;
    for segment in segments {
        let chain = UserChain::decode(&segment)?;
        verify_hostchain_at_tail(&hostchain, &chain.merkle.root().hostchain)?;
        verified = verify_user_chain_increment(
            &segment,
            &verified,
            &uid,
            &host,
            authenticated_roots,
            &chain.merkle.root().hostchain,
        )?;
    }
    let reproduced = verified.hard_state_snapshot()?;
    let reproduced = reproduced.parts();
    if reproduced.host_id != persisted.host_id
        || reproduced.uid != persisted.uid
        || reproduced.chain_seqno != persisted.chain_seqno
        || reproduced.chain_tail_hash != persisted.chain_tail_hash
        || reproduced.chain_bytes != persisted.chain_bytes
        || reproduced.evidence_bytes != persisted.evidence_bytes
        || reproduced.username != persisted.username
        || reproduced.username_utf8 != persisted.username_utf8
        || reproduced.username_sequence != persisted.username_sequence
        || reproduced.merkle_epoch != persisted.merkle_epoch
        || reproduced.merkle_root_hash != persisted.merkle_root_hash
        || reproduced.merkle_root_bytes != persisted.merkle_root_bytes
        || reproduced.devices != persisted.devices
        || reproduced.shared_keys != persisted.shared_keys
    {
        return Err(Error::PersistedUserEvidence);
    }
    Ok(verified)
}

pub(crate) fn authenticated_user_chain_bytes(links: &[foks_proto::UserLink]) -> Result<Vec<u8>> {
    let links = links
        .iter()
        .map(|link| Ok(foks_snowpack::decode(&link.encoded()?)?))
        .collect::<Result<Vec<_>>>()?;
    Ok(encode(&Value::Array(links))?)
}

pub(crate) fn append_authenticated_user_chain_bytes(
    prior: &[u8],
    links: &[foks_proto::UserLink],
) -> Result<Vec<u8>> {
    let Value::Array(mut values) = foks_snowpack::decode(prior)? else {
        return Err(Error::UserChainContinuity);
    };
    values.extend(
        links
            .iter()
            .map(|link| Ok(foks_snowpack::decode(&link.encoded()?)?))
            .collect::<Result<Vec<_>>>()?,
    );
    Ok(encode(&Value::Array(values))?)
}

pub(crate) fn authenticated_user_chain_link_at(
    chain: &[u8],
    index: usize,
) -> Result<foks_proto::UserLink> {
    let Value::Array(values) = foks_snowpack::decode(chain)? else {
        return Err(Error::UserChainContinuity);
    };
    let value = values.get(index).ok_or(Error::UserChainContinuity)?;
    Ok(foks_proto::UserLink::decode(&encode(value)?)?)
}

fn user_evidence_segments(evidence: &[u8]) -> Result<Vec<Vec<u8>>> {
    match foks_snowpack::decode(evidence)? {
        Value::Variant(Some((tag, value))) if tag == b"1" => {
            let Value::Array(values) = *value else {
                return Err(Error::UserChainContinuity);
            };
            values
                .into_iter()
                .map(|value| match value {
                    Value::Binary(bytes) if !bytes.is_empty() => Ok(bytes),
                    _ => Err(Error::UserChainContinuity),
                })
                .collect()
        }
        _ => Ok(vec![evidence.to_vec()]),
    }
}

fn append_user_evidence(prior: &[u8], suffix: &[u8]) -> Result<Vec<u8>> {
    let mut segments = user_evidence_segments(prior)?;
    while segments.len() > 1
        && segments
            .last()
            .map(|segment| UserChain::decode(segment).map(|chain| chain.links.is_empty()))
            .transpose()?
            .unwrap_or(false)
    {
        segments.pop();
    }
    segments.push(suffix.to_vec());
    Ok(encode(&Value::Variant(Some((
        b"1".to_vec(),
        Box::new(Value::Array(
            segments.into_iter().map(Value::Binary).collect(),
        )),
    ))))?)
}

fn verify_incremental_user_disclosures(
    chain: &UserChain,
    prior: &VerifiedUserState,
    expected_uid: &EntityId,
    expected_host: &EntityId,
) -> Result<(Vec<u8>, Vec<u8>, u64)> {
    let mut username_commitments = Vec::new();
    let mut device_name_commitments = Vec::new();
    for link in &chain.links {
        for metadata in link.decode_group_change()?.metadata {
            match metadata {
                ChangeMetadata::Username(value) => username_commitments.push(value),
                ChangeMetadata::DeviceName(value) => device_name_commitments.push(value),
                _ => {}
            }
        }
    }
    if username_commitments.len() != chain.usernames.len()
        || device_name_commitments.len() != chain.device_names.len()
    {
        return Err(Error::UserDisclosure);
    }
    for (disclosed, expected) in chain.usernames.iter().zip(username_commitments) {
        let wire = encode(&Value::Array(vec![
            Value::Text(disclosed.name.clone()),
            Value::Unsigned(disclosed.sequence),
        ]))?;
        if commitment(NAME_COMMITMENT_TYPE_ID, &wire, &disclosed.commitment_key) != expected {
            return Err(Error::UserDisclosure);
        }
    }
    for (disclosed, expected) in chain.device_names.iter().zip(device_name_commitments) {
        if disclosed.normalization_version != 0
            || normalize_device_name(&disclosed.display_name).as_deref()
                != Some(disclosed.label.normalized_name.as_slice())
        {
            return Err(Error::UserDisclosure);
        }
        let wire = encode(&Value::Array(vec![
            Value::Unsigned(disclosed.label.device_type.protocol_value()),
            Value::Text(disclosed.label.normalized_name.clone()),
            Value::Unsigned(disclosed.label.serial),
        ]))?;
        if commitment(DEVICE_LABEL_TYPE_ID, &wire, &disclosed.commitment_key) != expected {
            return Err(Error::UserDisclosure);
        }
    }

    let normalized = normalize_username(&chain.username_utf8).ok_or(Error::UserDisclosure)?;
    let (username, username_sequence, name_start) = match chain.usernames.last() {
        Some(last) if last.name == normalized => {
            let start = if normalized == prior.username {
                prior
                    .username_sequence
                    .checked_add(1)
                    .ok_or(Error::UserDisclosure)?
            } else {
                1
            };
            (normalized, last.sequence, start)
        }
        None if normalized == prior.username => (
            prior.username.clone(),
            prior.username_sequence,
            prior
                .username_sequence
                .checked_add(1)
                .ok_or(Error::UserDisclosure)?,
        ),
        _ => return Err(Error::UserDisclosure),
    };
    let path_count =
        usize::try_from(chain.num_username_links).map_err(|_| Error::UserDisclosure)?;
    let expected_count = username_sequence
        .checked_add(2)
        .and_then(|end| end.checked_sub(name_start))
        .and_then(|count| usize::try_from(count).ok())
        .ok_or(Error::UserDisclosure)?;
    if path_count != expected_count || path_count == 0 {
        return Err(Error::UserDisclosure);
    }
    for (index, path) in chain.merkle.paths()[..path_count].iter().enumerate() {
        let sequence = name_start
            .checked_add(u64::try_from(index).map_err(|_| Error::UserDisclosure)?)
            .ok_or(Error::UserDisclosure)?;
        let key = username_merkle_key(&username, expected_host, sequence)?;
        if index + 1 == path_count {
            verify_merkle_path(path, &key, None, &chain.merkle.root().root_node)?;
        } else if index + 2 == path_count {
            verify_merkle_path(
                path,
                &key,
                Some(&username_merkle_leaf(expected_uid)?),
                &chain.merkle.root().root_node,
            )?;
        } else {
            verify_merkle_path_present(path, &key, &chain.merkle.root().root_node)?;
        }
    }
    Ok((username, chain.username_utf8.clone(), username_sequence))
}

fn verify_user_disclosures(
    chain: &UserChain,
    expected_uid: &EntityId,
    expected_host: &EntityId,
) -> Result<(Vec<u8>, Vec<u8>, u64)> {
    let mut username_commitments = Vec::new();
    let mut device_name_commitments = Vec::new();
    for link in &chain.links {
        for metadata in link.decode_group_change()?.metadata {
            match metadata {
                ChangeMetadata::Username(value) => username_commitments.push(value),
                ChangeMetadata::DeviceName(value) => device_name_commitments.push(value),
                _ => {}
            }
        }
    }
    if username_commitments.len() != chain.usernames.len()
        || device_name_commitments.len() != chain.device_names.len()
        || chain.usernames.is_empty()
    {
        return Err(Error::UserDisclosure);
    }
    for (disclosed, expected) in chain.usernames.iter().zip(username_commitments) {
        let wire = encode(&Value::Array(vec![
            Value::Text(disclosed.name.clone()),
            Value::Unsigned(disclosed.sequence),
        ]))?;
        if commitment(NAME_COMMITMENT_TYPE_ID, &wire, &disclosed.commitment_key) != expected {
            return Err(Error::UserDisclosure);
        }
    }
    for (disclosed, expected) in chain.device_names.iter().zip(device_name_commitments) {
        if disclosed.normalization_version != 0
            || normalize_device_name(&disclosed.display_name).as_deref()
                != Some(disclosed.label.normalized_name.as_slice())
        {
            return Err(Error::UserDisclosure);
        }
        let wire = encode(&Value::Array(vec![
            Value::Unsigned(disclosed.label.device_type.protocol_value()),
            Value::Text(disclosed.label.normalized_name.clone()),
            Value::Unsigned(disclosed.label.serial),
        ]))?;
        if commitment(DEVICE_LABEL_TYPE_ID, &wire, &disclosed.commitment_key) != expected {
            return Err(Error::UserDisclosure);
        }
    }

    let normalized = normalize_username(&chain.username_utf8).ok_or(Error::UserDisclosure)?;
    if chain.usernames.last().map(|value| value.name.as_slice()) != Some(normalized.as_slice()) {
        return Err(Error::UserDisclosure);
    }
    let path_count =
        usize::try_from(chain.num_username_links).map_err(|_| Error::UserDisclosure)?;
    let last_sequence = chain
        .usernames
        .last()
        .ok_or(Error::UserDisclosure)?
        .sequence;
    if path_count < 2 || last_sequence.checked_add(1) != Some(chain.num_username_links) {
        return Err(Error::UserDisclosure);
    }
    for (index, path) in chain.merkle.paths()[..path_count].iter().enumerate() {
        let sequence = u64::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .ok_or(Error::UserDisclosure)?;
        let key = username_merkle_key(&normalized, expected_host, sequence)?;
        if index + 1 == path_count {
            verify_merkle_path(path, &key, None, &chain.merkle.root().root_node)?;
        } else if index + 2 == path_count {
            verify_merkle_path(
                path,
                &key,
                Some(&username_merkle_leaf(expected_uid)?),
                &chain.merkle.root().root_node,
            )?;
        } else {
            verify_merkle_path_present(path, &key, &chain.merkle.root().root_node)?;
        }
    }
    Ok((normalized, chain.username_utf8.clone(), last_sequence))
}

/// Applies the v0.1.9 normalized device-label rules. Presentation-level
/// whitespace and smart-punctuation fixing remains a client concern.
pub fn normalize_device_name(input: &[u8]) -> Option<Vec<u8>> {
    let input = std::str::from_utf8(input).ok()?;
    let normalized = flatten_unicode(input)?.to_lowercase().into_bytes();
    let punctuation = |byte: u8| matches!(byte, b'.' | b'_' | b'+' | b'\'' | b'-');
    if normalized.len() > 200
        || normalized.len() < 2
        || !normalized[0].is_ascii_alphanumeric()
        || normalized.last() == Some(&b' ')
        || normalized.windows(2).any(|pair| pair == b"  ")
        || !normalized.iter().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(*byte, b'_' | b' ' | b'.' | b'\'' | b'+' | b'-')
        })
        || normalized.split(|byte| *byte == b' ').any(|token| {
            token == b"+"
                || token == b"-"
                || token
                    .windows(2)
                    .any(|pair| punctuation(pair[0]) && punctuation(pair[1]))
                || token
                    .last()
                    .is_some_and(|byte| matches!(*byte, b'_' | b'\'' | b'.'))
        })
    {
        None
    } else {
        Some(normalized)
    }
}

/// Applies the exact v0.1.9 username normalization and validity rules.
pub fn normalize_username(input: &[u8]) -> Option<Vec<u8>> {
    let input = std::str::from_utf8(input).ok()?;
    let lower = input.to_lowercase();
    let normalized = flatten_unicode(&lower)?
        .bytes()
        .map(|byte| match byte {
            b'.' | b'-' => b'_',
            value => value,
        })
        .collect::<Vec<_>>();
    if !(3..=25).contains(&normalized.len())
        || !normalized
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
        || normalized.first() == Some(&b'_')
        || normalized.last() == Some(&b'_')
        || normalized.windows(2).any(|pair| pair == b"__")
    {
        None
    } else {
        Some(normalized)
    }
}

fn flatten_unicode(input: &str) -> Option<String> {
    let replaced = input
        .chars()
        .map(|character| match character {
            'ą' => 'a',
            'ć' => 'c',
            'ę' => 'e',
            'ł' => 'l',
            'ń' => 'n',
            'ó' => 'o',
            'ś' => 's',
            'ż' | 'ź' => 'z',
            'ø' => 'o',
            'æ' => 'a',
            'ß' => 's',
            other => other,
        })
        .collect::<String>();
    let flattened = replaced
        .nfd()
        .filter(|character| !is_combining_mark(*character))
        .collect::<String>();
    flattened.is_ascii().then_some(flattened)
}

pub(crate) fn username_merkle_key(
    normalized_name: &[u8],
    host: &EntityId,
    sequence: u64,
) -> Result<[u8; 32]> {
    let preimage = encode(&Value::Array(vec![
        Value::Text(normalized_name.to_vec()),
        Value::Binary(host.as_bytes().to_vec()),
    ]))?;
    let mut name_entity = Vec::with_capacity(33);
    name_entity.push(9);
    name_entity.extend_from_slice(&prefixed_hash(NAME_HASH_PREIMAGE_TYPE_ID, &preimage)?);
    let input = encode(&Value::Array(vec![
        Value::Unsigned(1),
        Value::Binary(name_entity),
        Value::Unsigned(sequence),
        Value::Null,
    ]))?;
    prefixed_hash(MERKLE_TREE_RF_INPUT_TYPE_ID, &input)
}

pub(crate) fn username_merkle_leaf(uid: &EntityId) -> Result<[u8; 32]> {
    let value = encode(&Value::Array(vec![
        Value::Unsigned(1),
        Value::Variant(Some((
            b"1".to_vec(),
            Box::new(Value::Binary(uid.as_bytes().to_vec())),
        ))),
    ]))?;
    prefixed_hash(ENTITY_ID_MERKLE_VALUE_TYPE_ID, &value)
}

fn persistent_user_id(verify_key: &EntityId) -> Result<EntityId> {
    let mut bytes = verify_key.as_bytes().to_vec();
    bytes[0] = foks_proto::ENTITY_USER;
    Ok(EntityId::from_bytes(bytes)?)
}

fn verify_eldest_bindings(
    eldest: &UserEldest,
    expected_uid: &EntityId,
    expected_host: &EntityId,
) -> Result<()> {
    if &eldest.uid != expected_uid
        || &eldest.host != expected_host
        || eldest.signer != eldest.member
        || eldest.member != eldest.member_verify_key
        || eldest.member_role != Role::OWNER
        || eldest.member_source_role != Role::NONE
        || eldest.member_scoped_host.is_some()
        || eldest.metadata.len() < 3
        || !matches!(eldest.metadata[0], ChangeMetadata::Username(_))
        || !matches!(eldest.metadata[1], ChangeMetadata::DeviceName(_))
        || !matches!(eldest.metadata[2], ChangeMetadata::Eldest { .. })
    {
        Err(Error::UserBinding)
    } else {
        Ok(())
    }
}

pub(crate) fn find_hepk(hepks: &[Hepk], fingerprint: [u8; 32]) -> Result<Hepk> {
    for hepk in hepks {
        if prefixed_hash(HEPK_TYPE_ID, &hepk.encoded()?)? == fingerprint {
            return Ok(hepk.clone());
        }
    }
    Err(Error::MissingHepk)
}

fn user_merkle_key(uid: &EntityId, seqno: u64, location: Option<&[u8; 32]>) -> Result<[u8; 32]> {
    chain_merkle_key(0, uid, seqno, location)
}

pub(crate) fn chain_merkle_key(
    chain_type: u64,
    entity: &EntityId,
    seqno: u64,
    location: Option<&[u8; 32]>,
) -> Result<[u8; 32]> {
    let encoded = encode(&Value::Array(vec![
        Value::Unsigned(chain_type),
        Value::Binary(entity.as_bytes().to_vec()),
        Value::Unsigned(seqno),
        location.map_or(Value::Null, |value| Value::Binary(value.to_vec())),
    ]))?;
    prefixed_hash(MERKLE_TREE_RF_INPUT_TYPE_ID, &encoded)
}

#[cfg(test)]
mod incremental_tests {
    use super::*;
    use crate::{verify_merkle_advance, verify_public_host};

    const PROBE: &[u8] = include_bytes!(
        "../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
    );
    const USER_CHAIN: &[u8] =
        include_bytes!("../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/user-chain.snowp");
    const USER_ROOT: &[u8] =
        include_bytes!("../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/merkle-root-998.snowp");
    const USER_HISTORY: &[u8] = include_bytes!(
        "../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/merkle-historical-response.snowp"
    );

    fn entity_fixture(bytes: &[u8]) -> EntityId {
        let Value::Binary(bytes) = foks_snowpack::decode(bytes).unwrap() else {
            panic!("entity fixture is not binary");
        };
        EntityId::from_bytes(bytes).unwrap()
    }

    fn suffix_after_eldest(chain: &UserChain) -> Vec<u8> {
        let Value::Array(mut fields) = foks_snowpack::decode(USER_CHAIN).unwrap() else {
            panic!("user chain is not an array");
        };
        let Value::Array(links) = fields[0].clone() else {
            panic!("user links are not an array");
        };
        let Value::Array(locations) = fields[1].clone() else {
            panic!("user locations are not an array");
        };
        let Value::Array(device_names) = fields[4].clone() else {
            panic!("device names are not an array");
        };
        let Value::Array(mut merkle) = fields[3].clone() else {
            panic!("user Merkle evidence is not an array");
        };
        let Value::Array(paths) = merkle[1].clone() else {
            panic!("user Merkle paths are not an array");
        };
        let name_paths = usize::try_from(chain.num_username_links).unwrap();
        let mut suffix_paths = vec![paths[name_paths - 1].clone()];
        suffix_paths.extend_from_slice(&paths[name_paths + 1..]);
        merkle[1] = Value::Array(suffix_paths);
        fields[0] = Value::Array(links[1..].to_vec());
        fields[1] = Value::Array(locations);
        fields[2] = Value::Null;
        fields[3] = Value::Array(merkle);
        fields[4] = Value::Array(device_names[1..].to_vec());
        fields[6] = Value::Unsigned(1);
        encode(&Value::Array(fields)).unwrap()
    }

    #[test]
    fn multi_link_suffix_replays_from_trusted_tail_and_rejects_fork() {
        let chain = UserChain::decode(USER_CHAIN).unwrap();
        let uid = entity_fixture(include_bytes!(
            "../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/uid.snowp"
        ));
        let eldest = chain.links[0].decode_eldest().unwrap();
        let host = eldest.host.clone();
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let advance = verify_merkle_advance(
            public.snapshot.merkle_root(),
            USER_ROOT,
            USER_HISTORY,
            &HostchainTail {
                seqno: public.snapshot.chain_seqno,
                hash: public.snapshot.chain_tail_hash,
            },
        )
        .unwrap();
        let final_state = verify_user_chain(
            USER_CHAIN,
            &uid,
            &host,
            advance.authenticated_roots(),
            &chain.merkle.root().hostchain,
        )
        .unwrap();
        let replay = UserReplayState::from_eldest(
            VerifiedDevice {
                id: eldest.member,
                role: Role::OWNER,
                hepk: find_hepk(&chain.hepks, eldest.member_hepk_fingerprint).unwrap(),
                subkey: eldest.member_subkey,
            },
            VerifiedSharedKey {
                role: Role::OWNER,
                generation: 1,
                verify_key: eldest.puk_verify_key,
                hepk: find_hepk(&chain.hepks, eldest.puk_hepk_fingerprint).unwrap(),
            },
        );
        let (devices, shared_keys, shared_key_history) = replay.into_parts();
        let first_hash =
            prefixed_hash(LINK_OUTER_TYPE_ID, &chain.links[0].encoded().unwrap()).unwrap();
        let prior = VerifiedUserState {
            uid: uid.clone(),
            host: host.clone(),
            chain_tail_hash: first_hash,
            merkle_root_hash: final_state.merkle_root_hash,
            merkle_epoch: final_state.merkle_epoch,
            merkle_root_bytes: final_state.merkle_root_bytes.clone(),
            authenticated_chain_bytes: authenticated_user_chain_bytes(&chain.links[..1]).unwrap(),
            evidence_bytes: USER_CHAIN.to_vec(),
            chain_seqno: 1,
            next_tree_location: chain.locations[0],
            username: final_state.username.clone(),
            username_utf8: final_state.username_utf8.clone(),
            username_sequence: final_state.username_sequence,
            devices,
            shared_keys,
            shared_key_history,
        };
        let suffix = suffix_after_eldest(&chain);
        assert_eq!(UserChain::decode(&suffix).unwrap().links.len(), 2);
        let incremented = verify_user_chain_increment(
            &suffix,
            &prior,
            &uid,
            &host,
            advance.authenticated_roots(),
            &chain.merkle.root().hostchain,
        )
        .unwrap();
        assert_eq!(incremented.chain_seqno, final_state.chain_seqno);
        assert_eq!(incremented.chain_tail_hash, final_state.chain_tail_hash);
        assert_eq!(incremented.devices, final_state.devices);
        assert_eq!(incremented.shared_keys, final_state.shared_keys);

        let mut forked = prior;
        forked.chain_tail_hash[0] ^= 1;
        assert!(matches!(
            verify_user_chain_increment(
                &suffix,
                &forked,
                &uid,
                &host,
                advance.authenticated_roots(),
                &chain.merkle.root().hostchain,
            ),
            Err(Error::UserChainContinuity)
        ));
    }
}
