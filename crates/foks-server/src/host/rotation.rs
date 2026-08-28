use foks_proto::{
    BaseChainer, EntityId, HostchainChange, HostchainChangeItem, HostchainLink, HostchainTail,
    MerkleRoot, ProbeResponse, SignedBlob, TreeRoot, ENTITY_HOST, HOSTCHAIN_LINK_OUTER_TYPE_ID,
    HOSTCHAIN_LINK_OUTER_V1_TYPE_ID, MERKLE_ROOT_BLOB_TYPE_ID, MERKLE_ROOT_TYPE_ID,
};
use foks_server_db::{
    Database, HostKeyGeneration, HostKeyGenerationState, HostRotationOperation, HostRotationPhase,
    HostRotationPublication,
};
use foks_snowpack::{encode, Value};

use crate::keys::{HostKeyProvider, KeyGenerationId, KeyPurpose, SecretKey};
use crate::{Error, Result};

pub const MINIMUM_HOST_KEY_OBSERVATION_MICROS: u64 = 24 * 60 * 60 * 1_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostKeyRotationObservation {
    /// The add-key hostchain sequence observed by an independently syncing
    /// client or canary. This explicit acknowledgement prevents an operator
    /// from accidentally completing an unobserved rotation.
    pub add_link_seqno: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostKeyRotationState {
    pub operation_id: [u8; 16],
    pub old_generation_id: [u8; 16],
    pub new_generation_id: [u8; 16],
    pub phase: HostRotationPhase,
    pub add_link_seqno: Option<u64>,
    pub revoke_link_seqno: Option<u64>,
    pub old_public_entity_id: [u8; 33],
    pub new_public_entity_id: [u8; 33],
    pub observation_not_before: Option<u64>,
}

/// Creates and durably stages a new immutable host key without publishing a
/// hostchain link. Repeating this operation returns the same in-progress
/// operation, which gives crash recovery a stable operation ID.
pub fn stage_host_key_rotation(
    database: &mut Database,
    provider: &dyn HostKeyProvider,
    now: u64,
) -> Result<HostKeyRotationState> {
    retire_unreferenced_generation_files(database, provider)?;
    let operation = match database.active_host_rotation()? {
        Some(operation) => operation,
        None => {
            let key = provider.create_generation(KeyPurpose::Host)?;
            let generation = key.generation().as_bytes();
            let entity = host_entity(&key)?;
            let mut operation_id = [0; 16];
            getrandom::fill(&mut operation_id)
                .map_err(|_| Error::Key("operating-system entropy"))?;
            if let Err(error) = database.stage_host_key_rotation(
                operation_id,
                generation,
                &generation_file_name(generation),
                entity.as_bytes(),
                now,
            ) {
                // A same-host contender may have won after our initial read.
                // Retire the unpublished orphan before returning that durable
                // operation (or the original database error).
                provider.remove_generation(KeyPurpose::Host, key.generation())?;
                if let Some(operation) = database.active_host_rotation()? {
                    return rotation_state(database, &operation);
                }
                return Err(error.into());
            }
            database
                .active_host_rotation()?
                .ok_or(Error::Config("staged host rotation disappeared"))?
        }
    };
    validate_host_key_generations(database, provider)?;
    rotation_state(database, &operation)
}

/// Stages a new immutable host key and atomically publishes its add-key link.
/// The old signer remains usable until `complete_host_key_rotation` is called
/// after the mandatory client-observation interval.
pub fn begin_host_key_rotation(
    database: &mut Database,
    provider: &dyn HostKeyProvider,
    now: u64,
) -> Result<HostKeyRotationState> {
    let staged = stage_host_key_rotation(database, provider, now)?;
    let operation = database
        .host_rotation_operation(staged.operation_id)?
        .ok_or(Error::Config("staged host rotation disappeared"))?;
    if operation.phase == HostRotationPhase::Published {
        return rotation_state(database, &operation);
    }
    let generations = database.host_key_generations()?;
    let old = generation(&generations, operation.old_generation_id)?;
    let new = generation(&generations, operation.new_generation_id)?;
    let old_key = load_generation(provider, old)?;
    let new_key = load_generation(provider, new)?;
    validate_generation_key(old, &old_key)?;
    validate_generation_key(new, &new_key)?;
    let publication = construct_publication(
        database,
        provider,
        operation.operation_id,
        HostchainChangeItem::Key(host_entity(&new_key)?),
        &[new_key, old_key],
        1,
        now,
    )?;
    database.publish_host_key_addition(&publication)?;
    let operation = database
        .active_host_rotation()?
        .ok_or(Error::Config("published host rotation disappeared"))?;
    rotation_state(database, &operation)
}

/// Publishes the separate revoke-old-key link and completes the durable
/// operation. Callers deliberately choose when the overlap interval ends.
pub fn complete_host_key_rotation(
    database: &mut Database,
    provider: &dyn HostKeyProvider,
    operation_id: [u8; 16],
    observation: HostKeyRotationObservation,
    now: u64,
) -> Result<HostKeyRotationState> {
    let operation = database
        .host_rotation_operation(operation_id)?
        .ok_or(Error::Config("host key rotation operation does not exist"))?;
    validate_host_key_generations(database, provider)?;
    if operation.phase == HostRotationPhase::Complete {
        retire_revoked_secret(database, provider, &operation)?;
        return rotation_state(database, &operation);
    }
    if operation.phase != HostRotationPhase::Published {
        return Err(Error::Config("host key addition has not been published"));
    }
    if operation.add_link_seqno != Some(observation.add_link_seqno) {
        return Err(Error::Config(
            "observed host key addition sequence does not match",
        ));
    }
    let published_at = operation.add_published_at.ok_or(Error::Config(
        "host key addition publication time is missing",
    ))?;
    let not_before = published_at
        .checked_add(MINIMUM_HOST_KEY_OBSERVATION_MICROS)
        .ok_or(Error::Config("host key observation deadline overflow"))?;
    if now < not_before {
        return Err(Error::Config(
            "host key observation interval has not elapsed",
        ));
    }
    let generations = database.host_key_generations()?;
    let old = generation(&generations, operation.old_generation_id)?;
    let new = generation(&generations, operation.new_generation_id)?;
    let new_key = load_generation(provider, new)?;
    validate_generation_key(new, &new_key)?;
    let old_entity = EntityId::from_bytes(old.public_entity_id.clone())?;
    let publication = construct_publication(
        database,
        provider,
        operation.operation_id,
        HostchainChangeItem::Revoke(old_entity),
        &[new_key],
        0,
        now,
    )?;
    database.publish_host_key_revocation(&publication)?;
    let completed = database
        .host_rotation_operation(operation_id)?
        .ok_or(Error::Config("completed host rotation disappeared"))?;
    retire_revoked_secret(database, provider, &completed)?;
    rotation_state(database, &completed)
}

pub(crate) fn validate_host_key_generations(
    database: &Database,
    provider: &dyn HostKeyProvider,
) -> Result<()> {
    let generations = database.host_key_generations()?;
    let operation = database.active_host_rotation()?;
    let active = generations
        .iter()
        .filter(|generation| generation.state == HostKeyGenerationState::Active)
        .collect::<Vec<_>>();
    let staged = generations
        .iter()
        .filter(|generation| generation.state == HostKeyGenerationState::Staged)
        .collect::<Vec<_>>();
    let retiring = generations
        .iter()
        .filter(|generation| generation.state == HostKeyGenerationState::Retiring)
        .collect::<Vec<_>>();
    let states_match = match (active.as_slice(), &operation) {
        ([_], None) => staged.is_empty() && retiring.is_empty(),
        ([active], Some(operation)) if operation.phase == HostRotationPhase::Staged => {
            staged.len() == 1
                && retiring.is_empty()
                && staged[0].generation_id == operation.new_generation_id
                && active.generation_id == operation.old_generation_id
        }
        ([active], Some(operation)) if operation.phase == HostRotationPhase::Published => {
            staged.is_empty()
                && retiring.len() == 1
                && active.generation_id == operation.new_generation_id
                && retiring[0].generation_id == operation.old_generation_id
        }
        _ => false,
    };
    if generations.is_empty() || active.len() != 1 || !states_match {
        return Err(Error::Config("invalid host key generation ledger"));
    }
    for generation in generations
        .iter()
        .filter(|generation| generation.state != HostKeyGenerationState::Revoked)
    {
        let key = load_generation(provider, generation)?;
        validate_generation_key(generation, &key)?;
    }
    Ok(())
}

fn construct_publication(
    database: &Database,
    provider: &dyn HostKeyProvider,
    operation_id: [u8; 16],
    change_item: HostchainChangeItem,
    signers: &[SecretKey],
    signer_index: usize,
    now: u64,
) -> Result<HostRotationPublication> {
    let stored = database
        .host_bootstrap()?
        .ok_or(Error::Config("host is not bootstrapped"))?;
    let current_root = database
        .current_root()?
        .ok_or(Error::Config("host has no Merkle root"))?;
    let mut probe = ProbeResponse::decode(&stored.probe_response)?;
    let probe_root = MerkleRoot::decode(&probe.merkle_root.inner)?;
    let stored_probe_root = database
        .roots_at(&[probe_root.epoch])?
        .and_then(|mut roots| roots.pop())
        .ok_or(Error::Config("stored probe Merkle root is missing"))?;
    if stored_probe_root.root_node != probe_root.root_node
        || stored_probe_root.exact_root != probe.merkle_root.inner
        || stored_probe_root.exact_signed_root != probe.merkle_root.encoded()?
        || stored_probe_root.root_hash
            != foks_crypto::prefixed_hash(MERKLE_ROOT_TYPE_ID, &stored_probe_root.exact_root)
        || current_root.epoch < probe_root.epoch
        || probe_root.hostchain.seqno
            != u64::try_from(probe.hostchain.len())
                .map_err(|_| Error::Config("hostchain length overflow"))?
    {
        return Err(Error::Config(
            "stored probe and Merkle database history disagree",
        ));
    }
    let current_wire_root = MerkleRoot::decode(&current_root.exact_root)?;
    let current_signed_root = SignedBlob::decode(&current_root.exact_signed_root)?;
    if current_wire_root.epoch != current_root.epoch
        || current_wire_root.root_node != current_root.root_node
        || current_wire_root.hostchain != probe_root.hostchain
        || current_signed_root.inner != current_root.exact_root
        || foks_crypto::prefixed_hash(MERKLE_ROOT_TYPE_ID, &current_root.exact_root)
            != current_root.root_hash
    {
        return Err(Error::Config("current Merkle database head is invalid"));
    }
    if now < current_wire_root.time {
        return Err(Error::Config("host rotation time moved backwards"));
    }
    let host = EntityId::from_bytes(stored.host_id)?;
    let signer = host_entity(
        signers
            .get(signer_index)
            .ok_or(Error::Config("hostchain signer index"))?,
    )?;
    let hostchain_seqno = current_wire_root
        .hostchain
        .seqno
        .checked_add(1)
        .ok_or(Error::Config("hostchain sequence overflow"))?;
    let change = HostchainChange {
        chainer: BaseChainer {
            seqno: hostchain_seqno,
            previous: Some(probe_root.hostchain.hash),
            root: TreeRoot {
                epoch: current_root.epoch,
                hash: current_root.root_hash,
            },
            time: now,
        },
        host,
        signer,
        changes: vec![change_item],
    };
    let mut link = HostchainLink {
        inner: change.encoded()?,
        signatures: Vec::new(),
    };
    for key in signers {
        let bytes = link.signing_bytes(link.signatures.len())?;
        link.signatures.push(foks_crypto::sign_ed25519_typed(
            key.expose(),
            HOSTCHAIN_LINK_OUTER_V1_TYPE_ID,
            &bytes,
        )?);
    }
    let exact_hostchain_link = link.encoded()?;
    let hostchain_link_hash =
        foks_crypto::prefixed_hash(HOSTCHAIN_LINK_OUTER_TYPE_ID, &exact_hostchain_link);
    probe.hostchain.push(link);

    let root_epoch = current_root
        .epoch
        .checked_add(1)
        .ok_or(Error::Config("Merkle root epoch overflow"))?;
    let pointer_epochs = foks_merkle_store::back_pointer_sequence(root_epoch);
    let pointer_roots = database
        .roots_at(&pointer_epochs)?
        .ok_or(Error::Config("missing Merkle root history"))?;
    let back_pointers = pointer_roots
        .iter()
        .map(|root| (root.epoch, root.root_hash))
        .collect::<Vec<_>>();
    let root = MerkleRoot {
        epoch: root_epoch,
        time: now,
        back_pointers: foks_merkle_store::back_pointer_hash(&back_pointers)?,
        root_node: current_root.root_node,
        hostchain: HostchainTail {
            seqno: hostchain_seqno,
            hash: hostchain_link_hash,
        },
    };
    let exact_root = root.encoded()?;
    let root_hash = foks_crypto::prefixed_hash(MERKLE_ROOT_TYPE_ID, &exact_root);
    let root_blob = encode(&Value::Binary(exact_root.clone()))?;
    let merkle_key = provider.load_existing(KeyPurpose::Merkle)?;
    let signed_root = SignedBlob {
        signature: foks_crypto::sign_ed25519_typed(
            merkle_key.expose(),
            MERKLE_ROOT_BLOB_TYPE_ID,
            &root_blob,
        )?,
        inner: exact_root.clone(),
    };
    let exact_signed_root = signed_root.encoded()?;
    probe.merkle_root = signed_root;
    let probe_response = probe.encoded()?;
    foks_verify::verify_public_host(&stored.canonical_name, &probe_response)?;
    Ok(HostRotationPublication {
        operation_id,
        expected_root_epoch: current_root.epoch,
        expected_root_hash: current_root.root_hash,
        hostchain_seqno,
        hostchain_link_hash,
        exact_hostchain_link,
        root_epoch,
        root_hash,
        root_node: current_root.root_node,
        exact_root,
        exact_signed_root,
        back_pointers,
        probe_response,
        now,
    })
}

fn generation(generations: &[HostKeyGeneration], id: [u8; 16]) -> Result<&HostKeyGeneration> {
    generations
        .iter()
        .find(|generation| generation.generation_id == id)
        .ok_or(Error::Config("host key generation is missing"))
}

fn load_generation(
    provider: &dyn HostKeyProvider,
    generation: &HostKeyGeneration,
) -> Result<SecretKey> {
    if generation.encrypted_file_name == "host.key" {
        provider.load_existing(KeyPurpose::Host)
    } else {
        provider.load_generation(
            KeyPurpose::Host,
            KeyGenerationId::from_bytes(generation.generation_id),
        )
    }
}

fn validate_generation_key(generation: &HostKeyGeneration, key: &SecretKey) -> Result<()> {
    if key.generation().as_bytes() != generation.generation_id
        || host_entity(key)?.as_bytes() != generation.public_entity_id
    {
        return Err(Error::Config("host key generation does not match ledger"));
    }
    Ok(())
}

fn retire_revoked_secret(
    database: &Database,
    provider: &dyn HostKeyProvider,
    operation: &HostRotationOperation,
) -> Result<()> {
    let generations = database.host_key_generations()?;
    let old = generation(&generations, operation.old_generation_id)?;
    if operation.phase != HostRotationPhase::Complete
        || old.state != HostKeyGenerationState::Revoked
    {
        return Err(Error::Config("host key retirement is not durable"));
    }
    provider.remove_generation(
        KeyPurpose::Host,
        KeyGenerationId::from_bytes(old.generation_id),
    )
}

fn retire_unreferenced_generation_files(
    database: &Database,
    provider: &dyn HostKeyProvider,
) -> Result<()> {
    let generations = database.host_key_generations()?;
    for generation in provider.list_generations(KeyPurpose::Host)? {
        let required = generations.iter().any(|stored| {
            stored.generation_id == generation.as_bytes()
                && stored.state != HostKeyGenerationState::Revoked
        });
        if !required {
            provider.remove_generation(KeyPurpose::Host, generation)?;
        }
    }
    Ok(())
}

fn rotation_state(
    database: &Database,
    operation: &HostRotationOperation,
) -> Result<HostKeyRotationState> {
    let generations = database.host_key_generations()?;
    let old = generation(&generations, operation.old_generation_id)?;
    let new = generation(&generations, operation.new_generation_id)?;
    let observation_not_before = operation
        .add_published_at
        .map(|published| {
            published
                .checked_add(MINIMUM_HOST_KEY_OBSERVATION_MICROS)
                .ok_or(Error::Config("host key observation deadline overflow"))
        })
        .transpose()?;
    Ok(HostKeyRotationState {
        operation_id: operation.operation_id,
        old_generation_id: operation.old_generation_id,
        new_generation_id: operation.new_generation_id,
        phase: operation.phase,
        add_link_seqno: operation.add_link_seqno,
        revoke_link_seqno: operation.revoke_link_seqno,
        old_public_entity_id: old
            .public_entity_id
            .as_slice()
            .try_into()
            .map_err(|_| Error::Config("stored old host public entity"))?,
        new_public_entity_id: new
            .public_entity_id
            .as_slice()
            .try_into()
            .map_err(|_| Error::Config("stored new host public entity"))?,
        observation_not_before,
    })
}

fn host_entity(key: &SecretKey) -> Result<EntityId> {
    let mut bytes = Vec::with_capacity(33);
    bytes.push(ENTITY_HOST);
    bytes.extend_from_slice(&foks_crypto::ed25519_public_key(key.expose()));
    Ok(EntityId::from_bytes(bytes)?)
}

pub fn generation_file_name(generation: [u8; 16]) -> String {
    let hex = generation
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("host.{hex}.key")
}
