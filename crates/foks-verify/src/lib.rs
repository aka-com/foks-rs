//! Native verification of the public FOKS v0.1.9 trust bootstrap.
//!
//! The input is an untrusted probe response. The output is a sealed verified
//! value only after the host chain, delegated public-zone key, delegated
//! Merkle key, and Merkle-to-hostchain binding have all verified.

#![forbid(unsafe_code)]

mod user_transition;

use std::collections::{BTreeMap, HashSet};

use foks_crypto::{commitment, prefixed_hash, verify_blob, verify_typed};
use foks_proto::{
    ChangeMetadata, EntityId, Hepk, HistoricalMerkleRoots, HostchainChangeItem, HostchainLink,
    HostchainTail, MerklePathCompressed, MerkleRoot, MerkleTerminal, ProbeResponse, PublicZone,
    Role, SignedBlob, UserChain, UserEldest, DEVICE_LABEL_TYPE_ID, ENTITY_HOST,
    ENTITY_HOST_MERKLE_SIGNER, ENTITY_HOST_METADATA_SIGNER, ENTITY_ID_MERKLE_VALUE_TYPE_ID,
    HEPK_TYPE_ID, HOSTCHAIN_LINK_OUTER_TYPE_ID, HOSTCHAIN_LINK_OUTER_V1_TYPE_ID,
    LINK_OUTER_TYPE_ID, LINK_OUTER_V1_TYPE_ID, MERKLE_BACK_POINTERS_TYPE_ID, MERKLE_NODE_TYPE_ID,
    MERKLE_ROOT_BLOB_TYPE_ID, MERKLE_ROOT_TYPE_ID, MERKLE_TREE_RF_INPUT_TYPE_ID,
    NAME_COMMITMENT_TYPE_ID, NAME_HASH_PREIMAGE_TYPE_ID, PUBLIC_ZONE_BLOB_TYPE_ID,
    TREE_LOCATION_TYPE_ID,
};
use foks_snowpack::{encode, Value};
use thiserror::Error;
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization as _};
use user_transition::replay_user_transition;
use x509_parser::prelude::parse_x509_certificate;

const ED25519_OID: &str = "1.3.101.112";

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid FOKS protocol data: {0}")]
    Protocol(#[from] foks_proto::Error),
    #[error("FOKS cryptographic verification failed: {0}")]
    Crypto(#[from] foks_crypto::Error),
    #[error("invalid canonical Snowpack: {0}")]
    Snowpack(#[from] foks_snowpack::Error),
    #[error("hostchain must contain at least one link")]
    EmptyHostchain,
    #[error("hostchain sequence is {received}, expected {expected}")]
    Sequence { expected: u64, received: u64 },
    #[error("hostchain genesis link has a previous hash")]
    GenesisHasPrevious,
    #[error("non-genesis hostchain link is missing its previous hash")]
    MissingPrevious,
    #[error("hostchain previous hash does not match the accepted tail")]
    PreviousMismatch,
    #[error("host identity changed within the hostchain")]
    HostChanged,
    #[error("the genesis host key did not sign the first link")]
    GenesisSigner,
    #[error("hostchain signature count is {signatures}, expected {keys}")]
    SignatureCount { signatures: usize, keys: usize },
    #[error("a non-genesis link used an inactive or revoked host key")]
    InactiveHostSigner,
    #[error("invalid TLS CA certificate")]
    InvalidCertificate,
    #[error("TLS CA certificate does not contain the EntityID's Ed25519 key")]
    CertificateKeyMismatch,
    #[error("hostchain contains no active delegated {0} key")]
    MissingDelegatedKey(&'static str),
    #[error("no active delegated {0} key verifies the object")]
    DelegatedSignature(&'static str),
    #[error("Merkle root commits to a different hostchain tail")]
    MerkleHostchainMismatch,
    #[error("invalid canonical probe service address")]
    InvalidProbeAddress,
    #[error("this verifier currently accepts exactly one eldest user-chain link")]
    UnsupportedUserChain,
    #[error("user-chain identity, host, or eldest invariants do not match")]
    UserBinding,
    #[error("user-chain username or device-name disclosure is invalid")]
    UserDisclosure,
    #[error("user-chain signature count is {0}, expected 2")]
    UserSignatureCount(usize),
    #[error("user chain references an unknown HEPK fingerprint")]
    MissingHepk,
    #[error("user Merkle root is not the trusted root")]
    UntrustedUserRoot,
    #[error("user Merkle proof is invalid")]
    UserMerkleProof,
    #[error("Merkle root rolled back from epoch {stored} to {received}")]
    MerkleRollback { stored: u64, received: u64 },
    #[error("Merkle root forked at epoch {0}")]
    MerkleFork(u64),
    #[error("Merkle historical response does not match the requested epochs")]
    MerkleHistoryShape,
    #[error("Merkle skip-pointer verification failed")]
    MerkleBackPointer,
    #[error("persisted Merkle evidence is incomplete or inconsistent")]
    PersistedMerkleEvidence,
    #[error("user chain sequence, previous hash, or location commitment is invalid")]
    UserChainContinuity,
    #[error("user chain transition {seqno} violates {rule}")]
    UserTransition {
        seqno: u64,
        rule: UserTransitionRule,
    },
    #[error("persisted user evidence does not reproduce its stored projection")]
    PersistedUserEvidence,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum UserTransitionRule {
    #[error("the one-member-change limit")]
    MultipleMemberChanges,
    #[error("the active-signer requirement")]
    UnknownSigner,
    #[error("shared-key generation, ordering, or HEPK rules")]
    SharedKeyRotation,
    #[error("device provisioning authorization or metadata rules")]
    Provisioning,
    #[error("device revocation authorization or key-rotation rules")]
    Revocation,
    #[error("the requirement to retain an owner device")]
    LastOwner,
    #[error("username-change or standalone-rotation metadata rules")]
    StandaloneChange,
    #[error("the requirement to retain devices and an owner shared key")]
    EmptyResult,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostService {
    pub service_type: u64,
    pub endpoint_bytes: Vec<u8>,
}

impl HostService {
    #[doc(hidden)]
    pub fn from_persisted_parts(service_type: u64, endpoint_bytes: Vec<u8>) -> Self {
        Self {
            service_type,
            endpoint_bytes,
        }
    }

    pub fn service_type(&self) -> u64 {
        self.service_type
    }

    pub fn endpoint_bytes(&self) -> &[u8] {
        &self.endpoint_bytes
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MerkleRootEvidence {
    SignedBootstrap(Vec<u8>),
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
    epoch: u64,
    root_hash: [u8; 32],
    root_bytes: Vec<u8>,
    evidence: MerkleRootEvidence,
    authenticated_roots: Vec<AuthenticatedMerkleRoot>,
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
pub struct VerifiedHostSnapshot {
    lookup_name: String,
    host_id: Vec<u8>,
    canonical_name: String,
    genesis_key: Vec<u8>,
    chain_seqno: u64,
    chain_tail_hash: [u8; 32],
    chain_bytes: Vec<u8>,
    public_zone_bytes: Vec<u8>,
    services: Vec<HostService>,
    merkle_root: VerifiedMerkleRoot,
}

impl VerifiedHostSnapshot {
    pub fn parts(&self) -> VerifiedHostSnapshotParts<'_> {
        VerifiedHostSnapshotParts {
            lookup_name: &self.lookup_name,
            host_id: &self.host_id,
            canonical_name: &self.canonical_name,
            genesis_key: &self.genesis_key,
            chain_seqno: self.chain_seqno,
            chain_tail_hash: self.chain_tail_hash,
            chain_bytes: &self.chain_bytes,
            public_zone_bytes: &self.public_zone_bytes,
            services: &self.services,
            merkle_root: self.merkle_root.parts(),
        }
    }

    pub fn lookup_name(&self) -> &str {
        &self.lookup_name
    }
    pub fn host_id(&self) -> &[u8] {
        &self.host_id
    }
    pub fn canonical_name(&self) -> &str {
        &self.canonical_name
    }
    pub fn genesis_key(&self) -> &[u8] {
        &self.genesis_key
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
    pub fn public_zone_bytes(&self) -> &[u8] {
        &self.public_zone_bytes
    }
    pub fn services(&self) -> &[HostService] {
        &self.services
    }
    pub fn merkle_root(&self) -> &VerifiedMerkleRoot {
        &self.merkle_root
    }
}

#[derive(Clone, Copy)]
pub struct VerifiedHostSnapshotParts<'a> {
    pub lookup_name: &'a str,
    pub host_id: &'a [u8],
    pub canonical_name: &'a str,
    pub genesis_key: &'a [u8],
    pub chain_seqno: u64,
    pub chain_tail_hash: [u8; 32],
    pub chain_bytes: &'a [u8],
    pub public_zone_bytes: &'a [u8],
    pub services: &'a [HostService],
    pub merkle_root: VerifiedMerkleRootParts<'a>,
}

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
    host_id: Vec<u8>,
    uid: Vec<u8>,
    chain_seqno: u64,
    chain_tail_hash: [u8; 32],
    chain_bytes: Vec<u8>,
    evidence_bytes: Vec<u8>,
    username: Vec<u8>,
    username_utf8: Vec<u8>,
    username_sequence: u64,
    merkle_epoch: u64,
    merkle_root_hash: [u8; 32],
    merkle_root_bytes: Vec<u8>,
    devices: Vec<VerifiedUserDevice>,
    shared_keys: Vec<VerifiedUserSharedKey>,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedPublicHost {
    pub snapshot: VerifiedHostSnapshot,
    pub public_zone: PublicZone,
    pub merkle_root: MerkleRoot,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedHostIdentity {
    host_id: EntityId,
    canonical_name: String,
    chain_seqno: u64,
    chain_tail_hash: [u8; 32],
    services: Vec<HostService>,
}

impl VerifiedHostIdentity {
    pub fn host_id(&self) -> &EntityId {
        &self.host_id
    }
    pub fn canonical_name(&self) -> &str {
        &self.canonical_name
    }
    pub fn chain_seqno(&self) -> u64 {
        self.chain_seqno
    }
    pub fn chain_tail_hash(&self) -> [u8; 32] {
        self.chain_tail_hash
    }
    pub fn services(&self) -> &[HostService] {
        &self.services
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedMerkleAdvance {
    root: MerkleRoot,
    snapshot: VerifiedMerkleRoot,
    authenticated_roots: AuthenticatedMerkleRoots,
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
pub struct AuthenticatedMerkleRoots(BTreeMap<u64, [u8; 32]>);

impl AuthenticatedMerkleRoots {
    pub fn contains_epoch(&self, epoch: u64) -> bool {
        self.0.contains_key(&epoch)
    }

    fn get(&self, epoch: &u64) -> Option<&[u8; 32]> {
        self.0.get(epoch)
    }
}

pub fn merkle_history_requirements(latest: u64, pinned: u64) -> Result<(Vec<u64>, Vec<u64>)> {
    if latest < pinned {
        return Err(Error::MerkleRollback {
            stored: pinned,
            received: latest,
        });
    }
    if latest == pinned {
        return Ok((Vec::new(), Vec::new()));
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
    Ok((full, hashes))
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

    let (full_epochs, hash_epochs) = merkle_history_requirements(latest.epoch, pinned_epoch)?;
    let historical = HistoricalMerkleRoots::decode(historical_bytes)?;
    if historical.roots.len() != full_epochs.len() || historical.hashes.len() != hash_epochs.len() {
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
    for (expected_epoch, root) in full_epochs.iter().copied().zip(historical.roots) {
        if root.epoch != expected_epoch {
            return Err(Error::MerkleHistoryShape);
        }
        hashes.insert(
            root.epoch,
            prefixed_hash(MERKLE_ROOT_TYPE_ID, &root.encoded()?),
        );
        roots.insert(root.epoch, root);
    }
    for (epoch, hash) in hash_epochs.iter().copied().zip(historical.hashes) {
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

fn verify_hostchain_at_tail(
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

fn merkle_backpointer_sequence(epoch: u64) -> Vec<u64> {
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

    let mut devices = BTreeMap::<Vec<u8>, VerifiedDevice>::new();
    let mut shared_keys = BTreeMap::<Role, VerifiedSharedKey>::new();
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
            devices.insert(
                eldest.member.as_bytes().to_vec(),
                VerifiedDevice {
                    id: eldest.member,
                    role: Role::OWNER,
                    hepk: device_hepk,
                    subkey: eldest.member_subkey,
                },
            );
            shared_keys.insert(
                Role::OWNER,
                VerifiedSharedKey {
                    role: Role::OWNER,
                    generation: 1,
                    verify_key: eldest.puk_verify_key,
                    hepk: puk_hepk,
                },
            );
        } else {
            replay_user_transition(
                link,
                &change,
                &chain.hepks,
                expected_host,
                &mut devices,
                &mut shared_keys,
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
    if devices.is_empty() || !shared_keys.contains_key(&Role::OWNER) {
        return Err(Error::UserTransition {
            seqno: chain_seqno,
            rule: UserTransitionRule::EmptyResult,
        });
    }
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
        username,
        username_utf8,
        username_sequence,
        devices: devices.into_values().collect(),
        shared_keys: shared_keys.into_values().collect(),
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

fn authenticated_user_chain_bytes(links: &[foks_proto::UserLink]) -> Result<Vec<u8>> {
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

fn normalize_device_name(input: &[u8]) -> Option<Vec<u8>> {
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

fn normalize_username(input: &[u8]) -> Option<Vec<u8>> {
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

fn username_merkle_key(normalized_name: &[u8], host: &EntityId, sequence: u64) -> Result<[u8; 32]> {
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

fn username_merkle_leaf(uid: &EntityId) -> Result<[u8; 32]> {
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

fn find_hepk(hepks: &[Hepk], fingerprint: [u8; 32]) -> Result<Hepk> {
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
    let encoded = encode(&Value::Array(vec![
        Value::Unsigned(0),
        Value::Binary(uid.as_bytes().to_vec()),
        Value::Unsigned(seqno),
        location.map_or(Value::Null, |value| Value::Binary(value.to_vec())),
    ]))?;
    Ok(prefixed_hash(MERKLE_TREE_RF_INPUT_TYPE_ID, &encoded))
}

fn verify_merkle_path(
    path: &MerklePathCompressed,
    query_key: &[u8; 32],
    expected_leaf: Option<&[u8; 32]>,
    expected_root_node: &[u8; 32],
) -> Result<()> {
    let mut bit_cursor = 0usize;
    let mut interiors = Vec::with_capacity(path.edges.len());
    for edge in &path.edges {
        let prefix_count = usize::from(edge[0]);
        let prefix =
            copy_and_clamp(query_key, bit_cursor, prefix_count).ok_or(Error::UserMerkleProof)?;
        bit_cursor = bit_cursor
            .checked_add(prefix_count)
            .ok_or(Error::UserMerkleProof)?;
        let branch = bit_at(query_key, bit_cursor).ok_or(Error::UserMerkleProof)?;
        bit_cursor += 1;
        let sibling: [u8; 32] = edge[1..].try_into().map_err(|_| Error::UserMerkleProof)?;
        interiors.push((
            bit_cursor - prefix_count - 1,
            prefix_count,
            prefix,
            branch,
            sibling,
        ));
    }
    let mut current = match &path.terminal {
        MerkleTerminal::Leaf { leaf, found_key } => {
            match expected_leaf {
                Some(expected) if found_key.is_none() && leaf == expected => {}
                None if found_key.is_some_and(|found| found != *query_key) => {}
                _ => return Err(Error::UserMerkleProof),
            }
            if let Some(found) = found_key.as_ref() {
                if !bits_equal(query_key, found, bit_cursor) {
                    return Err(Error::UserMerkleProof);
                }
            }
            let leaf_key = found_key.as_ref().unwrap_or(query_key);
            let wire = Value::Array(vec![
                Value::Unsigned(0),
                Value::Variant(Some((
                    b"1".to_vec(),
                    Box::new(Value::Array(vec![
                        Value::Binary(leaf_key.to_vec()),
                        Value::Binary(leaf.to_vec()),
                    ])),
                ))),
            ]);
            prefixed_hash(MERKLE_NODE_TYPE_ID, &encode(&wire)?)
        }
        MerkleTerminal::PrefixMiss {
            prefix_bit_start,
            prefix_bit_count,
            prefix,
            left,
            right,
        } => {
            let prefix_bit_count_usize =
                usize::try_from(*prefix_bit_count).map_err(|_| Error::UserMerkleProof)?;
            if expected_leaf.is_some()
                || usize::try_from(*prefix_bit_start).ok() != Some(bit_cursor)
                || prefix_matches(query_key, prefix, bit_cursor, prefix_bit_count_usize)
            {
                return Err(Error::UserMerkleProof);
            }
            let wire = Value::Array(vec![
                Value::Unsigned(1),
                Value::Variant(Some((
                    b"0".to_vec(),
                    Box::new(Value::Array(vec![
                        Value::Unsigned(*prefix_bit_start),
                        Value::Unsigned(*prefix_bit_count),
                        Value::Binary(prefix.clone()),
                        Value::Binary(left.to_vec()),
                        Value::Binary(right.to_vec()),
                    ])),
                ))),
            ]);
            prefixed_hash(MERKLE_NODE_TYPE_ID, &encode(&wire)?)
        }
    };
    for (start, count, prefix, branch, sibling) in interiors.into_iter().rev() {
        let start = u64::try_from(start).map_err(|_| Error::UserMerkleProof)?;
        let count = u64::try_from(count).map_err(|_| Error::UserMerkleProof)?;
        let (left, right) = if branch {
            (sibling, current)
        } else {
            (current, sibling)
        };
        let node = Value::Array(vec![
            Value::Unsigned(1),
            Value::Variant(Some((
                b"0".to_vec(),
                Box::new(Value::Array(vec![
                    Value::Unsigned(start),
                    Value::Unsigned(count),
                    Value::Binary(prefix),
                    Value::Binary(left.to_vec()),
                    Value::Binary(right.to_vec()),
                ])),
            ))),
        ]);
        current = prefixed_hash(MERKLE_NODE_TYPE_ID, &encode(&node)?);
    }
    if &current == expected_root_node {
        Ok(())
    } else {
        Err(Error::UserMerkleProof)
    }
}

fn prefix_matches(key: &[u8], prefix: &[u8], start: usize, count: usize) -> bool {
    if count == 0 {
        return true;
    }
    let start_byte = start >> 3;
    let end_bit = match start
        .checked_add(count)
        .and_then(|value| value.checked_sub(1))
    {
        Some(value) => value,
        None => return false,
    };
    let end_byte = end_bit >> 3;
    let Some(key_slice) = key.get(start_byte..=end_byte) else {
        return false;
    };
    if prefix.len() != key_slice.len() {
        return false;
    }
    key_slice
        .iter()
        .zip(prefix)
        .enumerate()
        .all(|(index, (left, right))| {
            let mut mask = 0xff;
            if index == 0 {
                mask >>= start & 7;
            }
            if index + 1 == key_slice.len() {
                mask &= 0xff << (7 - (end_bit & 7));
            }
            (left ^ right) & mask == 0
        })
}

fn bit_at(bytes: &[u8], bit: usize) -> Option<bool> {
    bytes
        .get(bit >> 3)
        .map(|byte| byte & (1 << (7 - (bit & 7))) != 0)
}

fn copy_and_clamp(bytes: &[u8], start: usize, count: usize) -> Option<Vec<u8>> {
    if count == 0 {
        return Some(Vec::new());
    }
    let start_byte = start >> 3;
    let end_bit = start.checked_add(count)?.checked_sub(1)?;
    let end_byte = end_bit >> 3;
    let mut output = bytes.get(start_byte..=end_byte)?.to_vec();
    output[0] &= 0xff >> (start & 7);
    let last = output.len() - 1;
    output[last] &= 0xff << (7 - (end_bit & 7));
    Some(output)
}

fn bits_equal(left: &[u8], right: &[u8], count: usize) -> bool {
    (0..count).all(|bit| bit_at(left, bit) == bit_at(right, bit))
}

#[derive(Clone, Default)]
struct HostchainState {
    seqno: u64,
    host: Option<EntityId>,
    time: u64,
    tail: [u8; 32],
    keys: BTreeMap<u8, Vec<EntityId>>,
    revoked: HashSet<EntityId>,
}

impl HostchainState {
    fn active_keys(&self, entity_type: u8) -> impl DoubleEndedIterator<Item = &EntityId> {
        self.keys
            .get(&entity_type)
            .into_iter()
            .flatten()
            .filter(|key| !self.revoked.contains(*key))
    }

    fn add_key(&mut self, key: EntityId) {
        self.keys.entry(key.entity_type()).or_default().push(key);
    }

    fn revoke(&mut self, key: &EntityId) {
        if let Some(keys) = self.keys.get_mut(&key.entity_type()) {
            keys.retain(|candidate| candidate != key);
        }
        self.revoked.insert(key.clone());
    }
}

/// Verifies an untrusted public probe response and constructs one atomic hard
/// state transaction payload. This function performs no I/O.
#[allow(clippy::too_many_arguments)]
pub fn restore_public_host_identity(
    expected_host_id: &[u8],
    expected_genesis_key: &[u8],
    expected_chain_seqno: u64,
    expected_chain_tail_hash: [u8; 32],
    chain_bytes: &[u8],
    signed_public_zone_bytes: &[u8],
) -> Result<VerifiedHostIdentity> {
    let links = foks_proto::decode_hostchain(chain_bytes)?;
    let chain = verify_hostchain(&links)?;
    let host = chain.host.clone().ok_or(Error::EmptyHostchain)?;
    if host.as_bytes() != expected_host_id
        || host.as_bytes().get(1..) != Some(expected_genesis_key)
        || chain.seqno != expected_chain_seqno
        || chain.tail != expected_chain_tail_hash
    {
        return Err(Error::HostChanged);
    }
    let signed = SignedBlob::decode(signed_public_zone_bytes)?;
    verify_with_delegated_blob_key(
        &chain,
        ENTITY_HOST_METADATA_SIGNER,
        "host metadata",
        &signed.signature,
        PUBLIC_ZONE_BLOB_TYPE_ID,
        &signed.inner,
    )?;
    let public_zone = PublicZone::decode(&signed.inner)?;
    Ok(VerifiedHostIdentity {
        host_id: host,
        canonical_name: canonical_host(&public_zone.services.probe)?.to_owned(),
        chain_seqno: chain.seqno,
        chain_tail_hash: chain.tail,
        services: public_zone_services(&public_zone)?,
    })
}

pub fn verify_public_host(lookup_name: &str, probe_bytes: &[u8]) -> Result<VerifiedPublicHost> {
    let probe = ProbeResponse::decode(probe_bytes)?;
    if probe.hostchain.is_empty() {
        return Err(Error::EmptyHostchain);
    }
    let chain = verify_hostchain(&probe.hostchain)?;
    let host = chain.host.clone().ok_or(Error::EmptyHostchain)?;

    verify_with_delegated_blob_key(
        &chain,
        ENTITY_HOST_METADATA_SIGNER,
        "host metadata",
        &probe.public_zone.signature,
        PUBLIC_ZONE_BLOB_TYPE_ID,
        &probe.public_zone.inner,
    )?;
    let public_zone = PublicZone::decode(&probe.public_zone.inner)?;

    verify_with_delegated_blob_key(
        &chain,
        ENTITY_HOST_MERKLE_SIGNER,
        "Merkle signer",
        &probe.merkle_root.signature,
        MERKLE_ROOT_BLOB_TYPE_ID,
        &probe.merkle_root.inner,
    )?;
    let merkle_root = MerkleRoot::decode(&probe.merkle_root.inner)?;
    verify_merkle_binding(&chain, &merkle_root)?;

    let root_hash = prefixed_hash(MERKLE_ROOT_TYPE_ID, &merkle_root.encoded()?);
    let services = public_zone_services(&public_zone)?;
    let canonical_name = canonical_host(&public_zone.services.probe)?.to_owned();
    let host_id = host.into_bytes();

    Ok(VerifiedPublicHost {
        snapshot: VerifiedHostSnapshot {
            lookup_name: lookup_name.to_owned(),
            genesis_key: host_id[1..].to_vec(),
            host_id,
            canonical_name,
            chain_seqno: chain.seqno,
            chain_tail_hash: chain.tail,
            chain_bytes: probe.encoded_hostchain()?,
            public_zone_bytes: probe.public_zone.encoded()?,
            services,
            merkle_root: VerifiedMerkleRoot {
                epoch: merkle_root.epoch,
                root_hash,
                root_bytes: probe.merkle_root.inner.clone(),
                evidence: MerkleRootEvidence::SignedBootstrap(probe.merkle_root.encoded()?),
                authenticated_roots: vec![AuthenticatedMerkleRoot {
                    epoch: merkle_root.epoch,
                    root_hash,
                    root_bytes: Some(probe.merkle_root.inner.clone()),
                }],
            },
        },
        public_zone,
        merkle_root,
    })
}

fn public_zone_services(public_zone: &PublicZone) -> Result<Vec<HostService>> {
    let mut services = public_zone
        .services
        .entries()
        .into_iter()
        .map(|(service_type, endpoint)| {
            Ok(HostService {
                service_type,
                endpoint_bytes: encode(&Value::Text(endpoint.as_bytes().to_vec()))?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    services.sort_by_key(|service| service.service_type);
    Ok(services)
}

fn verify_hostchain(links: &[HostchainLink]) -> Result<HostchainState> {
    let mut state = HostchainState::default();
    for link in links {
        state = verify_hostchain_link(&state, link)?;
    }
    Ok(state)
}

fn verify_hostchain_link(prior: &HostchainState, link: &HostchainLink) -> Result<HostchainState> {
    let change = link.decode_change()?;
    let expected = prior.seqno.checked_add(1).ok_or(Error::Sequence {
        expected: u64::MAX,
        received: change.chainer.seqno,
    })?;
    if change.chainer.seqno != expected {
        return Err(Error::Sequence {
            expected,
            received: change.chainer.seqno,
        });
    }
    if expected == 1 {
        if change.chainer.previous.is_some() {
            return Err(Error::GenesisHasPrevious);
        }
        if change.host != change.signer {
            return Err(Error::GenesisSigner);
        }
    } else {
        let previous = change.chainer.previous.ok_or(Error::MissingPrevious)?;
        if previous != prior.tail {
            return Err(Error::PreviousMismatch);
        }
        if prior.host.as_ref() != Some(&change.host) {
            return Err(Error::HostChanged);
        }
        let signer_is_active = prior
            .active_keys(ENTITY_HOST)
            .any(|key| key == &change.signer);
        if !signer_is_active || prior.revoked.contains(&change.signer) {
            return Err(Error::InactiveHostSigner);
        }
    }

    let link_hash = prefixed_hash(HOSTCHAIN_LINK_OUTER_TYPE_ID, &link.encoded()?);
    let mut candidate = prior.clone();
    candidate.seqno = change.chainer.seqno;
    candidate.host = Some(change.host.clone());
    candidate.time = change.chainer.time;
    candidate.tail = link_hash;

    let mut verifying_keys = Vec::new();
    for item in &change.changes {
        match item {
            HostchainChangeItem::Revoke(key) => candidate.revoke(key),
            HostchainChangeItem::Key(key) => {
                key.ed25519_key()?;
                candidate.add_key(key.clone());
                verifying_keys.push(key.clone());
            }
            HostchainChangeItem::TlsCa { id, certificate } => {
                verify_tls_ca(id, certificate)?;
                candidate.add_key(id.clone());
                verifying_keys.push(id.clone());
            }
        }
    }
    verifying_keys.push(change.signer.clone());
    if expected == 1 {
        candidate.add_key(change.signer);
    }

    if link.signatures.len() != verifying_keys.len() {
        return Err(Error::SignatureCount {
            signatures: link.signatures.len(),
            keys: verifying_keys.len(),
        });
    }
    for (index, (signature, key)) in link.signatures.iter().zip(&verifying_keys).enumerate() {
        verify_typed(
            key,
            signature,
            HOSTCHAIN_LINK_OUTER_V1_TYPE_ID,
            &link.signing_bytes(index)?,
        )?;
    }
    Ok(candidate)
}

fn verify_tls_ca(id: &EntityId, certificate: &[u8]) -> Result<()> {
    let (remaining, certificate) =
        parse_x509_certificate(certificate).map_err(|_| Error::InvalidCertificate)?;
    if !remaining.is_empty() {
        return Err(Error::InvalidCertificate);
    }
    let public_key = certificate.public_key();
    if public_key.algorithm.algorithm.to_id_string() != ED25519_OID
        || public_key.subject_public_key.data.as_ref() != &id.as_bytes()[1..]
    {
        return Err(Error::CertificateKeyMismatch);
    }
    Ok(())
}

fn verify_merkle_binding(chain: &HostchainState, root: &MerkleRoot) -> Result<()> {
    if root.hostchain.seqno != chain.seqno || root.hostchain.hash != chain.tail {
        Err(Error::MerkleHostchainMismatch)
    } else {
        Ok(())
    }
}

fn verify_with_delegated_blob_key(
    chain: &HostchainState,
    entity_type: u8,
    label: &'static str,
    signature: &foks_proto::Signature,
    blob_type_id: u64,
    inner: &[u8],
) -> Result<()> {
    let keys = chain.active_keys(entity_type).rev().collect::<Vec<_>>();
    if keys.is_empty() {
        return Err(Error::MissingDelegatedKey(label));
    }
    if keys
        .into_iter()
        .any(|key| verify_blob(key, signature, blob_type_id, inner).is_ok())
    {
        Ok(())
    } else {
        Err(Error::DelegatedSignature(label))
    }
}

fn canonical_host(address: &str) -> Result<&str> {
    if let Some(address) = address.strip_prefix('[') {
        let (host, port) = address.split_once("]:").ok_or(Error::InvalidProbeAddress)?;
        if host.is_empty() {
            return Err(Error::InvalidProbeAddress);
        }
        port.parse::<u16>()
            .map_err(|_| Error::InvalidProbeAddress)?;
        return Ok(host);
    }
    let (host, port) = address.rsplit_once(':').ok_or(Error::InvalidProbeAddress)?;
    if host.is_empty() {
        return Err(Error::InvalidProbeAddress);
    }
    port.parse::<u16>()
        .map_err(|_| Error::InvalidProbeAddress)?;
    Ok(host)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer as _, SigningKey};
    use quickcheck::QuickCheck;

    const PROBE: &[u8] = include_bytes!(
        "../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
    );
    const HOST_ID: &[u8] =
        include_bytes!("../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/host-id.snowp");
    const USER_CHAIN: &[u8] =
        include_bytes!("../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/user-chain.snowp");
    const USER_ROOT: &[u8] =
        include_bytes!("../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/merkle-root-998.snowp");
    const USER_HISTORY: &[u8] = include_bytes!(
        "../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/merkle-historical-response.snowp"
    );

    fn binary_entity(bytes: &[u8]) -> EntityId {
        let Value::Binary(bytes) = foks_snowpack::decode(bytes).unwrap() else {
            panic!("fixture is not binary");
        };
        EntityId::from_bytes(bytes).unwrap()
    }

    fn trusted_tail(public: &VerifiedPublicHost) -> HostchainTail {
        HostchainTail {
            seqno: public.snapshot.chain_seqno,
            hash: public.snapshot.chain_tail_hash,
        }
    }

    #[test]
    fn official_probe_verifies_to_a_complete_hard_state_snapshot() {
        let verified = verify_public_host("foks.app", PROBE).unwrap();
        let encoded_host_id = foks_snowpack::decode(HOST_ID).unwrap();
        let Value::Binary(host_id) = encoded_host_id else {
            panic!("HostID fixture is not binary");
        };
        assert_eq!(verified.snapshot.host_id, host_id);
        assert_eq!(verified.snapshot.canonical_name, "foks.app");
        assert_eq!(verified.snapshot.chain_seqno, 1);
        assert_eq!(
            verified.snapshot.chain_tail_hash,
            [
                0x4b, 0x42, 0x9c, 0x68, 0xb8, 0x4d, 0x4b, 0xd8, 0x44, 0xd4, 0x4e, 0x08, 0xee, 0xb9,
                0xd9, 0x61, 0x9e, 0x3b, 0x95, 0x6c, 0xa6, 0xed, 0x02, 0xa7, 0x75, 0x2b, 0xd8, 0x3d,
                0x12, 0x2e, 0x84, 0x2c,
            ]
        );
        assert_eq!(verified.snapshot.services.len(), 6);
        assert_eq!(verified.snapshot.merkle_root.epoch, 995);
        assert_eq!(
            verified.snapshot.merkle_root.root_hash,
            [
                0x02, 0x0c, 0xb9, 0x06, 0x18, 0xf6, 0x1a, 0xaf, 0xf8, 0xab, 0x9a, 0xaa, 0xd9, 0x7e,
                0xdf, 0x05, 0x69, 0x2d, 0x10, 0x0d, 0x96, 0xa9, 0x67, 0x4e, 0x41, 0xe4, 0x33, 0x61,
                0xa5, 0x20, 0xf6, 0x67,
            ]
        );
        assert_eq!(verified.merkle_root.hostchain.seqno, 1);
    }

    #[test]
    fn merkle_skip_sequences_match_the_v019_reference_cases() {
        assert_eq!(merkle_backpointer_sequence(0), []);
        assert_eq!(merkle_backpointer_sequence(1), []);
        assert_eq!(merkle_backpointer_sequence(2), [1]);
        assert_eq!(merkle_backpointer_sequence(3), [2, 1]);
        assert_eq!(merkle_backpointer_sequence(4), [3, 2, 1]);
        assert_eq!(merkle_backpointer_sequence(996), [995, 994, 992]);
        assert_eq!(merkle_backpointer_sequence(998), [997, 996]);
    }

    #[test]
    fn merkle_backpointer_sequences_are_strictly_descending() {
        fn property(epoch: u64) -> bool {
            let sequence = merkle_backpointer_sequence(epoch);
            sequence.iter().all(|target| *target < epoch)
                && sequence.windows(2).all(|pair| pair[0] > pair[1])
        }
        QuickCheck::new()
            .tests(10_000)
            .quickcheck(property as fn(u64) -> bool);
    }

    #[test]
    fn unchanged_merkle_root_preserves_the_signed_bootstrap_bytes() {
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let unchanged = verify_merkle_advance(
            &public.snapshot.merkle_root,
            &public.merkle_root.encoded().unwrap(),
            &encode(&Value::Array(vec![Value::Null, Value::Null])).unwrap(),
            &trusted_tail(&public),
        )
        .unwrap();
        assert_eq!(unchanged.snapshot, public.snapshot.merkle_root);

        let mut rollback = public.merkle_root.clone();
        rollback.epoch -= 1;
        assert!(matches!(
            verify_merkle_advance(
                &public.snapshot.merkle_root,
                &rollback.encoded().unwrap(),
                &[],
                &trusted_tail(&public),
            ),
            Err(Error::MerkleRollback { .. })
        ));
    }

    #[test]
    fn official_user_transitions_and_merkle_advancement_verify() {
        let chain = UserChain::decode(USER_CHAIN).unwrap();
        let uid = binary_entity(include_bytes!(
            "../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/uid.snowp"
        ));
        let host = chain.links[0].decode_eldest().unwrap().host;
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let advance = verify_merkle_advance(
            &public.snapshot.merkle_root,
            USER_ROOT,
            USER_HISTORY,
            &trusted_tail(&public),
        )
        .unwrap();
        assert_eq!(advance.snapshot.epoch, 998);
        assert_eq!(
            merkle_history_requirements(998, 995).unwrap(),
            (vec![996], vec![997, 996, 994, 992])
        );
        let verified = verify_user_chain(
            USER_CHAIN,
            &uid,
            &host,
            advance.authenticated_roots(),
            &chain.merkle.root().hostchain,
        )
        .unwrap();
        assert_eq!(verified.uid(), &uid);
        assert_eq!(verified.chain_seqno(), 3);
        assert_eq!(verified.devices().len(), 1);
        assert_eq!(verified.username(), b"fixtureuser");
        assert_eq!(verified.username_utf8(), b"fixtureuser");
        assert_eq!(verified.username_sequence(), 1);
        assert_eq!(verified.shared_keys()[0].generation, 2);
        assert_eq!(
            verified
                .shared_key(Role::OWNER)
                .unwrap()
                .verify_key
                .entity_type(),
            foks_proto::ENTITY_PUK_VERIFY
        );

        let mut snapshot = verified.hard_state_snapshot().unwrap();
        let restored = restore_verified_user(
            snapshot.parts(),
            advance.authenticated_roots(),
            public.snapshot.chain_bytes(),
        )
        .unwrap();
        assert_eq!(restored, verified);
        snapshot.username[0] ^= 1;
        assert!(matches!(
            restore_verified_user(
                snapshot.parts(),
                advance.authenticated_roots(),
                public.snapshot.chain_bytes(),
            ),
            Err(Error::PersistedUserEvidence)
        ));
    }

    #[test]
    fn persisted_merkle_anchor_is_reauthenticated_from_its_signed_bootstrap() {
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let advance = verify_merkle_advance(
            public.snapshot.merkle_root(),
            USER_ROOT,
            USER_HISTORY,
            &trusted_tail(&public),
        )
        .unwrap();
        let root = advance.snapshot();
        let restored = restore_merkle_anchor(
            root.epoch(),
            root.root_hash(),
            root.root_bytes(),
            root.evidence(),
            root.authenticated_roots(),
            public.snapshot.chain_bytes(),
        )
        .unwrap();
        assert_eq!(restored, *root);

        let mut bad_hash = root.root_hash();
        bad_hash[0] ^= 1;
        assert!(restore_merkle_anchor(
            root.epoch(),
            bad_hash,
            root.root_bytes(),
            root.evidence(),
            root.authenticated_roots(),
            public.snapshot.chain_bytes(),
        )
        .is_err());

        let mut bad_evidence = root.evidence().clone();
        let MerkleRootEvidence::SkipPath {
            historical_response,
            ..
        } = &mut bad_evidence
        else {
            panic!("fixture advance did not use a skip path");
        };
        let middle = historical_response.len() / 2;
        historical_response[middle] ^= 1;
        assert!(restore_merkle_anchor(
            root.epoch(),
            root.root_hash(),
            root.root_bytes(),
            &bad_evidence,
            root.authenticated_roots(),
            public.snapshot.chain_bytes(),
        )
        .is_err());

        let mut bad_hostchain = public.snapshot.chain_bytes().to_vec();
        let last = bad_hostchain.len() - 1;
        bad_hostchain[last] ^= 1;
        assert!(restore_merkle_anchor(
            root.epoch(),
            root.root_hash(),
            root.root_bytes(),
            root.evidence(),
            root.authenticated_roots(),
            &bad_hostchain,
        )
        .is_err());
    }

    #[test]
    fn provisioning_requires_device_metadata_and_lower_role_for_a_new_puk() {
        let chain = UserChain::decode(USER_CHAIN).unwrap();
        let eldest = chain.links[0].decode_eldest().unwrap();
        let mut devices = BTreeMap::from([(
            eldest.member.as_bytes().to_vec(),
            VerifiedDevice {
                id: eldest.member.clone(),
                role: Role::OWNER,
                hepk: find_hepk(&chain.hepks, eldest.member_hepk_fingerprint).unwrap(),
                subkey: eldest.member_subkey.clone(),
            },
        )]);
        let mut shared_keys = BTreeMap::from([(
            Role::OWNER,
            VerifiedSharedKey {
                role: Role::OWNER,
                generation: 1,
                verify_key: eldest.puk_verify_key,
                hepk: find_hepk(&chain.hepks, eldest.puk_hepk_fingerprint).unwrap(),
            },
        )]);
        let provision = &chain.links[1];
        let original = provision.decode_group_change().unwrap();

        let mut missing_name = original.clone();
        missing_name.metadata.clear();
        assert!(matches!(
            replay_user_transition(
                provision,
                &missing_name,
                &chain.hepks,
                &eldest.host,
                &mut devices,
                &mut shared_keys,
            ),
            Err(Error::UserTransition {
                seqno: 2,
                rule: UserTransitionRule::Provisioning
            })
        ));

        let mut owner_with_puk = original;
        owner_with_puk.shared_keys = chain.links[2].decode_group_change().unwrap().shared_keys;
        assert!(matches!(
            replay_user_transition(
                provision,
                &owner_with_puk,
                &chain.hepks,
                &eldest.host,
                &mut devices,
                &mut shared_keys,
            ),
            Err(Error::UserTransition {
                seqno: 2,
                rule: UserTransitionRule::Provisioning
            })
        ));
    }

    #[test]
    fn user_root_and_merkle_leaf_tampering_are_rejected() {
        let chain = UserChain::decode(USER_CHAIN).unwrap();
        let uid = binary_entity(include_bytes!(
            "../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/uid.snowp"
        ));
        let host = chain.links[0].decode_eldest().unwrap().host;
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let advance = verify_merkle_advance(
            &public.snapshot.merkle_root,
            USER_ROOT,
            USER_HISTORY,
            &trusted_tail(&public),
        )
        .unwrap();
        let mut wrong_roots = advance.authenticated_roots;
        wrong_roots.0.get_mut(&998).unwrap()[0] ^= 1;
        assert!(matches!(
            verify_user_chain(
                USER_CHAIN,
                &uid,
                &host,
                &wrong_roots,
                &chain.merkle.root().hostchain,
            ),
            Err(Error::UntrustedUserRoot)
        ));

        let mut bad_history = USER_HISTORY.to_vec();
        *bad_history.last_mut().unwrap() ^= 1;
        assert!(verify_merkle_advance(
            &public.snapshot.merkle_root,
            USER_ROOT,
            &bad_history,
            &trusted_tail(&public),
        )
        .is_err());

        let mut history_value = foks_snowpack::decode(USER_HISTORY).unwrap();
        let Value::Array(fields) = &mut history_value else {
            panic!("historical fixture is not a struct");
        };
        let Value::Array(hashes) = &mut fields[1] else {
            panic!("historical hashes are not a list");
        };
        let Value::Binary(duplicate_root_hash) = &mut hashes[1] else {
            panic!("historical root hash is not binary");
        };
        duplicate_root_hash[0] ^= 1;
        assert!(matches!(
            verify_merkle_advance(
                &public.snapshot.merkle_root,
                USER_ROOT,
                &encode(&history_value).unwrap(),
                &trusted_tail(&public),
            ),
            Err(Error::MerkleBackPointer)
        ));

        let mut tampered_chain = USER_CHAIN.to_vec();
        let midpoint = tampered_chain.len() / 2;
        tampered_chain[midpoint] ^= 1;
        assert!(verify_user_chain(
            &tampered_chain,
            &uid,
            &host,
            &wrong_roots,
            &chain.merkle.root().hostchain,
        )
        .is_err());
    }

    #[test]
    fn user_response_mutations_cannot_change_verified_state() {
        let chain = UserChain::decode(USER_CHAIN).unwrap();
        let uid = binary_entity(include_bytes!(
            "../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/uid.snowp"
        ));
        let host = chain.links[0].decode_eldest().unwrap().host;
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let advance = verify_merkle_advance(
            &public.snapshot.merkle_root,
            USER_ROOT,
            USER_HISTORY,
            &trusted_tail(&public),
        )
        .unwrap();
        let baseline = verify_user_chain(
            USER_CHAIN,
            &uid,
            &host,
            advance.authenticated_roots(),
            &chain.merkle.root().hostchain,
        )
        .unwrap();

        let mut tampered = USER_CHAIN.to_vec();
        for index in 0..tampered.len() {
            tampered[index] ^= 1;
            if let Ok(candidate) = verify_user_chain(
                &tampered,
                &uid,
                &host,
                advance.authenticated_roots(),
                &chain.merkle.root().hostchain,
            ) {
                assert_eq!(
                    candidate, baseline,
                    "unauthenticated byte {index} changed verified state"
                );
            }
            tampered[index] ^= 1;
        }
    }

    #[test]
    fn disclosed_username_and_device_names_are_not_malleable() {
        let chain = UserChain::decode(USER_CHAIN).unwrap();
        let uid = binary_entity(include_bytes!(
            "../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/uid.snowp"
        ));
        let host = chain.links[0].decode_eldest().unwrap().host;
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let advance = verify_merkle_advance(
            public.snapshot.merkle_root(),
            USER_ROOT,
            USER_HISTORY,
            &trusted_tail(&public),
        )
        .unwrap();
        for needle in [b"fixtureuser".as_slice(), b"fixture-device".as_slice()] {
            let offsets = USER_CHAIN
                .windows(needle.len())
                .enumerate()
                .filter_map(|(offset, value)| (value == needle).then_some(offset))
                .collect::<Vec<_>>();
            assert!(
                !offsets.is_empty(),
                "fixture no longer contains disclosed field"
            );
            for offset in offsets {
                let mut tampered = USER_CHAIN.to_vec();
                tampered[offset] ^= 1;
                assert!(
                    verify_user_chain(
                        &tampered,
                        &uid,
                        &host,
                        advance.authenticated_roots(),
                        &chain.merkle.root().hostchain,
                    )
                    .is_err(),
                    "disclosed field mutation at {offset} was accepted"
                );
            }
        }
    }

    #[test]
    fn names_match_the_v019_unicode_normalization_matrix() {
        for (input, expected) in [
            ("max", Some("max")),
            ("m_a.x", Some("m_a_x")),
            ("Wkładam-Kurtkę", Some("wkladam_kurtke")),
            ("für_Ihre_Beiträge", Some("fur_ihre_beitrage")),
            ("nå_mål", Some("na_mal")),
            ("Æok", Some("aok")),
            ("ßßs", Some("sss")),
            ("Vláda_zvyšuje", Some("vlada_zvysuje")),
            ("m+a+x", None),
            ("m__a_x", None),
            ("这是书", None),
        ] {
            assert_eq!(
                normalize_username(input.as_bytes()).as_deref(),
                expected.map(str::as_bytes),
                "{input}"
            );
        }
        for (input, expected) in [
            ("max's iPhone", Some("max's iphone")),
            ("M_A.X 7.4+ Bizzle-", Some("m_a.x 7.4+ bizzle-")),
            ("Wkładam_Kurtkę-", Some("wkladam_kurtke-")),
            ("a-t-il réagi ça ne", Some("a-t-il reagi ca ne")),
            ("Æok", None),
            ("Łukasz", None),
            ("maa__a", None),
            ("maaa ", None),
            ("a’b’c’", None),
        ] {
            assert_eq!(
                normalize_device_name(input.as_bytes()).as_deref(),
                expected.map(str::as_bytes),
                "{input}"
            );
        }
    }

    #[test]
    fn canonical_public_zone_tampering_fails_signature_verification() {
        let mut tampered = PROBE.to_vec();
        let offset = tampered
            .windows(b"foks.app:4430".len())
            .position(|window| window == b"foks.app:4430")
            .unwrap();
        tampered[offset] = b'g';
        assert!(matches!(
            verify_public_host("foks.app", &tampered),
            Err(Error::DelegatedSignature("host metadata"))
        ));
    }

    #[test]
    fn persisted_public_zone_is_reauthenticated_before_service_reuse() {
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let snapshot = &public.snapshot;
        let identity = restore_public_host_identity(
            snapshot.host_id(),
            snapshot.genesis_key(),
            snapshot.chain_seqno(),
            snapshot.chain_tail_hash(),
            snapshot.chain_bytes(),
            snapshot.public_zone_bytes(),
        )
        .unwrap();
        assert_eq!(identity.services(), snapshot.services());

        let mut zone = snapshot.public_zone_bytes().to_vec();
        *zone.last_mut().unwrap() ^= 1;
        assert!(restore_public_host_identity(
            snapshot.host_id(),
            snapshot.genesis_key(),
            snapshot.chain_seqno(),
            snapshot.chain_tail_hash(),
            snapshot.chain_bytes(),
            &zone,
        )
        .is_err());
    }

    #[test]
    fn hostchain_signature_tampering_is_rejected() {
        let mut probe = ProbeResponse::decode(PROBE).unwrap();
        let foks_proto::Signature::Ed25519(signature) = &mut probe.hostchain[0].signatures[0]
        else {
            panic!("fixture uses unexpected signature type");
        };
        signature[0] ^= 1;
        assert!(verify_hostchain(&probe.hostchain).is_err());
    }

    #[test]
    fn merkle_binding_is_checked_separately_from_its_signature() {
        let probe = ProbeResponse::decode(PROBE).unwrap();
        let chain = verify_hostchain(&probe.hostchain).unwrap();
        let mut root = MerkleRoot::decode(&probe.merkle_root.inner).unwrap();
        root.hostchain.hash[0] ^= 1;
        assert!(matches!(
            verify_merkle_binding(&chain, &root),
            Err(Error::MerkleHostchainMismatch)
        ));
    }

    #[test]
    fn canonical_address_parser_handles_dns_and_ipv6() {
        assert_eq!(canonical_host("foks.app:4430").unwrap(), "foks.app");
        assert_eq!(canonical_host("[::1]:4430").unwrap(), "::1");
        assert!(canonical_host("[]:4430").is_err());
        assert!(canonical_host("foks.app").is_err());
        assert!(canonical_host(":4430").is_err());
    }

    #[test]
    fn valid_extension_and_revocation_links_verify() {
        let original = SigningKey::from_bytes(&[1; 32]);
        let added = SigningKey::from_bytes(&[2; 32]);
        let host = entity_id(ENTITY_HOST, &original);
        let added_host = entity_id(ENTITY_HOST, &added);

        let mut genesis = link(1, None, &host, &host, Value::Null, vec![&original]);
        let genesis_hash = prefixed_hash(HOSTCHAIN_LINK_OUTER_TYPE_ID, &genesis.encoded().unwrap());
        let add_change = Value::Array(vec![Value::Array(vec![
            Value::Unsigned(2),
            Value::Variant(Some((
                b"2".to_vec(),
                Box::new(Value::Binary(added_host.as_bytes().to_vec())),
            ))),
        ])]);
        let extension = link(
            2,
            Some(genesis_hash),
            &host,
            &host,
            add_change,
            vec![&added, &original],
        );

        let extension_hash =
            prefixed_hash(HOSTCHAIN_LINK_OUTER_TYPE_ID, &extension.encoded().unwrap());
        let revoke_change = Value::Array(vec![Value::Array(vec![
            Value::Unsigned(1),
            Value::Variant(Some((
                b"1".to_vec(),
                Box::new(Value::Binary(added_host.as_bytes().to_vec())),
            ))),
        ])]);
        let revocation = link(
            3,
            Some(extension_hash),
            &host,
            &host,
            revoke_change,
            vec![&original],
        );

        let state = verify_hostchain(&[genesis.clone(), extension, revocation]).unwrap();
        assert_eq!(state.seqno, 3);
        assert!(state.revoked.contains(&added_host));
        assert!(!state.active_keys(ENTITY_HOST).any(|key| key == &added_host));

        genesis.signatures[0] = foks_proto::Signature::Ed25519([0; 64]);
        assert!(verify_hostchain(&[genesis]).is_err());
    }

    fn entity_id(entity_type: u8, key: &SigningKey) -> EntityId {
        let mut bytes = vec![entity_type];
        bytes.extend(key.verifying_key().as_bytes());
        EntityId::from_bytes(bytes).unwrap()
    }

    fn link(
        seqno: u64,
        previous: Option<[u8; 32]>,
        host: &EntityId,
        signer: &EntityId,
        changes: Value,
        signing_keys: Vec<&SigningKey>,
    ) -> HostchainLink {
        let inner = encode(&Value::Array(vec![
            Value::Unsigned(1),
            Value::Variant(Some((
                b"1".to_vec(),
                Box::new(Value::Array(vec![
                    Value::Array(vec![
                        Value::Unsigned(seqno),
                        previous.map_or(Value::Null, |hash| Value::Binary(hash.to_vec())),
                        Value::Array(vec![Value::Unsigned(0), Value::Binary(vec![0; 32])]),
                        Value::Unsigned(seqno),
                    ]),
                    Value::Binary(host.as_bytes().to_vec()),
                    Value::Binary(signer.as_bytes().to_vec()),
                    changes,
                ])),
            ))),
        ]))
        .unwrap();
        let mut link = HostchainLink {
            inner,
            signatures: Vec::new(),
        };
        for key in signing_keys {
            let object = link.signing_bytes(link.signatures.len()).unwrap();
            let mut message = Vec::with_capacity(8 + object.len());
            message.extend(HOSTCHAIN_LINK_OUTER_V1_TYPE_ID.to_be_bytes());
            message.extend(object);
            link.signatures.push(foks_proto::Signature::Ed25519(
                key.sign(&message).to_bytes(),
            ));
        }
        link
    }
}
