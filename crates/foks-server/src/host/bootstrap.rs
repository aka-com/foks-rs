use foks_proto::{
    BaseChainer, EntityId, HostchainChange, HostchainChangeItem, HostchainLink, HostchainTail,
    MerkleRoot, ProbeResponse, PublicServices, PublicZone, SignedBlob, TreeRoot, ENTITY_HOST,
    ENTITY_HOST_MERKLE_SIGNER, ENTITY_HOST_METADATA_SIGNER, ENTITY_HOST_TLS_CA,
    HOSTCHAIN_LINK_OUTER_TYPE_ID, HOSTCHAIN_LINK_OUTER_V1_TYPE_ID, MERKLE_BACK_POINTERS_TYPE_ID,
    MERKLE_ROOT_BLOB_TYPE_ID, MERKLE_ROOT_TYPE_ID, PUBLIC_ZONE_BLOB_TYPE_ID,
};
use foks_server_db::{BootstrapService, Database, HostBootstrap};
use foks_snowpack::{encode, Value};

use crate::keys::{HostKeyProvider, KeyGenerationManifest, KeyPurpose, SecretKey};
use crate::Result;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BootstrapEndpoints {
    pub probe: String,
    pub public_services: String,
    pub authenticated: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BootstrapInput {
    pub canonical_name: String,
    pub endpoints: BootstrapEndpoints,
    pub ttl_seconds: i64,
    pub now_microseconds: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BootstrapState {
    pub created: bool,
    pub host_id: EntityId,
    pub probe_response: Vec<u8>,
    pub delegated_tls_ca: Vec<u8>,
    pub key_manifest: KeyGenerationManifest,
}

pub fn bootstrap(
    database: &mut Database,
    provider: &dyn HostKeyProvider,
    input: &BootstrapInput,
) -> Result<BootstrapState> {
    validate_input(input)?;
    let manifest = KeyGenerationManifest::load_or_create(provider)?;
    let host_key = provider.load_or_create(KeyPurpose::Host)?;
    let metadata_key = provider.load_or_create(KeyPurpose::Metadata)?;
    let merkle_key = provider.load_or_create(KeyPurpose::Merkle)?;
    let delegated_tls_key = provider.load_or_create(KeyPurpose::DelegatedTls)?;
    let capability_key = provider.load_or_create(KeyPurpose::Capability)?;

    let host_id = entity(ENTITY_HOST, &host_key)?;
    let metadata_id = entity(ENTITY_HOST_METADATA_SIGNER, &metadata_key)?;
    let merkle_id = entity(ENTITY_HOST_MERKLE_SIGNER, &merkle_key)?;
    let delegated_tls_id = entity(ENTITY_HOST_TLS_CA, &delegated_tls_key)?;
    let delegated_tls_ca =
        crate::pki::host_tls::delegated_tls_ca_der(&delegated_tls_key, &input.canonical_name)?;

    let change = HostchainChange {
        chainer: BaseChainer {
            seqno: 1,
            previous: None,
            root: TreeRoot {
                epoch: 0,
                hash: [0; 32],
            },
            time: input.now_microseconds,
        },
        host: host_id.clone(),
        signer: host_id.clone(),
        changes: vec![
            HostchainChangeItem::Key(metadata_id),
            HostchainChangeItem::Key(merkle_id),
            HostchainChangeItem::TlsCa {
                id: delegated_tls_id,
                certificate: delegated_tls_ca.clone(),
            },
        ],
    };
    let mut hostchain = HostchainLink {
        inner: change.encoded()?,
        signatures: Vec::new(),
    };
    for signer in [&metadata_key, &merkle_key, &delegated_tls_key, &host_key] {
        let bytes = hostchain.signing_bytes(hostchain.signatures.len())?;
        hostchain.signatures.push(foks_crypto::sign_ed25519_typed(
            signer.expose(),
            HOSTCHAIN_LINK_OUTER_V1_TYPE_ID,
            &bytes,
        )?);
    }
    let exact_hostchain = hostchain.encoded()?;
    let hostchain_hash = foks_crypto::prefixed_hash(HOSTCHAIN_LINK_OUTER_TYPE_ID, &exact_hostchain);

    let services = PublicServices {
        probe: input.endpoints.probe.clone(),
        registration: input.endpoints.public_services.clone(),
        user: input.endpoints.authenticated.clone(),
        merkle_query: input.endpoints.public_services.clone(),
        kv_store: input.endpoints.authenticated.clone(),
        realtime: input.endpoints.authenticated.clone(),
    };
    let public_zone = PublicZone {
        ttl_seconds: input.ttl_seconds,
        services,
    };
    let public_zone_inner = public_zone.encoded()?;
    let public_zone_blob = encode(&Value::Binary(public_zone_inner.clone()))?;
    let signed_public_zone = SignedBlob {
        signature: foks_crypto::sign_ed25519_typed(
            metadata_key.expose(),
            PUBLIC_ZONE_BLOB_TYPE_ID,
            &public_zone_blob,
        )?,
        inner: public_zone_inner,
    };

    let empty_back_pointers = encode(&Value::Null)?;
    let merkle_root = MerkleRoot {
        epoch: 1,
        time: input.now_microseconds,
        back_pointers: foks_crypto::prefixed_hash(
            MERKLE_BACK_POINTERS_TYPE_ID,
            &empty_back_pointers,
        ),
        root_node: [0; 32],
        hostchain: HostchainTail {
            seqno: 1,
            hash: hostchain_hash,
        },
    };
    let exact_root = merkle_root.encoded()?;
    let root_hash = foks_crypto::prefixed_hash(MERKLE_ROOT_TYPE_ID, &exact_root);
    let root_blob = encode(&Value::Binary(exact_root.clone()))?;
    let signed_root = SignedBlob {
        signature: foks_crypto::sign_ed25519_typed(
            merkle_key.expose(),
            MERKLE_ROOT_BLOB_TYPE_ID,
            &root_blob,
        )?,
        inner: exact_root.clone(),
    };
    let exact_signed_root = signed_root.encoded()?;
    let probe = ProbeResponse {
        merkle_root: signed_root,
        public_zone: signed_public_zone,
        hostchain: vec![hostchain],
    };
    let probe_response = probe.encoded()?;
    let verified = foks_verify::verify_public_host(&input.canonical_name, &probe_response)?;
    if verified.snapshot.host_id() != host_id.as_bytes()
        || verified.snapshot.canonical_name() != input.canonical_name
    {
        return Err(crate::Error::Config("constructed bootstrap host binding"));
    }

    let services = public_zone
        .services
        .entries()
        .into_iter()
        .map(|(service_type, endpoint)| {
            Ok(BootstrapService {
                service_type: service_type.protocol_value(),
                endpoint: endpoint.to_owned(),
                advertised_blob: encode(&Value::Text(endpoint.as_bytes().to_vec()))?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let stored = HostBootstrap {
        host_id: host_id.as_bytes().to_vec(),
        canonical_name: input.canonical_name.clone(),
        probe_response: probe_response.clone(),
        key_manifest: manifest.encode(),
        host_key_generation: host_key.generation().as_bytes(),
        capability_key_generation: capability_key.generation().as_bytes(),
        hostchain_link_hash: hostchain_hash,
        exact_hostchain_link: exact_hostchain,
        services,
        root_hash,
        root_node: [0; 32],
        root_epoch: 1,
        exact_root,
        exact_signed_root,
        created_at: input.now_microseconds,
    };
    let created = database.bootstrap_host(&stored)?;
    Ok(BootstrapState {
        created,
        host_id,
        probe_response,
        delegated_tls_ca,
        key_manifest: manifest,
    })
}

/// Loads an existing immutable host bootstrap or creates it for an empty
/// installation. Restart validation deliberately does not reconstruct genesis
/// bytes from the current wall clock.
pub fn load_or_bootstrap(
    database: &mut Database,
    provider: &dyn HostKeyProvider,
    input: &BootstrapInput,
) -> Result<BootstrapState> {
    let Some(stored) = database.host_bootstrap()? else {
        return bootstrap(database, provider, input);
    };
    validate_input(input)?;

    let manifest = KeyGenerationManifest::decode(&stored.key_manifest)?;
    if stored.canonical_name != input.canonical_name {
        return Err(crate::Error::Config("stored host bootstrap configuration"));
    }
    let generations = database.host_key_generations()?;
    let require_genesis_host = generations.iter().any(|generation| {
        generation.encrypted_file_name == "host.key"
            && generation.state != foks_server_db::HostKeyGenerationState::Revoked
    });
    let capability_generations = database.capability_key_generations()?;
    let require_genesis_capability = capability_generations.iter().any(|generation| {
        generation.encrypted_file_name == "capability.key"
            && generation.state != foks_server_db::CapabilityKeyGenerationState::Revoked
    });
    manifest.validate_existing(provider, require_genesis_host, require_genesis_capability)?;
    super::rotation::validate_host_key_generations(database, provider)?;
    crate::keys::validate_capability_key_generations(database, provider)?;
    let verified = foks_verify::verify_public_host(&input.canonical_name, &stored.probe_response)?;
    if verified.snapshot.host_id() != stored.host_id
        || verified.public_zone.ttl_seconds != input.ttl_seconds
        || verified.public_zone.services.probe != input.endpoints.probe
        || verified.public_zone.services.registration != input.endpoints.public_services
        || verified.public_zone.services.merkle_query != input.endpoints.public_services
        || verified.public_zone.services.user != input.endpoints.authenticated
        || verified.public_zone.services.kv_store != input.endpoints.authenticated
        || verified.public_zone.services.realtime != input.endpoints.authenticated
    {
        return Err(crate::Error::Config("stored host bootstrap configuration"));
    }
    let probe = ProbeResponse::decode(&stored.probe_response)?;
    validate_generation_ledger_against_probe(&generations, &stored.host_id, &probe)?;
    let wire_root = MerkleRoot::decode(&probe.merkle_root.inner)?;
    let probe_root = database
        .roots_at(&[wire_root.epoch])?
        .and_then(|mut roots| roots.pop())
        .ok_or(crate::Error::Config(
            "stored probe Merkle root is absent from the database",
        ))?;
    let current_root = database
        .current_root()?
        .ok_or(crate::Error::Config("stored host has no Merkle root"))?;
    if probe_root.epoch != wire_root.epoch
        || probe_root.root_node != wire_root.root_node
        || probe_root.exact_root != probe.merkle_root.inner
        || probe_root.exact_signed_root != probe.merkle_root.encoded()?
        || probe_root.root_hash
            != foks_crypto::prefixed_hash(MERKLE_ROOT_TYPE_ID, &probe_root.exact_root)
        || current_root.epoch < probe_root.epoch
        || wire_root.hostchain.seqno != verified.snapshot.chain_seqno()
        || wire_root.hostchain.hash != verified.snapshot.chain_tail_hash()
    {
        return Err(crate::Error::Config(
            "stored probe and Merkle database history disagree",
        ));
    }
    let stored_links = database.hostchain_links()?;
    if stored_links.len() != probe.hostchain.len() {
        return Err(crate::Error::Config(
            "stored probe and hostchain database disagree",
        ));
    }
    for (index, (stored_link, wire_link)) in stored_links.iter().zip(&probe.hostchain).enumerate() {
        let exact = wire_link.encoded()?;
        if stored_link.seqno != u64::try_from(index + 1).unwrap_or(u64::MAX)
            || stored_link.exact_link != exact
            || stored_link.link_hash
                != foks_crypto::prefixed_hash(HOSTCHAIN_LINK_OUTER_TYPE_ID, &exact)
        {
            return Err(crate::Error::Config(
                "stored probe and hostchain database disagree",
            ));
        }
    }

    let parts = verified.snapshot.parts();
    let identity = foks_verify::restore_public_host_identity(
        parts.host_id,
        parts.genesis_key,
        parts.chain_seqno,
        parts.chain_tail_hash,
        parts.chain_bytes,
        parts.public_zone_bytes,
    )?;
    let delegated_key = provider.load_existing(KeyPurpose::DelegatedTls)?;
    let delegated_tls_ca =
        crate::pki::host_tls::delegated_tls_ca_der(&delegated_key, &input.canonical_name)?;
    if !identity
        .tls_ca_certificates()
        .iter()
        .any(|certificate| certificate == &delegated_tls_ca)
    {
        return Err(crate::Error::Config("stored delegated TLS key"));
    }

    Ok(BootstrapState {
        created: false,
        host_id: EntityId::from_bytes(stored.host_id)?,
        probe_response: stored.probe_response,
        delegated_tls_ca,
        key_manifest: manifest,
    })
}

fn entity(entity_type: u8, key: &SecretKey) -> Result<EntityId> {
    let mut bytes = Vec::with_capacity(33);
    bytes.push(entity_type);
    bytes.extend_from_slice(&foks_crypto::ed25519_public_key(key.expose()));
    Ok(EntityId::from_bytes(bytes)?)
}

fn validate_generation_ledger_against_probe(
    generations: &[foks_server_db::HostKeyGeneration],
    stored_host_id: &[u8],
    probe: &ProbeResponse,
) -> Result<()> {
    let mut additions = Vec::new();
    let mut revocations = Vec::new();
    for link in &probe.hostchain {
        let change = link.decode_change()?;
        for item in change.changes {
            match item {
                HostchainChangeItem::Key(entity) if entity.as_bytes()[0] == ENTITY_HOST => {
                    additions.push((change.chainer.seqno, entity.as_bytes().to_vec()));
                }
                HostchainChangeItem::Revoke(entity) if entity.as_bytes()[0] == ENTITY_HOST => {
                    revocations.push(entity.as_bytes().to_vec());
                }
                _ => {}
            }
        }
    }

    for generation in generations {
        let is_genesis = generation.encrypted_file_name == "host.key";
        let addition_count = additions
            .iter()
            .filter(|(_, entity)| entity == &generation.public_entity_id)
            .count();
        let revoke_count = revocations
            .iter()
            .filter(|entity| *entity == &generation.public_entity_id)
            .count();
        let activation_matches = if is_genesis {
            generation.public_entity_id == stored_host_id
                && generation.activated_hostchain_seqno == Some(1)
                && addition_count == 0
        } else if generation.state == foks_server_db::HostKeyGenerationState::Staged {
            generation.activated_hostchain_seqno.is_none() && addition_count == 0
        } else {
            generation.activated_hostchain_seqno.is_some_and(|seqno| {
                additions.iter().any(|(added_at, entity)| {
                    *added_at == seqno && entity == &generation.public_entity_id
                })
            }) && addition_count == 1
        };
        let revocation_matches =
            if generation.state == foks_server_db::HostKeyGenerationState::Revoked {
                revoke_count == 1
            } else {
                revoke_count == 0
            };
        if !activation_matches || !revocation_matches {
            return Err(crate::Error::Config(
                "stored host generation ledger and hostchain disagree",
            ));
        }
    }
    let non_genesis_generations = generations
        .iter()
        .filter(|generation| {
            generation.encrypted_file_name != "host.key"
                && generation.state != foks_server_db::HostKeyGenerationState::Staged
        })
        .count();
    if additions.len() != non_genesis_generations {
        return Err(crate::Error::Config(
            "stored host generation ledger and hostchain disagree",
        ));
    }
    Ok(())
}

fn validate_input(input: &BootstrapInput) -> Result<()> {
    if input.canonical_name.is_empty()
        || input.ttl_seconds <= 0
        || input.ttl_seconds > 86_400
        || [
            input.endpoints.probe.as_str(),
            input.endpoints.public_services.as_str(),
            input.endpoints.authenticated.as_str(),
        ]
        .into_iter()
        .any(|endpoint| endpoint_host(endpoint) != Some(input.canonical_name.as_str()))
    {
        return Err(crate::Error::Config("invalid host bootstrap input"));
    }
    Ok(())
}

fn endpoint_host(endpoint: &str) -> Option<&str> {
    if let Some(endpoint) = endpoint.strip_prefix('[') {
        let (host, port) = endpoint.split_once("]:")?;
        (!host.is_empty() && port.parse::<u16>().is_ok()).then_some(host)
    } else {
        let (host, port) = endpoint.rsplit_once(':')?;
        (!host.is_empty() && port.parse::<u16>().is_ok()).then_some(host)
    }
}
