//! Hostchain, service-delegation, and public-probe verification.

use crate::{
    encode, parse_x509_certificate, prefixed_hash, verify_blob, verify_typed,
    AuthenticatedMerkleRoot, BTreeMap, EntityId, Error, HashSet, MerkleRoot, MerkleRootEvidence,
    ProbeResponse, PublicZone, Result, ServiceType, SignedBlob, Value, VerifiedMerkleRoot,
    VerifiedMerkleRootParts, ED25519_OID, ENTITY_HOST, ENTITY_HOST_MERKLE_SIGNER,
    ENTITY_HOST_METADATA_SIGNER, HOSTCHAIN_LINK_OUTER_TYPE_ID, HOSTCHAIN_LINK_OUTER_V1_TYPE_ID,
    MERKLE_ROOT_BLOB_TYPE_ID, MERKLE_ROOT_TYPE_ID, PUBLIC_ZONE_BLOB_TYPE_ID,
};
use foks_proto::{HostchainChangeItem, HostchainLink};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostService {
    pub service_type: ServiceType,
    pub endpoint_bytes: Vec<u8>,
}

impl HostService {
    #[doc(hidden)]
    pub fn from_persisted_parts(service_type: ServiceType, endpoint_bytes: Vec<u8>) -> Self {
        Self {
            service_type,
            endpoint_bytes,
        }
    }

    pub fn service_type(&self) -> ServiceType {
        self.service_type
    }

    pub fn endpoint_bytes(&self) -> &[u8] {
        &self.endpoint_bytes
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedHostSnapshot {
    pub(crate) lookup_name: String,
    pub(crate) host_id: Vec<u8>,
    pub(crate) canonical_name: String,
    pub(crate) genesis_key: Vec<u8>,
    pub(crate) chain_seqno: u64,
    pub(crate) chain_tail_hash: [u8; 32],
    pub(crate) chain_bytes: Vec<u8>,
    pub(crate) public_zone_bytes: Vec<u8>,
    pub(crate) services: Vec<HostService>,
    pub(crate) merkle_root: VerifiedMerkleRoot,
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
    tls_ca_certificates: Vec<Vec<u8>>,
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
    pub fn tls_ca_certificates(&self) -> &[Vec<u8>] {
        &self.tls_ca_certificates
    }
}

#[derive(Clone, Default)]
pub(crate) struct HostchainState {
    pub(crate) seqno: u64,
    pub(crate) host: Option<EntityId>,
    pub(crate) time: u64,
    pub(crate) tail: [u8; 32],
    pub(crate) keys: BTreeMap<u8, Vec<EntityId>>,
    pub(crate) revoked: HashSet<EntityId>,
    pub(crate) tls_cas: Vec<(EntityId, Vec<u8>)>,
}

impl HostchainState {
    pub(crate) fn active_keys(
        &self,
        entity_type: u8,
    ) -> impl DoubleEndedIterator<Item = &EntityId> {
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
        self.tls_cas.retain(|(candidate, _)| candidate != key);
    }

    fn add_tls_ca(&mut self, key: EntityId, certificate: Vec<u8>) {
        self.add_key(key.clone());
        self.tls_cas.retain(|(candidate, _)| candidate != &key);
        self.tls_cas.push((key, certificate));
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
        tls_ca_certificates: chain
            .tls_cas
            .into_iter()
            .map(|(_, certificate)| certificate)
            .collect(),
    })
}

/// Re-authenticates a persisted hostchain and proves that an externalized
/// historical tail is an ancestor of its current head.
pub fn hostchain_contains_tail(
    chain_bytes: &[u8],
    expected_seqno: u64,
    expected_tail: [u8; 32],
) -> Result<bool> {
    let links = foks_proto::decode_hostchain(chain_bytes)?;
    if expected_seqno == 0 || expected_seqno as usize > links.len() {
        return Ok(false);
    }
    let mut state = HostchainState::default();
    for link in links.into_iter().take(expected_seqno as usize) {
        state = verify_hostchain_link(&state, &link)?;
    }
    Ok(state.seqno == expected_seqno && state.tail == expected_tail)
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

    let root_hash = prefixed_hash(MERKLE_ROOT_TYPE_ID, &merkle_root.encoded()?)?;
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

pub(crate) fn verify_hostchain(links: &[HostchainLink]) -> Result<HostchainState> {
    let mut state = HostchainState::default();
    for link in links {
        state = verify_hostchain_link(&state, link)?;
    }
    Ok(state)
}

pub(crate) fn verify_hostchain_link(
    prior: &HostchainState,
    link: &HostchainLink,
) -> Result<HostchainState> {
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

    let link_hash = prefixed_hash(HOSTCHAIN_LINK_OUTER_TYPE_ID, &link.encoded()?)?;
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
                candidate.add_tls_ca(id.clone(), certificate.clone());
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

pub(crate) fn verify_merkle_binding(chain: &HostchainState, root: &MerkleRoot) -> Result<()> {
    if root.hostchain.seqno != chain.seqno || root.hostchain.hash != chain.tail {
        Err(Error::MerkleHostchainMismatch)
    } else {
        Ok(())
    }
}

pub(crate) fn verify_with_delegated_blob_key(
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

pub(crate) fn canonical_host(address: &str) -> Result<&str> {
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
