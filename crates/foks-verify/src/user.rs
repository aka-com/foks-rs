//! User-chain replay, disclosure verification, and sealed user state.

use unicode_normalization::{char::is_combining_mark, UnicodeNormalization as _};

use crate::{
    commitment, encode, prefixed_hash, verify_hostchain_at_tail, verify_merkle_path, verify_typed,
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
    let root_hash = prefixed_hash(MERKLE_ROOT_TYPE_ID, &root_bytes);
    if authenticated_roots.get(&chain.merkle.root().epoch) != Some(&root_hash)
        || &chain.merkle.root().hostchain != trusted_hostchain
    {
        return Err(Error::UntrustedUserRoot);
    }

    let (username, username_utf8, username_sequence) =
        verify_user_disclosures(&chain, expected_uid, expected_host)?;
    let path_offset =
        usize::try_from(chain.num_username_links).map_err(|_| Error::UserMerkleProof)?;

    let mut replay_state = None;
    let mut previous_hash = None;

    for (index, ((link, location), path)) in chain
        .links
        .iter()
        .zip(&chain.locations)
        .zip(&chain.merkle.paths()[path_offset..path_offset + chain.links.len()])
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
        if prefixed_hash(TREE_LOCATION_TYPE_ID, &location_wire) != change.next_location_commitment {
            return Err(Error::UserChainContinuity);
        }
        let location_for_key = index.checked_sub(1).map(|prior| &chain.locations[prior]);
        let merkle_key = user_merkle_key(expected_uid, sequence, location_for_key)?;
        let link_hash = prefixed_hash(LINK_OUTER_TYPE_ID, &link.encoded()?);
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
    let (devices, shared_keys) = replay_state.into_parts();
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
    let evidence = UserChain::decode(persisted.evidence_bytes)?;
    let hostchain = foks_proto::decode_hostchain(trusted_hostchain_bytes)?;
    verify_hostchain_at_tail(&hostchain, &evidence.merkle.root().hostchain)?;
    let verified = verify_user_chain(
        persisted.evidence_bytes,
        &uid,
        &host,
        authenticated_roots,
        &evidence.merkle.root().hostchain,
    )?;
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
            Value::Unsigned(disclosed.label.device_type),
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
        let expected_leaf = if index + 1 == path_count {
            None
        } else {
            Some(username_merkle_leaf(expected_uid)?)
        };
        verify_merkle_path(
            path,
            &key,
            expected_leaf.as_ref(),
            &chain.merkle.root().root_node,
        )?;
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
    name_entity.extend_from_slice(&prefixed_hash(NAME_HASH_PREIMAGE_TYPE_ID, &preimage));
    let input = encode(&Value::Array(vec![
        Value::Unsigned(1),
        Value::Binary(name_entity),
        Value::Unsigned(sequence),
        Value::Null,
    ]))?;
    Ok(prefixed_hash(MERKLE_TREE_RF_INPUT_TYPE_ID, &input))
}

pub(crate) fn username_merkle_leaf(uid: &EntityId) -> Result<[u8; 32]> {
    let value = encode(&Value::Array(vec![
        Value::Unsigned(1),
        Value::Variant(Some((
            b"1".to_vec(),
            Box::new(Value::Binary(uid.as_bytes().to_vec())),
        ))),
    ]))?;
    Ok(prefixed_hash(ENTITY_ID_MERKLE_VALUE_TYPE_ID, &value))
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
    hepks
        .iter()
        .find(|hepk| {
            hepk.encoded()
                .map(|bytes| prefixed_hash(HEPK_TYPE_ID, &bytes) == fingerprint)
                .unwrap_or(false)
        })
        .cloned()
        .ok_or(Error::MissingHepk)
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
    Ok(prefixed_hash(MERKLE_TREE_RF_INPUT_TYPE_ID, &encoded))
}
