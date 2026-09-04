//! Team-chain replay, roster validation, and sealed team state.

use std::collections::BTreeSet;

use crate::{
    append_authenticated_user_chain_bytes, authenticated_user_chain_bytes,
    authenticated_user_chain_link_at, chain_merkle_key, commitment, encode, find_hepk,
    normalize_username, prefixed_hash, username_merkle_key, username_merkle_leaf,
    verify_hostchain_at_tail, verify_merkle_path, verify_merkle_path_present, verify_typed,
    AuthenticatedMerkleRoots, BTreeMap, ChangeMetadata, EntityId, Error, HashSet, Hepk, Result,
    Role, RoleType, TeamChain, UserDeviceProvisionLeaf, UserDeviceSigningBookends, Value,
    VerifiedMerkleAdvance, VerifiedSharedKey, VerifiedUserSharedKey, ENTITY_AD_HOC_TEAM,
    ENTITY_NAMED_TEAM, ENTITY_PTK_VERIFY, ENTITY_USER, LINK_OUTER_TYPE_ID, LINK_OUTER_V1_TYPE_ID,
    MERKLE_ROOT_TYPE_ID, NAME_COMMITMENT_TYPE_ID, TREE_LOCATION_TYPE_ID,
};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct VerifiedTeamMember {
    pub party_id: Vec<u8>,
    pub scoped_host_id: Option<Vec<u8>>,
    pub source_role: Role,
    pub role: Role,
    pub generation: u64,
    pub verify_key: Vec<u8>,
    pub hepk_fingerprint: [u8; 32],
    pub removal_key_commitment: Option<[u8; 32]>,
}

/// Extracts the Merkle epochs referenced by an untrusted team-chain response.
/// The returned epochs carry no authority; callers must prove them from an
/// already authenticated later root before replaying the chain. Network
/// callers bound the encoded chain with the RPC frame limit.
pub fn team_chain_root_epochs(chain_bytes: &[u8]) -> Result<Vec<u64>> {
    let chain = TeamChain::decode(chain_bytes)?;
    let mut epochs = BTreeSet::from([chain.merkle.root().epoch]);
    for link in &chain.links {
        epochs.insert(link.decode_team_group_change()?.root.epoch);
    }
    Ok(epochs.into_iter().collect())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedTeamSnapshot {
    pub(crate) host_id: Vec<u8>,
    pub(crate) team_id: Vec<u8>,
    pub(crate) chain_seqno: u64,
    pub(crate) chain_tail_hash: [u8; 32],
    pub(crate) chain_bytes: Vec<u8>,
    pub(crate) evidence_bytes: Vec<u8>,
    pub(crate) team_name: Vec<u8>,
    pub(crate) team_name_utf8: Vec<u8>,
    pub(crate) team_name_sequence: u64,
    pub(crate) merkle_epoch: u64,
    pub(crate) merkle_root_hash: [u8; 32],
    pub(crate) merkle_root_bytes: Vec<u8>,
    pub(crate) members: Vec<VerifiedTeamMember>,
    pub(crate) shared_keys: Vec<VerifiedUserSharedKey>,
}

impl VerifiedTeamSnapshot {
    pub fn parts(&self) -> VerifiedTeamSnapshotParts<'_> {
        VerifiedTeamSnapshotParts {
            host_id: &self.host_id,
            team_id: &self.team_id,
            chain_seqno: self.chain_seqno,
            chain_tail_hash: self.chain_tail_hash,
            chain_bytes: &self.chain_bytes,
            evidence_bytes: &self.evidence_bytes,
            team_name: &self.team_name,
            team_name_utf8: &self.team_name_utf8,
            team_name_sequence: self.team_name_sequence,
            merkle_epoch: self.merkle_epoch,
            merkle_root_hash: self.merkle_root_hash,
            merkle_root_bytes: &self.merkle_root_bytes,
            members: &self.members,
            shared_keys: &self.shared_keys,
        }
    }
}

#[derive(Clone, Copy)]
pub struct VerifiedTeamSnapshotParts<'a> {
    pub host_id: &'a [u8],
    pub team_id: &'a [u8],
    pub chain_seqno: u64,
    pub chain_tail_hash: [u8; 32],
    pub chain_bytes: &'a [u8],
    pub evidence_bytes: &'a [u8],
    pub team_name: &'a [u8],
    pub team_name_utf8: &'a [u8],
    pub team_name_sequence: u64,
    pub merkle_epoch: u64,
    pub merkle_root_hash: [u8; 32],
    pub merkle_root_bytes: &'a [u8],
    pub members: &'a [VerifiedTeamMember],
    pub shared_keys: &'a [VerifiedUserSharedKey],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedTeamState {
    team: EntityId,
    host: EntityId,
    chain_tail_hash: [u8; 32],
    merkle_root_hash: [u8; 32],
    merkle_epoch: u64,
    merkle_root_bytes: Vec<u8>,
    authenticated_chain_bytes: Vec<u8>,
    evidence_bytes: Vec<u8>,
    chain_seqno: u64,
    next_tree_location: [u8; 32],
    team_name: Vec<u8>,
    team_name_utf8: Vec<u8>,
    team_name_sequence: u64,
    member_load_floor: Role,
    members: Vec<VerifiedTeamMemberState>,
    shared_keys: Vec<VerifiedSharedKey>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedTeamMemberState {
    pub party: EntityId,
    pub scoped_host: Option<EntityId>,
    pub source_role: Role,
    pub role: Role,
    pub generation: u64,
    pub verify_key: EntityId,
    pub hepk_fingerprint: [u8; 32],
    pub removal_key_commitment: Option<[u8; 32]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedTeamFounding {
    pub link_hash: [u8; 32],
    pub member_load_floor: Role,
    pub members: Vec<VerifiedTeamMemberState>,
    pub shared_keys: Vec<VerifiedSharedKey>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedTeamTransition {
    pub link_hash: [u8; 32],
    pub members: Vec<VerifiedTeamMemberState>,
    pub shared_keys: Vec<VerifiedSharedKey>,
    pub introduced_keys: Vec<VerifiedSharedKey>,
}

#[allow(clippy::too_many_arguments)]
pub fn verify_team_transition(
    link: &foks_proto::UserLink,
    hepks: &[Hepk],
    expected_team: &EntityId,
    expected_host: &EntityId,
    expected_sequence: u64,
    expected_previous: [u8; 32],
    expected_root: foks_proto::TreeRoot,
    next_tree_location: [u8; 32],
    current_members: &[VerifiedTeamMemberState],
    current_shared_keys: &[VerifiedSharedKey],
) -> Result<VerifiedTeamTransition> {
    if expected_team.entity_type() == ENTITY_AD_HOC_TEAM {
        return Err(Error::TeamRoster);
    }
    let change = link.decode_team_group_change()?;
    let location_wire = encode(&Value::Binary(next_tree_location.to_vec()))?;
    if change.seqno != expected_sequence
        || change.previous != Some(expected_previous)
        || change.team != *expected_team
        || change.host != *expected_host
        || change.root != expected_root
        || prefixed_hash(TREE_LOCATION_TYPE_ID, &location_wire)? != change.next_location_commitment
    {
        return Err(Error::TeamChainContinuity);
    }
    let mut members = current_members
        .iter()
        .cloned()
        .map(|member| {
            (
                team_member_key(
                    &member.party,
                    member.scoped_host.as_ref(),
                    member.source_role,
                ),
                member,
            )
        })
        .collect::<BTreeMap<_, _>>();
    if members.len() != current_members.len() {
        return Err(Error::TeamRoster);
    }
    let current = current_shared_keys
        .iter()
        .cloned()
        .map(|key| (key.role, key))
        .collect::<BTreeMap<_, _>>();
    if current.len() != current_shared_keys.len() {
        return Err(Error::TeamKeySchedule);
    }
    let introduced = validate_team_shared_keys(&change, hepks, &current, false)?;
    validate_team_rotation_schedule(&change, &members, &current, &introduced)?;
    replay_team_transition(link, &change, &introduced, &mut members)?;
    let mut keys = current;
    for key in &introduced {
        keys.insert(key.role, key.clone());
    }
    Ok(VerifiedTeamTransition {
        link_hash: prefixed_hash(LINK_OUTER_TYPE_ID, &link.encoded()?)?,
        members: members.into_values().collect(),
        shared_keys: keys.into_values().collect(),
        introduced_keys: introduced,
    })
}

/// Verifies a founding team link independently of persistence and of the new
/// Merkle root the server will publish for it. The link must cite the exact
/// currently authenticated root supplied by the caller.
pub fn verify_team_founding(
    link: &foks_proto::UserLink,
    hepks: &[Hepk],
    expected_team: &EntityId,
    expected_host: &EntityId,
    expected_root: foks_proto::TreeRoot,
    next_tree_location: [u8; 32],
) -> Result<VerifiedTeamFounding> {
    let change = link.decode_team_group_change()?;
    let location_wire = encode(&Value::Binary(next_tree_location.to_vec()))?;
    if change.seqno != 1
        || change.previous.is_some()
        || change.team != *expected_team
        || change.host != *expected_host
        || change.root != expected_root
        || prefixed_hash(TREE_LOCATION_TYPE_ID, &location_wire)? != change.next_location_commitment
    {
        return Err(Error::TeamChainContinuity);
    }
    let current = BTreeMap::new();
    let shared_keys = validate_team_shared_keys(&change, hepks, &current, true)?;
    let mut members = BTreeMap::new();
    let member_load_floor =
        verify_team_eldest(link, &change, expected_team, &shared_keys, &mut members)?;
    Ok(VerifiedTeamFounding {
        link_hash: prefixed_hash(LINK_OUTER_TYPE_ID, &link.encoded()?)?,
        member_load_floor,
        members: members.into_values().collect(),
        shared_keys,
    })
}

impl VerifiedTeamState {
    pub fn team(&self) -> &EntityId {
        &self.team
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
    pub fn next_tree_location(&self) -> [u8; 32] {
        self.next_tree_location
    }
    pub fn team_name(&self) -> &[u8] {
        &self.team_name
    }
    pub fn team_name_utf8(&self) -> &[u8] {
        &self.team_name_utf8
    }
    pub fn team_name_sequence(&self) -> u64 {
        self.team_name_sequence
    }
    pub fn member_load_floor(&self) -> Role {
        self.member_load_floor
    }
    pub fn members(&self) -> &[VerifiedTeamMemberState] {
        &self.members
    }
    pub fn shared_keys(&self) -> &[VerifiedSharedKey] {
        &self.shared_keys
    }
    pub fn shared_key(&self, role: Role) -> Option<&VerifiedSharedKey> {
        self.shared_keys.iter().find(|key| key.role == role)
    }
    pub fn shared_key_history(&self) -> Result<Vec<foks_proto::UserSharedKey>> {
        let mut history = Vec::new();
        for sequence in 1..=self.chain_seqno {
            history.extend(self.group_change_at(sequence)?.shared_keys);
        }
        Ok(history)
    }

    /// Checks that `signer` was an authenticated PTK at `epoch` and returns
    /// the team-chain Merkle bookends needed to prove its provisioning and,
    /// for a rotated key, that the generic link preceded the rotation.
    pub fn shared_key_signing_bookends(
        &self,
        signer: &EntityId,
        epoch: u64,
    ) -> Result<Option<UserDeviceSigningBookends>> {
        let mut intervals = Vec::<(
            Role,
            EntityId,
            foks_proto::TreeRoot,
            UserDeviceProvisionLeaf,
            Option<foks_proto::TreeRoot>,
        )>::new();
        let mut prior_location = None;
        let mut sequence = 0_u64;
        for segment in team_evidence_segments(&self.evidence_bytes)? {
            let chain = TeamChain::decode(&segment)?;
            let offset = match chain.locations.len().checked_sub(chain.links.len()) {
                Some(0) if sequence == 0 => 0,
                Some(1) if sequence > 0 && chain.locations.first() == prior_location.as_ref() => 1,
                _ if chain.links.is_empty() => continue,
                _ => return Err(Error::TeamChainContinuity),
            };
            for (index, link) in chain.links.iter().enumerate() {
                sequence = sequence.checked_add(1).ok_or(Error::TeamChainContinuity)?;
                let change = link.decode_team_group_change()?;
                if change.seqno != sequence {
                    return Err(Error::TeamChainContinuity);
                }
                let key = chain_merkle_key(3, &self.team, sequence, prior_location.as_ref())?;
                let value = prefixed_hash(LINK_OUTER_TYPE_ID, &link.encoded()?)?;
                for introduced in &change.shared_keys {
                    if let Some((_, _, _, _, revoke)) =
                        intervals.iter_mut().rev().find(|(role, _, _, _, revoke)| {
                            *role == introduced.role && revoke.is_none()
                        })
                    {
                        *revoke = Some(change.root.clone());
                    }
                    intervals.push((
                        introduced.role,
                        introduced.verify_key.clone(),
                        change.root.clone(),
                        UserDeviceProvisionLeaf { key, value },
                        None,
                    ));
                }
                prior_location = chain.locations.get(index + offset).copied();
            }
        }
        if sequence != self.chain_seqno {
            return Err(Error::TeamChainContinuity);
        }
        for (_, key, provision_root, provision, revoke_root) in intervals {
            if key != *signer || epoch < provision_root.epoch {
                continue;
            }
            if revoke_root.as_ref().is_none_or(|root| root.epoch >= epoch) {
                return Ok(Some(UserDeviceSigningBookends {
                    provision,
                    revoke_root,
                }));
            }
        }
        Ok(None)
    }
    pub fn tree_root(&self) -> foks_proto::TreeRoot {
        foks_proto::TreeRoot {
            epoch: self.merkle_epoch,
            hash: self.merkle_root_hash,
        }
    }

    /// Returns one already-authenticated chain transition by its one-based
    /// sequence number. This is intended for exact mutation reconciliation:
    /// callers can prove a prepared link occupied its expected position even
    /// when the latest chain has advanced further.
    pub fn group_change_at(&self, sequence: u64) -> Result<foks_proto::TeamGroupChange> {
        let index = sequence
            .checked_sub(1)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or(Error::TeamChainContinuity)?;
        authenticated_user_chain_link_at(&self.authenticated_chain_bytes, index)?
            .decode_team_group_change()
            .map_err(Into::into)
    }

    pub fn hard_state_snapshot(&self) -> Result<VerifiedTeamSnapshot> {
        let mut members = self
            .members
            .iter()
            .map(|member| {
                Ok(VerifiedTeamMember {
                    party_id: member.party.as_bytes().to_vec(),
                    scoped_host_id: member
                        .scoped_host
                        .as_ref()
                        .map(|host| host.as_bytes().to_vec()),
                    source_role: member.source_role,
                    role: member.role,
                    generation: member.generation,
                    verify_key: member.verify_key.as_bytes().to_vec(),
                    hepk_fingerprint: member.hepk_fingerprint,
                    removal_key_commitment: member.removal_key_commitment,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        members.sort_unstable();
        let mut shared_keys = self
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
            .collect::<Result<Vec<_>>>()?;
        shared_keys.sort_unstable();
        Ok(VerifiedTeamSnapshot {
            host_id: self.host.as_bytes().to_vec(),
            team_id: self.team.as_bytes().to_vec(),
            chain_seqno: self.chain_seqno,
            chain_tail_hash: self.chain_tail_hash,
            chain_bytes: self.authenticated_chain_bytes.clone(),
            evidence_bytes: self.evidence_bytes.clone(),
            team_name: self.team_name.clone(),
            team_name_utf8: self.team_name_utf8.clone(),
            team_name_sequence: self.team_name_sequence,
            merkle_epoch: self.merkle_epoch,
            merkle_root_hash: self.merkle_root_hash,
            merkle_root_bytes: self.merkle_root_bytes.clone(),
            members,
            shared_keys,
        })
    }
}

/// Replays a FOKS v0.1.9 team chain and authenticates its roster, PTK
/// generations, name history, and every link against independently pinned
/// Merkle roots. The response-wide roster and terminal absence proofs must be
/// anchored at `latest`; historical roots remain valid only for roots
/// cited by individual links.
pub fn verify_team_chain(
    chain_bytes: &[u8],
    expected_team: &EntityId,
    expected_host: &EntityId,
    authenticated_roots: &AuthenticatedMerkleRoots,
    latest: &VerifiedMerkleAdvance,
) -> Result<VerifiedTeamState> {
    verify_team_chain_at_root(
        chain_bytes,
        expected_team,
        expected_host,
        authenticated_roots,
        latest.root(),
    )
}

fn verify_team_chain_at_root(
    chain_bytes: &[u8],
    expected_team: &EntityId,
    expected_host: &EntityId,
    authenticated_roots: &AuthenticatedMerkleRoots,
    expected_root: &foks_proto::MerkleRoot,
) -> Result<VerifiedTeamState> {
    let chain = TeamChain::decode(chain_bytes)?;
    if chain.links.is_empty() || chain.locations.len() != chain.links.len() {
        return Err(Error::TeamChainContinuity);
    }
    if !matches!(
        expected_team.entity_type(),
        ENTITY_NAMED_TEAM | ENTITY_AD_HOC_TEAM
    ) {
        return Err(Error::TeamBinding);
    }
    let root_bytes = chain.merkle.encoded_root()?;
    let root_hash = prefixed_hash(MERKLE_ROOT_TYPE_ID, &root_bytes)?;
    if chain.merkle.root().epoch < expected_root.epoch
        || (chain.merkle.root().epoch == expected_root.epoch
            && chain.merkle.root() != expected_root)
        || authenticated_roots.get(&chain.merkle.root().epoch) != Some(&root_hash)
    {
        return Err(Error::UntrustedUserRoot);
    }
    let (team_name, team_name_utf8, team_name_sequence) =
        verify_team_disclosures(&chain, expected_team, expected_host)?;
    let path_offset =
        usize::try_from(chain.num_team_name_links).map_err(|_| Error::TeamChainContinuity)?;
    // The name-link offset and link count are attacker-controlled fields of the
    // decoded chain; reject a response whose Merkle path slice would fall outside
    // the supplied paths rather than panicking on an out-of-bounds index.
    let path_end = path_offset
        .checked_add(chain.links.len())
        .filter(|end| *end <= chain.merkle.paths().len())
        .ok_or(Error::TeamChainContinuity)?;
    let chain_paths = chain
        .merkle
        .paths()
        .get(path_offset..path_end)
        .ok_or(Error::TeamChainContinuity)?;
    let mut members = BTreeMap::<Vec<u8>, VerifiedTeamMemberState>::new();
    let mut shared_keys = BTreeMap::<Role, VerifiedSharedKey>::new();
    let mut previous_hash = None;
    let mut member_load_floor = Role::member(0);

    for (index, ((link, location), path)) in chain
        .links
        .iter()
        .zip(&chain.locations)
        .zip(chain_paths)
        .enumerate()
    {
        let sequence = u64::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .ok_or(Error::TeamChainContinuity)?;
        let change = link.decode_team_group_change()?;
        if change.seqno != sequence
            || change.previous != previous_hash
            || &change.team != expected_team
            || &change.host != expected_host
            || authenticated_roots.get(&change.root.epoch) != Some(&change.root.hash)
        {
            return Err(Error::TeamChainContinuity);
        }
        let location_wire = encode(&Value::Binary(location.to_vec()))?;
        if prefixed_hash(TREE_LOCATION_TYPE_ID, &location_wire)? != change.next_location_commitment
        {
            return Err(Error::TeamChainContinuity);
        }
        let location_for_key = index.checked_sub(1).map(|prior| &chain.locations[prior]);
        let merkle_key = chain_merkle_key(3, expected_team, sequence, location_for_key)?;
        let link_hash = prefixed_hash(LINK_OUTER_TYPE_ID, &link.encoded()?)?;
        verify_merkle_path(
            path,
            &merkle_key,
            Some(&link_hash),
            &chain.merkle.root().root_node,
        )
        .map_err(|_| Error::TeamChainContinuity)?;

        let introduced =
            validate_team_shared_keys(&change, &chain.hepks, &shared_keys, index == 0)?;
        if index == 0 {
            member_load_floor =
                verify_team_eldest(link, &change, expected_team, &introduced, &mut members)?;
        } else {
            validate_team_rotation_schedule(&change, &members, &shared_keys, &introduced)?;
            replay_team_transition(link, &change, &introduced, &mut members)?;
        }
        for key in introduced {
            shared_keys.insert(key.role, key);
        }
        previous_hash = Some(link_hash);
    }
    let chain_seqno = u64::try_from(chain.links.len()).map_err(|_| Error::TeamChainContinuity)?;
    let next_key = chain_merkle_key(
        3,
        expected_team,
        chain_seqno
            .checked_add(1)
            .ok_or(Error::TeamChainContinuity)?,
        chain.locations.last(),
    )?;
    verify_merkle_path(
        chain
            .merkle
            .paths()
            .last()
            .ok_or(Error::TeamChainContinuity)?,
        &next_key,
        None,
        &chain.merkle.root().root_node,
    )
    .map_err(|_| Error::TeamChainContinuity)?;
    Ok(VerifiedTeamState {
        team: expected_team.clone(),
        host: expected_host.clone(),
        chain_tail_hash: previous_hash.ok_or(Error::TeamChainContinuity)?,
        merkle_root_hash: root_hash,
        merkle_epoch: chain.merkle.root().epoch,
        merkle_root_bytes: root_bytes,
        authenticated_chain_bytes: authenticated_user_chain_bytes(&chain.links)?,
        evidence_bytes: chain_bytes.to_vec(),
        chain_seqno,
        next_tree_location: *chain.locations.last().ok_or(Error::TeamChainContinuity)?,
        team_name,
        team_name_utf8,
        team_name_sequence,
        member_load_floor,
        members: members.into_values().collect(),
        shared_keys: shared_keys.into_values().collect(),
    })
}

/// Verifies and replays only the team links returned after a trusted tail,
/// requiring the response-wide proofs to use `latest`.
pub fn verify_team_chain_increment(
    chain_bytes: &[u8],
    prior: &VerifiedTeamState,
    expected_team: &EntityId,
    expected_host: &EntityId,
    authenticated_roots: &AuthenticatedMerkleRoots,
    latest: &VerifiedMerkleAdvance,
) -> Result<VerifiedTeamState> {
    verify_team_chain_increment_at_root(
        chain_bytes,
        prior,
        expected_team,
        expected_host,
        authenticated_roots,
        latest.root(),
    )
}

fn verify_team_chain_increment_at_root(
    chain_bytes: &[u8],
    prior: &VerifiedTeamState,
    expected_team: &EntityId,
    expected_host: &EntityId,
    authenticated_roots: &AuthenticatedMerkleRoots,
    expected_root: &foks_proto::MerkleRoot,
) -> Result<VerifiedTeamState> {
    if prior.team != *expected_team || prior.host != *expected_host {
        return Err(Error::TeamChainContinuity);
    }
    let chain = TeamChain::decode(chain_bytes)?;
    if chain.locations.len() != chain.links.len().saturating_add(1)
        || chain.locations.first() != Some(&prior.next_tree_location)
    {
        return Err(Error::TeamChainContinuity);
    }
    let root_bytes = chain.merkle.encoded_root()?;
    let root_hash = prefixed_hash(MERKLE_ROOT_TYPE_ID, &root_bytes)?;
    if chain.merkle.root().epoch < expected_root.epoch
        || (chain.merkle.root().epoch == expected_root.epoch
            && chain.merkle.root() != expected_root)
        || authenticated_roots.get(&chain.merkle.root().epoch) != Some(&root_hash)
    {
        return Err(Error::UntrustedUserRoot);
    }
    let (team_name, team_name_utf8, team_name_sequence) =
        verify_incremental_team_disclosures(&chain, prior, expected_team, expected_host)?;
    let path_offset =
        usize::try_from(chain.num_team_name_links).map_err(|_| Error::TeamChainContinuity)?;
    let path_end = path_offset
        .checked_add(chain.links.len())
        .ok_or(Error::TeamChainContinuity)?;
    let chain_paths = chain
        .merkle
        .paths()
        .get(path_offset..path_end)
        .ok_or(Error::TeamChainContinuity)?;
    let mut members = prior
        .members
        .iter()
        .cloned()
        .map(|member| {
            (
                team_member_key(
                    &member.party,
                    member.scoped_host.as_ref(),
                    member.source_role,
                ),
                member,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut shared_keys = prior
        .shared_keys
        .iter()
        .cloned()
        .map(|key| (key.role, key))
        .collect::<BTreeMap<_, _>>();
    let mut previous_hash = prior.chain_tail_hash;
    let start_sequence = prior
        .chain_seqno
        .checked_add(1)
        .ok_or(Error::TeamChainContinuity)?;

    for (index, ((link, locations), path)) in chain
        .links
        .iter()
        .zip(chain.locations.windows(2))
        .zip(chain_paths)
        .enumerate()
    {
        let sequence = start_sequence
            .checked_add(u64::try_from(index).map_err(|_| Error::TeamChainContinuity)?)
            .ok_or(Error::TeamChainContinuity)?;
        let change = link.decode_team_group_change()?;
        if change.seqno != sequence
            || change.previous != Some(previous_hash)
            || &change.team != expected_team
            || &change.host != expected_host
            || authenticated_roots.get(&change.root.epoch) != Some(&change.root.hash)
        {
            return Err(Error::TeamChainContinuity);
        }
        let location_wire = encode(&Value::Binary(locations[1].to_vec()))?;
        if prefixed_hash(TREE_LOCATION_TYPE_ID, &location_wire)? != change.next_location_commitment
        {
            return Err(Error::TeamChainContinuity);
        }
        let merkle_key = chain_merkle_key(3, expected_team, sequence, Some(&locations[0]))?;
        let link_hash = prefixed_hash(LINK_OUTER_TYPE_ID, &link.encoded()?)?;
        verify_merkle_path(
            path,
            &merkle_key,
            Some(&link_hash),
            &chain.merkle.root().root_node,
        )
        .map_err(|_| Error::TeamChainContinuity)?;
        let introduced = validate_team_shared_keys(&change, &chain.hepks, &shared_keys, false)?;
        validate_team_rotation_schedule(&change, &members, &shared_keys, &introduced)?;
        replay_team_transition(link, &change, &introduced, &mut members)?;
        for key in introduced {
            shared_keys.insert(key.role, key);
        }
        previous_hash = link_hash;
    }
    let chain_seqno = prior
        .chain_seqno
        .checked_add(u64::try_from(chain.links.len()).map_err(|_| Error::TeamChainContinuity)?)
        .ok_or(Error::TeamChainContinuity)?;
    let next_tree_location = *chain.locations.last().ok_or(Error::TeamChainContinuity)?;
    let next_key = chain_merkle_key(
        3,
        expected_team,
        chain_seqno
            .checked_add(1)
            .ok_or(Error::TeamChainContinuity)?,
        Some(&next_tree_location),
    )?;
    verify_merkle_path(
        chain
            .merkle
            .paths()
            .last()
            .ok_or(Error::TeamChainContinuity)?,
        &next_key,
        None,
        &chain.merkle.root().root_node,
    )
    .map_err(|_| Error::TeamChainContinuity)?;
    if chain.links.is_empty()
        && root_hash == prior.merkle_root_hash
        && team_name == prior.team_name
        && team_name_sequence == prior.team_name_sequence
    {
        return Ok(prior.clone());
    }
    Ok(VerifiedTeamState {
        team: expected_team.clone(),
        host: expected_host.clone(),
        chain_tail_hash: previous_hash,
        merkle_root_hash: root_hash,
        merkle_epoch: chain.merkle.root().epoch,
        merkle_root_bytes: root_bytes,
        authenticated_chain_bytes: append_authenticated_user_chain_bytes(
            &prior.authenticated_chain_bytes,
            &chain.links,
        )?,
        evidence_bytes: append_team_evidence(&prior.evidence_bytes, chain_bytes)?,
        chain_seqno,
        next_tree_location,
        team_name,
        team_name_utf8,
        team_name_sequence,
        member_load_floor: prior.member_load_floor,
        members: members.into_values().collect(),
        shared_keys: shared_keys.into_values().collect(),
    })
}

pub(crate) fn validate_team_shared_keys(
    change: &foks_proto::TeamGroupChange,
    hepks: &[Hepk],
    current: &BTreeMap<Role, VerifiedSharedKey>,
    eldest: bool,
) -> Result<Vec<VerifiedSharedKey>> {
    let eldest_roles = [
        Role::member(-0x4000),
        Role::member(0),
        Role::ADMIN,
        Role::OWNER,
    ];
    if eldest
        && (change.shared_keys.len() != eldest_roles.len()
            || change
                .shared_keys
                .iter()
                .zip(eldest_roles)
                .any(|(key, role)| key.role != role || key.generation != 1))
    {
        return Err(Error::TeamKeySchedule);
    }
    let mut output = Vec::with_capacity(change.shared_keys.len());
    let mut prior = None;
    for key in &change.shared_keys {
        let expected_generation = current
            .get(&key.role)
            .map_or(1, |old| old.generation.saturating_add(1));
        if key.verify_key.entity_type() != ENTITY_PTK_VERIFY
            || key.generation != expected_generation
            || prior.is_some_and(|prior| prior >= key.role)
        {
            return Err(Error::TeamKeySchedule);
        }
        output.push(VerifiedSharedKey {
            role: key.role,
            generation: key.generation,
            verify_key: key.verify_key.clone(),
            hepk: find_hepk(hepks, key.hepk_fingerprint)?,
        });
        prior = Some(key.role);
    }
    Ok(output)
}

pub(crate) fn team_member_key(
    party: &EntityId,
    scoped_host: Option<&EntityId>,
    source_role: Role,
) -> Vec<u8> {
    let mut key = party.as_bytes().to_vec();
    if let Some(host) = scoped_host {
        key.extend_from_slice(host.as_bytes());
    }
    key.extend_from_slice(&source_role.protocol_value().to_be_bytes());
    key.extend_from_slice(&source_role.visibility().unwrap_or_default().to_be_bytes());
    key
}

pub(crate) fn verified_team_member(
    change: &foks_proto::TeamMemberChange,
) -> Result<VerifiedTeamMemberState> {
    let keys = change.keys.as_ref().ok_or(Error::TeamRoster)?;
    if change.role == Role::NONE
        || change.source_role == Role::NONE
        || keys.generation == 0
        || !matches!(
            keys.verify_key.entity_type(),
            foks_proto::ENTITY_PUK_VERIFY | ENTITY_PTK_VERIFY
        )
    {
        return Err(Error::TeamRoster);
    }
    let source_matches_key = match keys.verify_key.entity_type() {
        foks_proto::ENTITY_PUK_VERIFY => change.party.entity_type() == ENTITY_USER,
        ENTITY_PTK_VERIFY => {
            matches!(
                change.party.entity_type(),
                ENTITY_NAMED_TEAM | ENTITY_AD_HOC_TEAM
            )
        }
        _ => false,
    };
    if !source_matches_key {
        return Err(Error::TeamRoster);
    }
    Ok(VerifiedTeamMemberState {
        party: change.party.clone(),
        scoped_host: change.scoped_host.clone(),
        source_role: change.source_role,
        role: change.role,
        generation: keys.generation,
        verify_key: keys.verify_key.clone(),
        hepk_fingerprint: keys.hepk_fingerprint,
        removal_key_commitment: keys.removal_key_commitment,
    })
}

fn verify_team_eldest(
    link: &foks_proto::UserLink,
    change: &foks_proto::TeamGroupChange,
    expected_team: &EntityId,
    introduced: &[VerifiedSharedKey],
    members: &mut BTreeMap<Vec<u8>, VerifiedTeamMemberState>,
) -> Result<Role> {
    let member_load_floor = team_eldest_member_load_floor(change, expected_team)?;
    let named = expected_team.entity_type() == ENTITY_NAMED_TEAM;
    if change.seqno != 1 || change.previous.is_some() || (named && change.changes.len() != 1) {
        return Err(Error::TeamBinding);
    }
    let admin = introduced
        .iter()
        .find(|key| key.role == Role::ADMIN)
        .ok_or(Error::TeamBinding)?;
    let mut persistent = admin.verify_key.as_bytes().to_vec();
    persistent[0] = expected_team.entity_type();
    if EntityId::from_bytes(persistent)? != *expected_team {
        return Err(Error::TeamBinding);
    }
    let mut signer = None;
    for member in &change.changes {
        if named
            && member
                .keys
                .as_ref()
                .is_none_or(|keys| keys.removal_key_commitment.is_none())
        {
            return Err(Error::TeamRoster);
        }
        if !named
            && (member.scoped_host.is_some()
                || member.party.entity_type() != ENTITY_USER
                || member
                    .keys
                    .as_ref()
                    .is_some_and(|keys| keys.removal_key_commitment.is_some()))
        {
            return Err(Error::TeamRoster);
        }
        let key = team_member_key(
            &member.party,
            member.scoped_host.as_ref(),
            member.source_role,
        );
        if members.contains_key(&key) {
            return Err(Error::TeamRoster);
        }
        let verified = verified_team_member(member)?;
        if member.party == change.signer_owner.party
            && member.source_role == change.signer_owner.source_role
        {
            if member.role != Role::OWNER
                || verified.verify_key.as_bytes()[1..] != change.signer.as_bytes()[1..]
            {
                return Err(Error::TeamSigner);
            }
            signer = Some(change.signer.clone());
        }
        members.insert(key, verified);
    }
    let signer = signer.ok_or(Error::TeamSigner)?;
    verify_team_signature_stack(link, introduced, &signer)?;
    Ok(member_load_floor)
}

fn team_eldest_member_load_floor(
    change: &foks_proto::TeamGroupChange,
    expected_team: &EntityId,
) -> Result<Role> {
    let named = expected_team.entity_type() == ENTITY_NAMED_TEAM;
    let offset = usize::from(named);
    let minimum = offset + 2;
    if change.metadata.len() < minimum
        || (named && !matches!(change.metadata[0], ChangeMetadata::TeamName(_)))
        || !matches!(change.metadata[offset], ChangeMetadata::Eldest { .. })
        || !matches!(
            change.metadata[offset + 1],
            ChangeMetadata::TeamIndexRange(_)
        )
    {
        return Err(Error::TeamBinding);
    }
    match change.metadata.get(offset + 2) {
        None => Ok(Role::member(0)),
        Some(ChangeMetadata::MemberLoadFloor(role)) => Ok(*role),
        Some(_) => Err(Error::TeamBinding),
    }
}

fn replay_team_transition(
    link: &foks_proto::UserLink,
    change: &foks_proto::TeamGroupChange,
    introduced: &[VerifiedSharedKey],
    members: &mut BTreeMap<Vec<u8>, VerifiedTeamMemberState>,
) -> Result<()> {
    if change.team.entity_type() == ENTITY_AD_HOC_TEAM {
        return Err(Error::TeamRoster);
    }
    let mut matching = members
        .values()
        .filter(|member| {
            member.party == change.signer_owner.party
                && member.source_role == change.signer_owner.source_role
        })
        .cloned();
    let signer = matching.next().ok_or(Error::TeamSigner)?;
    if matching.next().is_some()
        || signer.verify_key.as_bytes()[1..] != change.signer.as_bytes()[1..]
        || !matches!(signer.role.kind(), RoleType::Admin | RoleType::Owner)
        || signer.scoped_host.is_some()
    {
        return Err(Error::TeamSigner);
    }
    let team_name_count = change
        .metadata
        .iter()
        .filter(|metadata| matches!(metadata, ChangeMetadata::TeamName(_)))
        .count();
    let index_range_count = change
        .metadata
        .iter()
        .filter(|metadata| matches!(metadata, ChangeMetadata::TeamIndexRange(_)))
        .count();
    if team_name_count > 1
        || index_range_count > 1
        || change.metadata.iter().any(|metadata| {
            !matches!(
                metadata,
                ChangeMetadata::TeamName(_) | ChangeMetadata::TeamIndexRange(_)
            )
        })
    {
        return Err(Error::TeamBinding);
    }
    verify_team_signature_stack(link, introduced, &change.signer)?;
    let mut seen = HashSet::new();
    for member in &change.changes {
        let key = team_member_key(
            &member.party,
            member.scoped_host.as_ref(),
            member.source_role,
        );
        if !seen.insert(key.clone()) {
            return Err(Error::TeamRoster);
        }
        let prior = members.get(&key);
        if member.role == Role::NONE {
            if member.keys.is_some() || prior.is_none() {
                return Err(Error::TeamRoster);
            }
            if signer.role < prior.expect("checked").role {
                return Err(Error::TeamRoster);
            }
            members.remove(&key);
            continue;
        }
        let mut verified = verified_team_member(member)?;
        if prior.is_none() && verified.removal_key_commitment.is_none() {
            return Err(Error::TeamRoster);
        }
        if let Some(old) = prior {
            if verified
                .removal_key_commitment
                .is_some_and(|commitment| Some(commitment) != old.removal_key_commitment)
            {
                return Err(Error::TeamRoster);
            }
            verified.removal_key_commitment = old.removal_key_commitment;
        }
        if signer.role < verified.role
            || prior.is_some_and(|old| signer.role < old.role)
            || prior.is_some_and(|old| verified.generation < old.generation)
            || prior.is_some_and(|old| {
                old.role == verified.role && old.generation == verified.generation
            })
            || (verified.role.kind() == RoleType::Admin || verified.role.kind() == RoleType::Owner)
                && verified.scoped_host.is_some()
        {
            return Err(Error::TeamRoster);
        }
        members.insert(key, verified);
    }
    if !members
        .values()
        .any(|member| member.role.kind() == RoleType::Owner)
    {
        return Err(Error::TeamRoster);
    }
    Ok(())
}

pub(crate) fn validate_team_rotation_schedule(
    change: &foks_proto::TeamGroupChange,
    members: &BTreeMap<Vec<u8>, VerifiedTeamMemberState>,
    current: &BTreeMap<Role, VerifiedSharedKey>,
    introduced: &[VerifiedSharedKey],
) -> Result<()> {
    let mut required = std::collections::BTreeSet::new();
    let mut seen = HashSet::new();
    for member in &change.changes {
        let key = team_member_key(
            &member.party,
            member.scoped_host.as_ref(),
            member.source_role,
        );
        if !seen.insert(key.clone()) {
            return Err(Error::TeamRoster);
        }
        let old = members.get(&key);
        if member.role != Role::NONE && !current.contains_key(&member.role) {
            required.insert(member.role);
        }
        if let Some(old) = old {
            let new_generation = member.keys.as_ref().map_or(0, |keys| keys.generation);
            let downgrade = old.role > member.role;
            let generation_advance = old.role == member.role && old.generation < new_generation;
            if downgrade || generation_advance {
                for role in current.keys().copied() {
                    let above_new_floor = if old.generation == new_generation {
                        role > member.role
                    } else {
                        true
                    };
                    if role <= old.role && above_new_floor {
                        required.insert(role);
                    }
                }
            }
        }
    }
    let actual = introduced.iter().map(|key| key.role).collect::<Vec<_>>();
    if actual != required.into_iter().collect::<Vec<_>>() {
        return Err(Error::TeamKeySchedule);
    }
    Ok(())
}

fn verify_team_signature_stack(
    link: &foks_proto::UserLink,
    introduced: &[VerifiedSharedKey],
    signer: &EntityId,
) -> Result<()> {
    if link.signatures().len() != introduced.len() + 1 {
        return Err(Error::TeamSignatureStack);
    }
    for (index, key) in introduced.iter().enumerate() {
        verify_typed(
            &key.verify_key,
            &link.signatures()[index],
            LINK_OUTER_V1_TYPE_ID,
            &link.signing_bytes(index)?,
        )
        .map_err(|_| Error::TeamSignatureStack)?;
    }
    let index = introduced.len();
    verify_typed(
        signer,
        &link.signatures()[index],
        LINK_OUTER_V1_TYPE_ID,
        &link.signing_bytes(index)?,
    )
    .map_err(|_| Error::TeamSignatureStack)
}

fn verify_team_disclosures(
    chain: &TeamChain,
    team: &EntityId,
    host: &EntityId,
) -> Result<(Vec<u8>, Vec<u8>, u64)> {
    if team.entity_type() == ENTITY_AD_HOC_TEAM {
        verify_adhoc_team_name_paths(chain, host)?;
        return Ok((b"-".to_vec(), b"-".to_vec(), 0));
    }
    let commitments = chain
        .links
        .iter()
        .flat_map(|link| link.decode_team_group_change().into_iter())
        .flat_map(|change| change.metadata)
        .filter_map(|metadata| match metadata {
            ChangeMetadata::TeamName(value) => Some(value),
            _ => None,
        })
        .collect::<Vec<_>>();
    if commitments.len() != chain.team_names.len() || chain.team_names.is_empty() {
        return Err(Error::TeamDisclosure);
    }
    for (disclosed, expected) in chain.team_names.iter().zip(commitments) {
        let wire = encode(&Value::Array(vec![
            Value::Text(disclosed.name.clone()),
            Value::Unsigned(disclosed.sequence),
        ]))?;
        if commitment(NAME_COMMITMENT_TYPE_ID, &wire, &disclosed.commitment_key) != expected {
            return Err(Error::TeamDisclosure);
        }
    }
    let normalized = normalize_username(&chain.team_name_utf8).ok_or(Error::TeamDisclosure)?;
    let last = chain.team_names.last().ok_or(Error::TeamDisclosure)?;
    if last.name != normalized
        || chain.num_team_name_links < 2
        || last.sequence.checked_add(1) != Some(chain.num_team_name_links)
    {
        return Err(Error::TeamDisclosure);
    }
    let path_count =
        usize::try_from(chain.num_team_name_links).map_err(|_| Error::TeamDisclosure)?;
    let name_paths = chain
        .merkle
        .paths()
        .get(..path_count)
        .ok_or(Error::TeamDisclosure)?;
    for (index, path) in name_paths.iter().enumerate() {
        let sequence = u64::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .ok_or(Error::TeamDisclosure)?;
        let key = username_merkle_key(&normalized, host, sequence)?;
        let proof = if index + 1 == path_count {
            verify_merkle_path(path, &key, None, &chain.merkle.root().root_node)
        } else if index + 2 == path_count {
            verify_merkle_path(
                path,
                &key,
                Some(&username_merkle_leaf(team)?),
                &chain.merkle.root().root_node,
            )
        } else {
            verify_merkle_path_present(path, &key, &chain.merkle.root().root_node)
        };
        proof.map_err(|_| Error::TeamDisclosure)?;
    }
    Ok((normalized, chain.team_name_utf8.clone(), last.sequence))
}

fn team_evidence_segments(evidence: &[u8]) -> Result<Vec<Vec<u8>>> {
    match foks_snowpack::decode(evidence)? {
        Value::Variant(Some((tag, value))) if tag == b"1" => {
            let Value::Array(values) = *value else {
                return Err(Error::TeamChainContinuity);
            };
            values
                .into_iter()
                .map(|value| match value {
                    Value::Binary(bytes) if !bytes.is_empty() => Ok(bytes),
                    _ => Err(Error::TeamChainContinuity),
                })
                .collect()
        }
        _ => Ok(vec![evidence.to_vec()]),
    }
}

fn append_team_evidence(prior: &[u8], suffix: &[u8]) -> Result<Vec<u8>> {
    let mut segments = team_evidence_segments(prior)?;
    while segments.len() > 1
        && segments
            .last()
            .map(|segment| TeamChain::decode(segment).map(|chain| chain.links.is_empty()))
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

fn verify_incremental_team_disclosures(
    chain: &TeamChain,
    prior: &VerifiedTeamState,
    team: &EntityId,
    host: &EntityId,
) -> Result<(Vec<u8>, Vec<u8>, u64)> {
    if team.entity_type() == ENTITY_AD_HOC_TEAM {
        if prior.team_name != b"-" || prior.team_name_sequence != 0 {
            return Err(Error::TeamDisclosure);
        }
        verify_adhoc_team_name_paths(chain, host)?;
        return Ok((b"-".to_vec(), b"-".to_vec(), 0));
    }
    let commitments = chain
        .links
        .iter()
        .flat_map(|link| link.decode_team_group_change().into_iter())
        .flat_map(|change| change.metadata)
        .filter_map(|metadata| match metadata {
            ChangeMetadata::TeamName(value) => Some(value),
            _ => None,
        })
        .collect::<Vec<_>>();
    if commitments.len() != chain.team_names.len() {
        return Err(Error::TeamDisclosure);
    }
    for (disclosed, expected) in chain.team_names.iter().zip(commitments) {
        let wire = encode(&Value::Array(vec![
            Value::Text(disclosed.name.clone()),
            Value::Unsigned(disclosed.sequence),
        ]))?;
        if commitment(NAME_COMMITMENT_TYPE_ID, &wire, &disclosed.commitment_key) != expected {
            return Err(Error::TeamDisclosure);
        }
    }
    let normalized = normalize_username(&chain.team_name_utf8).ok_or(Error::TeamDisclosure)?;
    let (team_name, team_name_sequence, name_start) = match chain.team_names.last() {
        Some(last) if last.name == normalized => {
            let start = if normalized == prior.team_name {
                prior
                    .team_name_sequence
                    .checked_add(1)
                    .ok_or(Error::TeamDisclosure)?
            } else {
                1
            };
            (normalized, last.sequence, start)
        }
        None if normalized == prior.team_name => (
            prior.team_name.clone(),
            prior.team_name_sequence,
            prior
                .team_name_sequence
                .checked_add(1)
                .ok_or(Error::TeamDisclosure)?,
        ),
        _ => return Err(Error::TeamDisclosure),
    };
    let path_count =
        usize::try_from(chain.num_team_name_links).map_err(|_| Error::TeamDisclosure)?;
    let expected_count = team_name_sequence
        .checked_add(2)
        .and_then(|end| end.checked_sub(name_start))
        .and_then(|count| usize::try_from(count).ok())
        .ok_or(Error::TeamDisclosure)?;
    if path_count != expected_count || path_count == 0 {
        return Err(Error::TeamDisclosure);
    }
    let name_paths = chain
        .merkle
        .paths()
        .get(..path_count)
        .ok_or(Error::TeamDisclosure)?;
    for (index, path) in name_paths.iter().enumerate() {
        let sequence = name_start
            .checked_add(u64::try_from(index).map_err(|_| Error::TeamDisclosure)?)
            .ok_or(Error::TeamDisclosure)?;
        let key = username_merkle_key(&team_name, host, sequence)?;
        let proof = if index + 1 == path_count {
            verify_merkle_path(path, &key, None, &chain.merkle.root().root_node)
        } else if index + 2 == path_count {
            verify_merkle_path(
                path,
                &key,
                Some(&username_merkle_leaf(team)?),
                &chain.merkle.root().root_node,
            )
        } else {
            verify_merkle_path_present(path, &key, &chain.merkle.root().root_node)
        };
        proof.map_err(|_| Error::TeamDisclosure)?;
    }
    Ok((team_name, chain.team_name_utf8.clone(), team_name_sequence))
}

fn verify_adhoc_team_name_paths(chain: &TeamChain, host: &EntityId) -> Result<()> {
    // v0.1.9 stores one host-wide `-` row so ad-hoc teams can use the ordinary
    // loader. It is not assigned to any individual team and therefore has no
    // name-tree leaves. The server still returns the first slot and terminal
    // slot as two absence proofs; Go's team loader skips entity-name binding
    // but counts both paths before the team-chain proofs.
    if !chain.team_names.is_empty()
        || chain.num_team_name_links != 2
        || chain.team_name_utf8 != b"-"
    {
        return Err(Error::TeamDisclosure);
    }
    let name_paths = chain.merkle.paths().get(..2).ok_or(Error::TeamDisclosure)?;
    for (index, path) in name_paths.iter().enumerate() {
        let sequence = u64::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .ok_or(Error::TeamDisclosure)?;
        let key = username_merkle_key(b"-", host, sequence)?;
        verify_merkle_path(path, &key, None, &chain.merkle.root().root_node)
            .map_err(|_| Error::TeamDisclosure)?;
    }
    Ok(())
}

/// Replays persisted team-chain evidence before recreating a sealed team
/// capability. Parsed SQLite columns are never trusted independently.
pub fn restore_verified_team(
    persisted: VerifiedTeamSnapshotParts<'_>,
    authenticated_roots: &AuthenticatedMerkleRoots,
    trusted_hostchain_bytes: &[u8],
) -> Result<VerifiedTeamState> {
    let team = EntityId::from_bytes(persisted.team_id.to_vec())?;
    let host = EntityId::from_bytes(persisted.host_id.to_vec())?;
    let hostchain = foks_proto::decode_hostchain(trusted_hostchain_bytes)?;
    let segments = team_evidence_segments(persisted.evidence_bytes)?;
    let last_segment = segments
        .len()
        .checked_sub(1)
        .ok_or(Error::PersistedTeamEvidence)?;
    let mut segments = segments.into_iter();
    let first = segments.next().ok_or(Error::PersistedTeamEvidence)?;
    let first_chain = TeamChain::decode(&first)?;
    verify_hostchain_at_tail(&hostchain, &first_chain.merkle.root().hostchain)?;
    let persisted_root = foks_proto::MerkleRoot::decode(persisted.merkle_root_bytes)?;
    let mut verified = verify_team_chain_at_root(
        &first,
        &team,
        &host,
        authenticated_roots,
        if last_segment == 0 {
            &persisted_root
        } else {
            first_chain.merkle.root()
        },
    )?;
    for (index, segment) in segments.enumerate() {
        let chain = TeamChain::decode(&segment)?;
        verify_hostchain_at_tail(&hostchain, &chain.merkle.root().hostchain)?;
        verified = verify_team_chain_increment_at_root(
            &segment,
            &verified,
            &team,
            &host,
            authenticated_roots,
            if index.checked_add(1) == Some(last_segment) {
                &persisted_root
            } else {
                chain.merkle.root()
            },
        )?;
    }
    let snapshot = verified.hard_state_snapshot()?;
    let reproduced = snapshot.parts();
    if reproduced.host_id != persisted.host_id
        || reproduced.team_id != persisted.team_id
        || reproduced.chain_seqno != persisted.chain_seqno
        || reproduced.chain_tail_hash != persisted.chain_tail_hash
        || reproduced.chain_bytes != persisted.chain_bytes
        || reproduced.evidence_bytes != persisted.evidence_bytes
        || reproduced.team_name != persisted.team_name
        || reproduced.team_name_utf8 != persisted.team_name_utf8
        || reproduced.team_name_sequence != persisted.team_name_sequence
        || reproduced.merkle_epoch != persisted.merkle_epoch
        || reproduced.merkle_root_hash != persisted.merkle_root_hash
        || reproduced.merkle_root_bytes != persisted.merkle_root_bytes
        || reproduced.members != persisted.members
        || reproduced.shared_keys != persisted.shared_keys
    {
        return Err(Error::PersistedTeamEvidence);
    }
    Ok(verified)
}

#[cfg(test)]
mod transition_tests {
    use super::{
        replay_team_transition, team_eldest_member_load_floor, team_member_key,
        validate_team_rotation_schedule, verified_team_member, BTreeMap, EntityId, Error, Role,
        VerifiedSharedKey,
    };
    use foks_crypto::derive_shared_public;
    use foks_proto::{ChangeMetadata, SecretSeed, UserLink, ENTITY_PTK_VERIFY};

    const MUTATION_DIR: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9/user-mutations";

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{MUTATION_DIR}/{name}")).unwrap()
    }

    #[test]
    fn historical_and_evolved_team_eldest_metadata_preserves_the_load_floor() {
        let link = UserLink::decode(&fixture("named-team-link.snowp")).unwrap();
        let change = link.decode_team_group_change().unwrap();
        assert_eq!(
            team_eldest_member_load_floor(&change, &change.team).unwrap(),
            Role::member(0)
        );

        let mut historical = change.clone();
        historical.metadata.truncate(3);
        assert_eq!(
            team_eldest_member_load_floor(&historical, &historical.team).unwrap(),
            Role::member(0)
        );

        let mut nondefault = change.clone();
        nondefault.metadata[3] = ChangeMetadata::MemberLoadFloor(Role::ADMIN);
        nondefault
            .metadata
            .push(ChangeMetadata::DeviceName([7; 32]));
        assert_eq!(
            team_eldest_member_load_floor(&nondefault, &nondefault.team).unwrap(),
            Role::ADMIN
        );

        historical
            .metadata
            .push(ChangeMetadata::DeviceName([8; 32]));
        assert!(matches!(
            team_eldest_member_load_floor(&historical, &historical.team),
            Err(Error::TeamBinding)
        ));
    }

    #[test]
    fn official_addition_replays_and_retains_removal_commitment() {
        let eldest = UserLink::decode(&fixture("named-team-link.snowp")).unwrap();
        let eldest_change = eldest.decode_team_group_change().unwrap();
        let owner = verified_team_member(&eldest_change.changes[0]).unwrap();
        let owner_key =
            team_member_key(&owner.party, owner.scoped_host.as_ref(), owner.source_role);
        let mut members = BTreeMap::from([(owner_key, owner)]);
        let addition = UserLink::decode(&fixture("add-member-link.snowp")).unwrap();
        let addition_change = addition.decode_team_group_change().unwrap();
        replay_team_transition(
            &addition,
            &addition_change,
            &Vec::<VerifiedSharedKey>::new(),
            &mut members,
        )
        .unwrap();
        let target = &addition_change.changes[0];
        let key = team_member_key(&target.party, None, Role::OWNER);
        assert_eq!(
            members.get(&key).unwrap().removal_key_commitment,
            target.keys.as_ref().unwrap().removal_key_commitment
        );
    }

    #[test]
    fn later_adhoc_edits_are_rejected_before_signature_use() {
        let addition = UserLink::decode(&fixture("add-member-link.snowp")).unwrap();
        let mut change = addition.decode_team_group_change().unwrap();
        let mut team = change.team.as_bytes().to_vec();
        team[0] = foks_proto::ENTITY_AD_HOC_TEAM;
        change.team = EntityId::from_bytes(team).unwrap();
        assert!(matches!(
            replay_team_transition(
                &addition,
                &change,
                &Vec::<VerifiedSharedKey>::new(),
                &mut BTreeMap::new(),
            ),
            Err(Error::TeamRoster)
        ));
    }

    #[test]
    fn official_removal_rotates_exactly_the_exposed_ptks() {
        let eldest = UserLink::decode(&fixture("named-team-link.snowp")).unwrap();
        let eldest_change = eldest.decode_team_group_change().unwrap();
        let owner = verified_team_member(&eldest_change.changes[0]).unwrap();
        let owner_key =
            team_member_key(&owner.party, owner.scoped_host.as_ref(), owner.source_role);
        let mut members = BTreeMap::from([(owner_key, owner)]);
        let addition = UserLink::decode(&fixture("add-member-link.snowp")).unwrap();
        let addition_change = addition.decode_team_group_change().unwrap();
        replay_team_transition(
            &addition,
            &addition_change,
            &Vec::<VerifiedSharedKey>::new(),
            &mut members,
        )
        .unwrap();

        let old_seed_names = [
            "adhoc-ptk-member-min-seed.bin",
            "adhoc-ptk-member-seed.bin",
            "adhoc-ptk-admin-seed.bin",
            "adhoc-ptk-owner-seed.bin",
        ];
        let mut current = BTreeMap::new();
        for (wire, name) in eldest_change.shared_keys.iter().zip(old_seed_names) {
            let seed = SecretSeed::new(fixture(name).try_into().unwrap());
            let public = derive_shared_public(&seed, ENTITY_PTK_VERIFY).unwrap();
            current.insert(
                wire.role,
                VerifiedSharedKey {
                    role: wire.role,
                    generation: wire.generation,
                    verify_key: public.verify_key,
                    hepk: public.hepk,
                },
            );
        }
        let rotation = UserLink::decode(&fixture("remove-member-link.snowp")).unwrap();
        let rotation_change = rotation.decode_team_group_change().unwrap();
        let new_seed_names = [
            "remove-member-ptk-member-min-seed.bin",
            "remove-member-ptk-member-seed.bin",
        ];
        let introduced = rotation_change
            .shared_keys
            .iter()
            .zip(new_seed_names)
            .map(|(wire, name)| {
                let seed = SecretSeed::new(fixture(name).try_into().unwrap());
                let public = derive_shared_public(&seed, ENTITY_PTK_VERIFY).unwrap();
                VerifiedSharedKey {
                    role: wire.role,
                    generation: wire.generation,
                    verify_key: public.verify_key,
                    hepk: public.hepk,
                }
            })
            .collect::<Vec<_>>();
        validate_team_rotation_schedule(&rotation_change, &members, &current, &introduced).unwrap();
        assert!(matches!(
            validate_team_rotation_schedule(&rotation_change, &members, &current, &introduced[..1],),
            Err(Error::TeamKeySchedule)
        ));
        replay_team_transition(&rotation, &rotation_change, &introduced, &mut members).unwrap();
        assert_eq!(members.len(), 1);
    }

    #[test]
    fn official_demotion_rotates_only_lost_visibility_and_retains_commitment() {
        let eldest = UserLink::decode(&fixture("named-team-link.snowp")).unwrap();
        let eldest_change = eldest.decode_team_group_change().unwrap();
        let owner = verified_team_member(&eldest_change.changes[0]).unwrap();
        let owner_key =
            team_member_key(&owner.party, owner.scoped_host.as_ref(), owner.source_role);
        let mut members = BTreeMap::from([(owner_key, owner)]);
        let addition = UserLink::decode(&fixture("add-member-link.snowp")).unwrap();
        let addition_change = addition.decode_team_group_change().unwrap();
        replay_team_transition(
            &addition,
            &addition_change,
            &Vec::<VerifiedSharedKey>::new(),
            &mut members,
        )
        .unwrap();

        let mut current = BTreeMap::new();
        for (wire, name) in eldest_change.shared_keys.iter().zip([
            "adhoc-ptk-member-min-seed.bin",
            "adhoc-ptk-member-seed.bin",
            "adhoc-ptk-admin-seed.bin",
            "adhoc-ptk-owner-seed.bin",
        ]) {
            let public = derive_shared_public(
                &SecretSeed::new(fixture(name).try_into().unwrap()),
                ENTITY_PTK_VERIFY,
            )
            .unwrap();
            current.insert(
                wire.role,
                VerifiedSharedKey {
                    role: wire.role,
                    generation: wire.generation,
                    verify_key: public.verify_key,
                    hepk: public.hepk,
                },
            );
        }
        let demotion = UserLink::decode(&fixture("demote-member-link.snowp")).unwrap();
        let change = demotion.decode_team_group_change().unwrap();
        let public = derive_shared_public(
            &SecretSeed::new(fixture("demote-member-ptk-seed.bin").try_into().unwrap()),
            ENTITY_PTK_VERIFY,
        )
        .unwrap();
        let introduced = [VerifiedSharedKey {
            role: change.shared_keys[0].role,
            generation: change.shared_keys[0].generation,
            verify_key: public.verify_key,
            hepk: public.hepk,
        }];
        validate_team_rotation_schedule(&change, &members, &current, &introduced).unwrap();
        let commitment = addition_change.changes[0]
            .keys
            .as_ref()
            .unwrap()
            .removal_key_commitment;
        replay_team_transition(&demotion, &change, &introduced, &mut members).unwrap();
        let changed = &change.changes[0];
        let key = team_member_key(&changed.party, None, changed.source_role);
        assert_eq!(members[&key].role, Role::member(-0x4000));
        assert_eq!(members[&key].removal_key_commitment, commitment);
    }
}
