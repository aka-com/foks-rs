//! Authenticated Merkle advancement and persisted-evidence restoration.

use crate::{
    encode, prefixed_hash, verify_hostchain, verify_hostchain_link, verify_with_delegated_blob_key,
    BTreeMap, Error, HashSet, HistoricalMerkleRoots, HostchainLink, HostchainState, HostchainTail,
    MerkleRoot, Result, SignedBlob, Value, ENTITY_HOST_MERKLE_SIGNER, MERKLE_BACK_POINTERS_TYPE_ID,
    MERKLE_ROOT_BLOB_TYPE_ID, MERKLE_ROOT_TYPE_ID,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MerkleRootEvidence {
    SignedBootstrap(Vec<u8>),
    /// A later independently signed public-probe root plus the evidence that
    /// authenticated the prior durable head. Host-key rotation produces a new
    /// signed probe anchor without invalidating roots used by persisted user
    /// and team snapshots.
    SignedRefresh {
        signed_root: Vec<u8>,
        prior_epoch: u64,
        prior: Box<MerkleRootEvidence>,
    },
    SkipPath {
        anchor_epoch: u64,
        historical_response: Vec<u8>,
        prior: Box<MerkleRootEvidence>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedMerkleRoot {
    pub epoch: u64,
    pub root_hash: [u8; 32],
    pub root_bytes: Option<Vec<u8>>,
}

impl AuthenticatedMerkleRoot {
    #[doc(hidden)]
    pub fn from_persisted_parts(
        epoch: u64,
        root_hash: [u8; 32],
        root_bytes: Option<Vec<u8>>,
    ) -> Self {
        Self {
            epoch,
            root_hash,
            root_bytes,
        }
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn root_hash(&self) -> [u8; 32] {
        self.root_hash
    }

    pub fn root_bytes(&self) -> Option<&[u8]> {
        self.root_bytes.as_deref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedMerkleRoot {
    pub(crate) epoch: u64,
    pub(crate) root_hash: [u8; 32],
    pub(crate) root_bytes: Vec<u8>,
    pub(crate) evidence: MerkleRootEvidence,
    pub(crate) authenticated_roots: Vec<AuthenticatedMerkleRoot>,
}

impl VerifiedMerkleRoot {
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn root_hash(&self) -> [u8; 32] {
        self.root_hash
    }
    pub fn root_bytes(&self) -> &[u8] {
        &self.root_bytes
    }
    pub fn evidence(&self) -> &MerkleRootEvidence {
        &self.evidence
    }
    pub fn authenticated_roots(&self) -> &[AuthenticatedMerkleRoot] {
        &self.authenticated_roots
    }

    pub fn authenticated_root_set(&self) -> AuthenticatedMerkleRoots {
        AuthenticatedMerkleRoots(
            self.authenticated_roots
                .iter()
                .map(|root| (root.epoch, root.root_hash))
                .collect(),
        )
    }

    pub fn parts(&self) -> VerifiedMerkleRootParts<'_> {
        VerifiedMerkleRootParts {
            epoch: self.epoch,
            root_hash: self.root_hash,
            root_bytes: &self.root_bytes,
            evidence: &self.evidence,
            authenticated_roots: &self.authenticated_roots,
        }
    }
}

#[derive(Clone, Copy)]
pub struct VerifiedMerkleRootParts<'a> {
    pub epoch: u64,
    pub root_hash: [u8; 32],
    pub root_bytes: &'a [u8],
    pub evidence: &'a MerkleRootEvidence,
    pub authenticated_roots: &'a [AuthenticatedMerkleRoot],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedMerkleAdvance {
    pub(crate) root: MerkleRoot,
    pub(crate) snapshot: VerifiedMerkleRoot,
    pub(crate) authenticated_roots: AuthenticatedMerkleRoots,
}

impl VerifiedMerkleAdvance {
    pub fn root(&self) -> &MerkleRoot {
        &self.root
    }

    pub fn snapshot(&self) -> &VerifiedMerkleRoot {
        &self.snapshot
    }

    pub fn authenticated_roots(&self) -> &AuthenticatedMerkleRoots {
        &self.authenticated_roots
    }
}

/// Merkle root hashes issued only by successful host-root verification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedMerkleRoots(pub(crate) BTreeMap<u64, [u8; 32]>);

impl AuthenticatedMerkleRoots {
    pub fn contains_epoch(&self, epoch: u64) -> bool {
        self.0.contains_key(&epoch)
    }

    pub(crate) fn get(&self, epoch: &u64) -> Option<&[u8; 32]> {
        self.0.get(epoch)
    }
}

/// Authenticates historical roots selected by a subsequently verified object
/// (for example, signed user links) back to an already trusted latest root.
/// The supplied response is untrusted and contributes hashes only after each
/// target's complete skip-pointer proof reaches `latest`.
pub fn authenticate_historical_roots_from_latest(
    latest: &VerifiedMerkleAdvance,
    targets: &[u64],
    full_epochs: &[u64],
    hash_epochs: &[u64],
    historical_bytes: &[u8],
) -> Result<AuthenticatedMerkleRoots> {
    if targets.len() > 64 || full_epochs.len() > 64 || hash_epochs.len() > 64 {
        return Err(Error::MerkleHistoryShape);
    }
    let historical = HistoricalMerkleRoots::decode(historical_bytes)?;
    if historical.roots.len() != full_epochs.len() || historical.hashes.len() != hash_epochs.len() {
        return Err(Error::MerkleHistoryShape);
    }
    let mut full = BTreeMap::new();
    for (&epoch, root) in full_epochs.iter().zip(historical.roots) {
        if root.epoch != epoch || full.insert(epoch, root).is_some() {
            return Err(Error::MerkleHistoryShape);
        }
    }
    let mut hashes = BTreeMap::new();
    for (&epoch, hash) in hash_epochs.iter().zip(historical.hashes) {
        if hashes.insert(epoch, hash).is_some() {
            return Err(Error::MerkleHistoryShape);
        }
    }
    let mut authenticated = latest.authenticated_roots.0.clone();
    for &target in targets {
        if authenticated.contains_key(&target) {
            continue;
        }
        if target == 0 || target >= latest.root.epoch {
            return Err(Error::MerkleHistoryShape);
        }
        let target_root = full.get(&target).ok_or(Error::MerkleHistoryShape)?;
        let target_bytes = target_root.encoded()?;
        let target_hash = prefixed_hash(MERKLE_ROOT_TYPE_ID, &target_bytes);
        let requirements = merkle_history_requirements(latest.root.epoch, target)?;
        let tailored = HistoricalMerkleRoots {
            roots: requirements
                .full_roots
                .iter()
                .map(|epoch| full.get(epoch).cloned().ok_or(Error::MerkleHistoryShape))
                .collect::<Result<Vec<_>>>()?,
            hashes: requirements
                .hashes
                .iter()
                .map(|epoch| hashes.get(epoch).copied().ok_or(Error::MerkleHistoryShape))
                .collect::<Result<Vec<_>>>()?,
        }
        .encoded()?;
        let untrusted_target = VerifiedMerkleRoot {
            epoch: target,
            root_hash: target_hash,
            root_bytes: target_bytes,
            evidence: MerkleRootEvidence::SignedBootstrap(Vec::new()),
            authenticated_roots: vec![AuthenticatedMerkleRoot {
                epoch: target,
                root_hash: target_hash,
                root_bytes: Some(target_root.encoded()?),
            }],
        };
        let proof = verify_merkle_advance(
            &untrusted_target,
            latest.snapshot.root_bytes(),
            &tailored,
            &latest.root.hostchain,
        )?;
        if proof.snapshot.root_hash != latest.snapshot.root_hash {
            return Err(Error::MerkleFork(latest.root.epoch));
        }
        for (epoch, hash) in proof.authenticated_roots.0 {
            if authenticated
                .insert(epoch, hash)
                .is_some_and(|known| known != hash)
            {
                return Err(Error::MerkleFork(epoch));
            }
        }
    }
    Ok(AuthenticatedMerkleRoots(authenticated))
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MerkleHistoryRequest {
    pub full_roots: Vec<u64>,
    pub hashes: Vec<u64>,
}

impl MerkleHistoryRequest {
    pub fn is_empty(&self) -> bool {
        self.full_roots.is_empty() && self.hashes.is_empty()
    }
}

pub fn merkle_history_requirements(latest: u64, pinned: u64) -> Result<MerkleHistoryRequest> {
    if latest < pinned {
        return Err(Error::MerkleRollback {
            stored: pinned,
            received: latest,
        });
    }
    if latest == pinned {
        return Ok(MerkleHistoryRequest::default());
    }
    let (roots, siblings) = merkle_collect_roots(latest, pinned);
    let full = roots
        .into_iter()
        .filter(|epoch| *epoch != latest && *epoch != pinned)
        .collect();
    let hashes = siblings
        .into_iter()
        .filter(|epoch| *epoch != latest && *epoch != pinned)
        .collect();
    Ok(MerkleHistoryRequest {
        full_roots: full,
        hashes,
    })
}

pub fn verify_merkle_advance(
    pinned: &VerifiedMerkleRoot,
    latest_bytes: &[u8],
    historical_bytes: &[u8],
    trusted_hostchain: &HostchainTail,
) -> Result<VerifiedMerkleAdvance> {
    let pinned_epoch = pinned.epoch;
    let pinned_hash_value = pinned.root_hash;
    let pinned_root = MerkleRoot::decode(&pinned.root_bytes)?;
    let pinned_hash = prefixed_hash(MERKLE_ROOT_TYPE_ID, &pinned_root.encoded()?);
    if pinned_root.epoch != pinned_epoch || pinned_hash != pinned_hash_value {
        return Err(Error::MerkleFork(pinned_epoch));
    }
    let latest = MerkleRoot::decode(latest_bytes)?;
    if latest.epoch < pinned_epoch {
        return Err(Error::MerkleRollback {
            stored: pinned_epoch,
            received: latest.epoch,
        });
    }
    verify_root_hostchain(&latest, trusted_hostchain)?;
    let latest_hash = prefixed_hash(MERKLE_ROOT_TYPE_ID, &latest.encoded()?);
    if latest.epoch == pinned_epoch {
        if latest_hash != pinned_hash_value {
            return Err(Error::MerkleFork(latest.epoch));
        }
        return Ok(VerifiedMerkleAdvance {
            root: latest,
            snapshot: VerifiedMerkleRoot {
                epoch: pinned_epoch,
                root_hash: pinned_hash_value,
                root_bytes: pinned.root_bytes.clone(),
                evidence: pinned.evidence.clone(),
                authenticated_roots: pinned.authenticated_roots.clone(),
            },
            authenticated_roots: AuthenticatedMerkleRoots(
                pinned
                    .authenticated_roots
                    .iter()
                    .map(|root| (root.epoch, root.root_hash))
                    .collect(),
            ),
        });
    }

    let history = merkle_history_requirements(latest.epoch, pinned_epoch)?;
    let historical = HistoricalMerkleRoots::decode(historical_bytes)?;
    if historical.roots.len() != history.full_roots.len()
        || historical.hashes.len() != history.hashes.len()
    {
        return Err(Error::MerkleHistoryShape);
    }
    let mut roots = BTreeMap::from([(pinned_epoch, pinned_root), (latest.epoch, latest.clone())]);
    let mut hashes = pinned
        .authenticated_roots
        .iter()
        .map(|root| (root.epoch, root.root_hash))
        .collect::<BTreeMap<_, _>>();
    if hashes
        .insert(latest.epoch, latest_hash)
        .is_some_and(|known| known != latest_hash)
    {
        return Err(Error::MerkleFork(latest.epoch));
    }
    for (expected_epoch, root) in history.full_roots.iter().copied().zip(historical.roots) {
        if root.epoch != expected_epoch {
            return Err(Error::MerkleHistoryShape);
        }
        hashes.insert(
            root.epoch,
            prefixed_hash(MERKLE_ROOT_TYPE_ID, &root.encoded()?),
        );
        roots.insert(root.epoch, root);
    }
    for (epoch, hash) in history.hashes.iter().copied().zip(historical.hashes) {
        if hashes
            .insert(epoch, hash)
            .is_some_and(|known| known != hash)
        {
            return Err(Error::MerkleBackPointer);
        }
    }

    let (path, _) = merkle_collect_roots(latest.epoch, pinned_epoch);
    let mut prior_pointers: Option<Vec<(u64, [u8; 32])>> = None;
    for epoch in path {
        let root = roots.get(&epoch).ok_or(Error::MerkleHistoryShape)?;
        if let Some(pointers) = &prior_pointers {
            let expected = hashes.get(&epoch).ok_or(Error::MerkleHistoryShape)?;
            if !pointers
                .iter()
                .any(|(target, hash)| target == &epoch && hash == expected)
            {
                return Err(Error::MerkleBackPointer);
            }
        }
        let pointers = merkle_backpointer_sequence(epoch)
            .into_iter()
            .map(|target| {
                hashes
                    .get(&target)
                    .copied()
                    .map(|hash| (target, hash))
                    .ok_or(Error::MerkleHistoryShape)
            })
            .collect::<Result<Vec<_>>>()?;
        if hash_back_pointers(&pointers)? != root.back_pointers {
            return Err(Error::MerkleBackPointer);
        }
        prior_pointers = Some(pointers);
    }
    let final_pointers = prior_pointers.ok_or(Error::MerkleBackPointer)?;
    if !final_pointers
        .iter()
        .any(|(epoch, hash)| epoch == &pinned_epoch && hash == &pinned_hash_value)
    {
        return Err(Error::MerkleBackPointer);
    }

    let latest_epoch = latest.epoch;
    let mut authenticated_roots = pinned
        .authenticated_roots
        .iter()
        .cloned()
        .map(|root| (root.epoch, root))
        .collect::<BTreeMap<_, _>>();
    for (epoch, hash) in &hashes {
        let root_bytes = if *epoch == latest_epoch {
            Some(latest_bytes.to_vec())
        } else {
            roots.get(epoch).map(MerkleRoot::encoded).transpose()?
        };
        authenticated_roots
            .entry(*epoch)
            .and_modify(|stored| {
                if stored.root_bytes.is_none() {
                    stored.root_bytes.clone_from(&root_bytes);
                }
            })
            .or_insert(AuthenticatedMerkleRoot {
                epoch: *epoch,
                root_hash: *hash,
                root_bytes,
            });
    }
    Ok(VerifiedMerkleAdvance {
        root: latest,
        snapshot: VerifiedMerkleRoot {
            epoch: latest_epoch,
            root_hash: latest_hash,
            root_bytes: latest_bytes.to_vec(),
            evidence: MerkleRootEvidence::SkipPath {
                anchor_epoch: pinned_epoch,
                historical_response: historical_bytes.to_vec(),
                prior: Box::new(pinned.evidence.clone()),
            },
            authenticated_roots: authenticated_roots.into_values().collect(),
        },
        authenticated_roots: AuthenticatedMerkleRoots(hashes),
    })
}

/// Revalidates an untrusted SQLite representation of a Merkle anchor back to
/// its signed public-probe bootstrap before recreating the sealed capability.
#[allow(clippy::too_many_arguments)]
pub fn restore_merkle_anchor(
    epoch: u64,
    root_hash: [u8; 32],
    root_bytes: &[u8],
    evidence: &MerkleRootEvidence,
    authenticated_roots: &[AuthenticatedMerkleRoot],
    hostchain_bytes: &[u8],
) -> Result<VerifiedMerkleRoot> {
    let links = foks_proto::decode_hostchain(hostchain_bytes)?;
    verify_hostchain(&links)?;
    let restored = restore_merkle_evidence(
        epoch,
        root_hash,
        root_bytes,
        evidence,
        authenticated_roots,
        &links,
        0,
    )?;
    let mut expected = authenticated_roots.to_vec();
    expected.sort_unstable_by_key(|root| root.epoch);
    if expected != restored.authenticated_roots {
        return Err(Error::PersistedMerkleEvidence);
    }
    Ok(restored)
}

#[allow(clippy::too_many_arguments)]
fn restore_merkle_evidence(
    epoch: u64,
    root_hash: [u8; 32],
    root_bytes: &[u8],
    evidence: &MerkleRootEvidence,
    authenticated_roots: &[AuthenticatedMerkleRoot],
    hostchain: &[HostchainLink],
    depth: usize,
) -> Result<VerifiedMerkleRoot> {
    if depth > 4096 {
        return Err(Error::PersistedMerkleEvidence);
    }
    let root = MerkleRoot::decode(root_bytes)?;
    let chain = verify_hostchain_at_tail(hostchain, &root.hostchain)?;
    if root.epoch != epoch || prefixed_hash(MERKLE_ROOT_TYPE_ID, &root.encoded()?) != root_hash {
        return Err(Error::PersistedMerkleEvidence);
    }
    match evidence {
        MerkleRootEvidence::SignedBootstrap(signed_bytes) => {
            let signed = SignedBlob::decode(signed_bytes)?;
            if signed.inner != root_bytes {
                return Err(Error::PersistedMerkleEvidence);
            }
            verify_with_delegated_blob_key(
                &chain,
                ENTITY_HOST_MERKLE_SIGNER,
                "Merkle signer",
                &signed.signature,
                MERKLE_ROOT_BLOB_TYPE_ID,
                &signed.inner,
            )?;
            Ok(VerifiedMerkleRoot {
                epoch,
                root_hash,
                root_bytes: root_bytes.to_vec(),
                evidence: evidence.clone(),
                authenticated_roots: vec![AuthenticatedMerkleRoot {
                    epoch,
                    root_hash,
                    root_bytes: Some(root_bytes.to_vec()),
                }],
            })
        }
        MerkleRootEvidence::SignedRefresh {
            signed_root,
            prior_epoch,
            prior,
        } => {
            if *prior_epoch >= epoch {
                return Err(Error::PersistedMerkleEvidence);
            }
            let signed = SignedBlob::decode(signed_root)?;
            if signed.inner != root_bytes {
                return Err(Error::PersistedMerkleEvidence);
            }
            verify_with_delegated_blob_key(
                &chain,
                ENTITY_HOST_MERKLE_SIGNER,
                "Merkle signer",
                &signed.signature,
                MERKLE_ROOT_BLOB_TYPE_ID,
                &signed.inner,
            )?;
            let prior_root = authenticated_roots
                .iter()
                .find(|candidate| candidate.epoch == *prior_epoch)
                .ok_or(Error::PersistedMerkleEvidence)?;
            let prior_bytes = prior_root
                .root_bytes
                .as_deref()
                .ok_or(Error::PersistedMerkleEvidence)?;
            let restored_prior = restore_merkle_evidence(
                prior_root.epoch,
                prior_root.root_hash,
                prior_bytes,
                prior,
                authenticated_roots,
                hostchain,
                depth + 1,
            )?;
            let mut roots = restored_prior
                .authenticated_roots
                .into_iter()
                .map(|root| (root.epoch, root))
                .collect::<BTreeMap<_, _>>();
            roots.insert(
                epoch,
                AuthenticatedMerkleRoot {
                    epoch,
                    root_hash,
                    root_bytes: Some(root_bytes.to_vec()),
                },
            );
            Ok(VerifiedMerkleRoot {
                epoch,
                root_hash,
                root_bytes: root_bytes.to_vec(),
                evidence: evidence.clone(),
                authenticated_roots: roots.into_values().collect(),
            })
        }
        MerkleRootEvidence::SkipPath {
            anchor_epoch,
            historical_response,
            prior,
        } => {
            if *anchor_epoch >= epoch {
                return Err(Error::PersistedMerkleEvidence);
            }
            let anchor = authenticated_roots
                .iter()
                .find(|root| root.epoch == *anchor_epoch)
                .ok_or(Error::PersistedMerkleEvidence)?;
            let anchor_bytes = anchor
                .root_bytes
                .as_deref()
                .ok_or(Error::PersistedMerkleEvidence)?;
            let restored_anchor = restore_merkle_evidence(
                anchor.epoch,
                anchor.root_hash,
                anchor_bytes,
                prior,
                authenticated_roots,
                hostchain,
                depth + 1,
            )?;
            let advanced = verify_merkle_advance(
                &restored_anchor,
                root_bytes,
                historical_response,
                &HostchainTail {
                    seqno: root.hostchain.seqno,
                    hash: root.hostchain.hash,
                },
            )?;
            if advanced.snapshot.epoch != epoch || advanced.snapshot.root_hash != root_hash {
                return Err(Error::PersistedMerkleEvidence);
            }
            Ok(advanced.snapshot)
        }
    }
}

pub(crate) fn verify_hostchain_at_tail(
    links: &[HostchainLink],
    expected: &HostchainTail,
) -> Result<HostchainState> {
    let mut state = HostchainState::default();
    for link in links {
        state = verify_hostchain_link(&state, link)?;
        if state.seqno == expected.seqno {
            return if state.tail == expected.hash {
                Ok(state)
            } else {
                Err(Error::PersistedMerkleEvidence)
            };
        }
    }
    Err(Error::PersistedMerkleEvidence)
}

fn verify_root_hostchain(root: &MerkleRoot, tail: &HostchainTail) -> Result<()> {
    if &root.hostchain == tail {
        Ok(())
    } else {
        Err(Error::MerkleHostchainMismatch)
    }
}

pub(crate) fn merkle_backpointer_sequence(epoch: u64) -> Vec<u64> {
    match epoch {
        0 | 1 => return Vec::new(),
        2 => return vec![1],
        3 => return vec![2, 1],
        4 => return vec![3, 2, 1],
        _ => {}
    }
    let mut cursor = 1u64;
    let mut output = Vec::new();
    while epoch > cursor {
        output.push(epoch - cursor);
        if epoch & cursor != 0 {
            break;
        }
        cursor <<= 1;
    }
    output
}

fn merkle_collect_roots(mut start: u64, end: u64) -> (Vec<u64>, Vec<u64>) {
    let mut path = Vec::new();
    let mut root_set = HashSet::new();
    let mut sibling_set = HashSet::new();
    while start > end {
        path.push(start);
        root_set.insert(start);
        for current in merkle_backpointer_sequence(start) {
            if current >= end {
                start = current;
            }
            if !root_set.contains(&current) {
                sibling_set.insert(current);
            }
        }
    }
    let mut siblings = sibling_set.into_iter().collect::<Vec<_>>();
    siblings.sort_unstable_by(|left, right| right.cmp(left));
    (path, siblings)
}

fn hash_back_pointers(pointers: &[(u64, [u8; 32])]) -> Result<[u8; 32]> {
    let value = if pointers.is_empty() {
        Value::Null
    } else {
        Value::Array(
            pointers
                .iter()
                .map(|(epoch, hash)| {
                    Value::Array(vec![Value::Unsigned(*epoch), Value::Binary(hash.to_vec())])
                })
                .collect(),
        )
    };
    Ok(prefixed_hash(
        MERKLE_BACK_POINTERS_TYPE_ID,
        &encode(&value)?,
    ))
}
